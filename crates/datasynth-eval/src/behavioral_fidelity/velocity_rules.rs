//! P4 — Velocity-rule trigger rate gap.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

use super::math::{is_weekend, percentile};
use super::types::{Record, RuleSet, VelocityRuleKind, VelocityRuleSpec};

/// Per-rule trigger rate result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleResult {
    pub id: String,
    pub trigger_rate_reference: f64,
    pub trigger_rate_syn: f64,
    pub abs_gap: f64,
}

/// Computes per-rule TR(real) and TR(syn) and the mean absolute gap across rules.
pub fn evaluate_rule_set<F>(
    rules: &RuleSet,
    real: &[Record],
    syn: &[Record],
    entity_of: F,
) -> (Vec<RuleResult>, f64)
where
    F: Fn(&Record) -> Option<String> + Copy,
{
    let mut results = Vec::with_capacity(rules.rules.len());
    let mut gaps_sum = 0.0;
    for rule in &rules.rules {
        let tr_real = trigger_rate(rule, real, entity_of);
        let tr_syn = trigger_rate(rule, syn, entity_of);
        let gap = (tr_real - tr_syn).abs();
        gaps_sum += gap;
        results.push(RuleResult {
            id: rule.id.clone(),
            trigger_rate_reference: tr_real,
            trigger_rate_syn: tr_syn,
            abs_gap: gap,
        });
    }
    let mean_gap = if rules.rules.is_empty() {
        0.0
    } else {
        gaps_sum / rules.rules.len() as f64
    };
    (results, mean_gap)
}

fn trigger_rate<F>(rule: &VelocityRuleSpec, records: &[Record], entity_of: F) -> f64
where
    F: Fn(&Record) -> Option<String> + Copy,
{
    let by_entity: HashMap<String, Vec<&Record>> = group_by_entity(records, entity_of);
    if by_entity.is_empty() {
        return 0.0;
    }
    let triggered: usize = by_entity
        .values()
        .filter(|rows| entity_triggers(rule, rows))
        .count();
    triggered as f64 / by_entity.len() as f64
}

fn group_by_entity<F>(records: &[Record], entity_of: F) -> HashMap<String, Vec<&Record>>
where
    F: Fn(&Record) -> Option<String> + Copy,
{
    let mut by: HashMap<String, Vec<&Record>> = HashMap::new();
    for r in records {
        if let Some(e) = entity_of(r) {
            by.entry(e).or_default().push(r);
        }
    }
    by
}

fn entity_triggers(rule: &VelocityRuleSpec, rows: &[&Record]) -> bool {
    match &rule.kind {
        VelocityRuleKind::CountPerEntityPerDay { threshold } => {
            let mut by_day: HashMap<NaiveDate, u32> = HashMap::new();
            for r in rows {
                if !is_weekend(r.entry_date) {
                    *by_day.entry(r.entry_date).or_insert(0) += 1;
                }
            }
            by_day.values().any(|&c| c > *threshold)
        }
        VelocityRuleKind::DistinctAccountsPerEntityPerDay { threshold } => {
            let mut by_day: HashMap<NaiveDate, HashSet<&str>> = HashMap::new();
            for r in rows {
                by_day
                    .entry(r.entry_date)
                    .or_default()
                    .insert(r.gl_account.as_str());
            }
            by_day.values().any(|s| s.len() > *threshold as usize)
        }
        VelocityRuleKind::SumAmountPerEntityPerDayAbovePercentile { pct } => {
            let mut by_day: HashMap<NaiveDate, f64> = HashMap::new();
            for r in rows {
                *by_day.entry(r.entry_date).or_insert(0.0) += r.functional_amount.abs();
            }
            let sums: Vec<f64> = by_day.values().copied().collect();
            if sums.is_empty() {
                return false;
            }
            let threshold = percentile(&sums, *pct);
            sums.iter().any(|&s| s > threshold)
        }
        VelocityRuleKind::DormantAccountActivity { inactivity_days } => {
            let mut last_seen: HashMap<&str, NaiveDate> = HashMap::new();
            let mut sorted = rows.to_vec();
            sorted.sort_by_key(|r| r.entry_date);
            for r in sorted {
                if let Some(prev) = last_seen.get(r.gl_account.as_str()) {
                    if (r.entry_date - *prev).num_days() >= *inactivity_days {
                        return true;
                    }
                }
                last_seen.insert(r.gl_account.as_str(), r.entry_date);
            }
            false
        }
        VelocityRuleKind::DistinctTradingPartnersPerEntityPerDay { threshold } => {
            let mut by_day: HashMap<NaiveDate, HashSet<&str>> = HashMap::new();
            for r in rows {
                if let Some(tp) = r.trading_partner.as_deref() {
                    by_day.entry(r.entry_date).or_default().insert(tp);
                }
            }
            by_day.values().any(|s| s.len() > *threshold as usize)
        }
        VelocityRuleKind::AmountSpikeRatio { window_days, ratio } => {
            let mut sorted = rows.to_vec();
            sorted.sort_by_key(|r| r.entry_date);
            for end_idx in 0..sorted.len() {
                let end_date = sorted[end_idx].entry_date;
                let start_date = end_date - chrono::Duration::days(*window_days);
                let window: Vec<f64> = sorted
                    .iter()
                    .filter(|r| r.entry_date >= start_date && r.entry_date <= end_date)
                    .map(|r| r.functional_amount.abs())
                    .filter(|x| *x > 0.0)
                    .collect();
                if window.len() < 3 {
                    continue;
                }
                let max = window.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                let med = percentile(&window, 0.5);
                if med > 0.0 && (max / med) > *ratio {
                    return true;
                }
            }
            false
        }
        VelocityRuleKind::OffHoursPosting => rows.iter().any(|r| is_weekend(r.entry_date)),
        VelocityRuleKind::PostClosePosting {
            tolerance_business_days,
        } => rows.iter().any(|r| {
            let period_end = last_day_of_month(r.effective_date);
            let tol_days = *tolerance_business_days * 7 / 5;
            (r.entry_date - period_end).num_days() > tol_days
        }),
        VelocityRuleKind::RoundDollarConcentration { share_threshold } => {
            let n = rows.len();
            if n == 0 {
                return false;
            }
            let rounds = rows
                .iter()
                .filter(|r| {
                    let amt = r.functional_amount.abs().round() as i64;
                    amt > 0 && amt % 1000 == 0
                })
                .count();
            (rounds as f64 / n as f64) > *share_threshold
        }
        VelocityRuleKind::BackdatingDays { gap_days } => rows
            .iter()
            .any(|r| (r.effective_date - r.entry_date).num_days() > *gap_days),
    }
}

fn last_day_of_month(d: NaiveDate) -> NaiveDate {
    let (y, m) = (d.year(), d.month());
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1)
        .expect("valid month+1 date")
        .pred_opt()
        .expect("date before first of month always exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(src: &str, gl: &str, day: u32, amt: f64, tp: Option<&str>) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, day).unwrap();
        Record {
            source: src.into(),
            gl_account: gl.into(),
            cost_center: None,
            profit_center: None,
            trading_partner: tp.map(String::from),
            je_number: format!("J{src}{day}{gl}"),
            je_line_number: "001".into(),
            effective_date: d,
            entry_date: d,
            created_at: None,
            functional_amount: amt,
            header_text: String::new(),
            line_text: String::new(),
        }
    }

    #[test]
    fn r1_count_per_day_triggers() {
        let recs: Vec<Record> = (0..6).map(|_| r("A", "100", 3, 1.0, None)).collect();
        let kind = VelocityRuleSpec {
            id: "R1".into(),
            description: "".into(),
            kind: VelocityRuleKind::CountPerEntityPerDay { threshold: 5 },
        };
        let refs: Vec<&Record> = recs.iter().collect();
        assert!(entity_triggers(&kind, &refs));
    }

    #[test]
    fn r7_off_hours_triggers_on_weekend_record() {
        // 2022-01-01 was a Saturday.
        let recs = [r("A", "1", 1, 1.0, None)];
        let kind = VelocityRuleSpec {
            id: "R7".into(),
            description: "".into(),
            kind: VelocityRuleKind::OffHoursPosting,
        };
        let refs: Vec<&Record> = recs.iter().collect();
        assert!(entity_triggers(&kind, &refs));
    }

    #[test]
    fn r10_backdating_triggers_when_eff_minus_entry_over_30() {
        let mut rec = r("A", "1", 1, 1.0, None);
        rec.effective_date = NaiveDate::from_ymd_opt(2022, 3, 31).unwrap();
        rec.entry_date = NaiveDate::from_ymd_opt(2022, 1, 15).unwrap();
        let kind = VelocityRuleSpec {
            id: "R10".into(),
            description: "".into(),
            kind: VelocityRuleKind::BackdatingDays { gap_days: 30 },
        };
        assert!(entity_triggers(&kind, &[&rec]));
    }

    #[test]
    fn canonical_rules_has_ten_entries() {
        let rs = RuleSet::canonical_gl_rules();
        assert_eq!(rs.rules.len(), 10);
        let ids: Vec<&str> = rs.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["R1", "R2", "R3", "R4", "R5", "R6", "R7", "R8", "R9", "R10"]
        );
    }
}

//! P1 — Inter-event time distribution + within-entity autocorrelation.

use std::collections::HashMap;

use chrono::NaiveDate;

use super::math::{pearson_lag1_correlation, wasserstein_1};
use super::types::Record;

/// Result of P1 on one (real, synthetic) pair for one entity column.
#[derive(Debug, Clone, PartialEq)]
pub struct P1Outcome {
    pub ietd_w1_days: f64,
    pub autocorr_real: f64,
    pub autocorr_syn: f64,
    pub autocorr_gap: f64,
}

/// Compute pooled IETD W₁ and lag-1 within-entity autocorrelation gap.
///
/// `entity_of` projects each Record to its entity identifier; `date_of`
/// projects to its day-resolution timestamp. The pooled IETD is the union
/// of within-entity inter-event time sequences.
pub fn compute_p1<F, G>(real: &[Record], syn: &[Record], entity_of: F, date_of: G) -> P1Outcome
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> NaiveDate + Copy,
{
    let iets_real = pooled_iets(real, entity_of, date_of);
    let iets_syn = pooled_iets(syn, entity_of, date_of);
    let w1 = wasserstein_1(&iets_real, &iets_syn);

    let auto_real = pooled_autocorr(real, entity_of, date_of);
    let auto_syn = pooled_autocorr(syn, entity_of, date_of);
    P1Outcome {
        ietd_w1_days: w1,
        autocorr_real: auto_real,
        autocorr_syn: auto_syn,
        autocorr_gap: (auto_real - auto_syn).abs(),
    }
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

fn pooled_iets<F, G>(records: &[Record], entity_of: F, date_of: G) -> Vec<f64>
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> NaiveDate + Copy,
{
    let mut out = Vec::new();
    for (_e, mut rows) in group_by_entity(records, entity_of) {
        if rows.len() < 2 {
            continue;
        }
        rows.sort_by_key(|r| date_of(r));
        for w in rows.windows(2) {
            let d = (date_of(w[1]) - date_of(w[0])).num_days() as f64;
            if d >= 0.0 {
                out.push(d);
            }
        }
    }
    out
}

fn pooled_autocorr<F, G>(records: &[Record], entity_of: F, date_of: G) -> f64
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> NaiveDate + Copy,
{
    let mut acc = 0.0;
    let mut n = 0;
    for (_e, mut rows) in group_by_entity(records, entity_of) {
        if rows.len() < 3 {
            continue;
        }
        rows.sort_by_key(|r| date_of(r));
        let iets: Vec<f64> = rows
            .windows(2)
            .map(|w| (date_of(w[1]) - date_of(w[0])).num_days() as f64)
            .collect();
        if let Some(r) = pearson_lag1_correlation(&iets) {
            acc += r;
            n += 1;
        }
    }
    if n == 0 {
        0.0
    } else {
        acc / n as f64
    }
}

/// Convenience: project Record -> `Source`.
pub fn source_of(r: &Record) -> Option<String> {
    Some(r.source.clone())
}

/// Convenience: project Record -> `TradingPartner`.
pub fn trading_partner_of(r: &Record) -> Option<String> {
    r.trading_partner.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn rec(src: &str, year: i32, mon: u32, day: u32) -> Record {
        Record {
            source: src.into(),
            gl_account: "1".into(),
            cost_center: None,
            profit_center: None,
            trading_partner: None,
            je_number: format!("JE-{src}-{day}"),
            je_line_number: "001".into(),
            effective_date: NaiveDate::from_ymd_opt(year, mon, day).unwrap(),
            entry_date: NaiveDate::from_ymd_opt(year, mon, day).unwrap(),
            created_at: None,
            functional_amount: 1.0,
            header_text: String::new(),
            line_text: String::new(),
        }
    }

    #[test]
    fn p1_identical_data_w1_zero_autocorr_gap_zero() {
        let real = vec![
            rec("A", 2022, 1, 1),
            rec("A", 2022, 1, 2),
            rec("A", 2022, 1, 3),
            rec("A", 2022, 1, 4),
            rec("B", 2022, 1, 1),
            rec("B", 2022, 1, 5),
            rec("B", 2022, 1, 9),
        ];
        let out = compute_p1(&real, &real, source_of, |r| r.entry_date);
        assert!(out.ietd_w1_days.abs() < 1e-9);
        assert!(out.autocorr_gap.abs() < 1e-9);
    }

    #[test]
    fn p1_compressed_vs_uniform_detects_shift() {
        let real = vec![
            rec("A", 2022, 1, 1),
            rec("A", 2022, 1, 2),
            rec("A", 2022, 1, 3),
            rec("A", 2022, 1, 4),
            rec("B", 2022, 1, 1),
            rec("B", 2022, 1, 5),
            rec("B", 2022, 1, 9),
        ];
        let syn = vec![
            rec("A", 2022, 1, 1),
            rec("A", 2022, 1, 6),
            rec("A", 2022, 1, 11),
            rec("A", 2022, 1, 16),
            rec("B", 2022, 1, 1),
            rec("B", 2022, 1, 5),
            rec("B", 2022, 1, 9),
        ];
        let out = compute_p1(&real, &syn, source_of, |r| r.entry_date);
        assert!(
            out.ietd_w1_days > 0.5,
            "expected non-trivial W1, got {}",
            out.ietd_w1_days
        );
    }
}

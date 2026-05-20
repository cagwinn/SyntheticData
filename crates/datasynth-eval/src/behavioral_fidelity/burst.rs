//! P2 — Burst structure: active lifetime + burst length + JE-line-burst.

use std::collections::HashMap;

use chrono::NaiveDate;

use super::math::wasserstein_1;
use super::types::Record;

/// Per-entity active lifetime in days = max(date) - min(date), 0 for singletons.
pub fn active_lifetimes<F, G>(records: &[Record], entity_of: F, date_of: G) -> Vec<f64>
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> NaiveDate + Copy,
{
    let mut by: HashMap<String, (NaiveDate, NaiveDate)> = HashMap::new();
    for r in records {
        if let Some(e) = entity_of(r) {
            let d = date_of(r);
            by.entry(e)
                .and_modify(|(lo, hi)| {
                    if d < *lo {
                        *lo = d;
                    }
                    if d > *hi {
                        *hi = d;
                    }
                })
                .or_insert((d, d));
        }
    }
    by.into_values()
        .map(|(lo, hi)| (hi - lo).num_days() as f64)
        .collect()
}

pub fn active_lifetime_w1<F, G>(real: &[Record], syn: &[Record], entity_of: F, date_of: G) -> f64
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> NaiveDate + Copy,
{
    let r = active_lifetimes(real, entity_of, date_of);
    let s = active_lifetimes(syn, entity_of, date_of);
    wasserstein_1(&r, &s)
}

/// Compute pooled burst-length distribution at the given gap threshold (in days).
///
/// A burst is a maximal contiguous subsequence of an entity's events with
/// consecutive gaps `<= threshold_days`. Singletons contribute a burst of length 1.
pub fn burst_lengths_at_threshold<F, G>(
    records: &[Record],
    entity_of: F,
    date_of: G,
    threshold_days: i64,
) -> Vec<f64>
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> NaiveDate + Copy,
{
    let mut by: HashMap<String, Vec<NaiveDate>> = HashMap::new();
    for r in records {
        if let Some(e) = entity_of(r) {
            by.entry(e).or_default().push(date_of(r));
        }
    }
    let mut out = Vec::new();
    for (_e, mut dates) in by {
        dates.sort();
        let mut len = 1u32;
        for w in dates.windows(2) {
            let gap = (w[1] - w[0]).num_days();
            if gap <= threshold_days {
                len += 1;
            } else {
                out.push(len as f64);
                len = 1;
            }
        }
        out.push(len as f64);
    }
    out
}

pub fn burst_length_w1<F, G>(
    real: &[Record],
    syn: &[Record],
    entity_of: F,
    date_of: G,
    threshold_days: i64,
) -> f64
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> NaiveDate + Copy,
{
    let r = burst_lengths_at_threshold(real, entity_of, date_of, threshold_days);
    let s = burst_lengths_at_threshold(syn, entity_of, date_of, threshold_days);
    wasserstein_1(&r, &s)
}

#[cfg(test)]
mod burst_length_tests {
    use super::super::ietd::source_of;
    use super::*;

    fn rec(src: &str, day: u32) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, day).unwrap();
        Record {
            source: src.into(),
            gl_account: "1".into(),
            cost_center: None,
            profit_center: None,
            trading_partner: None,
            je_number: format!("J{src}{day}"),
            je_line_number: "001".into(),
            effective_date: d,
            entry_date: d,
            created_at: None,
            functional_amount: 1.0,
            header_text: String::new(),
            line_text: String::new(),
        }
    }

    #[test]
    fn burst_lengths_threshold_1day() {
        let rs = vec![
            rec("A", 1),
            rec("A", 2),
            rec("A", 3),
            rec("B", 1),
            rec("B", 5),
            rec("B", 6),
        ];
        let mut bl = burst_lengths_at_threshold(&rs, source_of, |r| r.entry_date, 1);
        bl.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(bl, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn burst_lengths_threshold_4days_merges_b() {
        let rs = vec![
            rec("A", 1),
            rec("A", 2),
            rec("A", 3),
            rec("B", 1),
            rec("B", 5),
            rec("B", 6),
        ];
        let mut bl = burst_lengths_at_threshold(&rs, source_of, |r| r.entry_date, 4);
        bl.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(bl, vec![3.0, 3.0]);
    }
}

#[cfg(test)]
mod active_lifetime_tests {
    use super::super::ietd::source_of;
    use super::*;

    fn rec(src: &str, day: u32) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, day).unwrap();
        Record {
            source: src.into(),
            gl_account: "1".into(),
            cost_center: None,
            profit_center: None,
            trading_partner: None,
            je_number: format!("J{day}"),
            je_line_number: "001".into(),
            effective_date: d,
            entry_date: d,
            created_at: None,
            functional_amount: 1.0,
            header_text: String::new(),
            line_text: String::new(),
        }
    }

    #[test]
    fn active_lifetimes_basic() {
        let rs = vec![
            rec("A", 1),
            rec("A", 10),
            rec("B", 5),
            rec("B", 5),
            rec("C", 1),
            rec("C", 31),
        ];
        let mut lifs = active_lifetimes(&rs, source_of, |r| r.entry_date);
        lifs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(lifs, vec![0.0, 9.0, 30.0]);
    }
}

/// Pooled lines-per-JE-Number distribution.
pub fn je_line_burst_lengths(records: &[Record]) -> Vec<f64> {
    let mut by: HashMap<String, u32> = HashMap::new();
    for r in records {
        *by.entry(r.je_number.clone()).or_insert(0) += 1;
    }
    by.values().map(|&n| n as f64).collect()
}

pub fn je_line_burst_w1(real: &[Record], syn: &[Record]) -> f64 {
    let r = je_line_burst_lengths(real);
    let s = je_line_burst_lengths(syn);
    wasserstein_1(&r, &s)
}

#[cfg(test)]
mod je_line_burst_tests {
    use super::*;

    fn rec(je: &str, line: &str) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        Record {
            source: "S".into(),
            gl_account: "1".into(),
            cost_center: None,
            profit_center: None,
            trading_partner: None,
            je_number: je.into(),
            je_line_number: line.into(),
            effective_date: d,
            entry_date: d,
            created_at: None,
            functional_amount: 1.0,
            header_text: String::new(),
            line_text: String::new(),
        }
    }

    #[test]
    fn lines_per_je_grouped_correctly() {
        let rs = vec![
            rec("J1", "001"),
            rec("J1", "002"),
            rec("J1", "003"),
            rec("J2", "001"),
            rec("J2", "002"),
            rec("J3", "001"),
        ];
        let mut lengths = je_line_burst_lengths(&rs);
        lengths.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(lengths, vec![1.0, 2.0, 3.0]);
    }
}

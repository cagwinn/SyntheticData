//! Synth-only intraday structural metrics (second resolution, off-hours rate).
//!
//! These are *informational*: they do not contribute to the composite BF score
//! because the corpus is date-only.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};

use super::math::pearson_lag1_correlation;
use super::types::Record;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntradayMetrics {
    /// Pooled within-entity IETD median in seconds (synth only).
    pub p1_intra_w1_seconds: f64,
    /// Pooled lag-1 autocorrelation at second resolution.
    pub p1_intra_autocorr: f64,
    /// Off-hours rate: weekend or hour ∈ [0..6) ∪ [22..24).
    pub off_hours_rate: f64,
}

pub fn compute_intraday<F>(records: &[Record], entity_of: F) -> Option<IntradayMetrics>
where
    F: Fn(&Record) -> Option<String> + Copy,
{
    if records.iter().all(|r| r.created_at.is_none()) {
        return None;
    }

    let mut by: HashMap<String, Vec<DateTime<Utc>>> = HashMap::new();
    for r in records {
        if let (Some(e), Some(ts)) = (entity_of(r), r.created_at) {
            by.entry(e).or_default().push(ts);
        }
    }
    let mut all_iets: Vec<f64> = Vec::new();
    let mut auto_sum = 0.0;
    let mut auto_n = 0;
    for (_e, mut times) in by {
        if times.len() < 2 {
            continue;
        }
        times.sort();
        let iets: Vec<f64> = times
            .windows(2)
            .map(|w| (w[1] - w[0]).num_seconds() as f64)
            .collect();
        all_iets.extend(iets.iter().copied());
        if let Some(rc) = pearson_lag1_correlation(&iets) {
            auto_sum += rc;
            auto_n += 1;
        }
    }
    let pooled_median = if all_iets.is_empty() {
        0.0
    } else {
        let mut sorted = all_iets.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted[sorted.len() / 2]
    };
    let autocorr = if auto_n == 0 {
        0.0
    } else {
        auto_sum / auto_n as f64
    };

    let mut off = 0usize;
    let mut total = 0usize;
    for r in records {
        if let Some(ts) = r.created_at {
            total += 1;
            let h = ts.hour();
            let wd = ts.weekday().num_days_from_monday();
            if wd >= 5 || !(6..22).contains(&h) {
                off += 1;
            }
        }
    }
    let off_rate = if total == 0 {
        0.0
    } else {
        off as f64 / total as f64
    };

    Some(IntradayMetrics {
        p1_intra_w1_seconds: pooled_median,
        p1_intra_autocorr: autocorr,
        off_hours_rate: off_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::super::ietd::source_of;
    use super::*;
    use chrono::{NaiveDate, TimeZone};

    fn r(src: &str, hour: u32, minute: u32) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, 3).unwrap(); // Monday
        let ts = Utc.with_ymd_and_hms(2022, 1, 3, hour, minute, 0).unwrap();
        Record {
            source: src.into(),
            gl_account: "1".into(),
            cost_center: None,
            profit_center: None,
            trading_partner: None,
            je_number: format!("J{src}{hour}{minute}"),
            je_line_number: "001".into(),
            effective_date: d,
            entry_date: d,
            created_at: Some(ts),
            functional_amount: 1.0,
            header_text: String::new(),
            line_text: String::new(),
        }
    }

    #[test]
    fn intraday_off_hours_rate_known() {
        let rs = vec![r("A", 23, 0), r("A", 3, 0), r("A", 10, 0), r("A", 14, 0)];
        let m = compute_intraday(&rs, source_of).unwrap();
        assert!((m.off_hours_rate - 0.5).abs() < 1e-9);
    }
}

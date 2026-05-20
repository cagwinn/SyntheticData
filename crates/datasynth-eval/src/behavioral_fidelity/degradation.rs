//! Noise-floor anchored degradation-ratio normaliser.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::types::Record;

/// Deterministic 50/50 split of `records` by `JENumber` (so multi-line JEs stay together).
pub fn split_5050(records: &[Record], seed: u64) -> (Vec<Record>, Vec<Record>) {
    let mut a = Vec::new();
    let mut b = Vec::new();
    for r in records {
        if hash_to_bucket(&r.je_number, seed) {
            a.push(r.clone());
        } else {
            b.push(r.clone());
        }
    }
    (a, b)
}

fn hash_to_bucket(key: &str, seed: u64) -> bool {
    let mut h = DefaultHasher::new();
    seed.hash(&mut h);
    key.hash(&mut h);
    (h.finish() & 1) == 0
}

/// Maximum DR returned when the baseline is degenerate (≈ 0).  Picked
/// large enough to surface a real signal in dashboards but small enough
/// that one degenerate metric can't overwhelm a composite average.
///
/// Background: when `real_split_baseline ≈ 0` the metric is essentially
/// "real_A and real_B agree perfectly"; any non-trivial synthetic value
/// is undefined-DR territory.  Capping at 100 keeps individual per-metric
/// reporting meaningful while preventing a single degenerate metric from
/// dominating a headline composite.  The composite itself excludes
/// degenerate-baseline metrics entirely (Fix A); this cap is belt-and-
/// braces for any future path that bypasses that filter.
pub const DEGENERATE_BASELINE_CAP: f64 = 100.0;

/// Epsilon below which a baseline value is considered degenerate (≈ 0).
/// Shared with the composite-aggregation filter in `mod.rs`.
pub const DEGENERATE_BASELINE_EPS: f64 = 1e-9;

/// Returns `true` when `baseline` is so close to zero that the degradation
/// ratio is undefined (noise floor indistinguishable from zero on this corpus).
#[inline]
pub fn is_degenerate_baseline(baseline: f64) -> bool {
    baseline.abs() < DEGENERATE_BASELINE_EPS
}

/// SP3.13 — Metric names whose raw value strongly depends on the synthetic
/// event count rather than the per-event fidelity.  Empirically determined
/// by comparing normal-volume vs 10× volume-scaled runs (v5.22 baselines).
///
/// Metrics flagged here as `is_volume_bounded` are still reported and
/// included in the composite by default.  Consumers wanting a volume-corrected
/// composite should filter these out or use `composite_bf_volume_corrected`.
pub const VOLUME_BOUNDED_METRICS: &[&str] = &[
    "P1_IETD_W1_days",         // raw shrinks with volume (more events = smaller IET)
    "P3_Fanout_W1_CostCenter", // raw grows with volume (more samples = bigger fanout)
    "P3_Fanout_W1_GLAccount",
    "P3_Fanout_W1_ProfitCenter",
    "P3_Fanout_W1_TradingPartner",
    "P2_BurstLen_W1_7d", // raw grows with volume in long tail
];

/// Returns `true` when the named metric is known to scale with synthetic event
/// volume rather than per-event fidelity.  See [`VOLUME_BOUNDED_METRICS`].
#[inline]
pub fn is_volume_bounded(metric_name: &str) -> bool {
    VOLUME_BOUNDED_METRICS.contains(&metric_name)
}

/// degradation_ratio(real_vs_syn, real_A_vs_real_B).
///
/// When the real-split baseline is at or below `EPS`, the comparison
/// is degenerate (the metric has no measurable noise floor on this
/// corpus).  Rather than dividing by a tiny float and producing
/// astronomic DRs that dominate the composite, we cap the return at
/// `DEGENERATE_BASELINE_CAP`.  If the synthetic side is also ≈ 0 the
/// result is 0.0.
pub fn degradation_ratio(real_vs_syn: f64, real_split_baseline: f64) -> f64 {
    const EPS: f64 = DEGENERATE_BASELINE_EPS;
    if real_split_baseline.abs() < EPS {
        if real_vs_syn.abs() < EPS {
            // Both ~0 — metric is degenerate but synthetic matches.
            0.0
        } else {
            DEGENERATE_BASELINE_CAP
        }
    } else {
        real_vs_syn / real_split_baseline
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn r(je: &str, line: &str) -> Record {
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
    fn split_keeps_multiline_jes_together() {
        let rs = vec![
            r("J1", "001"),
            r("J1", "002"),
            r("J1", "003"),
            r("J2", "001"),
            r("J3", "001"),
            r("J4", "001"),
            r("J5", "001"),
            r("J6", "001"),
        ];
        let (a, b) = split_5050(&rs, 42);
        let j1_a = a.iter().filter(|r| r.je_number == "J1").count();
        let j1_b = b.iter().filter(|r| r.je_number == "J1").count();
        assert!((j1_a == 3 && j1_b == 0) || (j1_a == 0 && j1_b == 3));
        let (a2, _) = split_5050(&rs, 42);
        assert_eq!(a, a2);
        let (a3, _) = split_5050(&rs, 99);
        assert_ne!(a, a3);
    }

    #[test]
    fn degradation_ratio_caps_at_degenerate_baseline() {
        // Non-zero synthetic with zero baseline → capped at DEGENERATE_BASELINE_CAP (100.0)
        let dr = degradation_ratio(1.0, 0.0);
        assert_eq!(dr, 100.0);
        assert_eq!(dr, DEGENERATE_BASELINE_CAP);
        // Real ~0 / synthetic ~0 → 0
        assert_eq!(degradation_ratio(0.0, 0.0), 0.0);
        let tiny = 1e-12;
        assert_eq!(degradation_ratio(tiny, tiny), 0.0);
        // Healthy ratio path unchanged
        assert_eq!(degradation_ratio(0.5, 0.25), 2.0);
        // Large finite ratio passes through
        let dr = degradation_ratio(100.0, 1.0);
        assert_eq!(dr, 100.0);
    }
}

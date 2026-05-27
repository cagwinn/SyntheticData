//! C3 Piece 1 — calibration objective.
//!
//! The objective is the scalar function L(synth, ref) the calibration
//! loop minimises. Initial cut supports the three headline scalars
//! the BF report already exposes (`composite_bf_score`,
//! `composite_bf_median`, `composite_bf_volume_corrected`). Per-
//! submetric weighting is deferred — the BF report's per-entity
//! shape uses typed fields rather than a generic map, so a weighted
//! sub-metric objective needs a small typed dispatch table that's
//! out of scope for the first cut.

use crate::behavioral_fidelity::report::BehavioralFidelityReport;

/// What we're minimising.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectiveMetric {
    /// Sajja BF composite mean (the default headline metric).
    /// Lower is better — 0 means synth matches reference within
    /// the noise floor.
    #[default]
    BfComposite,
    /// BF composite median — robust to a small number of very
    /// high-DR outlier sub-metrics. Useful when one sub-metric is
    /// wildly off and skews the mean.
    BfCompositeMedian,
    /// Volume-corrected BF composite (excludes degenerate +
    /// volume-bounded metrics). Use when the loop should ignore
    /// engine-volume-dependent gaps so calibration focuses on the
    /// structural-fidelity sub-metrics.
    BfCompositeVolumeCorrected,
}

impl ObjectiveMetric {
    /// Display-friendly identifier for logs + history persistence.
    pub fn name(&self) -> &'static str {
        match self {
            Self::BfComposite => "bf_composite",
            Self::BfCompositeMedian => "bf_composite_median",
            Self::BfCompositeVolumeCorrected => "bf_composite_volume_corrected",
        }
    }
}

/// One iterable target: which scalar drives the loop + optional
/// convergence threshold.
#[derive(Debug, Clone, Default)]
pub struct CalibrationObjective {
    /// Which scalar drives the loop.
    pub metric: ObjectiveMetric,
    /// Optional convergence target — stop when the multi-seed mean
    /// loss is ≤ this. `None` (default) lets the loop run to
    /// `max_iterations` / patience exhaustion.
    pub target: Option<f64>,
}

impl CalibrationObjective {
    /// Default — minimise the BF composite mean, no explicit target.
    pub fn bf_composite() -> Self {
        Self {
            metric: ObjectiveMetric::BfComposite,
            target: None,
        }
    }

    /// Pick a specific metric. Identical to constructing the struct
    /// directly; provided as a fluent builder for callers that read
    /// nicer with a chained form.
    pub fn with_metric(mut self, m: ObjectiveMetric) -> Self {
        self.metric = m;
        self
    }

    /// Set a convergence target. Stops the loop when E_seed[L] ≤ `t`.
    pub fn with_target(mut self, t: f64) -> Self {
        self.target = Some(t);
        self
    }

    /// Compute the scalar loss for one report.
    ///
    /// All three scalars are always present in the
    /// [`BehavioralFidelityReport`] (the v5.x writer fills them via
    /// `#[serde(default)]`-zero on older fixtures), so this never
    /// returns `None`. The return is `Option` to allow a future
    /// per-submetric variant to signal a missing-path error without
    /// a breaking signature change.
    pub fn evaluate(&self, report: &BehavioralFidelityReport) -> Option<f64> {
        Some(match self.metric {
            ObjectiveMetric::BfComposite => report.composite_bf_score,
            ObjectiveMetric::BfCompositeMedian => report.composite_bf_median,
            ObjectiveMetric::BfCompositeVolumeCorrected => report.composite_bf_volume_corrected,
        })
    }

    /// Aggregate multiple BF reports into a single (mean, std) pair —
    /// the multi-seed harness the C3 loop relies on for noise-floor
    /// rejection (T3 methodology: single-shard composite CV ≈ 25 %,
    /// so a step must beat the prior best by > ~σ to be credited).
    ///
    /// Returns `None` only when `reports` is empty.
    pub fn aggregate(&self, reports: &[BehavioralFidelityReport]) -> Option<(f64, f64)> {
        if reports.is_empty() {
            return None;
        }
        let vals: Vec<f64> = reports.iter().filter_map(|r| self.evaluate(r)).collect();
        if vals.is_empty() {
            return None;
        }
        let n = vals.len() as f64;
        let mean = vals.iter().sum::<f64>() / n;
        let variance = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
        Some((mean, variance.sqrt()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavioral_fidelity::report::{
        BaselineValues, CorpusSummary, EntityMetrics, GateResult, PerMetric,
    };
    use chrono::Utc;
    use std::collections::BTreeMap;

    fn empty_per_metric() -> PerMetric {
        PerMetric {
            raw: 0.0,
            baseline: 0.0,
            dr: 0.0,
            is_degenerate_baseline: false,
            is_volume_bounded: false,
        }
    }

    fn empty_entity_metrics() -> EntityMetrics {
        EntityMetrics {
            entity_column: "test".into(),
            p1_ietd: empty_per_metric(),
            p1_autocorr: empty_per_metric(),
            p2_active_lifetime: empty_per_metric(),
            p2_burst_len_by_threshold: BTreeMap::new(),
            p2_je_line_burst: empty_per_metric(),
            p3_fanout_by_attr: BTreeMap::new(),
            p3_clustering: empty_per_metric(),
            p3_triangle_log_ratio: empty_per_metric(),
            p4_rule_results: vec![],
            p4_mean_gap: empty_per_metric(),
        }
    }

    fn make_report(composite: f64, median: f64, vc: f64) -> BehavioralFidelityReport {
        BehavioralFidelityReport {
            profile: "test".into(),
            generator_id: "test".into(),
            generator_version: "v5.x".into(),
            seed: 0,
            generated_at: Utc::now(),
            reference_corpus: CorpusSummary {
                path: "/dev/null".into(),
                n_rows: 0,
                n_entities_primary: 0,
                n_entities_secondary: 0,
                period_start: None,
                period_end: None,
            },
            synthetic: CorpusSummary {
                path: "/dev/null".into(),
                n_rows: 0,
                n_entities_primary: 0,
                n_entities_secondary: 0,
                period_start: None,
                period_end: None,
            },
            noise_floor: BaselineValues {
                p1_ietd_w1_days: 0.0,
                p1_autocorr_gap: 0.0,
                p2_active_lifetime_w1: 0.0,
                p2_burst_len_by_threshold: BTreeMap::new(),
                p2_je_line_burst_w1: 0.0,
                p3_fanout_by_attr: BTreeMap::new(),
                p3_clustering_gap: 0.0,
                p3_triangle_log_ratio: 0.0,
                p4_mean_gap: 0.0,
            },
            per_entity: {
                let mut m = BTreeMap::new();
                m.insert("test".to_string(), empty_entity_metrics());
                m
            },
            composite_bf_score: composite,
            composite_bf_median: median,
            n_metrics_aggregated: 1,
            n_metrics_excluded_degenerate: 0,
            composite_bf_volume_corrected: vc,
            n_metrics_excluded_volume: 0,
            intraday_structural: None,
            gates: GateResult {
                fail_if_dr_above: 100.0,
                fail_if_composite_above: 100.0,
                passed: true,
                failures: vec![],
            },
        }
    }

    #[test]
    fn bf_composite_default() {
        let obj = CalibrationObjective::default();
        assert_eq!(obj.metric, ObjectiveMetric::BfComposite);
        assert_eq!(obj.target, None);
        let report = make_report(42.0, 17.0, 36.0);
        assert_eq!(obj.evaluate(&report), Some(42.0));
    }

    #[test]
    fn bf_composite_median_picks_median_field() {
        let obj = CalibrationObjective::default().with_metric(ObjectiveMetric::BfCompositeMedian);
        let report = make_report(42.0, 17.0, 36.0);
        assert_eq!(obj.evaluate(&report), Some(17.0));
    }

    #[test]
    fn bf_composite_volume_corrected_picks_vc_field() {
        let obj = CalibrationObjective::default()
            .with_metric(ObjectiveMetric::BfCompositeVolumeCorrected);
        let report = make_report(42.0, 17.0, 36.0);
        assert_eq!(obj.evaluate(&report), Some(36.0));
    }

    #[test]
    fn target_round_trips() {
        let obj = CalibrationObjective::bf_composite().with_target(25.0);
        assert_eq!(obj.target, Some(25.0));
    }

    #[test]
    fn aggregate_returns_mean_and_std() {
        let obj = CalibrationObjective::bf_composite();
        let reports = vec![
            make_report(40.0, 0.0, 0.0),
            make_report(42.0, 0.0, 0.0),
            make_report(44.0, 0.0, 0.0),
        ];
        let (mean, std) = obj.aggregate(&reports).expect("non-empty");
        assert!((mean - 42.0).abs() < 1e-9, "mean = {mean}");
        // Population std of {40, 42, 44}: sqrt(((-2)² + 0 + 2²) / 3) = sqrt(8/3) ≈ 1.6330
        assert!((std - (8.0_f64 / 3.0).sqrt()).abs() < 1e-9, "std = {std}");
    }

    #[test]
    fn aggregate_empty_input_is_none() {
        let obj = CalibrationObjective::bf_composite();
        assert_eq!(obj.aggregate(&[]), None);
    }
}

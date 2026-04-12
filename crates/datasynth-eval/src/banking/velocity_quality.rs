//! Velocity feature quality evaluator.
//!
//! Validates that pre-computed velocity features on bank transactions are
//! internally consistent (windows nest correctly) and statistically calibrated
//! (z-scores are centered, amounts sum coherently).

use serde::{Deserialize, Serialize};

use crate::error::EvalResult;

/// Velocity features extracted from a single transaction.
#[derive(Debug, Clone, Default)]
pub struct VelocityFeaturesData {
    pub txn_count_1h: u32,
    pub txn_count_24h: u32,
    pub txn_count_7d: u32,
    pub txn_count_30d: u32,
    pub amount_sum_24h: f64,
    pub amount_sum_7d: f64,
    pub amount_sum_30d: f64,
    pub amount_zscore: f64,
}

/// Thresholds for velocity quality evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VelocityQualityThresholds {
    /// Minimum fraction of transactions that must have velocity features populated
    pub min_coverage: f64,
    /// Maximum fraction allowed to have window-ordering violations
    pub max_ordering_violation_rate: f64,
    /// Minimum fraction allowed to have amount-sum-ordering violations
    pub max_amount_violation_rate: f64,
    /// Expected mean of aggregated z-scores (should be near 0)
    pub zscore_mean_tolerance: f64,
}

impl Default for VelocityQualityThresholds {
    fn default() -> Self {
        Self {
            min_coverage: 0.95,
            max_ordering_violation_rate: 0.01,
            max_amount_violation_rate: 0.01,
            zscore_mean_tolerance: 0.5,
        }
    }
}

/// Velocity feature quality analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VelocityQualityAnalysis {
    pub total_transactions: usize,
    pub with_velocity: usize,
    pub coverage_rate: f64,
    pub window_ordering_violations: usize,
    pub amount_ordering_violations: usize,
    pub zscore_mean: f64,
    pub zscore_std: f64,
    pub passes: bool,
    pub issues: Vec<String>,
}

/// Velocity feature quality analyzer.
pub struct VelocityQualityAnalyzer {
    pub thresholds: VelocityQualityThresholds,
}

impl VelocityQualityAnalyzer {
    pub fn new() -> Self {
        Self {
            thresholds: VelocityQualityThresholds::default(),
        }
    }

    pub fn with_thresholds(thresholds: VelocityQualityThresholds) -> Self {
        Self { thresholds }
    }

    /// Analyze velocity features.
    ///
    /// `features`: iterator of Option<VelocityFeaturesData> — None means no velocity computed.
    /// `total_transactions`: total number of transactions (including those without velocity).
    pub fn analyze(
        &self,
        features: impl IntoIterator<Item = Option<VelocityFeaturesData>>,
        total_transactions: usize,
    ) -> EvalResult<VelocityQualityAnalysis> {
        let mut with_velocity = 0usize;
        let mut window_violations = 0usize;
        let mut amount_violations = 0usize;
        let mut zscores: Vec<f64> = Vec::new();

        for opt_f in features {
            let Some(f) = opt_f else { continue };
            with_velocity += 1;

            // Window ordering: 1h ≤ 24h ≤ 7d ≤ 30d
            if !(f.txn_count_1h <= f.txn_count_24h
                && f.txn_count_24h <= f.txn_count_7d
                && f.txn_count_7d <= f.txn_count_30d)
            {
                window_violations += 1;
            }

            // Amount sum ordering: 24h ≤ 7d ≤ 30d
            if !(f.amount_sum_24h <= f.amount_sum_7d + 1e-6
                && f.amount_sum_7d <= f.amount_sum_30d + 1e-6)
            {
                amount_violations += 1;
            }

            if f.amount_zscore.is_finite() {
                zscores.push(f.amount_zscore);
            }
        }

        let coverage_rate = if total_transactions > 0 {
            with_velocity as f64 / total_transactions as f64
        } else {
            0.0
        };

        let zscore_mean = if !zscores.is_empty() {
            zscores.iter().sum::<f64>() / zscores.len() as f64
        } else {
            0.0
        };
        let zscore_std = if zscores.len() >= 2 {
            let var = zscores
                .iter()
                .map(|z| (z - zscore_mean).powi(2))
                .sum::<f64>()
                / (zscores.len() as f64 - 1.0);
            var.sqrt()
        } else {
            0.0
        };

        let window_rate = if with_velocity > 0 {
            window_violations as f64 / with_velocity as f64
        } else {
            0.0
        };
        let amount_rate = if with_velocity > 0 {
            amount_violations as f64 / with_velocity as f64
        } else {
            0.0
        };

        let mut issues = Vec::new();
        if coverage_rate < self.thresholds.min_coverage {
            issues.push(format!(
                "Velocity coverage {:.1}% below minimum {:.1}%",
                coverage_rate * 100.0,
                self.thresholds.min_coverage * 100.0,
            ));
        }
        if window_rate > self.thresholds.max_ordering_violation_rate {
            issues.push(format!(
                "{} transactions have window ordering violations ({:.2}%)",
                window_violations,
                window_rate * 100.0,
            ));
        }
        if amount_rate > self.thresholds.max_amount_violation_rate {
            issues.push(format!(
                "{} transactions have amount ordering violations ({:.2}%)",
                amount_violations,
                amount_rate * 100.0,
            ));
        }
        if zscore_mean.abs() > self.thresholds.zscore_mean_tolerance {
            issues.push(format!(
                "Z-score mean {:.3} deviates from expected ≈0",
                zscore_mean,
            ));
        }

        Ok(VelocityQualityAnalysis {
            total_transactions,
            with_velocity,
            coverage_rate,
            window_ordering_violations: window_violations,
            amount_ordering_violations: amount_violations,
            zscore_mean,
            zscore_std,
            passes: issues.is_empty(),
            issues,
        })
    }
}

impl Default for VelocityQualityAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_well_ordered_velocity_passes() {
        let data = vec![
            Some(VelocityFeaturesData {
                txn_count_1h: 1,
                txn_count_24h: 5,
                txn_count_7d: 20,
                txn_count_30d: 80,
                amount_sum_24h: 500.0,
                amount_sum_7d: 2000.0,
                amount_sum_30d: 8000.0,
                amount_zscore: 0.2,
            }),
            Some(VelocityFeaturesData {
                txn_count_1h: 0,
                txn_count_24h: 3,
                txn_count_7d: 15,
                txn_count_30d: 60,
                amount_sum_24h: 200.0,
                amount_sum_7d: 1500.0,
                amount_sum_30d: 6000.0,
                amount_zscore: -0.1,
            }),
        ];
        let analyzer = VelocityQualityAnalyzer::new();
        let result = analyzer.analyze(data, 2).unwrap();
        assert!(result.passes, "Issues: {:?}", result.issues);
        assert_eq!(result.with_velocity, 2);
        assert_eq!(result.window_ordering_violations, 0);
    }

    #[test]
    fn test_window_ordering_violation_detected() {
        let data = vec![Some(VelocityFeaturesData {
            // 24h > 7d — violation!
            txn_count_1h: 1,
            txn_count_24h: 50,
            txn_count_7d: 20,
            txn_count_30d: 80,
            amount_sum_24h: 100.0,
            amount_sum_7d: 200.0,
            amount_sum_30d: 300.0,
            amount_zscore: 0.0,
        })];
        let analyzer = VelocityQualityAnalyzer::new();
        let result = analyzer.analyze(data, 1).unwrap();
        assert!(!result.passes);
        assert_eq!(result.window_ordering_violations, 1);
    }

    #[test]
    fn test_low_coverage_flagged() {
        // 1 with velocity out of 10 total = 10% coverage, fails min_coverage=95%
        let data: Vec<Option<VelocityFeaturesData>> =
            std::iter::once(Some(VelocityFeaturesData::default()))
                .chain(std::iter::repeat_n(None, 9))
                .collect();
        let analyzer = VelocityQualityAnalyzer::new();
        let result = analyzer.analyze(data, 10).unwrap();
        assert!(!result.passes);
        assert!(result.issues.iter().any(|i| i.contains("coverage")));
    }
}

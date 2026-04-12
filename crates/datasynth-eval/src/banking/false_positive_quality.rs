//! False positive quality evaluator.
//!
//! Validates that false positive injections are:
//! - Within the expected rate range (not too few, not too many)
//! - Mutually exclusive with `is_suspicious = true` (an FP is clean ground truth
//!   that looks suspicious)
//! - Accompanied by a non-empty `false_positive_reason`

use serde::{Deserialize, Serialize};

use crate::error::EvalResult;

/// Label data needed for FP quality evaluation.
#[derive(Debug, Clone)]
pub struct LabelData {
    pub is_suspicious: bool,
    pub is_false_positive: bool,
    pub has_fp_reason: bool,
}

/// Thresholds for FP quality.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalsePositiveThresholds {
    /// Minimum FP rate (realistic datasets should have some)
    pub min_fp_rate: f64,
    /// Maximum FP rate (otherwise signal-to-noise ratio breaks)
    pub max_fp_rate: f64,
    /// Maximum allowed overlap between is_suspicious and is_false_positive (should be 0)
    pub max_overlap_rate: f64,
    /// Minimum fraction of FPs with populated reason
    pub min_reason_coverage: f64,
}

impl Default for FalsePositiveThresholds {
    fn default() -> Self {
        Self {
            min_fp_rate: 0.01,
            max_fp_rate: 0.30,
            max_overlap_rate: 0.0,
            min_reason_coverage: 0.95,
        }
    }
}

/// False positive quality analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalsePositiveAnalysis {
    pub total_transactions: usize,
    pub suspicious_count: usize,
    pub false_positive_count: usize,
    pub overlap_count: usize,
    pub missing_reason_count: usize,
    pub fp_rate: f64,
    pub passes: bool,
    pub issues: Vec<String>,
}

pub struct FalsePositiveAnalyzer {
    pub thresholds: FalsePositiveThresholds,
}

impl FalsePositiveAnalyzer {
    pub fn new() -> Self {
        Self {
            thresholds: FalsePositiveThresholds::default(),
        }
    }

    pub fn with_thresholds(thresholds: FalsePositiveThresholds) -> Self {
        Self { thresholds }
    }

    pub fn analyze(&self, labels: &[LabelData]) -> EvalResult<FalsePositiveAnalysis> {
        let total = labels.len();
        let mut suspicious = 0usize;
        let mut fp = 0usize;
        let mut overlap = 0usize;
        let mut missing_reason = 0usize;

        for l in labels {
            if l.is_suspicious {
                suspicious += 1;
            }
            if l.is_false_positive {
                fp += 1;
                if l.is_suspicious {
                    overlap += 1;
                }
                if !l.has_fp_reason {
                    missing_reason += 1;
                }
            }
        }

        let fp_rate = if total > 0 {
            fp as f64 / total as f64
        } else {
            0.0
        };
        let overlap_rate = if total > 0 {
            overlap as f64 / total as f64
        } else {
            0.0
        };
        let reason_coverage = if fp > 0 {
            1.0 - (missing_reason as f64 / fp as f64)
        } else {
            1.0
        };

        let mut issues = Vec::new();
        if total > 0 {
            if fp_rate < self.thresholds.min_fp_rate {
                issues.push(format!(
                    "FP rate {:.2}% below minimum {:.2}% (realistic datasets need false positives)",
                    fp_rate * 100.0,
                    self.thresholds.min_fp_rate * 100.0,
                ));
            }
            if fp_rate > self.thresholds.max_fp_rate {
                issues.push(format!(
                    "FP rate {:.2}% above maximum {:.2}%",
                    fp_rate * 100.0,
                    self.thresholds.max_fp_rate * 100.0,
                ));
            }
        }
        if overlap_rate > self.thresholds.max_overlap_rate {
            issues.push(format!(
                "{overlap} transactions marked both is_suspicious AND is_false_positive (label inconsistency)"
            ));
        }
        if fp > 0 && reason_coverage < self.thresholds.min_reason_coverage {
            issues.push(format!(
                "{missing_reason} of {fp} false positives missing false_positive_reason"
            ));
        }

        Ok(FalsePositiveAnalysis {
            total_transactions: total,
            suspicious_count: suspicious,
            false_positive_count: fp,
            overlap_count: overlap,
            missing_reason_count: missing_reason,
            fp_rate,
            passes: issues.is_empty(),
            issues,
        })
    }
}

impl Default for FalsePositiveAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_fp_passes() {
        let labels: Vec<LabelData> = (0..100)
            .map(|i| LabelData {
                is_suspicious: i < 2,
                is_false_positive: (2..7).contains(&i), // 5% FP
                has_fp_reason: (2..7).contains(&i),
            })
            .collect();
        let analyzer = FalsePositiveAnalyzer::new();
        let result = analyzer.analyze(&labels).unwrap();
        assert!(result.passes, "Issues: {:?}", result.issues);
        assert_eq!(result.false_positive_count, 5);
    }

    #[test]
    fn test_overlap_detected() {
        // A txn marked both suspicious AND false positive — inconsistent
        let labels = vec![LabelData {
            is_suspicious: true,
            is_false_positive: true,
            has_fp_reason: true,
        }];
        let analyzer = FalsePositiveAnalyzer::new();
        let result = analyzer.analyze(&labels).unwrap();
        assert!(!result.passes);
        assert_eq!(result.overlap_count, 1);
    }

    #[test]
    fn test_missing_reason_detected() {
        let labels: Vec<LabelData> = (0..20)
            .map(|i| LabelData {
                is_suspicious: false,
                is_false_positive: i < 2,
                has_fp_reason: false, // no reason populated!
            })
            .collect();
        let analyzer = FalsePositiveAnalyzer::new();
        let result = analyzer.analyze(&labels).unwrap();
        assert!(!result.passes);
        assert!(result.issues.iter().any(|i| i.contains("reason")));
    }
}

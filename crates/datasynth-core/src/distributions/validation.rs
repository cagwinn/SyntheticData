//! Statistical validation runner for generated amount distributions
//! (v3.5.1+).
//!
//! Executes the tests declared in
//! `distributions.validation.tests` (schema-side [`StatisticalTest
//! Config`](../../../../datasynth-config/src/schema.rs)) against a slice
//! of sampled amounts and emits a [`StatisticalValidationReport`]
//! summarising which tests passed, warned, or failed.
//!
//! This module deliberately keeps the surface minimal: the schema already
//! has richer test types (Anderson-Darling, correlation check) that will
//! land in follow-up releases. v3.5.1 implements Benford first-digit,
//! chi-squared goodness-of-fit, and a lightweight Kolmogorov-Smirnov
//! distribution-fit check — enough to catch the most common realism
//! regressions without pulling in a heavyweight stats dependency.

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::benford::get_first_digit;

/// Outcome of a single statistical test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestOutcome {
    /// Test passed all thresholds.
    Passed,
    /// Test passed the hard threshold but exceeded a warning band.
    Warning,
    /// Test failed the hard threshold.
    Failed,
    /// Test was skipped (e.g. too few samples, not-yet-implemented variant).
    Skipped,
}

/// Result of running a single statistical test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatisticalTestResult {
    /// Human-readable test name (e.g. "benford_first_digit").
    pub name: String,
    /// Test outcome.
    pub outcome: TestOutcome,
    /// Key measured statistic (e.g. MAD for Benford, chi-squared value).
    pub statistic: f64,
    /// Threshold compared against (typically the hard-fail threshold).
    pub threshold: f64,
    /// One-line human description of what happened.
    pub message: String,
}

/// Aggregate report covering every test run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatisticalValidationReport {
    /// Number of samples the report was computed over.
    pub sample_count: usize,
    /// Per-test results in input order.
    pub results: Vec<StatisticalTestResult>,
}

impl StatisticalValidationReport {
    /// Did every test pass? (Warnings do not count as failures.)
    pub fn all_passed(&self) -> bool {
        self.results
            .iter()
            .all(|r| !matches!(r.outcome, TestOutcome::Failed))
    }

    /// Is there at least one warning?
    pub fn has_warnings(&self) -> bool {
        self.results
            .iter()
            .any(|r| matches!(r.outcome, TestOutcome::Warning))
    }

    /// Collect all failed test names.
    pub fn failed_names(&self) -> Vec<String> {
        self.results
            .iter()
            .filter(|r| matches!(r.outcome, TestOutcome::Failed))
            .map(|r| r.name.clone())
            .collect()
    }
}

/// Benford first-digit mean-absolute-deviation (MAD) test.
///
/// Returns a [`StatisticalTestResult`] where `statistic` is the MAD and
/// `threshold` is the hard-fail threshold. `Warning` when MAD > warning
/// threshold but <= hard threshold. `Skipped` when fewer than 100
/// positive amounts are available (sample too small for stable MAD).
pub fn run_benford_first_digit(
    amounts: &[Decimal],
    threshold_mad: f64,
    warning_mad: f64,
) -> StatisticalTestResult {
    let mut counts = [0u32; 10]; // index 0 unused; 1..=9 used
    let mut total = 0u32;
    for amount in amounts {
        if let Some(d) = get_first_digit(*amount) {
            counts[d as usize] += 1;
            total += 1;
        }
    }

    if total < 100 {
        return StatisticalTestResult {
            name: "benford_first_digit".to_string(),
            outcome: TestOutcome::Skipped,
            statistic: 0.0,
            threshold: threshold_mad,
            message: format!("only {total} samples with valid first digit; need ≥100"),
        };
    }

    // Expected Benford probability for digit d: log10(1 + 1/d).
    // Index 0 is unused; values for d ∈ {1..=9}.
    const EXPECTED: [f64; 10] = [
        0.0,
        std::f64::consts::LOG10_2, // log10(2)
        0.17609125905568124,       // log10(3/2)
        0.12493873660829995,
        0.09691001300805642,
        0.07918124604762482,
        0.06694678963061322,
        0.057991946977686726,
        0.05115252244738129,
        0.04575749056067514,
    ];

    let total_f = total as f64;
    let mad: f64 = (1..=9)
        .map(|d| (counts[d] as f64 / total_f - EXPECTED[d]).abs())
        .sum::<f64>()
        / 9.0;

    let outcome = if mad > threshold_mad {
        TestOutcome::Failed
    } else if mad > warning_mad {
        TestOutcome::Warning
    } else {
        TestOutcome::Passed
    };

    StatisticalTestResult {
        name: "benford_first_digit".to_string(),
        outcome,
        statistic: mad,
        threshold: threshold_mad,
        message: format!(
            "MAD={mad:.4} over {total} first digits (threshold={threshold_mad:.4}, warn={warning_mad:.4})"
        ),
    }
}

/// Chi-squared goodness-of-fit test against a uniform binning of
/// log-scale amounts.
///
/// This is intentionally lightweight — it checks that amounts aren't
/// concentrated in one log-bin (which would indicate a broken mixture
/// or collapsed distribution). Hard fails when the chi-squared statistic
/// exceeds the critical value at the configured significance.
pub fn run_chi_squared(
    amounts: &[Decimal],
    bins: usize,
    significance: f64,
) -> StatisticalTestResult {
    if amounts.len() < 100 {
        return StatisticalTestResult {
            name: "chi_squared".to_string(),
            outcome: TestOutcome::Skipped,
            statistic: 0.0,
            threshold: 0.0,
            message: format!("only {} samples; need ≥100", amounts.len()),
        };
    }

    let bins = bins.max(2);
    let positives: Vec<f64> = amounts
        .iter()
        .filter_map(|a| a.to_f64())
        .filter(|v| *v > 0.0)
        .collect();
    if positives.len() < 100 {
        return StatisticalTestResult {
            name: "chi_squared".to_string(),
            outcome: TestOutcome::Skipped,
            statistic: 0.0,
            threshold: 0.0,
            message: format!("only {} positive samples; need ≥100", positives.len()),
        };
    }

    let logs: Vec<f64> = positives.iter().map(|v| v.ln()).collect();
    let min = logs.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = logs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if !min.is_finite() || !max.is_finite() || max <= min {
        return StatisticalTestResult {
            name: "chi_squared".to_string(),
            outcome: TestOutcome::Skipped,
            statistic: 0.0,
            threshold: 0.0,
            message: "degenerate log-range".to_string(),
        };
    }

    let bin_width = (max - min) / bins as f64;
    let mut observed = vec![0u32; bins];
    for v in &logs {
        let idx = (((v - min) / bin_width) as usize).min(bins - 1);
        observed[idx] += 1;
    }

    let n = logs.len() as f64;
    let expected_per_bin = n / bins as f64;
    let chi_sq: f64 = observed
        .iter()
        .map(|o| {
            let diff = *o as f64 - expected_per_bin;
            diff * diff / expected_per_bin
        })
        .sum();

    // Approximate chi-squared critical value for df = bins - 1 at the
    // configured significance. We ship hard-coded tables for the common
    // cases (α ∈ {0.01, 0.05, 0.10}, df ∈ {4,5,6,7,8,9,10,14,19,24,29})
    // and fall back to a generous ceiling otherwise.
    let df = bins - 1;
    let critical = chi_sq_critical(df, significance);

    let outcome = if chi_sq > critical {
        TestOutcome::Failed
    } else {
        TestOutcome::Passed
    };

    StatisticalTestResult {
        name: "chi_squared".to_string(),
        outcome,
        statistic: chi_sq,
        threshold: critical,
        message: format!(
            "χ²={chi_sq:.2} over {bins} log-bins ({n} samples), critical={critical:.2} at α={significance}"
        ),
    }
}

/// Kolmogorov-Smirnov goodness-of-fit against a uniform CDF on the
/// log-scale of amounts.
///
/// This is the simplest version — compares the empirical log-amount CDF
/// against a uniform CDF on `[min_log, max_log]`. Useful for detecting
/// grossly skewed outputs; more sophisticated target-distribution fits
/// (Normal/LogNormal/Exponential) ship in v3.5.2.
pub fn run_ks_uniform_log(amounts: &[Decimal], significance: f64) -> StatisticalTestResult {
    let positives: Vec<f64> = amounts
        .iter()
        .filter_map(|a| a.to_f64())
        .filter(|v| *v > 0.0)
        .collect();
    if positives.len() < 100 {
        return StatisticalTestResult {
            name: "ks_uniform_log".to_string(),
            outcome: TestOutcome::Skipped,
            statistic: 0.0,
            threshold: 0.0,
            message: format!("only {} positive samples; need ≥100", positives.len()),
        };
    }

    let mut logs: Vec<f64> = positives.iter().map(|v| v.ln()).collect();
    logs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let min = logs[0];
    let max = logs[logs.len() - 1];
    if max <= min {
        return StatisticalTestResult {
            name: "ks_uniform_log".to_string(),
            outcome: TestOutcome::Skipped,
            statistic: 0.0,
            threshold: 0.0,
            message: "degenerate log-range".to_string(),
        };
    }

    let n = logs.len() as f64;
    let mut max_diff: f64 = 0.0;
    for (i, v) in logs.iter().enumerate() {
        let empirical = (i as f64 + 1.0) / n;
        let uniform = (v - min) / (max - min);
        let diff = (empirical - uniform).abs();
        if diff > max_diff {
            max_diff = diff;
        }
    }

    // Approximate KS critical value at large n (Kolmogorov):
    //   D_α ≈ c(α) / sqrt(n)
    // where c(0.05) ≈ 1.358, c(0.01) ≈ 1.628, c(0.10) ≈ 1.224.
    let c = if significance <= 0.011 {
        1.628
    } else if significance <= 0.051 {
        1.358
    } else {
        1.224
    };
    let critical = c / n.sqrt();

    let outcome = if max_diff > critical {
        TestOutcome::Failed
    } else {
        TestOutcome::Passed
    };

    StatisticalTestResult {
        name: "ks_uniform_log".to_string(),
        outcome,
        statistic: max_diff,
        threshold: critical,
        message: format!(
            "D={max_diff:.4} over {n} samples, critical={critical:.4} at α={significance}"
        ),
    }
}

/// Chi-squared critical values for common (df, α) combinations.
/// Returns a generous upper ceiling for rarely-used df values so the
/// test defaults to passing in ambiguous cases.
fn chi_sq_critical(df: usize, alpha: f64) -> f64 {
    // Rows: (df, α=0.10, α=0.05, α=0.01)
    let table: &[(usize, f64, f64, f64)] = &[
        (1, 2.706, 3.841, 6.635),
        (2, 4.605, 5.991, 9.210),
        (3, 6.251, 7.815, 11.345),
        (4, 7.779, 9.488, 13.277),
        (5, 9.236, 11.070, 15.086),
        (6, 10.645, 12.592, 16.812),
        (7, 12.017, 14.067, 18.475),
        (8, 13.362, 15.507, 20.090),
        (9, 14.684, 16.919, 21.666),
        (10, 15.987, 18.307, 23.209),
        (14, 21.064, 23.685, 29.141),
        (19, 27.204, 30.144, 36.191),
        (24, 33.196, 36.415, 42.980),
        (29, 39.087, 42.557, 49.588),
    ];

    let row = table
        .iter()
        .min_by_key(|(d, _, _, _)| (*d as i64 - df as i64).unsigned_abs());
    if let Some(&(_, c_10, c_05, c_01)) = row {
        if alpha <= 0.011 {
            c_01
        } else if alpha <= 0.051 {
            c_05
        } else {
            c_10
        }
    } else {
        // Very generous fallback — don't fail tests on exotic df values.
        1_000_000.0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use rand_distr::{Distribution, LogNormal};

    fn lognormal_samples(n: usize, mu: f64, sigma: f64, seed: u64) -> Vec<Decimal> {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let ln = LogNormal::new(mu, sigma).unwrap();
        (0..n)
            .map(|_| Decimal::from_f64_retain(ln.sample(&mut rng)).unwrap_or(Decimal::ONE))
            .collect()
    }

    #[test]
    fn benford_passes_for_lognormal() {
        let samples = lognormal_samples(2000, 7.0, 2.0, 42);
        let r = run_benford_first_digit(&samples, 0.015, 0.010);
        assert!(
            !matches!(r.outcome, TestOutcome::Failed),
            "expected pass/warning, got {:?}: {}",
            r.outcome,
            r.message
        );
    }

    #[test]
    fn benford_fails_for_concentrated_single_digit() {
        // All values start with 5 — catastrophic Benford violation.
        let samples: Vec<Decimal> = (0..500).map(|i| Decimal::from(5000 + i)).collect();
        let r = run_benford_first_digit(&samples, 0.015, 0.010);
        assert!(matches!(r.outcome, TestOutcome::Failed));
    }

    #[test]
    fn benford_skipped_below_100_samples() {
        let samples: Vec<Decimal> = (0..50).map(Decimal::from).collect();
        let r = run_benford_first_digit(&samples, 0.015, 0.010);
        assert!(matches!(r.outcome, TestOutcome::Skipped));
    }

    #[test]
    fn chi_squared_passes_for_log_uniform() {
        // chi_squared tests uniformity on log scale. Feed it data that is
        // uniform-on-log (i.e. log-uniform) to get the expected pass.
        // A log-normal would — correctly — fail uniformity.
        let samples: Vec<Decimal> = (0..1000)
            .map(|i| {
                // Evenly-spaced log values → exactly uniform on log scale.
                let log_val = (i as f64 / 1000.0) * 10.0;
                let v = log_val.exp();
                Decimal::from_f64_retain(v).unwrap_or(Decimal::ONE)
            })
            .collect();
        let r = run_chi_squared(&samples, 10, 0.05);
        assert!(
            !matches!(r.outcome, TestOutcome::Failed),
            "expected pass, got {:?}: {}",
            r.outcome,
            r.message
        );
    }

    #[test]
    fn chi_squared_fails_for_bimodal_concentration() {
        // Bimodal: 450 small values, 50 huge values. Every mid bin empty.
        // Chi-squared against a uniform expectation will fail hard.
        let mut samples: Vec<Decimal> = (0..450).map(|_| Decimal::from(1000)).collect();
        samples.extend((0..50).map(|_| Decimal::from(1_000_000)));
        let r = run_chi_squared(&samples, 10, 0.05);
        assert!(
            matches!(r.outcome, TestOutcome::Failed),
            "expected Failed for bimodal, got {:?}: {}",
            r.outcome,
            r.message
        );
    }

    #[test]
    fn report_all_passed_tracks_failures() {
        let rep = StatisticalValidationReport {
            sample_count: 100,
            results: vec![
                StatisticalTestResult {
                    name: "a".into(),
                    outcome: TestOutcome::Passed,
                    statistic: 0.0,
                    threshold: 1.0,
                    message: "".into(),
                },
                StatisticalTestResult {
                    name: "b".into(),
                    outcome: TestOutcome::Warning,
                    statistic: 0.0,
                    threshold: 1.0,
                    message: "".into(),
                },
            ],
        };
        assert!(rep.all_passed()); // warnings don't count
        assert!(rep.has_warnings());

        let rep_failed = StatisticalValidationReport {
            sample_count: 100,
            results: vec![StatisticalTestResult {
                name: "c".into(),
                outcome: TestOutcome::Failed,
                statistic: 2.0,
                threshold: 1.0,
                message: "".into(),
            }],
        };
        assert!(!rep_failed.all_passed());
        assert_eq!(rep_failed.failed_names(), vec!["c".to_string()]);
    }
}

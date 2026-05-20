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

/// Spearman rank correlation between two equal-length samples.
///
/// Returns a value in `[-1, 1]`. Used by [`run_correlation_check`]
/// and `run_expected_correlations`.
pub fn spearman_rank_correlation(xs: &[f64], ys: &[f64]) -> f64 {
    let n = xs.len().min(ys.len());
    if n < 2 {
        return 0.0;
    }
    let rank = |v: &[f64]| -> Vec<f64> {
        let mut idx: Vec<usize> = (0..v.len()).collect();
        idx.sort_by(|&a, &b| v[a].partial_cmp(&v[b]).unwrap_or(std::cmp::Ordering::Equal));
        let mut ranks = vec![0.0; v.len()];
        // Handle ties via average-rank. Walk the sorted idx in groups.
        let mut i = 0;
        while i < idx.len() {
            let mut j = i;
            while j + 1 < idx.len() && v[idx[j + 1]] == v[idx[i]] {
                j += 1;
            }
            // Positions i..=j all tied; average rank is (i + j) / 2 + 1.
            let avg = (i + j) as f64 / 2.0 + 1.0;
            for k in i..=j {
                ranks[idx[k]] = avg;
            }
            i = j + 1;
        }
        ranks
    };
    let rx = rank(&xs[..n]);
    let ry = rank(&ys[..n]);
    let mean_x = rx.iter().sum::<f64>() / n as f64;
    let mean_y = ry.iter().sum::<f64>() / n as f64;
    let mut num = 0.0;
    let mut den_x = 0.0;
    let mut den_y = 0.0;
    for i in 0..n {
        let dx = rx[i] - mean_x;
        let dy = ry[i] - mean_y;
        num += dx * dy;
        den_x += dx * dx;
        den_y += dy * dy;
    }
    let denom = (den_x * den_y).sqrt();
    if denom == 0.0 {
        0.0
    } else {
        num / denom
    }
}

/// Correlation check: assert that the empirical Spearman correlation
/// between two paired samples falls within `±tolerance` of the
/// expected value. Used by the v3.5.1 statistical-validation phase to
/// honour `distributions.correlations.expected_correlations`.
pub fn run_correlation_check(
    name: &str,
    xs: &[f64],
    ys: &[f64],
    expected: f64,
    tolerance: f64,
) -> StatisticalTestResult {
    if xs.len().min(ys.len()) < 100 {
        return StatisticalTestResult {
            name: format!("correlation_check_{name}"),
            outcome: TestOutcome::Skipped,
            statistic: 0.0,
            threshold: tolerance,
            message: format!("only {} paired samples; need ≥100", xs.len().min(ys.len())),
        };
    }
    let rho = spearman_rank_correlation(xs, ys);
    let diff = (rho - expected).abs();
    let outcome = if diff > tolerance {
        TestOutcome::Failed
    } else {
        TestOutcome::Passed
    };
    StatisticalTestResult {
        name: format!("correlation_check_{name}"),
        outcome,
        statistic: rho,
        threshold: tolerance,
        message: format!(
            "Spearman ρ={rho:.4} (expected {expected:.4} ±{tolerance:.4}; diff {diff:.4})"
        ),
    }
}

/// Anderson-Darling test for log-normality on the log-scale.
///
/// Applies the classical A² statistic to the standardised log-amounts
/// and compares against the critical value at the configured
/// significance. Log-normal fit ≡ Gaussian fit on `ln(amount)`.
pub fn run_anderson_darling(amounts: &[Decimal], significance: f64) -> StatisticalTestResult {
    let positives: Vec<f64> = amounts
        .iter()
        .filter_map(|a| a.to_f64())
        .filter(|v| *v > 0.0)
        .collect();
    if positives.len() < 100 {
        return StatisticalTestResult {
            name: "anderson_darling".to_string(),
            outcome: TestOutcome::Skipped,
            statistic: 0.0,
            threshold: 0.0,
            message: format!("only {} positive samples; need ≥100", positives.len()),
        };
    }
    let mut logs: Vec<f64> = positives.iter().map(|v| v.ln()).collect();
    let n = logs.len() as f64;
    let mean = logs.iter().sum::<f64>() / n;
    let var = logs.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let sd = var.sqrt();
    if sd == 0.0 {
        return StatisticalTestResult {
            name: "anderson_darling".to_string(),
            outcome: TestOutcome::Skipped,
            statistic: 0.0,
            threshold: 0.0,
            message: "zero log-variance (degenerate input)".to_string(),
        };
    }
    logs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // Standardise + CDF-map via standard normal.
    let standard_normal_cdf = |x: f64| -> f64 { 0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2)) };
    let mut s = 0.0;
    for (i, v) in logs.iter().enumerate() {
        let z = (v - mean) / sd;
        let p = standard_normal_cdf(z).clamp(1e-12, 1.0 - 1e-12);
        let q = 1.0
            - standard_normal_cdf((logs[logs.len() - 1 - i] - mean) / sd).clamp(1e-12, 1.0 - 1e-12);
        s += (2.0 * (i + 1) as f64 - 1.0) * (p.ln() + q.ln());
    }
    let a_sq = -n - s / n;
    // Case-corrected A² for mean+sd both estimated: A*² = A²(1 + 0.75/n + 2.25/n²).
    let a_sq_star = a_sq * (1.0 + 0.75 / n + 2.25 / n.powi(2));
    // Critical values for mean+sd-estimated A* at common α (D'Agostino & Stephens).
    let critical = if significance <= 0.011 {
        1.035
    } else if significance <= 0.026 {
        0.873
    } else if significance <= 0.051 {
        0.752
    } else if significance <= 0.101 {
        0.631
    } else {
        0.500
    };
    let outcome = if a_sq_star > critical {
        TestOutcome::Failed
    } else {
        TestOutcome::Passed
    };
    StatisticalTestResult {
        name: "anderson_darling".to_string(),
        outcome,
        statistic: a_sq_star,
        threshold: critical,
        message: format!(
            "A*²={a_sq_star:.4} vs log-normal, critical={critical:.4} at α={significance} (n={n})"
        ),
    }
}

/// Error function approximation for the standard normal CDF.
/// Accurate to ~1e-7 (Abramowitz & Stegun 7.1.26).
fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0
        - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t
            * (-x * x).exp();
    sign * y
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
    fn spearman_rho_perfect_positive() {
        let xs: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let ys: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let rho = spearman_rank_correlation(&xs, &ys);
        assert!((rho - 1.0).abs() < 1e-6, "expected ρ=1.0, got {rho}");
    }

    #[test]
    fn spearman_rho_perfect_negative() {
        let xs: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let ys: Vec<f64> = (1..=100).rev().map(|i| i as f64).collect();
        let rho = spearman_rank_correlation(&xs, &ys);
        assert!((rho + 1.0).abs() < 1e-6, "expected ρ=-1.0, got {rho}");
    }

    #[test]
    fn correlation_check_passes_when_within_tolerance() {
        let xs: Vec<f64> = (1..=200).map(|i| i as f64).collect();
        let ys: Vec<f64> = xs.iter().map(|v| v + 0.5).collect();
        let r = run_correlation_check("test", &xs, &ys, 1.0, 0.05);
        assert!(matches!(r.outcome, TestOutcome::Passed));
    }

    #[test]
    fn correlation_check_fails_when_off_target() {
        let xs: Vec<f64> = (1..=200).map(|i| i as f64).collect();
        let ys: Vec<f64> = xs.iter().rev().copied().collect();
        let r = run_correlation_check("test", &xs, &ys, 1.0, 0.05);
        assert!(matches!(r.outcome, TestOutcome::Failed));
    }

    #[test]
    fn anderson_darling_passes_for_lognormal() {
        let samples = lognormal_samples(2000, 7.0, 1.5, 42);
        let r = run_anderson_darling(&samples, 0.05);
        assert!(
            !matches!(r.outcome, TestOutcome::Failed),
            "expected pass/warning for log-normal, got {:?}: {}",
            r.outcome,
            r.message
        );
    }

    #[test]
    fn anderson_darling_fails_for_uniform() {
        // Uniform data is very unlike log-normal.
        let samples: Vec<Decimal> = (0..2000)
            .map(|i| Decimal::from(1000 + (i % 500) * 20))
            .collect();
        let r = run_anderson_darling(&samples, 0.05);
        assert!(
            matches!(r.outcome, TestOutcome::Failed),
            "expected fail for uniform-like data, got {:?}: {}",
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

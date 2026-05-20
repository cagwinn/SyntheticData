//! Numerical primitives shared across the behavioral-fidelity metrics.

use std::cmp::Ordering;

use chrono::{Datelike, NaiveDate, Weekday};

/// Wasserstein-1 distance between two empirical 1-D samples.
///
/// Implementation: integrate |F_a^{-1}(t) - F_b^{-1}(t)| over t ∈ \[0,1\]
/// on a uniform grid of `quantile_steps` knots. For equal-length sorted
/// samples this reduces to the mean L¹ distance, which we use directly
/// as a fast path. Quantile-step default of 1024 keeps error < 1e-6 for
/// practical distributions.
pub fn wasserstein_1(a: &[f64], b: &[f64]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut sa: Vec<f64> = a.iter().copied().filter(|x| x.is_finite()).collect();
    let mut sb: Vec<f64> = b.iter().copied().filter(|x| x.is_finite()).collect();
    sa.sort_by(|x, y| x.partial_cmp(y).unwrap_or(Ordering::Equal));
    sb.sort_by(|x, y| x.partial_cmp(y).unwrap_or(Ordering::Equal));
    if sa.len() == sb.len() {
        return sa
            .iter()
            .zip(sb.iter())
            .map(|(x, y)| (x - y).abs())
            .sum::<f64>()
            / sa.len() as f64;
    }
    const STEPS: usize = 1024;
    let mut acc = 0.0;
    for k in 0..STEPS {
        let t = (k as f64 + 0.5) / STEPS as f64;
        let qa = quantile_sorted(&sa, t);
        let qb = quantile_sorted(&sb, t);
        acc += (qa - qb).abs();
    }
    acc / STEPS as f64
}

fn quantile_sorted(sorted: &[f64], t: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = t * (sorted.len() as f64 - 1.0);
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let frac = pos - lo as f64;
        sorted[lo] * (1.0 - frac) + sorted[hi] * frac
    }
}

/// Lag-1 Pearson correlation between consecutive elements of `xs`.
///
/// Returns `None` if `xs.len() < 3` or if either of the two shifted
/// series has zero variance.
pub fn pearson_lag1_correlation(xs: &[f64]) -> Option<f64> {
    if xs.len() < 3 {
        return None;
    }
    let a = &xs[..xs.len() - 1];
    let b = &xs[1..];
    let n = a.len() as f64;
    let mean_a = a.iter().sum::<f64>() / n;
    let mean_b = b.iter().sum::<f64>() / n;
    let mut num = 0.0;
    let mut da = 0.0;
    let mut db = 0.0;
    for i in 0..a.len() {
        let xa = a[i] - mean_a;
        let xb = b[i] - mean_b;
        num += xa * xb;
        da += xa * xa;
        db += xb * xb;
    }
    if da == 0.0 || db == 0.0 {
        return None;
    }
    Some(num / (da.sqrt() * db.sqrt()))
}

/// Empirical percentile of an unsorted slice (clones + sorts).
pub fn percentile(xs: &[f64], pct: f64) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut s: Vec<f64> = xs.iter().copied().filter(|x| x.is_finite()).collect();
    s.sort_by(|x, y| x.partial_cmp(y).unwrap_or(Ordering::Equal));
    quantile_sorted(&s, pct.clamp(0.0, 1.0))
}

/// Days between two dates, can be negative.
pub fn days_between(a: NaiveDate, b: NaiveDate) -> i64 {
    (b - a).num_days()
}

/// `true` if the date falls on Sat or Sun.
pub fn is_weekend(d: NaiveDate) -> bool {
    matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn w1_identical_samples_is_zero() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let b = a.clone();
        assert!((wasserstein_1(&a, &b)).abs() < 1e-9);
    }

    #[test]
    fn w1_shifted_samples_equals_shift() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let b: Vec<f64> = a.iter().map(|x| x + 3.0).collect();
        assert!((wasserstein_1(&a, &b) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn w1_unequal_lengths_handles_gracefully() {
        let a = vec![1.0; 10];
        let b = vec![2.0; 100];
        let d = wasserstein_1(&a, &b);
        assert!((d - 1.0).abs() < 1e-3);
    }

    #[test]
    fn pearson_lag1_positive_autocorr() {
        let xs = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let r = pearson_lag1_correlation(&xs).unwrap();
        assert!((r - 1.0).abs() < 1e-9);
    }

    #[test]
    fn pearson_lag1_negative_autocorr() {
        let xs = vec![1.0, 10.0, 1.0, 10.0, 1.0, 10.0];
        let r = pearson_lag1_correlation(&xs).unwrap();
        assert!(r < -0.9);
    }

    #[test]
    fn pearson_lag1_short_series_returns_none() {
        let xs = vec![1.0, 2.0];
        assert!(pearson_lag1_correlation(&xs).is_none());
    }

    #[test]
    fn percentile_known_values() {
        let xs: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let p50 = percentile(&xs, 0.50);
        assert!((p50 - 50.5).abs() < 1.0);
        let p90 = percentile(&xs, 0.90);
        assert!((p90 - 90.0).abs() < 1.0);
    }

    #[test]
    fn days_between_known() {
        let a = NaiveDate::from_ymd_opt(2022, 4, 25).unwrap();
        let b = NaiveDate::from_ymd_opt(2022, 5, 2).unwrap();
        assert_eq!(days_between(a, b), 7);
    }

    #[test]
    fn is_weekend_known() {
        // 2022-04-30 is a Saturday
        assert!(is_weekend(NaiveDate::from_ymd_opt(2022, 4, 30).unwrap()));
        // 2022-04-25 is a Monday
        assert!(!is_weekend(NaiveDate::from_ymd_opt(2022, 4, 25).unwrap()));
    }
}

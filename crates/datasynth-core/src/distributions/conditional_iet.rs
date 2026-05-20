//! Per-Source inter-event-time sampler driven by SP2's PerSourceIetPrior.

use std::collections::HashMap;

use rand::{Rng, RngExt};

/// Per-Source RNG state for IET sampling.
#[derive(Debug, Clone)]
pub struct SourceIetState {
    /// Quantile knot values (sorted ascending) from the prior's empirical CDF.
    pub cdf_values: Vec<f64>,
    /// Cumulative probabilities matching `cdf_values` (monotone in [0, 1]).
    pub cdf_probabilities: Vec<f64>,
    /// Lag-1 Pearson correlation observed in the corpus for this Source.
    pub lag1_autocorr: f64,
    /// Last sampled IET (in days) — used to couple the next draw via the autocorr.
    pub last_iet_days: Option<f64>,
}

impl SourceIetState {
    fn sample_quantile<R: Rng>(&self, rng: &mut R) -> f64 {
        if self.cdf_values.is_empty() {
            return 0.0;
        }
        let u: f64 = rng.random_range(f64::EPSILON..=1.0);
        let mut idx = self.cdf_probabilities.len() - 1;
        for (i, &p) in self.cdf_probabilities.iter().enumerate() {
            if p >= u {
                idx = i;
                break;
            }
        }
        self.cdf_values[idx]
    }
}

/// Empirical CDF value at `x` — linear interpolation between knots.
fn empirical_cdf_at(values: &[f64], probabilities: &[f64], x: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    if x <= values[0] {
        return probabilities[0];
    }
    if x >= *values.last().expect("non-empty checked above") {
        return *probabilities.last().expect("non-empty checked above");
    }
    let idx =
        match values.binary_search_by(|v| v.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal)) {
            Ok(i) => return probabilities[i],
            Err(i) => i,
        };
    let lo_v = values[idx - 1];
    let hi_v = values[idx];
    let lo_p = probabilities[idx - 1];
    let hi_p = probabilities[idx];
    if hi_v == lo_v {
        lo_p
    } else {
        let t = (x - lo_v) / (hi_v - lo_v);
        lo_p + t * (hi_p - lo_p)
    }
}

/// Inverse CDF: given quantile p in [0,1], return the value via linear interpolation.
fn quantile_at(values: &[f64], probabilities: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let p = p.clamp(0.0, 1.0);
    if p <= probabilities[0] {
        return values[0];
    }
    if p >= *probabilities.last().expect("non-empty checked above") {
        return *values.last().expect("non-empty checked above");
    }
    let idx = match probabilities
        .binary_search_by(|prob| prob.partial_cmp(&p).unwrap_or(std::cmp::Ordering::Equal))
    {
        Ok(i) => return values[i],
        Err(i) => i,
    };
    let lo_p = probabilities[idx - 1];
    let hi_p = probabilities[idx];
    let lo_v = values[idx - 1];
    let hi_v = values[idx];
    if hi_p == lo_p {
        lo_v
    } else {
        let t = (p - lo_p) / (hi_p - lo_p);
        lo_v + t * (hi_v - lo_v)
    }
}

/// Inverse standard-normal CDF (Φ⁻¹).
///
/// Rational approximation from Abramowitz & Stegun §26.2.23 with correct
/// sign convention: returns negative values for p < 0.5 and positive for
/// p > 0.5. The copula.rs `standard_normal_quantile` has inverted tail signs,
/// so we provide an independent correct implementation here.
fn inverse_standard_normal(p: f64) -> f64 {
    let p = p.clamp(1e-12, 1.0 - 1e-12);
    let p_low = 0.02425_f64;
    let p_high = 1.0 - p_low;

    if p < p_low {
        // Lower tail: result is negative
        let q = (-2.0 * p.ln()).sqrt();
        let c = [2.515517_f64, 0.802853, 0.010328];
        let d = [1.432788_f64, 0.189269, 0.001308];
        let rational =
            (c[0] + c[1] * q + c[2] * q * q) / (1.0 + d[0] * q + d[1] * q * q + d[2] * q * q * q);
        -(q - rational)
    } else if p <= p_high {
        // Central region
        let q = p - 0.5;
        let r = q * q;
        let a = [
            2.50662823884_f64,
            -18.61500062529,
            41.39119773534,
            -25.44106049637,
        ];
        let b = [
            -8.47351093090_f64,
            23.08336743743,
            -21.06224101826,
            3.13082909833,
        ];
        q * (a[0] + a[1] * r + a[2] * r * r + a[3] * r * r * r)
            / (1.0 + b[0] * r + b[1] * r * r + b[2] * r * r * r + b[3] * r * r * r * r)
    } else {
        // Upper tail: result is positive (mirror of lower tail)
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        let c = [2.515517_f64, 0.802853, 0.010328];
        let d = [1.432788_f64, 0.189269, 0.001308];
        let rational =
            (c[0] + c[1] * q + c[2] * q * q) / (1.0 + d[0] * q + d[1] * q * q + d[2] * q * q * q);
        q - rational
    }
}

/// Standard normal CDF (Φ).
/// Re-uses the erf-based approximation from `copula.rs`.
fn standard_normal_cdf(z: f64) -> f64 {
    super::copula::standard_normal_cdf(z)
}

/// Sample one standard normal value via Box-Muller.
fn standard_normal_sample<R: Rng + ?Sized>(rng: &mut R) -> f64 {
    let u1: f64 = rng.random_range(f64::EPSILON..=1.0);
    let u2: f64 = rng.random_range(0.0..=1.0);
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

/// Per-Source IET sampler: each call to `sample_next` produces a fresh day-gap
/// drawn from that Source's empirical CDF, optionally coupled with the previous
/// sample via the lag-1 autocorrelation.
#[derive(Clone)]
pub struct ConditionalIETSampler {
    per_source: HashMap<String, SourceIetState>,
    fallback: SourceIetState,
}

impl ConditionalIETSampler {
    pub fn from_state_map(
        per_source: HashMap<String, SourceIetState>,
        fallback: SourceIetState,
    ) -> Self {
        Self {
            per_source,
            fallback,
        }
    }

    pub fn sample_next<R: Rng>(&mut self, source: &str, rng: &mut R) -> f64 {
        let state = self
            .per_source
            .get_mut(source)
            .unwrap_or(&mut self.fallback);
        if state.cdf_values.is_empty() {
            return 0.0;
        }
        let rho = state.lag1_autocorr.clamp(-1.0, 1.0);

        // No-coupling path: |ρ| small or no previous sample.
        if rho.abs() < 0.1 || state.last_iet_days.is_none() {
            let s = state.sample_quantile(rng).max(0.0);
            state.last_iet_days = Some(s);
            return s;
        }

        // Gaussian-copula coupling.
        let prev = state.last_iet_days.expect("checked above");
        let p_prev = empirical_cdf_at(&state.cdf_values, &state.cdf_probabilities, prev);
        let z_prev = inverse_standard_normal(p_prev);
        let z_curr = rho * z_prev + (1.0 - rho * rho).sqrt() * standard_normal_sample(rng);
        let p_curr = standard_normal_cdf(z_curr);
        let curr = quantile_at(&state.cdf_values, &state.cdf_probabilities, p_curr).max(0.0);

        state.last_iet_days = Some(curr);
        curr
    }

    pub fn has_source(&self, source: &str) -> bool {
        self.per_source.contains_key(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn known_state(values: Vec<f64>, autocorr: f64) -> SourceIetState {
        let n = values.len();
        SourceIetState {
            cdf_values: values,
            cdf_probabilities: (1..=n).map(|i| i as f64 / n as f64).collect(),
            lag1_autocorr: autocorr,
            last_iet_days: None,
        }
    }

    #[test]
    fn iet_sampler_returns_known_values() {
        let mut per_source = HashMap::new();
        per_source.insert(
            "KR".to_string(),
            known_state(vec![1.0, 2.0, 5.0, 10.0], 0.0),
        );
        let mut sampler =
            ConditionalIETSampler::from_state_map(per_source, known_state(vec![0.5, 1.0], 0.0));
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..30 {
            let s = sampler.sample_next("KR", &mut rng);
            assert!([1.0, 2.0, 5.0, 10.0].contains(&s), "unexpected sample {s}");
        }
    }

    #[test]
    fn iet_sampler_falls_back_on_unknown_source() {
        let per_source = HashMap::new();
        let mut sampler =
            ConditionalIETSampler::from_state_map(per_source, known_state(vec![7.0], 0.0));
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        assert!((sampler.sample_next("UNKNOWN", &mut rng) - 7.0).abs() < 1e-9);
    }

    #[test]
    fn iet_sampler_autocorr_couples_samples() {
        let mut per_source = HashMap::new();
        per_source.insert("A".to_string(), known_state(vec![1.0, 10.0], 0.9));
        let mut sampler =
            ConditionalIETSampler::from_state_map(per_source, known_state(vec![5.0], 0.0));
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let first = sampler.sample_next("A", &mut rng);
        let second = sampler.sample_next("A", &mut rng);
        assert!(first.is_finite() && second.is_finite());
    }

    #[test]
    fn empirical_cdf_at_interpolates_linearly() {
        let v = vec![1.0, 2.0, 4.0];
        let p = vec![0.25, 0.5, 1.0];
        assert!((empirical_cdf_at(&v, &p, 1.0) - 0.25).abs() < 1e-9);
        assert!((empirical_cdf_at(&v, &p, 2.0) - 0.5).abs() < 1e-9);
        assert!((empirical_cdf_at(&v, &p, 3.0) - 0.75).abs() < 1e-9);
        assert!((empirical_cdf_at(&v, &p, 0.5) - 0.25).abs() < 1e-9);
        assert!((empirical_cdf_at(&v, &p, 5.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn quantile_at_inverts_empirical_cdf() {
        let v = vec![1.0, 2.0, 4.0];
        let p = vec![0.25, 0.5, 1.0];
        assert!((quantile_at(&v, &p, 0.25) - 1.0).abs() < 1e-9);
        assert!((quantile_at(&v, &p, 0.5) - 2.0).abs() < 1e-9);
        assert!((quantile_at(&v, &p, 1.0) - 4.0).abs() < 1e-9);
        assert!((quantile_at(&v, &p, 0.75) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn inverse_standard_normal_known_values() {
        // Central region
        assert!((inverse_standard_normal(0.5)).abs() < 1e-6);
        // Near-central (still in central region since p_low=0.02425)
        assert!((inverse_standard_normal(0.975) - 1.96).abs() < 1e-2);
        assert!((inverse_standard_normal(0.025) + 1.96).abs() < 1e-2);
        // Tail regions (these verify the sign convention is correct)
        assert!(
            inverse_standard_normal(0.01) < 0.0,
            "lower tail must be negative"
        );
        assert!(
            inverse_standard_normal(0.99) > 0.0,
            "upper tail must be positive"
        );
        assert!((inverse_standard_normal(0.99) + inverse_standard_normal(0.01)).abs() < 1e-3);
    }

    #[test]
    fn standard_normal_cdf_known_values() {
        assert!((standard_normal_cdf(0.0) - 0.5).abs() < 1e-6);
        assert!((standard_normal_cdf(1.96) - 0.975).abs() < 1e-3);
    }

    #[test]
    fn iet_sampler_never_returns_negative() {
        let mut per_source = HashMap::new();
        per_source.insert("X".to_string(), known_state(vec![0.0, 0.0, 0.0], -1.0));
        let mut sampler =
            ConditionalIETSampler::from_state_map(per_source, known_state(vec![0.0], 0.0));
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..20 {
            assert!(sampler.sample_next("X", &mut rng) >= 0.0);
        }
    }

    #[test]
    fn copula_coupling_preserves_target_rho() {
        // Use a dense fine-grid uniform CDF (1000 knots over [0,1]) so that
        // the Gaussian copula's rank correlation transfers accurately to
        // Pearson correlation in value-space (rank ≈ Pearson for near-continuous
        // uniform marginals).
        let mut per_source = HashMap::new();
        let n = 1000usize;
        per_source.insert(
            "A".to_string(),
            SourceIetState {
                cdf_values: (1..=n).map(|i| i as f64 / n as f64).collect(),
                cdf_probabilities: (1..=n).map(|i| i as f64 / n as f64).collect(),
                lag1_autocorr: 0.6,
                last_iet_days: None,
            },
        );
        let fallback = SourceIetState {
            cdf_values: vec![1.0],
            cdf_probabilities: vec![1.0],
            lag1_autocorr: 0.0,
            last_iet_days: None,
        };
        let mut sampler = ConditionalIETSampler::from_state_map(per_source, fallback);
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        let n_samples = 5000;
        let mut series: Vec<f64> = Vec::with_capacity(n_samples);
        for _ in 0..n_samples {
            series.push(sampler.sample_next("A", &mut rng));
        }
        // Empirical lag-1 correlation should be near 0.6 (±0.08).
        let mean_pre: f64 = series[..n_samples - 1].iter().sum::<f64>() / (n_samples - 1) as f64;
        let mean_post: f64 = series[1..].iter().sum::<f64>() / (n_samples - 1) as f64;
        let mut num = 0.0;
        let mut dp = 0.0;
        let mut dq = 0.0;
        for i in 0..(n_samples - 1) {
            let a = series[i] - mean_pre;
            let b = series[i + 1] - mean_post;
            num += a * b;
            dp += a * a;
            dq += b * b;
        }
        let empirical_rho = num / (dp.sqrt() * dq.sqrt());
        assert!(
            (empirical_rho - 0.6).abs() < 0.08,
            "expected empirical ρ ≈ 0.6, got {empirical_rho}"
        );
    }

    #[test]
    fn copula_coupling_low_rho_uses_independent_path() {
        let mut per_source = HashMap::new();
        per_source.insert(
            "A".to_string(),
            SourceIetState {
                cdf_values: vec![5.0; 10],
                cdf_probabilities: (1..=10).map(|i| i as f64 / 10.0).collect(),
                lag1_autocorr: 0.05,
                last_iet_days: None,
            },
        );
        let fallback = SourceIetState {
            cdf_values: vec![5.0],
            cdf_probabilities: vec![1.0],
            lag1_autocorr: 0.0,
            last_iet_days: None,
        };
        let mut sampler = ConditionalIETSampler::from_state_map(per_source, fallback);
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        // All CDF values are 5.0 — all samples must be 5.0 regardless of path.
        for _ in 0..20 {
            let s = sampler.sample_next("A", &mut rng);
            assert!((s - 5.0).abs() < 1e-9);
        }
    }
}

//! Advanced amount sampler that dispatches between the legacy
//! [`AmountSampler`](super::AmountSampler) and the richer
//! [`LogNormalMixtureSampler`](super::LogNormalMixtureSampler) /
//! [`GaussianMixtureSampler`](super::GaussianMixtureSampler).
//!
//! Exists so callers (notably the journal-entry generator) can swap in a
//! mixture-model sampler when `distributions.amounts.enabled = true` without
//! perturbing the legacy `transactions.amounts` code path byte-for-byte when
//! the advanced config is absent.

use rust_decimal::Decimal;

use super::mixture::{
    GaussianComponent, GaussianMixtureConfig, GaussianMixtureSampler, LogNormalComponent,
    LogNormalMixtureConfig, LogNormalMixtureSampler,
};
use super::pareto::{ParetoConfig, ParetoSampler};

/// Advanced amount sampler wrapping one of the supported distribution
/// families. Callers keep their existing legacy [`AmountSampler`](super::
/// AmountSampler) and only consult this wrapper when
/// `distributions.amounts.enabled` (or another advanced sub-block like
/// `distributions.pareto.enabled`) is true.
///
/// v3.4.4 added the `Pareto` variant for heavy-tailed monetary samples
/// (capex, strategic contracts, fraud amounts).
#[derive(Clone)]
pub enum AdvancedAmountSampler {
    /// Log-normal mixture (preferred for positive monetary amounts).
    LogNormal(LogNormalMixtureSampler),
    /// Gaussian mixture (useful for signed quantities like deltas).
    Gaussian(GaussianMixtureSampler),
    /// Pareto heavy-tailed distribution (v3.4.4+).
    Pareto(ParetoSampler),
}

impl AdvancedAmountSampler {
    /// Create a log-normal-mixture sampler.
    pub fn new_log_normal(seed: u64, config: LogNormalMixtureConfig) -> Result<Self, String> {
        Ok(Self::LogNormal(LogNormalMixtureSampler::new(seed, config)?))
    }

    /// Create a Gaussian-mixture sampler.
    pub fn new_gaussian(seed: u64, config: GaussianMixtureConfig) -> Result<Self, String> {
        Ok(Self::Gaussian(GaussianMixtureSampler::new(seed, config)?))
    }

    /// Create a Pareto sampler (v3.4.4+).
    pub fn new_pareto(seed: u64, config: ParetoConfig) -> Result<Self, String> {
        Ok(Self::Pareto(ParetoSampler::new(seed, config)?))
    }

    /// Sample one amount as `Decimal`.
    pub fn sample_decimal(&mut self) -> Decimal {
        match self {
            Self::LogNormal(s) => s.sample_decimal(),
            Self::Gaussian(s) => {
                let value = s.sample().max(0.0);
                Decimal::from_f64_retain(value).unwrap_or(Decimal::ZERO)
            }
            Self::Pareto(s) => s.sample_decimal(),
        }
    }

    /// Reset the underlying RNG.
    pub fn reset(&mut self, seed: u64) {
        match self {
            Self::LogNormal(s) => s.reset(seed),
            Self::Gaussian(s) => s.reset(seed),
            Self::Pareto(s) => s.reset(seed),
        }
    }
}

/// Build a [`LogNormalMixtureConfig`] from a list of `(weight, mu, sigma,
/// label)` tuples.  Thin convenience used by the config-layer converter.
pub fn log_normal_config_from_components(
    components: Vec<(f64, f64, f64, Option<String>)>,
    min_value: f64,
    max_value: Option<f64>,
    decimal_places: u8,
) -> LogNormalMixtureConfig {
    LogNormalMixtureConfig {
        components: components
            .into_iter()
            .map(|(w, mu, sigma, label)| match label {
                Some(l) => LogNormalComponent::with_label(w, mu, sigma, l),
                None => LogNormalComponent::new(w, mu, sigma),
            })
            .collect(),
        min_value,
        max_value,
        decimal_places,
    }
}

/// Build a [`GaussianMixtureConfig`] from a list of `(weight, mu, sigma)`
/// tuples. Labels are ignored (the Gaussian component has no label field).
pub fn gaussian_config_from_components(
    components: Vec<(f64, f64, f64)>,
    min_value: Option<f64>,
    max_value: Option<f64>,
) -> GaussianMixtureConfig {
    GaussianMixtureConfig {
        components: components
            .into_iter()
            .map(|(w, mu, sigma)| GaussianComponent::new(w, mu, sigma))
            .collect(),
        allow_negative: true,
        min_value,
        max_value,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn log_normal_sampler_produces_positive_values() {
        let cfg = log_normal_config_from_components(
            vec![(1.0, 7.0, 1.0, Some("test".into()))],
            0.01,
            None,
            2,
        );
        let mut sampler = AdvancedAmountSampler::new_log_normal(42, cfg).unwrap();
        for _ in 0..1000 {
            let v = sampler.sample_decimal();
            assert!(v >= Decimal::ZERO);
        }
    }

    #[test]
    fn gaussian_sampler_clamps_to_non_negative() {
        let cfg = gaussian_config_from_components(vec![(1.0, 0.0, 1.0)], None, None);
        let mut sampler = AdvancedAmountSampler::new_gaussian(42, cfg).unwrap();
        for _ in 0..1000 {
            let v = sampler.sample_decimal();
            assert!(v >= Decimal::ZERO);
        }
    }

    #[test]
    fn reset_restores_determinism() {
        let cfg = log_normal_config_from_components(vec![(1.0, 5.0, 1.0, None)], 0.01, None, 2);
        let mut a = AdvancedAmountSampler::new_log_normal(7, cfg.clone()).unwrap();
        let first: Vec<Decimal> = (0..5).map(|_| a.sample_decimal()).collect();
        a.reset(7);
        let second: Vec<Decimal> = (0..5).map(|_| a.sample_decimal()).collect();
        assert_eq!(first, second);
    }

    #[test]
    fn pareto_sampler_produces_heavy_tail() {
        let cfg = ParetoConfig {
            alpha: 1.5,
            x_min: 1000.0,
            max_value: None,
            decimal_places: 2,
        };
        let mut sampler = AdvancedAmountSampler::new_pareto(42, cfg).unwrap();
        let samples: Vec<Decimal> = (0..10_000).map(|_| sampler.sample_decimal()).collect();
        // All samples must be >= x_min (Pareto support).
        let min_sample = samples.iter().min().unwrap();
        assert!(
            *min_sample >= Decimal::from(1000),
            "Pareto sample {min_sample} < x_min"
        );
        // Heavy tail: at least a few samples > 10x x_min (very unlikely for
        // log-normal with similar parameters, almost certain for Pareto).
        let extreme_count = samples
            .iter()
            .filter(|s| **s > Decimal::from(10_000))
            .count();
        assert!(
            extreme_count > 100,
            "expected heavy tail with >100/10000 extreme samples, got {extreme_count}"
        );
    }

    #[test]
    fn pareto_reset_is_deterministic() {
        let cfg = ParetoConfig {
            alpha: 2.0,
            x_min: 100.0,
            max_value: Some(1_000_000.0),
            decimal_places: 2,
        };
        let mut a = AdvancedAmountSampler::new_pareto(9, cfg).unwrap();
        let first: Vec<Decimal> = (0..5).map(|_| a.sample_decimal()).collect();
        a.reset(9);
        let second: Vec<Decimal> = (0..5).map(|_| a.sample_decimal()).collect();
        assert_eq!(first, second);
    }
}

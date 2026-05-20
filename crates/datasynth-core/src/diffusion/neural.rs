//! Neural diffusion backend using a learned score network.
//!
//! Unlike the statistical backend which targets parametric distributions (mean, std,
//! Cholesky correlations), the neural backend learns the actual data distribution
//! from samples using denoising score matching. It captures nonlinear cross-column
//! dependencies that the parametric approach misses.
//!
//! The backend implements [`DiffusionBackend`] so it slots into the existing
//! [`HybridGenerator`](super::HybridGenerator) seamlessly.

use std::path::Path;

use candle_core::{DType, Device, Tensor};
use candle_nn::{VarBuilder, VarMap};
use serde::{Deserialize, Serialize};

use super::backend::{DiffusionBackend, DiffusionConfig};
use super::schedule::NoiseSchedule;
use super::score_network::{ScoreNetwork, ScoreNetworkConfig};
use crate::error::SynthError;

/// Configuration for the neural diffusion backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeuralDiffusionConfig {
    /// Score network architecture.
    pub network: ScoreNetworkConfig,
    /// Diffusion process configuration (steps, schedule, seed).
    pub diffusion: DiffusionConfig,
}

/// A trained neural diffusion model that generates samples using a learned score network.
///
/// The model stores network weights and can be saved/loaded for reuse.
/// Generation uses the DDPM reverse process: start from Gaussian noise,
/// iteratively denoise using the score network's noise predictions.
pub struct NeuralDiffusionBackend {
    network: ScoreNetwork,
    config: NeuralDiffusionConfig,
    schedule: NoiseSchedule,
    /// Per-column means used to center generated data (learned during training).
    col_means: Vec<f32>,
    /// Per-column stds used to scale generated data (learned during training).
    col_stds: Vec<f32>,
    /// VarMap holding the network weights (needed for serialization).
    var_map: VarMap,
}

impl NeuralDiffusionBackend {
    /// Create a new backend from a trained VarMap and column statistics.
    ///
    /// Typically constructed by [`NeuralDiffusionTrainer::train`](super::NeuralDiffusionTrainer::train).
    pub fn new(
        config: NeuralDiffusionConfig,
        var_map: VarMap,
        col_means: Vec<f32>,
        col_stds: Vec<f32>,
    ) -> Result<Self, SynthError> {
        let schedule = config.diffusion.build_schedule();
        // v4.2.0+: honour CUDA when the `neural-cuda` feature is on and
        // a GPU is present; fall back to CPU otherwise.
        let device = super::preferred_device();
        let vb = VarBuilder::from_varmap(&var_map, DType::F32, &device);
        let network = ScoreNetwork::new(&config.network, vb)
            .map_err(|e| SynthError::generation(format!("Failed to build score network: {e}")))?;

        Ok(Self {
            network,
            config,
            schedule,
            col_means,
            col_stds,
            var_map,
        })
    }

    /// Save the model to a directory (config.json + weights.safetensors).
    pub fn save(&self, dir: &Path) -> Result<(), SynthError> {
        std::fs::create_dir_all(dir).map_err(|e| {
            SynthError::generation(format!("Failed to create model dir {}: {e}", dir.display()))
        })?;

        // Save config + column statistics
        let meta = NeuralModelMeta {
            config: self.config.clone(),
            col_means: self.col_means.clone(),
            col_stds: self.col_stds.clone(),
        };
        let json = serde_json::to_string_pretty(&meta).map_err(|e| {
            SynthError::generation(format!("Failed to serialize model config: {e}"))
        })?;
        std::fs::write(dir.join("config.json"), json)
            .map_err(|e| SynthError::generation(format!("Failed to write config.json: {e}")))?;

        // Save weights
        self.var_map
            .save(dir.join("weights.safetensors"))
            .map_err(|e| SynthError::generation(format!("Failed to save weights: {e}")))?;

        Ok(())
    }

    /// Load a model from a directory containing config.json + weights.safetensors.
    pub fn load(dir: &Path) -> Result<Self, SynthError> {
        let config_path = dir.join("config.json");
        let weights_path = dir.join("weights.safetensors");

        let json = std::fs::read_to_string(&config_path).map_err(|e| {
            SynthError::generation(format!("Failed to read {}: {e}", config_path.display()))
        })?;
        let meta: NeuralModelMeta = serde_json::from_str(&json)
            .map_err(|e| SynthError::generation(format!("Failed to parse config.json: {e}")))?;

        let var_map = VarMap::new();
        let device = super::preferred_device();
        let vb = VarBuilder::from_varmap(&var_map, DType::F32, &device);

        // Build network (this initializes the var_map with the right structure)
        let _network = ScoreNetwork::new(&meta.config.network, vb)
            .map_err(|e| SynthError::generation(format!("Failed to build network: {e}")))?;

        // Load saved weights into the var_map
        let mut var_map = var_map;
        var_map
            .load(weights_path)
            .map_err(|e| SynthError::generation(format!("Failed to load weights: {e}")))?;

        Self::new(meta.config, var_map, meta.col_means, meta.col_stds)
    }

    /// Convert `Vec<Vec<f64>>` to a candle Tensor (f32) on the active device.
    fn vecs_to_tensor(data: &[Vec<f64>]) -> Result<Tensor, SynthError> {
        let n_rows = data.len();
        let n_cols = data.first().map_or(0, |r| r.len());
        if n_rows == 0 || n_cols == 0 {
            return Err(SynthError::generation("Empty data"));
        }
        let flat: Vec<f32> = data
            .iter()
            .flat_map(|r| r.iter().map(|&v| v as f32))
            .collect();
        let device = super::preferred_device();
        Tensor::from_vec(flat, (n_rows, n_cols), &device)
            .map_err(|e| SynthError::generation(format!("Tensor creation failed: {e}")))
    }

    /// Convert a candle Tensor to `Vec<Vec<f64>>`.
    fn tensor_to_vecs(tensor: &Tensor) -> Result<Vec<Vec<f64>>, SynthError> {
        let data: Vec<Vec<f32>> = tensor
            .to_vec2()
            .map_err(|e| SynthError::generation(format!("Tensor to vec failed: {e}")))?;
        Ok(data
            .iter()
            .map(|r| r.iter().map(|&v| v as f64).collect())
            .collect())
    }

    /// DDPM reverse process: denoise from pure noise to data using the score network.
    fn reverse_process(&self, n_samples: usize, seed: u64) -> Result<Tensor, SynthError> {
        let n_features = self.config.network.n_features;
        let device = self.network.device();
        let n_steps = self.schedule.n_steps();

        // Start from standard Gaussian noise (deterministic via seed)
        let mut x_t = seeded_randn(n_samples, n_features, seed, device)?;

        // Reverse process: t = T-1 down to 0
        for t in (0..n_steps).rev() {
            let alpha_t = self.schedule.alphas[t] as f32;
            let alpha_bar_t = self.schedule.alpha_bars[t] as f32;
            let beta_t = self.schedule.betas[t] as f32;

            let sqrt_alpha_t = alpha_t.sqrt();
            let sqrt_one_minus_alpha_bar_t = (1.0 - alpha_bar_t).sqrt();

            // Predict noise using the score network
            let t_tensor = Tensor::from_vec(vec![t as u32; n_samples], (n_samples,), device)
                .map_err(|e| SynthError::generation(format!("Timestep tensor: {e}")))?;

            let predicted_noise = self
                .network
                .forward_with_t(&x_t, &t_tensor)
                .map_err(|e| SynthError::generation(format!("Score network forward: {e}")))?;

            // DDPM update: x_{t-1} = (1/sqrt(alpha_t)) * (x_t - (beta_t / sqrt(1 - alpha_bar_t)) * eps_theta) + sigma_t * z
            let coeff = beta_t / sqrt_one_minus_alpha_bar_t.max(1e-8);
            let noise_scaled = predicted_noise
                .affine(coeff as f64, 0.0)
                .map_err(|e| SynthError::generation(format!("Noise scaling: {e}")))?;
            let mean = (&x_t - &noise_scaled)
                .map_err(|e| SynthError::generation(format!("Mean computation: {e}")))?
                .affine(1.0 / sqrt_alpha_t as f64, 0.0)
                .map_err(|e| SynthError::generation(format!("Mean scaling: {e}")))?;

            if t > 0 {
                let sigma_t = beta_t.sqrt();
                let noise = seeded_randn(
                    n_samples,
                    n_features,
                    seed.wrapping_add(t as u64).wrapping_add(1_000_000),
                    device,
                )?;
                let noise_part = noise
                    .affine(sigma_t as f64, 0.0)
                    .map_err(|e| SynthError::generation(format!("Noise affine: {e}")))?;
                x_t = (&mean + &noise_part)
                    .map_err(|e| SynthError::generation(format!("Noise addition: {e}")))?;
            } else {
                x_t = mean;
            }
        }

        Ok(x_t)
    }

    /// Denormalize samples from standardized space back to original data scale.
    fn denormalize(&self, samples: &Tensor) -> Result<Tensor, SynthError> {
        let n_features = self.col_means.len();
        let means = Tensor::from_vec(
            self.col_means.clone(),
            (1, n_features),
            self.network.device(),
        )
        .map_err(|e| SynthError::generation(format!("Means tensor: {e}")))?;
        let stds = Tensor::from_vec(
            self.col_stds.clone(),
            (1, n_features),
            self.network.device(),
        )
        .map_err(|e| SynthError::generation(format!("Stds tensor: {e}")))?;

        let result = samples
            .broadcast_mul(&stds)
            .map_err(|e| SynthError::generation(format!("Mul stds: {e}")))?
            .broadcast_add(&means)
            .map_err(|e| SynthError::generation(format!("Add means: {e}")))?;
        Ok(result)
    }
}

impl DiffusionBackend for NeuralDiffusionBackend {
    fn name(&self) -> &str {
        "neural"
    }

    fn forward(&self, x: &[Vec<f64>], t: usize) -> Vec<Vec<f64>> {
        let Ok(x_tensor) = Self::vecs_to_tensor(x) else {
            return x.to_vec();
        };

        let t_clamped = t.min(self.schedule.n_steps().saturating_sub(1));
        let sqrt_alpha_bar = self.schedule.sqrt_alpha_bars[t_clamped] as f32;
        let sqrt_one_minus = self.schedule.sqrt_one_minus_alpha_bars[t_clamped] as f32;

        let n_features = x.first().map_or(0, |r| r.len());
        let noise = match seeded_randn(
            x.len(),
            n_features,
            self.config.diffusion.seed.wrapping_add(t as u64),
            self.network.device(),
        ) {
            Ok(n) => n,
            Err(_) => return x.to_vec(),
        };

        let result = match x_tensor
            .affine(sqrt_alpha_bar as f64, 0.0)
            .and_then(|signal| {
                let noise_part = noise.affine(sqrt_one_minus as f64, 0.0)?;
                &signal + &noise_part
            }) {
            Ok(r) => r,
            Err(_) => return x.to_vec(),
        };

        Self::tensor_to_vecs(&result).unwrap_or_else(|_| x.to_vec())
    }

    fn reverse(&self, x_t: &[Vec<f64>], t: usize) -> Vec<Vec<f64>> {
        let Ok(x_tensor) = Self::vecs_to_tensor(x_t) else {
            return x_t.to_vec();
        };

        let t_clamped = t.min(self.schedule.n_steps().saturating_sub(1));
        let n_samples = x_t.len();

        let t_tensor = match Tensor::from_vec(
            vec![t_clamped as u32; n_samples],
            (n_samples,),
            self.network.device(),
        ) {
            Ok(t) => t,
            Err(_) => return x_t.to_vec(),
        };

        let predicted_noise = match self.network.forward_with_t(&x_tensor, &t_tensor) {
            Ok(n) => n,
            Err(_) => return x_t.to_vec(),
        };

        let beta_t = self.schedule.betas[t_clamped] as f32;
        let alpha_t = self.schedule.alphas[t_clamped] as f32;
        let alpha_bar_t = self.schedule.alpha_bars[t_clamped] as f32;
        let sqrt_one_minus = (1.0 - alpha_bar_t).sqrt().max(1e-8);

        let coeff = beta_t / sqrt_one_minus;
        let result = match x_tensor
            .sub(
                &predicted_noise
                    .affine(coeff as f64, 0.0)
                    .unwrap_or(predicted_noise),
            )
            .and_then(|r| r.affine(1.0 / alpha_t.sqrt() as f64, 0.0))
        {
            Ok(r) => r,
            Err(_) => return x_t.to_vec(),
        };

        Self::tensor_to_vecs(&result).unwrap_or_else(|_| x_t.to_vec())
    }

    fn generate(&self, n_samples: usize, n_features: usize, seed: u64) -> Vec<Vec<f64>> {
        if n_samples == 0 || n_features == 0 {
            return vec![];
        }

        debug_assert_eq!(
            n_features, self.config.network.n_features,
            "n_features ({n_features}) does not match model dimension ({})",
            self.config.network.n_features
        );

        let samples = match self.reverse_process(n_samples, seed) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Neural generation failed, returning noise: {e}");
                return super::generate_noise(n_samples, n_features, seed);
            }
        };

        let denormalized = match self.denormalize(&samples) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Denormalization failed: {e}");
                return Self::tensor_to_vecs(&samples)
                    .unwrap_or_else(|_| super::generate_noise(n_samples, n_features, seed));
            }
        };

        Self::tensor_to_vecs(&denormalized)
            .unwrap_or_else(|_| super::generate_noise(n_samples, n_features, seed))
    }
}

/// Metadata for saving/loading a neural model.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct NeuralModelMeta {
    config: NeuralDiffusionConfig,
    col_means: Vec<f32>,
    col_stds: Vec<f32>,
}

/// Generate a deterministic random normal tensor using a seeded RNG.
///
/// candle's `Tensor::randn` uses the global RNG. For deterministic generation,
/// we generate samples in Rust and convert to a tensor.
fn seeded_randn(
    n_rows: usize,
    n_cols: usize,
    seed: u64,
    device: &Device,
) -> Result<Tensor, SynthError> {
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use rand_distr::{Distribution, StandardNormal};

    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let normal = StandardNormal;
    let data: Vec<f32> = (0..n_rows * n_cols)
        .map(|_| {
            let v: f64 = normal.sample(&mut rng);
            v as f32
        })
        .collect();

    Tensor::from_vec(data, (n_rows, n_cols), device)
        .map_err(|e| SynthError::generation(format!("seeded_randn failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_backend(n_features: usize) -> NeuralDiffusionBackend {
        let config = NeuralDiffusionConfig {
            network: ScoreNetworkConfig {
                n_features,
                hidden_dims: vec![32, 32],
                timestep_embed_dim: 16,
            },
            diffusion: DiffusionConfig {
                n_steps: 20,
                schedule: super::super::NoiseScheduleType::Linear,
                seed: 42,
            },
        };

        let var_map = VarMap::new();
        let col_means = vec![0.0f32; n_features];
        let col_stds = vec![1.0f32; n_features];

        NeuralDiffusionBackend::new(config, var_map, col_means, col_stds).unwrap()
    }

    #[test]
    fn test_generate_output_shape() {
        let backend = make_backend(4);
        let samples = backend.generate(50, 4, 42);
        assert_eq!(samples.len(), 50);
        for row in &samples {
            assert_eq!(row.len(), 4);
        }
    }

    #[test]
    fn test_generate_deterministic() {
        let backend = make_backend(3);
        let s1 = backend.generate(20, 3, 99);
        let s2 = backend.generate(20, 3, 99);

        for (r1, r2) in s1.iter().zip(s2.iter()) {
            for (&v1, &v2) in r1.iter().zip(r2.iter()) {
                assert!((v1 - v2).abs() < 1e-5, "Determinism failed: {v1} vs {v2}");
            }
        }
    }

    #[test]
    fn test_generate_empty() {
        let backend = make_backend(3);
        assert!(backend.generate(0, 3, 0).is_empty());
        assert!(backend.generate(10, 0, 0).is_empty());
    }

    #[test]
    fn test_forward_adds_noise() {
        let backend = make_backend(2);
        let original = vec![vec![1.0, 2.0]; 10];

        let noised = backend.forward(&original, 5);
        assert_eq!(noised.len(), 10);

        // At least some values should differ from original
        let changed = noised
            .iter()
            .zip(original.iter())
            .any(|(n, o)| (n[0] - o[0]).abs() > 1e-6);
        assert!(changed, "Forward should add noise");
    }

    #[test]
    fn test_name() {
        let backend = make_backend(2);
        assert_eq!(backend.name(), "neural");
    }

    #[test]
    fn test_save_load_roundtrip() {
        let backend = make_backend(3);
        let dir = tempfile::tempdir().expect("temp dir");

        backend.save(dir.path()).unwrap();
        let loaded = NeuralDiffusionBackend::load(dir.path()).unwrap();

        // Both should produce the same output
        let s1 = backend.generate(10, 3, 42);
        let s2 = loaded.generate(10, 3, 42);

        for (r1, r2) in s1.iter().zip(s2.iter()) {
            for (&v1, &v2) in r1.iter().zip(r2.iter()) {
                assert!(
                    (v1 - v2).abs() < 1e-4,
                    "Save/load roundtrip mismatch: {v1} vs {v2}"
                );
            }
        }
    }
}

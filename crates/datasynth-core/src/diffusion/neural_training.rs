//! Training pipeline for the neural diffusion backend.
//!
//! Implements denoising score matching: given real data, add noise at random
//! timesteps, train the score network to predict the added noise. The trained
//! model can then reverse the process to generate new samples from noise.
//!
//! # Example
//!
//! ```ignore
//! use datasynth_core::diffusion::{NeuralDiffusionTrainer, NeuralTrainingConfig};
//!
//! let data: Vec<Vec<f64>> = /* your training data */;
//! let config = NeuralTrainingConfig::default();
//! let backend = NeuralDiffusionTrainer::train(&data, &config, 42)?;
//! let samples = backend.generate(1000, data[0].len(), 42);
//! ```

use candle_core::{DType, Device, Tensor};
use candle_nn::{Optimizer, VarBuilder, VarMap};
use serde::{Deserialize, Serialize};

use super::backend::DiffusionConfig;
use super::neural::{NeuralDiffusionBackend, NeuralDiffusionConfig};
use super::schedule::NoiseSchedule;
use super::score_network::{ScoreNetwork, ScoreNetworkConfig};
use crate::error::SynthError;

/// Configuration for the neural diffusion training process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeuralTrainingConfig {
    /// Hidden layer dimensions for the score network.
    #[serde(default = "default_hidden_dims")]
    pub hidden_dims: Vec<usize>,
    /// Timestep embedding dimension.
    #[serde(default = "default_embed_dim")]
    pub timestep_embed_dim: usize,
    /// Number of diffusion timesteps.
    #[serde(default = "default_n_steps")]
    pub n_steps: usize,
    /// Noise schedule type.
    #[serde(default = "default_schedule")]
    pub schedule: String,
    /// Learning rate for Adam optimizer.
    #[serde(default = "default_lr")]
    pub learning_rate: f64,
    /// Number of training epochs.
    #[serde(default = "default_epochs")]
    pub epochs: usize,
    /// Mini-batch size.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

fn default_hidden_dims() -> Vec<usize> {
    vec![256, 256, 128]
}
fn default_embed_dim() -> usize {
    64
}
fn default_n_steps() -> usize {
    100
}
fn default_schedule() -> String {
    "cosine".to_string()
}
fn default_lr() -> f64 {
    1e-3
}
fn default_epochs() -> usize {
    100
}
fn default_batch_size() -> usize {
    256
}

impl Default for NeuralTrainingConfig {
    fn default() -> Self {
        Self {
            hidden_dims: default_hidden_dims(),
            timestep_embed_dim: default_embed_dim(),
            n_steps: default_n_steps(),
            schedule: default_schedule(),
            learning_rate: default_lr(),
            epochs: default_epochs(),
            batch_size: default_batch_size(),
        }
    }
}

/// Result of the training process, including loss history.
#[derive(Debug, Clone)]
pub struct TrainingReport {
    /// Loss value at the end of each epoch.
    pub epoch_losses: Vec<f64>,
    /// Final epoch loss.
    pub final_loss: f64,
    /// Number of epochs completed.
    pub epochs_completed: usize,
}

/// Trainer for the neural diffusion backend.
///
/// Given raw data samples, fits a score network via denoising score matching
/// and returns a [`NeuralDiffusionBackend`] ready for generation.
pub struct NeuralDiffusionTrainer;

impl NeuralDiffusionTrainer {
    /// Train a neural diffusion model from raw data.
    ///
    /// # Arguments
    /// * `data` - Training data as rows of feature vectors
    /// * `config` - Training hyperparameters
    /// * `seed` - Random seed for reproducible training
    ///
    /// # Returns
    /// A trained [`NeuralDiffusionBackend`] and a [`TrainingReport`].
    pub fn train(
        data: &[Vec<f64>],
        config: &NeuralTrainingConfig,
        seed: u64,
    ) -> Result<(NeuralDiffusionBackend, TrainingReport), SynthError> {
        let n_samples = data.len();
        let n_features = data.first().map_or(0, |r| r.len());
        if n_samples == 0 || n_features == 0 {
            return Err(SynthError::generation(
                "Training data must have at least one row with at least one feature",
            ));
        }

        // v4.2.0+: honour CUDA when `neural-cuda` is compiled in + GPU present.
        let device = super::preferred_device();

        // Normalize data to zero mean, unit variance (reuse utils)
        let (normalized, col_means, col_stds) = super::utils::normalize_features(data);

        // Convert to tensor
        let flat: Vec<f32> = normalized
            .iter()
            .flat_map(|r| r.iter().map(|&v| v as f32))
            .collect();
        let data_tensor = Tensor::from_vec(flat, (n_samples, n_features), &device)
            .map_err(|e| SynthError::generation(format!("Data tensor creation: {e}")))?;

        // Build noise schedule
        let schedule_type = match config.schedule.as_str() {
            "cosine" => super::backend::NoiseScheduleType::Cosine,
            "sigmoid" => super::backend::NoiseScheduleType::Sigmoid,
            _ => super::backend::NoiseScheduleType::Linear,
        };
        let diffusion_config = DiffusionConfig {
            n_steps: config.n_steps,
            schedule: schedule_type.clone(),
            seed,
        };
        let schedule = diffusion_config.build_schedule();

        // Build network
        let net_config = ScoreNetworkConfig {
            n_features,
            hidden_dims: config.hidden_dims.clone(),
            timestep_embed_dim: config.timestep_embed_dim,
        };

        let var_map = VarMap::new();
        let vb = VarBuilder::from_varmap(&var_map, DType::F32, &device);
        let network = ScoreNetwork::new(&net_config, vb)
            .map_err(|e| SynthError::generation(format!("Network build: {e}")))?;

        // Set up optimizer
        let params = var_map.all_vars();
        let mut optimizer = candle_nn::optim::AdamW::new_lr(params, config.learning_rate)
            .map_err(|e| SynthError::generation(format!("Optimizer init: {e}")))?;

        // Training loop
        let mut epoch_losses = Vec::with_capacity(config.epochs);
        let mut rng = <rand_chacha::ChaCha8Rng as rand::SeedableRng>::seed_from_u64(seed);

        for epoch in 0..config.epochs {
            let epoch_loss = train_one_epoch(
                &network,
                &data_tensor,
                &schedule,
                config.batch_size,
                &mut optimizer,
                &mut rng,
                &device,
            )?;

            epoch_losses.push(epoch_loss);

            if epoch % 20 == 0 || epoch == config.epochs - 1 {
                tracing::debug!(
                    "Epoch {}/{}: loss = {:.6}",
                    epoch + 1,
                    config.epochs,
                    epoch_loss
                );
            }
        }

        let final_loss = epoch_losses.last().copied().unwrap_or(f64::INFINITY);

        let report = TrainingReport {
            epoch_losses,
            final_loss,
            epochs_completed: config.epochs,
        };

        let col_means_f32: Vec<f32> = col_means.iter().map(|&v| v as f32).collect();
        let col_stds_f32: Vec<f32> = col_stds.iter().map(|&v| v as f32).collect();

        let backend_config = NeuralDiffusionConfig {
            network: net_config,
            diffusion: diffusion_config,
        };

        let backend =
            NeuralDiffusionBackend::new(backend_config, var_map, col_means_f32, col_stds_f32)?;

        Ok((backend, report))
    }
}

/// Train for one epoch, returning the average loss.
fn train_one_epoch(
    network: &ScoreNetwork,
    data: &Tensor,
    schedule: &NoiseSchedule,
    batch_size: usize,
    optimizer: &mut candle_nn::optim::AdamW,
    rng: &mut rand_chacha::ChaCha8Rng,
    device: &Device,
) -> Result<f64, SynthError> {
    use rand::RngExt;

    let n_samples = data
        .dim(0)
        .map_err(|e| SynthError::generation(format!("{e}")))?;
    let n_features = data
        .dim(1)
        .map_err(|e| SynthError::generation(format!("{e}")))?;
    let n_steps = schedule.n_steps();

    let n_batches = n_samples.div_ceil(batch_size);
    let mut total_loss = 0.0;
    let mut batch_count = 0;

    for batch_idx in 0..n_batches {
        let start = batch_idx * batch_size;
        let end = (start + batch_size).min(n_samples);
        let actual_batch = end - start;
        if actual_batch == 0 {
            continue;
        }

        // Get batch
        let batch = data
            .narrow(0, start, actual_batch)
            .map_err(|e| SynthError::generation(format!("Batch slice: {e}")))?;

        // Sample random timesteps
        let timesteps: Vec<u32> = (0..actual_batch)
            .map(|_| rng.random_range(0..n_steps as u32))
            .collect();
        let t_tensor = Tensor::from_vec(timesteps.clone(), (actual_batch,), device)
            .map_err(|e| SynthError::generation(format!("Timestep tensor: {e}")))?;

        // Sample random noise
        let noise_data: Vec<f32> = (0..actual_batch * n_features)
            .map(|_| {
                use rand_distr::Distribution;
                let normal = rand_distr::StandardNormal;
                let v: f64 = normal.sample(rng);
                v as f32
            })
            .collect();
        let noise = Tensor::from_vec(noise_data, (actual_batch, n_features), device)
            .map_err(|e| SynthError::generation(format!("Noise tensor: {e}")))?;

        // Forward diffusion: x_t = sqrt(alpha_bar_t) * x_0 + sqrt(1 - alpha_bar_t) * noise
        let sqrt_alpha_bars: Vec<f32> = timesteps
            .iter()
            .map(|&t| schedule.sqrt_alpha_bars[t as usize] as f32)
            .collect();
        let sqrt_one_minus: Vec<f32> = timesteps
            .iter()
            .map(|&t| schedule.sqrt_one_minus_alpha_bars[t as usize] as f32)
            .collect();

        let sab = Tensor::from_vec(sqrt_alpha_bars, (actual_batch, 1), device)
            .map_err(|e| SynthError::generation(format!("{e}")))?;
        let som = Tensor::from_vec(sqrt_one_minus, (actual_batch, 1), device)
            .map_err(|e| SynthError::generation(format!("{e}")))?;

        let x_t = (batch
            .broadcast_mul(&sab)
            .map_err(|e| SynthError::generation(format!("{e}")))?
            + noise
                .broadcast_mul(&som)
                .map_err(|e| SynthError::generation(format!("{e}")))?)
        .map_err(|e| SynthError::generation(format!("x_t computation: {e}")))?;

        // Predict noise
        let predicted = network
            .forward_with_t(&x_t, &t_tensor)
            .map_err(|e| SynthError::generation(format!("Network forward: {e}")))?;

        // MSE loss: ||predicted - actual_noise||^2
        let diff =
            (&predicted - &noise).map_err(|e| SynthError::generation(format!("Loss diff: {e}")))?;
        let loss = diff
            .sqr()
            .map_err(|e| SynthError::generation(format!("Sqr: {e}")))?
            .mean_all()
            .map_err(|e| SynthError::generation(format!("Mean: {e}")))?;

        // Backward + step
        optimizer
            .backward_step(&loss)
            .map_err(|e| SynthError::generation(format!("Optimizer step: {e}")))?;

        let loss_val: f32 = loss
            .to_scalar()
            .map_err(|e| SynthError::generation(format!("Loss scalar: {e}")))?;
        total_loss += loss_val as f64;
        batch_count += 1;
    }

    Ok(if batch_count > 0 {
        total_loss / batch_count as f64
    } else {
        0.0
    })
}

#[cfg(test)]
mod tests {
    use super::super::DiffusionBackend;
    use super::*;

    /// Generate simple synthetic training data: 2D Gaussian clusters.
    fn make_training_data(n: usize, seed: u64) -> Vec<Vec<f64>> {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;
        use rand_distr::{Distribution, Normal};

        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let normal = Normal::new(0.0, 1.0).unwrap();

        (0..n)
            .map(|_| {
                let x: f64 = 100.0 + 15.0 * normal.sample(&mut rng);
                let y: f64 = 50.0 + 10.0 * normal.sample(&mut rng);
                vec![x, y]
            })
            .collect()
    }

    #[test]
    fn test_train_produces_backend() {
        let data = make_training_data(200, 42);
        let config = NeuralTrainingConfig {
            hidden_dims: vec![32, 32],
            timestep_embed_dim: 16,
            n_steps: 20,
            epochs: 5,
            batch_size: 64,
            ..Default::default()
        };

        let (backend, report) = NeuralDiffusionTrainer::train(&data, &config, 42).unwrap();

        assert_eq!(report.epochs_completed, 5);
        assert_eq!(report.epoch_losses.len(), 5);
        assert!(report.final_loss.is_finite());

        let samples = backend.generate(50, 2, 99);
        assert_eq!(samples.len(), 50);
        for row in &samples {
            assert_eq!(row.len(), 2);
        }
    }

    #[test]
    fn test_train_loss_decreases() {
        let data = make_training_data(500, 42);
        let config = NeuralTrainingConfig {
            hidden_dims: vec![64, 64],
            timestep_embed_dim: 32,
            n_steps: 50,
            epochs: 30,
            batch_size: 128,
            learning_rate: 1e-3,
            ..Default::default()
        };

        let (_backend, report) = NeuralDiffusionTrainer::train(&data, &config, 42).unwrap();

        // Loss should generally decrease (compare first vs last)
        let first_loss = report.epoch_losses[0];
        let last_loss = report.final_loss;
        assert!(
            last_loss < first_loss,
            "Loss should decrease: first={first_loss:.4}, last={last_loss:.4}"
        );
    }

    #[test]
    fn test_train_empty_data_fails() {
        let config = NeuralTrainingConfig::default();
        let result = NeuralDiffusionTrainer::train(&[], &config, 42);
        assert!(result.is_err());
    }

    #[test]
    fn test_normalize_features_stats() {
        let data = vec![vec![10.0, 20.0], vec![20.0, 40.0], vec![30.0, 60.0]];
        let (_normalized, means, stds) = super::super::utils::normalize_features(&data);

        assert!((means[0] - 20.0).abs() < 1e-10);
        assert!((means[1] - 40.0).abs() < 1e-10);
        assert!(stds[0] > 0.0);
        assert!(stds[1] > 0.0);
    }

    #[test]
    fn test_normalize_roundtrip() {
        let data = vec![vec![100.0, 200.0], vec![120.0, 220.0]];
        let (normalized, _means, _stds) = super::super::utils::normalize_features(&data);

        // Normalized mean should be ~0
        let mean_0: f64 = normalized.iter().map(|r| r[0]).sum::<f64>() / normalized.len() as f64;
        assert!(mean_0.abs() < 1e-10);
    }
}

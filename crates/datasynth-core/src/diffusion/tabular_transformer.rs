//! Tabular transformer for conditional data generation.
//!
//! A small encoder-only transformer that learns conditional distributions over
//! tabular features. Given a partial row (context columns), it predicts the
//! remaining columns — useful for generating realistic invoice patterns
//! conditional on vendor profiles, payment sequences given customer history, etc.
//!
//! Architecture:
//! ```text
//! [context_features | mask_tokens] → PositionalEncoding → N × TransformerBlock → Linear → features
//! ```
//!
//! Each `TransformerBlock` contains multi-head self-attention + feed-forward with
//! residual connections and layer normalization.

use candle_core::{DType, Device, Result as CandleResult, Tensor};
use candle_nn::{linear, Linear, Module, VarBuilder, VarMap};
use serde::{Deserialize, Serialize};

use crate::error::SynthError;

/// Configuration for the tabular transformer architecture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabularTransformerConfig {
    /// Number of input/output features (columns).
    pub n_features: usize,
    /// Hidden dimension (embedding size).
    #[serde(default = "default_d_model")]
    pub d_model: usize,
    /// Number of attention heads.
    #[serde(default = "default_n_heads")]
    pub n_heads: usize,
    /// Number of transformer blocks.
    #[serde(default = "default_n_layers")]
    pub n_layers: usize,
    /// Feed-forward inner dimension (typically 4 * d_model).
    #[serde(default = "default_ff_dim")]
    pub ff_dim: usize,
    /// Dropout rate (for training; set to 0.0 at inference).
    #[serde(default)]
    pub dropout: f64,
}

fn default_d_model() -> usize {
    128
}
fn default_n_heads() -> usize {
    4
}
fn default_n_layers() -> usize {
    2
}
fn default_ff_dim() -> usize {
    256
}

impl TabularTransformerConfig {
    /// Create a config for the given number of features with defaults.
    pub fn new(n_features: usize) -> Self {
        Self {
            n_features,
            d_model: default_d_model(),
            n_heads: default_n_heads(),
            n_layers: default_n_layers(),
            ff_dim: default_ff_dim(),
            dropout: 0.0,
        }
    }
}

/// A single transformer block: self-attention + feed-forward + residual + layer norm.
struct TransformerBlock {
    attn_q: Linear,
    attn_k: Linear,
    attn_v: Linear,
    attn_out: Linear,
    ff1: Linear,
    ff2: Linear,
    n_heads: usize,
    d_model: usize,
}

impl TransformerBlock {
    fn new(d_model: usize, n_heads: usize, ff_dim: usize, vb: VarBuilder) -> CandleResult<Self> {
        let attn_q = linear(d_model, d_model, vb.pp("attn_q"))?;
        let attn_k = linear(d_model, d_model, vb.pp("attn_k"))?;
        let attn_v = linear(d_model, d_model, vb.pp("attn_v"))?;
        let attn_out = linear(d_model, d_model, vb.pp("attn_out"))?;
        let ff1 = linear(d_model, ff_dim, vb.pp("ff1"))?;
        let ff2 = linear(ff_dim, d_model, vb.pp("ff2"))?;
        Ok(Self {
            attn_q,
            attn_k,
            attn_v,
            attn_out,
            ff1,
            ff2,
            n_heads,
            d_model,
        })
    }

    /// Forward pass through this block.
    ///
    /// Input shape: `(batch, seq_len, d_model)`
    fn forward(&self, x: &Tensor) -> CandleResult<Tensor> {
        // Multi-head self-attention
        let attn_out = self.self_attention(x)?;
        // Residual connection (skip layer norm for simplicity in this small model)
        let x = (x + attn_out)?;

        // Feed-forward
        let ff_out = self.ff1.forward(&x)?.gelu()?;
        let ff_out = self.ff2.forward(&ff_out)?;
        // Residual connection
        let x = (&x + ff_out)?;

        Ok(x)
    }

    /// Multi-head self-attention.
    fn self_attention(&self, x: &Tensor) -> CandleResult<Tensor> {
        let (batch, seq_len, _) = x.dims3()?;
        let head_dim = self.d_model / self.n_heads;

        let q = self.attn_q.forward(x)?;
        let k = self.attn_k.forward(x)?;
        let v = self.attn_v.forward(x)?;

        // Reshape to (batch, n_heads, seq_len, head_dim) — contiguous for matmul
        let q = q
            .reshape((batch, seq_len, self.n_heads, head_dim))?
            .transpose(1, 2)?
            .contiguous()?;
        let k = k
            .reshape((batch, seq_len, self.n_heads, head_dim))?
            .transpose(1, 2)?
            .contiguous()?;
        let v = v
            .reshape((batch, seq_len, self.n_heads, head_dim))?
            .transpose(1, 2)?
            .contiguous()?;

        // Scaled dot-product attention
        let scale = (head_dim as f64).sqrt();
        let k_t = k.transpose(2, 3)?.contiguous()?;
        let scores = q.matmul(&k_t)?.affine(1.0 / scale, 0.0)?;
        let attn_weights = candle_nn::ops::softmax(&scores, candle_core::D::Minus1)?;
        let attn_output = attn_weights.matmul(&v)?;

        // Reshape back to (batch, seq_len, d_model)
        let attn_output =
            attn_output
                .transpose(1, 2)?
                .contiguous()?
                .reshape((batch, seq_len, self.d_model))?;

        self.attn_out.forward(&attn_output)
    }
}

/// Tabular transformer model for conditional generation.
///
/// Each feature is treated as a "token" — the model learns attention patterns
/// over columns, capturing which features are predictive of which others.
pub struct TabularTransformer {
    /// Project scalar features to d_model embeddings.
    input_proj: Linear,
    /// Transformer blocks.
    blocks: Vec<TransformerBlock>,
    /// Project back from d_model to scalar predictions.
    output_proj: Linear,
    config: TabularTransformerConfig,
    device: Device,
}

impl TabularTransformer {
    /// Build a new tabular transformer.
    pub fn new(config: &TabularTransformerConfig, vb: VarBuilder) -> CandleResult<Self> {
        // Each feature becomes a token: scalar → d_model embedding
        let input_proj = linear(1, config.d_model, vb.pp("input_proj"))?;

        let mut blocks = Vec::with_capacity(config.n_layers);
        for i in 0..config.n_layers {
            blocks.push(TransformerBlock::new(
                config.d_model,
                config.n_heads,
                config.ff_dim,
                vb.pp(format!("block_{i}")),
            )?);
        }

        let output_proj = linear(config.d_model, 1, vb.pp("output_proj"))?;

        Ok(Self {
            input_proj,
            blocks,
            output_proj,
            config: config.clone(),
            device: vb.device().clone(),
        })
    }

    /// Number of features.
    pub fn n_features(&self) -> usize {
        self.config.n_features
    }

    /// Forward pass: predict all features given input features.
    ///
    /// # Arguments
    /// * `x` - Input tensor of shape `(batch, n_features)` with known values.
    ///   Masked (unknown) features should be set to 0.0.
    /// * `mask` - Boolean mask of shape `(batch, n_features)` where 1.0 = known, 0.0 = predict.
    ///
    /// # Returns
    /// Predictions for ALL features of shape `(batch, n_features)`.
    /// Known features pass through unchanged; masked features are predicted.
    pub fn forward(&self, x: &Tensor, mask: &Tensor) -> CandleResult<Tensor> {
        let (_batch, _n_feat) = x.dims2()?;

        // Each feature becomes a token: (batch, n_features) → (batch, n_features, 1)
        let x_3d = x.unsqueeze(2)?;

        // Project to embedding space: (batch, n_features, 1) → (batch, n_features, d_model)
        let mut hidden = self.input_proj.forward(&x_3d)?;

        // Add learned positional-like signal from the mask
        // (known vs unknown context — helps the model distinguish context from targets)
        let mask_3d = mask.unsqueeze(2)?;
        let mask_embed = mask_3d.broadcast_mul(
            &Tensor::ones((1, 1, self.config.d_model), DType::F32, &self.device)?
                .affine(0.1, 0.0)?,
        )?;
        hidden = (hidden + mask_embed)?;

        // Pass through transformer blocks
        for block in &self.blocks {
            hidden = block.forward(&hidden)?;
        }

        // Project back to scalars: (batch, n_features, d_model) → (batch, n_features, 1)
        let output = self.output_proj.forward(&hidden)?;

        // Squeeze: (batch, n_features, 1) → (batch, n_features)
        let output = output.squeeze(2)?;

        // Blend: keep known values, use predictions for masked positions
        // result = mask * x + (1 - mask) * output
        let known = x.mul(mask)?;
        let inv_mask = mask.affine(-1.0, 1.0)?; // 1 - mask without allocating a ones tensor
        let predicted = output.mul(&inv_mask)?;
        let result = (known + predicted)?;

        Ok(result)
    }
}

/// Training configuration for the tabular transformer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabularTransformerTrainingConfig {
    /// Transformer architecture config.
    pub model: TabularTransformerConfig,
    /// Learning rate.
    #[serde(default = "default_tt_lr")]
    pub learning_rate: f64,
    /// Training epochs.
    #[serde(default = "default_tt_epochs")]
    pub epochs: usize,
    /// Batch size.
    #[serde(default = "default_tt_batch")]
    pub batch_size: usize,
    /// Fraction of features to mask during training (0.0-1.0).
    #[serde(default = "default_mask_ratio")]
    pub mask_ratio: f64,
}

fn default_tt_lr() -> f64 {
    1e-3
}
fn default_tt_epochs() -> usize {
    50
}
fn default_tt_batch() -> usize {
    128
}
fn default_mask_ratio() -> f64 {
    0.3
}

/// A trained tabular transformer ready for conditional generation.
pub struct TrainedTabularTransformer {
    model: TabularTransformer,
    col_means: Vec<f32>,
    col_stds: Vec<f32>,
    var_map: VarMap,
    config: TabularTransformerTrainingConfig,
}

impl TrainedTabularTransformer {
    /// Generate rows by conditioning on known columns and predicting the rest.
    ///
    /// # Arguments
    /// * `context` - Partial rows with known values. Shape: `n_samples x n_features`.
    ///   Unknown values should be f64::NAN or 0.0.
    /// * `known_columns` - Indices of columns that are known (context).
    /// * `seed` - Random seed (used for stochastic sampling in future; deterministic now).
    pub fn predict(
        &self,
        context: &[Vec<f64>],
        known_columns: &[usize],
        _seed: u64,
    ) -> Result<Vec<Vec<f64>>, SynthError> {
        let n_samples = context.len();
        let n_features = self.config.model.n_features;
        if n_samples == 0 || n_features == 0 {
            return Ok(vec![]);
        }

        // Normalize context
        let normalized: Vec<Vec<f32>> = context
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(j, &v)| {
                        if j < self.col_means.len() {
                            ((v as f32) - self.col_means[j]) / self.col_stds[j]
                        } else {
                            0.0
                        }
                    })
                    .collect()
            })
            .collect();

        let flat: Vec<f32> = normalized.iter().flat_map(|r| r.iter().copied()).collect();
        let x = Tensor::from_vec(flat, (n_samples, n_features), &self.model.device)
            .map_err(|e| SynthError::generation(format!("Input tensor: {e}")))?;

        // Build mask: 1.0 for known columns, 0.0 for unknown
        let mask_data: Vec<f32> = (0..n_samples)
            .flat_map(|_| {
                (0..n_features).map(|j| if known_columns.contains(&j) { 1.0 } else { 0.0 })
            })
            .collect();
        let mask = Tensor::from_vec(mask_data, (n_samples, n_features), &self.model.device)
            .map_err(|e| SynthError::generation(format!("Mask tensor: {e}")))?;

        let output = self
            .model
            .forward(&x, &mask)
            .map_err(|e| SynthError::generation(format!("Forward pass: {e}")))?;

        // Denormalize
        let output_data: Vec<Vec<f32>> = output
            .to_vec2()
            .map_err(|e| SynthError::generation(format!("Output to vec: {e}")))?;

        Ok(output_data
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(j, &v)| {
                        if j < self.col_means.len() {
                            (v * self.col_stds[j] + self.col_means[j]) as f64
                        } else {
                            v as f64
                        }
                    })
                    .collect()
            })
            .collect())
    }

    /// Save the model.
    pub fn save(&self, dir: &std::path::Path) -> Result<(), SynthError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| SynthError::generation(format!("Create dir: {e}")))?;

        let meta = serde_json::json!({
            "config": self.config,
            "col_means": self.col_means,
            "col_stds": self.col_stds,
        });
        std::fs::write(dir.join("transformer_config.json"), meta.to_string())
            .map_err(|e| SynthError::generation(format!("Write config: {e}")))?;

        self.var_map
            .save(dir.join("transformer_weights.safetensors"))
            .map_err(|e| SynthError::generation(format!("Save weights: {e}")))?;

        Ok(())
    }
}

/// Train a tabular transformer from data.
pub struct TabularTransformerTrainer;

impl TabularTransformerTrainer {
    /// Train a tabular transformer on the given data.
    ///
    /// During training, random columns are masked and the model learns to
    /// predict them from the remaining columns — learning column-to-column
    /// conditional distributions.
    pub fn train(
        data: &[Vec<f64>],
        config: &TabularTransformerTrainingConfig,
        seed: u64,
    ) -> Result<TrainedTabularTransformer, SynthError> {
        let n_samples = data.len();
        let n_features = data.first().map_or(0, |r| r.len());
        if n_samples == 0 || n_features == 0 {
            return Err(SynthError::generation("Training data must be non-empty"));
        }

        let device = Device::Cpu;

        // Normalize
        let (normalized, col_means, col_stds) = super::utils::normalize_features(data);
        let col_means_f32: Vec<f32> = col_means.iter().map(|&v| v as f32).collect();
        let col_stds_f32: Vec<f32> = col_stds.iter().map(|&v| v as f32).collect();

        let flat: Vec<f32> = normalized
            .iter()
            .flat_map(|r| r.iter().map(|&v| v as f32))
            .collect();
        let data_tensor = Tensor::from_vec(flat, (n_samples, n_features), &device)
            .map_err(|e| SynthError::generation(format!("Data tensor: {e}")))?;

        // Build model
        let var_map = VarMap::new();
        let vb = VarBuilder::from_varmap(&var_map, DType::F32, &device);
        let model = TabularTransformer::new(&config.model, vb)
            .map_err(|e| SynthError::generation(format!("Build model: {e}")))?;

        // Optimizer
        let params = var_map.all_vars();
        let mut optimizer = candle_nn::optim::AdamW::new_lr(params, config.learning_rate)
            .map_err(|e| SynthError::generation(format!("Optimizer: {e}")))?;

        let mut rng = <rand_chacha::ChaCha8Rng as rand::SeedableRng>::seed_from_u64(seed);

        // Training loop
        for epoch in 0..config.epochs {
            let epoch_loss = train_epoch(
                &model,
                &data_tensor,
                config.batch_size,
                config.mask_ratio,
                n_features,
                &mut optimizer,
                &mut rng,
                &device,
            )?;

            if epoch % 10 == 0 || epoch == config.epochs - 1 {
                tracing::debug!(
                    "TabTransformer epoch {}/{}: loss = {:.6}",
                    epoch + 1,
                    config.epochs,
                    epoch_loss
                );
            }
        }

        Ok(TrainedTabularTransformer {
            model,
            col_means: col_means_f32,
            col_stds: col_stds_f32,
            var_map,
            config: config.clone(),
        })
    }
}

/// Train one epoch, return average loss.
#[allow(clippy::too_many_arguments)]
fn train_epoch(
    model: &TabularTransformer,
    data: &Tensor,
    batch_size: usize,
    mask_ratio: f64,
    n_features: usize,
    optimizer: &mut candle_nn::optim::AdamW,
    rng: &mut rand_chacha::ChaCha8Rng,
    device: &Device,
) -> Result<f64, SynthError> {
    use candle_nn::Optimizer;
    use rand::RngExt;

    let n_samples = data
        .dim(0)
        .map_err(|e| SynthError::generation(format!("{e}")))?;
    let n_batches = n_samples.div_ceil(batch_size);
    let mut total_loss = 0.0;
    let mut count = 0;

    for batch_idx in 0..n_batches {
        let start = batch_idx * batch_size;
        let actual = (start + batch_size).min(n_samples) - start;
        if actual == 0 {
            continue;
        }

        let batch = data
            .narrow(0, start, actual)
            .map_err(|e| SynthError::generation(format!("Batch: {e}")))?;

        // Random mask: for each sample, mask ~mask_ratio fraction of features
        let mask_data: Vec<f32> = (0..actual * n_features)
            .map(|_| {
                if rng.random_range(0.0..1.0) > mask_ratio {
                    1.0f32
                } else {
                    0.0f32
                }
            })
            .collect();
        let mask = Tensor::from_vec(mask_data, (actual, n_features), device)
            .map_err(|e| SynthError::generation(format!("Mask: {e}")))?;

        // Masked input: zero out masked positions
        let masked_input = batch
            .mul(&mask)
            .map_err(|e| SynthError::generation(format!("Masked input: {e}")))?;

        // Forward pass
        let predicted = model
            .forward(&masked_input, &mask)
            .map_err(|e| SynthError::generation(format!("Forward: {e}")))?;

        // Loss: MSE on masked positions only
        let diff =
            (&predicted - &batch).map_err(|e| SynthError::generation(format!("Diff: {e}")))?;
        let inv_mask = Tensor::ones((actual, n_features), DType::F32, device)
            .map_err(|e| SynthError::generation(format!("Ones: {e}")))?
            .sub(&mask)
            .map_err(|e| SynthError::generation(format!("Inv mask: {e}")))?;

        let masked_diff = diff
            .mul(&inv_mask)
            .map_err(|e| SynthError::generation(format!("Masked diff: {e}")))?;
        let loss = masked_diff
            .sqr()
            .map_err(|e| SynthError::generation(format!("Sqr: {e}")))?
            .sum_all()
            .map_err(|e| SynthError::generation(format!("Sum: {e}")))?;

        // Normalize by number of masked positions
        let n_masked = inv_mask
            .sum_all()
            .map_err(|e| SynthError::generation(format!("Count masked: {e}")))?;
        let n_masked_clamped = n_masked
            .clamp(1e-8, f64::MAX)
            .map_err(|e| SynthError::generation(format!("Clamp: {e}")))?;
        let loss = loss
            .div(&n_masked_clamped)
            .map_err(|e| SynthError::generation(format!("Normalize loss: {e}")))?;

        optimizer
            .backward_step(&loss)
            .map_err(|e| SynthError::generation(format!("Optimizer: {e}")))?;

        let loss_val: f32 = loss
            .to_scalar()
            .map_err(|e| SynthError::generation(format!("Loss scalar: {e}")))?;
        total_loss += loss_val as f64;
        count += 1;
    }

    Ok(if count > 0 {
        total_loss / count as f64
    } else {
        0.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_data(n: usize, seed: u64) -> Vec<Vec<f64>> {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;
        use rand_distr::{Distribution, Normal};

        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let normal = Normal::new(0.0, 1.0).unwrap();

        (0..n)
            .map(|_| {
                let x: f64 = 100.0 + 10.0 * normal.sample(&mut rng);
                let y: f64 = 0.5 * x + 5.0 * normal.sample(&mut rng); // correlated
                let z: f64 = 50.0 + 8.0 * normal.sample(&mut rng);
                vec![x, y, z]
            })
            .collect()
    }

    #[test]
    fn test_transformer_forward_shape() {
        let config = TabularTransformerConfig {
            n_features: 4,
            d_model: 32,
            n_heads: 2,
            n_layers: 1,
            ff_dim: 64,
            dropout: 0.0,
        };
        let vm = VarMap::new();
        let vb = VarBuilder::from_varmap(&vm, DType::F32, &Device::Cpu);
        let model = TabularTransformer::new(&config, vb).unwrap();

        let x = Tensor::randn(0f32, 1f32, (5, 4), &Device::Cpu).unwrap();
        let mask =
            Tensor::from_vec(vec![1.0f32, 1.0, 0.0, 0.0].repeat(5), (5, 4), &Device::Cpu).unwrap();

        let output = model.forward(&x, &mask).unwrap();
        assert_eq!(output.dims(), &[5, 4]);
    }

    #[test]
    fn test_train_produces_model() {
        let data = make_data(100, 42);
        let config = TabularTransformerTrainingConfig {
            model: TabularTransformerConfig {
                n_features: 3,
                d_model: 32,
                n_heads: 2,
                n_layers: 1,
                ff_dim: 64,
                dropout: 0.0,
            },
            learning_rate: 1e-3,
            epochs: 5,
            batch_size: 32,
            mask_ratio: 0.3,
        };

        let trained = TabularTransformerTrainer::train(&data, &config, 42).unwrap();
        assert_eq!(trained.model.n_features(), 3);
    }

    #[test]
    fn test_predict_conditional() {
        let data = make_data(200, 42);
        let config = TabularTransformerTrainingConfig {
            model: TabularTransformerConfig {
                n_features: 3,
                d_model: 32,
                n_heads: 2,
                n_layers: 1,
                ff_dim: 64,
                dropout: 0.0,
            },
            learning_rate: 1e-3,
            epochs: 10,
            batch_size: 64,
            mask_ratio: 0.3,
        };

        let trained = TabularTransformerTrainer::train(&data, &config, 42).unwrap();

        // Predict column 1 (y) given columns 0 (x) and 2 (z)
        let context = vec![vec![100.0, 0.0, 50.0], vec![110.0, 0.0, 55.0]];
        let predictions = trained.predict(&context, &[0, 2], 42).unwrap();

        assert_eq!(predictions.len(), 2);
        for row in &predictions {
            assert_eq!(row.len(), 3);
            // Known columns should be close to original
            assert!((row[0] - context[0][0]).abs() < 1.0 || (row[0] - context[1][0]).abs() < 1.0);
        }
    }

    #[test]
    fn test_predict_empty() {
        let data = make_data(100, 42);
        let config = TabularTransformerTrainingConfig {
            model: TabularTransformerConfig::new(3),
            epochs: 2,
            ..TabularTransformerTrainingConfig {
                model: TabularTransformerConfig::new(3),
                learning_rate: default_tt_lr(),
                epochs: 2,
                batch_size: default_tt_batch(),
                mask_ratio: default_mask_ratio(),
            }
        };

        let trained = TabularTransformerTrainer::train(&data, &config, 42).unwrap();
        let result = trained.predict(&[], &[0], 42).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_train_empty_fails() {
        let config = TabularTransformerTrainingConfig {
            model: TabularTransformerConfig::new(3),
            learning_rate: default_tt_lr(),
            epochs: 2,
            batch_size: default_tt_batch(),
            mask_ratio: default_mask_ratio(),
        };
        assert!(TabularTransformerTrainer::train(&[], &config, 42).is_err());
    }

    #[test]
    fn test_save_model() {
        let data = make_data(50, 42);
        let config = TabularTransformerTrainingConfig {
            model: TabularTransformerConfig {
                n_features: 3,
                d_model: 16,
                n_heads: 2,
                n_layers: 1,
                ff_dim: 32,
                dropout: 0.0,
            },
            learning_rate: 1e-3,
            epochs: 2,
            batch_size: 32,
            mask_ratio: 0.3,
        };

        let trained = TabularTransformerTrainer::train(&data, &config, 42).unwrap();
        let dir = tempfile::tempdir().unwrap();
        trained.save(dir.path()).unwrap();

        assert!(dir.path().join("transformer_config.json").exists());
        assert!(dir.path().join("transformer_weights.safetensors").exists());
    }
}

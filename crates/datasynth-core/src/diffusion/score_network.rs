//! Small MLP score network for tabular diffusion.
//!
//! Architecture: `(features + timestep_embed) -> [Linear -> SiLU] x N -> Linear -> features`
//!
//! The network predicts the noise added at a given timestep, which the reverse
//! diffusion process uses to progressively denoise samples.

use candle_core::{DType, Device, Result as CandleResult, Tensor};
use candle_nn::{linear, Linear, Module, VarBuilder};
use serde::{Deserialize, Serialize};

/// Configuration for the score network architecture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreNetworkConfig {
    /// Number of data features (input and output dimension).
    pub n_features: usize,
    /// Hidden layer dimensions (e.g., `[256, 256, 128]`).
    #[serde(default = "default_hidden_dims")]
    pub hidden_dims: Vec<usize>,
    /// Dimension of the sinusoidal timestep embedding.
    #[serde(default = "default_timestep_embed_dim")]
    pub timestep_embed_dim: usize,
}

fn default_hidden_dims() -> Vec<usize> {
    vec![256, 256, 128]
}

fn default_timestep_embed_dim() -> usize {
    64
}

impl ScoreNetworkConfig {
    /// Create a config for the given number of features with default architecture.
    pub fn new(n_features: usize) -> Self {
        Self {
            n_features,
            hidden_dims: default_hidden_dims(),
            timestep_embed_dim: default_timestep_embed_dim(),
        }
    }
}

/// MLP-based score network for denoising score matching on tabular data.
///
/// Takes noisy data concatenated with a sinusoidal timestep embedding and
/// predicts the noise component. Used by [`super::NeuralDiffusionBackend`]
/// during the reverse diffusion process.
pub struct ScoreNetwork {
    layers: Vec<Linear>,
    timestep_embed_dim: usize,
    n_features: usize,
    device: Device,
}

impl ScoreNetwork {
    /// Build a new score network from the given configuration.
    ///
    /// Parameters are initialized by the `VarBuilder` (typically from a `VarMap`
    /// for training, or from saved weights for inference).
    pub fn new(config: &ScoreNetworkConfig, vb: VarBuilder) -> CandleResult<Self> {
        let input_dim = config.n_features + config.timestep_embed_dim;
        let mut layers = Vec::new();
        let mut prev_dim = input_dim;

        for (i, &hidden_dim) in config.hidden_dims.iter().enumerate() {
            layers.push(linear(prev_dim, hidden_dim, vb.pp(format!("h{i}")))?);
            prev_dim = hidden_dim;
        }

        // Output layer: predict noise with same dimensionality as input features
        layers.push(linear(prev_dim, config.n_features, vb.pp("out"))?);

        Ok(Self {
            layers,
            timestep_embed_dim: config.timestep_embed_dim,
            n_features: config.n_features,
            device: vb.device().clone(),
        })
    }

    /// Number of data features this network was built for.
    pub fn n_features(&self) -> usize {
        self.n_features
    }

    /// Device the network lives on.
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Compute sinusoidal timestep embedding.
    ///
    /// Maps integer timesteps to a dense vector using the positional-encoding
    /// scheme from "Attention Is All You Need" / DDPM.
    ///
    /// # Arguments
    /// * `t` - Tensor of shape `(batch,)` containing integer timesteps
    ///
    /// # Returns
    /// Tensor of shape `(batch, timestep_embed_dim)`
    pub fn timestep_embedding(&self, t: &Tensor) -> CandleResult<Tensor> {
        let half_dim = self.timestep_embed_dim / 2;
        if half_dim == 0 {
            return Tensor::zeros(
                (t.dim(0)?, self.timestep_embed_dim),
                DType::F32,
                &self.device,
            );
        }

        // freq_i = exp(-ln(10000) * i / half_dim)
        let log_scale = -(10000.0_f64.ln()) / half_dim as f64;
        let freqs: Vec<f32> = (0..half_dim)
            .map(|i| (log_scale * i as f64).exp() as f32)
            .collect();
        let freqs = Tensor::from_vec(freqs, (1, half_dim), &self.device)?;

        let t_float = t.to_dtype(DType::F32)?.unsqueeze(1)?; // (batch, 1)
        let angles = t_float.broadcast_mul(&freqs)?; // (batch, half_dim)

        let sin_emb = angles.sin()?;
        let cos_emb = angles.cos()?;
        Tensor::cat(&[&sin_emb, &cos_emb], 1) // (batch, timestep_embed_dim)
    }

    /// Forward pass: predict noise given noisy data and timestep.
    ///
    /// # Arguments
    /// * `x` - Noisy data tensor of shape `(batch, n_features)`
    /// * `t` - Timestep tensor of shape `(batch,)`
    ///
    /// # Returns
    /// Predicted noise of shape `(batch, n_features)`
    pub fn forward_with_t(&self, x: &Tensor, t: &Tensor) -> CandleResult<Tensor> {
        let t_emb = self.timestep_embedding(t)?;
        let mut hidden = Tensor::cat(&[x, &t_emb], 1)?; // (batch, n_features + embed_dim)

        for (i, layer) in self.layers.iter().enumerate() {
            hidden = layer.forward(&hidden)?;
            // SiLU activation for all hidden layers, not the output
            if i < self.layers.len() - 1 {
                hidden = silu(&hidden)?;
            }
        }

        Ok(hidden)
    }
}

/// SiLU (Sigmoid Linear Unit) activation: x * sigmoid(x).
fn silu(x: &Tensor) -> CandleResult<Tensor> {
    let sigmoid = x.neg()?.exp()?.affine(1.0, 1.0)?.recip()?;
    x.mul(&sigmoid)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use candle_nn::VarMap;

    fn make_network(n_features: usize) -> (ScoreNetwork, VarMap) {
        let config = ScoreNetworkConfig {
            n_features,
            hidden_dims: vec![32, 32],
            timestep_embed_dim: 16,
        };
        let vm = VarMap::new();
        let vb = VarBuilder::from_varmap(&vm, DType::F32, &Device::Cpu);
        let net = ScoreNetwork::new(&config, vb).unwrap();
        (net, vm)
    }

    #[test]
    fn test_output_shape() {
        let (net, _vm) = make_network(5);
        let batch = 10;
        let x = Tensor::randn(0f32, 1f32, (batch, 5), &Device::Cpu).unwrap();
        let t = Tensor::from_vec(vec![50u32; batch], (batch,), &Device::Cpu).unwrap();

        let out = net.forward_with_t(&x, &t).unwrap();
        assert_eq!(out.dims(), &[batch, 5]);
    }

    #[test]
    fn test_timestep_embedding_shape() {
        let (net, _vm) = make_network(3);
        let t = Tensor::from_vec(vec![0u32, 50, 99], (3,), &Device::Cpu).unwrap();
        let emb = net.timestep_embedding(&t).unwrap();
        assert_eq!(emb.dims(), &[3, 16]);
    }

    #[test]
    fn test_different_timesteps_produce_different_embeddings() {
        let (net, _vm) = make_network(3);
        let t1 = Tensor::from_vec(vec![10u32], (1,), &Device::Cpu).unwrap();
        let t2 = Tensor::from_vec(vec![90u32], (1,), &Device::Cpu).unwrap();

        let emb1 = net.timestep_embedding(&t1).unwrap();
        let emb2 = net.timestep_embedding(&t2).unwrap();

        let diff = (&emb1 - &emb2).unwrap().sqr().unwrap().sum_all().unwrap();
        let diff_val: f32 = diff.to_scalar().unwrap();
        assert!(
            diff_val > 0.01,
            "Different timesteps should produce different embeddings"
        );
    }

    #[test]
    fn test_deterministic_forward() {
        let (net, _vm) = make_network(4);
        let x = Tensor::randn(0f32, 1f32, (5, 4), &Device::Cpu).unwrap();
        let t = Tensor::from_vec(vec![25u32; 5], (5,), &Device::Cpu).unwrap();

        let out1 = net.forward_with_t(&x, &t).unwrap();
        let out2 = net.forward_with_t(&x, &t).unwrap();

        let diff = (&out1 - &out2).unwrap().sqr().unwrap().sum_all().unwrap();
        let diff_val: f32 = diff.to_scalar().unwrap();
        assert!(diff_val < 1e-10, "Same input should produce same output");
    }

    #[test]
    fn test_silu_activation() {
        let x = Tensor::from_vec(vec![0.0f32, 1.0, -1.0, 2.0], (4,), &Device::Cpu).unwrap();
        let result = silu(&x).unwrap();
        let vals: Vec<f32> = result.to_vec1().unwrap();

        // SiLU(0) = 0 * 0.5 = 0
        assert!((vals[0]).abs() < 1e-5);
        // SiLU(1) = 1 * sigmoid(1) ≈ 0.7311
        assert!((vals[1] - 0.7311).abs() < 0.01);
        // SiLU(-1) = -1 * sigmoid(-1) ≈ -0.2689
        assert!((vals[2] - (-0.2689)).abs() < 0.01);
    }
}

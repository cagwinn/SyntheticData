//! GNN-informed graph structure generator.
//!
//! Trains a simple Graph Neural Network on entity relationship data to learn
//! plausible edge patterns, then generates new graph structures with similar
//! properties. Used to produce realistic vendor collusion rings, payment
//! networks, and other entity relationship patterns.
//!
//! Architecture: Message-passing GNN with learned node embeddings.
//! - Nodes: entities (vendors, customers, employees, accounts)
//! - Edges: relationships (transactions, approvals, ownership)
//! - Learning: edge prediction — which node pairs are likely connected?

use candle_core::{DType, Device, Tensor};
use candle_nn::{linear, Linear, Module, VarBuilder, VarMap};
use serde::{Deserialize, Serialize};

use crate::error::SynthError;

/// Configuration for the GNN graph generator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GnnGeneratorConfig {
    /// Number of node features.
    pub n_node_features: usize,
    /// Node embedding dimension.
    #[serde(default = "default_embed_dim")]
    pub embed_dim: usize,
    /// Number of message-passing layers.
    #[serde(default = "default_gnn_layers")]
    pub n_layers: usize,
}

fn default_embed_dim() -> usize {
    64
}
fn default_gnn_layers() -> usize {
    2
}

impl GnnGeneratorConfig {
    /// Create a config for the given number of node features.
    pub fn new(n_node_features: usize) -> Self {
        Self {
            n_node_features,
            embed_dim: default_embed_dim(),
            n_layers: default_gnn_layers(),
        }
    }
}

/// A single GNN message-passing layer.
///
/// Update rule: h_i' = W_self * h_i + W_neigh * mean(h_j for j in neighbors(i))
struct GnnLayer {
    w_self: Linear,
    w_neigh: Linear,
}

impl GnnLayer {
    fn new(in_dim: usize, out_dim: usize, vb: VarBuilder) -> Result<Self, candle_core::Error> {
        Ok(Self {
            w_self: linear(in_dim, out_dim, vb.pp("w_self"))?,
            w_neigh: linear(in_dim, out_dim, vb.pp("w_neigh"))?,
        })
    }

    /// Message passing: aggregate neighbor embeddings and update node embeddings.
    ///
    /// # Arguments
    /// * `node_emb` - Node embeddings of shape `(n_nodes, in_dim)`
    /// * `adj` - Adjacency matrix of shape `(n_nodes, n_nodes)`, values in [0, 1]
    fn forward(&self, node_emb: &Tensor, adj: &Tensor) -> Result<Tensor, candle_core::Error> {
        // Compute degree for normalization: D = sum(adj, dim=1)
        let degree = adj.sum(1)?.clamp(1e-8, f64::MAX)?;
        let degree_inv = degree.recip()?.unsqueeze(1)?; // (n_nodes, 1)

        // Aggregate neighbor features: agg = D^{-1} * A * H
        let neighbor_agg = adj.matmul(node_emb)?.broadcast_mul(&degree_inv)?;

        // Update: H' = ReLU(W_self * H + W_neigh * agg)
        let self_term = self.w_self.forward(node_emb)?;
        let neigh_term = self.w_neigh.forward(&neighbor_agg)?;
        let combined = (&self_term + &neigh_term)?;
        combined.relu()
    }
}

/// GNN model for edge prediction on entity relationship graphs.
///
/// After message passing, edge scores are computed as the dot product of
/// node embeddings: `score(i, j) = sigmoid(h_i . h_j)`.
pub struct GnnEdgePredictor {
    input_proj: Linear,
    layers: Vec<GnnLayer>,
    #[allow(dead_code)]
    config: GnnGeneratorConfig,
    device: Device,
}

impl GnnEdgePredictor {
    /// Build a new GNN edge predictor.
    pub fn new(config: &GnnGeneratorConfig, vb: VarBuilder) -> Result<Self, candle_core::Error> {
        let input_proj = linear(config.n_node_features, config.embed_dim, vb.pp("input"))?;

        let mut layers = Vec::new();
        for i in 0..config.n_layers {
            layers.push(GnnLayer::new(
                config.embed_dim,
                config.embed_dim,
                vb.pp(format!("gnn_{i}")),
            )?);
        }

        Ok(Self {
            input_proj,
            layers,
            config: config.clone(),
            device: vb.device().clone(),
        })
    }

    /// Run message passing and return node embeddings.
    ///
    /// # Arguments
    /// * `node_features` - Node feature matrix of shape `(n_nodes, n_node_features)`
    /// * `adj` - Adjacency matrix of shape `(n_nodes, n_nodes)`
    pub fn encode(
        &self,
        node_features: &Tensor,
        adj: &Tensor,
    ) -> Result<Tensor, candle_core::Error> {
        let mut h = self.input_proj.forward(node_features)?.relu()?;

        for layer in &self.layers {
            h = layer.forward(&h, adj)?;
        }

        Ok(h)
    }

    /// Predict edge probabilities between all node pairs.
    ///
    /// Returns a matrix of shape `(n_nodes, n_nodes)` with values in [0, 1].
    pub fn predict_edges(
        &self,
        node_features: &Tensor,
        adj: &Tensor,
    ) -> Result<Tensor, candle_core::Error> {
        let h = self.encode(node_features, adj)?;

        // Edge scores: sigmoid(H * H^T)
        let h_t = h.t()?;
        let scores = h.matmul(&h_t)?;

        // Sigmoid
        let neg_scores = scores.neg()?;
        let exp_neg = neg_scores.exp()?;
        let one_plus_exp = exp_neg.affine(1.0, 1.0)?;
        one_plus_exp.recip()
    }
}

/// Training configuration for the GNN graph generator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GnnTrainingConfig {
    /// GNN architecture.
    pub model: GnnGeneratorConfig,
    /// Learning rate.
    #[serde(default = "default_gnn_lr")]
    pub learning_rate: f64,
    /// Training epochs.
    #[serde(default = "default_gnn_epochs")]
    pub epochs: usize,
    /// Negative sampling ratio (negatives per positive edge).
    #[serde(default = "default_neg_ratio")]
    pub neg_ratio: usize,
}

fn default_gnn_lr() -> f64 {
    1e-3
}
fn default_gnn_epochs() -> usize {
    50
}
fn default_neg_ratio() -> usize {
    3
}

/// A trained GNN model ready for graph generation.
pub struct TrainedGnnGenerator {
    predictor: GnnEdgePredictor,
    var_map: VarMap,
    config: GnnTrainingConfig,
}

impl TrainedGnnGenerator {
    /// Generate a new adjacency matrix given node features.
    ///
    /// Uses the trained GNN to predict edge probabilities, then samples
    /// edges above the given threshold.
    ///
    /// # Arguments
    /// * `node_features` - Features for the nodes to connect. Shape: `n_nodes x n_features`.
    /// * `threshold` - Minimum edge probability to create an edge (0.0-1.0).
    /// * `seed_adj` - Optional seed adjacency for the GNN's message passing.
    ///   If None, uses an identity matrix (no initial connections).
    pub fn generate(
        &self,
        node_features: &[Vec<f64>],
        threshold: f64,
        seed_adj: Option<&[Vec<f64>]>,
    ) -> Result<Vec<Vec<bool>>, SynthError> {
        let n_nodes = node_features.len();
        let n_feat = self.config.model.n_node_features;
        if n_nodes == 0 {
            return Ok(vec![]);
        }

        let device = &self.predictor.device;

        // Build feature tensor
        let flat: Vec<f32> = node_features
            .iter()
            .flat_map(|r| r.iter().map(|&v| v as f32))
            .collect();
        let feat_tensor = Tensor::from_vec(flat, (n_nodes, n_feat), device)
            .map_err(|e| SynthError::generation(format!("Feature tensor: {e}")))?;

        // Build adjacency tensor
        let adj_tensor = if let Some(adj) = seed_adj {
            let flat: Vec<f32> = adj
                .iter()
                .flat_map(|r| r.iter().map(|&v| v as f32))
                .collect();
            Tensor::from_vec(flat, (n_nodes, n_nodes), device)
                .map_err(|e| SynthError::generation(format!("Adj tensor: {e}")))?
        } else {
            // Identity + small random for initial message passing
            Tensor::eye(n_nodes, DType::F32, device)
                .map_err(|e| SynthError::generation(format!("Eye tensor: {e}")))?
        };

        // Predict edge probabilities
        let probs = self
            .predictor
            .predict_edges(&feat_tensor, &adj_tensor)
            .map_err(|e| SynthError::generation(format!("Edge prediction: {e}")))?;

        let probs_data: Vec<Vec<f32>> = probs
            .to_vec2()
            .map_err(|e| SynthError::generation(format!("Probs to vec: {e}")))?;

        // Threshold to binary adjacency
        let threshold_f32 = threshold as f32;
        Ok(probs_data
            .iter()
            .enumerate()
            .map(|(i, row)| {
                row.iter()
                    .enumerate()
                    .map(|(j, &p)| i != j && p >= threshold_f32) // No self-loops
                    .collect()
            })
            .collect())
    }

    /// Save the model.
    pub fn save(&self, dir: &std::path::Path) -> Result<(), SynthError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| SynthError::generation(format!("Create dir: {e}")))?;

        let meta = serde_json::to_string_pretty(&self.config)
            .map_err(|e| SynthError::generation(format!("Serialize config: {e}")))?;
        std::fs::write(dir.join("gnn_config.json"), meta)
            .map_err(|e| SynthError::generation(format!("Write config: {e}")))?;

        self.var_map
            .save(dir.join("gnn_weights.safetensors"))
            .map_err(|e| SynthError::generation(format!("Save weights: {e}")))?;

        Ok(())
    }
}

/// Train a GNN graph generator from observed graph data.
pub struct GnnGraphTrainer;

impl GnnGraphTrainer {
    /// Train a GNN edge predictor from node features and an adjacency matrix.
    ///
    /// Uses binary cross-entropy loss with negative sampling.
    pub fn train(
        node_features: &[Vec<f64>],
        adjacency: &[Vec<f64>],
        config: &GnnTrainingConfig,
        _seed: u64,
    ) -> Result<TrainedGnnGenerator, SynthError> {
        let n_nodes = node_features.len();
        let n_feat = config.model.n_node_features;
        if n_nodes == 0 {
            return Err(SynthError::generation("Training data must be non-empty"));
        }

        let device = Device::Cpu;

        // Build tensors
        let feat_flat: Vec<f32> = node_features
            .iter()
            .flat_map(|r| r.iter().map(|&v| v as f32))
            .collect();
        let feat_tensor = Tensor::from_vec(feat_flat, (n_nodes, n_feat), &device)
            .map_err(|e| SynthError::generation(format!("Feature tensor: {e}")))?;

        let adj_flat: Vec<f32> = adjacency
            .iter()
            .flat_map(|r| r.iter().map(|&v| v as f32))
            .collect();
        let adj_tensor = Tensor::from_vec(adj_flat, (n_nodes, n_nodes), &device)
            .map_err(|e| SynthError::generation(format!("Adj tensor: {e}")))?;

        // Build model
        let var_map = VarMap::new();
        let vb = VarBuilder::from_varmap(&var_map, DType::F32, &device);
        let predictor = GnnEdgePredictor::new(&config.model, vb)
            .map_err(|e| SynthError::generation(format!("Build GNN: {e}")))?;

        // Optimizer
        let params = var_map.all_vars();
        let mut optimizer = candle_nn::optim::AdamW::new_lr(params, config.learning_rate)
            .map_err(|e| SynthError::generation(format!("Optimizer: {e}")))?;

        // Pre-compute constant for BCE loss (adj_tensor doesn't change)
        let one_minus_adj =
            (1.0 - &adj_tensor).map_err(|e| SynthError::generation(format!("1-y: {e}")))?;

        use candle_nn::Optimizer;

        // Training loop: minimize BCE between predicted and actual adjacency
        for epoch in 0..config.epochs {
            let predicted = predictor
                .predict_edges(&feat_tensor, &adj_tensor)
                .map_err(|e| SynthError::generation(format!("Forward: {e}")))?;

            // Binary cross-entropy: -[y * log(p) + (1-y) * log(1-p)]
            let eps = 1e-7;
            let predicted_clamped = predicted
                .clamp(eps, 1.0 - eps)
                .map_err(|e| SynthError::generation(format!("Clamp: {e}")))?;
            let log_p = predicted_clamped
                .log()
                .map_err(|e| SynthError::generation(format!("Log: {e}")))?;
            let log_1mp = (1.0 - &predicted_clamped)
                .map_err(|e| SynthError::generation(format!("1-p: {e}")))?
                .log()
                .map_err(|e| SynthError::generation(format!("Log(1-p): {e}")))?;

            let term1 = adj_tensor
                .mul(&log_p)
                .map_err(|e| SynthError::generation(format!("y*log(p): {e}")))?;
            let term2 = one_minus_adj
                .mul(&log_1mp)
                .map_err(|e| SynthError::generation(format!("(1-y)*log(1-p): {e}")))?;

            let loss = (&term1 + &term2)
                .map_err(|e| SynthError::generation(format!("Sum terms: {e}")))?
                .neg()
                .map_err(|e| SynthError::generation(format!("Neg: {e}")))?
                .mean_all()
                .map_err(|e| SynthError::generation(format!("Mean: {e}")))?;

            optimizer
                .backward_step(&loss)
                .map_err(|e| SynthError::generation(format!("Optimizer: {e}")))?;

            if epoch % 10 == 0 || epoch == config.epochs - 1 {
                let loss_val: f32 = loss
                    .to_scalar()
                    .map_err(|e| SynthError::generation(format!("Loss: {e}")))?;
                tracing::debug!(
                    "GNN epoch {}/{}: loss = {:.6}",
                    epoch + 1,
                    config.epochs,
                    loss_val
                );
            }
        }

        Ok(TrainedGnnGenerator {
            predictor,
            var_map,
            config: config.clone(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Create a simple test graph: 5 nodes in a ring.
    fn ring_graph() -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
        let n = 5;
        let features: Vec<Vec<f64>> = (0..n).map(|i| vec![i as f64, (n - i) as f64]).collect();
        let mut adj = vec![vec![0.0; n]; n];
        for i in 0..n {
            adj[i][(i + 1) % n] = 1.0;
            adj[(i + 1) % n][i] = 1.0;
        }
        (features, adj)
    }

    #[test]
    fn test_gnn_encode_shape() {
        let config = GnnGeneratorConfig {
            n_node_features: 2,
            embed_dim: 16,
            n_layers: 1,
        };
        let vm = VarMap::new();
        let vb = VarBuilder::from_varmap(&vm, DType::F32, &Device::Cpu);
        let model = GnnEdgePredictor::new(&config, vb).unwrap();

        let features = Tensor::randn(0f32, 1f32, (5, 2), &Device::Cpu).unwrap();
        let adj = Tensor::eye(5, DType::F32, &Device::Cpu).unwrap();

        let embeddings = model.encode(&features, &adj).unwrap();
        assert_eq!(embeddings.dims(), &[5, 16]);
    }

    #[test]
    fn test_gnn_predict_edges_shape() {
        let config = GnnGeneratorConfig::new(2);
        let vm = VarMap::new();
        let vb = VarBuilder::from_varmap(&vm, DType::F32, &Device::Cpu);
        let model = GnnEdgePredictor::new(&config, vb).unwrap();

        let features = Tensor::randn(0f32, 1f32, (5, 2), &Device::Cpu).unwrap();
        let adj = Tensor::eye(5, DType::F32, &Device::Cpu).unwrap();

        let probs = model.predict_edges(&features, &adj).unwrap();
        assert_eq!(probs.dims(), &[5, 5]);

        // All probabilities should be in [0, 1] (sigmoid output)
        let vals: Vec<Vec<f32>> = probs.to_vec2().unwrap();
        for row in &vals {
            for &v in row {
                assert!(v >= 0.0 && v <= 1.0, "Probability out of range: {v}");
            }
        }
    }

    #[test]
    fn test_train_ring_graph() {
        let (features, adj) = ring_graph();
        let config = GnnTrainingConfig {
            model: GnnGeneratorConfig {
                n_node_features: 2,
                embed_dim: 16,
                n_layers: 1,
            },
            learning_rate: 1e-2,
            epochs: 10,
            neg_ratio: 1,
        };

        let trained = GnnGraphTrainer::train(&features, &adj, &config, 42).unwrap();

        let generated = trained.generate(&features, 0.5, Some(&adj)).unwrap();
        assert_eq!(generated.len(), 5);
        for row in &generated {
            assert_eq!(row.len(), 5);
        }
        // No self-loops
        for (i, row) in generated.iter().enumerate() {
            assert!(!row[i], "No self-loops expected");
        }
    }

    #[test]
    fn test_generate_empty() {
        let config = GnnTrainingConfig {
            model: GnnGeneratorConfig::new(2),
            learning_rate: 1e-3,
            epochs: 2,
            neg_ratio: 1,
        };
        let (features, adj) = ring_graph();
        let trained = GnnGraphTrainer::train(&features, &adj, &config, 42).unwrap();

        let result = trained.generate(&[], 0.5, None).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_train_empty_fails() {
        let config = GnnTrainingConfig {
            model: GnnGeneratorConfig::new(2),
            learning_rate: 1e-3,
            epochs: 2,
            neg_ratio: 1,
        };
        assert!(GnnGraphTrainer::train(&[], &[], &config, 42).is_err());
    }

    #[test]
    fn test_save_model() {
        let (features, adj) = ring_graph();
        let config = GnnTrainingConfig {
            model: GnnGeneratorConfig {
                n_node_features: 2,
                embed_dim: 8,
                n_layers: 1,
            },
            learning_rate: 1e-2,
            epochs: 2,
            neg_ratio: 1,
        };
        let trained = GnnGraphTrainer::train(&features, &adj, &config, 42).unwrap();

        let dir = tempfile::tempdir().unwrap();
        trained.save(dir.path()).unwrap();
        assert!(dir.path().join("gnn_config.json").exists());
        assert!(dir.path().join("gnn_weights.safetensors").exists());
    }
}

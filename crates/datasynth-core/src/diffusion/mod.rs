//! Diffusion model abstraction for statistical and neural data generation.
//!
//! Implements two diffusion backends behind a common [`DiffusionBackend`] trait:
//!
//! - **Statistical** (always available): pure-Rust Langevin-inspired denoising
//!   guided by target means, stds, and Cholesky-decomposed correlations.
//!
//! - **Neural** (requires `neural` feature): learned score network trained via
//!   denoising score matching. Captures nonlinear cross-column dependencies
//!   that parametric models miss. Powered by `candle`.
//!
//! Both backends slot into [`HybridGenerator`] for blended rule+diffusion output.

pub mod backend;
pub mod hybrid;
pub mod schedule;
pub mod statistical;
pub mod training;
pub mod utils;

#[cfg(feature = "neural")]
pub mod gnn_generator;
#[cfg(feature = "neural")]
pub mod neural;
#[cfg(feature = "neural")]
pub mod neural_training;
#[cfg(feature = "neural")]
pub mod score_network;
#[cfg(feature = "neural")]
pub mod tabular_transformer;

pub use backend::*;
pub use hybrid::*;
pub use schedule::*;
pub use statistical::*;
pub use training::*;
pub use utils::*;

#[cfg(feature = "neural")]
pub use gnn_generator::{
    GnnEdgePredictor, GnnGeneratorConfig, GnnGraphTrainer, GnnTrainingConfig, TrainedGnnGenerator,
};
#[cfg(feature = "neural")]
pub use neural::{NeuralDiffusionBackend, NeuralDiffusionConfig};
#[cfg(feature = "neural")]
pub use neural_training::{NeuralDiffusionTrainer, NeuralTrainingConfig, TrainingReport};
#[cfg(feature = "neural")]
pub use score_network::ScoreNetworkConfig;
#[cfg(feature = "neural")]
pub use tabular_transformer::{
    TabularTransformer, TabularTransformerConfig, TabularTransformerTrainer,
    TabularTransformerTrainingConfig, TrainedTabularTransformer,
};

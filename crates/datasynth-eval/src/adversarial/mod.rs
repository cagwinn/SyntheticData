//! Adversarial testing module for evaluating ML models against synthetic data.
//!
//! Loads customer fraud detection models via ONNX Runtime and generates synthetic
//! data that probes their decision boundaries. This enables:
//!
//! - **Robustness testing**: How does model performance degrade under distribution shifts?
//! - **Fairness auditing**: Do predictions change when protected attributes are varied?
//! - **Boundary probing**: What does the model's decision surface look like near thresholds?
//!
//! Requires the `adversarial` cargo feature.

mod model_probe;

pub use model_probe::{ModelProbe, ModelProbeConfig, PredictionStats, ProbeResult, ProbeSample};

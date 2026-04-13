//! ONNX model probing for adversarial testing.
//!
//! Loads a customer's fraud detection (or other classification) model and
//! probes its decision boundaries by generating synthetic feature vectors
//! near the decision threshold.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Configuration for model probing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProbeConfig {
    /// Number of features the model expects as input.
    pub n_features: usize,
    /// Number of probe samples to generate per boundary point.
    #[serde(default = "default_n_probes")]
    pub n_probes: usize,
    /// Perturbation budget: max fraction of each feature to perturb (0.0-1.0).
    #[serde(default = "default_perturbation_budget")]
    pub perturbation_budget: f64,
    /// Decision threshold for binary classification.
    #[serde(default = "default_threshold")]
    pub threshold: f64,
    /// Index of the output class to probe (for multi-class models).
    #[serde(default)]
    pub target_class: usize,
}

fn default_n_probes() -> usize {
    1000
}
fn default_perturbation_budget() -> f64 {
    0.05
}
fn default_threshold() -> f64 {
    0.5
}

/// A single probe sample and the model's response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeSample {
    /// The input feature vector.
    pub features: Vec<f32>,
    /// Model's raw prediction score.
    pub score: f32,
    /// Whether the model classified this as positive (score >= threshold).
    pub predicted_positive: bool,
    /// Distance from the decision threshold.
    pub margin: f32,
}

/// Aggregated prediction statistics from probing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionStats {
    /// Mean prediction score across all probes.
    pub mean_score: f64,
    /// Std deviation of prediction scores.
    pub std_score: f64,
    /// Fraction classified as positive.
    pub positive_rate: f64,
    /// Number of samples near the decision boundary (margin < 0.1).
    pub boundary_samples: usize,
    /// Mean absolute margin (distance from threshold).
    pub mean_margin: f64,
}

/// Complete result of a model probing session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    /// All probe samples.
    pub samples: Vec<ProbeSample>,
    /// Aggregated statistics.
    pub stats: PredictionStats,
    /// Configuration used.
    pub config: ModelProbeConfig,
}

/// Loads and probes ONNX models for adversarial testing.
///
/// The probing process:
/// 1. Load the model from an ONNX file
/// 2. Generate synthetic feature vectors (random perturbations around seeds)
/// 3. Run inference and collect predictions
/// 4. Identify samples near the decision boundary
/// 5. Report statistics and boundary-proximate samples
pub struct ModelProbe {
    session: ort::session::Session,
    config: ModelProbeConfig,
}

impl ModelProbe {
    /// Load an ONNX model from a file path.
    pub fn load(model_path: &Path, config: ModelProbeConfig) -> Result<Self, String> {
        let session = ort::session::Session::builder()
            .and_then(|mut b| b.commit_from_file(model_path))
            .map_err(|e| {
                format!(
                    "Failed to load ONNX model from {}: {e}",
                    model_path.display()
                )
            })?;

        Ok(Self { session, config })
    }

    /// Run inference on a batch of feature vectors.
    ///
    /// Returns raw prediction scores (one per sample).
    pub fn predict(&mut self, features: &[Vec<f32>]) -> Result<Vec<f32>, String> {
        if features.is_empty() {
            return Ok(vec![]);
        }

        let n_samples = features.len();
        let n_features = self.config.n_features;

        // Flatten to a contiguous array
        let flat: Vec<f32> = features.iter().flat_map(|r| r.iter().copied()).collect();

        let input_tensor =
            ort::value::Tensor::<f32>::from_array(([n_samples as i64, n_features as i64], flat))
                .map_err(|e| format!("Failed to create input tensor: {e}"))?;

        let outputs = self
            .session
            .run(ort::inputs![input_tensor])
            .map_err(|e| format!("Inference failed: {e}"))?;

        // Extract predictions from first output
        let (_name, output) = outputs.iter().next().ok_or("Model returned no outputs")?;

        let (_shape, scores_view) = output
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Failed to extract output tensor: {e}"))?;

        let scores: Vec<f32> = scores_view.to_vec();

        // For multi-class, extract the target class column
        if scores.len() == n_samples {
            Ok(scores)
        } else if scores.len() >= n_samples * (self.config.target_class + 1) {
            let n_classes = scores.len() / n_samples;
            Ok(scores
                .chunks(n_classes)
                .map(|chunk| chunk.get(self.config.target_class).copied().unwrap_or(0.0))
                .collect())
        } else {
            Ok(scores.into_iter().take(n_samples).collect())
        }
    }

    /// Probe the model by generating random perturbations around seed samples.
    pub fn probe(&mut self, seed_samples: &[Vec<f32>], seed: u64) -> Result<ProbeResult, String> {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;
        use rand_distr::{Distribution, Normal};

        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let normal = Normal::new(0.0, 1.0).map_err(|e| format!("Normal dist: {e}"))?;

        let n_features = self.config.n_features;
        let budget = self.config.perturbation_budget as f32;
        let threshold = self.config.threshold as f32;

        // Generate probe samples by perturbing seeds
        let mut probe_features = Vec::with_capacity(self.config.n_probes);

        for i in 0..self.config.n_probes {
            let base = if seed_samples.is_empty() {
                (0..n_features)
                    .map(|_| {
                        let v: f64 = normal.sample(&mut rng);
                        v as f32
                    })
                    .collect::<Vec<_>>()
            } else {
                seed_samples[i % seed_samples.len()].clone()
            };

            let perturbed: Vec<f32> = base
                .iter()
                .map(|&v| {
                    let delta: f64 = normal.sample(&mut rng);
                    v + budget * v.abs().max(1.0) * delta as f32
                })
                .collect();

            probe_features.push(perturbed);
        }

        // Run inference
        let scores = self.predict(&probe_features)?;

        // Build probe samples
        let samples: Vec<ProbeSample> = probe_features
            .iter()
            .zip(scores.iter())
            .map(|(feat, &score)| {
                let margin = (score - threshold).abs();
                ProbeSample {
                    features: feat.clone(),
                    score,
                    predicted_positive: score >= threshold,
                    margin,
                }
            })
            .collect();

        let stats = compute_stats(&samples);

        Ok(ProbeResult {
            samples,
            stats,
            config: self.config.clone(),
        })
    }

    /// Find samples closest to the decision boundary.
    pub fn boundary_samples(result: &ProbeResult, top_n: usize) -> Vec<&ProbeSample> {
        let mut sorted: Vec<&ProbeSample> = result.samples.iter().collect();
        sorted.sort_by(|a, b| {
            a.margin
                .partial_cmp(&b.margin)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted.truncate(top_n);
        sorted
    }
}

/// Compute aggregate prediction statistics.
fn compute_stats(samples: &[ProbeSample]) -> PredictionStats {
    if samples.is_empty() {
        return PredictionStats {
            mean_score: 0.0,
            std_score: 0.0,
            positive_rate: 0.0,
            boundary_samples: 0,
            mean_margin: 0.0,
        };
    }

    let n = samples.len() as f64;
    let mean_score = samples.iter().map(|s| s.score as f64).sum::<f64>() / n;
    let std_score = (samples
        .iter()
        .map(|s| (s.score as f64 - mean_score).powi(2))
        .sum::<f64>()
        / n)
        .sqrt();
    let positive_rate = samples.iter().filter(|s| s.predicted_positive).count() as f64 / n;
    let boundary_samples = samples.iter().filter(|s| s.margin < 0.1).count();
    let mean_margin = samples.iter().map(|s| s.margin as f64).sum::<f64>() / n;

    PredictionStats {
        mean_score,
        std_score,
        positive_rate,
        boundary_samples,
        mean_margin,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_probe_config_defaults() {
        let config = ModelProbeConfig {
            n_features: 10,
            n_probes: default_n_probes(),
            perturbation_budget: default_perturbation_budget(),
            threshold: default_threshold(),
            target_class: 0,
        };
        assert_eq!(config.n_probes, 1000);
        assert!((config.perturbation_budget - 0.05).abs() < 1e-10);
        assert!((config.threshold - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_prediction_stats_serialization() {
        let stats = PredictionStats {
            mean_score: 0.45,
            std_score: 0.15,
            positive_rate: 0.3,
            boundary_samples: 50,
            mean_margin: 0.12,
        };
        let json = serde_json::to_string(&stats).unwrap();
        let parsed: PredictionStats = serde_json::from_str(&json).unwrap();
        assert!((parsed.mean_score - 0.45).abs() < 1e-10);
    }

    #[test]
    fn test_boundary_samples_sorting() {
        let result = ProbeResult {
            samples: vec![
                ProbeSample {
                    features: vec![1.0],
                    score: 0.9,
                    predicted_positive: true,
                    margin: 0.4,
                },
                ProbeSample {
                    features: vec![2.0],
                    score: 0.51,
                    predicted_positive: true,
                    margin: 0.01,
                },
                ProbeSample {
                    features: vec![3.0],
                    score: 0.3,
                    predicted_positive: false,
                    margin: 0.2,
                },
            ],
            stats: compute_stats(&[]),
            config: ModelProbeConfig {
                n_features: 1,
                n_probes: 3,
                perturbation_budget: 0.05,
                threshold: 0.5,
                target_class: 0,
            },
        };

        let top = ModelProbe::boundary_samples(&result, 2);
        assert_eq!(top.len(), 2);
        assert!(top[0].margin <= top[1].margin);
        assert!((top[0].margin - 0.01).abs() < 1e-5);
    }

    #[test]
    fn test_compute_stats_empty() {
        let stats = compute_stats(&[]);
        assert!((stats.mean_score).abs() < 1e-10);
        assert_eq!(stats.boundary_samples, 0);
    }

    #[test]
    fn test_compute_stats_basic() {
        let samples = vec![
            ProbeSample {
                features: vec![],
                score: 0.8,
                predicted_positive: true,
                margin: 0.3,
            },
            ProbeSample {
                features: vec![],
                score: 0.2,
                predicted_positive: false,
                margin: 0.3,
            },
        ];
        let stats = compute_stats(&samples);
        assert!((stats.mean_score - 0.5).abs() < 1e-6);
        assert!((stats.positive_rate - 0.5).abs() < 1e-10);
    }

    // Note: ModelProbe::load/predict/probe tests require an actual ONNX model file.
    // Integration tests with real models should be added separately.
}

//! AI-powered evaluation-driven tuning loop.
//!
//! Wraps [`AutoTuner`] and [`RecommendationEngine`] with an LLM interpretation
//! layer that provides intelligent gap analysis and creative config suggestions
//! beyond what the rule-based tuner can derive.
//!
//! The tuning loop:
//! 1. Evaluate generated data against quality thresholds
//! 2. AutoTuner produces rule-based config patches
//! 3. LLM interprets remaining gaps and suggests additional patches
//! 4. Patches are merged, applied, and the cycle repeats
//! 5. Convergence when health score stabilizes or max iterations reached

use serde::{Deserialize, Serialize};

use datasynth_core::llm::provider::{LlmProvider, LlmRequest};

use super::auto_tuner::{AutoTuneResult, AutoTuner, ConfigPatch};
use super::recommendation_engine::{EnhancementReport, RecommendationEngine};
use crate::config::EvaluationThresholds;
use crate::ComprehensiveEvaluation;

/// Configuration for the AI tuning loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiTunerConfig {
    /// Maximum number of tuning iterations.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    /// Stop if health score improvement is below this threshold.
    #[serde(default = "default_convergence_threshold")]
    pub convergence_threshold: f64,
    /// Minimum confidence to accept an LLM-suggested patch.
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f64,
    /// Whether to include LLM-generated patches (false = rule-based only).
    #[serde(default = "default_use_llm")]
    pub use_llm: bool,
}

fn default_max_iterations() -> usize {
    5
}
fn default_convergence_threshold() -> f64 {
    0.01
}
fn default_min_confidence() -> f64 {
    0.5
}
fn default_use_llm() -> bool {
    true
}

impl Default for AiTunerConfig {
    fn default() -> Self {
        Self {
            max_iterations: default_max_iterations(),
            convergence_threshold: default_convergence_threshold(),
            min_confidence: default_min_confidence(),
            use_llm: default_use_llm(),
        }
    }
}

/// Result of a single tuning iteration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuningIteration {
    /// Iteration number (1-based).
    pub iteration: usize,
    /// Health score at the start of this iteration.
    pub health_score: f64,
    /// Number of failures at the start.
    pub failure_count: usize,
    /// Rule-based patches from AutoTuner.
    pub rule_patches: Vec<ConfigPatch>,
    /// AI-suggested patches from LLM interpretation.
    pub ai_patches: Vec<ConfigPatch>,
    /// Combined patches applied this iteration.
    pub applied_patches: Vec<ConfigPatch>,
}

/// Complete result of the AI tuning loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiTuneResult {
    /// All iterations performed.
    pub iterations: Vec<TuningIteration>,
    /// Final combined patches (union of all iterations).
    pub final_patches: Vec<ConfigPatch>,
    /// Initial health score.
    pub initial_health_score: f64,
    /// Final health score.
    pub final_health_score: f64,
    /// Whether convergence was reached (vs max iterations).
    pub converged: bool,
    /// Human-readable summary.
    pub summary: String,
}

impl AiTuneResult {
    /// Health score improvement from initial to final.
    pub fn improvement(&self) -> f64 {
        self.final_health_score - self.initial_health_score
    }
}

/// AI-powered auto-tuner that combines rule-based analysis with LLM interpretation.
pub struct AiTuner<'a> {
    auto_tuner: AutoTuner,
    recommendation_engine: RecommendationEngine,
    provider: &'a dyn LlmProvider,
    config: AiTunerConfig,
}

impl<'a> AiTuner<'a> {
    /// Create a new AI tuner with default thresholds.
    pub fn new(provider: &'a dyn LlmProvider, config: AiTunerConfig) -> Self {
        Self {
            auto_tuner: AutoTuner::new(),
            recommendation_engine: RecommendationEngine::new(),
            provider,
            config,
        }
    }

    /// Create with custom evaluation thresholds.
    pub fn with_thresholds(
        provider: &'a dyn LlmProvider,
        config: AiTunerConfig,
        thresholds: EvaluationThresholds,
    ) -> Self {
        Self {
            auto_tuner: AutoTuner::with_thresholds(thresholds.clone()),
            recommendation_engine: RecommendationEngine::with_thresholds(thresholds),
            provider,
            config,
        }
    }

    /// Run a single tuning iteration: analyze evaluation results and produce patches.
    ///
    /// This is the core method for integrating into a generation loop.
    pub fn analyze_iteration(
        &mut self,
        evaluation: &ComprehensiveEvaluation,
        iteration: usize,
    ) -> TuningIteration {
        // Rule-based analysis
        let auto_result = self.auto_tuner.analyze(evaluation);
        let report = self.recommendation_engine.generate_report(evaluation);

        let rule_patches = auto_result.patches.clone();

        // LLM-powered analysis of remaining gaps
        let ai_patches = if self.config.use_llm && !auto_result.unaddressable_metrics.is_empty() {
            self.llm_analyze_gaps(&auto_result, &report)
        } else {
            vec![]
        };

        // Merge patches: rule-based first, then AI suggestions that don't conflict
        let applied_patches = merge_patches(&rule_patches, &ai_patches, self.config.min_confidence);

        TuningIteration {
            iteration,
            health_score: report.health_score,
            failure_count: evaluation.failures.len(),
            rule_patches,
            ai_patches,
            applied_patches,
        }
    }

    /// Use LLM to interpret gaps that the rule-based tuner couldn't address.
    fn llm_analyze_gaps(
        &self,
        auto_result: &AutoTuneResult,
        report: &EnhancementReport,
    ) -> Vec<ConfigPatch> {
        let prompt = self.build_gap_analysis_prompt(auto_result, report);

        let request = LlmRequest::new(prompt)
            .with_system(Self::tuning_system_prompt().to_string())
            .with_temperature(0.3)
            .with_max_tokens(2048);

        match self.provider.complete(&request) {
            Ok(response) => self.parse_llm_patches(&response.content),
            Err(e) => {
                tracing::warn!("LLM gap analysis failed: {e}");
                vec![]
            }
        }
    }

    /// Build a structured prompt describing the evaluation gaps.
    fn build_gap_analysis_prompt(
        &self,
        auto_result: &AutoTuneResult,
        report: &EnhancementReport,
    ) -> String {
        let mut prompt = String::with_capacity(2048);

        prompt
            .push_str("Analyze these synthetic data quality gaps and suggest config patches.\n\n");

        // Unaddressable metrics
        if !auto_result.unaddressable_metrics.is_empty() {
            prompt.push_str("## Metrics the rule-based tuner could not address:\n");
            for metric in &auto_result.unaddressable_metrics {
                prompt.push_str(&format!("- {metric}\n"));
            }
            prompt.push('\n');
        }

        // Top issues
        if !report.top_issues.is_empty() {
            prompt.push_str("## Top issues:\n");
            for issue in &report.top_issues {
                prompt.push_str(&format!("- {issue}\n"));
            }
            prompt.push('\n');
        }

        // Already applied patches (to avoid duplicates)
        if auto_result.has_patches() {
            prompt.push_str("## Already suggested patches (do not repeat):\n");
            for patch in &auto_result.patches {
                prompt.push_str(&format!("- {}: {}\n", patch.path, patch.suggested_value));
            }
            prompt.push('\n');
        }

        prompt.push_str(&format!(
            "Current health score: {:.2}\n",
            report.health_score
        ));
        prompt
    }

    /// Parse LLM response into config patches.
    fn parse_llm_patches(&self, content: &str) -> Vec<ConfigPatch> {
        // Try to find JSON array of patches in the response
        let json_str = datasynth_core::llm::extract_json_array(content);

        match json_str {
            Some(json) => match serde_json::from_str::<Vec<LlmPatchSuggestion>>(json) {
                Ok(suggestions) => suggestions
                    .into_iter()
                    .filter(|s| s.confidence >= self.config.min_confidence)
                    .map(|s| {
                        ConfigPatch::new(s.path, s.value)
                            .with_confidence(s.confidence)
                            .with_impact(s.reasoning)
                    })
                    .collect(),
                Err(e) => {
                    tracing::debug!("Failed to parse LLM patches as JSON: {e}");
                    vec![]
                }
            },
            None => {
                tracing::debug!("No JSON array found in LLM response");
                vec![]
            }
        }
    }

    /// System prompt for the LLM gap analyzer.
    fn tuning_system_prompt() -> &'static str {
        concat!(
            "You are a synthetic data quality tuner for DataSynth. ",
            "Given evaluation gaps, suggest config patches to improve data quality.\n\n",
            "Return a JSON array of patches. Each patch has:\n",
            "- path: dot-separated config path (e.g., \"distributions.amounts.components[0].mu\")\n",
            "- value: new value as string\n",
            "- confidence: 0.0-1.0 confidence this will help\n",
            "- reasoning: one sentence explaining why\n\n",
            "Valid config paths include:\n",
            "- transactions.count, transactions.anomaly_rate\n",
            "- distributions.amounts.*, distributions.correlations.*\n",
            "- temporal_patterns.period_end.*, temporal_patterns.intraday.*\n",
            "- anomaly_injection.base_rate, anomaly_injection.types\n",
            "- data_quality.missing_value_rate, data_quality.typo_rate\n",
            "- fraud.injection_rate, fraud.types\n",
            "- graph_export.ensure_connected\n\n",
            "Rules:\n",
            "- Only suggest patches for unaddressed metrics\n",
            "- Don't repeat patches already applied\n",
            "- Keep confidence realistic\n",
            "- Return ONLY the JSON array, no other text\n"
        )
    }
}

/// An LLM-suggested patch (deserialized from JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LlmPatchSuggestion {
    path: String,
    value: String,
    #[serde(default = "default_llm_confidence")]
    confidence: f64,
    #[serde(default)]
    reasoning: String,
}

fn default_llm_confidence() -> f64 {
    0.5
}

/// Merge rule-based and AI patches, filtering by confidence and removing conflicts.
fn merge_patches(
    rule_patches: &[ConfigPatch],
    ai_patches: &[ConfigPatch],
    min_confidence: f64,
) -> Vec<ConfigPatch> {
    let mut merged = rule_patches.to_vec();

    // Track paths already covered by rule-based patches
    let rule_paths: std::collections::HashSet<&str> =
        rule_patches.iter().map(|p| p.path.as_str()).collect();

    // Add AI patches that don't conflict with rule-based ones
    for patch in ai_patches {
        if patch.confidence >= min_confidence && !rule_paths.contains(patch.path.as_str()) {
            merged.push(patch.clone());
        }
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use datasynth_core::llm::MockLlmProvider;

    #[test]
    fn test_ai_tuner_single_iteration() {
        let provider = MockLlmProvider::new(42);
        let config = AiTunerConfig {
            max_iterations: 1,
            use_llm: false, // Rule-based only for deterministic test
            ..Default::default()
        };
        let mut tuner = AiTuner::new(&provider, config);

        let evaluation = ComprehensiveEvaluation::new();
        let iteration = tuner.analyze_iteration(&evaluation, 1);

        assert_eq!(iteration.iteration, 1);
        assert!(iteration.ai_patches.is_empty());
        // A passing evaluation should have no failures
        assert_eq!(iteration.failure_count, 0);
    }

    #[test]
    fn test_ai_tuner_config_defaults() {
        let config = AiTunerConfig::default();
        assert_eq!(config.max_iterations, 5);
        assert!((config.convergence_threshold - 0.01).abs() < 1e-10);
        assert!((config.min_confidence - 0.5).abs() < 1e-10);
        assert!(config.use_llm);
    }

    #[test]
    fn test_merge_patches_no_conflicts() {
        let rule = vec![
            ConfigPatch::new("path.a", "1").with_confidence(0.9),
            ConfigPatch::new("path.b", "2").with_confidence(0.8),
        ];
        let ai = vec![
            ConfigPatch::new("path.c", "3").with_confidence(0.7),
            ConfigPatch::new("path.d", "4").with_confidence(0.3), // Below threshold
        ];

        let merged = merge_patches(&rule, &ai, 0.5);
        assert_eq!(merged.len(), 3); // a, b, c (d filtered by confidence)
    }

    #[test]
    fn test_merge_patches_with_conflicts() {
        let rule = vec![ConfigPatch::new("path.a", "1").with_confidence(0.9)];
        let ai = vec![
            ConfigPatch::new("path.a", "2").with_confidence(0.8), // Conflicts
            ConfigPatch::new("path.b", "3").with_confidence(0.7),
        ];

        let merged = merge_patches(&rule, &ai, 0.5);
        assert_eq!(merged.len(), 2); // a (rule) + b (ai, no conflict)
        assert_eq!(merged[0].suggested_value, "1"); // Rule wins for path.a
    }

    // JSON extraction tests moved to datasynth_core::llm::json_utils

    #[test]
    fn test_parse_llm_patches_valid() {
        let provider = MockLlmProvider::new(42);
        let config = AiTunerConfig::default();
        let tuner = AiTuner::new(&provider, config);

        let json = r#"[{"path": "transactions.count", "value": "10000", "confidence": 0.8, "reasoning": "More samples improve distribution fidelity"}]"#;
        let patches = tuner.parse_llm_patches(json);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].path, "transactions.count");
        assert_eq!(patches[0].suggested_value, "10000");
        assert!((patches[0].confidence - 0.8).abs() < 1e-10);
    }

    #[test]
    fn test_parse_llm_patches_filters_low_confidence() {
        let provider = MockLlmProvider::new(42);
        let config = AiTunerConfig {
            min_confidence: 0.6,
            ..Default::default()
        };
        let tuner = AiTuner::new(&provider, config);

        let json = r#"[
            {"path": "a", "value": "1", "confidence": 0.8},
            {"path": "b", "value": "2", "confidence": 0.3}
        ]"#;
        let patches = tuner.parse_llm_patches(json);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].path, "a");
    }

    #[test]
    fn test_ai_tune_result_improvement() {
        let result = AiTuneResult {
            iterations: vec![],
            final_patches: vec![],
            initial_health_score: 0.6,
            final_health_score: 0.85,
            converged: true,
            summary: String::new(),
        };
        assert!((result.improvement() - 0.25).abs() < 1e-10);
    }
}

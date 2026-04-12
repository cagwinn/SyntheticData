//! AML sophistication distribution evaluator.
//!
//! Validates that sophistication levels correlate realistically with context:
//! - Small amounts correlate with lower sophistication
//! - High-complexity typologies (trade-based ML, sanctions evasion) skew higher
//! - Retail customers rarely run state-level schemes

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::EvalResult;

#[derive(Debug, Clone)]
pub struct SophisticationObservation {
    pub amount: f64,
    pub typology: String,
    pub customer_type: String, // "retail" | "business" | "trust"
    pub sophistication: String, // "basic" | "standard" | "professional" | "advanced" | "state_level"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SophisticationThresholds {
    /// Small-amount (<$10K) retail Basic+Standard should be >60%
    pub min_small_retail_low_soph: f64,
    /// Sanctions evasion Basic should be <30% (inherently sophisticated)
    pub max_sanctions_basic: f64,
    /// All sophistication levels should be represented (coverage)
    pub min_level_coverage: f64,
}

impl Default for SophisticationThresholds {
    fn default() -> Self {
        Self {
            min_small_retail_low_soph: 0.60,
            max_sanctions_basic: 0.30,
            min_level_coverage: 0.4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SophisticationAnalysis {
    pub total_observations: usize,
    pub level_distribution: HashMap<String, f64>,
    pub small_retail_low_soph_rate: f64,
    pub sanctions_basic_rate: f64,
    pub levels_observed: usize,
    pub passes: bool,
    pub issues: Vec<String>,
}

pub struct SophisticationAnalyzer {
    pub thresholds: SophisticationThresholds,
}

impl SophisticationAnalyzer {
    pub fn new() -> Self {
        Self {
            thresholds: SophisticationThresholds::default(),
        }
    }

    pub fn analyze(
        &self,
        observations: &[SophisticationObservation],
    ) -> EvalResult<SophisticationAnalysis> {
        let total = observations.len();
        if total == 0 {
            return Ok(SophisticationAnalysis {
                total_observations: 0,
                level_distribution: HashMap::new(),
                small_retail_low_soph_rate: 0.0,
                sanctions_basic_rate: 0.0,
                levels_observed: 0,
                passes: true,
                issues: Vec::new(),
            });
        }

        // Level distribution
        let mut level_counts: HashMap<String, usize> = HashMap::new();
        for o in observations {
            *level_counts.entry(o.sophistication.clone()).or_insert(0) += 1;
        }
        let level_distribution: HashMap<String, f64> = level_counts
            .iter()
            .map(|(k, v)| (k.clone(), *v as f64 / total as f64))
            .collect();
        let levels_observed = level_distribution.len();

        // Small-amount retail low-sophistication rate
        let small_retail: Vec<_> = observations
            .iter()
            .filter(|o| o.amount < 10_000.0 && o.customer_type == "retail")
            .collect();
        let small_retail_low_soph = small_retail
            .iter()
            .filter(|o| matches!(o.sophistication.as_str(), "basic" | "standard"))
            .count();
        let small_retail_rate = if !small_retail.is_empty() {
            small_retail_low_soph as f64 / small_retail.len() as f64
        } else {
            1.0
        };

        // Sanctions evasion Basic rate
        let sanctions: Vec<_> = observations
            .iter()
            .filter(|o| o.typology == "sanctions_evasion")
            .collect();
        let sanctions_basic = sanctions
            .iter()
            .filter(|o| o.sophistication == "basic")
            .count();
        let sanctions_basic_rate = if !sanctions.is_empty() {
            sanctions_basic as f64 / sanctions.len() as f64
        } else {
            0.0
        };

        let mut issues = Vec::new();
        if small_retail.len() >= 10 && small_retail_rate < self.thresholds.min_small_retail_low_soph {
            issues.push(format!(
                "Small-retail low-sophistication rate {:.1}% below minimum {:.1}%",
                small_retail_rate * 100.0,
                self.thresholds.min_small_retail_low_soph * 100.0,
            ));
        }
        if sanctions.len() >= 10 && sanctions_basic_rate > self.thresholds.max_sanctions_basic {
            issues.push(format!(
                "Sanctions-evasion Basic rate {:.1}% above maximum {:.1}% — not sophisticated enough",
                sanctions_basic_rate * 100.0,
                self.thresholds.max_sanctions_basic * 100.0,
            ));
        }
        // Coverage: should see at least 2 levels
        if levels_observed < 2 {
            issues.push(format!(
                "Only {} sophistication levels observed — distribution too narrow",
                levels_observed,
            ));
        }

        Ok(SophisticationAnalysis {
            total_observations: total,
            level_distribution,
            small_retail_low_soph_rate: small_retail_rate,
            sanctions_basic_rate,
            levels_observed,
            passes: issues.is_empty(),
            issues,
        })
    }
}

impl Default for SophisticationAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_realistic_distribution_passes() {
        let mut obs = Vec::new();
        // 50 small retail — 80% basic/standard
        for i in 0..50 {
            let soph = if i < 40 { "basic" } else { "professional" };
            obs.push(SophisticationObservation {
                amount: 5_000.0,
                typology: "structuring".into(),
                customer_type: "retail".into(),
                sophistication: soph.into(),
            });
        }
        // 20 sanctions evasion — 20% basic (under threshold)
        for i in 0..20 {
            let soph = if i < 4 { "basic" } else { "professional" };
            obs.push(SophisticationObservation {
                amount: 100_000.0,
                typology: "sanctions_evasion".into(),
                customer_type: "business".into(),
                sophistication: soph.into(),
            });
        }
        let a = SophisticationAnalyzer::new();
        let r = a.analyze(&obs).unwrap();
        assert!(r.passes, "Issues: {:?}", r.issues);
    }

    #[test]
    fn test_sanctions_all_basic_flagged() {
        let obs: Vec<_> = (0..20)
            .map(|_| SophisticationObservation {
                amount: 100_000.0,
                typology: "sanctions_evasion".into(),
                customer_type: "business".into(),
                sophistication: "basic".into(),
            })
            .collect();
        let a = SophisticationAnalyzer::new();
        let r = a.analyze(&obs).unwrap();
        assert!(!r.passes);
        assert!(r.issues.iter().any(|i| i.contains("Sanctions")));
    }
}

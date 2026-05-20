//! Network structure evaluator.
//!
//! Validates that multi-party criminal networks have realistic topology:
//! - Power-law degree distribution (max >> average)
//! - Hubs + long tail (not uniform)
//! - Role distribution reflects network type

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::EvalResult;

/// Per-node observation from a generated network.
#[derive(Debug, Clone)]
pub struct NetworkNodeObservation {
    pub network_id: String,
    pub degree: usize,
    pub role: String, // "coordinator" | "smurf" | "middleman" | "cash_out" | ...
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStructureThresholds {
    /// Max degree should exceed average by at least this factor (power-law signature)
    pub min_hub_ratio: f64,
    /// Minimum number of distinct roles per network
    pub min_roles: usize,
    /// Maximum allowed fraction of degree-uniform networks (topology quality check)
    pub max_uniform_degree_rate: f64,
}

impl Default for NetworkStructureThresholds {
    fn default() -> Self {
        Self {
            min_hub_ratio: 2.5,
            min_roles: 2,
            max_uniform_degree_rate: 0.30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStructureAnalysis {
    pub total_networks: usize,
    pub mean_hub_ratio: f64,
    pub mean_role_diversity: f64,
    pub uniform_degree_networks: usize,
    pub power_law_networks: usize,
    pub passes: bool,
    pub issues: Vec<String>,
}

pub struct NetworkStructureAnalyzer {
    pub thresholds: NetworkStructureThresholds,
}

impl NetworkStructureAnalyzer {
    pub fn new() -> Self {
        Self {
            thresholds: NetworkStructureThresholds::default(),
        }
    }

    pub fn analyze(
        &self,
        observations: &[NetworkNodeObservation],
    ) -> EvalResult<NetworkStructureAnalysis> {
        // Group by network_id
        let mut by_network: HashMap<String, Vec<&NetworkNodeObservation>> = HashMap::new();
        for obs in observations {
            by_network
                .entry(obs.network_id.clone())
                .or_default()
                .push(obs);
        }
        let total_networks = by_network.len();

        let mut hub_ratios = Vec::new();
        let mut role_counts = Vec::new();
        let mut uniform_count = 0usize;
        let mut power_law_count = 0usize;

        for nodes in by_network.values() {
            // Degree stats
            let degrees: Vec<usize> = nodes.iter().map(|n| n.degree).collect();
            let max_deg = *degrees.iter().max().unwrap_or(&0);
            let avg_deg = if !degrees.is_empty() {
                degrees.iter().sum::<usize>() as f64 / degrees.len() as f64
            } else {
                0.0
            };
            let hub_ratio = if avg_deg > 0.0 {
                max_deg as f64 / avg_deg
            } else {
                1.0
            };
            hub_ratios.push(hub_ratio);

            // Check uniformity (all nodes same degree = hub-and-spoke limitation)
            let unique_degrees: std::collections::HashSet<usize> =
                degrees.iter().copied().collect();
            if unique_degrees.len() <= 2 {
                uniform_count += 1;
            }
            if hub_ratio >= self.thresholds.min_hub_ratio {
                power_law_count += 1;
            }

            // Role diversity
            let roles: std::collections::HashSet<&str> =
                nodes.iter().map(|n| n.role.as_str()).collect();
            role_counts.push(roles.len());
        }

        let mean_hub_ratio = if !hub_ratios.is_empty() {
            hub_ratios.iter().sum::<f64>() / hub_ratios.len() as f64
        } else {
            0.0
        };
        let mean_role_diversity = if !role_counts.is_empty() {
            role_counts.iter().sum::<usize>() as f64 / role_counts.len() as f64
        } else {
            0.0
        };
        let uniform_rate = if total_networks > 0 {
            uniform_count as f64 / total_networks as f64
        } else {
            0.0
        };

        let mut issues = Vec::new();
        if total_networks > 0 && mean_hub_ratio < self.thresholds.min_hub_ratio {
            issues.push(format!(
                "Mean hub ratio {:.2} below minimum {:.2} — networks are too uniform",
                mean_hub_ratio, self.thresholds.min_hub_ratio,
            ));
        }
        if total_networks > 0 && mean_role_diversity < self.thresholds.min_roles as f64 {
            issues.push(format!(
                "Mean role diversity {:.1} below minimum {} — networks lack role variety",
                mean_role_diversity, self.thresholds.min_roles,
            ));
        }
        if uniform_rate > self.thresholds.max_uniform_degree_rate {
            issues.push(format!(
                "{:.1}% of networks have uniform degree — too hub-and-spoke",
                uniform_rate * 100.0,
            ));
        }

        Ok(NetworkStructureAnalysis {
            total_networks,
            mean_hub_ratio,
            mean_role_diversity,
            uniform_degree_networks: uniform_count,
            power_law_networks: power_law_count,
            passes: issues.is_empty(),
            issues,
        })
    }
}

impl Default for NetworkStructureAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_power_law_network_passes() {
        // Simulate a power-law network: one hub (degree 10), many leaves (degree 1)
        let mut obs = Vec::new();
        obs.push(NetworkNodeObservation {
            network_id: "NET1".into(),
            degree: 10,
            role: "coordinator".into(),
        });
        for _ in 0..10 {
            obs.push(NetworkNodeObservation {
                network_id: "NET1".into(),
                degree: 1,
                role: "smurf".into(),
            });
        }
        obs.push(NetworkNodeObservation {
            network_id: "NET1".into(),
            degree: 3,
            role: "middleman".into(),
        });

        let a = NetworkStructureAnalyzer::new();
        let r = a.analyze(&obs).unwrap();
        assert!(r.passes, "Issues: {:?}", r.issues);
        assert_eq!(r.power_law_networks, 1);
    }

    #[test]
    fn test_uniform_degree_fails() {
        // Hub-and-spoke: coordinator + 5 smurfs all with degree 1
        let mut obs = Vec::new();
        for _ in 0..10 {
            obs.push(NetworkNodeObservation {
                network_id: "NET1".into(),
                degree: 1,
                role: "smurf".into(),
            });
        }
        let a = NetworkStructureAnalyzer::new();
        let r = a.analyze(&obs).unwrap();
        assert!(!r.passes);
    }
}

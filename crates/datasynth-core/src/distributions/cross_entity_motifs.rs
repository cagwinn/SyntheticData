//! SP3.3 — CrossEntityMotifSampler: biases fanout sampling toward shared
//! attribute pools per entity cluster.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::distributions::behavioral_priors::EntityClustersPrior;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossEntityMotifSampler {
    pub neighbors_of: HashMap<String, Vec<String>>,
    pub clustering_rate: f64,
}

impl CrossEntityMotifSampler {
    /// Build from an EntityClustersPrior. Each cluster member gets a list of
    /// its cluster-mates (excluding itself) as "neighbors".
    pub fn from_prior(prior: &EntityClustersPrior) -> Self {
        let mut neighbors_of: HashMap<String, Vec<String>> = HashMap::new();
        for cluster in &prior.clusters {
            for member in &cluster.members {
                let others: Vec<String> = cluster
                    .members
                    .iter()
                    .filter(|m| *m != member)
                    .cloned()
                    .collect();
                neighbors_of.insert(member.clone(), others);
            }
        }
        Self {
            neighbors_of,
            clustering_rate: prior.clustering_rate,
        }
    }

    pub fn neighbors(&self, entity: &str) -> &[String] {
        self.neighbors_of
            .get(entity)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Probability that the caller should pick a "shared" value vs a fresh one.
    pub fn should_share(&self, entity: &str) -> f64 {
        if self.neighbors_of.contains_key(entity) {
            self.clustering_rate
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distributions::behavioral_priors::EntityCluster;

    #[test]
    fn neighbors_returns_cluster_mates_excluding_self() {
        let prior = EntityClustersPrior {
            clusters: vec![EntityCluster {
                members: vec!["A".into(), "B".into(), "C".into()],
                avg_jaccard: 0.5,
            }],
            clustering_rate: 0.6,
        };
        let s = CrossEntityMotifSampler::from_prior(&prior);
        let neighbors_a: std::collections::HashSet<_> = s.neighbors("A").iter().collect();
        assert_eq!(neighbors_a.len(), 2);
        assert!(neighbors_a.contains(&"B".to_string()));
        assert!(neighbors_a.contains(&"C".to_string()));
        assert!(!neighbors_a.contains(&"A".to_string()));
    }

    #[test]
    fn isolate_has_no_neighbors_and_zero_share_prob() {
        let prior = EntityClustersPrior::default();
        let s = CrossEntityMotifSampler::from_prior(&prior);
        assert!(s.neighbors("X").is_empty());
        assert!((s.should_share("X")).abs() < 1e-9);
    }

    #[test]
    fn cluster_member_share_prob_equals_clustering_rate() {
        let prior = EntityClustersPrior {
            clusters: vec![EntityCluster {
                members: vec!["A".into(), "B".into()],
                avg_jaccard: 0.5,
            }],
            clustering_rate: 0.42,
        };
        let s = CrossEntityMotifSampler::from_prior(&prior);
        assert!((s.should_share("A") - 0.42).abs() < 1e-9);
        assert!((s.should_share("B") - 0.42).abs() < 1e-9);
        // Non-member gets 0.
        assert!((s.should_share("Z")).abs() < 1e-9);
    }
}

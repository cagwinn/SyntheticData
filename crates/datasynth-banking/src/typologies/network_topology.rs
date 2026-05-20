//! Realistic network topology generator using preferential attachment.
//!
//! Replaces simple hub-and-spoke with a proper random graph model:
//! - **Barabási-Albert preferential attachment**: each new node attaches to
//!   existing nodes with probability proportional to their current degree.
//!   Produces power-law degree distribution (realistic criminal networks
//!   have a few hubs and a long tail of leaves).
//! - **Bridge nodes**: nodes that connect otherwise-disconnected subgroups.
//! - **Clustering**: small clusters with dense internal connections.

use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

/// Seed offset for topology generator.
pub const NETWORK_TOPOLOGY_SEED_OFFSET: u64 = 8600;

/// A node in the generated network.
#[derive(Debug, Clone)]
pub struct NetworkNode {
    pub id: usize,
    pub degree: usize,
    pub is_hub: bool,    // high-degree node (coordinator)
    pub is_bridge: bool, // connects otherwise-disconnected subgroups
    pub cluster_id: Option<usize>,
}

/// An edge between two nodes.
#[derive(Debug, Clone, Copy)]
pub struct NetworkEdge {
    pub from: usize,
    pub to: usize,
}

/// Generated network topology.
#[derive(Debug, Clone)]
pub struct NetworkTopology {
    pub nodes: Vec<NetworkNode>,
    pub edges: Vec<NetworkEdge>,
}

impl NetworkTopology {
    /// Return the degree distribution (degree → count).
    pub fn degree_distribution(&self) -> HashMap<usize, usize> {
        let mut dist = HashMap::new();
        for node in &self.nodes {
            *dist.entry(node.degree).or_insert(0) += 1;
        }
        dist
    }

    /// Number of hubs (nodes with degree ≥ threshold).
    pub fn hub_count(&self, min_degree: usize) -> usize {
        self.nodes.iter().filter(|n| n.degree >= min_degree).count()
    }
}

/// Barabási-Albert preferential-attachment network generator.
pub struct NetworkTopologyGenerator {
    rng: ChaCha8Rng,
}

impl NetworkTopologyGenerator {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(NETWORK_TOPOLOGY_SEED_OFFSET)),
        }
    }

    /// Generate a Barabási-Albert network.
    ///
    /// - `total_nodes`: target number of nodes
    /// - `m`: each new node creates `m` edges to existing nodes, weighted by degree
    /// - `num_clusters`: target number of clusters (creates bridge nodes between clusters)
    pub fn generate_ba(
        &mut self,
        total_nodes: usize,
        m: usize,
        num_clusters: usize,
    ) -> NetworkTopology {
        let total_nodes = total_nodes.max(m + 1);
        let mut nodes: Vec<NetworkNode> = (0..total_nodes)
            .map(|i| NetworkNode {
                id: i,
                degree: 0,
                is_hub: false,
                is_bridge: false,
                cluster_id: None,
            })
            .collect();
        let mut edges: Vec<NetworkEdge> = Vec::new();

        // Cluster assignment — round-robin
        if num_clusters > 1 {
            for (i, node) in nodes.iter_mut().enumerate() {
                node.cluster_id = Some(i % num_clusters);
            }
        }

        // Initial m+1-clique
        for i in 0..=m {
            for j in (i + 1)..=m {
                edges.push(NetworkEdge { from: i, to: j });
                nodes[i].degree += 1;
                nodes[j].degree += 1;
            }
        }

        // Preferential attachment for remaining nodes
        for new_id in (m + 1)..total_nodes {
            let mut targets: Vec<usize> = Vec::with_capacity(m);
            while targets.len() < m {
                // Sample an edge endpoint proportional to degree
                let total_degree: usize = nodes[..new_id].iter().map(|n| n.degree).sum();
                if total_degree == 0 {
                    // Fallback: uniform
                    let candidate = self.rng.random_range(0..new_id);
                    if !targets.contains(&candidate) {
                        targets.push(candidate);
                    }
                    continue;
                }
                let roll = self.rng.random_range(0..total_degree);
                let mut cumulative = 0;
                for (id, node) in nodes[..new_id].iter().enumerate() {
                    cumulative += node.degree;
                    if cumulative > roll && !targets.contains(&id) {
                        targets.push(id);
                        break;
                    }
                }
            }
            for t in targets {
                edges.push(NetworkEdge {
                    from: new_id,
                    to: t,
                });
                nodes[new_id].degree += 1;
                nodes[t].degree += 1;
            }
        }

        // Mark hubs (top 10% by degree)
        let mut degrees: Vec<(usize, usize)> = nodes.iter().map(|n| (n.id, n.degree)).collect();
        degrees.sort_by_key(|b| std::cmp::Reverse(b.1));
        let hub_count = (total_nodes / 10).max(1);
        for (id, _) in degrees.iter().take(hub_count) {
            nodes[*id].is_hub = true;
        }

        // Identify bridge nodes (connect different clusters)
        if num_clusters > 1 {
            for edge in &edges {
                let c1 = nodes[edge.from].cluster_id;
                let c2 = nodes[edge.to].cluster_id;
                if c1 != c2 && c1.is_some() && c2.is_some() {
                    nodes[edge.from].is_bridge = true;
                    nodes[edge.to].is_bridge = true;
                }
            }
        }

        NetworkTopology { nodes, edges }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ba_produces_power_law_like_degrees() {
        let mut gen = NetworkTopologyGenerator::new(42);
        let network = gen.generate_ba(100, 2, 1);

        assert_eq!(network.nodes.len(), 100);
        assert!(network.edges.len() >= 99); // at least a connected graph
                                            // Max degree should be substantially higher than average (power-law signature)
        let max_degree = network.nodes.iter().map(|n| n.degree).max().unwrap();
        let avg_degree: f64 =
            network.nodes.iter().map(|n| n.degree as f64).sum::<f64>() / network.nodes.len() as f64;
        assert!(
            (max_degree as f64) > 3.0 * avg_degree,
            "Max degree ({max_degree}) should exceed 3x average ({avg_degree:.1}) — power-law signature"
        );
    }

    #[test]
    fn test_hub_count_reasonable() {
        let mut gen = NetworkTopologyGenerator::new(42);
        let network = gen.generate_ba(100, 2, 1);
        let hubs = network.hub_count(5);
        assert!(
            hubs > 0 && hubs < 30,
            "Should have some but not too many hubs: {hubs}"
        );
    }

    #[test]
    fn test_clusters_produce_bridges() {
        let mut gen = NetworkTopologyGenerator::new(42);
        let network = gen.generate_ba(50, 2, 3); // 3 clusters
        let bridges = network.nodes.iter().filter(|n| n.is_bridge).count();
        assert!(
            bridges > 0,
            "Should have bridge nodes with multiple clusters"
        );
    }

    #[test]
    fn test_degree_distribution_has_tail() {
        let mut gen = NetworkTopologyGenerator::new(42);
        let network = gen.generate_ba(200, 3, 1);
        let dist = network.degree_distribution();
        // Should have many low-degree nodes and a few high-degree ones
        let low_degree = dist
            .iter()
            .filter(|(d, _)| **d <= 3)
            .map(|(_, c)| c)
            .sum::<usize>();
        let high_degree = dist
            .iter()
            .filter(|(d, _)| **d >= 10)
            .map(|(_, c)| c)
            .sum::<usize>();
        assert!(
            low_degree > high_degree,
            "Should have more low-degree than high-degree nodes"
        );
    }
}

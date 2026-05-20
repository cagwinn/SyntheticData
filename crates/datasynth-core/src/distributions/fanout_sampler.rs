//! Bipartite fan-out sampler driven by SP2's FanoutHistogram.

use std::collections::HashSet;

use rand::{Rng, RngExt};

#[derive(Debug, Clone)]
pub struct AttributeBucket {
    pub attribute_value: String,
    pub target_fanout: u32,
    pub current_users: HashSet<String>,
}

impl AttributeBucket {
    pub fn has_capacity(&self) -> bool {
        self.current_users.len() < self.target_fanout as usize
    }

    pub fn remaining_capacity(&self) -> i64 {
        self.target_fanout as i64 - self.current_users.len() as i64
    }
}

#[derive(Clone)]
pub struct BipartiteFanoutSampler {
    pub buckets: Vec<AttributeBucket>,
}

impl BipartiteFanoutSampler {
    pub fn new_with_targets(targets: Vec<u32>, value_gen: impl Fn(usize) -> String) -> Self {
        let buckets = targets
            .into_iter()
            .enumerate()
            .map(|(i, t)| AttributeBucket {
                attribute_value: value_gen(i),
                target_fanout: t.max(1),
                current_users: HashSet::new(),
            })
            .collect();
        Self { buckets }
    }

    pub fn pick_for<R: Rng>(&mut self, entity_id: &str, rng: &mut R) -> String {
        if self.buckets.is_empty() {
            return String::new();
        }
        let candidate_idxs: Vec<usize> = (0..self.buckets.len())
            .filter(|&i| {
                self.buckets[i].has_capacity() || self.buckets[i].current_users.contains(entity_id)
            })
            .collect();
        let chosen_idx = if !candidate_idxs.is_empty() {
            candidate_idxs[rng.random_range(0..candidate_idxs.len())]
        } else {
            self.buckets
                .iter()
                .enumerate()
                .max_by_key(|(_, b)| b.remaining_capacity())
                .map(|(i, _)| i)
                .unwrap_or(0)
        };
        self.buckets[chosen_idx]
            .current_users
            .insert(entity_id.to_string());
        self.buckets[chosen_idx].attribute_value.clone()
    }

    pub fn current_fanouts(&self) -> Vec<u32> {
        self.buckets
            .iter()
            .map(|b| b.current_users.len() as u32)
            .collect()
    }

    /// SP3.3: pick an attribute value, preferring buckets already used by a
    /// neighbor entity. Falls back to `pick_for` when no neighbor-used buckets
    /// exist or when the share-probability roll declines.
    pub fn pick_for_with_neighbors<R: rand::Rng>(
        &mut self,
        entity_id: &str,
        neighbors: &[String],
        share_probability: f64,
        rng: &mut R,
    ) -> String {
        use rand::RngExt;
        if self.buckets.is_empty() {
            return String::new();
        }
        if !neighbors.is_empty()
            && share_probability > 0.0
            && rng.random_range(0.0..1.0) < share_probability
        {
            let neighbor_buckets: Vec<usize> = (0..self.buckets.len())
                .filter(|&i| {
                    neighbors
                        .iter()
                        .any(|n| self.buckets[i].current_users.contains(n))
                })
                .collect();
            if !neighbor_buckets.is_empty() {
                let chosen_idx = neighbor_buckets[rng.random_range(0..neighbor_buckets.len())];
                self.buckets[chosen_idx]
                    .current_users
                    .insert(entity_id.to_string());
                return self.buckets[chosen_idx].attribute_value.clone();
            }
        }
        self.pick_for(entity_id, rng)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn fanout_sampler_respects_targets() {
        let s = BipartiteFanoutSampler::new_with_targets(vec![3, 1], |i| format!("ACC-{i}"));
        assert_eq!(s.buckets.len(), 2);
        assert_eq!(s.buckets[0].target_fanout, 3);
        assert_eq!(s.buckets[1].target_fanout, 1);
    }

    #[test]
    fn fanout_sampler_assigns_distinct_entities_to_buckets() {
        let mut s = BipartiteFanoutSampler::new_with_targets(vec![2, 2], |i| format!("ACC-{i}"));
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for e in &["E1", "E2", "E3", "E4"] {
            let _ = s.pick_for(e, &mut rng);
        }
        let total: usize = s.buckets.iter().map(|b| b.current_users.len()).sum();
        assert_eq!(total, 4);
        for b in &s.buckets {
            assert!(b.current_users.len() <= 2);
        }
    }

    #[test]
    fn fanout_sampler_empty_returns_empty_string() {
        let mut s = BipartiteFanoutSampler::new_with_targets(vec![], |i| format!("X{i}"));
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        assert_eq!(s.pick_for("E1", &mut rng), "");
    }

    #[test]
    fn pick_for_with_neighbors_prefers_neighbor_bucket() {
        use rand::SeedableRng;
        // Build a sampler with 3 buckets. Manually mark bucket 1 as "used by N1".
        let mut s = BipartiteFanoutSampler::new_with_targets(vec![5, 5, 5], |i| format!("A{i}"));
        s.buckets[1].current_users.insert("N1".to_string());
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        // share_probability = 1.0 forces the neighbor-preferring path.
        let picked = s.pick_for_with_neighbors("E1", &["N1".to_string()], 1.0, &mut rng);
        assert_eq!(picked, "A1", "should pick bucket 1 (the one N1 has used)");
    }

    #[test]
    fn pick_for_with_neighbors_falls_back_when_no_neighbors() {
        use rand::SeedableRng;
        let mut s = BipartiteFanoutSampler::new_with_targets(vec![5, 5], |i| format!("A{i}"));
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        // Empty neighbor list — falls back to pick_for.
        let picked = s.pick_for_with_neighbors("E1", &[], 1.0, &mut rng);
        assert!(picked == "A0" || picked == "A1");
    }
}

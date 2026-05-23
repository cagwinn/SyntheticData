//! Source-conditional Dirichlet account-pair sampler (SOTA-8).
//!
//! Per source string, fits a Dirichlet-multinomial over a per-source account pool.
//! Round 0 (`FINDINGS §14`) showed the synthetic engine's source-conditional structure
//! is too uniform (entropy 0.97 vs corpus 0.68) and too narrow (5 vs 23.5 accounts per
//! source). This sampler closes both gaps simultaneously: a configurable larger pool,
//! drawn through a *concentrated* (low-α) Dirichlet.
//!
//! Math: symmetric Dirichlet(α, …, α) is realised by `pᵢ = Gᵢ / Σⱼ Gⱼ` with each
//! `Gᵢ ~ Gamma(α, 1)`. Lower α ⇒ concentrated PMF. With α = 0.5 and `N_s = 25` the
//! expected normalised entropy is ≈ 0.65 — matching the corpus median of 0.68.
//!
//! This module is wired in by `je_generator` only when the `transactions
//! .source_conditional_account_pair.enabled` config flag is set (default off — opt-in
//! so existing users' synthetic streams stay byte-identical).

use std::collections::HashMap;

use rand::distr::weighted::WeightedIndex;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, Gamma, LogNormal};

/// One source's account pool with a fitted Dirichlet PMF, ready to sample from.
#[derive(Debug, Clone)]
pub struct SourcePool {
    /// Accounts in this source's pool (size = `n()`).
    pub accounts: Vec<String>,
    /// Cumulative PMF over `accounts`; used for O(log N) inverse-CDF sampling.
    /// Always normalised so `cumulative.last() == 1.0`.
    cumulative: Vec<f64>,
}

impl SourcePool {
    /// Build a pool of `pool_size` accounts drawn from `all_accounts` weighted by
    /// `account_weights` (deduplicated), with a symmetric Dirichlet(α) PMF over them.
    pub fn new(
        pool_size: usize,
        all_accounts: &[String],
        account_weights: &[f64],
        alpha: f64,
        rng: &mut ChaCha8Rng,
    ) -> Self {
        assert_eq!(
            all_accounts.len(),
            account_weights.len(),
            "all_accounts and account_weights must align"
        );
        assert!(alpha > 0.0, "alpha must be > 0");

        // Weighted sampling-with-replacement + dedup until we have `pool_size`
        // distinct accounts (or run out of attempts on pathological weights).
        let widx = WeightedIndex::new(account_weights).expect("non-negative weights");
        let mut chosen: Vec<String> = Vec::with_capacity(pool_size);
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let cap = (16 * pool_size.max(1)).max(64);
        for _ in 0..cap {
            if chosen.len() >= pool_size {
                break;
            }
            let i = widx.sample(rng);
            if seen.insert(all_accounts[i].clone()) {
                chosen.push(all_accounts[i].clone());
            }
        }

        // Symmetric Dirichlet via Gamma normalisation.
        let gamma = Gamma::new(alpha, 1.0).expect("alpha > 0");
        let raw: Vec<f64> = (0..chosen.len())
            .map(|_| gamma.sample(rng).max(1e-300))
            .collect();
        let total: f64 = raw.iter().sum();
        let mut cumulative = Vec::with_capacity(raw.len());
        let mut running = 0.0;
        for r in raw {
            running += r / total;
            cumulative.push(running);
        }
        // Guard against fp drift on the upper bound.
        if let Some(last) = cumulative.last_mut() {
            *last = 1.0;
        }
        Self {
            accounts: chosen,
            cumulative,
        }
    }

    /// Number of accounts in the pool.
    pub fn n(&self) -> usize {
        self.accounts.len()
    }

    /// Draw a single account from the PMF.
    pub fn sample_one(&self, rng: &mut ChaCha8Rng) -> &str {
        if self.accounts.is_empty() {
            return "";
        }
        let u: f64 = rng.random();
        let idx = self
            .cumulative
            .partition_point(|&c| c < u)
            .min(self.accounts.len() - 1);
        &self.accounts[idx]
    }

    /// Draw a `(debit_account, credit_account)` pair from the per-source PMF, with the
    /// distinct-accounts constraint. Returns `None` if the pool has fewer than 2
    /// accounts (the caller should fall back to the global picker).
    pub fn sample_pair(&self, rng: &mut ChaCha8Rng) -> Option<(String, String)> {
        if self.accounts.len() < 2 {
            return None;
        }
        let d = self.sample_one(rng).to_string();
        // Re-sample until the credit account differs. Typically 1 attempt; bounded to
        // avoid infinite loops on pathological PMFs with one near-mass-1 component.
        for _ in 0..16 {
            let c = self.sample_one(rng);
            if c != d {
                return Some((d, c.to_string()));
            }
        }
        // Deterministic fallback: pick any other account in the pool.
        let other = self
            .accounts
            .iter()
            .find(|a| **a != d)
            .expect("len() >= 2 was checked above");
        Some((d, other.clone()))
    }

    /// Normalised Shannon entropy of the PMF in `[0, 1]`. Useful for tests +
    /// observability (e.g. comparing to the corpus's source-conditional entropy band).
    pub fn normalised_entropy(&self) -> f64 {
        if self.accounts.len() <= 1 {
            return 0.0;
        }
        let n = self.accounts.len() as f64;
        let mut prev = 0.0;
        let mut h = 0.0;
        for &c in &self.cumulative {
            let p = c - prev;
            prev = c;
            if p > 0.0 {
                h -= p * p.ln();
            }
        }
        h / n.ln()
    }
}

/// Top-level sampler — one `SourcePool` per source string.
#[derive(Debug, Clone, Default)]
pub struct SourceConditionalPairSampler {
    pools: HashMap<String, SourcePool>,
}

impl SourceConditionalPairSampler {
    /// Build a sampler for every source in `sources`. Each gets a pool of
    /// approximately `accts_per_source_target` accounts (multiplied by a LogNormal(0,
    /// 0.3) jitter so the per-source pool size has corpus-like variance), drawn from
    /// `all_accounts` weighted by `account_weights`, with PMF ∼ Dir(α).
    pub fn new(
        sources: &[String],
        all_accounts: &[String],
        account_weights: &[f64],
        accts_per_source_target: usize,
        alpha: f64,
        rng: &mut ChaCha8Rng,
    ) -> Self {
        assert_eq!(all_accounts.len(), account_weights.len());
        let jitter = LogNormal::new(0.0, 0.3).expect("sigma > 0");
        let mut pools = HashMap::with_capacity(sources.len());
        for s in sources {
            let mult = jitter.sample(rng);
            let n_s = ((accts_per_source_target as f64 * mult).round() as usize)
                .max(2)
                .min(all_accounts.len());
            pools.insert(
                s.clone(),
                SourcePool::new(n_s, all_accounts, account_weights, alpha, rng),
            );
        }
        Self { pools }
    }

    /// Get the per-source pool (for diagnostics / tests).
    pub fn pool(&self, source: &str) -> Option<&SourcePool> {
        self.pools.get(source)
    }

    /// Lazy-add a per-source pool if one isn't already present. Returns `true` iff a
    /// new pool was inserted; `false` if `source` was already pooled (no-op). Uses the
    /// same LogNormal(0, 0.3) jitter on the pool size as `new`, so a sampler built up
    /// one source at a time has the same distribution as one built with all sources
    /// at once.
    pub fn ensure_pool(
        &mut self,
        source: &str,
        all_accounts: &[String],
        account_weights: &[f64],
        accts_per_source_target: usize,
        alpha: f64,
        rng: &mut ChaCha8Rng,
    ) -> bool {
        if self.pools.contains_key(source) {
            return false;
        }
        let jitter = LogNormal::new(0.0, 0.3).expect("sigma > 0");
        let mult = jitter.sample(rng);
        let n_s = ((accts_per_source_target as f64 * mult).round() as usize)
            .max(2)
            .min(all_accounts.len());
        self.pools.insert(
            source.to_string(),
            SourcePool::new(n_s, all_accounts, account_weights, alpha, rng),
        );
        true
    }

    /// Sample a `(debit_account, credit_account)` pair conditioned on `source`.
    /// Returns `None` if the source isn't in the sampler — the caller should fall back
    /// to the existing global account picker.
    pub fn sample_pair(
        &self,
        source: &str,
        rng: &mut ChaCha8Rng,
    ) -> Option<(String, String)> {
        self.pools.get(source).and_then(|p| p.sample_pair(rng))
    }

    pub fn is_empty(&self) -> bool {
        self.pools.is_empty()
    }
    pub fn n_sources(&self) -> usize {
        self.pools.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn synthetic_accounts(n: usize) -> (Vec<String>, Vec<f64>) {
        // Lognormal-ish weights (a stand-in for the existing account-Pareto in tests).
        let accounts: Vec<String> = (0..n).map(|i| format!("ACC{i:04}")).collect();
        let weights: Vec<f64> = (0..n)
            .map(|i| 1.0 / ((i + 1) as f64).powf(1.2))
            .collect();
        (accounts, weights)
    }

    #[test]
    fn small_alpha_yields_concentrated_pmf() {
        let (acc, wts) = synthetic_accounts(200);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let pool = SourcePool::new(25, &acc, &wts, 0.5, &mut rng);
        let h = pool.normalised_entropy();
        assert_eq!(pool.n(), 25);
        // α = 0.5, N = 25 ⇒ expected entropy ≈ 0.6–0.75; allow a wider single-draw band.
        assert!(
            (0.45..=0.85).contains(&h),
            "expected concentrated entropy in [0.45, 0.85], got {h}"
        );
    }

    #[test]
    fn large_alpha_yields_diffuse_pmf() {
        let (acc, wts) = synthetic_accounts(200);
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let pool = SourcePool::new(25, &acc, &wts, 10.0, &mut rng);
        let h = pool.normalised_entropy();
        // α = 10, N = 25 ⇒ entropy near-uniform.
        assert!(h > 0.9, "expected diffuse entropy > 0.9, got {h}");
    }

    #[test]
    fn same_seed_same_pool() {
        let (acc, wts) = synthetic_accounts(100);
        let a = SourcePool::new(20, &acc, &wts, 0.5, &mut ChaCha8Rng::seed_from_u64(1));
        let b = SourcePool::new(20, &acc, &wts, 0.5, &mut ChaCha8Rng::seed_from_u64(1));
        assert_eq!(a.accounts, b.accounts);
        for (x, y) in a.cumulative.iter().zip(&b.cumulative) {
            assert!((x - y).abs() < 1e-12, "PMF mismatch: {x} vs {y}");
        }
    }

    #[test]
    fn sample_pair_returns_distinct_accounts() {
        let (acc, wts) = synthetic_accounts(50);
        let mut rng = ChaCha8Rng::seed_from_u64(3);
        let pool = SourcePool::new(10, &acc, &wts, 0.7, &mut rng);
        for _ in 0..200 {
            let (d, c) = pool.sample_pair(&mut rng).expect("pool has 2+ accounts");
            assert_ne!(d, c);
            assert!(pool.accounts.contains(&d));
            assert!(pool.accounts.contains(&c));
        }
    }

    #[test]
    fn full_sampler_per_source_diversity() {
        let (acc, wts) = synthetic_accounts(200);
        let sources: Vec<String> = (0..5).map(|i| format!("S{i}")).collect();
        let sampler = SourceConditionalPairSampler::new(
            &sources,
            &acc,
            &wts,
            25,
            0.5,
            &mut ChaCha8Rng::seed_from_u64(99),
        );
        assert_eq!(sampler.n_sources(), 5);
        // Pools across sources should not be near-identical: with pool size 25 drawn
        // (weighted) from 200 accounts the typical overlap is well below total.
        let p0: std::collections::HashSet<_> =
            sampler.pool("S0").unwrap().accounts.iter().collect();
        let p1: std::collections::HashSet<_> =
            sampler.pool("S1").unwrap().accounts.iter().collect();
        let overlap = p0.intersection(&p1).count() as f64 / p0.len() as f64;
        assert!(overlap < 0.85, "pools too similar across sources: overlap={overlap:.2}");
    }
}

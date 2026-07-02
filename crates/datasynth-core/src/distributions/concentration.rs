//! Discrete counterparty-concentration sampler (spec 19 §4-R1 / R1b).
//!
//! The document-flow generators historically pick a counterparty by **uniform round-robin**
//! (`vendors[i % vendors.len()]`), so every vendor/customer carries an equal share of volume — the
//! opposite of a real ledger, where a few big counterparties carry most of the spend (the "80/20"
//! Pareto shape). This module replaces that `i % len` index with a **discrete weighted-choice**:
//! given a concentration target `{top_n, top_n_share}` it builds a truncated power-law weight table
//! over the (already-sorted) entity list so the first `top_n` entities receive ~`top_n_share` of the
//! draws, then draws an index from an **isolated** ChaCha8 stream.
//!
//! This is NOT the [`super::pareto::ParetoSampler`] — that is a *continuous* amount sampler (it draws
//! a dollar figure from a Pareto law). Concentration is a *discrete index* problem (which of N
//! counterparties gets this document), so it needs its own weighted-choice construction.
//!
//! # Determinism discipline
//!
//! The sampler owns its **own** `ChaCha8Rng`, seeded from a caller-derived dedicated seed (see
//! [`concentration_seed`]) that is a pure hash of the base generation seed + a per-cycle label. It
//! therefore never touches — and never perturbs — the generation RNG the rest of the orchestrator
//! consumes. The weighted draw is only ever invoked when a real concentration target is configured;
//! with the feature off the caller keeps the exact `i % len` code path, so an off build is
//! byte-for-byte identical to the pre-fork engine.

use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use std::hash::{Hash, Hasher};

/// A weighted-choice sampler over a fixed set of `n` entities (indices `0..n`), where entity `i` is
/// drawn with probability `weights[i] / sum(weights)`. Owns an isolated ChaCha8 stream so draws are
/// reproducible from the seed and never disturb any other RNG.
#[derive(Clone, Debug)]
pub struct ConcentrationSampler {
    rng: ChaCha8Rng,
    /// Cumulative weights (monotonically non-decreasing); `cumulative.last()` is the total weight.
    cumulative: Vec<f64>,
}

impl ConcentrationSampler {
    /// Build a sampler for `n` entities whose weights follow a **truncated Zipf/Pareto** shape tuned
    /// so the first `top_n` entities collectively receive ~`top_n_share` of the total weight.
    ///
    /// The weight of rank `r` (0-based) is `1 / (r + 1)^s` for a Zipf exponent `s`; `s` is solved so
    /// that `Σ_{r<top_n} w_r / Σ_{r<n} w_r == top_n_share`. When `top_n >= n`, or the target is
    /// degenerate (`<= 0`, `>= 1`, or matches the uniform share `top_n / n`), the caller should not
    /// be using a weighted draw at all — but for safety this still returns a valid sampler (uniform
    /// when `s == 0`).
    ///
    /// `seed` must be the **dedicated** concentration seed (see [`concentration_seed`]), NOT the raw
    /// generation seed — the whole point is an isolated stream.
    pub fn new(seed: u64, n: usize, top_n: usize, top_n_share: f64) -> Self {
        let n = n.max(1);
        let s = solve_zipf_exponent(n, top_n, top_n_share);
        let mut cumulative = Vec::with_capacity(n);
        let mut acc = 0.0_f64;
        for r in 0..n {
            acc += 1.0 / ((r as f64) + 1.0).powf(s);
            cumulative.push(acc);
        }
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed),
            cumulative,
        }
    }

    /// The number of entities this sampler chooses among.
    pub fn len(&self) -> usize {
        self.cumulative.len()
    }

    /// Whether the sampler has no entities (it never does — `new` clamps `n >= 1` — but clippy asks).
    pub fn is_empty(&self) -> bool {
        self.cumulative.is_empty()
    }

    /// The (normalized) probability that rank `r` is drawn — for tests / diagnostics.
    pub fn weight_share(&self, r: usize) -> f64 {
        let n = self.cumulative.len();
        if r >= n {
            return 0.0;
        }
        let total = self.cumulative[n - 1];
        let lower = if r == 0 { 0.0 } else { self.cumulative[r - 1] };
        (self.cumulative[r] - lower) / total
    }

    /// The combined share of the top `k` ranks — the metric SHAPE-DB-001 measures.
    pub fn top_k_share(&self, k: usize) -> f64 {
        let n = self.cumulative.len();
        if n == 0 {
            return 0.0;
        }
        let k = k.min(n);
        if k == 0 {
            return 0.0;
        }
        self.cumulative[k - 1] / self.cumulative[n - 1]
    }

    /// Draw one entity index in `0..len()` per the weight table, advancing the isolated stream.
    pub fn sample(&mut self) -> usize {
        let n = self.cumulative.len();
        if n <= 1 {
            return 0;
        }
        let total = self.cumulative[n - 1];
        // gen a uniform in [0, total); `partition_point` finds the first cumulative bucket strictly
        // greater than the draw — the inverse-CDF of the discrete weight table.
        let u: f64 = self.rng.random::<f64>() * total;
        let idx = self.cumulative.partition_point(|&c| c <= u);
        idx.min(n - 1)
    }
}

/// Solve the Zipf exponent `s >= 0` such that the top-`top_n` ranks hold ~`top_n_share` of the total
/// Zipf weight over `n` ranks. Monotone in `s` (larger `s` → more mass on rank 0), so a bisection on
/// `[0, S_MAX]` converges quickly and deterministically. Returns `0.0` (uniform) for degenerate
/// targets — the caller gates on a *real* target, so this only guards against misuse.
fn solve_zipf_exponent(n: usize, top_n: usize, top_n_share: f64) -> f64 {
    // Guardrails: a target that can't be realized by concentrating mass on the head → uniform.
    if n <= 1 || top_n == 0 || top_n >= n {
        return 0.0;
    }
    let uniform_share = top_n as f64 / n as f64;
    // Target at/below the uniform head share, or non-sensical (>=1 or <=0): no concentration.
    if top_n_share <= uniform_share || top_n_share >= 1.0 {
        return 0.0;
    }

    // Bisection: find s with top_k_share(s) == top_n_share. share is INCREASING in s.
    const S_MAX: f64 = 12.0; // s=12 concentrates essentially all mass on rank 0 — a hard ceiling.
    const ITERS: usize = 60; // 2^-60 precision on s — far tighter than the 8% SHAPE band needs.
    let mut lo = 0.0_f64;
    let mut hi = S_MAX;
    for _ in 0..ITERS {
        let mid = 0.5 * (lo + hi);
        if zipf_top_share(n, top_n, mid) < top_n_share {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// The top-`top_n` share of the Zipf(`s`) weight over `n` ranks: `Σ_{r<top_n} (r+1)^-s / Σ_{r<n}`.
fn zipf_top_share(n: usize, top_n: usize, s: f64) -> f64 {
    let mut top = 0.0_f64;
    let mut total = 0.0_f64;
    for r in 0..n {
        let w = 1.0 / ((r as f64) + 1.0).powf(s);
        total += w;
        if r < top_n {
            top += w;
        }
    }
    if total <= 0.0 {
        0.0
    } else {
        top / total
    }
}

/// Derive the **dedicated** concentration seed from the base generation seed and a per-cycle label
/// (e.g. `"p2p_vendor"`, `"o2c_customer"`). A pure hash — the same `(base_seed, label)` always maps
/// to the same stream, and it is disjoint from the base seed so the concentration draw is isolated
/// from the generation RNG.
pub fn concentration_seed(base_seed: u64, label: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // Domain tag so this never collides with any other ad-hoc `hash((seed, str))` in the codebase.
    "datasynth.concentration".hash(&mut hasher);
    base_seed.hash(&mut hasher);
    label.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_deterministic_and_label_isolated() {
        // Same (seed, label) → same derived seed; different label → (almost surely) different seed;
        // and neither equals the raw base seed (the isolation contract).
        assert_eq!(
            concentration_seed(42, "p2p_vendor"),
            concentration_seed(42, "p2p_vendor")
        );
        assert_ne!(
            concentration_seed(42, "p2p_vendor"),
            concentration_seed(42, "o2c_customer")
        );
        assert_ne!(concentration_seed(42, "p2p_vendor"), 42);
    }

    #[test]
    fn weights_sum_to_one() {
        let s = ConcentrationSampler::new(1, 18, 5, 0.55);
        let sum: f64 = (0..s.len()).map(|r| s.weight_share(r)).sum();
        assert!(
            (sum - 1.0).abs() < 1e-9,
            "weight shares must sum to 1, got {sum}"
        );
    }

    #[test]
    fn top_n_share_hits_target() {
        // The solved weight table must put ~top_n_share on the top-N ranks (the SHAPE-DB-001 metric).
        for &(n, top_n, target) in &[(18usize, 5usize, 0.55f64), (30, 5, 0.45), (100, 10, 0.60)] {
            let s = ConcentrationSampler::new(7, n, top_n, target);
            let got = s.top_k_share(top_n);
            assert!(
                (got - target).abs() < 1e-3,
                "n={n} top_n={top_n}: weight-table top-{top_n} share {got:.4} vs target {target:.4}",
            );
        }
    }

    #[test]
    fn realized_draw_share_near_target() {
        // Draw many samples and confirm the EMPIRICAL top-N share lands near the target (well inside
        // the ±0.08 SHAPE-DB-001 band). Deterministic seed → this is a fixed, reproducible check.
        let n = 18;
        let top_n = 5;
        let target = 0.55;
        let mut s = ConcentrationSampler::new(2024, n, top_n, target);
        let draws = 200_000;
        let mut counts = vec![0u64; n];
        for _ in 0..draws {
            counts[s.sample()] += 1;
        }
        // rank order is already the weight order (rank 0 heaviest); top-N = first top_n indices.
        let top: u64 = counts[..top_n].iter().sum();
        let realized = top as f64 / draws as f64;
        assert!(
            (realized - target).abs() < 0.02,
            "empirical top-{top_n} share {realized:.4} should be near target {target:.4}",
        );
    }

    #[test]
    fn same_seed_is_reproducible() {
        let mut a = ConcentrationSampler::new(99, 30, 5, 0.45);
        let mut b = ConcentrationSampler::new(99, 30, 5, 0.45);
        let seq_a: Vec<usize> = (0..500).map(|_| a.sample()).collect();
        let seq_b: Vec<usize> = (0..500).map(|_| b.sample()).collect();
        assert_eq!(
            seq_a, seq_b,
            "same seed + config must reproduce the exact draw sequence"
        );
    }

    #[test]
    fn degenerate_targets_fall_back_to_uniform() {
        // top_n >= n, or a target at/below the uniform head share, or an out-of-range target → the
        // weight table is uniform (s == 0), so every rank has share 1/n. (The caller never actually
        // invokes a weighted draw in these cases — this only guards misuse.)
        for s in [
            ConcentrationSampler::new(1, 5, 5, 0.9),  // top_n == n
            ConcentrationSampler::new(1, 5, 6, 0.9),  // top_n > n
            ConcentrationSampler::new(1, 10, 5, 0.5), // target == uniform head share
            ConcentrationSampler::new(1, 10, 5, 0.4), // target below uniform head share
            ConcentrationSampler::new(1, 10, 5, 1.0), // target >= 1
        ] {
            let n = s.len();
            for r in 0..n {
                assert!(
                    (s.weight_share(r) - 1.0 / n as f64).abs() < 1e-9,
                    "degenerate target must yield a uniform table",
                );
            }
        }
    }
}

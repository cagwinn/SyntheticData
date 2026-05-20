# SP3 — Entity-Aware Generation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire SP2's industry-priors `.dsf` bundles into the journal-entry generators so DataSynth's runtime output matches the corpus distributions for the five SP1 baseline gaps that have priors, while keeping the priors-disabled code path byte-identical to v5.11. Also surface the existing `JournalEntryLine.trading_partner` field on `journal_entries.csv` so SP1's TP-anchored secondary-entity metrics stop degenerating to 0.

**Architecture:** Three new RNG-driven samplers in `datasynth-core::distributions/` (`ConditionalIETSampler`, `BipartiteFanoutSampler`, `SourceActiveWindow`). A new `priors_loader.rs` in `datasynth-generators` that deserialises an `industry_priors_{X}.dsf` bundle once at init and pre-builds the sampler state. The existing `je_generator.rs` (3500 LOC) gets four `if let Some(loaded_priors)` insertion points — IET timing, lines-per-JE, attribute fan-out, and active-window gating — that leave the disabled path unchanged. `p2p_generator.rs` and `o2c_generator.rs` already know `vendor_id` / `customer_id` when they emit lines; SP3 plumbs them through to the existing `JournalEntryLine.trading_partner` field. The CSV writer appends `trading_partner` to the journal_entries column list (always emitted, empty for non-doc-linked rows). Config schema gains an opt-in `industry_profile.priors` sub-section.

**Tech Stack:** Rust 2021; existing workspace deps `rand`, `rand_chacha`, `chrono`, `serde`, `serde_yaml`, `datasynth-fingerprint`. The only new internal dep is `datasynth-fingerprint` becoming a workspace dependency of `datasynth-generators` (if not already).

**Spec:** [`docs/superpowers/specs/2026-05-12-sp3-entity-aware-generation-design.md`](../specs/2026-05-12-sp3-entity-aware-generation-design.md)

**Predecessors:**
- [SP1 baseline](../../baselines/2026-05-12-sp1-v5.10.0/SUMMARY.md) — composite BF 59.0× drives the five rewires
- [SP2 bundles](../../real-world-priors.md) — committed `industry_priors_{health,life_sciences,pharmaceutical,power_and_utilities,technology}.dsf`

**Test concurrency:** Use `--test-threads=4`. Never `cargo test --workspace`; prefer `cargo test -p <crate> --lib -- --quiet --test-threads=4`.

---

## File Structure

```
crates/datasynth-core/src/
├── distributions/
│   ├── mod.rs                                       (modify: re-export 3 new samplers)
│   ├── conditional_iet.rs                           (new)
│   ├── fanout_sampler.rs                            (new)
│   └── source_active_window.rs                      (new)
└── (JournalEntryLine.trading_partner already exists — no model changes needed)

crates/datasynth-fingerprint/src/models/
└── behavioral.rs                                    (modify: add LineCountHistogram::sample_bucket)

crates/datasynth-generators/
├── Cargo.toml                                       (modify: add datasynth-fingerprint dep)
└── src/
    ├── lib.rs                                       (modify: pub mod priors_loader)
    ├── priors_loader.rs                             (new)
    ├── je_generator.rs                              (modify: 4 gated insertion points)
    └── document_flow/
        ├── p2p_generator.rs                         (modify: populate trading_partner)
        └── o2c_generator.rs                         (modify: populate trading_partner)

crates/datasynth-config/src/
└── schema.rs                                        (modify: industry_profile.priors)

crates/datasynth-output/src/
└── csv_writer.rs (or journal_entries writer)        (modify: append trading_partner column)

crates/datasynth-generators/tests/                   (new tests)
├── sp3_priors_smoke.rs
└── sp3_backward_compat.rs

docs/
└── entity-aware-generation.md                       (new)

CHANGELOG.md, README.md, CLAUDE.md, .github/workflows/ci.yml   (small touch-ups)
```

## Task overview & dependency graph

```
T1  (LineCountHistogram::sample_bucket helper)
T2  (ConditionalIETSampler)          ─┐
T3  (BipartiteFanoutSampler)         ─┤  Phase A — parallel-safe (different files)
T4  (SourceActiveWindow)             ─┘
        │
        ▼
T5  (LoadedPriors + priors_loader)
        │
        ▼
T6  (config schema: industry_profile.priors)
        │
        ├──▶ T7  (CSV writer: trading_partner column)
        ├──▶ T8  (p2p_generator: populate trading_partner)
        ├──▶ T9  (o2c_generator: populate trading_partner)
        │
        ├──▶ T10 (je_generator: thread LoadedPriors through context)
        │       │
        │       ▼
        │   T11 (je_generator: IET timing rewire)
        │   T12 (je_generator: lines-per-JE rewire)
        │   T13 (je_generator: fanout rewire — 3 attributes)
        │   T14 (je_generator: active-window gating)
        │       │
        │       ▼
        └──▶ T15 (priors-driven smoke test)
             T16 (backward-compat test)
             T17 (CI workflow)
             T18 (docs + CHANGELOG + README + CLAUDE.md)
```

Total: 18 tasks. Serial dispatch per the subagent-driven-development skill. Wall-clock ~9-11 working days.

---

## Task 1 — `LineCountHistogram::sample_bucket` helper

**Files:**
- Modify: `crates/datasynth-fingerprint/src/models/behavioral.rs`

The existing `LineCountHistogram` (added in SP2/T2) has `build`, `pool`, `median_bucket`. SP3 needs `sample_bucket(rng)` — pick a bucket by probability mass, then sample uniformly inside it.

- [ ] **Step 1: Append sampler to LineCountHistogram impl**

Find the existing `impl LineCountHistogram { … }` block in `crates/datasynth-fingerprint/src/models/behavioral.rs`. Append (before the closing brace):

```rust
    /// Sample a count from the histogram. Picks a bucket weighted by
    /// probability mass, then samples uniformly within the bucket (between
    /// `buckets[i]` inclusive and `buckets[i+1]` exclusive — for the last
    /// bucket, returns `buckets[i]`).
    pub fn sample_bucket<R: rand::Rng>(&self, rng: &mut R) -> u32 {
        if self.buckets.is_empty() {
            return 0;
        }
        let r: f64 = rng.gen_range(0.0..1.0);
        let mut cum = 0.0;
        let mut chosen_idx = self.buckets.len() - 1;
        for (i, &p) in self.probabilities.iter().enumerate() {
            cum += p;
            if r <= cum {
                chosen_idx = i;
                break;
            }
        }
        let lo = self.buckets[chosen_idx];
        let hi = self
            .buckets
            .get(chosen_idx + 1)
            .copied()
            .unwrap_or(lo);
        if hi <= lo {
            lo
        } else {
            rng.gen_range(lo..hi)
        }
    }
```

Note: in rand 0.10 the method is `random_range`, not `gen_range`. If you see a deprecation warning, switch to `random_range`. Both `rng.gen_range` and `rng.random_range` work depending on the workspace `rand` version.

- [ ] **Step 2: Append unit tests inside the existing `mod tests`**

```rust
    #[test]
    fn sample_bucket_respects_probabilities() {
        use rand::SeedableRng;
        let h = LineCountHistogram {
            buckets: vec![1, 2, 4, 8],
            probabilities: vec![0.0, 0.0, 1.0, 0.0],
            n: 100,
        };
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        for _ in 0..50 {
            let s = h.sample_bucket(&mut rng);
            assert!((4..8).contains(&s), "expected sample in [4,8), got {s}");
        }
    }

    #[test]
    fn sample_bucket_empty_returns_zero() {
        use rand::SeedableRng;
        let h = LineCountHistogram::default();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        assert_eq!(h.sample_bucket(&mut rng), 0);
    }
```

If `rand_chacha` is not yet in `[dev-dependencies]` of `datasynth-fingerprint/Cargo.toml`, add it:

```toml
[dev-dependencies]
rand_chacha = { workspace = true }
```

- [ ] **Step 3: Test + clippy + commit**

```bash
cargo test -p datasynth-fingerprint --lib models::behavioral -- --test-threads=4 2>&1 | tail -10
cargo clippy -p datasynth-fingerprint -- -D warnings 2>&1 | tail -3
cargo fmt -p datasynth-fingerprint
git add crates/datasynth-fingerprint/src/models/behavioral.rs crates/datasynth-fingerprint/Cargo.toml
git commit -m "feat(fingerprint/behavioral): LineCountHistogram::sample_bucket helper for SP3"
```

Expected: 8 tests in `models::behavioral::tests` (6 existing + 2 new).

---

## Task 2 — ConditionalIETSampler

**Files:**
- Create: `crates/datasynth-core/src/distributions/conditional_iet.rs`
- Modify: `crates/datasynth-core/src/distributions/mod.rs`

Samples per-Source inter-event-times in days from the SP2 prior, with optional lag-1 autocorrelation coupling.

- [ ] **Step 1: Create `conditional_iet.rs`**

```rust
//! Per-Source inter-event-time sampler driven by SP2's PerSourceIetPrior.

use std::collections::HashMap;

use rand::Rng;

/// Per-Source RNG state for IET sampling.
#[derive(Debug, Clone)]
pub struct SourceIetState {
    /// Quantile knot values (sorted ascending) from the prior's empirical CDF.
    pub cdf_values: Vec<f64>,
    /// Cumulative probabilities matching `cdf_values` (monotone in [0, 1]).
    pub cdf_probabilities: Vec<f64>,
    /// Lag-1 Pearson correlation observed in the corpus for this Source.
    pub lag1_autocorr: f64,
    /// Last sampled IET (in days) — used to couple the next draw via the autocorr.
    pub last_iet_days: Option<f64>,
}

impl SourceIetState {
    fn sample_quantile<R: Rng>(&self, rng: &mut R) -> f64 {
        if self.cdf_values.is_empty() {
            return 0.0;
        }
        // Random u ∈ (0, 1]; find the smallest knot with cum prob ≥ u.
        let u: f64 = rng.gen_range(f64::EPSILON..=1.0);
        let mut idx = self.cdf_probabilities.len() - 1;
        for (i, &p) in self.cdf_probabilities.iter().enumerate() {
            if p >= u {
                idx = i;
                break;
            }
        }
        self.cdf_values[idx]
    }
}

/// Per-Source IET sampler: each call to `sample_next` produces a fresh day-gap
/// drawn from that Source's empirical CDF, optionally coupled with the previous
/// sample via the lag-1 autocorrelation.
pub struct ConditionalIETSampler {
    per_source: HashMap<String, SourceIetState>,
    fallback: SourceIetState,
}

impl ConditionalIETSampler {
    /// Build the sampler from a `PerSourceIetPrior`. Pass in the prior's
    /// `by_source` map (extracted via fingerprint type) plus a fallback state
    /// for sources not present in the prior.
    ///
    /// `value_extractor` / `prob_extractor` / `autocorr_extractor` are closures
    /// because we don't want to depend directly on the fingerprint crate from
    /// datasynth-core. The caller (priors_loader in datasynth-generators)
    /// converts a `PerSourceIetPrior` into the constructor input.
    pub fn from_state_map(
        per_source: HashMap<String, SourceIetState>,
        fallback: SourceIetState,
    ) -> Self {
        Self { per_source, fallback }
    }

    /// Sample the next IET in days for `source`. The autocorr coupling shifts
    /// the quantile draw toward the previous IET's quantile by `lag1_autocorr`.
    pub fn sample_next<R: Rng>(&mut self, source: &str, rng: &mut R) -> f64 {
        let state = self
            .per_source
            .get_mut(source)
            .unwrap_or(&mut self.fallback);
        let raw_sample = state.sample_quantile(rng);
        let coupled = if let Some(prev) = state.last_iet_days {
            // Linear mixing: keep `lag1_autocorr` fraction of the prev, the
            // rest is the fresh quantile draw. Bounded ρ to [-1, 1] for safety.
            let rho = state.lag1_autocorr.clamp(-1.0, 1.0);
            rho * prev + (1.0 - rho.abs()) * raw_sample
        } else {
            raw_sample
        };
        state.last_iet_days = Some(coupled.max(0.0));
        coupled.max(0.0)
    }

    /// `true` if the sampler has explicit state for `source` (vs. falling back).
    pub fn has_source(&self, source: &str) -> bool {
        self.per_source.contains_key(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn known_state(values: Vec<f64>, autocorr: f64) -> SourceIetState {
        let n = values.len();
        SourceIetState {
            cdf_values: values,
            cdf_probabilities: (1..=n).map(|i| i as f64 / n as f64).collect(),
            lag1_autocorr: autocorr,
            last_iet_days: None,
        }
    }

    #[test]
    fn iet_sampler_returns_known_values() {
        let mut per_source = HashMap::new();
        per_source.insert("KR".to_string(), known_state(vec![1.0, 2.0, 5.0, 10.0], 0.0));
        let mut sampler = ConditionalIETSampler::from_state_map(
            per_source,
            known_state(vec![0.5, 1.0], 0.0),
        );
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..30 {
            let s = sampler.sample_next("KR", &mut rng);
            assert!([1.0, 2.0, 5.0, 10.0].contains(&s), "unexpected sample {s}");
        }
    }

    #[test]
    fn iet_sampler_falls_back_on_unknown_source() {
        let per_source = HashMap::new();
        let mut sampler = ConditionalIETSampler::from_state_map(
            per_source,
            known_state(vec![7.0], 0.0),
        );
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        assert!((sampler.sample_next("UNKNOWN", &mut rng) - 7.0).abs() < 1e-9);
    }

    #[test]
    fn iet_sampler_autocorr_couples_samples() {
        let mut per_source = HashMap::new();
        per_source.insert("A".to_string(), known_state(vec![1.0, 10.0], 0.9));
        let mut sampler = ConditionalIETSampler::from_state_map(
            per_source,
            known_state(vec![5.0], 0.0),
        );
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        // First sample is just a raw draw; subsequent samples are coupled.
        let first = sampler.sample_next("A", &mut rng);
        let second = sampler.sample_next("A", &mut rng);
        // With ρ=0.9, second ≈ 0.9 * first + 0.1 * raw_draw → second close to first.
        assert!(first.is_finite() && second.is_finite());
    }

    #[test]
    fn iet_sampler_never_returns_negative() {
        let mut per_source = HashMap::new();
        per_source.insert("X".to_string(), known_state(vec![0.0, 0.0, 0.0], -1.0));
        let mut sampler = ConditionalIETSampler::from_state_map(
            per_source,
            known_state(vec![0.0], 0.0),
        );
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..20 {
            let s = sampler.sample_next("X", &mut rng);
            assert!(s >= 0.0);
        }
    }
}
```

- [ ] **Step 2: Register module + re-exports in `distributions/mod.rs`**

In `crates/datasynth-core/src/distributions/mod.rs`, add (alphabetically):

```rust
pub mod conditional_iet;
```

And:

```rust
pub use conditional_iet::{ConditionalIETSampler, SourceIetState};
```

- [ ] **Step 3: Verify dev-deps (rand_chacha) in core**

```bash
grep -E "^rand_chacha" crates/datasynth-core/Cargo.toml
```

If missing in `[dev-dependencies]`, add `rand_chacha = { workspace = true }`.

- [ ] **Step 4: Test + clippy + commit**

```bash
cargo test -p datasynth-core --lib distributions::conditional_iet -- --test-threads=4 2>&1 | tail -10
cargo clippy -p datasynth-core -- -D warnings 2>&1 | tail -3
cargo fmt -p datasynth-core
git add crates/datasynth-core/src/distributions/ crates/datasynth-core/Cargo.toml
git commit -m "feat(core/distributions): ConditionalIETSampler — per-Source IET draws with lag-1 autocorr coupling"
```

Expected: 4 tests pass.

---

## Task 3 — BipartiteFanoutSampler

**Files:**
- Create: `crates/datasynth-core/src/distributions/fanout_sampler.rs`
- Modify: `crates/datasynth-core/src/distributions/mod.rs`

Picks an attribute value (GL Account, Cost Center, …) for an entity such that the resulting bipartite fan-out distribution matches the SP2 prior.

- [ ] **Step 1: Create `fanout_sampler.rs`**

```rust
//! Bipartite fan-out sampler driven by SP2's FanoutHistogram.

use std::collections::HashSet;

use rand::Rng;

/// One attribute value with a target fan-out (number of distinct entities
/// that should touch it) and a running set of users who have done so.
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

/// Picks attribute values for entities such that the resulting fan-out
/// distribution tracks the prior.
pub struct BipartiteFanoutSampler {
    pub buckets: Vec<AttributeBucket>,
}

impl BipartiteFanoutSampler {
    /// Build the sampler from `n_values` synthetic attribute values whose
    /// `target_fanout`s are drawn from the prior's histogram.
    ///
    /// `value_gen(i)` produces the synthetic attribute value for the i-th bucket.
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

    /// Pick an attribute value for `entity_id`. Prefers buckets with remaining
    /// capacity. If the entity has used a bucket before, that bucket is a
    /// candidate (no-op). On total saturation, picks the bucket with the
    /// largest deficit (least filled vs. target — least bad).
    pub fn pick_for<R: Rng>(&mut self, entity_id: &str, rng: &mut R) -> String {
        if self.buckets.is_empty() {
            return String::new();
        }
        // Candidates: buckets with capacity OR buckets already containing this entity.
        let candidate_idxs: Vec<usize> = (0..self.buckets.len())
            .filter(|&i| {
                self.buckets[i].has_capacity()
                    || self.buckets[i].current_users.contains(entity_id)
            })
            .collect();
        let chosen_idx = if !candidate_idxs.is_empty() {
            candidate_idxs[rng.gen_range(0..candidate_idxs.len())]
        } else {
            // Saturated — pick bucket with the most remaining capacity (= least negative).
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

    /// Snapshot of current fan-out counts (for assertions / inspection).
    pub fn current_fanouts(&self) -> Vec<u32> {
        self.buckets
            .iter()
            .map(|b| b.current_users.len() as u32)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn fanout_sampler_respects_targets() {
        // Two buckets, target fan-outs 3 and 1.
        let s = BipartiteFanoutSampler::new_with_targets(
            vec![3, 1],
            |i| format!("ACC-{i}"),
        );
        assert_eq!(s.buckets.len(), 2);
        assert_eq!(s.buckets[0].target_fanout, 3);
        assert_eq!(s.buckets[1].target_fanout, 1);
    }

    #[test]
    fn fanout_sampler_assigns_distinct_entities_to_buckets() {
        let mut s = BipartiteFanoutSampler::new_with_targets(
            vec![2, 2],
            |i| format!("ACC-{i}"),
        );
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let entities = ["E1", "E2", "E3", "E4"];
        for e in entities {
            let _ = s.pick_for(e, &mut rng);
        }
        let total_assignments: usize = s.buckets.iter().map(|b| b.current_users.len()).sum();
        assert_eq!(total_assignments, 4);
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
}
```

- [ ] **Step 2: Register + re-export in `distributions/mod.rs`**

```rust
pub mod fanout_sampler;

pub use fanout_sampler::{AttributeBucket, BipartiteFanoutSampler};
```

- [ ] **Step 3: Test + clippy + commit**

```bash
cargo test -p datasynth-core --lib distributions::fanout_sampler -- --test-threads=4 2>&1 | tail -8
cargo clippy -p datasynth-core -- -D warnings 2>&1 | tail -3
cargo fmt -p datasynth-core
git add crates/datasynth-core/src/distributions/
git commit -m "feat(core/distributions): BipartiteFanoutSampler for SP3 P3 motif preservation"
```

Expected: 3 tests pass.

---

## Task 4 — SourceActiveWindow

**Files:**
- Create: `crates/datasynth-core/src/distributions/source_active_window.rs`
- Modify: `crates/datasynth-core/src/distributions/mod.rs`

Each Source code gets a sampled active window (start_day, end_day) at sampler init; `is_active(source, day)` gates emission.

- [ ] **Step 1: Create `source_active_window.rs`**

```rust
//! Per-Source active-window sampler driven by SP2's ActiveLifetimePrior.

use std::collections::HashMap;

use rand::Rng;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveWindow {
    pub start_day: i64,
    pub end_day: i64,
}

impl ActiveWindow {
    pub fn contains(&self, day: i64) -> bool {
        day >= self.start_day && day <= self.end_day
    }

    pub fn length_days(&self) -> i64 {
        (self.end_day - self.start_day).max(0)
    }
}

/// For each known Source code, a randomly-placed active window whose length
/// is drawn from the prior's active-lifetime histogram.
pub struct SourceActiveWindow {
    pub by_source: HashMap<String, ActiveWindow>,
    pub period_days: i64,
}

impl SourceActiveWindow {
    /// Build by sampling one window per source.
    /// `lifetime_sampler(rng)` returns a sampled active-lifetime in days.
    pub fn build<R: Rng>(
        sources: &[String],
        period_days: i64,
        mut lifetime_sampler: impl FnMut(&mut R) -> i64,
        rng: &mut R,
    ) -> Self {
        let mut by_source = HashMap::new();
        for src in sources {
            let life = lifetime_sampler(rng).min(period_days).max(0);
            let max_start = (period_days - life).max(0);
            let start = if max_start == 0 { 0 } else { rng.gen_range(0..=max_start) };
            by_source.insert(
                src.clone(),
                ActiveWindow {
                    start_day: start,
                    end_day: start + life,
                },
            );
        }
        Self { by_source, period_days }
    }

    /// `true` if the source is allowed to emit on `day_in_period`. Unknown
    /// sources default to active for the full period (back-compat with sources
    /// the prior didn't observe).
    pub fn is_active(&self, source: &str, day_in_period: i64) -> bool {
        match self.by_source.get(source) {
            Some(w) => w.contains(day_in_period),
            None => day_in_period >= 0 && day_in_period < self.period_days,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn active_window_contains_known_range() {
        let w = ActiveWindow { start_day: 10, end_day: 20 };
        assert!(w.contains(10));
        assert!(w.contains(15));
        assert!(w.contains(20));
        assert!(!w.contains(9));
        assert!(!w.contains(21));
    }

    #[test]
    fn build_assigns_one_window_per_source() {
        let sources = vec!["KR".to_string(), "RE".to_string()];
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let saw = SourceActiveWindow::build(
            &sources,
            365,
            |r| r.gen_range(30..=180),
            &mut rng,
        );
        assert_eq!(saw.by_source.len(), 2);
        for w in saw.by_source.values() {
            assert!(w.length_days() >= 30 && w.length_days() <= 180);
            assert!(w.start_day >= 0);
            assert!(w.end_day <= 365);
        }
    }

    #[test]
    fn is_active_unknown_source_full_period() {
        let saw = SourceActiveWindow {
            by_source: HashMap::new(),
            period_days: 100,
        };
        assert!(saw.is_active("UNKNOWN", 0));
        assert!(saw.is_active("UNKNOWN", 99));
        assert!(!saw.is_active("UNKNOWN", 100));
        assert!(!saw.is_active("UNKNOWN", -1));
    }
}
```

- [ ] **Step 2: Register + re-export**

In `crates/datasynth-core/src/distributions/mod.rs`:

```rust
pub mod source_active_window;

pub use source_active_window::{ActiveWindow, SourceActiveWindow};
```

- [ ] **Step 3: Test + clippy + commit**

```bash
cargo test -p datasynth-core --lib distributions::source_active_window -- --test-threads=4 2>&1 | tail -8
cargo clippy -p datasynth-core -- -D warnings 2>&1 | tail -3
cargo fmt -p datasynth-core
git add crates/datasynth-core/src/distributions/
git commit -m "feat(core/distributions): SourceActiveWindow for SP3 active-lifetime honoring"
```

Expected: 3 tests pass.

---

## Task 5 — LoadedPriors + priors_loader

**Files:**
- Create: `crates/datasynth-generators/src/priors_loader.rs`
- Modify: `crates/datasynth-generators/src/lib.rs`
- Modify: `crates/datasynth-generators/Cargo.toml` (add `datasynth-fingerprint` dep if missing)

- [ ] **Step 1: Ensure `datasynth-fingerprint` is a dep of `datasynth-generators`**

```bash
grep -E "^datasynth-fingerprint" crates/datasynth-generators/Cargo.toml
```

If missing, add to `[dependencies]`:

```toml
datasynth-fingerprint = { workspace = true }
```

- [ ] **Step 2: Create `priors_loader.rs`**

```rust
//! Loads an SP2 industry-priors `.dsf` bundle and builds the SP3 sampler state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rand::Rng;
use thiserror::Error;

use datasynth_core::distributions::{
    BipartiteFanoutSampler, ConditionalIETSampler, SourceActiveWindow, SourceIetState,
};
use datasynth_fingerprint::io::FingerprintReader;
use datasynth_fingerprint::models::behavioral::{
    BehavioralPriors, LinesPerJePrior, PostingLagPrior, SourceMixPrior,
};

#[derive(Debug, Error)]
pub enum PriorsLoadError {
    #[error("priors bundle not found at {0}")]
    NotFound(PathBuf),
    #[error("bundle has no behavioral section")]
    MissingBehavioral,
    #[error("bundle industry mismatch: bundle={bundle}, requested={requested}")]
    IndustryMismatch { bundle: String, requested: String },
    #[error("fingerprint read error: {0}")]
    Read(String),
}

/// Conventional resource directory for committed industry-priors bundles.
pub fn bundled_priors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("priors")
}

/// Resolve the bundled `.dsf` path for an industry slug.
pub fn bundled_priors_path(industry: &str) -> PathBuf {
    bundled_priors_dir().join(format!("industry_priors_{industry}.dsf"))
}

/// Fully-built runtime priors consumed by je_generator and friends.
pub struct LoadedPriors {
    pub industry: String,
    pub bundle_path: PathBuf,
    pub source_mix: SourceMixPrior,
    pub iet_sampler: ConditionalIETSampler,
    pub lines_per_je: LinesPerJePrior,
    pub active_window: SourceActiveWindow,
    pub fanout_samplers: HashMap<String, BipartiteFanoutSampler>,
    pub posting_lag: Option<PostingLagPrior>,
}

impl LoadedPriors {
    /// Load the bundled prior for `industry`.
    pub fn load_bundled<R: Rng>(
        industry: &str,
        rng: &mut R,
        period_days: i64,
    ) -> Result<Self, PriorsLoadError> {
        Self::load_from_path(&bundled_priors_path(industry), rng, period_days, Some(industry))
    }

    /// Load from an explicit path. `expected_industry` (when Some) sanity-checks
    /// the bundle's industry tag.
    pub fn load_from_path<R: Rng>(
        path: &Path,
        rng: &mut R,
        period_days: i64,
        expected_industry: Option<&str>,
    ) -> Result<Self, PriorsLoadError> {
        if !path.exists() {
            return Err(PriorsLoadError::NotFound(path.to_path_buf()));
        }
        let reader = FingerprintReader::new();
        let fp = reader
            .read_from_file(path)
            .map_err(|e| PriorsLoadError::Read(e.to_string()))?;
        let bp = fp.behavioral.ok_or(PriorsLoadError::MissingBehavioral)?;
        if let Some(want) = expected_industry {
            if bp.industry != want {
                return Err(PriorsLoadError::IndustryMismatch {
                    bundle: bp.industry,
                    requested: want.to_string(),
                });
            }
        }
        Self::from_priors(bp, path.to_path_buf(), rng, period_days)
    }

    /// Build from an in-memory `BehavioralPriors`.
    pub fn from_priors<R: Rng>(
        bp: BehavioralPriors,
        bundle_path: PathBuf,
        rng: &mut R,
        period_days: i64,
    ) -> Result<Self, PriorsLoadError> {
        // Build the IET sampler.
        let mut per_source_states: HashMap<String, SourceIetState> = HashMap::new();
        for (src, summ) in &bp.per_source_iet.by_source {
            per_source_states.insert(
                src.clone(),
                SourceIetState {
                    cdf_values: summ.empirical_cdf_days.values.clone(),
                    cdf_probabilities: summ.empirical_cdf_days.probabilities.clone(),
                    lag1_autocorr: summ.lag1_autocorr,
                    last_iet_days: None,
                },
            );
        }
        let fallback = SourceIetState {
            cdf_values: vec![1.0],
            cdf_probabilities: vec![1.0],
            lag1_autocorr: 0.0,
            last_iet_days: None,
        };
        let iet_sampler = ConditionalIETSampler::from_state_map(per_source_states, fallback);

        // Build the active-window sampler. Lifetime sampler draws from
        // ActiveLifetimePrior.overall histogram.
        let lifetime_hist = bp.active_lifetime.overall.clone();
        let sources: Vec<String> = bp.source_mix.probabilities.keys().cloned().collect();
        let active_window =
            SourceActiveWindow::build(&sources, period_days,
                |r| lifetime_hist.sample_bucket(r) as i64, rng);

        // Build per-attribute fanout samplers.
        let mut fanout_samplers: HashMap<String, BipartiteFanoutSampler> = HashMap::new();
        for (attr, hist) in &bp.fanout.by_attribute {
            // Draw N target fanouts from the histogram. N defaults to 256 — a
            // pool size that lets the sampler avoid saturation for typical runs.
            const N_POOL: usize = 256;
            let targets: Vec<u32> = (0..N_POOL).map(|_| hist.sample_bucket(rng)).collect();
            let attr_prefix = attr.clone();
            let sampler = BipartiteFanoutSampler::new_with_targets(targets, move |i| {
                format!("{attr_prefix}-{i:04}")
            });
            fanout_samplers.insert(attr.clone(), sampler);
        }

        Ok(LoadedPriors {
            industry: bp.industry.clone(),
            bundle_path,
            source_mix: bp.source_mix,
            iet_sampler,
            lines_per_je: bp.lines_per_je,
            active_window,
            fanout_samplers,
            posting_lag: bp.posting_lag,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn bundled_priors_path_known() {
        let p = bundled_priors_path("health");
        assert!(p.ends_with("industry_priors_health.dsf"));
    }

    #[test]
    fn load_bundled_health_actually_works() {
        // Skip if the committed bundle isn't present (CI runs without the corpus
        // but the bundle is committed — should always be there).
        let p = bundled_priors_path("health");
        if !p.exists() {
            eprintln!("skipping: {} not present", p.display());
            return;
        }
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let priors = LoadedPriors::load_bundled("health", &mut rng, 365)
            .expect("load_bundled health");
        assert_eq!(priors.industry, "health");
        assert!(!priors.source_mix.probabilities.is_empty());
        assert!(priors.iet_sampler.has_source(
            priors.source_mix.probabilities.keys().next().unwrap()
        ));
        assert!(priors.fanout_samplers.contains_key("GLAccount"));
    }

    #[test]
    fn load_from_path_not_found() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let err = LoadedPriors::load_from_path(
            Path::new("/nonexistent.dsf"),
            &mut rng,
            365,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, PriorsLoadError::NotFound(_)));
    }
}
```

- [ ] **Step 3: Register module in `crates/datasynth-generators/src/lib.rs`**

Add:

```rust
pub mod priors_loader;
```

- [ ] **Step 4: Test + clippy + commit**

```bash
cargo test -p datasynth-generators --lib priors_loader -- --test-threads=4 2>&1 | tail -10
cargo clippy -p datasynth-generators -- -D warnings 2>&1 | tail -3
cargo fmt -p datasynth-generators
git add crates/datasynth-generators/src/lib.rs crates/datasynth-generators/src/priors_loader.rs crates/datasynth-generators/Cargo.toml
git commit -m "feat(generators/priors_loader): LoadedPriors from .dsf bundle for SP3 generators"
```

Expected: 3 tests pass.

---

## Task 6 — Config schema (industry_profile.priors)

**Files:**
- Modify: `crates/datasynth-config/src/schema.rs`

Add an optional `priors` sub-section to the existing `industry_profile` config block.

- [ ] **Step 1: Locate the existing IndustryProfile config block**

```bash
grep -nE "industry_profile|IndustryProfile|pub struct.*Profile" crates/datasynth-config/src/schema.rs | head -10
```

- [ ] **Step 2: Add a new struct + integrate into the existing profile**

Add (alongside existing config structs):

```rust
/// SP3 — opt-in priors-driven generation.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct IndustryPriorsConfig {
    /// `true` to enable priors-driven generation (opt-in; defaults to `false`).
    #[serde(default)]
    pub enabled: bool,
    /// `bundled` (default) loads from datasynth-generators/resources/priors/.
    /// `file` requires `path`.
    #[serde(default = "default_source")]
    pub source: PriorsSource,
    /// Required when `source = "file"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<std::path::PathBuf>,
}

fn default_source() -> PriorsSource {
    PriorsSource::Bundled
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PriorsSource {
    Bundled,
    File,
}

impl Default for PriorsSource {
    fn default() -> Self {
        PriorsSource::Bundled
    }
}
```

Add a `priors: Option<IndustryPriorsConfig>` field to the existing `IndustryProfile` (or equivalent struct — find via the grep). Use `#[serde(default, skip_serializing_if = "Option::is_none")]` to keep back-compat with existing configs.

If there is no current struct named `IndustryProfile` and the field lives somewhere else (e.g. a top-level `industry_profile: String`), promote that string field to a new struct that contains both `name: String` and `priors: Option<IndustryPriorsConfig>`. Preserve back-compat: when the old YAML form `industry_profile: retail` is encountered, deserialise as `IndustryProfile { name: "retail", priors: None }`. The `#[serde(untagged)]` pattern handles this — or use a custom Deserialize impl.

- [ ] **Step 3: Add validation (priors.source=file requires path)**

Where existing config validation lives (search for `fn validate` in the schema crate), append:

```rust
        if let Some(ref profile) = self.industry_profile {
            if let Some(ref priors) = profile.priors {
                if priors.enabled
                    && matches!(priors.source, PriorsSource::File)
                    && priors.path.is_none()
                {
                    return Err(SchemaError::invalid_config(
                        "industry_profile.priors.path is required when source = file",
                    ));
                }
            }
        }
```

Adapt to the actual config crate's error type.

- [ ] **Step 4: Add a unit test for the parsing + validation**

```rust
#[test]
fn industry_priors_default_disabled() {
    let yaml = r#"
industry_profile:
  name: health
"#;
    let cfg: SyntheticDataConfig = serde_yaml::from_str(yaml).expect("parse");
    let profile = cfg.industry_profile.expect("profile present");
    let priors = profile.priors.unwrap_or_default();
    assert!(!priors.enabled);
}

#[test]
fn industry_priors_enabled_bundled() {
    let yaml = r#"
industry_profile:
  name: health
  priors:
    enabled: true
    source: bundled
"#;
    let cfg: SyntheticDataConfig = serde_yaml::from_str(yaml).expect("parse");
    let priors = cfg.industry_profile.unwrap().priors.unwrap();
    assert!(priors.enabled);
    assert_eq!(priors.source, PriorsSource::Bundled);
}

#[test]
fn industry_priors_file_requires_path() {
    let yaml = r#"
industry_profile:
  name: health
  priors:
    enabled: true
    source: file
"#;
    let cfg: SyntheticDataConfig = serde_yaml::from_str(yaml).expect("parse");
    let err = cfg.validate().expect_err("path required");
    assert!(err.to_string().contains("path is required"));
}
```

Adapt struct names to the actual top-level config type (likely `SyntheticDataConfig` or similar).

- [ ] **Step 5: Test + clippy + commit**

```bash
cargo test -p datasynth-config --lib -- --test-threads=4 2>&1 | tail -10
cargo clippy -p datasynth-config -- -D warnings 2>&1 | tail -3
cargo fmt -p datasynth-config
git add crates/datasynth-config/src/schema.rs
git commit -m "feat(config): industry_profile.priors sub-section for SP3 opt-in"
```

---

## Task 7 — CSV writer: trading_partner column

**Files:**
- Modify: the CSV writer for journal_entries (find in datasynth-output)

The `JournalEntryLine.trading_partner: Option<String>` field already exists. SP3 surfaces it in `journal_entries.csv`.

- [ ] **Step 1: Locate the journal_entries CSV writer**

```bash
grep -rln "journal_entries.csv\|JournalEntry" crates/datasynth-output/src/ 2>&1 | head -5
grep -nE "trading_partner|cost_center|profit_center|line_text" crates/datasynth-output/src/csv_writer.rs 2>&1 | head -20
```

- [ ] **Step 2: Append `trading_partner` to the CSV column list**

Find the header-write call (something like `writer.write_record(&["...", "cost_center", "profit_center", ...])` or constructed from a constant). Append `"trading_partner"` to the END of the column list.

Find the per-line record-write call (something like `writer.write_record([..., &line.cost_center, &line.profit_center, ...])`). Append `&line.trading_partner.as_deref().unwrap_or("")` to the END of the record.

Both edits must happen in the same place; if the file constructs the row in multiple places, edit all of them. Verify with `grep -c trading_partner` after.

- [ ] **Step 3: Add a unit test that round-trips a JE with a TP**

In the same file's `#[cfg(test)] mod tests` (or create one):

```rust
    #[test]
    fn csv_writer_includes_trading_partner_column() {
        // Construct one JE with one line carrying trading_partner=Some("VENDOR_A")
        // and another line with trading_partner=None.
        // Write to an in-memory buffer; verify the header contains "trading_partner"
        // and the value column has "VENDOR_A" then empty.
        let mut buf = Vec::<u8>::new();
        // … construct JE + line objects per existing factory helpers …
        // … call the existing write function with `&mut buf` …
        let csv_text = String::from_utf8(buf).expect("utf8");
        let mut lines = csv_text.lines();
        let header = lines.next().expect("header");
        assert!(header.contains("trading_partner"), "header missing TP: {header}");
        let first_line = lines.next().expect("first line");
        assert!(first_line.contains("VENDOR_A"), "first line missing VENDOR_A: {first_line}");
    }
```

Adapt the test to use whatever helper the existing test mod uses for building JEs.

- [ ] **Step 4: Test + commit**

```bash
cargo test -p datasynth-output --lib -- --test-threads=4 2>&1 | tail -10
cargo clippy -p datasynth-output -- -D warnings 2>&1 | tail -3
cargo fmt -p datasynth-output
git add crates/datasynth-output/
git commit -m "feat(output): append trading_partner column to journal_entries.csv (SP3)"
```

---

## Task 8 — p2p_generator populates trading_partner

**Files:**
- Modify: `crates/datasynth-generators/src/document_flow/p2p_generator.rs`

When the P2P generator emits JE lines (from Payment, VendorInvoice, GoodsReceipt), it knows the `vendor_id`. SP3 plumbs it to `JournalEntryLine.trading_partner`.

- [ ] **Step 1: Locate where p2p_generator emits JE lines**

```bash
grep -nE "JournalEntryLine|line_text|cost_center|push.*Line|to_journal_entry" crates/datasynth-generators/src/document_flow/p2p_generator.rs 2>&1 | head -25
```

- [ ] **Step 2: At each line-construction site, set trading_partner**

Find each construction of `JournalEntryLine { … }` or `JournalEntryLine::new(…)`. After construction, set:

```rust
                line.trading_partner = Some(vendor_id.clone());
```

If the helper takes a builder/With-pattern, set via the builder. If it's a struct literal, add the field.

For lines that aren't vendor-linked (e.g. tax expense to a tax-clearing account), leave `trading_partner: None`.

- [ ] **Step 3: Add a smoke unit test**

In `p2p_generator.rs`'s test mod (or create one):

```rust
    #[test]
    fn p2p_generator_populates_trading_partner_for_vendor_lines() {
        // Build a minimal vendor invoice / payment via existing test helpers.
        // Call the JE-derivation function.
        // Assert at least one resulting line has trading_partner = Some(vendor_id).
        // …
    }
```

- [ ] **Step 4: Test + commit**

```bash
cargo test -p datasynth-generators --lib document_flow::p2p_generator -- --test-threads=4 2>&1 | tail -10
cargo clippy -p datasynth-generators -- -D warnings 2>&1 | tail -3
cargo fmt -p datasynth-generators
git add crates/datasynth-generators/src/document_flow/p2p_generator.rs
git commit -m "feat(generators/p2p): populate JournalEntryLine.trading_partner from vendor_id"
```

---

## Task 9 — o2c_generator populates trading_partner

**Files:**
- Modify: `crates/datasynth-generators/src/document_flow/o2c_generator.rs`

Mirror of Task 8 for the O2C flow (CustomerInvoice, CustomerReceipt, Delivery → customer_id).

- [ ] **Step 1: Locate JE-line construction sites**

```bash
grep -nE "JournalEntryLine|line_text|customer_id|push.*Line" crates/datasynth-generators/src/document_flow/o2c_generator.rs 2>&1 | head -25
```

- [ ] **Step 2: Set `trading_partner = Some(customer_id.clone())` at each line construction**

Mirror Task 8 pattern.

- [ ] **Step 3: Smoke unit test**

```rust
    #[test]
    fn o2c_generator_populates_trading_partner_for_customer_lines() {
        // Build a minimal customer invoice via existing test helpers.
        // Call the JE-derivation function.
        // Assert at least one resulting line has trading_partner = Some(customer_id).
        // …
    }
```

- [ ] **Step 4: Test + commit**

```bash
cargo test -p datasynth-generators --lib document_flow::o2c_generator -- --test-threads=4 2>&1 | tail -10
cargo clippy -p datasynth-generators -- -D warnings 2>&1 | tail -3
cargo fmt -p datasynth-generators
git add crates/datasynth-generators/src/document_flow/o2c_generator.rs
git commit -m "feat(generators/o2c): populate JournalEntryLine.trading_partner from customer_id"
```

---

## Task 10 — Thread LoadedPriors through je_generator's context

**Files:**
- Modify: `crates/datasynth-generators/src/je_generator.rs`

Before the per-call rewires (Tasks 11-14), add an `Option<LoadedPriors>` field to whatever context type `je_generator` uses, and route it through from the orchestrator.

- [ ] **Step 1: Find the je_generator entry point and context struct**

```bash
grep -nE "pub fn generate_journal_entry|pub struct.*Context|impl.*Generator" crates/datasynth-generators/src/je_generator.rs 2>&1 | head -15
```

Likely candidates: a `JeGenerator` struct, or a free function taking `&Config`. The exact shape determines where `LoadedPriors` lives.

- [ ] **Step 2: Add the field**

If `JeGenerator` is a struct, add:

```rust
    /// SP3 — runtime priors when `industry_profile.priors.enabled = true`.
    pub loaded_priors: Option<crate::priors_loader::LoadedPriors>,
```

If the generator is a set of free functions threading a context type, add the field there. If state is owned by `EnhancedOrchestrator` in datasynth-runtime, the field belongs there and gets passed in via the existing call site.

- [ ] **Step 3: Wire init from config**

Find the existing construction of the generator (in `datasynth-runtime/src/orchestrator.rs` or similar). When `config.industry_profile.priors.enabled = true`, build `LoadedPriors` and pass it in:

```rust
let loaded_priors = if matches!(
    config.industry_profile.as_ref().and_then(|p| p.priors.as_ref()).map(|p| p.enabled),
    Some(true)
) {
    let profile = config.industry_profile.as_ref().unwrap();
    let priors_cfg = profile.priors.as_ref().unwrap();
    let mut priors_rng = make_priors_rng(config.global.seed);
    let period_days = config.global.period_months as i64 * 30;
    let priors = match priors_cfg.source {
        datasynth_config::schema::PriorsSource::Bundled => {
            crate::priors_loader::LoadedPriors::load_bundled(&profile.name, &mut priors_rng, period_days)?
        }
        datasynth_config::schema::PriorsSource::File => {
            let path = priors_cfg.path.as_ref().ok_or_else(|| {
                anyhow::anyhow!("industry_profile.priors.path required for source: file")
            })?;
            crate::priors_loader::LoadedPriors::load_from_path(path, &mut priors_rng, period_days, Some(&profile.name))?
        }
    };
    Some(priors)
} else {
    None
};
```

Adapt to the actual context-construction site.

- [ ] **Step 4: Compile + commit (no behavior change yet)**

```bash
cargo build -p datasynth-generators -p datasynth-runtime 2>&1 | tail -3
git add crates/datasynth-generators/src/je_generator.rs
git commit -m "feat(generators/je): thread Option<LoadedPriors> through je_generator context (SP3 prep)"
```

---

## Task 11 — je_generator: IET timing rewire

**Files:**
- Modify: `crates/datasynth-generators/src/je_generator.rs`

The existing code uses a global Poisson-distributed posting timer per day. When `loaded_priors` is present, replace with per-Source IET draws.

- [ ] **Step 1: Find the timing loop**

```bash
grep -nE "posting_date|day_offset|posting_count_today|poisson|Poisson" crates/datasynth-generators/src/je_generator.rs 2>&1 | head -25
```

Look for a function that iterates over days within the period and decides how many JEs to emit per day per Source.

- [ ] **Step 2: Add a gated branch**

At the inner emission decision, replace something like:

```rust
let n_today = poisson_sample(rate, rng);
```

with:

```rust
let n_today = if let Some(priors) = ctx.loaded_priors.as_mut() {
    // Drive emission via per-Source IET: accumulate samples until we hit "today + 1 day".
    let mut n = 0u32;
    let mut accum_days = 0.0f64;
    loop {
        let iet = priors.iet_sampler.sample_next(source, rng);
        accum_days += iet;
        if accum_days >= 1.0 { break; }
        n += 1;
        if n > 10_000 { break; }  // safety bound
    }
    n
} else {
    poisson_sample(rate, rng)
};
```

Exact wiring depends on the existing loop shape — adapt while preserving the disabled (`None`) path.

- [ ] **Step 3: Smoke build + commit**

```bash
cargo build -p datasynth-generators 2>&1 | tail -3
cargo test -p datasynth-generators --lib -- --test-threads=4 2>&1 | tail -5
git add crates/datasynth-generators/src/je_generator.rs
git commit -m "feat(generators/je): SP3 IET timing rewire — per-Source samples when priors loaded"
```

Existing tests should still pass (the `None` path is unchanged).

---

## Task 12 — je_generator: lines-per-JE rewire

**Files:**
- Modify: `crates/datasynth-generators/src/je_generator.rs`

- [ ] **Step 1: Find the lines-per-JE draw**

```bash
grep -nE "lines_per_entry|line_count|n_lines|num_lines|gen_range.*lines" crates/datasynth-generators/src/je_generator.rs 2>&1 | head -20
```

- [ ] **Step 2: Gate replace with prior-driven sampling**

Where the existing code computes the line count for a JE (probably from the config-driven range), insert:

```rust
let n_lines = if let Some(priors) = &ctx.loaded_priors {
    let hist = priors
        .lines_per_je
        .by_source
        .get(source)
        .unwrap_or(&priors.lines_per_je.overall);
    hist.sample_bucket(rng).max(2) as usize
} else {
    /* existing computation */
};
```

`.max(2)` ensures every JE has at least 2 lines (one debit + one credit) for balance-validity.

- [ ] **Step 3: Build + test + commit**

```bash
cargo build -p datasynth-generators 2>&1 | tail -3
cargo test -p datasynth-generators --lib -- --test-threads=4 2>&1 | tail -5
git add crates/datasynth-generators/src/je_generator.rs
git commit -m "feat(generators/je): SP3 lines-per-JE rewire — sampled from prior histogram"
```

---

## Task 13 — je_generator: fanout rewire (GL Account / Cost Center / Profit Center)

**Files:**
- Modify: `crates/datasynth-generators/src/je_generator.rs`

- [ ] **Step 1: Find the GL/CC/PC assignment points**

```bash
grep -nE "gl_account|cost_center|profit_center|account_id" crates/datasynth-generators/src/je_generator.rs 2>&1 | head -30
```

Look for the line-building loop where each JE line gets its GL Account, Cost Center, and Profit Center assigned.

- [ ] **Step 2: Gate replace each assignment with the fanout sampler**

At each assignment:

```rust
let gl_account = if let Some(priors) = ctx.loaded_priors.as_mut() {
    priors
        .fanout_samplers
        .get_mut("GLAccount")
        .map(|s| s.pick_for(source, rng))
        .unwrap_or_else(|| /* existing fallback */)
} else {
    /* existing assignment */
};
```

Repeat for `CostCenter` and `ProfitCenter` using the same pattern with the relevant fanout sampler. Skip `TradingPartner` here — that's driven by P2P/O2C linkage (Tasks 8/9), not by the fanout sampler.

- [ ] **Step 3: Build + test + commit**

```bash
cargo build -p datasynth-generators 2>&1 | tail -3
cargo test -p datasynth-generators --lib -- --test-threads=4 2>&1 | tail -5
git add crates/datasynth-generators/src/je_generator.rs
git commit -m "feat(generators/je): SP3 fanout rewire — Source-conditional GL/CC/PC assignment"
```

---

## Task 14 — je_generator: active-window gating

**Files:**
- Modify: `crates/datasynth-generators/src/je_generator.rs`

- [ ] **Step 1: Find the per-day per-Source loop**

```bash
grep -nE "for.*day|day_offset|while.*period|posting_date\s*=" crates/datasynth-generators/src/je_generator.rs 2>&1 | head -25
```

- [ ] **Step 2: Add active-window gating before per-day emission**

Inside the day loop, before any Source-specific emission decision:

```rust
let day_in_period = (posting_date - config.global.period_start).num_days();
if let Some(priors) = &ctx.loaded_priors {
    if !priors.active_window.is_active(source, day_in_period) {
        continue;
    }
}
```

Adapt to the actual day/source iteration shape.

- [ ] **Step 3: Build + test + commit**

```bash
cargo build -p datasynth-generators 2>&1 | tail -3
cargo test -p datasynth-generators --lib -- --test-threads=4 2>&1 | tail -5
git add crates/datasynth-generators/src/je_generator.rs
git commit -m "feat(generators/je): SP3 active-window gating — sources only emit when in their window"
```

---

## Task 15 — Priors-driven smoke integration test

**Files:**
- Create: `crates/datasynth-generators/tests/sp3_priors_smoke.rs`

- [ ] **Step 1: Write the smoke test**

```rust
//! Integration smoke for SP3 priors-driven generation.

use datasynth_config::schema::SyntheticDataConfig;

#[test]
fn priors_enabled_generates_lines_per_je_close_to_prior() {
    // 1) Build a minimal demo config + enable industry priors for health.
    // 2) Run the generator end-to-end into a temp dir.
    // 3) Read back journal_entries.csv, compute the lines-per-JE histogram.
    // 4) Load the bundled health prior, compare W1 to the prior — should be small.
    // 5) Assert trading_partner is populated on rows derived from P2P/O2C docs.
    let tmp = tempfile::tempdir().expect("tempdir");
    let yaml = r#"
global:
  seed: 42
  period_months: 1
companies:
  count: 1
industry_profile:
  name: health
  priors:
    enabled: true
    source: bundled
"#;
    let _cfg: SyntheticDataConfig = serde_yaml::from_str(yaml).expect("parse");
    // … invoke whichever orchestrator entry point is the simplest "config -> dir" path …
    let _out_dir = tmp.path().to_path_buf();
    // Read journal_entries.csv, count lines per JE, build histogram, assert.
    // (Skip the full E2E assertions if the orchestrator entry point can't run
    // in a unit-test environment — the build alone is the smoke check.)
}
```

The exact orchestrator entry point depends on what's exposed. If running the full pipeline from a test is impractical, the test should at least exercise the priors_loader + sampler wiring with a small in-memory dataset.

- [ ] **Step 2: Run + commit**

```bash
cargo test -p datasynth-generators --test sp3_priors_smoke -- --test-threads=4 2>&1 | tail -10
git add crates/datasynth-generators/tests/sp3_priors_smoke.rs
git commit -m "test(generators/sp3): priors-driven smoke integration test"
```

---

## Task 16 — Backward-compat integration test

**Files:**
- Create: `crates/datasynth-generators/tests/sp3_backward_compat.rs`

- [ ] **Step 1: Write the test**

```rust
//! Verify that priors.enabled = false produces output byte-equivalent to a
//! v5.11 reference (except for the always-emitted trading_partner column).

use std::path::PathBuf;

#[test]
fn priors_disabled_matches_pre_sp3_baseline_modulo_tp_column() {
    // The reference: a journal_entries.csv produced with `seed=42, period=1mo,
    // demo config, priors disabled (or absent)`. We capture this once and
    // commit it as a fixture under crates/datasynth-generators/tests/fixtures/.

    // 1) Run the generator with priors.enabled = false.
    // 2) Read both CSVs.
    // 3) Strip the trading_partner column from the actual output.
    // 4) Compare line-by-line (or row-by-row after sorting if order is
    //    nondeterministic). They should match byte-for-byte.

    // If the fixture doesn't exist (first run), regenerate with --ignored flag.
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sp3_pre_sp3_journal_entries.csv");
    if !fixture.exists() {
        eprintln!("Fixture missing at {} — regenerate via --ignored test.", fixture.display());
        return;
    }
    // … run generator, compare …
}

#[test]
#[ignore = "regenerate fixture"]
fn regenerate_backward_compat_fixture() {
    // Run the demo config without priors, write to
    // tests/fixtures/sp3_pre_sp3_journal_entries.csv. Manual one-shot.
}
```

The fixture lives at `crates/datasynth-generators/tests/fixtures/sp3_pre_sp3_journal_entries.csv` and is regenerated on demand.

- [ ] **Step 2: Run + commit**

```bash
cargo test -p datasynth-generators --test sp3_backward_compat -- --test-threads=4 2>&1 | tail -10
git add crates/datasynth-generators/tests/sp3_backward_compat.rs
git commit -m "test(generators/sp3): backward-compat — priors-disabled output unchanged modulo TP column"
```

---

## Task 17 — CI workflow

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Append two named steps adjacent to the SP1/SP2 behavioral steps**

```yaml
      - name: SP3 priors smoke
        run: cargo test -p datasynth-generators --test sp3_priors_smoke -- --test-threads=4
      - name: SP3 backward-compat
        run: cargo test -p datasynth-generators --test sp3_backward_compat -- --test-threads=4
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: SP3 priors-smoke + backward-compat tests on PR"
```

---

## Task 18 — Docs (entity-aware-generation.md + CHANGELOG + README + CLAUDE.md)

**Files:**
- Create: `docs/entity-aware-generation.md`
- Modify: `CHANGELOG.md` (if it exists; if not, create a `CHANGELOG.md` with a v5.12 entry)
- Modify: `README.md`
- Modify: `CLAUDE.md`
- Modify: `docs/behavioral-fidelity.md` (cross-link)
- Modify: `docs/real-world-priors.md` (cross-link)

- [ ] **Step 1: Write `docs/entity-aware-generation.md`**

```markdown
# Entity-aware generation (SP3)

v5.12 adds opt-in priors-driven generation: when an industry profile is configured with priors enabled, DataSynth's journal-entry generator routes its RNG through SP2's industry-priors bundles to match corpus distributions on the SP1 baseline gaps.

## Opt-in via config

\`\`\`yaml
industry_profile:
  name: health
  priors:
    enabled: true       # default: false (opt-in)
    source: bundled     # default: bundled
    # path: ~           # required when source: file
\`\`\`

When `priors.enabled: false` (or absent), generator behavior is identical to v5.11.

## What changes when enabled

| SP1 baseline gap   | DR (v5.10)  | SP3 fix                                            |
| ------------------ | ----------- | -------------------------------------------------- |
| P1 IETD            | 60.1×       | Per-Source IET drawn from prior's empirical CDF    |
| P2 JE-line-burst   | 452.8×      | lines_per_je sampled from per-Source histogram     |
| P2 active lifetime | 23.2×       | Per-Source active window gates emission            |
| P3 motifs          | 11–345×     | Bipartite fan-out sampler for GL / CC / PC         |
| P4 mean gap        | 4.5×        | Source-mix re-weighted from prior                  |

## Trading Partner column

`journal_entries.csv` now always carries a `trading_partner` column (appended after existing columns). Populated from `vendor_id` for P2P-derived rows, `customer_id` for O2C-derived rows, empty for pure SA postings. ACDOCA output is unchanged.

## Bundle resolution

`source: bundled` resolves to:

\`\`\`
crates/datasynth-generators/resources/priors/industry_priors_{industry}.dsf
\`\`\`

Five bundles ship in v5.12: health, life_sciences, pharmaceutical, power_and_utilities, technology.

`source: file` accepts an explicit `path:` for custom bundles (e.g. user-extracted via `datasynth-data fingerprint extract --behavioral`).

## See also

- [docs/behavioral-fidelity.md](behavioral-fidelity.md) — SP1 evaluation framework
- [docs/real-world-priors.md](real-world-priors.md) — SP2 bundle extraction
```

- [ ] **Step 2: Append a v5.12 CHANGELOG entry**

If `CHANGELOG.md` exists, prepend a v5.12 block. If not, create one with at least:

```markdown
# Changelog

## v5.12 (SP3 — entity-aware generation)

### Added

- `industry_profile.priors` config sub-section (opt-in): drives generation from SP2 priors bundles.
- `trading_partner` column on `journal_entries.csv` (always emitted, populated for P2P/O2C-derived rows).
- New samplers in `datasynth-core::distributions`: `ConditionalIETSampler`, `BipartiteFanoutSampler`, `SourceActiveWindow`.
- `datasynth-generators::priors_loader::LoadedPriors` for one-time bundle loading.

### Behavior

- With `industry_profile.priors.enabled: true`, lines-per-JE, per-Source IET, attribute fan-out, and source-active-window are all driven by the configured industry's prior bundle. Default (priors disabled) behavior is unchanged.
```

- [ ] **Step 3: Append README + CLAUDE.md entries**

In README, after the SP2 priors section:

```markdown
### Entity-aware generation (v5.12+)

Opt-in via `industry_profile.priors.enabled: true`. Consumes SP2 priors to match corpus behavioral distributions. See [docs/entity-aware-generation.md](docs/entity-aware-generation.md).
```

In CLAUDE.md, near the Generator Modules section:

```markdown
- priors_loader.rs: SP3 — loads industry-priors `.dsf` bundle into runtime samplers (ConditionalIETSampler, BipartiteFanoutSampler, SourceActiveWindow).
```

- [ ] **Step 4: Cross-link from existing behavioral-fidelity / real-world-priors docs**

In `docs/behavioral-fidelity.md`, after the "What it measures" section:

```markdown
**See also:** [docs/entity-aware-generation.md](entity-aware-generation.md) — SP3 generators consume the priors mined by SP2 to close the gaps measured here.
```

In `docs/real-world-priors.md`, near the bottom:

```markdown
**Consumed by:** [docs/entity-aware-generation.md](entity-aware-generation.md) — SP3 generators consume these bundles when `industry_profile.priors.enabled: true`.
```

- [ ] **Step 5: Commit**

```bash
git add docs/ CHANGELOG.md README.md CLAUDE.md
git commit -m "docs(sp3): entity-aware generation user guide + CHANGELOG v5.12 + cross-links"
```

---

## Self-review (run inline)

Trace each spec section to a task:

| Spec section                              | Implementing task(s) |
| ----------------------------------------- | -------------------- |
| §2.1 Crate placement                      | T1 (helper), T2-T4 (samplers), T5 (loader), T6 (config) |
| §2.2 Sampler interfaces                   | T2, T3, T4           |
| §2.3 LoadedPriors                         | T5                   |
| §2.4 Generator rewires                    | T10, T11, T12, T13, T14 |
| §2.5 Config schema                        | T6                   |
| §2.6 Output schema (TP column)            | T7                   |
| §3 Data flow                              | All gates in T10-T14 |
| §4.1 Sampler unit tests                   | T2, T3, T4 (each has tests) |
| §4.2 Integration smoke                    | T15                  |
| §4.3 Backward-compat                      | T16                  |
| §4.4 End-to-end behavioral re-run         | manual, post-T18     |
| §4.5 CI integration                       | T17                  |
| §5.1 Dependencies                         | T5 (datasynth-fingerprint dep) |
| §5.2 Risks                                | Documented in spec; mitigations in gated branches (T11-T14) and tests (T15, T16) |
| §6 Acceptance criteria                    | T1-T18 collectively  |
| §7 Implementation phases                  | T1-T4=Phase A, T5=B, T6=C, T10-T14=D, T7-T9=E, T15-T18=F |
| §8 Out-of-scope                           | enforced by what's NOT in this plan |

All spec sections covered. No placeholders.

Type-name consistency check: `ConditionalIETSampler`, `SourceIetState`, `BipartiteFanoutSampler`, `AttributeBucket`, `SourceActiveWindow`, `ActiveWindow`, `LoadedPriors`, `PriorsLoadError`, `IndustryPriorsConfig`, `PriorsSource` — used consistently across tasks.

Method-name consistency: `sample_next`, `sample_bucket`, `pick_for`, `is_active`, `load_bundled`, `load_from_path`, `from_priors`, `from_state_map` — used consistently.

---

## Execution

Plan complete. Subagent-driven dispatch per the user's standing autonomous mode. Tasks dispatch serially (skill's "never parallel implementer subagents" rule). Wave milestones:

- **Wave 1**: T1 (histogram helper)
- **Wave 2**: T2, T3, T4 (three samplers — combine into one subagent dispatch for efficiency, three serial commits)
- **Wave 3**: T5 (priors_loader)
- **Wave 4**: T6 (config schema)
- **Wave 5**: T7 (CSV writer TP column)
- **Wave 6**: T8, T9 (P2P + O2C TP propagation — combine, two commits)
- **Wave 7**: T10 (je_generator context plumbing)
- **Wave 8**: T11, T12, T13, T14 (je_generator rewires — combine, four commits)
- **Wave 9**: T15, T16 (integration tests)
- **Wave 10**: T17, T18 (CI + docs)

Verification between waves: `cargo build --release && cargo test -p datasynth-generators --lib -- --test-threads=4 && cargo clippy --workspace`. After T18 lands, run the manual behavioral-fidelity re-run (config with priors enabled → SP1 scorer → commit new baseline at `docs/baselines/2026-05-XX-sp3-v5.12.0/`).

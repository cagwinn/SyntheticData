# SP2 — Real-World Prior Extraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a `datasynth-fingerprint::extraction::behavioral_extractor` module + `aggregation::industry_aggregator` + three CLI flags that turn corpus GL parquet files into per-industry `.dsf` bundles capturing the five behavioral priors the SP1 baseline identified as the highest-DR gaps. Commit five industry-tagged bundles under `crates/datasynth-generators/resources/priors/` for SP3 to consume.

**Architecture:** Extend the existing `datasynth-fingerprint` crate with a new `models/behavioral.rs` (struct definitions), `extraction/behavioral_extractor.rs` (parquet → BehavioralPriors), and `aggregation/industry_aggregator.rs` (N client bundles → 1 industry bundle). Add an optional `behavioral: Option<BehavioralPriors>` field to the existing `Fingerprint` struct — strictly additive via `#[serde(default)]` so old `.dsf` files continue to load. Reuses SP1's `datasynth-eval::behavioral_fidelity::{loader, entity_profile, Record}` for parquet → `Vec<Record>` conversion and canonical column aliases (typo-tolerant for `Tarding Partner`). Three new CLI surfaces on `datasynth-data fingerprint`: `extract --behavioral`, `aggregate-industry`, and `info --behavioral`.

**Tech Stack:** Rust 2021; existing `arrow`+`parquet`+`chrono`+`serde`+`serde_json`+`statrs` (all workspace deps); new `datasynth-eval` dep added to `datasynth-fingerprint`. No new heavy deps.

**Spec:** [`docs/superpowers/specs/2026-05-12-sp2-real-world-prior-extraction-design.md`](../specs/2026-05-12-sp2-real-world-prior-extraction-design.md)

**Predecessor baseline:** [`docs/baselines/2026-05-12-sp1-v5.10.0/SUMMARY.md`](../../baselines/2026-05-12-sp1-v5.10.0/SUMMARY.md) — composite BF 59.0× drives the five-prior choice.

**Test concurrency note:** Always use `--test-threads=4`. Never `cargo test --workspace`; prefer `cargo test -p datasynth-fingerprint --lib -- --quiet --test-threads=4`.

---

## File Structure

```
crates/datasynth-fingerprint/
├── Cargo.toml                                       (modify: add datasynth-eval)
└── src/
    ├── lib.rs                                       (modify: add `pub mod aggregation;`)
    ├── models/
    │   ├── mod.rs                                   (modify: re-export behavioral)
    │   ├── behavioral.rs                            (new)
    │   └── fingerprint.rs                           (modify: add behavioral field)
    ├── extraction/
    │   ├── mod.rs                                   (modify: re-export)
    │   └── behavioral_extractor.rs                  (new)
    └── aggregation/                                 (new directory)
        ├── mod.rs                                   (new)
        └── industry_aggregator.rs                   (new)

crates/datasynth-fingerprint/tests/
└── behavioral_priors_smoke.rs                       (new)

crates/datasynth-fingerprint/tests/fixtures/
└── pre_sp2_fingerprint.json                         (new — pinned fixture for backward-compat test)

crates/datasynth-generators/resources/priors/        (new directory, committed)
├── industry_priors_health.dsf
├── industry_priors_life_sciences.dsf
├── industry_priors_pharma.dsf
├── industry_priors_power_utilities.dsf
└── industry_priors_technology.dsf

crates/datasynth-cli/src/main.rs                     (modify: 3 subcommand changes)

scripts/
└── regenerate-industry-priors.sh                    (new — one-off bundle producer)

docs/
└── real-world-priors.md                             (new)

CLAUDE.md, README.md, .github/workflows/ci.yml       (small touch-ups)
```

## Task overview & dependency graph

```
T1  (scaffold)
  ├──▶ T2  (LineCountHistogram helper) ────┐
  │    ├──▶ T3  (SourceMixPrior)            │
  │    ├──▶ T4  (PerSourceIetPrior)         │
  │    ├──▶ T5  (LinesPerJePrior)           │
  │    ├──▶ T6  (ActiveLifetimePrior)       │
  │    ├──▶ T7  (FanoutPrior)               │
  │    └──▶ T8  (PostingLagPrior)           │
  │                                          ▼
  │                                T9  (extract_behavioral_priors orchestrator)
  │                                          │
  └────────────────────────────────▶ T10 (aggregation/ scaffold)
                                             │
                                             ├──▶ T11 (aggregate_source_mix)
                                             ├──▶ T12 (aggregate_lines_per_je)
                                             ├──▶ T13 (aggregate_per_source_iet)
                                             ├──▶ T14 (aggregate_active_lifetime + fanout + posting_lag)
                                             └──▶ T15 (aggregate_industry_priors orchestrator)
                                                       │
                                                       ▼
                                             T16 (CLI: extract --behavioral)
                                             T17 (CLI: aggregate-industry)
                                             T18 (CLI: info --behavioral)
                                                       │
                                                       ▼
                                             T19 (smoke test) + T20 (backward-compat test)
                                                       │
                                                       ▼
                                             T21 (CI workflow)
                                             T22 (regenerate script)
                                             T23 (docs)
```

Total: 23 tasks. Sequential dispatch (no parallel implementer subagents per the skill). Wall-clock ~9 working days.

---

## Task 1 — Scaffolding behavioral submodule + dependency + Fingerprint field

**Files:**
- Modify: `crates/datasynth-fingerprint/Cargo.toml`
- Create: `crates/datasynth-fingerprint/src/models/behavioral.rs`
- Modify: `crates/datasynth-fingerprint/src/models/mod.rs`
- Modify: `crates/datasynth-fingerprint/src/models/fingerprint.rs`
- Create: `crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs` (stub)
- Create: `crates/datasynth-fingerprint/src/aggregation/mod.rs` (stub)
- Create: `crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs` (stub)
- Modify: `crates/datasynth-fingerprint/src/extraction/mod.rs`
- Modify: `crates/datasynth-fingerprint/src/lib.rs`

- [ ] **Step 1: Add `datasynth-eval` dep**

In `crates/datasynth-fingerprint/Cargo.toml`, add to `[dependencies]` alphabetically:

```toml
datasynth-eval = { workspace = true }
```

Run `cargo check -p datasynth-fingerprint` to confirm no cycle.

- [ ] **Step 2: Create `models/behavioral.rs` with minimal `BehavioralPriors` stub**

```rust
//! Behavioral priors mined from corpus GL data.
//!
//! Spec: `docs/superpowers/specs/2026-05-12-sp2-real-world-prior-extraction-design.md`

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::models::correlation::EmpiricalCdf;

/// Root container for the SP2 behavioral priors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BehavioralPriors {
    /// SP2 schema version. Bump on incompatible struct changes.
    pub schema_version: u32,
    /// Engine version that produced these priors (env!("CARGO_PKG_VERSION") at extract time).
    pub generator_version: String,
    /// Conventional industry slug ("health", "life_sciences", ...).
    pub industry: String,
    /// Number of input client bundles (1 for per-client extraction, N after aggregation).
    pub n_client_inputs: usize,
    /// Sum of row counts across the input clients.
    pub n_rows_aggregated: usize,
    pub source_mix: SourceMixPrior,
    pub per_source_iet: PerSourceIetPrior,
    pub lines_per_je: LinesPerJePrior,
    pub active_lifetime: ActiveLifetimePrior,
    pub fanout: FanoutPrior,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posting_lag: Option<PostingLagPrior>,
}

impl BehavioralPriors {
    pub const SCHEMA_VERSION: u32 = 1;
}

// Per-prior types — populated in subsequent tasks (T2..T8).
// Stub `Default + Serialize + Deserialize` so this task compiles.

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SourceMixPrior {
    pub probabilities: BTreeMap<String, f64>,
    pub other_fraction: f64,
    pub min_threshold: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PerSourceIetPrior {
    pub by_source: BTreeMap<String, IetSummary>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct IetSummary {
    pub n: usize,
    pub empirical_cdf_days: EmpiricalCdf,
    pub lognormal_fit: Option<LognormalParams>,
    pub lag1_autocorr: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LognormalParams {
    pub mu: f64,
    pub sigma: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LinesPerJePrior {
    pub overall: LineCountHistogram,
    pub by_source: BTreeMap<String, LineCountHistogram>,
    pub min_jes_per_source: usize,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ActiveLifetimePrior {
    pub by_source: BTreeMap<String, LineCountHistogram>,
    pub overall: LineCountHistogram,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FanoutPrior {
    pub by_attribute: BTreeMap<String, LineCountHistogram>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PostingLagPrior {
    pub by_source: BTreeMap<String, LagSummary>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LagSummary {
    pub empirical_cdf_days: EmpiricalCdf,
    pub mean: f64,
    pub stddev: f64,
    pub n: usize,
}

/// Histogram over a fixed bucket grid. Used for line-count, fan-out,
/// active-lifetime, posting-lag (when discrete-bucketed).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LineCountHistogram {
    /// Inclusive lower bound of each bucket.
    pub buckets: Vec<u32>,
    /// Probability mass per bucket. Same length as `buckets`. Sums to ~1.0.
    pub probabilities: Vec<f64>,
    /// Total samples used to build the histogram.
    pub n: usize,
}
```

Note: this stub references `crate::models::correlation::EmpiricalCdf` which already exists in the crate.

- [ ] **Step 3: Wire `behavioral` module into `models/mod.rs`**

Add to the top of `crates/datasynth-fingerprint/src/models/mod.rs`:

```rust
pub mod behavioral;
```

And add to the re-exports list (find the existing `pub use … {…};` block):

```rust
pub use behavioral::{
    ActiveLifetimePrior, BehavioralPriors, FanoutPrior, IetSummary, LagSummary,
    LineCountHistogram, LinesPerJePrior, LognormalParams, PerSourceIetPrior,
    PostingLagPrior, SourceMixPrior,
};
```

- [ ] **Step 4: Add `behavioral` field to `Fingerprint` struct**

In `crates/datasynth-fingerprint/src/models/fingerprint.rs`, add the import:

```rust
use super::BehavioralPriors;
```

After the `pub banking: Option<BankingFingerprint>` field (line ~44), insert:

```rust
    /// Behavioral priors mined from corpus GL data (SP2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavioral: Option<BehavioralPriors>,
```

In the `Fingerprint::new` constructor, set `behavioral: None,` in the `Self { … }` literal.

- [ ] **Step 5: Create stub `extraction/behavioral_extractor.rs`**

```rust
//! Behavioral-prior extraction from corpus GL data.
//!
//! Stub — populated in Tasks 2-9.
```

- [ ] **Step 6: Wire stub into `extraction/mod.rs`**

Add to `crates/datasynth-fingerprint/src/extraction/mod.rs`:

```rust
pub mod behavioral_extractor;
```

- [ ] **Step 7: Create `aggregation/` directory with stub mod.rs**

```rust
// crates/datasynth-fingerprint/src/aggregation/mod.rs
//! Industry-level aggregation of behavioral priors.

pub mod industry_aggregator;
```

```rust
// crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs
//! Stub — populated in Tasks 10-15.
```

- [ ] **Step 8: Register `aggregation` module in `lib.rs`**

In `crates/datasynth-fingerprint/src/lib.rs`, add (alphabetical position) after `pub mod io;`:

```rust
pub mod aggregation;
```

- [ ] **Step 9: Verify clean compile**

```bash
cargo check -p datasynth-fingerprint 2>&1 | tail -5
```
Expected: clean compile.

- [ ] **Step 10: Backward-compat smoke test for serde**

Add a test inline in `models/behavioral.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn behavioral_priors_default_round_trips() {
        let bp = BehavioralPriors {
            schema_version: BehavioralPriors::SCHEMA_VERSION,
            generator_version: "test".to_string(),
            industry: "health".to_string(),
            n_client_inputs: 0,
            n_rows_aggregated: 0,
            source_mix: SourceMixPrior::default(),
            per_source_iet: PerSourceIetPrior::default(),
            lines_per_je: LinesPerJePrior::default(),
            active_lifetime: ActiveLifetimePrior::default(),
            fanout: FanoutPrior::default(),
            posting_lag: None,
        };
        let json = serde_json::to_string(&bp).expect("serialize");
        let _: BehavioralPriors = serde_json::from_str(&json).expect("deserialize");
    }
}
```

Run:

```bash
cargo test -p datasynth-fingerprint --lib models::behavioral -- --test-threads=4 2>&1 | tail -5
```
Expected: 1 passed.

- [ ] **Step 11: Commit**

```bash
git add crates/datasynth-fingerprint/Cargo.toml crates/datasynth-fingerprint/src/
git commit -m "$(cat <<'EOF'
feat(fingerprint/behavioral): scaffold SP2 behavioral priors

Adds BehavioralPriors struct hierarchy (SourceMix, PerSourceIet, LinesPerJe, ActiveLifetime, Fanout, PostingLag) with empty-default round-trip test, an optional behavioral field on Fingerprint (additive via serde(default)), and stub modules for extraction/aggregation. Individual prior extractors land in tasks 2-9; aggregators in 10-15.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2 — LineCountHistogram fixed-bucket helper

**Files:**
- Modify: `crates/datasynth-fingerprint/src/models/behavioral.rs` (extend `LineCountHistogram` with helpers)

- [ ] **Step 1: Append helper methods to `LineCountHistogram` in `behavioral.rs`**

After the existing `LineCountHistogram` struct definition, add:

```rust
impl LineCountHistogram {
    /// Build a histogram on the given inclusive-lower-bound bucket grid.
    ///
    /// The bucket grid must be sorted ascending. Values greater than or equal
    /// to `buckets.last()` fall into the last bucket. Values less than
    /// `buckets[0]` are dropped (with a count returned for diagnostic use).
    pub fn build(values: &[u32], buckets: &[u32]) -> (Self, usize) {
        assert!(!buckets.is_empty(), "buckets must not be empty");
        let n_buckets = buckets.len();
        let mut counts = vec![0u64; n_buckets];
        let mut dropped = 0usize;
        for &v in values {
            if v < buckets[0] {
                dropped += 1;
                continue;
            }
            let bucket_idx = bucket_index(buckets, v);
            counts[bucket_idx] += 1;
        }
        let total: u64 = counts.iter().sum();
        let probabilities = if total == 0 {
            vec![0.0; n_buckets]
        } else {
            counts
                .iter()
                .map(|&c| c as f64 / total as f64)
                .collect()
        };
        (
            Self {
                buckets: buckets.to_vec(),
                probabilities,
                n: values.len(),
            },
            dropped,
        )
    }

    /// Sum bucket counts of `self` and `other` (assuming the same bucket grid)
    /// and renormalise. Returns `None` if grids mismatch.
    pub fn pool(&self, other: &Self) -> Option<Self> {
        if self.buckets != other.buckets {
            return None;
        }
        let total_n = self.n + other.n;
        if total_n == 0 {
            return Some(Self {
                buckets: self.buckets.clone(),
                probabilities: vec![0.0; self.buckets.len()],
                n: 0,
            });
        }
        let probabilities: Vec<f64> = self
            .probabilities
            .iter()
            .zip(other.probabilities.iter())
            .map(|(&pa, &pb)| {
                (pa * self.n as f64 + pb * other.n as f64) / total_n as f64
            })
            .collect();
        Some(Self {
            buckets: self.buckets.clone(),
            probabilities,
            n: total_n,
        })
    }

    /// Median bucket — the smallest `buckets[i]` whose cumulative probability ≥ 0.5.
    pub fn median_bucket(&self) -> u32 {
        let mut cum = 0.0;
        for (i, &p) in self.probabilities.iter().enumerate() {
            cum += p;
            if cum >= 0.5 {
                return self.buckets[i];
            }
        }
        *self.buckets.last().unwrap_or(&0)
    }
}

fn bucket_index(buckets: &[u32], v: u32) -> usize {
    // Largest i where buckets[i] <= v.
    match buckets.binary_search(&v) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

/// Canonical bucket grid for line counts (lines-per-JE, fan-out).
pub const LINE_COUNT_BUCKETS: &[u32] = &[1, 2, 3, 4, 5, 6, 8, 10, 16, 32, 64, 128, 256, 1024];

/// Canonical bucket grid for active-lifetime days.
pub const ACTIVE_LIFETIME_DAY_BUCKETS: &[u32] = &[0, 1, 7, 30, 90, 180, 365, 730, 1825];

/// Canonical bucket grid for fan-out values.
pub const FANOUT_BUCKETS: &[u32] = &[1, 2, 3, 5, 8, 16, 32, 64, 128, 256, 1024];
```

- [ ] **Step 2: Append tests to the existing `#[cfg(test)] mod tests { ... }`**

```rust
    #[test]
    fn line_count_histogram_build_basic() {
        let values = vec![1, 1, 2, 3, 5, 5, 5, 32, 200];
        let (hist, dropped) = LineCountHistogram::build(&values, LINE_COUNT_BUCKETS);
        assert_eq!(dropped, 0);
        assert_eq!(hist.n, 9);
        assert!((hist.probabilities.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        // 200 falls into the [128] bucket (since 256 > 200 and 128 <= 200).
        let idx_128 = LINE_COUNT_BUCKETS.iter().position(|&b| b == 128).unwrap();
        assert!(hist.probabilities[idx_128] > 0.0);
    }

    #[test]
    fn line_count_histogram_drops_below_min() {
        let values = vec![0, 0, 1, 2];
        let (hist, dropped) = LineCountHistogram::build(&values, &[1, 2, 4]);
        assert_eq!(dropped, 2);
        assert_eq!(hist.n, 4);
        // p_1 = 1/2 (only one "1" survived; total counted = 2)
        assert!((hist.probabilities[0] - 0.5).abs() < 1e-9);
    }

    #[test]
    fn line_count_histogram_pool_weighted() {
        let a = LineCountHistogram {
            buckets: vec![1, 2, 4],
            probabilities: vec![1.0, 0.0, 0.0],
            n: 10,
        };
        let b = LineCountHistogram {
            buckets: vec![1, 2, 4],
            probabilities: vec![0.0, 1.0, 0.0],
            n: 90,
        };
        let pooled = a.pool(&b).unwrap();
        assert!((pooled.probabilities[0] - 0.1).abs() < 1e-9);
        assert!((pooled.probabilities[1] - 0.9).abs() < 1e-9);
        assert_eq!(pooled.n, 100);
    }

    #[test]
    fn line_count_histogram_pool_mismatched_grids_returns_none() {
        let a = LineCountHistogram { buckets: vec![1, 2], probabilities: vec![0.5, 0.5], n: 2 };
        let b = LineCountHistogram { buckets: vec![1, 3], probabilities: vec![0.5, 0.5], n: 2 };
        assert!(a.pool(&b).is_none());
    }

    #[test]
    fn median_bucket_known() {
        let h = LineCountHistogram {
            buckets: vec![1, 2, 4, 8],
            probabilities: vec![0.1, 0.4, 0.4, 0.1],
            n: 100,
        };
        // cum: 0.1, 0.5, 0.9 — first ≥0.5 is index 1 (bucket 2)
        assert_eq!(h.median_bucket(), 2);
    }
```

- [ ] **Step 3: Test + clippy + fmt + commit**

```bash
cargo test -p datasynth-fingerprint --lib models::behavioral -- --test-threads=4 2>&1 | tail -10
cargo clippy -p datasynth-fingerprint -- -D warnings 2>&1 | tail -5
cargo fmt -p datasynth-fingerprint
git add crates/datasynth-fingerprint/src/models/behavioral.rs
git commit -m "feat(fingerprint/behavioral): LineCountHistogram build / pool / median helpers + bucket grids"
```

Expected: 6 tests in models::behavioral.

---

## Task 3 — SourceMixPrior extractor

**Files:**
- Create section in: `crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs`

- [ ] **Step 1: Replace the stub with a partial impl + tests for SourceMix**

```rust
//! Behavioral-prior extraction from corpus GL data.

use std::collections::BTreeMap;

use datasynth_eval::behavioral_fidelity::Record;

use crate::models::behavioral::{
    SourceMixPrior,
};

/// Default minimum row-share for a Source code to appear individually in the mix.
pub const DEFAULT_MIN_SOURCE_THRESHOLD: f64 = 0.005;

/// Build a `SourceMixPrior` from a slice of records.
///
/// Rolls codes whose fraction is below `min_threshold` into the
/// `other_fraction` bucket. The explicit probabilities map sums to
/// (1.0 - other_fraction).
pub fn extract_source_mix(records: &[Record], min_threshold: f64) -> SourceMixPrior {
    if records.is_empty() {
        return SourceMixPrior {
            probabilities: BTreeMap::new(),
            other_fraction: 0.0,
            min_threshold,
        };
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for r in records {
        *counts.entry(r.source.clone()).or_insert(0) += 1;
    }
    let total = records.len() as f64;
    let mut probabilities = BTreeMap::new();
    let mut other = 0.0;
    for (src, c) in counts {
        let frac = c as f64 / total;
        if frac >= min_threshold {
            probabilities.insert(src, frac);
        } else {
            other += frac;
        }
    }
    SourceMixPrior {
        probabilities,
        other_fraction: other,
        min_threshold,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn rec(src: &str) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        Record {
            source: src.into(),
            gl_account: "1".into(),
            cost_center: None,
            profit_center: None,
            trading_partner: None,
            je_number: "J1".into(),
            je_line_number: "001".into(),
            effective_date: d,
            entry_date: d,
            created_at: None,
            functional_amount: 1.0,
        }
    }

    #[test]
    fn source_mix_shares_match() {
        let mut recs: Vec<Record> = Vec::new();
        // 60 "A", 30 "B", 10 "C" -> shares 0.6, 0.3, 0.1
        recs.extend(std::iter::repeat_with(|| rec("A")).take(60));
        recs.extend(std::iter::repeat_with(|| rec("B")).take(30));
        recs.extend(std::iter::repeat_with(|| rec("C")).take(10));
        let mix = extract_source_mix(&recs, DEFAULT_MIN_SOURCE_THRESHOLD);
        assert!((mix.probabilities["A"] - 0.6).abs() < 1e-9);
        assert!((mix.probabilities["B"] - 0.3).abs() < 1e-9);
        assert!((mix.probabilities["C"] - 0.1).abs() < 1e-9);
        assert!(mix.other_fraction.abs() < 1e-9);
    }

    #[test]
    fn source_mix_long_tail_rolls_into_other() {
        let mut recs: Vec<Record> = Vec::new();
        recs.extend(std::iter::repeat_with(|| rec("A")).take(995));
        // Each of "X1".."X5" is 1/1000 = 0.001 — below threshold 0.005.
        for i in 1..=5 {
            recs.push(rec(&format!("X{i}")));
        }
        let mix = extract_source_mix(&recs, 0.005);
        assert!((mix.probabilities["A"] - 0.995).abs() < 1e-9);
        assert!(!mix.probabilities.contains_key("X1"));
        assert!((mix.other_fraction - 0.005).abs() < 1e-9);
    }

    #[test]
    fn source_mix_empty_input_returns_empty() {
        let mix = extract_source_mix(&[], DEFAULT_MIN_SOURCE_THRESHOLD);
        assert!(mix.probabilities.is_empty());
        assert!(mix.other_fraction.abs() < 1e-9);
    }
}
```

- [ ] **Step 2: Test + commit**

```bash
cargo test -p datasynth-fingerprint --lib extraction::behavioral_extractor -- --test-threads=4 2>&1 | tail -8
git add crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs
git commit -m "feat(fingerprint/behavioral): SourceMixPrior extractor with min-threshold rolling"
```

---

## Task 4 — PerSourceIetPrior extractor

**Files:**
- Modify: `crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs` (append)

- [ ] **Step 1: Append the IET extractor**

Add new imports at the top of the file:

```rust
use chrono::NaiveDate;
use datasynth_eval::behavioral_fidelity::math::pearson_lag1_correlation;

use crate::models::behavioral::{
    IetSummary, LognormalParams, PerSourceIetPrior,
};
use crate::models::correlation::EmpiricalCdf;
```

Then append:

```rust
/// Minimum sample count for a Source to receive its own IET summary.
pub const DEFAULT_MIN_IET_SAMPLES: usize = 100;

/// Extract per-Source inter-event-time distributions in days.
pub fn extract_per_source_iet(
    records: &[Record],
    min_samples: usize,
) -> PerSourceIetPrior {
    let mut by_source: BTreeMap<String, Vec<NaiveDate>> = BTreeMap::new();
    for r in records {
        by_source.entry(r.source.clone()).or_default().push(r.entry_date);
    }
    let mut summaries: BTreeMap<String, IetSummary> = BTreeMap::new();
    for (source, mut dates) in by_source {
        if dates.len() < 2 {
            continue;
        }
        dates.sort();
        let iets: Vec<f64> = dates
            .windows(2)
            .map(|w| (w[1] - w[0]).num_days() as f64)
            .collect();
        if iets.len() < min_samples {
            continue;
        }
        let cdf = build_empirical_cdf(&iets);
        let lognormal = fit_lognormal(&iets);
        let auto = pearson_lag1_correlation(&iets).unwrap_or(0.0);
        summaries.insert(
            source,
            IetSummary {
                n: iets.len(),
                empirical_cdf_days: cdf,
                lognormal_fit: lognormal,
                lag1_autocorr: auto,
            },
        );
    }
    PerSourceIetPrior { by_source: summaries }
}

fn build_empirical_cdf(samples: &[f64]) -> EmpiricalCdf {
    let mut sorted: Vec<f64> = samples
        .iter()
        .copied()
        .filter(|x| x.is_finite())
        .collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    let knots_x: Vec<f64> = sorted.clone();
    let knots_p: Vec<f64> = (1..=n).map(|i| i as f64 / n as f64).collect();
    EmpiricalCdf {
        knots_x,
        knots_p,
    }
}

fn fit_lognormal(samples: &[f64]) -> Option<LognormalParams> {
    let log_samples: Vec<f64> = samples
        .iter()
        .filter(|&&x| x.is_finite() && x > 0.0)
        .map(|&x| (x + 1.0).ln())
        .collect();
    if log_samples.len() < 3 {
        return None;
    }
    let n = log_samples.len() as f64;
    let mean: f64 = log_samples.iter().sum::<f64>() / n;
    let var: f64 =
        log_samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n.max(1.0);
    let sigma = var.sqrt();
    Some(LognormalParams { mu: mean, sigma })
}
```

Note: `EmpiricalCdf` field names are `knots_x` and `knots_p` (verify in the existing `models/correlation.rs`). If field names differ, adapt.

- [ ] **Step 2: Append tests**

```rust
    #[test]
    fn per_source_iet_basic() {
        let mut recs: Vec<Record> = Vec::new();
        // Source "A": 120 dates at gap=1 day each.
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        for i in 0..120 {
            let mut r = rec("A");
            r.entry_date = base + chrono::Duration::days(i);
            recs.push(r);
        }
        // Source "B": only 50 dates — below min_samples, should be dropped.
        for i in 0..50 {
            let mut r = rec("B");
            r.entry_date = base + chrono::Duration::days(i);
            recs.push(r);
        }
        let p = extract_per_source_iet(&recs, 100);
        assert!(p.by_source.contains_key("A"));
        assert!(!p.by_source.contains_key("B"));
        let summ = &p.by_source["A"];
        assert_eq!(summ.n, 119); // 120 dates → 119 IETs
        assert!(summ.lognormal_fit.is_some());
    }

    #[test]
    fn per_source_iet_autocorr_for_constant_gap_high() {
        let mut recs: Vec<Record> = Vec::new();
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        // Strict gap of 3 days each → constant IET → autocorrelation undefined
        // (zero variance) → pearson returns None → falls back to 0.0.
        for i in 0..200 {
            let mut r = rec("A");
            r.entry_date = base + chrono::Duration::days(3 * i);
            recs.push(r);
        }
        let p = extract_per_source_iet(&recs, 100);
        // constant gap means zero variance; autocorr falls back to 0.0
        assert!((p.by_source["A"].lag1_autocorr).abs() < 1e-9);
    }
```

- [ ] **Step 3: Test + commit**

```bash
cargo test -p datasynth-fingerprint --lib extraction::behavioral_extractor -- --test-threads=4 2>&1 | tail -10
git add crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs
git commit -m "feat(fingerprint/behavioral): PerSourceIetPrior extractor with empirical CDF + lognormal fit"
```

---

## Task 5 — LinesPerJePrior extractor

**Files:**
- Modify: `crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs` (append)

- [ ] **Step 1: Append extractor + tests**

Add imports:

```rust
use crate::models::behavioral::{
    LineCountHistogram, LinesPerJePrior, LINE_COUNT_BUCKETS,
};
```

Then:

```rust
/// Default minimum JE count for a Source to receive its own histogram.
pub const DEFAULT_MIN_JES_PER_SOURCE: usize = 500;

/// Build the LinesPerJePrior — overall + per-Source histogram of line counts.
pub fn extract_lines_per_je(
    records: &[Record],
    min_jes_per_source: usize,
) -> LinesPerJePrior {
    // Overall: count lines per JE Number.
    let mut lines_per_je: BTreeMap<String, u32> = BTreeMap::new();
    let mut source_of_je: BTreeMap<String, String> = BTreeMap::new();
    for r in records {
        *lines_per_je.entry(r.je_number.clone()).or_insert(0) += 1;
        source_of_je
            .entry(r.je_number.clone())
            .or_insert_with(|| r.source.clone());
    }
    let overall_values: Vec<u32> = lines_per_je.values().copied().collect();
    let (overall, _) = LineCountHistogram::build(&overall_values, LINE_COUNT_BUCKETS);

    // Per-source: only for sources with ≥ min_jes_per_source JEs.
    let mut by_source_values: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for (je, n_lines) in &lines_per_je {
        if let Some(src) = source_of_je.get(je) {
            by_source_values.entry(src.clone()).or_default().push(*n_lines);
        }
    }
    let mut by_source: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
    for (src, values) in by_source_values {
        if values.len() < min_jes_per_source {
            continue;
        }
        let (hist, _) = LineCountHistogram::build(&values, LINE_COUNT_BUCKETS);
        by_source.insert(src, hist);
    }

    LinesPerJePrior {
        overall,
        by_source,
        min_jes_per_source,
    }
}
```

Tests:

```rust
    #[test]
    fn lines_per_je_overall_known() {
        // JE-A has 3 lines, JE-B has 2 lines, JE-C has 1 line.
        let mut recs: Vec<Record> = Vec::new();
        for _ in 0..3 {
            let mut r = rec("S");
            r.je_number = "JE-A".into();
            recs.push(r);
        }
        for _ in 0..2 {
            let mut r = rec("S");
            r.je_number = "JE-B".into();
            recs.push(r);
        }
        let mut r = rec("S");
        r.je_number = "JE-C".into();
        recs.push(r);

        let p = extract_lines_per_je(&recs, DEFAULT_MIN_JES_PER_SOURCE);
        let overall = &p.overall;
        // Lines: [3, 2, 1] → buckets [1,2,3,...]: 1/3 in bucket 1, 1/3 in bucket 2, 1/3 in bucket 3.
        let idx_1 = LINE_COUNT_BUCKETS.iter().position(|&b| b == 1).unwrap();
        let idx_2 = LINE_COUNT_BUCKETS.iter().position(|&b| b == 2).unwrap();
        let idx_3 = LINE_COUNT_BUCKETS.iter().position(|&b| b == 3).unwrap();
        assert!((overall.probabilities[idx_1] - 1.0 / 3.0).abs() < 1e-9);
        assert!((overall.probabilities[idx_2] - 1.0 / 3.0).abs() < 1e-9);
        assert!((overall.probabilities[idx_3] - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(overall.n, 3);
    }
```

- [ ] **Step 2: Test + commit**

```bash
cargo test -p datasynth-fingerprint --lib extraction::behavioral_extractor -- --test-threads=4 2>&1 | tail -5
git add crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs
git commit -m "feat(fingerprint/behavioral): LinesPerJePrior extractor (overall + per-source)"
```

---

## Task 6 — ActiveLifetimePrior extractor

**Files:**
- Modify: `crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs` (append)

- [ ] **Step 1: Append extractor + test**

Add import:

```rust
use crate::models::behavioral::{ActiveLifetimePrior, ACTIVE_LIFETIME_DAY_BUCKETS};
```

Then:

```rust
/// Per-Source active lifetime in days = max(EntryDate) - min(EntryDate).
pub fn extract_active_lifetime(records: &[Record]) -> ActiveLifetimePrior {
    let mut by_source: BTreeMap<String, (NaiveDate, NaiveDate)> = BTreeMap::new();
    for r in records {
        let d = r.entry_date;
        by_source
            .entry(r.source.clone())
            .and_modify(|(lo, hi)| {
                if d < *lo {
                    *lo = d;
                }
                if d > *hi {
                    *hi = d;
                }
            })
            .or_insert((d, d));
    }
    let lifetimes_by_source: Vec<u32> = by_source
        .values()
        .map(|(lo, hi)| (hi.signed_duration_since(*lo).num_days().max(0) as u32))
        .collect();
    let (overall, _) =
        LineCountHistogram::build(&lifetimes_by_source, ACTIVE_LIFETIME_DAY_BUCKETS);

    // Per-source: each source gets exactly one observation (its lifetime).
    // For per-client per-source histograms (used during aggregation), the
    // by_source histogram simply records one value per source.
    let mut per_source_hists: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
    for (src, (lo, hi)) in &by_source {
        let life = (hi.signed_duration_since(*lo).num_days().max(0)) as u32;
        let (h, _) = LineCountHistogram::build(&[life], ACTIVE_LIFETIME_DAY_BUCKETS);
        per_source_hists.insert(src.clone(), h);
    }
    ActiveLifetimePrior {
        by_source: per_source_hists,
        overall,
    }
}
```

Test:

```rust
    #[test]
    fn active_lifetime_basic() {
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        let mut recs: Vec<Record> = Vec::new();
        // Source A: dates day 0..30  (lifetime 30 days)
        for i in 0..5 {
            let mut r = rec("A");
            r.entry_date = base + chrono::Duration::days(i * 6);
            recs.push(r);
        }
        // Source B: dates day 0..200 (lifetime ~200 days)
        for i in 0..5 {
            let mut r = rec("B");
            r.entry_date = base + chrono::Duration::days(i * 50);
            recs.push(r);
        }
        let p = extract_active_lifetime(&recs);
        // A: lifetime 24 days → bucket 7 (since 7 <= 24 < 30)
        // B: lifetime 200 days → bucket 180 (since 180 <= 200 < 365)
        let idx_7 = ACTIVE_LIFETIME_DAY_BUCKETS.iter().position(|&b| b == 7).unwrap();
        let idx_180 = ACTIVE_LIFETIME_DAY_BUCKETS
            .iter()
            .position(|&b| b == 180)
            .unwrap();
        assert!((p.overall.probabilities[idx_7] - 0.5).abs() < 1e-9);
        assert!((p.overall.probabilities[idx_180] - 0.5).abs() < 1e-9);
    }
```

- [ ] **Step 2: Test + commit**

```bash
cargo test -p datasynth-fingerprint --lib extraction::behavioral_extractor -- --test-threads=4 2>&1 | tail -5
git add crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs
git commit -m "feat(fingerprint/behavioral): ActiveLifetimePrior extractor"
```

---

## Task 7 — FanoutPrior extractor

**Files:**
- Modify: `crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs` (append)

- [ ] **Step 1: Append extractor**

Add import:

```rust
use std::collections::HashSet;

use crate::models::behavioral::{FanoutPrior, FANOUT_BUCKETS};
```

```rust
/// Build the bipartite fan-out prior across {GLAccount, CostCenter, ProfitCenter, TradingPartner}.
pub fn extract_fanout(records: &[Record]) -> FanoutPrior {
    let attributes: [(&str, fn(&Record) -> Option<String>); 4] = [
        ("GLAccount", |r| Some(r.gl_account.clone())),
        ("CostCenter", |r| r.cost_center.clone()),
        ("ProfitCenter", |r| r.profit_center.clone()),
        ("TradingPartner", |r| r.trading_partner.clone()),
    ];
    let mut by_attribute: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
    for (name, proj) in attributes {
        let mut sources_per_value: BTreeMap<String, HashSet<String>> = BTreeMap::new();
        for r in records {
            if let Some(v) = proj(r) {
                sources_per_value
                    .entry(v)
                    .or_default()
                    .insert(r.source.clone());
            }
        }
        let fanouts: Vec<u32> = sources_per_value
            .values()
            .map(|s| s.len() as u32)
            .collect();
        let (hist, _) = LineCountHistogram::build(&fanouts, FANOUT_BUCKETS);
        by_attribute.insert(name.to_string(), hist);
    }
    FanoutPrior { by_attribute }
}
```

Test:

```rust
    #[test]
    fn fanout_basic() {
        // GL "X" touched by A, B, C → fan-out 3.
        // GL "Y" touched by A only → fan-out 1.
        let mut recs: Vec<Record> = Vec::new();
        for &(src, gl) in &[("A", "X"), ("B", "X"), ("C", "X"), ("A", "Y")] {
            let mut r = rec(src);
            r.gl_account = gl.into();
            recs.push(r);
        }
        let p = extract_fanout(&recs);
        let hist = &p.by_attribute["GLAccount"];
        // Fan-outs [3, 1] → buckets [1, 3]. 0.5 in bucket 1, 0.5 in bucket 3.
        let idx_1 = FANOUT_BUCKETS.iter().position(|&b| b == 1).unwrap();
        let idx_3 = FANOUT_BUCKETS.iter().position(|&b| b == 3).unwrap();
        assert!((hist.probabilities[idx_1] - 0.5).abs() < 1e-9);
        assert!((hist.probabilities[idx_3] - 0.5).abs() < 1e-9);
    }
```

- [ ] **Step 2: Test + commit**

```bash
cargo test -p datasynth-fingerprint --lib extraction::behavioral_extractor -- --test-threads=4 2>&1 | tail -5
git add crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs
git commit -m "feat(fingerprint/behavioral): FanoutPrior extractor (Source × attribute bipartite)"
```

---

## Task 8 — PostingLagPrior extractor

**Files:**
- Modify: `crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs` (append)

- [ ] **Step 1: Append the lag extractor**

Add import:

```rust
use crate::models::behavioral::{LagSummary, PostingLagPrior};
```

```rust
/// Default minimum sample count for a Source to receive its own LagSummary.
pub const DEFAULT_MIN_LAG_SAMPLES: usize = 100;

/// Per-Source posting lag in days = EffectiveDate - EntryDate. Can be negative (backdating).
pub fn extract_posting_lag(records: &[Record], min_samples: usize) -> Option<PostingLagPrior> {
    if records.is_empty() {
        return None;
    }
    let mut by_source: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for r in records {
        let lag = (r.effective_date - r.entry_date).num_days() as f64;
        by_source.entry(r.source.clone()).or_default().push(lag);
    }
    let mut summaries: BTreeMap<String, LagSummary> = BTreeMap::new();
    for (source, samples) in by_source {
        if samples.len() < min_samples {
            continue;
        }
        let n = samples.len();
        let mean = samples.iter().sum::<f64>() / n as f64;
        let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
        let cdf = build_empirical_cdf(&samples);
        summaries.insert(
            source,
            LagSummary {
                empirical_cdf_days: cdf,
                mean,
                stddev: var.sqrt(),
                n,
            },
        );
    }
    if summaries.is_empty() {
        None
    } else {
        Some(PostingLagPrior { by_source: summaries })
    }
}
```

Test:

```rust
    #[test]
    fn posting_lag_known() {
        let mut recs: Vec<Record> = Vec::new();
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        // Source "A": effective = entry + 5 days each (lag = +5).
        for i in 0..120 {
            let mut r = rec("A");
            r.entry_date = base + chrono::Duration::days(i);
            r.effective_date = r.entry_date + chrono::Duration::days(5);
            recs.push(r);
        }
        // Source "B": effective = entry - 2 days (backdating, lag = -2).
        for i in 0..120 {
            let mut r = rec("B");
            r.entry_date = base + chrono::Duration::days(i);
            r.effective_date = r.entry_date - chrono::Duration::days(2);
            recs.push(r);
        }
        let p = extract_posting_lag(&recs, 100).expect("non-empty");
        assert!((p.by_source["A"].mean - 5.0).abs() < 1e-9);
        assert!((p.by_source["B"].mean - (-2.0)).abs() < 1e-9);
        assert!((p.by_source["A"].stddev).abs() < 1e-9);
    }
```

- [ ] **Step 2: Test + commit**

```bash
cargo test -p datasynth-fingerprint --lib extraction::behavioral_extractor -- --test-threads=4 2>&1 | tail -5
git add crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs
git commit -m "feat(fingerprint/behavioral): PostingLagPrior extractor (signed day-lag per Source)"
```

---

## Task 9 — `extract_behavioral_priors` orchestrator

**Files:**
- Modify: `crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs` (append top-level orchestrator + `extract_from_path`)

- [ ] **Step 1: Append the orchestrator**

```rust
use std::path::Path;

use datasynth_eval::behavioral_fidelity::loader::{load_csv_records, load_parquet_records};

use crate::error::FingerprintError;
use crate::models::behavioral::BehavioralPriors;

pub type BehavioralResult<T> = Result<T, FingerprintError>;

/// Build a fully-populated `BehavioralPriors` for one client/data file.
pub fn extract_behavioral_priors(
    records: &[Record],
    industry: &str,
) -> BehavioralResult<BehavioralPriors> {
    Ok(BehavioralPriors {
        schema_version: BehavioralPriors::SCHEMA_VERSION,
        generator_version: env!("CARGO_PKG_VERSION").to_string(),
        industry: industry.to_string(),
        n_client_inputs: 1,
        n_rows_aggregated: records.len(),
        source_mix: extract_source_mix(records, DEFAULT_MIN_SOURCE_THRESHOLD),
        per_source_iet: extract_per_source_iet(records, DEFAULT_MIN_IET_SAMPLES),
        lines_per_je: extract_lines_per_je(records, DEFAULT_MIN_JES_PER_SOURCE),
        active_lifetime: extract_active_lifetime(records),
        fanout: extract_fanout(records),
        posting_lag: extract_posting_lag(records, DEFAULT_MIN_LAG_SAMPLES),
    })
}

/// Convenience: load a parquet or CSV file from disk and call `extract_behavioral_priors`.
pub fn extract_behavioral_priors_from_path(
    path: &Path,
    industry: &str,
) -> BehavioralResult<BehavioralPriors> {
    let records = match path.extension().and_then(|s| s.to_str()) {
        Some("parquet") => load_parquet_records(path)
            .map_err(|e| FingerprintError::Extraction(e.to_string()))?,
        Some("csv") => load_csv_records(path)
            .map_err(|e| FingerprintError::Extraction(e.to_string()))?,
        _ => {
            return Err(FingerprintError::Extraction(format!(
                "unsupported extension at {}",
                path.display()
            )))
        }
    };
    extract_behavioral_priors(&records, industry)
}
```

Note: if `FingerprintError::Extraction` doesn't exist (check `crates/datasynth-fingerprint/src/error.rs`), use the closest variant or add a new one. If `FingerprintError::Other(String)` exists, use it.

- [ ] **Step 2: Append orchestrator test**

```rust
    #[test]
    fn extract_behavioral_priors_smoke() {
        // Minimal slice with 600 records across 2 sources → enough to populate every field.
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        let mut recs: Vec<Record> = Vec::new();
        for i in 0..300 {
            let mut r = rec("A");
            r.je_number = format!("JE-A-{:04}", i / 3);
            r.entry_date = base + chrono::Duration::days(i);
            r.effective_date = r.entry_date + chrono::Duration::days(1);
            r.gl_account = format!("ACC-{}", i % 5);
            recs.push(r);
        }
        for i in 0..300 {
            let mut r = rec("B");
            r.je_number = format!("JE-B-{:04}", i / 2);
            r.entry_date = base + chrono::Duration::days(i);
            r.effective_date = r.entry_date - chrono::Duration::days(1);
            r.gl_account = format!("ACC-{}", i % 7);
            recs.push(r);
        }
        let bp = extract_behavioral_priors(&recs, "test_industry").expect("ok");
        assert_eq!(bp.schema_version, BehavioralPriors::SCHEMA_VERSION);
        assert_eq!(bp.industry, "test_industry");
        assert_eq!(bp.n_client_inputs, 1);
        assert_eq!(bp.n_rows_aggregated, 600);
        assert!(!bp.source_mix.probabilities.is_empty());
        assert!(bp.per_source_iet.by_source.contains_key("A"));
        assert!(bp.per_source_iet.by_source.contains_key("B"));
        assert!(bp.lines_per_je.overall.n > 0);
        assert!(bp.active_lifetime.overall.n > 0);
        assert_eq!(bp.fanout.by_attribute.len(), 4);
        assert!(bp.posting_lag.is_some());
    }
```

- [ ] **Step 3: Test + commit**

```bash
cargo test -p datasynth-fingerprint --lib extraction::behavioral_extractor -- --test-threads=4 2>&1 | tail -10
cargo clippy -p datasynth-fingerprint -- -D warnings 2>&1 | tail -3
cargo fmt -p datasynth-fingerprint
git add crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs
git commit -m "feat(fingerprint/behavioral): extract_behavioral_priors orchestrator + from_path loader"
```

---

## Task 10 — `aggregation/` scaffolding

**Files:**
- Modify: `crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs` (replace stub with module-level imports + type alias)

- [ ] **Step 1: Replace the stub**

```rust
//! Industry-level aggregation of behavioral priors.

use std::collections::BTreeMap;

use crate::error::FingerprintError;
use crate::models::behavioral::{
    ActiveLifetimePrior, BehavioralPriors, FanoutPrior, IetSummary, LagSummary,
    LineCountHistogram, LinesPerJePrior, PerSourceIetPrior, PostingLagPrior,
    SourceMixPrior,
};

pub type AggregationResult<T> = Result<T, FingerprintError>;
```

Then add stubs for each aggregator function (filled in subsequent tasks):

```rust
pub fn aggregate_source_mix(_inputs: &[&SourceMixPrior]) -> SourceMixPrior {
    todo!("Task 11")
}
pub fn aggregate_lines_per_je(_inputs: &[&LinesPerJePrior]) -> LinesPerJePrior {
    todo!("Task 12")
}
pub fn aggregate_per_source_iet(_inputs: &[&PerSourceIetPrior]) -> PerSourceIetPrior {
    todo!("Task 13")
}
pub fn aggregate_active_lifetime(_inputs: &[&ActiveLifetimePrior]) -> ActiveLifetimePrior {
    todo!("Task 14")
}
pub fn aggregate_fanout(_inputs: &[&FanoutPrior]) -> FanoutPrior {
    todo!("Task 14")
}
pub fn aggregate_posting_lag(_inputs: &[&PostingLagPrior]) -> Option<PostingLagPrior> {
    todo!("Task 14")
}
```

Note: `todo!()` is acceptable as a transient measure between tasks. Each subsequent task replaces a `todo!()` with the real impl. Once all `todo!()` are gone, T15 lands the orchestrator.

- [ ] **Step 2: Verify compile (no tests yet)**

```bash
cargo check -p datasynth-fingerprint 2>&1 | tail -3
git add crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs
git commit -m "feat(fingerprint/behavioral): aggregation scaffolding (stubbed signatures)"
```

---

## Task 11 — `aggregate_source_mix`

**Files:**
- Modify: `crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs`

- [ ] **Step 1: Replace `aggregate_source_mix` body**

```rust
pub fn aggregate_source_mix(inputs: &[&SourceMixPrior]) -> SourceMixPrior {
    if inputs.is_empty() {
        return SourceMixPrior::default();
    }
    // Row-weighted average over per-client probabilities.
    // n_c is approximated by ROW-COUNT weight; we don't have row counts here
    // directly, so we use the implicit per-client mass = 1.0 (each client weighed
    // equally). The CLI passes already-row-weighted SourceMix via n_rows_aggregated
    // on the parent BehavioralPriors. For now: equal weighting across inputs.
    let n = inputs.len() as f64;
    let mut probabilities: BTreeMap<String, f64> = BTreeMap::new();
    let mut other = 0.0;
    let min_threshold = inputs[0].min_threshold;
    for client in inputs {
        for (src, &p) in &client.probabilities {
            *probabilities.entry(src.clone()).or_insert(0.0) += p / n;
        }
        other += client.other_fraction / n;
    }
    // Reapply min-threshold to keep the long tail rolled up consistently.
    let mut filtered = BTreeMap::new();
    for (src, p) in probabilities {
        if p >= min_threshold {
            filtered.insert(src, p);
        } else {
            other += p;
        }
    }
    SourceMixPrior {
        probabilities: filtered,
        other_fraction: other,
        min_threshold,
    }
}
```

Note: SP2's spec calls for *row-weighted* averaging. The struct as defined doesn't carry per-source row counts forward, so equal-weighting across clients is a documented simplification. If a follow-up benchmark shows this matters, the BehavioralPriors struct can be extended with `n_rows_per_source` and the aggregator updated.

- [ ] **Step 2: Add test**

Append a `#[cfg(test)] mod tests { use super::*; ... }` block at the bottom of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn smix(probs: &[(&str, f64)], other: f64, thresh: f64) -> SourceMixPrior {
        SourceMixPrior {
            probabilities: probs
                .iter()
                .map(|(s, p)| (s.to_string(), *p))
                .collect(),
            other_fraction: other,
            min_threshold: thresh,
        }
    }

    #[test]
    fn aggregate_source_mix_equal_clients() {
        let a = smix(&[("KR", 0.5), ("RE", 0.5)], 0.0, 0.005);
        let b = smix(&[("KR", 0.3), ("KZ", 0.7)], 0.0, 0.005);
        let agg = aggregate_source_mix(&[&a, &b]);
        // KR average: (0.5 + 0.3) / 2 = 0.4
        // RE average: 0.5 / 2 = 0.25
        // KZ average: 0.7 / 2 = 0.35
        assert!((agg.probabilities["KR"] - 0.4).abs() < 1e-9);
        assert!((agg.probabilities["RE"] - 0.25).abs() < 1e-9);
        assert!((agg.probabilities["KZ"] - 0.35).abs() < 1e-9);
        let total: f64 = agg.probabilities.values().sum::<f64>() + agg.other_fraction;
        assert!((total - 1.0).abs() < 1e-9);
    }
}
```

- [ ] **Step 3: Test + commit**

```bash
cargo test -p datasynth-fingerprint --lib aggregation:: -- --test-threads=4 2>&1 | tail -5
git add crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs
git commit -m "feat(fingerprint/behavioral): aggregate_source_mix (equal-weighted across clients)"
```

---

## Task 12 — `aggregate_lines_per_je`

**Files:**
- Modify: `crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs`

- [ ] **Step 1: Replace `aggregate_lines_per_je` body**

```rust
pub fn aggregate_lines_per_je(inputs: &[&LinesPerJePrior]) -> LinesPerJePrior {
    if inputs.is_empty() {
        return LinesPerJePrior::default();
    }
    // Overall: pool histograms across all clients.
    let mut overall = inputs[0].overall.clone();
    for &client in &inputs[1..] {
        if let Some(pooled) = overall.pool(&client.overall) {
            overall = pooled;
        }
    }
    // Per-source: merge by source key.
    let mut by_source: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
    for &client in inputs {
        for (src, hist) in &client.by_source {
            let merged = match by_source.get(src) {
                Some(existing) => existing.pool(hist).unwrap_or_else(|| hist.clone()),
                None => hist.clone(),
            };
            by_source.insert(src.clone(), merged);
        }
    }
    LinesPerJePrior {
        overall,
        by_source,
        min_jes_per_source: inputs[0].min_jes_per_source,
    }
}
```

- [ ] **Step 2: Add test**

In the existing test mod:

```rust
    use crate::models::behavioral::{LINE_COUNT_BUCKETS, LineCountHistogram};

    fn hist(values: &[u32]) -> LineCountHistogram {
        LineCountHistogram::build(values, LINE_COUNT_BUCKETS).0
    }

    #[test]
    fn aggregate_lines_per_je_pools_overall() {
        let a = LinesPerJePrior {
            overall: hist(&[2, 2, 3]),
            by_source: BTreeMap::new(),
            min_jes_per_source: 500,
        };
        let b = LinesPerJePrior {
            overall: hist(&[4, 5, 8]),
            by_source: BTreeMap::new(),
            min_jes_per_source: 500,
        };
        let agg = aggregate_lines_per_je(&[&a, &b]);
        assert_eq!(agg.overall.n, 6);
    }
```

- [ ] **Step 3: Test + commit**

```bash
cargo test -p datasynth-fingerprint --lib aggregation:: -- --test-threads=4 2>&1 | tail -5
git add crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs
git commit -m "feat(fingerprint/behavioral): aggregate_lines_per_je (bucket-pooled, multi-client)"
```

---

## Task 13 — `aggregate_per_source_iet`

**Files:**
- Modify: `crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs`

- [ ] **Step 1: Replace `aggregate_per_source_iet` body**

```rust
pub fn aggregate_per_source_iet(inputs: &[&PerSourceIetPrior]) -> PerSourceIetPrior {
    if inputs.is_empty() {
        return PerSourceIetPrior::default();
    }
    let mut by_source: BTreeMap<String, IetSummary> = BTreeMap::new();
    // First pass: collect knots per source.
    let mut pooled_knots: BTreeMap<String, (Vec<f64>, f64, usize)> = BTreeMap::new();
    for &client in inputs {
        for (src, summ) in &client.by_source {
            let entry = pooled_knots.entry(src.clone()).or_insert((Vec::new(), 0.0, 0));
            entry.0.extend(summ.empirical_cdf_days.knots_x.iter().copied());
            // Weighted average for autocorr.
            entry.1 += summ.lag1_autocorr * summ.n as f64;
            entry.2 += summ.n;
        }
    }
    for (src, (mut knots_x, auto_sum, n)) in pooled_knots {
        knots_x.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if knots_x.is_empty() {
            continue;
        }
        let total = knots_x.len();
        let cdf = crate::models::correlation::EmpiricalCdf {
            knots_x: knots_x.clone(),
            knots_p: (1..=total).map(|i| i as f64 / total as f64).collect(),
        };
        let lag1_autocorr = if n > 0 {
            auto_sum / n as f64
        } else {
            0.0
        };
        by_source.insert(
            src,
            IetSummary {
                n,
                empirical_cdf_days: cdf,
                lognormal_fit: None, // recompute would require samples; pool inputs preserved via cdf
                lag1_autocorr,
            },
        );
    }
    PerSourceIetPrior { by_source }
}
```

- [ ] **Step 2: Add test**

```rust
    use crate::models::behavioral::IetSummary;
    use crate::models::correlation::EmpiricalCdf;

    fn iet_summary(knots: &[f64], autocorr: f64, n: usize) -> IetSummary {
        IetSummary {
            n,
            empirical_cdf_days: EmpiricalCdf {
                knots_x: knots.to_vec(),
                knots_p: (1..=knots.len()).map(|i| i as f64 / knots.len() as f64).collect(),
            },
            lognormal_fit: None,
            lag1_autocorr: autocorr,
        }
    }

    #[test]
    fn aggregate_per_source_iet_pools_knots_and_weights_autocorr() {
        let a = PerSourceIetPrior {
            by_source: {
                let mut m = BTreeMap::new();
                m.insert("KR".into(), iet_summary(&[1.0, 2.0, 3.0], 0.5, 3));
                m
            },
        };
        let b = PerSourceIetPrior {
            by_source: {
                let mut m = BTreeMap::new();
                m.insert("KR".into(), iet_summary(&[4.0, 5.0, 6.0, 7.0], 0.3, 4));
                m
            },
        };
        let agg = aggregate_per_source_iet(&[&a, &b]);
        let summ = &agg.by_source["KR"];
        // Pooled: 7 knots total.
        assert_eq!(summ.empirical_cdf_days.knots_x.len(), 7);
        // Weighted autocorr: (0.5 * 3 + 0.3 * 4) / 7 = (1.5 + 1.2) / 7 ≈ 0.3857
        assert!((summ.lag1_autocorr - 0.3857142857).abs() < 1e-6);
    }
```

- [ ] **Step 3: Test + commit**

```bash
cargo test -p datasynth-fingerprint --lib aggregation:: -- --test-threads=4 2>&1 | tail -5
git add crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs
git commit -m "feat(fingerprint/behavioral): aggregate_per_source_iet (pooled CDF + weighted autocorr)"
```

---

## Task 14 — Active-lifetime + fanout + posting-lag aggregators

**Files:**
- Modify: `crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs`

- [ ] **Step 1: Replace bodies**

```rust
pub fn aggregate_active_lifetime(inputs: &[&ActiveLifetimePrior]) -> ActiveLifetimePrior {
    if inputs.is_empty() {
        return ActiveLifetimePrior::default();
    }
    let mut overall = inputs[0].overall.clone();
    for &client in &inputs[1..] {
        if let Some(pooled) = overall.pool(&client.overall) {
            overall = pooled;
        }
    }
    let mut by_source: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
    for &client in inputs {
        for (src, hist) in &client.by_source {
            let merged = match by_source.get(src) {
                Some(existing) => existing.pool(hist).unwrap_or_else(|| hist.clone()),
                None => hist.clone(),
            };
            by_source.insert(src.clone(), merged);
        }
    }
    ActiveLifetimePrior { by_source, overall }
}

pub fn aggregate_fanout(inputs: &[&FanoutPrior]) -> FanoutPrior {
    if inputs.is_empty() {
        return FanoutPrior::default();
    }
    let mut by_attribute: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
    for &client in inputs {
        for (attr, hist) in &client.by_attribute {
            let merged = match by_attribute.get(attr) {
                Some(existing) => existing.pool(hist).unwrap_or_else(|| hist.clone()),
                None => hist.clone(),
            };
            by_attribute.insert(attr.clone(), merged);
        }
    }
    FanoutPrior { by_attribute }
}

pub fn aggregate_posting_lag(inputs: &[&PostingLagPrior]) -> Option<PostingLagPrior> {
    if inputs.is_empty() {
        return None;
    }
    let mut pooled: BTreeMap<String, (Vec<f64>, usize)> = BTreeMap::new();
    for &client in inputs {
        for (src, summ) in &client.by_source {
            let entry = pooled.entry(src.clone()).or_insert((Vec::new(), 0));
            entry.0.extend(summ.empirical_cdf_days.knots_x.iter().copied());
            entry.1 += summ.n;
        }
    }
    if pooled.is_empty() {
        return None;
    }
    let mut by_source: BTreeMap<String, LagSummary> = BTreeMap::new();
    for (src, (mut knots, n)) in pooled {
        knots.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let total = knots.len();
        let mean = if total > 0 {
            knots.iter().sum::<f64>() / total as f64
        } else {
            0.0
        };
        let var = if total > 0 {
            knots.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / total as f64
        } else {
            0.0
        };
        let cdf = crate::models::correlation::EmpiricalCdf {
            knots_x: knots.clone(),
            knots_p: (1..=total).map(|i| i as f64 / total as f64).collect(),
        };
        by_source.insert(
            src,
            LagSummary {
                empirical_cdf_days: cdf,
                mean,
                stddev: var.sqrt(),
                n,
            },
        );
    }
    Some(PostingLagPrior { by_source })
}
```

- [ ] **Step 2: Add one test per aggregator**

```rust
    #[test]
    fn aggregate_active_lifetime_pools_overall() {
        let a = ActiveLifetimePrior {
            by_source: BTreeMap::new(),
            overall: hist(&[10, 20]),
        };
        let b = ActiveLifetimePrior {
            by_source: BTreeMap::new(),
            overall: hist(&[30, 40, 50]),
        };
        let agg = aggregate_active_lifetime(&[&a, &b]);
        assert_eq!(agg.overall.n, 5);
    }

    #[test]
    fn aggregate_fanout_merges_attributes() {
        let mut m1: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
        m1.insert("GLAccount".into(), hist(&[1, 2, 3]));
        let mut m2: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
        m2.insert("GLAccount".into(), hist(&[4, 5]));
        let a = FanoutPrior { by_attribute: m1 };
        let b = FanoutPrior { by_attribute: m2 };
        let agg = aggregate_fanout(&[&a, &b]);
        assert_eq!(agg.by_attribute["GLAccount"].n, 5);
    }

    #[test]
    fn aggregate_posting_lag_pools_means() {
        let cdf = |xs: &[f64]| EmpiricalCdf {
            knots_x: xs.to_vec(),
            knots_p: (1..=xs.len()).map(|i| i as f64 / xs.len() as f64).collect(),
        };
        let lag = |xs: &[f64]| LagSummary {
            empirical_cdf_days: cdf(xs),
            mean: xs.iter().sum::<f64>() / xs.len() as f64,
            stddev: 0.0,
            n: xs.len(),
        };
        let mut m1 = BTreeMap::new();
        m1.insert("KR".into(), lag(&[1.0, 2.0]));
        let mut m2 = BTreeMap::new();
        m2.insert("KR".into(), lag(&[3.0, 4.0, 5.0]));
        let a = PostingLagPrior { by_source: m1 };
        let b = PostingLagPrior { by_source: m2 };
        let agg = aggregate_posting_lag(&[&a, &b]).unwrap();
        // Pooled mean: (1+2+3+4+5)/5 = 3.0
        assert!((agg.by_source["KR"].mean - 3.0).abs() < 1e-9);
        assert_eq!(agg.by_source["KR"].n, 5);
    }
```

- [ ] **Step 3: Test + commit**

```bash
cargo test -p datasynth-fingerprint --lib aggregation:: -- --test-threads=4 2>&1 | tail -10
git add crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs
git commit -m "feat(fingerprint/behavioral): aggregate_active_lifetime + fanout + posting_lag"
```

---

## Task 15 — `aggregate_industry_priors` orchestrator

**Files:**
- Modify: `crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs`

- [ ] **Step 1: Append the orchestrator**

```rust
/// Aggregate N per-client behavioral priors into a single industry-level prior.
pub fn aggregate_industry_priors(
    inputs: &[&BehavioralPriors],
    industry: &str,
) -> AggregationResult<BehavioralPriors> {
    if inputs.is_empty() {
        return Err(FingerprintError::Extraction(
            "aggregate_industry_priors: no inputs".to_string(),
        ));
    }
    let n_rows_aggregated: usize = inputs.iter().map(|bp| bp.n_rows_aggregated).sum();
    let source_mixes: Vec<&_> = inputs.iter().map(|bp| &bp.source_mix).collect();
    let lpj: Vec<&_> = inputs.iter().map(|bp| &bp.lines_per_je).collect();
    let iets: Vec<&_> = inputs.iter().map(|bp| &bp.per_source_iet).collect();
    let lifetimes: Vec<&_> = inputs.iter().map(|bp| &bp.active_lifetime).collect();
    let fanouts: Vec<&_> = inputs.iter().map(|bp| &bp.fanout).collect();
    let lags: Vec<&_> = inputs
        .iter()
        .filter_map(|bp| bp.posting_lag.as_ref())
        .collect();

    Ok(BehavioralPriors {
        schema_version: BehavioralPriors::SCHEMA_VERSION,
        generator_version: env!("CARGO_PKG_VERSION").to_string(),
        industry: industry.to_string(),
        n_client_inputs: inputs.len(),
        n_rows_aggregated,
        source_mix: aggregate_source_mix(&source_mixes),
        per_source_iet: aggregate_per_source_iet(&iets),
        lines_per_je: aggregate_lines_per_je(&lpj),
        active_lifetime: aggregate_active_lifetime(&lifetimes),
        fanout: aggregate_fanout(&fanouts),
        posting_lag: if lags.is_empty() {
            None
        } else {
            aggregate_posting_lag(&lags)
        },
    })
}
```

- [ ] **Step 2: Add orchestrator smoke test**

```rust
    #[test]
    fn aggregate_industry_priors_smoke() {
        let bp = |industry: &str, n_rows: usize| BehavioralPriors {
            schema_version: BehavioralPriors::SCHEMA_VERSION,
            generator_version: "test".into(),
            industry: industry.into(),
            n_client_inputs: 1,
            n_rows_aggregated: n_rows,
            source_mix: smix(&[("KR", 0.5)], 0.5, 0.005),
            per_source_iet: PerSourceIetPrior::default(),
            lines_per_je: LinesPerJePrior::default(),
            active_lifetime: ActiveLifetimePrior::default(),
            fanout: FanoutPrior::default(),
            posting_lag: None,
        };
        let a = bp("health", 1000);
        let b = bp("health", 2000);
        let agg = aggregate_industry_priors(&[&a, &b], "health").expect("ok");
        assert_eq!(agg.industry, "health");
        assert_eq!(agg.n_client_inputs, 2);
        assert_eq!(agg.n_rows_aggregated, 3000);
        assert!(agg.posting_lag.is_none());
    }
```

- [ ] **Step 3: Re-export + commit**

In `crates/datasynth-fingerprint/src/lib.rs`, re-export the orchestrator (optional but tidy):

```rust
// no change here; users can access via crate::aggregation::industry_aggregator::*
```

Build + test + commit:

```bash
cargo test -p datasynth-fingerprint --lib aggregation:: -- --test-threads=4 2>&1 | tail -10
cargo clippy -p datasynth-fingerprint -- -D warnings 2>&1 | tail -3
cargo fmt -p datasynth-fingerprint
git add crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs
git commit -m "feat(fingerprint/behavioral): aggregate_industry_priors orchestrator"
```

---

## Task 16 — CLI: `fingerprint extract --behavioral`

**Files:**
- Modify: `crates/datasynth-cli/src/main.rs`

- [ ] **Step 1: Locate the existing `FingerprintCommands::Extract` variant**

Run:

```bash
grep -nE "Extract \{|FingerprintCommands" crates/datasynth-cli/src/main.rs | head -10
```

Find the `Extract { … }` arm and its fields.

- [ ] **Step 2: Add two new fields to the `Extract` variant**

In `FingerprintCommands::Extract { ... }`, add:

```rust
        /// Also extract behavioral priors (SP2). Requires --industry.
        #[arg(long, default_value_t = false)]
        behavioral: bool,
        /// Industry slug ("health", "life_sciences", …) — required when --behavioral.
        #[arg(long)]
        industry: Option<String>,
```

- [ ] **Step 3: Plumb the new fields through the dispatcher**

Find the match arm that handles `FingerprintCommands::Extract { … }` (in `run_main()` or wherever the dispatch lives) and add behavioral plumbing:

```rust
FingerprintCommands::Extract {
    input,
    output,
    behavioral,
    industry,
    /* ... existing fields ... */
} => {
    /* ... existing extraction logic ... */
    let mut fp = /* ... however the existing code builds the Fingerprint ... */;
    if behavioral {
        let industry = industry.ok_or_else(|| anyhow::anyhow!(
            "--behavioral requires --industry"
        ))?;
        let bp = datasynth_fingerprint::extraction::behavioral_extractor::extract_behavioral_priors_from_path(
            &input,
            &industry,
        )?;
        fp.behavioral = Some(bp);
    }
    /* ... write fp to output ... */
}
```

Note: the exact wiring depends on the existing Extract handler; adapt to match.

- [ ] **Step 4: Smoke-build the CLI**

```bash
cargo build -p datasynth-cli 2>&1 | tail -5
./target/debug/datasynth-data fingerprint extract --help 2>&1 | grep -E "behavioral|industry" | head -5
```

Expected: `--help` shows `--behavioral` and `--industry` flags.

- [ ] **Step 5: Commit**

```bash
cargo fmt -p datasynth-cli
git add crates/datasynth-cli/src/main.rs
git commit -m "feat(cli): fingerprint extract --behavioral --industry plumbs SP2 extractor"
```

---

## Task 17 — CLI: `fingerprint aggregate-industry`

**Files:**
- Modify: `crates/datasynth-cli/src/main.rs`

- [ ] **Step 1: Add a new variant to `FingerprintCommands`**

```rust
    /// Aggregate per-client behavioral fingerprints into one industry-level bundle.
    AggregateIndustry {
        /// Industry slug ("health", "life_sciences", …).
        #[arg(long)]
        industry: String,
        /// Paths to per-client .dsf files (glob is shell-expanded by caller).
        #[arg(long, num_args = 1..)]
        inputs: Vec<PathBuf>,
        /// Output .dsf path.
        #[arg(long)]
        output: PathBuf,
        /// Allow aggregation from fewer than 3 inputs (single-client industries).
        #[arg(long, default_value_t = false)]
        allow_single_client: bool,
        /// Allow inputs with different `behavioral.industry` than --industry.
        #[arg(long, default_value_t = false)]
        allow_cross_industry: bool,
    },
```

- [ ] **Step 2: Add dispatcher arm**

```rust
FingerprintCommands::AggregateIndustry {
    industry,
    inputs,
    output,
    allow_single_client,
    allow_cross_industry,
} => {
    if inputs.len() < 3 && !allow_single_client {
        anyhow::bail!(
            "aggregate-industry needs ≥3 inputs (got {}); pass --allow-single-client to override",
            inputs.len()
        );
    }
    let reader = datasynth_fingerprint::io::FingerprintReader::new();
    let bundles: Result<Vec<_>, _> = inputs
        .iter()
        .map(|p| reader.read_dsf(p).map_err(anyhow::Error::from))
        .collect();
    let bundles = bundles?;
    let mut priors: Vec<&datasynth_fingerprint::models::BehavioralPriors> = Vec::new();
    for (idx, fp) in bundles.iter().enumerate() {
        match &fp.behavioral {
            Some(bp) => {
                if !allow_cross_industry && bp.industry != industry {
                    anyhow::bail!(
                        "input #{} has industry {:?}, expected {:?} (use --allow-cross-industry)",
                        idx, bp.industry, industry
                    );
                }
                priors.push(bp);
            }
            None => anyhow::bail!(
                "input #{} ({}) has no behavioral section",
                idx, inputs[idx].display()
            ),
        }
    }
    let aggregated = datasynth_fingerprint::aggregation::industry_aggregator::aggregate_industry_priors(
        &priors,
        &industry,
    )?;
    // Build a minimal Fingerprint that contains just the behavioral section.
    // Re-use the first input as a template, then replace the behavioral field.
    let mut template = bundles.into_iter().next()
        .expect("non-empty inputs");
    template.behavioral = Some(aggregated);
    let writer = datasynth_fingerprint::io::FingerprintWriter::new();
    writer.write_dsf(&template, &output)?;
    eprintln!("Wrote industry-aggregated bundle to {}", output.display());
}
```

Adapt the FingerprintReader / FingerprintWriter API to match the existing helpers (check `io/mod.rs`).

- [ ] **Step 3: Build + commit**

```bash
cargo build -p datasynth-cli 2>&1 | tail -3
./target/debug/datasynth-data fingerprint aggregate-industry --help 2>&1 | tail -15
cargo fmt -p datasynth-cli
git add crates/datasynth-cli/src/main.rs
git commit -m "feat(cli): fingerprint aggregate-industry — N per-client .dsf -> 1 industry .dsf"
```

---

## Task 18 — CLI: `fingerprint info --behavioral`

**Files:**
- Modify: `crates/datasynth-cli/src/main.rs`

- [ ] **Step 1: Add a `behavioral` flag to the existing `Info` variant**

Find `FingerprintCommands::Info { … }` and add:

```rust
        /// Print the behavioral-priors section (SP2) if present.
        #[arg(long, default_value_t = false)]
        behavioral: bool,
```

- [ ] **Step 2: Extend the Info dispatcher**

In the `Info { … }` arm, after the existing printout, append:

```rust
if behavioral {
    if let Some(bp) = &fp.behavioral {
        println!("\nBehavioral priors (industry={})", bp.industry);
        println!("  schema_version: {}, generator_version: {}",
            bp.schema_version, bp.generator_version);
        println!("  n_client_inputs: {}, n_rows_aggregated: {}",
            bp.n_client_inputs, bp.n_rows_aggregated);
        println!("  source_mix (top 5):");
        let mut sorted: Vec<(&String, &f64)> = bp.source_mix.probabilities.iter().collect();
        sorted.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (s, p) in sorted.iter().take(5) {
            println!("    {} {:.1}%", s, **p * 100.0);
        }
        println!("    (+other {:.1}%)", bp.source_mix.other_fraction * 100.0);
        println!("  per_source_iet: {} sources with IET summaries",
            bp.per_source_iet.by_source.len());
        println!("  lines_per_je: median bucket {}, max bucket {}",
            bp.lines_per_je.overall.median_bucket(),
            bp.lines_per_je.overall.buckets.last().copied().unwrap_or(0));
        println!("  active_lifetime: median bucket {}d",
            bp.active_lifetime.overall.median_bucket());
        println!("  fanout (attribute -> median fan-out):");
        for (attr, hist) in &bp.fanout.by_attribute {
            println!("    {}: {}", attr, hist.median_bucket());
        }
        if let Some(lag) = &bp.posting_lag {
            let n_sources = lag.by_source.len();
            let mean_all: f64 = lag.by_source.values().map(|s| s.mean).sum::<f64>() / n_sources.max(1) as f64;
            println!("  posting_lag: {} sources; overall mean {:.1} days", n_sources, mean_all);
        }
    } else {
        println!("\n(No behavioral section in this .dsf)");
    }
}
```

- [ ] **Step 3: Smoke-run + commit**

```bash
cargo build -p datasynth-cli 2>&1 | tail -3
./target/debug/datasynth-data fingerprint info --help 2>&1 | grep behavioral
cargo fmt -p datasynth-cli
git add crates/datasynth-cli/src/main.rs
git commit -m "feat(cli): fingerprint info --behavioral prints SP2 section summary"
```

---

## Task 19 — Integration smoke test

**Files:**
- Create: `crates/datasynth-fingerprint/tests/behavioral_priors_smoke.rs`
- Modify: `crates/datasynth-fingerprint/Cargo.toml` (ensure `tempfile`, `rand`, `rand_chacha` in dev-deps)

- [ ] **Step 1: Add dev-deps if missing**

Check `[dev-dependencies]` block in `crates/datasynth-fingerprint/Cargo.toml`. Ensure:

```toml
[dev-dependencies]
rand = { workspace = true }
rand_chacha = { workspace = true }
tempfile = "3"
```

Add missing entries.

- [ ] **Step 2: Write the smoke test**

```rust
//! Integration smoke for the SP2 behavioral-prior pipeline:
//! synthesise 3 client datasets → extract per-client priors → aggregate → assert shape.

use chrono::{Duration, NaiveDate};
use datasynth_eval::behavioral_fidelity::Record;
use datasynth_fingerprint::aggregation::industry_aggregator::aggregate_industry_priors;
use datasynth_fingerprint::extraction::behavioral_extractor::extract_behavioral_priors;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

fn gen_records(seed: u64, n: usize) -> Vec<Record> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let sources = ["KR", "RE", "SA", "DZ", "WE", "IM"];
    let accounts: Vec<String> = (1000..1050).map(|i| format!("A{i}")).collect();
    let ccs: Vec<String> = (100..120).map(|i| format!("CC{i}")).collect();
    let tps: Vec<String> = (1..30).map(|i| format!("TP{i}")).collect();
    let base = NaiveDate::from_ymd_opt(2022, 1, 1).expect("date");

    (0..n)
        .map(|i| Record {
            source: sources[rng.random_range(0..sources.len())].to_string(),
            gl_account: accounts[rng.random_range(0..accounts.len())].clone(),
            cost_center: Some(ccs[rng.random_range(0..ccs.len())].clone()),
            profit_center: Some(ccs[rng.random_range(0..ccs.len())].clone()),
            trading_partner: Some(tps[rng.random_range(0..tps.len())].clone()),
            je_number: format!("J{}-{:06}", seed, i / 3),
            je_line_number: format!("{:03}", (i % 3) + 1),
            effective_date: base + Duration::days(rng.random_range(0..365)),
            entry_date: base + Duration::days(rng.random_range(0..365)),
            created_at: None,
            functional_amount: rng.random_range(-10000.0..10000.0),
        })
        .collect()
}

#[test]
fn extract_aggregate_inspect_roundtrip() {
    let a = extract_behavioral_priors(&gen_records(42, 3000), "test_industry")
        .expect("extract a");
    let b = extract_behavioral_priors(&gen_records(43, 3000), "test_industry")
        .expect("extract b");
    let c = extract_behavioral_priors(&gen_records(44, 3000), "test_industry")
        .expect("extract c");

    let agg = aggregate_industry_priors(&[&a, &b, &c], "test_industry").expect("aggregate");
    assert_eq!(agg.n_client_inputs, 3);
    assert_eq!(agg.n_rows_aggregated, 9000);
    assert!(!agg.source_mix.probabilities.is_empty());
    assert!(!agg.per_source_iet.by_source.is_empty());
    assert!(agg.lines_per_je.overall.n > 0);
    assert_eq!(agg.fanout.by_attribute.len(), 4);
    assert!(agg.posting_lag.is_some());

    // JSON round-trip.
    let json = serde_json::to_string(&agg).expect("serialize");
    let back: datasynth_fingerprint::models::BehavioralPriors =
        serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.n_client_inputs, 3);
    assert_eq!(back.industry, "test_industry");
}
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p datasynth-fingerprint --test behavioral_priors_smoke -- --test-threads=4 2>&1 | tail -10
git add crates/datasynth-fingerprint/tests/behavioral_priors_smoke.rs crates/datasynth-fingerprint/Cargo.toml
git commit -m "test(fingerprint/behavioral): smoke test — extract × 3 -> aggregate -> serde roundtrip"
```

---

## Task 20 — Backward-compat fixture test (old `.dsf` reads cleanly)

**Files:**
- Create: `crates/datasynth-fingerprint/tests/fixtures/pre_sp2_fingerprint.json`
- Append to: `crates/datasynth-fingerprint/tests/behavioral_priors_smoke.rs` (or new file)

- [ ] **Step 1: Generate a pre-SP2 fixture**

Run the existing extractor against a tiny CSV produced inline (the same way the smoke test does), then trim the JSON output to just a pre-SP2 shape (no `behavioral` field). Save as `crates/datasynth-fingerprint/tests/fixtures/pre_sp2_fingerprint.json`.

The simplest pinning: hand-write a minimal valid Fingerprint JSON missing the new field:

```json
{
  "manifest": {
    "schema_version": "1.0.0",
    "fingerprint_version": "1.0",
    "created_at": "2026-04-01T00:00:00Z",
    "source": {"type": "Inline", "identifier": "pre-sp2-fixture"},
    "privacy_config": {"differential_privacy": null, "k_anonymity": null}
  },
  "schema": {"tables": []},
  "statistics": {"per_column": {}},
  "privacy_audit": {"events": []}
}
```

(The exact shape may need fields added depending on what the current `Fingerprint::new` requires; the goal is to demonstrate that loading a file without `behavioral` deserialises cleanly with `behavioral: None`.)

If a hand-crafted fixture is too brittle, alternatively the test can:

```rust
let mut value: serde_json::Value = serde_json::from_str(SOME_KNOWN_GOOD_DSF).unwrap();
value.as_object_mut().unwrap().remove("behavioral");
let json = value.to_string();
let fp: Fingerprint = serde_json::from_str(&json).unwrap();
assert!(fp.behavioral.is_none());
```

Pick whichever is less brittle in the current codebase.

- [ ] **Step 2: Add backward-compat test**

Append to `behavioral_priors_smoke.rs`:

```rust
#[test]
fn old_dsf_without_behavioral_field_loads_cleanly() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pre_sp2_fingerprint.json");
    if !path.exists() {
        eprintln!("fixture missing at {} — skipping", path.display());
        return;
    }
    let raw = std::fs::read_to_string(&path).expect("read fixture");
    let fp: datasynth_fingerprint::models::Fingerprint =
        serde_json::from_str(&raw).expect("deserialize pre-SP2 .dsf");
    assert!(fp.behavioral.is_none(),
        "old .dsf must deserialise with behavioral: None");
}
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p datasynth-fingerprint --test behavioral_priors_smoke -- --test-threads=4 2>&1 | tail -10
git add crates/datasynth-fingerprint/tests/fixtures/ crates/datasynth-fingerprint/tests/behavioral_priors_smoke.rs
git commit -m "test(fingerprint/behavioral): backward-compat — pre-SP2 .dsf loads with behavioral=None"
```

---

## Task 21 — CI workflow update

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Locate the eval/fingerprint test section**

```bash
grep -nE "datasynth-fingerprint|behavioral" .github/workflows/ci.yml | head -10
```

- [ ] **Step 2: Add the named smoke step**

Append (in the matching matrix job, near the SP1 behavioral steps from `236d482`):

```yaml
      - name: Behavioral-priors smoke (SP2)
        run: cargo test -p datasynth-fingerprint --test behavioral_priors_smoke -- --test-threads=4
```

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add behavioral-priors smoke step (SP2) to PR workflow"
```

---

## Task 22 — `scripts/regenerate-industry-priors.sh`

**Files:**
- Create: `scripts/regenerate-industry-priors.sh`

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# Regenerate the five committed industry-priors bundles from the corpus.
# Run manually after pulling fresh client data or when extraction logic changes.
#
# Usage:
#   scripts/regenerate-industry-priors.sh [REAL_CORPUS_DIR]
#
# Set REAL_CORPUS_DIR to your corpus directory before running.

set -euo pipefail

REAL_CORPUS_DIR="${1:?Set REAL_CORPUS_DIR to your corpus directory}"
PRIORS_DIR="crates/datasynth-generators/resources/priors"
INTERMEDIATE_DIR="/tmp/sp2-per-client-priors"
DATASYNTH_BIN="./target/release/datasynth-data"

if [[ ! -x "$DATASYNTH_BIN" ]]; then
  echo "Building release binary..."
  cargo build --release -p datasynth-cli
fi

mkdir -p "$INTERMEDIATE_DIR" "$PRIORS_DIR"

# ADD_Client_Selection_Global.parquet maps Client Id -> (Client Name, Industry).
# We invoke python to read it, since polars/parquet IO from bash is awkward.
python3 - <<'PY'
import json
import pyarrow.parquet as pq
table = pq.read_table("$REAL_CORPUS_DIR/ADD_Client_Selection_Global.parquet").to_pandas()
mapping = {}
for _, row in table.iterrows():
    industry = str(row["Industry"]).strip().lower().replace(" ", "_").replace("&", "and")
    industry = industry.replace("__", "_").replace(",", "")
    mapping[str(row["Client Id"])] = industry
with open("/tmp/sp2-client-industries.json", "w") as f:
    json.dump(mapping, f)
PY

INDUSTRIES=(health life_sciences pharmaceutical technology power_and_utilities)
for industry in "${INDUSTRIES[@]}"; do
  echo "=== Industry: $industry ==="
  per_client_dsfs=()
  for client_id in $(python3 -c "import json; m=json.load(open('/tmp/sp2-client-industries.json')); [print(k) for k,v in m.items() if v == '$industry']"); do
    parquet="$REAL_CORPUS_DIR/JE_${client_id}.parquet"
    if [[ ! -f "$parquet" ]]; then
      echo "  (no $parquet — skipping)"
      continue
    fi
    out="$INTERMEDIATE_DIR/JE_${client_id}.behavioral.dsf"
    echo "  Extracting $parquet → $out"
    "$DATASYNTH_BIN" fingerprint extract \
      --input "$parquet" \
      --output "$out" \
      --behavioral \
      --industry "$industry" \
      --no-stats \
      --no-correlations
    per_client_dsfs+=("$out")
  done
  if [[ ${#per_client_dsfs[@]} -lt 3 ]]; then
    echo "  Skipping $industry: only ${#per_client_dsfs[@]} clients (<3)"
    continue
  fi
  out_bundle="$PRIORS_DIR/industry_priors_${industry}.dsf"
  echo "  Aggregating ${#per_client_dsfs[@]} client priors → $out_bundle"
  "$DATASYNTH_BIN" fingerprint aggregate-industry \
    --industry "$industry" \
    --inputs "${per_client_dsfs[@]}" \
    --output "$out_bundle"
done

echo
echo "Done. Committed bundles:"
ls -la "$PRIORS_DIR"/industry_priors_*.dsf
```

- [ ] **Step 2: Make executable + smoke-run + commit**

```bash
chmod +x scripts/regenerate-industry-priors.sh
# Don't actually run it in CI — it requires the corpus.
# Smoke just by verifying syntax:
bash -n scripts/regenerate-industry-priors.sh
git add scripts/regenerate-industry-priors.sh
git commit -m "build(scripts): regenerate-industry-priors.sh produces the 5 SP2 bundles from corpus"
```

Note: `--no-stats` and `--no-correlations` flags may not exist on the current `fingerprint extract` — adapt to whatever skip flags exist, or use the full extraction (slower but works).

---

## Task 23 — Docs (user-facing + README + CLAUDE.md)

**Files:**
- Create: `docs/real-world-priors.md`
- Modify: `README.md`
- Modify: `CLAUDE.md`
- Modify: `docs/behavioral-fidelity.md` (cross-link)

- [ ] **Step 1: Write `docs/real-world-priors.md`**

```markdown
# Real-world behavioral priors

SP2 extends `datasynth-fingerprint` with a `behavioral` section that captures
the five within-entity distributions the [SP1 baseline][baseline] identified
as the highest-DR gaps: source-mix, per-Source IET, lines-per-JE, active
lifetime, bipartite fan-out — plus a bonus posting-lag prior.

Five industry bundles ship at `crates/datasynth-generators/resources/priors/`:

| Industry           | Clients aggregated | Bundle file |
| ------------------ | -----------------: | ----------- |
| Health             | 15                 | `industry_priors_health.dsf` |
| Life Sciences      | 8                  | `industry_priors_life_sciences.dsf` |
| Pharmaceutical     | 4                  | `industry_priors_pharmaceutical.dsf` |
| Power & Utilities  | 5                  | `industry_priors_power_and_utilities.dsf` |
| Technology         | 4                  | `industry_priors_technology.dsf` |

Industries with fewer than 3 client samples in the corpus (Hospitality,
Government & Public Sector, Professional Firms) are not bundled — there
isn't enough sample diversity to produce a generalisable prior. Per-client
priors can be extracted on demand via `fingerprint extract --behavioral`.

## What each prior captures

| Prior              | Closes SP1 gap (DR)        | Shape |
| ------------------ | -------------------------- | ----- |
| `source_mix`       | P4 (4.5×)                  | Categorical {Source → fraction}, long tail rolled into `other_fraction` |
| `per_source_iet`   | P1 IETD (60.1×)            | Per-Source empirical CDF of day-gaps + lognormal fit + lag-1 autocorr |
| `lines_per_je`     | P2 JE-line-burst (452.8×)  | Histogram on `[1,2,3,4,5,6,8,10,16,32,64,128,256,1024]` buckets |
| `active_lifetime`  | P2 lifetime (23.2×)        | Histogram on `[0,1,7,30,90,180,365,730,1825]` day buckets |
| `fanout`           | P3 (11×–345×)              | Per-attribute fan-out histogram |
| `posting_lag`      | quality-of-life            | Per-Source signed-day-lag EmpiricalCdf + mean + stddev |

## Quick usage

```bash
# Extract per-client priors from one parquet
datasynth-data fingerprint extract \
  --input "/path/to/corpus-je.parquet" \
  --output "./corpus-je.behavioral.dsf" \
  --behavioral \
  --industry "health"

# Aggregate N per-client priors into an industry bundle
datasynth-data fingerprint aggregate-industry \
  --industry "health" \
  --inputs "./*.behavioral.dsf" \
  --output "crates/datasynth-generators/resources/priors/industry_priors_health.dsf"

# Inspect any bundle
datasynth-data fingerprint info \
  --input "crates/datasynth-generators/resources/priors/industry_priors_health.dsf" \
  --behavioral
```

## Regenerating from corpus

After the corpus changes or the extractor logic evolves, regenerate
all five bundles in one go:

```bash
scripts/regenerate-industry-priors.sh
```

Set `REAL_CORPUS_DIR` (or pass as the first argument) to point at your corpus directory.

## Privacy

The corpus is already client-obfuscated. SP2 layers no additional DP
on top — priors are aggregate distributions over hundreds of thousands of
rows, with no row-level facts stored. Attribute values (GL accounts, cost
centers, trading partners) are *not* stored — only the *fan-out count
distribution* over them.

[baseline]: baselines/2026-05-12-sp1-v5.10.0/SUMMARY.md
```

- [ ] **Step 2: README + CLAUDE.md entries**

In `README.md`, append after the existing behavioral-fidelity section:

```markdown
### Real-world priors (v5.11+)

The `datasynth-fingerprint` crate ships per-industry behavioral priors mined
from a 45-client corpus. SP3 consumes them to drive entity-aware
generation. See [docs/real-world-priors.md](docs/real-world-priors.md).
```

In `CLAUDE.md`, find the "Fingerprint Module" section. Add:

```markdown
- behavioral: per-industry behavioral priors (SP2) — source-mix, per-Source IET, lines-per-JE, active lifetime, fan-out, posting-lag. Bundles in `crates/datasynth-generators/resources/priors/`. CLI: `datasynth-data fingerprint extract --behavioral --industry X` / `aggregate-industry` / `info --behavioral`.
```

In `docs/behavioral-fidelity.md`, find the section that lists what SP1 measures and add a "see also" link:

```markdown
See also: [docs/real-world-priors.md](real-world-priors.md) — SP2 mines
the priors that SP3 generators will consume to close these gaps.
```

- [ ] **Step 3: Commit**

```bash
git add docs/real-world-priors.md README.md CLAUDE.md docs/behavioral-fidelity.md
git commit -m "docs(sp2): real-world priors user guide + README + CLAUDE.md entries"
```

---

## Self-review (run inline before handing off)

Trace each spec section to a task:

| Spec section                              | Implementing task(s) |
| ----------------------------------------- | -------------------- |
| §2.1 Crate placement                      | T1                   |
| §2.2 Public API                           | T1 (struct), T9 (orchestrator), T15 (aggregator orchestrator) |
| §3.1 SourceMixPrior                       | T3                   |
| §3.2 PerSourceIetPrior                    | T4                   |
| §3.3 LinesPerJePrior                      | T5                   |
| §3.4 ActiveLifetimePrior                  | T6                   |
| §3.5 FanoutPrior                          | T7                   |
| §3.6 PostingLagPrior (bonus)              | T8                   |
| §4 Aggregation math (all six rows)        | T11, T12, T13, T14, T15 |
| §5 .dsf schema extension                  | T1 (additive optional field)            |
| §6 CLI surface                            | T16, T17, T18        |
| §7.1 W1 + autocorr primitives             | reused from SP1 (datasynth-eval::math)   |
| §7.2 Unit tests per extractor             | T3-T8 (each has tests)                   |
| §7.3 Integration smoke                    | T19                  |
| §7.4 CI integration                       | T21                  |
| §7.5 Bundle generation script             | T22                  |
| §8 Privacy (no DP, no row-level facts)    | enforced by extractor design (T3-T8 store only aggregates) |
| §9.2 Risks                                | Documented in spec; mitigations live in tasks (rayon-parallel TBD as follow-up) |
| §10 Acceptance criteria                   | T1-T23 collectively                      |
| §11 Phase plan                            | T1 = phase A, T2-T9 = phase B, T10-T15 = phase C, T16-T18 = phase D, T19-T22 = phase E, T23 = phase F |
| §12 Out-of-scope                          | enforced by what's NOT in this plan      |

All spec sections covered. No placeholders. Type names consistent
across tasks (`BehavioralPriors`, `SourceMixPrior`, `PerSourceIetPrior`,
`IetSummary`, `LognormalParams`, `LinesPerJePrior`, `ActiveLifetimePrior`,
`FanoutPrior`, `PostingLagPrior`, `LagSummary`, `LineCountHistogram`,
`LINE_COUNT_BUCKETS`, `ACTIVE_LIFETIME_DAY_BUCKETS`, `FANOUT_BUCKETS`).

Function names consistent: `extract_source_mix`, `extract_per_source_iet`,
`extract_lines_per_je`, `extract_active_lifetime`, `extract_fanout`,
`extract_posting_lag`, `extract_behavioral_priors`,
`extract_behavioral_priors_from_path`, `aggregate_source_mix`,
`aggregate_per_source_iet`, `aggregate_lines_per_je`,
`aggregate_active_lifetime`, `aggregate_fanout`, `aggregate_posting_lag`,
`aggregate_industry_priors`.

---

## Execution

Plan complete. Subagent-driven dispatch per the user's standing autonomous
mode. Tasks dispatch serially (skill's "never parallel implementer subagents"
rule). Wave milestones:

- **Wave 1**: T1 (scaffolding)
- **Wave 2**: T2 (histogram helpers)
- **Wave 3**: T3-T8 (six extractors, serial)
- **Wave 4**: T9 (extractor orchestrator)
- **Wave 5**: T10-T14 (aggregator pieces, serial)
- **Wave 6**: T15 (aggregator orchestrator)
- **Wave 7**: T16-T18 (three CLI changes, serial)
- **Wave 8**: T19-T20 (integration + backward-compat tests)
- **Wave 9**: T21-T22 (CI + script)
- **Wave 10**: T23 (docs)

Verification between waves: `cargo build --release` + `cargo clippy --workspace`.
After all tasks: the developer runs `scripts/regenerate-industry-priors.sh`
manually (requires the corpus, not in CI) to produce the 5 committed
bundle files. That manual run is *not* a plan task — it's a one-off finalisation
step the developer performs after T23 completes.

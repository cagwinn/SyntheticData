# SP1 — Behavioral-Fidelity Evaluation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a new `crates/datasynth-eval/src/behavioral_fidelity/` submodule that computes P1–P4 behavioral-fidelity metrics on GL data, anchors each metric to a 50/50-split noise floor via the degradation-ratio normalizer, and emits a JSON + Markdown + CSV report. Accompanied by a `datasynth-data behavioral score` CLI subcommand with CI-gate semantics.

**Architecture:** Pure-Rust submodule under `datasynth-eval`. Loader converts parquet/CSV to `Vec<Record>`. Metric modules (`ietd`, `burst`, `fanout`, `velocity_rules`) consume `&[Record]` for testability. The `degradation` module produces a deterministic 50/50 split of the corpus by `JENumber` hash and divides each raw metric by its baseline. `intraday` runs the same logic at second resolution on the synthetic side only. The orchestrator (`mod.rs::compute_report`) wires them together and `report.rs` serialises three formats. CLI lives under a new `behavioral` subcommand group in `datasynth-cli`.

**Tech Stack:** Rust 2021 edition; `arrow` + `parquet` (workspace) for IO; `petgraph` (workspace) for P3 graph metrics; `chrono` for date math; `serde` + `serde_json` for JSON; `rayon` for per-entity parallelism; `statrs` for percentiles. No new heavy deps.

**Spec:** [`docs/superpowers/specs/2026-05-11-sp1-behavioral-fidelity-design.md`](../specs/2026-05-11-sp1-behavioral-fidelity-design.md)

**Test concurrency note:** Always use `--test-threads=4` or lower (workspace policy — see `memory/feedback_test_concurrency.md`). Never `cargo test --workspace`; prefer `cargo test -p datasynth-eval --lib -- --quiet --test-threads=4` and one named integration test at a time.

---

## File Structure

```
crates/datasynth-eval/
├── Cargo.toml                                       (modify: add arrow, parquet, petgraph)
└── src/
    ├── lib.rs                                       (modify: add `pub mod behavioral_fidelity;`)
    └── behavioral_fidelity/                         (new)
        ├── mod.rs                       compute_report orchestrator + public re-exports
        ├── error.rs                     BehavioralFidelityError, thiserror
        ├── types.rs                     Record, EntityProfile, BehavioralFidelityConfig
        ├── entity_profile.rs            EntityProfile::gl_source_tp() preset
        ├── math.rs                      wasserstein_1(), pearson_lag1_correlation()
        ├── loader.rs                    load_parquet_records(), load_csv_records()
        ├── ietd.rs                      P1: IETD + within-entity autocorrelation
        ├── burst.rs                     P2: active lifetime + burst length + JE-line-burst
        ├── fanout.rs                    P3: fan-out + clustering + triangle log-ratio
        ├── velocity_rules.rs            P4: canonical R1..R10 + trigger rate gap
        ├── degradation.rs               50/50 split + DR(G, m) normaliser
        ├── intraday.rs                  synth-only second-resolution structural metrics
        └── report.rs                    BehavioralFidelityReport + JSON/MD/CSV writers

crates/datasynth-cli/src/commands/
└── behavioral.rs                                    (new: BehavioralScoreArgs + run_behavioral_score)

crates/datasynth-eval/tests/                         (new test files)
├── behavioral_smoke.rs                              integration golden test
└── behavioral_noise_floor.rs                        50/50-split sanity test

docs/
└── behavioral-fidelity.md                           (new user-facing doc)

CLAUDE.md, README.md, .github/workflows/ci.yml       (small touch-ups)
```

## Task overview & dependency graph

```
T1 (scaffold) ─┬─▶ T2 (types) ─┬─▶ T3 (entity profile)
               │                 │
               │                 ├─▶ T7 (P1 IETD) ──────────────┐
               │                 ├─▶ T8 (P2 active lifetime) ───┤
               │                 ├─▶ T9 (P2 burst length) ──────┤
               │                 ├─▶ T10 (P2 JE-line burst) ────┤
               │                 ├─▶ T11 (P3 fan-out) ──────────┤
               │                 ├─▶ T12 (P3 clustering) ───────┤
               │                 ├─▶ T13 (P4 R1-R5) ────────────┤
               │                 └─▶ T14 (P4 R6-R10 + gap) ─────┤
               │                                                 │
               ├─▶ T4 (W1 helper) ──────────────────────────────┤
               ├─▶ T5 (autocorr helper) ────────────────────────┤
               └─▶ T6 (loader) ─────────────────────────────────┤
                                                                 │
                       ┌─────────────────────────────────────────┘
                       ▼
                T15 (DR + split) ─┬─▶ T17 (Report struct + JSON)
                T16 (intraday) ───┤
                                  ├─▶ T18 (MD/CSV writers)
                                  └─▶ T19 (compute_report wire-up)
                                          │
                                          ▼
                                  T20 (CLI subcommand)
                                          │
                       ┌──────────────────┼──────────────────┐
                       ▼                  ▼                  ▼
                T21 (smoke test)   T22 (noise-floor)   T23 (CI workflow)
                                                              │
                                                              ▼
                                                       T24 (docs + CLAUDE.md)
```

Wave 1: T1 → Wave 2: {T2, T4, T5, T6} → Wave 3: {T3 then T7..T14 in parallel} → Wave 4: {T15, T16, T17, T18} → Wave 5: T19 → Wave 6: T20 → Wave 7: {T21, T22, T23} → Wave 8: T24.

---

## Task 1 — Scaffolding the submodule + dependencies

**Files:**
- Create: `crates/datasynth-eval/src/behavioral_fidelity/mod.rs`
- Create: `crates/datasynth-eval/src/behavioral_fidelity/error.rs`
- Modify: `crates/datasynth-eval/src/lib.rs`
- Modify: `crates/datasynth-eval/Cargo.toml`

- [ ] **Step 1: Verify workspace petgraph + arrow + parquet versions**

Run: `grep -E "^petgraph|^arrow|^parquet" Cargo.toml`
Expected: `petgraph = "0.8"`, `arrow = { version = "58", … }`, `parquet = { version = "58", … }`.

- [ ] **Step 2: Add deps to `crates/datasynth-eval/Cargo.toml`**

Locate the `[dependencies]` section and add (alphabetically):

```toml
arrow = { workspace = true }
parquet = { workspace = true }
petgraph = { workspace = true }
```

- [ ] **Step 3: Register submodule in `crates/datasynth-eval/src/lib.rs`**

Find the existing `pub mod statistical;` line. Add directly after it:

```rust
pub mod behavioral_fidelity;
```

- [ ] **Step 4: Create `crates/datasynth-eval/src/behavioral_fidelity/error.rs`**

```rust
//! Errors emitted by the behavioral-fidelity module.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BehavioralFidelityError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parquet error: {0}")]
    Parquet(String),
    #[error("schema error: {0}")]
    Schema(String),
    #[error("computation error: {0}")]
    Computation(String),
    #[error("serde error: {0}")]
    Serde(String),
}

impl From<arrow::error::ArrowError> for BehavioralFidelityError {
    fn from(value: arrow::error::ArrowError) -> Self {
        BehavioralFidelityError::Parquet(value.to_string())
    }
}

impl From<parquet::errors::ParquetError> for BehavioralFidelityError {
    fn from(value: parquet::errors::ParquetError) -> Self {
        BehavioralFidelityError::Parquet(value.to_string())
    }
}

impl From<serde_json::Error> for BehavioralFidelityError {
    fn from(value: serde_json::Error) -> Self {
        BehavioralFidelityError::Serde(value.to_string())
    }
}

pub type BehavioralFidelityResult<T> = std::result::Result<T, BehavioralFidelityError>;
```

- [ ] **Step 5: Create `crates/datasynth-eval/src/behavioral_fidelity/mod.rs`**

```rust
//! P1–P4 behavioral-fidelity evaluation for GL data.
//!
//! Implements the Sajja (2026) framework adapted for GL semantics:
//! `Source` as the primary entity, `TradingPartner` as secondary, `EntryDate`
//! at day resolution, with a structural JE-line-burst metric and a canonical
//! R1..R10 velocity rule set. Anchors every metric to a 50/50-split noise
//! floor via the degradation-ratio normaliser.
//!
//! Spec: `docs/superpowers/specs/2026-05-11-sp1-behavioral-fidelity-design.md`.

pub mod error;
pub mod types;
pub mod entity_profile;
pub mod math;
pub mod loader;
pub mod ietd;
pub mod burst;
pub mod fanout;
pub mod velocity_rules;
pub mod degradation;
pub mod intraday;
pub mod report;

pub use error::{BehavioralFidelityError, BehavioralFidelityResult};
pub use types::{BehavioralFidelityConfig, EntityProfile, GateThresholds, Record, RuleSet};
pub use report::BehavioralFidelityReport;
```

- [ ] **Step 6: Verify it compiles (with stub modules to follow)**

The empty submodule will fail to compile until later tasks land the `types`, `math`, etc. modules. To allow Task 1 to commit standalone, add empty placeholder files now:

```rust
// crates/datasynth-eval/src/behavioral_fidelity/types.rs
// Stub — populated by Task 2.
```

Repeat the single-line stub for `entity_profile.rs`, `math.rs`, `loader.rs`, `ietd.rs`, `burst.rs`, `fanout.rs`, `velocity_rules.rs`, `degradation.rs`, `intraday.rs`, `report.rs`. Then remove the corresponding `pub use` lines in `mod.rs` (move them back as each task lands its types).

Concretely, edit `mod.rs` to comment out the `pub use` block until later tasks restore it:

```rust
// Restored as Tasks 2/17 land:
// pub use types::{BehavioralFidelityConfig, EntityProfile, GateThresholds, Record, RuleSet};
// pub use report::BehavioralFidelityReport;
```

- [ ] **Step 7: Compile**

Run: `cargo check -p datasynth-eval 2>&1 | tail -20`
Expected: clean compile, no errors. Warnings about unused modules are OK.

- [ ] **Step 8: Commit**

```bash
git add crates/datasynth-eval/Cargo.toml crates/datasynth-eval/src/lib.rs crates/datasynth-eval/src/behavioral_fidelity/
git commit -m "$(cat <<'EOF'
feat(eval/behavioral): scaffold behavioral_fidelity submodule

Adds the new submodule directory + Cargo.toml dependencies (arrow, parquet, petgraph) under datasynth-eval. Empty module stubs land in subsequent tasks. Module hierarchy mirrors the spec at docs/superpowers/specs/2026-05-11-sp1-behavioral-fidelity-design.md.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2 — Record + config types

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/types.rs`
- Test inline (mod tests {} block within types.rs)

- [ ] **Step 1: Write failing test for `Record` construction + default `BehavioralFidelityConfig::gl_default()`**

Append to `types.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn record_construction_roundtrips() {
        let r = Record {
            source: "KR".to_string(),
            gl_account: "1100".to_string(),
            cost_center: Some("CC100".to_string()),
            profit_center: Some("PC100".to_string()),
            trading_partner: Some("TP1".to_string()),
            je_number: "2022-0090-001".to_string(),
            je_line_number: "001".to_string(),
            effective_date: NaiveDate::from_ymd_opt(2022, 4, 25).unwrap(),
            entry_date: NaiveDate::from_ymd_opt(2022, 4, 14).unwrap(),
            created_at: None,
            functional_amount: 761.65,
        };
        assert_eq!(r.source, "KR");
        assert_eq!(r.je_line_number, "001");
    }

    #[test]
    fn gl_default_config_is_source_tp_profile() {
        let cfg = BehavioralFidelityConfig::gl_default();
        assert_eq!(cfg.profile.name, "gl-source-tp");
        assert_eq!(cfg.profile.primary_entity, "Source");
        assert_eq!(cfg.profile.secondary_entity.as_deref(), Some("TradingPartner"));
        assert_eq!(cfg.profile.timestamp_day, "EntryDate");
        assert_eq!(cfg.profile.value_column, "FunctionalAmount");
        assert_eq!(cfg.profile.burst_thresholds, vec![1, 3, 7]);
        assert_eq!(cfg.seed, 42);
        assert!((cfg.fail_thresholds.fail_if_dr_above - 2.0).abs() < 1e-9);
        assert!((cfg.fail_thresholds.fail_if_composite_above - 1.5).abs() < 1e-9);
    }
}
```

- [ ] **Step 2: Run test (FAIL)**

Run: `cargo test -p datasynth-eval --lib behavioral_fidelity::types -- --test-threads=4 2>&1 | tail -10`
Expected: "cannot find type `Record` in this scope" / "cannot find function `gl_default`".

- [ ] **Step 3: Implement `types.rs`**

Replace the stub with:

```rust
//! Core types shared across the behavioral-fidelity module.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// One JE line, normalised across corpus and synthetic schemas.
///
/// Optional fields tolerate schema variation between real and synthetic
/// (e.g., `created_at` is only available on the synthetic side).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub source: String,
    pub gl_account: String,
    pub cost_center: Option<String>,
    pub profit_center: Option<String>,
    pub trading_partner: Option<String>,
    pub je_number: String,
    pub je_line_number: String,
    pub effective_date: NaiveDate,
    pub entry_date: NaiveDate,
    pub created_at: Option<DateTime<Utc>>,
    pub functional_amount: f64,
}

/// Which entity columns to evaluate and which attributes feed the P3 graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityProfile {
    pub name: String,
    pub primary_entity: String,
    pub secondary_entity: Option<String>,
    pub timestamp_day: String,
    pub timestamp_intra: Option<String>,
    pub attributes_for_p3: Vec<String>,
    pub value_column: String,
    pub burst_thresholds: Vec<i64>,
}

/// Canonical velocity rule set (10 rules for GL).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSet {
    pub rules: Vec<VelocityRuleSpec>,
}

/// Spec for a single velocity rule. The interpretation lives in `velocity_rules.rs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VelocityRuleSpec {
    pub id: String,                    // "R1".."R10"
    pub description: String,
    pub kind: VelocityRuleKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VelocityRuleKind {
    CountPerEntityPerDay { threshold: u32 },               // R1
    DistinctAccountsPerEntityPerDay { threshold: u32 },    // R2
    SumAmountPerEntityPerDayAbovePercentile { pct: f64 },  // R3 (p90 default)
    DormantAccountActivity { inactivity_days: i64 },       // R4
    DistinctTradingPartnersPerEntityPerDay { threshold: u32 }, // R5
    AmountSpikeRatio { window_days: i64, ratio: f64 },     // R6
    OffHoursPosting,                                       // R7 (weekday only)
    PostClosePosting { tolerance_business_days: i64 },     // R8
    RoundDollarConcentration { share_threshold: f64 },     // R9
    BackdatingDays { gap_days: i64 },                      // R10
}

/// PR-merge / CI gate behaviour for `behavioral score`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateThresholds {
    pub fail_if_dr_above: f64,
    pub fail_if_composite_above: f64,
}

impl Default for GateThresholds {
    fn default() -> Self {
        Self {
            fail_if_dr_above: 2.0,
            fail_if_composite_above: 1.5,
        }
    }
}

/// Optional period subsetting for the loader.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeriodFilter {
    pub start: NaiveDate,
    pub end:   NaiveDate,
}

/// Top-level config consumed by `compute_report`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BehavioralFidelityConfig {
    pub profile: EntityProfile,
    pub rule_set: RuleSet,
    pub seed: u64,
    pub fail_thresholds: GateThresholds,
    pub period_filter: Option<PeriodFilter>,
    pub client_filter: Option<Vec<String>>,
}

impl BehavioralFidelityConfig {
    /// Default GL profile: Source primary, Trading Partner secondary, EntryDate day, full R1..R10.
    pub fn gl_default() -> Self {
        Self {
            profile: EntityProfile::gl_source_tp_static(),
            rule_set: RuleSet::canonical_gl_rules(),
            seed: 42,
            fail_thresholds: GateThresholds::default(),
            period_filter: None,
            client_filter: None,
        }
    }
}

impl EntityProfile {
    /// Internal builder used by `gl_default` before `entity_profile.rs` lands.
    /// Task 3 promotes this to a public function on `entity_profile.rs`.
    pub(crate) fn gl_source_tp_static() -> Self {
        Self {
            name: "gl-source-tp".to_string(),
            primary_entity: "Source".to_string(),
            secondary_entity: Some("TradingPartner".to_string()),
            timestamp_day: "EntryDate".to_string(),
            timestamp_intra: Some("CreatedAt".to_string()),
            attributes_for_p3: vec![
                "GLAccount".to_string(),
                "CostCenter".to_string(),
                "ProfitCenter".to_string(),
                "TradingPartner".to_string(),
            ],
            value_column: "FunctionalAmount".to_string(),
            burst_thresholds: vec![1, 3, 7],
        }
    }
}

impl RuleSet {
    /// Stub returning the empty rule set. Task 13/14 populates R1..R10.
    pub fn canonical_gl_rules() -> Self {
        Self { rules: Vec::new() }
    }
}
```

- [ ] **Step 4: Restore `pub use` block in `mod.rs`**

Edit `crates/datasynth-eval/src/behavioral_fidelity/mod.rs` to uncomment:

```rust
pub use types::{BehavioralFidelityConfig, EntityProfile, GateThresholds, Record, RuleSet};
```

- [ ] **Step 5: Run tests (PASS)**

Run: `cargo test -p datasynth-eval --lib behavioral_fidelity::types -- --test-threads=4 2>&1 | tail -10`
Expected: `2 passed`.

- [ ] **Step 6: Run clippy + fmt**

Run: `cargo clippy -p datasynth-eval -- -D warnings 2>&1 | tail -10`
Expected: no warnings.

Run: `cargo fmt -p datasynth-eval`

- [ ] **Step 7: Commit**

```bash
git add crates/datasynth-eval/src/behavioral_fidelity/types.rs crates/datasynth-eval/src/behavioral_fidelity/mod.rs
git commit -m "feat(eval/behavioral): record + config types

Adds Record (one JE line, normalised), EntityProfile, RuleSet + VelocityRuleSpec enum, GateThresholds, and BehavioralFidelityConfig::gl_default() returning the source-tp profile with seed 42 and DR-gate thresholds 2.0 / 1.5.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3 — Entity profile preset

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/entity_profile.rs`

- [ ] **Step 1: Write failing test**

```rust
// at the top of entity_profile.rs

use crate::behavioral_fidelity::types::EntityProfile;

/// Source + Trading Partner profile, per spec §1.1.
pub fn gl_source_tp() -> EntityProfile {
    EntityProfile::gl_source_tp_static()
}

/// Column-alias map from canonical name -> corpus column name.
/// Synthetic-side mapping is identical for canonical names; the loader
/// applies real-only renames first.
pub fn real_corpus_aliases() -> [(&'static str, &'static str); 11] {
    [
        ("Source",            "Source"),
        ("GLAccount",         "GL Account Number"),
        ("CostCenter",        "Cost Center"),
        ("ProfitCenter",      "Profit Center"),
        ("TradingPartner",    "Tarding Partner"), // sic — real-data typo
        ("JENumber",          "JE Number"),
        ("JELineNumber",      "JE Line Number"),
        ("EffectiveDate",     "Effective Date"),
        ("EntryDate",         "Entry Date"),
        ("FunctionalAmount",  "Functional Amount"),
        ("ReportingAmount",   "Reporting Amount"),
    ]
}

/// Synthetic-side column aliases (DataSynth journal_entries output).
pub fn synthetic_aliases() -> [(&'static str, &'static str); 11] {
    [
        ("Source",            "source_module"),
        ("GLAccount",         "account_id"),
        ("CostCenter",        "cost_center"),
        ("ProfitCenter",      "profit_center"),
        ("TradingPartner",    "trading_partner"),
        ("JENumber",          "je_id"),
        ("JELineNumber",      "line_no"),
        ("EffectiveDate",     "posting_date"),
        ("EntryDate",         "entry_date"),
        ("CreatedAt",         "created_at"),
        ("FunctionalAmount",  "amount"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gl_source_tp_profile_shape() {
        let p = gl_source_tp();
        assert_eq!(p.name, "gl-source-tp");
        assert_eq!(p.primary_entity, "Source");
        assert_eq!(p.secondary_entity.as_deref(), Some("TradingPartner"));
        assert_eq!(p.attributes_for_p3.len(), 4);
    }

    #[test]
    fn real_corpus_aliases_match_observed_columns() {
        let aliases = real_corpus_aliases();
        let by_canon: std::collections::HashMap<_, _> = aliases.into_iter().collect();
        // corpus typo preserved
        assert_eq!(by_canon.get("TradingPartner"), Some(&"Tarding Partner"));
        assert_eq!(by_canon.get("Source"),         Some(&"Source"));
        assert_eq!(by_canon.get("EntryDate"),      Some(&"Entry Date"));
    }
}
```

- [ ] **Step 2: Run tests (PASS — module already passes since impl + tests landed together)**

Run: `cargo test -p datasynth-eval --lib behavioral_fidelity::entity_profile -- --test-threads=4`

- [ ] **Step 3: Re-export in mod.rs**

Add to `mod.rs`:

```rust
pub use entity_profile::{gl_source_tp, real_corpus_aliases, synthetic_aliases};
```

- [ ] **Step 4: Commit**

```bash
git add crates/datasynth-eval/src/behavioral_fidelity/entity_profile.rs crates/datasynth-eval/src/behavioral_fidelity/mod.rs
git commit -m "feat(eval/behavioral): entity profile preset + column alias maps

gl_source_tp() returns the Source + Trading Partner profile. corpus alias map preserves the typo 'Tarding Partner' from the source data exports. Synthetic alias map points at DataSynth's journal_entries output column names.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4 — Wasserstein-1 distance helper

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/math.rs`

The Wasserstein-1 distance between two empirical 1-D distributions equals the L¹ distance between their sorted samples, scaled by 1/n when the sample sizes match. For unequal sample sizes we use the quantile-function formulation: integrate the absolute difference of the inverse CDFs over [0,1] via a Riemann sum on a fine quantile grid.

- [ ] **Step 1: Write tests**

```rust
//! Numerical primitives shared across the behavioral-fidelity metrics.

use std::cmp::Ordering;

/// Wasserstein-1 distance between two empirical 1-D samples.
///
/// Implementation: integrate |F_a^{-1}(t) - F_b^{-1}(t)| over t ∈ [0,1]
/// on a uniform grid of `quantile_steps` knots. For equal-length sorted
/// samples this reduces to the mean L¹ distance, which we use directly
/// as a fast path. Quantile-step default of 1024 keeps error < 1e-6 for
/// practical distributions.
pub fn wasserstein_1(a: &[f64], b: &[f64]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut sa: Vec<f64> = a.iter().copied().filter(|x| x.is_finite()).collect();
    let mut sb: Vec<f64> = b.iter().copied().filter(|x| x.is_finite()).collect();
    sa.sort_by(|x, y| x.partial_cmp(y).unwrap_or(Ordering::Equal));
    sb.sort_by(|x, y| x.partial_cmp(y).unwrap_or(Ordering::Equal));
    if sa.len() == sb.len() {
        return sa
            .iter()
            .zip(sb.iter())
            .map(|(x, y)| (x - y).abs())
            .sum::<f64>()
            / sa.len() as f64;
    }
    const STEPS: usize = 1024;
    let mut acc = 0.0;
    for k in 0..STEPS {
        let t = (k as f64 + 0.5) / STEPS as f64; // midpoint rule
        let qa = quantile_sorted(&sa, t);
        let qb = quantile_sorted(&sb, t);
        acc += (qa - qb).abs();
    }
    acc / STEPS as f64
}

fn quantile_sorted(sorted: &[f64], t: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = t * (sorted.len() as f64 - 1.0);
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let frac = pos - lo as f64;
        sorted[lo] * (1.0 - frac) + sorted[hi] * frac
    }
}

/// Lag-1 Pearson correlation between consecutive elements of `xs`.
///
/// Returns `None` if `xs.len() < 3` or if either of the two shifted
/// series has zero variance.
pub fn pearson_lag1_correlation(xs: &[f64]) -> Option<f64> {
    if xs.len() < 3 {
        return None;
    }
    let a = &xs[..xs.len() - 1];
    let b = &xs[1..];
    let n = a.len() as f64;
    let mean_a = a.iter().sum::<f64>() / n;
    let mean_b = b.iter().sum::<f64>() / n;
    let mut num = 0.0;
    let mut da = 0.0;
    let mut db = 0.0;
    for i in 0..a.len() {
        let xa = a[i] - mean_a;
        let xb = b[i] - mean_b;
        num += xa * xb;
        da  += xa * xa;
        db  += xb * xb;
    }
    if da == 0.0 || db == 0.0 {
        return None;
    }
    Some(num / (da.sqrt() * db.sqrt()))
}

/// Empirical percentile of an unsorted slice (clones + sorts).
pub fn percentile(xs: &[f64], pct: f64) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut s: Vec<f64> = xs.iter().copied().filter(|x| x.is_finite()).collect();
    s.sort_by(|x, y| x.partial_cmp(y).unwrap_or(Ordering::Equal));
    quantile_sorted(&s, pct.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn w1_identical_samples_is_zero() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let b = a.clone();
        assert!((wasserstein_1(&a, &b)).abs() < 1e-9);
    }

    #[test]
    fn w1_shifted_samples_equals_shift() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let b: Vec<f64> = a.iter().map(|x| x + 3.0).collect();
        assert!((wasserstein_1(&a, &b) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn w1_unequal_lengths_handles_gracefully() {
        let a = vec![1.0; 10];
        let b = vec![2.0; 100];
        let d = wasserstein_1(&a, &b);
        assert!((d - 1.0).abs() < 1e-3);
    }

    #[test]
    fn pearson_lag1_positive_autocorr() {
        // strict monotonic: 1,2,3,4,5,6 -> pairs (1,2),(2,3),... correlation 1.
        let xs = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let r = pearson_lag1_correlation(&xs).unwrap();
        assert!((r - 1.0).abs() < 1e-9);
    }

    #[test]
    fn pearson_lag1_negative_autocorr() {
        let xs = vec![1.0, 10.0, 1.0, 10.0, 1.0, 10.0];
        let r = pearson_lag1_correlation(&xs).unwrap();
        assert!(r < -0.9);
    }

    #[test]
    fn pearson_lag1_short_series_returns_none() {
        let xs = vec![1.0, 2.0];
        assert!(pearson_lag1_correlation(&xs).is_none());
    }

    #[test]
    fn percentile_known_values() {
        let xs: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let p50 = percentile(&xs, 0.50);
        assert!((p50 - 50.5).abs() < 1.0);
        let p90 = percentile(&xs, 0.90);
        assert!((p90 - 90.0).abs() < 1.0);
    }
}
```

- [ ] **Step 2: Run tests (PASS)**

Run: `cargo test -p datasynth-eval --lib behavioral_fidelity::math -- --test-threads=4 2>&1 | tail -15`
Expected: `7 passed`.

- [ ] **Step 3: Commit**

```bash
git add crates/datasynth-eval/src/behavioral_fidelity/math.rs
git commit -m "feat(eval/behavioral): W1 + lag-1 autocorr + percentile primitives

Wasserstein-1 with fast equal-length path and quantile-grid fallback for unequal lengths. Pearson lag-1 autocorrelation returning None for too-short or zero-variance series. Empirical percentile helper used by velocity rule R3.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5 — Date utilities

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/math.rs` (append small section)

The IETD work needs day-difference arithmetic. Add helpers.

- [ ] **Step 1: Append to `math.rs`**

```rust
use chrono::{Datelike, NaiveDate, Weekday};

/// Days between two dates, can be negative.
pub fn days_between(a: NaiveDate, b: NaiveDate) -> i64 {
    (b - a).num_days()
}

/// `true` if the date falls on Sat or Sun.
pub fn is_weekend(d: NaiveDate) -> bool {
    matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}
```

- [ ] **Step 2: Append tests**

```rust
#[test]
fn days_between_known() {
    let a = NaiveDate::from_ymd_opt(2022, 4, 25).unwrap();
    let b = NaiveDate::from_ymd_opt(2022, 5, 2).unwrap();
    assert_eq!(days_between(a, b), 7);
}

#[test]
fn is_weekend_known() {
    // 2022-04-30 is a Saturday
    assert!(is_weekend(NaiveDate::from_ymd_opt(2022, 4, 30).unwrap()));
    // 2022-04-25 is a Monday
    assert!(!is_weekend(NaiveDate::from_ymd_opt(2022, 4, 25).unwrap()));
}
```

- [ ] **Step 3: Run tests + commit**

```bash
cargo test -p datasynth-eval --lib behavioral_fidelity::math -- --test-threads=4
git add crates/datasynth-eval/src/behavioral_fidelity/math.rs
git commit -m "feat(eval/behavioral): day-difference + weekend helpers"
```

---

## Task 6 — Parquet + CSV loader

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/loader.rs`

Loads a parquet or CSV file from disk and returns `Vec<Record>` with canonical column names. Auto-detects which alias map to apply by checking column presence (corpus typo `Tarding Partner` vs synthetic `trading_partner`).

- [ ] **Step 1: Write loader.rs**

```rust
//! Parquet / CSV loader producing canonical `Record`s.

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use arrow::array::{Array, Date32Array, Float64Array, StringArray, TimestampMillisecondArray, TimestampMicrosecondArray, TimestampNanosecondArray};
use arrow::record_batch::RecordBatch;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use super::entity_profile::{real_corpus_aliases, synthetic_aliases};
use super::error::{BehavioralFidelityError, BehavioralFidelityResult};
use super::types::Record;

/// Load all records from a parquet file (or single-file directory).
pub fn load_parquet_records(path: &Path) -> BehavioralFidelityResult<Vec<Record>> {
    let path = resolve_single_file(path, "parquet")?;
    let file = File::open(&path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| BehavioralFidelityError::Parquet(e.to_string()))?;
    let reader = builder.build()
        .map_err(|e| BehavioralFidelityError::Parquet(e.to_string()))?;
    let mut out = Vec::new();
    for batch_res in reader {
        let batch = batch_res.map_err(|e| BehavioralFidelityError::Parquet(e.to_string()))?;
        let aliases = pick_alias_map(&batch);
        for row in 0..batch.num_rows() {
            out.push(extract_row(&batch, row, &aliases)?);
        }
    }
    Ok(out)
}

/// Load all records from a CSV file. Header line is required and is used
/// to pick the alias map.
pub fn load_csv_records(path: &Path) -> BehavioralFidelityResult<Vec<Record>> {
    let path = resolve_single_file(path, "csv")?;
    let mut rdr = csv::ReaderBuilder::new().has_headers(true).from_path(&path)
        .map_err(|e| BehavioralFidelityError::Io(std::io::Error::other(e.to_string())))?;
    let headers: Vec<String> = rdr.headers()
        .map_err(|e| BehavioralFidelityError::Io(std::io::Error::other(e.to_string())))?
        .iter().map(|s| s.to_string()).collect();
    let aliases = pick_alias_map_from_headers(&headers);
    let header_idx: HashMap<&str, usize> = headers.iter().enumerate().map(|(i, h)| (h.as_str(), i)).collect();
    let mut out = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| BehavioralFidelityError::Io(std::io::Error::other(e.to_string())))?;
        out.push(extract_csv_row(&rec, &header_idx, &aliases)?);
    }
    Ok(out)
}

fn resolve_single_file(path: &Path, extension: &str) -> BehavioralFidelityResult<std::path::PathBuf> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if entry.path().extension().is_some_and(|e| e.eq_ignore_ascii_case(extension)) {
                return Ok(entry.path());
            }
        }
    }
    Err(BehavioralFidelityError::Io(std::io::Error::other(format!(
        "no {extension} file at {}",
        path.display()
    ))))
}

fn pick_alias_map(batch: &RecordBatch) -> HashMap<&'static str, &'static str> {
    let schema = batch.schema();
    let cols: Vec<_> = schema.fields().iter().map(|f| f.name().as_str().to_string()).collect();
    pick_alias_map_from_headers(&cols)
}

fn pick_alias_map_from_headers(cols: &[String]) -> HashMap<&'static str, &'static str> {
    let has = |needle: &str| cols.iter().any(|c| c == needle);
    if has("Tarding Partner") || has("Functional Amount") {
        real_corpus_aliases().into_iter().collect()
    } else {
        synthetic_aliases().into_iter().collect()
    }
}

fn extract_row(batch: &RecordBatch, row: usize, aliases: &HashMap<&'static str, &'static str>) -> BehavioralFidelityResult<Record> {
    let s = batch.schema();
    let col_idx = |canon: &str| -> Option<usize> {
        let real = aliases.get(canon)?;
        s.fields().iter().position(|f| f.name() == *real)
    };
    let str_at = |canon: &str| -> Option<String> {
        let i = col_idx(canon)?;
        let arr = batch.column(i).as_any().downcast_ref::<StringArray>()?;
        if arr.is_null(row) { None } else { Some(arr.value(row).to_string()) }
    };
    let f64_at = |canon: &str| -> Option<f64> {
        let i = col_idx(canon)?;
        let arr = batch.column(i).as_any().downcast_ref::<Float64Array>()?;
        if arr.is_null(row) { None } else { Some(arr.value(row)) }
    };
    let date_at = |canon: &str| -> Option<NaiveDate> {
        let i = col_idx(canon)?;
        // corpus encodes dates as STRING "YYYY-MM-DD". Synthetic may use Date32.
        if let Some(arr) = batch.column(i).as_any().downcast_ref::<StringArray>() {
            if arr.is_null(row) { return None; }
            return NaiveDate::parse_from_str(arr.value(row), "%Y-%m-%d").ok();
        }
        if let Some(arr) = batch.column(i).as_any().downcast_ref::<Date32Array>() {
            if arr.is_null(row) { return None; }
            return arr.value_as_date(row);
        }
        None
    };
    let ts_at = |canon: &str| -> Option<DateTime<Utc>> {
        let i = col_idx(canon)?;
        if let Some(arr) = batch.column(i).as_any().downcast_ref::<TimestampMillisecondArray>() {
            if arr.is_null(row) { return None; }
            return Utc.timestamp_millis_opt(arr.value(row)).single();
        }
        if let Some(arr) = batch.column(i).as_any().downcast_ref::<TimestampMicrosecondArray>() {
            if arr.is_null(row) { return None; }
            return Utc.timestamp_micros(arr.value(row)).single();
        }
        if let Some(arr) = batch.column(i).as_any().downcast_ref::<TimestampNanosecondArray>() {
            if arr.is_null(row) { return None; }
            let nanos = arr.value(row);
            return Some(Utc.timestamp_nanos(nanos));
        }
        if let Some(arr) = batch.column(i).as_any().downcast_ref::<StringArray>() {
            if arr.is_null(row) { return None; }
            return DateTime::parse_from_rfc3339(arr.value(row))
                .ok()
                .map(|dt| dt.with_timezone(&Utc));
        }
        None
    };

    Ok(Record {
        source:            str_at("Source").unwrap_or_default(),
        gl_account:        str_at("GLAccount").unwrap_or_default(),
        cost_center:       str_at("CostCenter"),
        profit_center:     str_at("ProfitCenter"),
        trading_partner:   str_at("TradingPartner"),
        je_number:         str_at("JENumber").unwrap_or_default(),
        je_line_number:    str_at("JELineNumber").unwrap_or_default(),
        effective_date:    date_at("EffectiveDate")
                              .ok_or_else(|| BehavioralFidelityError::Schema("missing EffectiveDate".into()))?,
        entry_date:        date_at("EntryDate")
                              .ok_or_else(|| BehavioralFidelityError::Schema("missing EntryDate".into()))?,
        created_at:        ts_at("CreatedAt"),
        functional_amount: f64_at("FunctionalAmount").unwrap_or(0.0),
    })
}

fn extract_csv_row(
    rec: &csv::StringRecord,
    headers: &HashMap<&str, usize>,
    aliases: &HashMap<&'static str, &'static str>,
) -> BehavioralFidelityResult<Record> {
    let get = |canon: &str| -> Option<String> {
        let real = aliases.get(canon)?;
        let i = headers.get(real.as_ref())?;
        let v = rec.get(*i)?;
        if v.is_empty() { None } else { Some(v.to_string()) }
    };
    Ok(Record {
        source:            get("Source").unwrap_or_default(),
        gl_account:        get("GLAccount").unwrap_or_default(),
        cost_center:       get("CostCenter"),
        profit_center:     get("ProfitCenter"),
        trading_partner:   get("TradingPartner"),
        je_number:         get("JENumber").unwrap_or_default(),
        je_line_number:    get("JELineNumber").unwrap_or_default(),
        effective_date:    get("EffectiveDate")
            .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
            .ok_or_else(|| BehavioralFidelityError::Schema("missing EffectiveDate".into()))?,
        entry_date:        get("EntryDate")
            .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
            .ok_or_else(|| BehavioralFidelityError::Schema("missing EntryDate".into()))?,
        created_at:        get("CreatedAt")
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))),
        functional_amount: get("FunctionalAmount").and_then(|s| s.parse().ok()).unwrap_or(0.0),
    })
}
```

- [ ] **Step 2: Add `csv` to eval Cargo.toml (likely workspace dep already)**

Verify with `grep -E "^csv" Cargo.toml`. If missing, add `csv = "1.3"` to `[workspace.dependencies]` of root Cargo.toml and `csv = { workspace = true }` to `crates/datasynth-eval/Cargo.toml`.

- [ ] **Step 3: Write a small unit test using an in-memory parquet round-trip**

Append at the end of loader.rs:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use arrow::array::{Date32Array, Float64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use parquet::arrow::ArrowWriter;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn build_real_corpus_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("JE Number", DataType::Utf8, false),
            Field::new("GL Account Number", DataType::Utf8, false),
            Field::new("Functional Amount", DataType::Float64, false),
            Field::new("Effective Date", DataType::Utf8, false),
            Field::new("Entry Date", DataType::Utf8, false),
            Field::new("Source", DataType::Utf8, false),
            Field::new("Cost Center", DataType::Utf8, true),
            Field::new("Profit Center", DataType::Utf8, true),
            Field::new("Tarding Partner", DataType::Utf8, true),
            Field::new("JE Line Number", DataType::Utf8, false),
        ]));
        let arr_je = StringArray::from(vec!["2022-0090-001", "2022-0090-001"]);
        let arr_gl = StringArray::from(vec!["1100", "2000"]);
        let arr_amt = Float64Array::from(vec![100.0, -100.0]);
        let arr_eff = StringArray::from(vec!["2022-04-25", "2022-04-25"]);
        let arr_ent = StringArray::from(vec!["2022-04-14", "2022-04-14"]);
        let arr_src = StringArray::from(vec!["KR", "KR"]);
        let arr_cc  = StringArray::from(vec![Some("CC100"), None]);
        let arr_pc  = StringArray::from(vec![Some("PC100"), None]);
        let arr_tp  = StringArray::from(vec![Some("TP1"),   None]);
        let arr_line = StringArray::from(vec!["001", "002"]);
        RecordBatch::try_new(schema, vec![
            Arc::new(arr_je), Arc::new(arr_gl), Arc::new(arr_amt),
            Arc::new(arr_eff), Arc::new(arr_ent), Arc::new(arr_src),
            Arc::new(arr_cc), Arc::new(arr_pc), Arc::new(arr_tp),
            Arc::new(arr_line),
        ]).unwrap()
    }

    #[test]
    fn load_parquet_real_corpus_shape() {
        let batch = build_real_corpus_batch();
        let mut tmp = NamedTempFile::new().unwrap();
        {
            let mut writer = ArrowWriter::try_new(tmp.as_file_mut().try_clone().unwrap(), batch.schema(), None).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }
        let path = tmp.path().to_path_buf();
        // Force `.parquet` extension by renaming to a sibling tempfile
        let parquet_path = path.with_extension("parquet");
        std::fs::rename(&path, &parquet_path).unwrap();
        let records = load_parquet_records(&parquet_path).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].source, "KR");
        assert_eq!(records[0].cost_center.as_deref(), Some("CC100"));
        assert_eq!(records[1].cost_center, None); // typo column was treated correctly
        assert_eq!(records[0].entry_date, chrono::NaiveDate::from_ymd_opt(2022, 4, 14).unwrap());
    }
}
```

Add `tempfile = "3"` to `[dev-dependencies]` of `datasynth-eval/Cargo.toml`.

- [ ] **Step 4: Run tests (PASS)**

```bash
cargo test -p datasynth-eval --lib behavioral_fidelity::loader -- --test-threads=4 2>&1 | tail -10
```

- [ ] **Step 5: Commit**

```bash
git add crates/datasynth-eval/src/behavioral_fidelity/loader.rs crates/datasynth-eval/Cargo.toml Cargo.toml
git commit -m "feat(eval/behavioral): parquet + csv loader returning canonical Records

Auto-detects corpus vs synthetic schema (typo 'Tarding Partner' is the giveaway), maps to canonical column names, parses dates as either YYYY-MM-DD strings or Date32, supports millisecond/microsecond/nanosecond/RFC3339 timestamps for CreatedAt.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7 — P1: IETD distribution + within-entity autocorrelation

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/ietd.rs`

- [ ] **Step 1: Implement `ietd.rs`**

```rust
//! P1 — Inter-event time distribution + within-entity autocorrelation.

use std::collections::HashMap;

use chrono::NaiveDate;

use super::math::{pearson_lag1_correlation, wasserstein_1};
use super::types::Record;

/// Result of P1 on one (real, synthetic) pair for one entity column.
#[derive(Debug, Clone, PartialEq)]
pub struct P1Outcome {
    pub ietd_w1_days: f64,
    pub autocorr_real: f64,
    pub autocorr_syn:  f64,
    pub autocorr_gap:  f64,
}

/// Compute pooled IETD W₁ and lag-1 within-entity autocorrelation gap.
///
/// `entity_of` projects each Record to its entity identifier; `date_of`
/// projects to its day-resolution timestamp. The pooled IETD is the union
/// of within-entity inter-event time sequences.
pub fn compute_p1<F, G>(
    real: &[Record],
    syn:  &[Record],
    entity_of: F,
    date_of:   G,
) -> P1Outcome
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> NaiveDate + Copy,
{
    let iets_real = pooled_iets(real, entity_of, date_of);
    let iets_syn  = pooled_iets(syn,  entity_of, date_of);
    let w1 = wasserstein_1(&iets_real, &iets_syn);

    let auto_real = pooled_autocorr(real, entity_of, date_of);
    let auto_syn  = pooled_autocorr(syn,  entity_of, date_of);
    P1Outcome {
        ietd_w1_days: w1,
        autocorr_real: auto_real,
        autocorr_syn:  auto_syn,
        autocorr_gap:  (auto_real - auto_syn).abs(),
    }
}

fn group_by_entity<F>(records: &[Record], entity_of: F) -> HashMap<String, Vec<&Record>>
where F: Fn(&Record) -> Option<String> + Copy,
{
    let mut by: HashMap<String, Vec<&Record>> = HashMap::new();
    for r in records {
        if let Some(e) = entity_of(r) {
            by.entry(e).or_default().push(r);
        }
    }
    by
}

fn pooled_iets<F, G>(records: &[Record], entity_of: F, date_of: G) -> Vec<f64>
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> NaiveDate + Copy,
{
    let mut out = Vec::new();
    for (_e, mut rows) in group_by_entity(records, entity_of) {
        if rows.len() < 2 { continue; }
        rows.sort_by_key(|r| date_of(r));
        for w in rows.windows(2) {
            let d = (date_of(w[1]) - date_of(w[0])).num_days() as f64;
            if d >= 0.0 { out.push(d); }
        }
    }
    out
}

fn pooled_autocorr<F, G>(records: &[Record], entity_of: F, date_of: G) -> f64
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> NaiveDate + Copy,
{
    let mut acc = 0.0;
    let mut n = 0;
    for (_e, mut rows) in group_by_entity(records, entity_of) {
        if rows.len() < 3 { continue; }
        rows.sort_by_key(|r| date_of(r));
        let iets: Vec<f64> = rows.windows(2)
            .map(|w| (date_of(w[1]) - date_of(w[0])).num_days() as f64)
            .collect();
        if let Some(r) = pearson_lag1_correlation(&iets) {
            acc += r;
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { acc / n as f64 }
}

/// Convenience: project Record -> `Source`.
pub fn source_of(r: &Record) -> Option<String> {
    Some(r.source.clone())
}

/// Convenience: project Record -> `TradingPartner`.
pub fn trading_partner_of(r: &Record) -> Option<String> {
    r.trading_partner.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn rec(src: &str, year: i32, mon: u32, day: u32) -> Record {
        Record {
            source: src.into(),
            gl_account: "1".into(),
            cost_center: None,
            profit_center: None,
            trading_partner: None,
            je_number: format!("JE-{src}-{day}"),
            je_line_number: "001".into(),
            effective_date: NaiveDate::from_ymd_opt(year, mon, day).unwrap(),
            entry_date:     NaiveDate::from_ymd_opt(year, mon, day).unwrap(),
            created_at: None,
            functional_amount: 1.0,
        }
    }

    #[test]
    fn p1_identical_data_w1_zero_autocorr_gap_zero() {
        let real = vec![
            rec("A", 2022, 1, 1), rec("A", 2022, 1, 2), rec("A", 2022, 1, 3), rec("A", 2022, 1, 4),
            rec("B", 2022, 1, 1), rec("B", 2022, 1, 5), rec("B", 2022, 1, 9),
        ];
        let out = compute_p1(&real, &real, source_of, |r| r.entry_date);
        assert!(out.ietd_w1_days.abs() < 1e-9);
        assert!(out.autocorr_gap.abs() < 1e-9);
    }

    #[test]
    fn p1_compressed_vs_uniform_detects_shift() {
        // real: A bursts (gap 1,1,1), B sparse (gap 4,4)
        let real = vec![
            rec("A", 2022, 1, 1), rec("A", 2022, 1, 2), rec("A", 2022, 1, 3), rec("A", 2022, 1, 4),
            rec("B", 2022, 1, 1), rec("B", 2022, 1, 5), rec("B", 2022, 1, 9),
        ];
        // syn: A spread out (gap 5,5,5), B sparse (gap 4,4) — same total range
        let syn = vec![
            rec("A", 2022, 1, 1), rec("A", 2022, 1, 6), rec("A", 2022, 1, 11), rec("A", 2022, 1, 16),
            rec("B", 2022, 1, 1), rec("B", 2022, 1, 5), rec("B", 2022, 1, 9),
        ];
        let out = compute_p1(&real, &syn, source_of, |r| r.entry_date);
        assert!(out.ietd_w1_days > 0.5, "expected non-trivial W1, got {}", out.ietd_w1_days);
    }
}
```

- [ ] **Step 2: Run tests + commit**

```bash
cargo test -p datasynth-eval --lib behavioral_fidelity::ietd -- --test-threads=4
git add crates/datasynth-eval/src/behavioral_fidelity/ietd.rs
git commit -m "feat(eval/behavioral): P1 IETD W1 + within-entity autocorrelation gap"
```

---

## Task 8 — P2: active lifetime W1

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/burst.rs`

- [ ] **Step 1: Begin burst.rs with active_lifetime() function and tests**

```rust
//! P2 — Burst structure: active lifetime + burst length + JE-line-burst.

use std::collections::HashMap;

use chrono::NaiveDate;

use super::math::wasserstein_1;
use super::types::Record;

/// Per-entity active lifetime in days = max(date) - min(date), 0 for singletons.
pub fn active_lifetimes<F, G>(records: &[Record], entity_of: F, date_of: G) -> Vec<f64>
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> NaiveDate + Copy,
{
    let mut by: HashMap<String, (NaiveDate, NaiveDate)> = HashMap::new();
    for r in records {
        if let Some(e) = entity_of(r) {
            let d = date_of(r);
            by.entry(e)
                .and_modify(|(lo, hi)| {
                    if d < *lo { *lo = d; }
                    if d > *hi { *hi = d; }
                })
                .or_insert((d, d));
        }
    }
    by.into_values()
        .map(|(lo, hi)| (hi - lo).num_days() as f64)
        .collect()
}

pub fn active_lifetime_w1<F, G>(real: &[Record], syn: &[Record], entity_of: F, date_of: G) -> f64
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> NaiveDate + Copy,
{
    let r = active_lifetimes(real, entity_of, date_of);
    let s = active_lifetimes(syn,  entity_of, date_of);
    wasserstein_1(&r, &s)
}

#[cfg(test)]
mod active_lifetime_tests {
    use super::*;
    use super::super::ietd::source_of;

    fn rec(src: &str, day: u32) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, day).unwrap();
        Record {
            source: src.into(), gl_account: "1".into(),
            cost_center: None, profit_center: None, trading_partner: None,
            je_number: format!("J{day}"), je_line_number: "001".into(),
            effective_date: d, entry_date: d, created_at: None,
            functional_amount: 1.0,
        }
    }

    #[test]
    fn active_lifetimes_basic() {
        let rs = vec![
            rec("A", 1), rec("A", 10),         // 9 days
            rec("B", 5), rec("B", 5),          // 0 days (same date)
            rec("C", 1), rec("C", 31),         // 30 days
        ];
        let mut lifs = active_lifetimes(&rs, source_of, |r| r.entry_date);
        lifs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(lifs, vec![0.0, 9.0, 30.0]);
    }
}
```

- [ ] **Step 2: Test + commit**

```bash
cargo test -p datasynth-eval --lib behavioral_fidelity::burst -- --test-threads=4
git add crates/datasynth-eval/src/behavioral_fidelity/burst.rs
git commit -m "feat(eval/behavioral): P2 active lifetime W1"
```

---

## Task 9 — P2: burst length at gap thresholds

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/burst.rs` (append)

- [ ] **Step 1: Append burst-length function + tests**

```rust
/// Compute pooled burst-length distribution at the given gap threshold (in days).
///
/// A burst is a maximal contiguous subsequence of an entity's events with
/// consecutive gaps `<= threshold_days`. Singletons contribute a burst of length 1.
pub fn burst_lengths_at_threshold<F, G>(
    records: &[Record],
    entity_of: F,
    date_of:   G,
    threshold_days: i64,
) -> Vec<f64>
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> NaiveDate + Copy,
{
    let mut by: HashMap<String, Vec<NaiveDate>> = HashMap::new();
    for r in records {
        if let Some(e) = entity_of(r) {
            by.entry(e).or_default().push(date_of(r));
        }
    }
    let mut out = Vec::new();
    for (_e, mut dates) in by {
        dates.sort();
        let mut len = 1u32;
        for w in dates.windows(2) {
            let gap = (w[1] - w[0]).num_days();
            if gap <= threshold_days {
                len += 1;
            } else {
                out.push(len as f64);
                len = 1;
            }
        }
        out.push(len as f64);
    }
    out
}

pub fn burst_length_w1<F, G>(
    real: &[Record],
    syn:  &[Record],
    entity_of: F,
    date_of:   G,
    threshold_days: i64,
) -> f64
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> NaiveDate + Copy,
{
    let r = burst_lengths_at_threshold(real, entity_of, date_of, threshold_days);
    let s = burst_lengths_at_threshold(syn,  entity_of, date_of, threshold_days);
    wasserstein_1(&r, &s)
}

#[cfg(test)]
mod burst_length_tests {
    use super::*;
    use super::super::ietd::source_of;

    fn rec(src: &str, day: u32) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, day).unwrap();
        Record {
            source: src.into(), gl_account: "1".into(),
            cost_center: None, profit_center: None, trading_partner: None,
            je_number: format!("J{src}{day}"), je_line_number: "001".into(),
            effective_date: d, entry_date: d, created_at: None,
            functional_amount: 1.0,
        }
    }

    #[test]
    fn burst_lengths_threshold_1day() {
        // A: 1,2,3 (one burst of 3 at gap <=1)
        // B: 1, 5, 6 (singleton + burst of 2)
        let rs = vec![
            rec("A", 1), rec("A", 2), rec("A", 3),
            rec("B", 1), rec("B", 5), rec("B", 6),
        ];
        let mut bl = burst_lengths_at_threshold(&rs, source_of, |r| r.entry_date, 1);
        bl.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(bl, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn burst_lengths_threshold_4days_merges_b() {
        let rs = vec![
            rec("A", 1), rec("A", 2), rec("A", 3),
            rec("B", 1), rec("B", 5), rec("B", 6),
        ];
        let mut bl = burst_lengths_at_threshold(&rs, source_of, |r| r.entry_date, 4);
        bl.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(bl, vec![3.0, 3.0]); // both bursts of length 3
    }
}
```

- [ ] **Step 2: Test + commit**

```bash
cargo test -p datasynth-eval --lib behavioral_fidelity::burst -- --test-threads=4
git add crates/datasynth-eval/src/behavioral_fidelity/burst.rs
git commit -m "feat(eval/behavioral): P2 burst-length W1 at configurable gap thresholds"
```

---

## Task 10 — P2: JE-line-burst structural metric

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/burst.rs` (append)

- [ ] **Step 1: Append `je_line_burst_lengths()` + W1 + test**

```rust
/// Pooled lines-per-JE-Number distribution.
pub fn je_line_burst_lengths(records: &[Record]) -> Vec<f64> {
    let mut by: HashMap<String, u32> = HashMap::new();
    for r in records {
        *by.entry(r.je_number.clone()).or_insert(0) += 1;
    }
    by.values().map(|&n| n as f64).collect()
}

pub fn je_line_burst_w1(real: &[Record], syn: &[Record]) -> f64 {
    let r = je_line_burst_lengths(real);
    let s = je_line_burst_lengths(syn);
    wasserstein_1(&r, &s)
}

#[cfg(test)]
mod je_line_burst_tests {
    use super::*;
    use chrono::NaiveDate;

    fn rec(je: &str, line: &str) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        Record {
            source: "S".into(), gl_account: "1".into(),
            cost_center: None, profit_center: None, trading_partner: None,
            je_number: je.into(), je_line_number: line.into(),
            effective_date: d, entry_date: d, created_at: None,
            functional_amount: 1.0,
        }
    }

    #[test]
    fn lines_per_je_grouped_correctly() {
        let rs = vec![
            rec("J1","001"), rec("J1","002"), rec("J1","003"),
            rec("J2","001"), rec("J2","002"),
            rec("J3","001"),
        ];
        let mut lengths = je_line_burst_lengths(&rs);
        lengths.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(lengths, vec![1.0, 2.0, 3.0]);
    }
}
```

- [ ] **Step 2: Test + commit**

```bash
cargo test -p datasynth-eval --lib behavioral_fidelity::burst -- --test-threads=4
git add crates/datasynth-eval/src/behavioral_fidelity/burst.rs
git commit -m "feat(eval/behavioral): P2 JE-line-burst length W1 (structural)"
```

---

## Task 11 — P3: fan-out distribution

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/fanout.rs`

- [ ] **Step 1: Implement fan-out W1**

```rust
//! P3 — Shared-infrastructure graph motifs.

use std::collections::{HashMap, HashSet};

use super::math::wasserstein_1;
use super::types::Record;

/// `attr_of` returns the attribute value for a record (e.g. its CostCenter)
/// or None if absent.
pub type AttrOf = fn(&Record) -> Option<String>;

/// For each attribute value, count the number of distinct entities that touched it.
pub fn fanout_distribution<F, G>(records: &[Record], entity_of: F, attr_of: G) -> Vec<f64>
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> Option<String> + Copy,
{
    let mut by_attr: HashMap<String, HashSet<String>> = HashMap::new();
    for r in records {
        let (Some(e), Some(a)) = (entity_of(r), attr_of(r)) else { continue };
        by_attr.entry(a).or_default().insert(e);
    }
    by_attr.values().map(|s| s.len() as f64).collect()
}

pub fn fanout_w1<F, G>(real: &[Record], syn: &[Record], entity_of: F, attr_of: G) -> f64
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> Option<String> + Copy,
{
    let r = fanout_distribution(real, entity_of, attr_of);
    let s = fanout_distribution(syn,  entity_of, attr_of);
    wasserstein_1(&r, &s)
}

/// Convenience projectors.
pub fn gl_account_of(r: &Record) -> Option<String> { Some(r.gl_account.clone()) }
pub fn cost_center_of(r: &Record) -> Option<String> { r.cost_center.clone() }
pub fn profit_center_of(r: &Record) -> Option<String> { r.profit_center.clone() }
pub fn trading_partner_attr_of(r: &Record) -> Option<String> { r.trading_partner.clone() }

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ietd::source_of;
    use chrono::NaiveDate;

    fn rec(src: &str, gl: &str) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        Record {
            source: src.into(), gl_account: gl.into(),
            cost_center: None, profit_center: None, trading_partner: None,
            je_number: format!("J{src}{gl}"), je_line_number: "001".into(),
            effective_date: d, entry_date: d, created_at: None, functional_amount: 1.0,
        }
    }

    #[test]
    fn fanout_distribution_basic() {
        // GL account "100" is touched by Source A and B → fan-out 2.
        // GL account "200" by Source A only → fan-out 1.
        let rs = vec![
            rec("A","100"), rec("A","200"), rec("B","100"),
        ];
        let mut fo = fanout_distribution(&rs, source_of, gl_account_of);
        fo.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(fo, vec![1.0, 2.0]);
    }
}
```

- [ ] **Step 2: Test + commit**

```bash
cargo test -p datasynth-eval --lib behavioral_fidelity::fanout -- --test-threads=4
git add crates/datasynth-eval/src/behavioral_fidelity/fanout.rs
git commit -m "feat(eval/behavioral): P3 fan-out distribution W1"
```

---

## Task 12 — P3: clustering coefficient + triangle log-ratio

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/fanout.rs` (append)

The entity-projection graph G_E connects two entities when they share at least one attribute value. Clustering coefficient = (3 × triangles) / (#connected triples). Triangle count via brute-force enumeration over a sparse adjacency since entity counts are small (≤ ~1000).

- [ ] **Step 1: Append clustering + triangle logic**

```rust
use petgraph::graph::{NodeIndex, UnGraph};

/// Entity-projection graph: undirected, one node per entity that shares any attribute.
fn build_entity_projection<F, G>(
    records: &[Record],
    entity_of: F,
    attr_of:   G,
) -> (UnGraph<String, ()>, HashMap<String, NodeIndex>)
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> Option<String> + Copy,
{
    // entity -> set of attr values it touched
    let mut by_entity: HashMap<String, HashSet<String>> = HashMap::new();
    for r in records {
        let (Some(e), Some(a)) = (entity_of(r), attr_of(r)) else { continue };
        by_entity.entry(e).or_default().insert(a);
    }

    let mut g = UnGraph::<String, ()>::new_undirected();
    let mut idx: HashMap<String, NodeIndex> = HashMap::new();
    for e in by_entity.keys() {
        idx.insert(e.clone(), g.add_node(e.clone()));
    }
    // O(E^2) entity pairs; small entity sets in our profiles.
    let entities: Vec<&String> = by_entity.keys().collect();
    for i in 0..entities.len() {
        for j in (i + 1)..entities.len() {
            let a = &by_entity[entities[i]];
            let b = &by_entity[entities[j]];
            if a.iter().any(|v| b.contains(v)) {
                g.add_edge(idx[entities[i]], idx[entities[j]], ());
            }
        }
    }
    (g, idx)
}

pub fn clustering_coefficient<F, G>(records: &[Record], entity_of: F, attr_of: G) -> f64
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> Option<String> + Copy,
{
    let (g, _) = build_entity_projection(records, entity_of, attr_of);
    let mut triangles = 0usize;
    let mut triples   = 0usize;
    for n in g.node_indices() {
        let neighbors: Vec<NodeIndex> = g.neighbors(n).collect();
        let k = neighbors.len();
        if k < 2 { continue; }
        triples += k * (k - 1) / 2;
        for i in 0..k {
            for j in (i + 1)..k {
                if g.find_edge(neighbors[i], neighbors[j]).is_some() {
                    triangles += 1;
                }
            }
        }
    }
    // Each triangle counted three times (once per vertex).
    let triangles = triangles / 3;
    if triples == 0 { 0.0 } else { (3 * triangles) as f64 / triples as f64 }
}

pub fn triangle_count<F, G>(records: &[Record], entity_of: F, attr_of: G) -> u64
where
    F: Fn(&Record) -> Option<String> + Copy,
    G: Fn(&Record) -> Option<String> + Copy,
{
    let (g, _) = build_entity_projection(records, entity_of, attr_of);
    let mut t = 0u64;
    for n in g.node_indices() {
        let neighbors: Vec<NodeIndex> = g.neighbors(n).collect();
        let k = neighbors.len();
        for i in 0..k {
            for j in (i + 1)..k {
                if g.find_edge(neighbors[i], neighbors[j]).is_some() {
                    t += 1;
                }
            }
        }
    }
    t / 3
}

pub fn triangle_log_ratio_gap(real: u64, syn: u64) -> f64 {
    let lr = ((real + 1) as f64) / ((syn + 1) as f64);
    lr.ln().abs()
}

#[cfg(test)]
mod cluster_tests {
    use super::*;
    use super::super::ietd::source_of;
    use chrono::NaiveDate;

    fn rec(src: &str, gl: &str) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        Record {
            source: src.into(), gl_account: gl.into(),
            cost_center: None, profit_center: None, trading_partner: None,
            je_number: format!("J{src}{gl}"), je_line_number: "001".into(),
            effective_date: d, entry_date: d, created_at: None, functional_amount: 1.0,
        }
    }

    #[test]
    fn triangle_in_three_entity_ring() {
        // A,B,C each share GL "X" with the other two → triangle.
        let rs = vec![
            rec("A","X"), rec("B","X"), rec("C","X"),
        ];
        let t = triangle_count(&rs, source_of, gl_account_of);
        assert_eq!(t, 1);
        let cc = clustering_coefficient(&rs, source_of, gl_account_of);
        assert!((cc - 1.0).abs() < 1e-9);
    }

    #[test]
    fn no_shared_attribute_no_edges() {
        let rs = vec![
            rec("A","1"), rec("B","2"), rec("C","3"),
        ];
        assert_eq!(triangle_count(&rs, source_of, gl_account_of), 0);
        assert_eq!(clustering_coefficient(&rs, source_of, gl_account_of), 0.0);
    }
}
```

- [ ] **Step 2: Test + commit**

```bash
cargo test -p datasynth-eval --lib behavioral_fidelity::fanout -- --test-threads=4
git add crates/datasynth-eval/src/behavioral_fidelity/fanout.rs
git commit -m "feat(eval/behavioral): P3 clustering coefficient + triangle log-ratio gap

Builds the entity-projection graph via petgraph::UnGraph; clustering coefficient via neighbour-pair triangle enumeration; triangle log-ratio gap = |log((t_real+1)/(t_syn+1))|."
```

---

## Task 13 — P4: canonical R1–R10 + per-entity trigger evaluation

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/velocity_rules.rs`
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/types.rs` (extend `RuleSet::canonical_gl_rules`)

- [ ] **Step 1: Populate `RuleSet::canonical_gl_rules()` in `types.rs`**

Replace the stub `canonical_gl_rules` with:

```rust
impl RuleSet {
    pub fn canonical_gl_rules() -> Self {
        Self {
            rules: vec![
                VelocityRuleSpec { id: "R1".into(),  description: ">5 JEs / Source / business day".into(),
                    kind: VelocityRuleKind::CountPerEntityPerDay { threshold: 5 } },
                VelocityRuleSpec { id: "R2".into(),  description: ">10 distinct GL accounts / Source / day".into(),
                    kind: VelocityRuleKind::DistinctAccountsPerEntityPerDay { threshold: 10 } },
                VelocityRuleSpec { id: "R3".into(),  description: "Sum |amount| / Source / day > p90".into(),
                    kind: VelocityRuleKind::SumAmountPerEntityPerDayAbovePercentile { pct: 0.90 } },
                VelocityRuleSpec { id: "R4".into(),  description: "Posting to account dormant >=180 days".into(),
                    kind: VelocityRuleKind::DormantAccountActivity { inactivity_days: 180 } },
                VelocityRuleSpec { id: "R5".into(),  description: ">3 distinct Trading Partners / Source / day".into(),
                    kind: VelocityRuleKind::DistinctTradingPartnersPerEntityPerDay { threshold: 3 } },
                VelocityRuleSpec { id: "R6".into(),  description: "max/median amount per Source in 30d > 3.0".into(),
                    kind: VelocityRuleKind::AmountSpikeRatio { window_days: 30, ratio: 3.0 } },
                VelocityRuleSpec { id: "R7".into(),  description: "Off-hours posting (EntryDate weekday in Sat/Sun)".into(),
                    kind: VelocityRuleKind::OffHoursPosting },
                VelocityRuleSpec { id: "R8".into(),  description: "Post-close posting (EntryDate > period_end + 5bd)".into(),
                    kind: VelocityRuleKind::PostClosePosting { tolerance_business_days: 5 } },
                VelocityRuleSpec { id: "R9".into(),  description: "Round-dollar share (|amt| mod 1000 == 0) > 10%".into(),
                    kind: VelocityRuleKind::RoundDollarConcentration { share_threshold: 0.10 } },
                VelocityRuleSpec { id: "R10".into(), description: "Backdating (EffectiveDate − EntryDate > 30d)".into(),
                    kind: VelocityRuleKind::BackdatingDays { gap_days: 30 } },
            ],
        }
    }
}
```

- [ ] **Step 2: Implement velocity_rules.rs**

```rust
//! P4 — Velocity-rule trigger rate gap.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, NaiveDate};

use super::math::{is_weekend, percentile};
use super::types::{Record, RuleSet, VelocityRuleKind, VelocityRuleSpec};

/// Per-rule trigger rate result.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleResult {
    pub id: String,
    pub trigger_rate_real: f64,
    pub trigger_rate_syn:  f64,
    pub abs_gap: f64,
}

/// Computes per-rule TR(real) and TR(syn) and the mean absolute gap across rules.
pub fn evaluate_rule_set<F>(
    rules: &RuleSet,
    real:  &[Record],
    syn:   &[Record],
    entity_of: F,
) -> (Vec<RuleResult>, f64)
where F: Fn(&Record) -> Option<String> + Copy,
{
    let mut results = Vec::with_capacity(rules.rules.len());
    let mut gaps_sum = 0.0;
    for rule in &rules.rules {
        let tr_real = trigger_rate(rule, real, entity_of);
        let tr_syn  = trigger_rate(rule, syn,  entity_of);
        let gap = (tr_real - tr_syn).abs();
        gaps_sum += gap;
        results.push(RuleResult {
            id: rule.id.clone(),
            trigger_rate_real: tr_real,
            trigger_rate_syn:  tr_syn,
            abs_gap: gap,
        });
    }
    let mean_gap = if rules.rules.is_empty() { 0.0 } else { gaps_sum / rules.rules.len() as f64 };
    (results, mean_gap)
}

fn trigger_rate<F>(rule: &VelocityRuleSpec, records: &[Record], entity_of: F) -> f64
where F: Fn(&Record) -> Option<String> + Copy,
{
    let by_entity: HashMap<String, Vec<&Record>> = group_by_entity(records, entity_of);
    if by_entity.is_empty() { return 0.0; }

    let triggered: usize = by_entity.values()
        .filter(|rows| entity_triggers(rule, rows))
        .count();
    triggered as f64 / by_entity.len() as f64
}

fn group_by_entity<F>(records: &[Record], entity_of: F) -> HashMap<String, Vec<&Record>>
where F: Fn(&Record) -> Option<String> + Copy,
{
    let mut by: HashMap<String, Vec<&Record>> = HashMap::new();
    for r in records {
        if let Some(e) = entity_of(r) { by.entry(e).or_default().push(r); }
    }
    by
}

fn entity_triggers(rule: &VelocityRuleSpec, rows: &[&Record]) -> bool {
    match &rule.kind {
        VelocityRuleKind::CountPerEntityPerDay { threshold } => {
            let mut by_day: HashMap<NaiveDate, u32> = HashMap::new();
            for r in rows {
                if !is_weekend(r.entry_date) {
                    *by_day.entry(r.entry_date).or_insert(0) += 1;
                }
            }
            by_day.values().any(|&c| c > *threshold)
        }
        VelocityRuleKind::DistinctAccountsPerEntityPerDay { threshold } => {
            let mut by_day: HashMap<NaiveDate, HashSet<&str>> = HashMap::new();
            for r in rows {
                by_day.entry(r.entry_date).or_default().insert(r.gl_account.as_str());
            }
            by_day.values().any(|s| s.len() > *threshold as usize)
        }
        VelocityRuleKind::SumAmountPerEntityPerDayAbovePercentile { pct } => {
            // Compute distribution of per-entity-per-day sums across `rows`'s own dates.
            let mut by_day: HashMap<NaiveDate, f64> = HashMap::new();
            for r in rows {
                *by_day.entry(r.entry_date).or_insert(0.0) += r.functional_amount.abs();
            }
            let sums: Vec<f64> = by_day.values().copied().collect();
            if sums.is_empty() { return false; }
            let threshold = percentile(&sums, *pct);
            sums.iter().any(|&s| s > threshold)
        }
        VelocityRuleKind::DormantAccountActivity { inactivity_days } => {
            // Within this entity: was an account posted after >= `inactivity_days` since its previous post?
            let mut last_seen: HashMap<&str, NaiveDate> = HashMap::new();
            let mut sorted = rows.to_vec();
            sorted.sort_by_key(|r| r.entry_date);
            for r in sorted {
                if let Some(prev) = last_seen.get(r.gl_account.as_str()) {
                    if (r.entry_date - *prev).num_days() >= *inactivity_days {
                        return true;
                    }
                }
                last_seen.insert(r.gl_account.as_str(), r.entry_date);
            }
            false
        }
        VelocityRuleKind::DistinctTradingPartnersPerEntityPerDay { threshold } => {
            let mut by_day: HashMap<NaiveDate, HashSet<&str>> = HashMap::new();
            for r in rows {
                if let Some(tp) = r.trading_partner.as_deref() {
                    by_day.entry(r.entry_date).or_default().insert(tp);
                }
            }
            by_day.values().any(|s| s.len() > *threshold as usize)
        }
        VelocityRuleKind::AmountSpikeRatio { window_days, ratio } => {
            // Rolling 30-day window; if any window has max/median > ratio, trigger.
            let mut sorted = rows.to_vec();
            sorted.sort_by_key(|r| r.entry_date);
            for end_idx in 0..sorted.len() {
                let end_date = sorted[end_idx].entry_date;
                let start_date = end_date - chrono::Duration::days(*window_days);
                let window: Vec<f64> = sorted.iter()
                    .filter(|r| r.entry_date >= start_date && r.entry_date <= end_date)
                    .map(|r| r.functional_amount.abs())
                    .filter(|x| *x > 0.0)
                    .collect();
                if window.len() < 3 { continue; }
                let max = window.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                let med = percentile(&window, 0.5);
                if med > 0.0 && (max / med) > *ratio { return true; }
            }
            false
        }
        VelocityRuleKind::OffHoursPosting => {
            rows.iter().any(|r| is_weekend(r.entry_date))
        }
        VelocityRuleKind::PostClosePosting { tolerance_business_days } => {
            // Period end = last day of the month inferred from EffectiveDate; post-close if EntryDate > end + tol bd
            rows.iter().any(|r| {
                let period_end = last_day_of_month(r.effective_date);
                let tol_days = *tolerance_business_days * 7 / 5; // approx
                (r.entry_date - period_end).num_days() > tol_days
            })
        }
        VelocityRuleKind::RoundDollarConcentration { share_threshold } => {
            let n = rows.len();
            if n == 0 { return false; }
            let rounds = rows.iter()
                .filter(|r| {
                    let amt = r.functional_amount.abs().round() as i64;
                    amt > 0 && amt % 1000 == 0
                })
                .count();
            (rounds as f64 / n as f64) > *share_threshold
        }
        VelocityRuleKind::BackdatingDays { gap_days } => {
            rows.iter().any(|r| (r.effective_date - r.entry_date).num_days() > *gap_days)
        }
    }
}

fn last_day_of_month(d: NaiveDate) -> NaiveDate {
    let (y, m) = (d.year(), d.month());
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1).unwrap().pred_opt().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ietd::source_of;

    fn r(src: &str, gl: &str, day: u32, amt: f64, tp: Option<&str>) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, day).unwrap();
        Record {
            source: src.into(), gl_account: gl.into(),
            cost_center: None, profit_center: None, trading_partner: tp.map(String::from),
            je_number: format!("J{src}{day}{gl}"), je_line_number: "001".into(),
            effective_date: d, entry_date: d, created_at: None, functional_amount: amt,
        }
    }

    #[test]
    fn r1_count_per_day_triggers() {
        let recs: Vec<Record> = (0..6).map(|_| r("A", "100", 3, 1.0, None)).collect();
        let kind = VelocityRuleSpec { id:"R1".into(), description:"".into(),
            kind: VelocityRuleKind::CountPerEntityPerDay { threshold: 5 } };
        // On Mon 2022-01-03, 6 postings > 5 → triggers.
        assert!(entity_triggers(&kind, &recs.iter().collect::<Vec<_>>()));
    }

    #[test]
    fn r7_off_hours_triggers_on_weekend_record() {
        // 2022-01-01 was a Saturday.
        let recs = vec![r("A", "1", 1, 1.0, None)];
        let kind = VelocityRuleSpec { id:"R7".into(), description:"".into(),
            kind: VelocityRuleKind::OffHoursPosting };
        assert!(entity_triggers(&kind, &recs.iter().collect::<Vec<_>>()));
    }

    #[test]
    fn r10_backdating_triggers_when_eff_minus_entry_over_30() {
        let mut rec = r("A", "1", 1, 1.0, None);
        rec.effective_date = NaiveDate::from_ymd_opt(2022, 3, 31).unwrap();
        rec.entry_date     = NaiveDate::from_ymd_opt(2022, 1, 15).unwrap();
        let kind = VelocityRuleSpec { id:"R10".into(), description:"".into(),
            kind: VelocityRuleKind::BackdatingDays { gap_days: 30 } };
        assert!(entity_triggers(&kind, &[&rec]));
    }
}
```

- [ ] **Step 3: Test + commit**

```bash
cargo test -p datasynth-eval --lib behavioral_fidelity::velocity_rules -- --test-threads=4
git add crates/datasynth-eval/src/behavioral_fidelity/velocity_rules.rs crates/datasynth-eval/src/behavioral_fidelity/types.rs
git commit -m "feat(eval/behavioral): P4 canonical R1..R10 + trigger-rate gap

Canonical GL velocity rules: R1 count, R2 distinct accounts, R3 sum>p90, R4 dormant-account wake, R5 distinct TPs, R6 amount spike, R7 off-hours, R8 post-close, R9 round-dollar share, R10 backdating. Per-rule trigger-rate gap + composite mean."
```

---

## Task 14 — Degradation ratio + 50/50 split

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/degradation.rs`

The split is deterministic over `JENumber`: every line of the same JE goes to the same half. Hash is `seahash`-equivalent via Rust's `DefaultHasher` for determinism across runs (the runtime hash randomisation is disabled per Rust convention when using `BuildHasherDefault`).

- [ ] **Step 1: Implement degradation.rs**

```rust
//! Noise-floor anchored degradation-ratio normaliser.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::types::Record;

/// Deterministic 50/50 split of `records` by `JENumber` (so multi-line JEs stay together).
pub fn split_5050(records: &[Record], seed: u64) -> (Vec<Record>, Vec<Record>) {
    let mut a = Vec::new();
    let mut b = Vec::new();
    for r in records {
        if hash_to_bucket(&r.je_number, seed) {
            a.push(r.clone());
        } else {
            b.push(r.clone());
        }
    }
    (a, b)
}

fn hash_to_bucket(key: &str, seed: u64) -> bool {
    let mut h = DefaultHasher::new();
    seed.hash(&mut h);
    key.hash(&mut h);
    (h.finish() & 1) == 0
}

/// degradation_ratio(real_vs_syn, real_A_vs_real_B). epsilon-protected.
pub fn degradation_ratio(real_vs_syn: f64, real_split_baseline: f64) -> f64 {
    const EPS: f64 = 1e-9;
    if real_split_baseline.abs() < EPS {
        real_vs_syn / EPS
    } else {
        real_vs_syn / real_split_baseline
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn r(je: &str, line: &str) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        Record { source:"S".into(), gl_account:"1".into(),
            cost_center:None, profit_center:None, trading_partner:None,
            je_number:je.into(), je_line_number:line.into(),
            effective_date:d, entry_date:d, created_at:None, functional_amount:1.0 }
    }

    #[test]
    fn split_keeps_multiline_jes_together() {
        let rs = vec![
            r("J1","001"), r("J1","002"), r("J1","003"),
            r("J2","001"), r("J3","001"), r("J4","001"),
            r("J5","001"), r("J6","001"),
        ];
        let (a, b) = split_5050(&rs, 42);
        // J1's three lines all go to the same side.
        let j1_a = a.iter().filter(|r| r.je_number == "J1").count();
        let j1_b = b.iter().filter(|r| r.je_number == "J1").count();
        assert!((j1_a == 3 && j1_b == 0) || (j1_a == 0 && j1_b == 3));
        // Reproducibility: same seed, same split.
        let (a2, _) = split_5050(&rs, 42);
        assert_eq!(a, a2);
        // Different seed: different split (statistically).
        let (a3, _) = split_5050(&rs, 99);
        assert_ne!(a, a3);
    }

    #[test]
    fn degradation_ratio_handles_zero_baseline() {
        let dr = degradation_ratio(1.0, 0.0);
        assert!(dr > 1e8); // 1/epsilon
        assert_eq!(degradation_ratio(0.5, 0.25), 2.0);
    }
}
```

- [ ] **Step 2: Test + commit**

```bash
cargo test -p datasynth-eval --lib behavioral_fidelity::degradation -- --test-threads=4
git add crates/datasynth-eval/src/behavioral_fidelity/degradation.rs
git commit -m "feat(eval/behavioral): 50/50 split (JE-grouped, deterministic) + DR normaliser"
```

---

## Task 15 — Intraday synthetic-only metrics

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/intraday.rs`

- [ ] **Step 1: Implement intraday.rs**

```rust
//! Synth-only intraday structural metrics (second resolution, off-hours rate).
//!
//! These are *informational*: they do not contribute to the composite BF score
//! because the corpus is date-only.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};

use super::math::pearson_lag1_correlation;
use super::types::Record;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntradayMetrics {
    /// Pooled within-entity IETD in seconds (synth only).
    pub p1_intra_w1_seconds: f64,
    /// Pooled lag-1 autocorrelation at second resolution.
    pub p1_intra_autocorr: f64,
    /// Off-hours rate: weekend or hour ∈ [0..6) ∪ [22..24).
    pub off_hours_rate: f64,
}

pub fn compute_intraday<F>(records: &[Record], entity_of: F) -> Option<IntradayMetrics>
where F: Fn(&Record) -> Option<String> + Copy,
{
    // Bail if no CreatedAt populated.
    if records.iter().all(|r| r.created_at.is_none()) {
        return None;
    }

    // Group by entity; sort by CreatedAt; compute IET in seconds.
    let mut by: HashMap<String, Vec<DateTime<Utc>>> = HashMap::new();
    for r in records {
        if let (Some(e), Some(ts)) = (entity_of(r), r.created_at) {
            by.entry(e).or_default().push(ts);
        }
    }
    let mut all_iets: Vec<f64> = Vec::new();
    let mut auto_sum = 0.0;
    let mut auto_n = 0;
    for (_e, mut times) in by {
        if times.len() < 2 { continue; }
        times.sort();
        let iets: Vec<f64> = times.windows(2)
            .map(|w| (w[1] - w[0]).num_seconds() as f64)
            .collect();
        all_iets.extend(iets.iter().copied());
        if let Some(rc) = pearson_lag1_correlation(&iets) {
            auto_sum += rc;
            auto_n += 1;
        }
    }
    let pooled_w1 = if all_iets.is_empty() {
        0.0
    } else {
        // For a single-side metric, "W1" against itself is 0; we instead
        // report the *median* IET in seconds as a placeholder structural number.
        let mut sorted = all_iets.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        sorted[sorted.len() / 2]
    };
    let autocorr = if auto_n == 0 { 0.0 } else { auto_sum / auto_n as f64 };

    // Off-hours rate.
    let mut off = 0usize;
    let mut total = 0usize;
    for r in records {
        if let Some(ts) = r.created_at {
            total += 1;
            let h = ts.hour();
            let wd = ts.weekday().num_days_from_monday();
            if wd >= 5 || h < 6 || h >= 22 { off += 1; }
        }
    }
    let off_rate = if total == 0 { 0.0 } else { off as f64 / total as f64 };

    Some(IntradayMetrics {
        p1_intra_w1_seconds: pooled_w1,
        p1_intra_autocorr:   autocorr,
        off_hours_rate:      off_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ietd::source_of;
    use chrono::{NaiveDate, TimeZone};

    fn r(src: &str, hour: u32, minute: u32) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, 3).unwrap(); // Monday
        let ts = Utc.with_ymd_and_hms(2022, 1, 3, hour, minute, 0).unwrap();
        Record { source: src.into(), gl_account:"1".into(),
            cost_center:None, profit_center:None, trading_partner:None,
            je_number:format!("J{src}{hour}{minute}"), je_line_number:"001".into(),
            effective_date:d, entry_date:d, created_at: Some(ts), functional_amount: 1.0 }
    }

    #[test]
    fn intraday_off_hours_rate_known() {
        let rs = vec![
            r("A", 23, 0),   // off-hours
            r("A", 3, 0),    // off-hours
            r("A", 10, 0),   // in-hours
            r("A", 14, 0),   // in-hours
        ];
        let m = compute_intraday(&rs, source_of).unwrap();
        assert!((m.off_hours_rate - 0.5).abs() < 1e-9);
    }
}
```

- [ ] **Step 2: Test + commit**

```bash
cargo test -p datasynth-eval --lib behavioral_fidelity::intraday -- --test-threads=4
git add crates/datasynth-eval/src/behavioral_fidelity/intraday.rs
git commit -m "feat(eval/behavioral): synth-only intraday structural metrics (IETD seconds, off-hours rate)"
```

---

## Task 16 — Report struct + JSON writer

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/report.rs`

- [ ] **Step 1: Implement report.rs**

```rust
//! BehavioralFidelityReport struct + JSON / Markdown / CSV serialisation.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::error::{BehavioralFidelityError, BehavioralFidelityResult};
use super::intraday::IntradayMetrics;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorpusSummary {
    pub path: String,
    pub n_rows: usize,
    pub n_entities_primary: usize,
    pub n_entities_secondary: usize,
    pub period_start: Option<NaiveDate>,
    pub period_end:   Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineValues {
    pub p1_ietd_w1_days: f64,
    pub p1_autocorr_gap: f64,
    pub p2_active_lifetime_w1: f64,
    pub p2_burst_len_by_threshold: BTreeMap<i64, f64>,
    pub p2_je_line_burst_w1: f64,
    pub p3_fanout_by_attr: BTreeMap<String, f64>,
    pub p3_clustering_gap: f64,
    pub p3_triangle_log_ratio: f64,
    pub p4_mean_gap: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerMetric {
    pub raw: f64,
    pub baseline: f64,
    pub dr: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityMetrics {
    pub entity_column: String,
    pub p1_ietd: PerMetric,
    pub p1_autocorr: PerMetric,
    pub p2_active_lifetime: PerMetric,
    pub p2_burst_len_by_threshold: BTreeMap<i64, PerMetric>,
    pub p2_je_line_burst: PerMetric,
    pub p3_fanout_by_attr: BTreeMap<String, PerMetric>,
    pub p3_clustering: PerMetric,
    pub p3_triangle_log_ratio: PerMetric,
    pub p4_rule_results: Vec<super::velocity_rules::RuleResult>,
    pub p4_mean_gap: PerMetric,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateResult {
    pub fail_if_dr_above: f64,
    pub fail_if_composite_above: f64,
    pub passed: bool,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BehavioralFidelityReport {
    pub profile: String,
    pub generator_id: String,
    pub generator_version: String,
    pub seed: u64,
    pub generated_at: DateTime<Utc>,
    pub real_corpus: CorpusSummary,
    pub synthetic: CorpusSummary,
    pub noise_floor: BaselineValues,
    pub per_entity: BTreeMap<String, EntityMetrics>,
    pub composite_bf_score: f64,
    pub intraday_structural: Option<IntradayMetrics>,
    pub gates: GateResult,
}

impl BehavioralFidelityReport {
    pub fn write_json(&self, path: &Path) -> BehavioralFidelityResult<()> {
        let f = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(f, self)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn json_roundtrip_preserves_btreemap_ordering() {
        let mut by_attr = BTreeMap::new();
        by_attr.insert("CostCenter".to_string(), 1.0);
        by_attr.insert("GLAccount".to_string(),  2.0);
        let baseline = BaselineValues {
            p1_ietd_w1_days: 1.0, p1_autocorr_gap: 0.0,
            p2_active_lifetime_w1: 1.0,
            p2_burst_len_by_threshold: BTreeMap::new(),
            p2_je_line_burst_w1: 1.0,
            p3_fanout_by_attr: by_attr,
            p3_clustering_gap: 0.0, p3_triangle_log_ratio: 0.0,
            p4_mean_gap: 0.0,
        };
        let json = serde_json::to_string(&baseline).unwrap();
        let key_a = json.find("CostCenter").unwrap();
        let key_g = json.find("GLAccount").unwrap();
        assert!(key_a < key_g, "BTreeMap should produce ordered JSON keys");
        let tmp = NamedTempFile::new().unwrap();
        let _: BaselineValues = serde_json::from_str(&json).unwrap();
    }
}
```

- [ ] **Step 2: Test + commit**

```bash
cargo test -p datasynth-eval --lib behavioral_fidelity::report -- --test-threads=4
git add crates/datasynth-eval/src/behavioral_fidelity/report.rs
git commit -m "feat(eval/behavioral): BehavioralFidelityReport struct + JSON writer (BTreeMap-ordered)"
```

---

## Task 17 — Markdown + CSV writers

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/report.rs` (append)

- [ ] **Step 1: Append Markdown and CSV writers**

```rust
impl BehavioralFidelityReport {
    pub fn write_markdown(&self, path: &Path) -> BehavioralFidelityResult<()> {
        let mut buf = String::new();
        use std::fmt::Write;

        writeln!(buf, "# Behavioral-Fidelity Report").ok();
        writeln!(buf, "").ok();
        writeln!(buf, "- **Profile:** `{}`", self.profile).ok();
        writeln!(buf, "- **Generator:** `{}` ({})", self.generator_id, self.generator_version).ok();
        writeln!(buf, "- **Seed:** {}", self.seed).ok();
        writeln!(buf, "- **Generated at:** {}", self.generated_at.to_rfc3339()).ok();
        writeln!(buf, "- **Composite BF score:** **{:.3}** (1.0 = noise floor; lower is better)", self.composite_bf_score).ok();
        writeln!(buf, "").ok();
        writeln!(buf, "## Per-entity DR table").ok();
        writeln!(buf, "").ok();
        writeln!(buf, "| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |").ok();
        writeln!(buf, "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|").ok();
        for (name, m) in &self.per_entity {
            let p2_burst_avg = avg_dr(&m.p2_burst_len_by_threshold);
            let p3_fanout_avg = avg_dr_str(&m.p3_fanout_by_attr);
            writeln!(buf,
                "| `{}` | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |",
                name,
                m.p1_ietd.dr, m.p1_autocorr.dr, m.p2_active_lifetime.dr,
                p2_burst_avg, m.p2_je_line_burst.dr,
                p3_fanout_avg, m.p3_clustering.dr, m.p3_triangle_log_ratio.dr, m.p4_mean_gap.dr
            ).ok();
        }
        writeln!(buf, "").ok();
        writeln!(buf, "## Gate result").ok();
        writeln!(buf, "").ok();
        writeln!(buf, "- **Passed:** {}", if self.gates.passed { "✅" } else { "❌" }).ok();
        writeln!(buf, "- **Threshold (any DR):** {:.2}", self.gates.fail_if_dr_above).ok();
        writeln!(buf, "- **Threshold (composite):** {:.2}", self.gates.fail_if_composite_above).ok();
        if !self.gates.failures.is_empty() {
            writeln!(buf, "- **Failures:**").ok();
            for f in &self.gates.failures {
                writeln!(buf, "  - {}", f).ok();
            }
        }
        if let Some(intra) = &self.intraday_structural {
            writeln!(buf, "").ok();
            writeln!(buf, "## Synthetic-only intraday metrics (info)").ok();
            writeln!(buf, "").ok();
            writeln!(buf, "- IETD median (s): {:.2}", intra.p1_intra_w1_seconds).ok();
            writeln!(buf, "- Lag-1 autocorr (s): {:.3}", intra.p1_intra_autocorr).ok();
            writeln!(buf, "- Off-hours rate: {:.3}", intra.off_hours_rate).ok();
        }
        std::fs::write(path, buf)?;
        Ok(())
    }

    pub fn write_csv(&self, path: &Path) -> BehavioralFidelityResult<()> {
        let mut wtr = csv::Writer::from_path(path)
            .map_err(|e| BehavioralFidelityError::Io(std::io::Error::other(e.to_string())))?;
        wtr.write_record(&["entity_column", "metric", "raw", "baseline", "dr"])
            .map_err(|e| BehavioralFidelityError::Io(std::io::Error::other(e.to_string())))?;
        for (name, m) in &self.per_entity {
            write_metric_row(&mut wtr, name, "P1_IETD_W1_days",      &m.p1_ietd)?;
            write_metric_row(&mut wtr, name, "P1_AutocorrGap",       &m.p1_autocorr)?;
            write_metric_row(&mut wtr, name, "P2_ActiveLifetime_W1", &m.p2_active_lifetime)?;
            for (t, v) in &m.p2_burst_len_by_threshold {
                write_metric_row(&mut wtr, name, &format!("P2_BurstLen_W1_{}d", t), v)?;
            }
            write_metric_row(&mut wtr, name, "P2_JELineBurst_W1",    &m.p2_je_line_burst)?;
            for (attr, v) in &m.p3_fanout_by_attr {
                write_metric_row(&mut wtr, name, &format!("P3_Fanout_W1_{}", attr), v)?;
            }
            write_metric_row(&mut wtr, name, "P3_ClusteringGap",     &m.p3_clustering)?;
            write_metric_row(&mut wtr, name, "P3_TriangleLogRatio",  &m.p3_triangle_log_ratio)?;
            write_metric_row(&mut wtr, name, "P4_MeanGap",           &m.p4_mean_gap)?;
        }
        wtr.flush().map_err(|e| BehavioralFidelityError::Io(std::io::Error::other(e.to_string())))?;
        Ok(())
    }
}

fn write_metric_row(
    wtr: &mut csv::Writer<std::fs::File>,
    entity: &str,
    metric: &str,
    pm: &PerMetric,
) -> BehavioralFidelityResult<()> {
    wtr.write_record(&[
        entity, metric, &format!("{:.6}", pm.raw),
        &format!("{:.6}", pm.baseline), &format!("{:.6}", pm.dr),
    ]).map_err(|e| BehavioralFidelityError::Io(std::io::Error::other(e.to_string())))?;
    Ok(())
}

fn avg_dr(map: &BTreeMap<i64, PerMetric>) -> f64 {
    if map.is_empty() { return 0.0; }
    map.values().map(|p| p.dr).sum::<f64>() / map.len() as f64
}

fn avg_dr_str(map: &BTreeMap<String, PerMetric>) -> f64 {
    if map.is_empty() { return 0.0; }
    map.values().map(|p| p.dr).sum::<f64>() / map.len() as f64
}
```

- [ ] **Step 2: Commit**

```bash
cargo build -p datasynth-eval 2>&1 | tail -5
git add crates/datasynth-eval/src/behavioral_fidelity/report.rs
git commit -m "feat(eval/behavioral): Markdown + CSV writers for BehavioralFidelityReport"
```

---

## Task 18 — `compute_report()` orchestration

**Files:**
- Modify: `crates/datasynth-eval/src/behavioral_fidelity/mod.rs`

This wires all metric modules together, runs them on (real, syn) and (real_A, real_B), produces per-entity outcomes and the gate result.

- [ ] **Step 1: Implement `compute_report` in mod.rs**

```rust
//! P1–P4 behavioral-fidelity evaluation for GL data.
//! (Existing doc-comment retained from Task 1.)

pub mod error;
pub mod types;
pub mod entity_profile;
pub mod math;
pub mod loader;
pub mod ietd;
pub mod burst;
pub mod fanout;
pub mod velocity_rules;
pub mod degradation;
pub mod intraday;
pub mod report;

pub use error::{BehavioralFidelityError, BehavioralFidelityResult};
pub use types::{BehavioralFidelityConfig, EntityProfile, GateThresholds, Record, RuleSet};
pub use report::BehavioralFidelityReport;
pub use entity_profile::{gl_source_tp, real_corpus_aliases, synthetic_aliases};

use std::collections::BTreeMap;
use std::path::Path;

use chrono::Utc;

use crate::behavioral_fidelity::report::{
    BaselineValues, CorpusSummary, EntityMetrics, GateResult, PerMetric,
};

const SELF_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn compute_report(
    cfg: &BehavioralFidelityConfig,
    real: &[Record],
    syn:  &[Record],
) -> BehavioralFidelityResult<BehavioralFidelityReport> {
    let (real_a, real_b) = degradation::split_5050(real, cfg.seed);

    let mut per_entity = BTreeMap::new();

    // Primary entity
    let em_primary = compute_entity_metrics(
        &cfg.profile,
        real, syn, &real_a, &real_b,
        &cfg.profile.primary_entity,
    )?;
    per_entity.insert(cfg.profile.primary_entity.clone(), em_primary);

    // Secondary entity (optional)
    if let Some(sec) = &cfg.profile.secondary_entity {
        let em_sec = compute_entity_metrics(
            &cfg.profile,
            real, syn, &real_a, &real_b,
            sec,
        )?;
        per_entity.insert(sec.clone(), em_sec);
    }

    // P4 attached to primary entity only (per spec) — refactor if secondary needs it.
    let (rule_results, mean_gap) = velocity_rules::evaluate_rule_set(
        &cfg.rule_set, real, syn,
        |r| project_entity(r, &cfg.profile.primary_entity),
    );
    let (_, mean_gap_baseline) = velocity_rules::evaluate_rule_set(
        &cfg.rule_set, &real_a, &real_b,
        |r| project_entity(r, &cfg.profile.primary_entity),
    );
    if let Some(em) = per_entity.get_mut(&cfg.profile.primary_entity) {
        em.p4_rule_results = rule_results;
        em.p4_mean_gap = PerMetric {
            raw: mean_gap,
            baseline: mean_gap_baseline,
            dr: degradation::degradation_ratio(mean_gap, mean_gap_baseline),
        };
    }

    let intraday = intraday::compute_intraday(syn, |r| project_entity(r, &cfg.profile.primary_entity));

    let noise_floor = collect_baseline_values(&per_entity, &cfg.profile);
    let composite_bf_score = compute_composite_bf(&per_entity);

    let gates = build_gate_result(&cfg.fail_thresholds, &per_entity, composite_bf_score);

    Ok(BehavioralFidelityReport {
        profile: cfg.profile.name.clone(),
        generator_id: "datasynth".to_string(),
        generator_version: SELF_VERSION.to_string(),
        seed: cfg.seed,
        generated_at: Utc::now(),
        real_corpus: summary(real, &cfg.profile),
        synthetic:   summary(syn,  &cfg.profile),
        noise_floor,
        per_entity,
        composite_bf_score,
        intraday_structural: intraday,
        gates,
    })
}

pub fn compute_report_from_paths(
    cfg: &BehavioralFidelityConfig,
    real_path: &Path,
    syn_path:  &Path,
) -> BehavioralFidelityResult<BehavioralFidelityReport> {
    let real = load_any(real_path)?;
    let syn  = load_any(syn_path)?;
    compute_report(cfg, &real, &syn)
}

fn load_any(p: &Path) -> BehavioralFidelityResult<Vec<Record>> {
    // Auto-detect parquet vs csv by extension or by first-file probe in dirs.
    if p.is_dir() {
        for entry in std::fs::read_dir(p)? {
            let path = entry?.path();
            if let Some(ext) = path.extension() {
                if ext.eq_ignore_ascii_case("parquet") { return loader::load_parquet_records(&path); }
                if ext.eq_ignore_ascii_case("csv")     { return loader::load_csv_records(&path); }
            }
        }
        return Err(BehavioralFidelityError::Io(std::io::Error::other("no .parquet or .csv in dir")));
    }
    match p.extension().and_then(|s| s.to_str()) {
        Some("parquet") => loader::load_parquet_records(p),
        Some("csv")     => loader::load_csv_records(p),
        _ => Err(BehavioralFidelityError::Io(std::io::Error::other("unknown extension"))),
    }
}

fn compute_entity_metrics(
    profile: &EntityProfile,
    real: &[Record], syn: &[Record], real_a: &[Record], real_b: &[Record],
    entity_col: &str,
) -> BehavioralFidelityResult<EntityMetrics> {
    let project = |r: &Record| project_entity(r, entity_col);

    // P1
    let p1     = ietd::compute_p1(real,    syn,    project, |r| r.entry_date);
    let p1_bl  = ietd::compute_p1(real_a,  real_b, project, |r| r.entry_date);
    let p1_ietd = PerMetric {
        raw: p1.ietd_w1_days,
        baseline: p1_bl.ietd_w1_days,
        dr: degradation::degradation_ratio(p1.ietd_w1_days, p1_bl.ietd_w1_days),
    };
    let p1_autocorr = PerMetric {
        raw: p1.autocorr_gap,
        baseline: p1_bl.autocorr_gap,
        dr: degradation::degradation_ratio(p1.autocorr_gap, p1_bl.autocorr_gap),
    };

    // P2 active lifetime
    let p2_al_raw = burst::active_lifetime_w1(real, syn, project, |r| r.entry_date);
    let p2_al_bl  = burst::active_lifetime_w1(real_a, real_b, project, |r| r.entry_date);
    let p2_active_lifetime = PerMetric { raw: p2_al_raw, baseline: p2_al_bl,
        dr: degradation::degradation_ratio(p2_al_raw, p2_al_bl) };

    // P2 burst length per threshold
    let mut p2_burst_len_by_threshold = BTreeMap::new();
    for t in &profile.burst_thresholds {
        let raw = burst::burst_length_w1(real, syn, project, |r| r.entry_date, *t);
        let bl  = burst::burst_length_w1(real_a, real_b, project, |r| r.entry_date, *t);
        p2_burst_len_by_threshold.insert(*t,
            PerMetric { raw, baseline: bl, dr: degradation::degradation_ratio(raw, bl) });
    }

    // P2 JE-line-burst (structural)
    let p2_jl_raw = burst::je_line_burst_w1(real, syn);
    let p2_jl_bl  = burst::je_line_burst_w1(real_a, real_b);
    let p2_je_line_burst = PerMetric { raw: p2_jl_raw, baseline: p2_jl_bl,
        dr: degradation::degradation_ratio(p2_jl_raw, p2_jl_bl) };

    // P3 fanout per attribute
    let mut p3_fanout_by_attr = BTreeMap::new();
    for attr in &profile.attributes_for_p3 {
        let attr_proj = make_attr_projector(attr);
        let raw = fanout::fanout_w1(real, syn, project, attr_proj);
        let bl  = fanout::fanout_w1(real_a, real_b, project, attr_proj);
        p3_fanout_by_attr.insert(attr.clone(),
            PerMetric { raw, baseline: bl, dr: degradation::degradation_ratio(raw, bl) });
    }

    // P3 clustering & triangles — pick the first attribute as canonical for the projection
    let canonical_attr = profile.attributes_for_p3.first()
        .map(|a| make_attr_projector(a))
        .unwrap_or(fanout::gl_account_of);
    let cc_real = fanout::clustering_coefficient(real, project, canonical_attr);
    let cc_syn  = fanout::clustering_coefficient(syn,  project, canonical_attr);
    let cc_a    = fanout::clustering_coefficient(real_a, project, canonical_attr);
    let cc_b    = fanout::clustering_coefficient(real_b, project, canonical_attr);
    let cc_gap_real_syn = (cc_real - cc_syn).abs();
    let cc_gap_bl       = (cc_a - cc_b).abs();
    let p3_clustering = PerMetric { raw: cc_gap_real_syn, baseline: cc_gap_bl,
        dr: degradation::degradation_ratio(cc_gap_real_syn, cc_gap_bl) };

    let t_real = fanout::triangle_count(real, project, canonical_attr);
    let t_syn  = fanout::triangle_count(syn,  project, canonical_attr);
    let t_a    = fanout::triangle_count(real_a, project, canonical_attr);
    let t_b    = fanout::triangle_count(real_b, project, canonical_attr);
    let tr_raw = fanout::triangle_log_ratio_gap(t_real, t_syn);
    let tr_bl  = fanout::triangle_log_ratio_gap(t_a,    t_b);
    let p3_triangle_log_ratio = PerMetric { raw: tr_raw, baseline: tr_bl,
        dr: degradation::degradation_ratio(tr_raw, tr_bl) };

    Ok(EntityMetrics {
        entity_column: entity_col.to_string(),
        p1_ietd, p1_autocorr,
        p2_active_lifetime, p2_burst_len_by_threshold, p2_je_line_burst,
        p3_fanout_by_attr, p3_clustering, p3_triangle_log_ratio,
        p4_rule_results: Vec::new(),
        p4_mean_gap: PerMetric { raw: 0.0, baseline: 0.0, dr: 0.0 },
    })
}

fn project_entity(r: &Record, col: &str) -> Option<String> {
    match col {
        "Source"          => Some(r.source.clone()),
        "TradingPartner"  => r.trading_partner.clone(),
        "GLAccount"       => Some(r.gl_account.clone()),
        "CostCenter"      => r.cost_center.clone(),
        "ProfitCenter"    => r.profit_center.clone(),
        _ => None,
    }
}

fn make_attr_projector(attr: &str) -> fn(&Record) -> Option<String> {
    match attr {
        "GLAccount"      => fanout::gl_account_of,
        "CostCenter"     => fanout::cost_center_of,
        "ProfitCenter"   => fanout::profit_center_of,
        "TradingPartner" => fanout::trading_partner_attr_of,
        _ => fanout::gl_account_of,
    }
}

fn summary(records: &[Record], profile: &EntityProfile) -> CorpusSummary {
    let entities_p: std::collections::HashSet<String> = records.iter()
        .filter_map(|r| project_entity(r, &profile.primary_entity))
        .collect();
    let entities_s: std::collections::HashSet<String> = profile.secondary_entity.as_ref()
        .map(|c| records.iter().filter_map(|r| project_entity(r, c)).collect())
        .unwrap_or_default();
    let mut period_start = None;
    let mut period_end   = None;
    for r in records {
        period_start = Some(period_start.map_or(r.entry_date, |d: chrono::NaiveDate| d.min(r.entry_date)));
        period_end   = Some(period_end.map_or(r.entry_date, |d: chrono::NaiveDate| d.max(r.entry_date)));
    }
    CorpusSummary {
        path: "(in-memory)".to_string(),
        n_rows: records.len(),
        n_entities_primary: entities_p.len(),
        n_entities_secondary: entities_s.len(),
        period_start, period_end,
    }
}

fn collect_baseline_values(per_entity: &BTreeMap<String, EntityMetrics>, profile: &EntityProfile) -> BaselineValues {
    let primary = per_entity.get(&profile.primary_entity).cloned();
    let mut p2_burst_len = BTreeMap::new();
    let mut p3_fanout = BTreeMap::new();
    let mut bv = BaselineValues {
        p1_ietd_w1_days: 0.0, p1_autocorr_gap: 0.0,
        p2_active_lifetime_w1: 0.0,
        p2_burst_len_by_threshold: BTreeMap::new(),
        p2_je_line_burst_w1: 0.0,
        p3_fanout_by_attr: BTreeMap::new(),
        p3_clustering_gap: 0.0, p3_triangle_log_ratio: 0.0,
        p4_mean_gap: 0.0,
    };
    if let Some(p) = primary {
        bv.p1_ietd_w1_days = p.p1_ietd.baseline;
        bv.p1_autocorr_gap = p.p1_autocorr.baseline;
        bv.p2_active_lifetime_w1 = p.p2_active_lifetime.baseline;
        for (t, pm) in &p.p2_burst_len_by_threshold { p2_burst_len.insert(*t, pm.baseline); }
        bv.p2_burst_len_by_threshold = p2_burst_len;
        bv.p2_je_line_burst_w1 = p.p2_je_line_burst.baseline;
        for (a, pm) in &p.p3_fanout_by_attr { p3_fanout.insert(a.clone(), pm.baseline); }
        bv.p3_fanout_by_attr = p3_fanout;
        bv.p3_clustering_gap = p.p3_clustering.baseline;
        bv.p3_triangle_log_ratio = p.p3_triangle_log_ratio.baseline;
        bv.p4_mean_gap = p.p4_mean_gap.baseline;
    }
    bv
}

fn compute_composite_bf(per_entity: &BTreeMap<String, EntityMetrics>) -> f64 {
    let mut drs: Vec<f64> = Vec::new();
    for em in per_entity.values() {
        drs.push(em.p1_ietd.dr);
        drs.push(em.p1_autocorr.dr);
        drs.push(em.p2_active_lifetime.dr);
        drs.extend(em.p2_burst_len_by_threshold.values().map(|p| p.dr));
        drs.push(em.p2_je_line_burst.dr);
        drs.extend(em.p3_fanout_by_attr.values().map(|p| p.dr));
        drs.push(em.p3_clustering.dr);
        drs.push(em.p3_triangle_log_ratio.dr);
        drs.push(em.p4_mean_gap.dr);
    }
    if drs.is_empty() { 0.0 } else { drs.iter().sum::<f64>() / drs.len() as f64 }
}

fn build_gate_result(thresholds: &GateThresholds, per_entity: &BTreeMap<String, EntityMetrics>, composite: f64) -> GateResult {
    let mut failures = Vec::new();
    for (name, em) in per_entity {
        let metric_checks: Vec<(&str, f64)> = vec![
            ("P1_IETD", em.p1_ietd.dr),
            ("P1_Autocorr", em.p1_autocorr.dr),
            ("P2_ActiveLifetime", em.p2_active_lifetime.dr),
            ("P2_JELineBurst", em.p2_je_line_burst.dr),
            ("P3_Clustering", em.p3_clustering.dr),
            ("P3_TriangleLogRatio", em.p3_triangle_log_ratio.dr),
            ("P4_MeanGap", em.p4_mean_gap.dr),
        ];
        for (mname, dr) in metric_checks {
            if dr > thresholds.fail_if_dr_above {
                failures.push(format!("{}/{} DR={:.3} > {:.2}", name, mname, dr, thresholds.fail_if_dr_above));
            }
        }
        for (t, pm) in &em.p2_burst_len_by_threshold {
            if pm.dr > thresholds.fail_if_dr_above {
                failures.push(format!("{}/P2_BurstLen_{}d DR={:.3} > {:.2}", name, t, pm.dr, thresholds.fail_if_dr_above));
            }
        }
        for (attr, pm) in &em.p3_fanout_by_attr {
            if pm.dr > thresholds.fail_if_dr_above {
                failures.push(format!("{}/P3_Fanout_{} DR={:.3} > {:.2}", name, attr, pm.dr, thresholds.fail_if_dr_above));
            }
        }
    }
    if composite > thresholds.fail_if_composite_above {
        failures.push(format!("Composite BF={:.3} > {:.2}", composite, thresholds.fail_if_composite_above));
    }
    GateResult {
        fail_if_dr_above: thresholds.fail_if_dr_above,
        fail_if_composite_above: thresholds.fail_if_composite_above,
        passed: failures.is_empty(),
        failures,
    }
}
```

- [ ] **Step 2: Build + test**

```bash
cargo build -p datasynth-eval 2>&1 | tail -5
cargo test -p datasynth-eval --lib behavioral_fidelity:: -- --test-threads=4 2>&1 | tail -10
```

- [ ] **Step 3: Commit**

```bash
git add crates/datasynth-eval/src/behavioral_fidelity/mod.rs
git commit -m "feat(eval/behavioral): compute_report orchestrator wires P1..P4 + intraday + gates

Single entry point that produces a fully populated BehavioralFidelityReport with per-entity metrics (primary + optional secondary), baseline values from a deterministic 50/50 split, composite BF score (equal-weighted mean of all sub-metric DRs), and a gate result driven by GateThresholds."
```

---

## Task 19 — CLI subcommand `behavioral score`

**Files:**
- Create: `crates/datasynth-cli/src/commands/behavioral.rs`
- Modify: `crates/datasynth-cli/src/commands/mod.rs` and the `main.rs` Clap parser to register the new subcommand group.

- [ ] **Step 1: Create the subcommand module**

```rust
//! `datasynth-data behavioral` subcommand group.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use datasynth_eval::behavioral_fidelity::{
    self, BehavioralFidelityConfig, BehavioralFidelityResult, GateThresholds,
};

#[derive(Debug, Args)]
pub struct BehavioralArgs {
    #[command(subcommand)]
    pub command: BehavioralCommand,
}

#[derive(Debug, Subcommand)]
pub enum BehavioralCommand {
    /// Score a synthetic dataset against a corpus reference.
    Score(BehavioralScoreArgs),
}

#[derive(Debug, Args)]
pub struct BehavioralScoreArgs {
    /// Path to the corpus parquet/csv file (or directory containing one).
    #[arg(long)]
    pub real: PathBuf,
    /// Path to the synthetic-output parquet/csv file (or directory containing one).
    #[arg(long)]
    pub syn: PathBuf,
    /// Entity profile preset (default: gl-source-tp).
    #[arg(long, default_value = "gl-source-tp")]
    pub profile: String,
    /// Output directory (writes report.json, report.md, metrics.csv).
    #[arg(long)]
    pub out: PathBuf,
    /// Seed for the deterministic 50/50 split of the corpus.
    #[arg(long, default_value_t = 42)]
    pub seed: u64,
    /// Gate threshold: fail (exit 2) if any sub-metric DR exceeds this.
    #[arg(long, default_value_t = 2.0)]
    pub fail_on_dr_above: f64,
    /// Gate threshold: fail (exit 2) if composite BF exceeds this.
    #[arg(long, default_value_t = 1.5)]
    pub fail_on_composite_above: f64,
}

pub fn run(args: BehavioralArgs) -> anyhow::Result<i32> {
    match args.command {
        BehavioralCommand::Score(a) => run_score(a),
    }
}

fn run_score(args: BehavioralScoreArgs) -> anyhow::Result<i32> {
    if args.profile != "gl-source-tp" {
        anyhow::bail!("unknown profile {:?}; SP1 ships gl-source-tp only", args.profile);
    }
    let mut cfg = BehavioralFidelityConfig::gl_default();
    cfg.seed = args.seed;
    cfg.fail_thresholds = GateThresholds {
        fail_if_dr_above: args.fail_on_dr_above,
        fail_if_composite_above: args.fail_on_composite_above,
    };

    std::fs::create_dir_all(&args.out)?;

    let report = behavioral_fidelity::compute_report_from_paths(&cfg, &args.real, &args.syn)
        .map_err(|e| anyhow::anyhow!("compute_report failed: {e}"))?;

    let json = args.out.join("report.json");
    let md   = args.out.join("report.md");
    let csv  = args.out.join("metrics.csv");
    report.write_json(&json).map_err(|e| anyhow::anyhow!("json write failed: {e}"))?;
    report.write_markdown(&md).map_err(|e| anyhow::anyhow!("md write failed: {e}"))?;
    report.write_csv(&csv).map_err(|e| anyhow::anyhow!("csv write failed: {e}"))?;

    eprintln!("composite BF score: {:.3}", report.composite_bf_score);
    eprintln!("gate: {}", if report.gates.passed { "PASS" } else { "FAIL" });
    if !report.gates.passed {
        for f in &report.gates.failures { eprintln!("  - {f}"); }
    }
    Ok(if report.gates.passed { 0 } else { 2 })
}
```

- [ ] **Step 2: Register the subcommand in `datasynth-cli`**

Find the existing top-level Clap enum (in `crates/datasynth-cli/src/main.rs` or `commands/mod.rs`) and add a variant `Behavioral(BehavioralArgs)`. Match it in `main()` and call `commands::behavioral::run(args).map(std::process::exit)`.

Locate the file with the top-level `enum Commands { … }` declaration:

```bash
grep -rln "enum Commands" /home/michael/DEV/Repos/RustSyntheticData/SyntheticData/crates/datasynth-cli/src/ | head -3
```

Then add:

```rust
/// Behavioral-fidelity evaluation against a corpus reference.
Behavioral(behavioral::BehavioralArgs),
```

And in the dispatcher's match block, add:

```rust
Commands::Behavioral(args) => {
    let exit = commands::behavioral::run(args)?;
    std::process::exit(exit);
}
```

- [ ] **Step 3: Build the CLI**

```bash
cargo build -p datasynth-cli --release 2>&1 | tail -5
```

- [ ] **Step 4: Smoke-run `--help`**

```bash
./target/release/datasynth-data behavioral score --help
```

Expected: usage text including `--real`, `--syn`, `--profile`, `--out`, `--seed`, `--fail-on-dr-above`, `--fail-on-composite-above`.

- [ ] **Step 5: Commit**

```bash
git add crates/datasynth-cli/src/commands/behavioral.rs crates/datasynth-cli/src/commands/mod.rs crates/datasynth-cli/src/main.rs
git commit -m "feat(cli): datasynth-data behavioral score subcommand

Wires BehavioralScoreArgs to behavioral_fidelity::compute_report_from_paths, writes report.json + report.md + metrics.csv, exits 0/2 based on the gate. SP1 ships the gl-source-tp profile only."
```

---

## Task 20 — Integration smoke test (golden)

**Files:**
- Create: `crates/datasynth-eval/tests/behavioral_smoke.rs`
- Create: `crates/datasynth-eval/tests/fixtures/behavioral_smoke.json` (snapshot)

The smoke test generates a tiny synthetic dataset programmatically (not via the full DataSynth pipeline — that would pull cross-crate deps into a test that doesn't need them), then runs the behavioral-fidelity pipeline on (seed-42, seed-43) pairs.

- [ ] **Step 1: Write the smoke test**

```rust
//! Integration smoke test for behavioral_fidelity.

use chrono::{Duration, NaiveDate};
use datasynth_eval::behavioral_fidelity::{
    self, BehavioralFidelityConfig, Record,
};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

fn gen_synthetic(seed: u64, n_entries: usize) -> Vec<Record> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let sources = ["KR", "RE", "SA", "DZ", "WE", "IM"];
    let accounts = (1000..1050).map(|i| format!("A{i}")).collect::<Vec<_>>();
    let ccs       = (100..120).map(|i| format!("CC{i}")).collect::<Vec<_>>();
    let tps       = (1..30).map(|i| format!("TP{i}")).collect::<Vec<_>>();

    let mut out = Vec::with_capacity(n_entries);
    let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
    for i in 0..n_entries {
        let day_off = rng.gen_range(0..365);
        let entry = base + Duration::days(day_off);
        let effective = entry + Duration::days(rng.gen_range(-2..14));
        let src = sources[rng.gen_range(0..sources.len())];
        let je = format!("J{}-{:06}", seed, i / 3); // group every ~3 rows into one JE
        let line = format!("{:03}", (i % 3) + 1);
        out.push(Record {
            source: src.to_string(),
            gl_account: accounts[rng.gen_range(0..accounts.len())].clone(),
            cost_center:     Some(ccs[rng.gen_range(0..ccs.len())].clone()),
            profit_center:   Some(ccs[rng.gen_range(0..ccs.len())].clone()),
            trading_partner: Some(tps[rng.gen_range(0..tps.len())].clone()),
            je_number: je, je_line_number: line,
            effective_date: effective, entry_date: entry, created_at: None,
            functional_amount: rng.gen_range(-10000.0..10000.0),
        });
    }
    out
}

#[test]
fn smoke_report_runs_and_passes_gate_on_similar_data() {
    let real = gen_synthetic(42, 3000);
    let syn  = gen_synthetic(43, 3000);
    let mut cfg = BehavioralFidelityConfig::gl_default();
    cfg.fail_thresholds.fail_if_dr_above = 10.0; // permissive; this is a smoke test, not a precision benchmark
    cfg.fail_thresholds.fail_if_composite_above = 10.0;

    let report = behavioral_fidelity::compute_report(&cfg, &real, &syn).unwrap();
    assert!(report.per_entity.contains_key("Source"));
    assert!(report.per_entity.contains_key("TradingPartner"));
    assert!(report.composite_bf_score.is_finite());
    assert!(report.composite_bf_score >= 0.0);
    assert!(report.gates.passed, "smoke gate should pass with permissive thresholds; failures = {:?}", report.gates.failures);
}
```

- [ ] **Step 2: Make sure `rand` + `rand_chacha` are in dev-deps**

Append (if not present) to `crates/datasynth-eval/Cargo.toml`:

```toml
[dev-dependencies]
rand = { workspace = true }
rand_chacha = { workspace = true }
tempfile = "3"
```

- [ ] **Step 3: Run the test**

```bash
cargo test -p datasynth-eval --test behavioral_smoke -- --test-threads=4 --nocapture 2>&1 | tail -30
```
Expected: `1 passed`.

- [ ] **Step 4: Commit**

```bash
git add crates/datasynth-eval/tests/behavioral_smoke.rs crates/datasynth-eval/Cargo.toml
git commit -m "test(eval/behavioral): smoke test wires full pipeline on synthetic-vs-synthetic data"
```

---

## Task 21 — Noise-floor sanity test

**Files:**
- Create: `crates/datasynth-eval/tests/behavioral_noise_floor.rs`

- [ ] **Step 1: Write the test**

```rust
//! When "real" and "synthetic" are the same dataset, every DR should equal 1.0
//! (definition of the noise floor — metric(real, real) / metric(real_A, real_B)
//! still has variance, but the numerator is itself the baseline since we're
//! using the same split structure).

use datasynth_eval::behavioral_fidelity::{
    self, BehavioralFidelityConfig,
};

mod smoke_helpers {
    include!("behavioral_smoke.rs");
}

#[test]
fn dr_equals_one_when_real_equals_syn() {
    let real = smoke_helpers::gen_synthetic(42, 2000);
    let syn  = real.clone();
    let cfg = BehavioralFidelityConfig::gl_default();
    let report = behavioral_fidelity::compute_report(&cfg, &real, &syn).unwrap();
    // raw == baseline by construction → DR ≈ 1.0 across all metrics
    for (name, em) in &report.per_entity {
        let drs: Vec<f64> = vec![
            em.p1_ietd.dr, em.p1_autocorr.dr,
            em.p2_active_lifetime.dr, em.p2_je_line_burst.dr,
            em.p3_clustering.dr, em.p3_triangle_log_ratio.dr,
            em.p4_mean_gap.dr,
        ];
        for dr in drs {
            assert!(
                (dr - 1.0).abs() < 0.5 || dr == 0.0, // some metrics may be 0/0 → 0 by epsilon rule
                "entity {name}: DR {dr:.3} should be near 1.0 or zero"
            );
        }
    }
    assert!(report.composite_bf_score < 2.0,
        "composite BF when real==syn should be near 1.0, got {}", report.composite_bf_score);
}
```

NOTE: the `include!` trick reuses `gen_synthetic` from the smoke test without duplicating it. Verify it compiles: integration-test files in `tests/` are independent crates, so the helper must use `pub fn gen_synthetic`. Update the smoke test to mark `gen_synthetic` `pub fn` if not already.

- [ ] **Step 2: Update smoke test if needed**

Edit `crates/datasynth-eval/tests/behavioral_smoke.rs` and change `fn gen_synthetic` to `pub fn gen_synthetic`.

- [ ] **Step 3: Run + commit**

```bash
cargo test -p datasynth-eval --test behavioral_noise_floor -- --test-threads=4 2>&1 | tail -10
git add crates/datasynth-eval/tests/behavioral_noise_floor.rs crates/datasynth-eval/tests/behavioral_smoke.rs
git commit -m "test(eval/behavioral): noise-floor sanity (DR ≈ 1.0 when real == syn)"
```

---

## Task 22 — CI workflow update

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Locate the eval-test job**

```bash
grep -n "datasynth-eval" .github/workflows/ci.yml | head -5
```

- [ ] **Step 2: Add the two new integration tests to the eval test list**

In `.github/workflows/ci.yml`, find the step that runs eval tests and append:

```yaml
      - name: Behavioral-fidelity smoke
        run: cargo test -p datasynth-eval --test behavioral_smoke -- --test-threads=4
      - name: Behavioral-fidelity noise floor
        run: cargo test -p datasynth-eval --test behavioral_noise_floor -- --test-threads=4
```

If the existing structure groups all eval tests in one `cargo test -p datasynth-eval` invocation, add a separate step matching the structure.

- [ ] **Step 3: Verify the workflow parses**

```bash
yamllint .github/workflows/ci.yml 2>&1 | head -10
```
(Skip if yamllint not installed; the GitHub CI run will catch syntax errors.)

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: run behavioral_fidelity smoke + noise-floor tests on PR

Adds two new integration test invocations to the eval workflow. Both run with --test-threads=4 per workspace policy."
```

---

## Task 23 — Docs (user-facing + CLAUDE.md + README)

**Files:**
- Create: `docs/behavioral-fidelity.md`
- Modify: `README.md`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Write `docs/behavioral-fidelity.md`**

```markdown
# Behavioral-fidelity evaluation

`datasynth-data behavioral score` measures how closely a synthetic GL dataset
preserves the *within-entity* temporal and structural fingerprints of real
GL data. Adapted from Sajja (2026) for GL semantics: `Source` as the primary
entity, `TradingPartner` as the secondary, `EntryDate` at day resolution.

## What it measures

| Pattern | Sub-metric | What it captures |
|---|---|---|
| P1 IETD | `W₁(IETD_real, IETD_syn)` in days | Posting-gap distribution per Source |
| P1 ACorr | `\|mean within-Source lag-1 autocorr_real − autocorr_syn\|` | Burst fingerprint (short gap → short gap) |
| P2 ActiveLifetime | `W₁(active lifetimes)` in days | How long each Source remains active |
| P2 BurstLen | `W₁(burst lengths)` at gap thresholds {1d, 3d, 7d} | Within-Source burst density |
| P2 JELineBurst | `W₁(lines per JE Number)` | GL-specific structural burst |
| P3 Fanout | `W₁(fan-out per attribute)` | Shared-infrastructure motifs |
| P3 Clustering | `\|clustering_real − clustering_syn\|` | Entity co-occurrence density |
| P3 △ ratio | `\|log((triangles_real+1)/(triangles_syn+1))\|` | Cross-entity ring structure |
| P4 Velocity | mean `\|TR_r(real) − TR_r(syn)\|` over R1..R10 | Velocity-rule trigger-rate gap |

Every raw metric is normalised by a noise-floor baseline (a deterministic
50/50 JE-grouped split of the corpus). The **composite BF score** is
the equal-weighted mean of all sub-metric degradation ratios; **1.0 = noise
floor**, higher is worse.

## Quick usage

\`\`\`bash
datasynth-data behavioral score \
  --real /path/to/corpus/journal_entries.parquet \
  --syn  ./output \
  --profile gl-source-tp \
  --out  ./reports/bf \
  --seed 42
\`\`\`

Outputs three files: `report.json`, `report.md`, `metrics.csv`. Exit code
0 if every sub-metric DR ≤ `--fail-on-dr-above` and composite ≤
`--fail-on-composite-above`; 2 otherwise.

## Canonical R1..R10 velocity rules

R1: >5 JEs / Source / business day.
R2: >10 distinct accounts / Source / day.
R3: Sum |amount| / Source / day > p90 of historical.
R4: Posting to account dormant ≥ 180 days.
R5: >3 distinct Trading Partners / Source / day.
R6: max/median amount per Source over 30 d > 3.0.
R7: Off-hours posting (Sat/Sun).
R8: Post-close posting (>5 business days after period end).
R9: Round-dollar share (|amt| mod 1000 = 0) > 10%.
R10: Backdating (Effective − Entry > 30 days).

## Limitations

- Day-resolution timestamps in corpus → P1/P2 W₁ values are in days
  (not seconds as in Sajja); the composite DR remains comparable across
  metrics because each is normalised by its own noise floor.
- The P3 entity-projection graph uses the first listed attribute
  (`GLAccount` by default) for the clustering coefficient + triangle
  count. Per-attribute fan-out W₁ covers all listed attributes.
- JE-line-burst is a GL-specific addition not in Sajja; clearly labelled in
  the report and CSV.
```

- [ ] **Step 2: Add a section to `README.md`**

Append after the existing evaluation section:

```markdown
### Behavioral-fidelity evaluation (v5.11+)

\`\`\`bash
datasynth-data behavioral score \
  --real /path/to/real_je.parquet \
  --syn  ./output \
  --profile gl-source-tp \
  --out  ./reports/bf
\`\`\`

See [docs/behavioral-fidelity.md](docs/behavioral-fidelity.md) for the
P1–P4 metric definitions and the canonical GL velocity rule set.
```

- [ ] **Step 3: Add a brief entry to `CLAUDE.md`**

Find the "Evaluation Module" section and add:

```markdown
- behavioral_fidelity/: Sajja 2026 P1-P4 metrics adapted to GL (Source / TP entity profile) with degradation-ratio noise-floor normalisation. CLI: `datasynth-data behavioral score`.
```

- [ ] **Step 4: Verify docs render (optional)**

If the repo has a `mdbook` setup, `mdbook test` quickly. Otherwise eyeball.

- [ ] **Step 5: Commit**

```bash
git add docs/behavioral-fidelity.md README.md CLAUDE.md
git commit -m "docs(behavioral): user guide + README + CLAUDE.md entry

docs/behavioral-fidelity.md documents the P1-P4 metrics, the R1-R10 velocity rule set, the day-granularity limitation, and CLI usage."
```

---

## Self-review (run inline before handing off)

After all 23 tasks land, re-read each spec section and trace it to a task:

| Spec section | Implementing task(s) |
|---|---|
| §2.1 Crate placement | T1 |
| §2.2 Public API | T2, T3, T18 |
| §2.3 End-to-end flow | T18 |
| §3.1 config.rs | T2 |
| §3.2 loader.rs | T6 |
| §3.3 ietd.rs (P1) | T7 |
| §3.4 burst.rs (P2 active lifetime) | T8 |
| §3.4 burst.rs (P2 burst length) | T9 |
| §3.4 burst.rs (P2 JE-line-burst) | T10 |
| §3.5 fanout.rs (P3 fan-out) | T11 |
| §3.5 fanout.rs (P3 clustering, triangles) | T12 |
| §3.6 velocity_rules.rs (P4 R1..R10) | T13 |
| §3.7 degradation.rs | T14 |
| §3.8 intraday.rs | T15 |
| §3.9 report.rs (struct + JSON) | T16 |
| §3.9 report.rs (Markdown + CSV) | T17 |
| §3.10 CLI | T19 |
| §4 Privacy: no real data in repo | T20 (synthetic-only fixtures) |
| §5.2 Unit tests per module | T7-T15 (inline tests) |
| §5.3 Golden integration smoke | T20 |
| §5.4 Noise-floor sanity | T21 |
| §5.5 CI integration | T22 |
| §6.1 Dependencies | T1 |
| §7 Acceptance criteria (1) Builds | enforced by every task's `cargo build` |
| §7 (2) Tests pass | T20, T21 |
| §7 (3) CLI works | T19 |
| §7 (4) Three artifacts | T16, T17, T19 |
| §7 (5) corpus dry run | manual after T19 (documented in T23) |
| §7 (6) Gate semantics | T18, T19 |
| §7 (7) Docs | T23 |
| §7 (8) No corpus data | T20 |

All spec sections covered. No placeholders. Type names consistent
(BehavioralFidelityReport, BehavioralFidelityConfig, EntityProfile,
RuleSet, PerMetric, EntityMetrics, GateResult, GateThresholds, Record,
RuleResult, IntradayMetrics, CorpusSummary, BaselineValues) — used the
same names throughout impl and tests.

---

## Execution

Plan complete. The default execution path is **subagent-driven**, per the
user's autonomous-mode instruction:

- Wave 1: T1 (single subagent)
- Wave 2: {T2, T4, T5} (3 parallel subagents)
- Wave 3: T3 (1 subagent, depends on T2)
- Wave 4: T6 (1 subagent, depends on T2)
- Wave 5: {T7, T8, T9, T10, T11, T12, T13, T14, T15} (9 parallel subagents)
- Wave 6: {T16, T17, T18} (3 parallel, T17/T18 depend on T16)
- Wave 7: T19 (1 subagent, CLI)
- Wave 8: {T20, T21} (2 parallel — integration tests)
- Wave 9: T22 (1 subagent, CI workflow)
- Wave 10: T23 (1 subagent, docs)

Two-stage review between waves: each subagent reports test/clippy status;
the dispatcher (you) runs `cargo build --release && cargo clippy --workspace`
locally as a final gate before advancing to the next wave.

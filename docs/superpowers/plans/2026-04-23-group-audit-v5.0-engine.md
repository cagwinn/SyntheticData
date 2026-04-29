# Group Audit v5.0 — Engine Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the `datasynth-group` crate with manifest / shard / aggregate phases, `group:` YAML schema, cross-entity intercompany matching, IAS 21 group translation + CTA, NCI rollforward, and consolidated financial statements — delivering real group simulation for up to ~50 entities with zero breaking changes to existing single-entity workflows.

**Architecture:** New `datasynth-group` crate at `crates/datasynth-group/`, layered above the unchanged `datasynth-runtime::EnhancedOrchestrator`. Three phases (manifest → shard → aggregate) each invocable via a new `datasynth-data group` CLI subcommand. Manifest carries allocations (ownership graph, IC relationships, FX rates, seeds); shards derive instances deterministically; aggregate joins on `ic_pair_id`. Per-entity archive sub-trees under `entities/{code}/` preserve the existing per-entity output shape byte-for-byte.

**Tech Stack:** Rust workspace, `serde_yaml`, `serde_json`, `blake3` for deterministic seed derivation, `rust_decimal`, `chrono`, `thiserror`, `tracing`, `rayon` for in-process parallelism. All workspace deps already present.

**Spec:** [`docs/superpowers/specs/2026-04-23-group-audit-simulation-design.md`](../specs/2026-04-23-group-audit-simulation-design.md)

**Plan structure:** Chunk 1 is fully detailed TDD (scaffolding + config types — the shape of the crate everything else depends on). Chunks 2–12 use task-level breakdowns with file paths and acceptance criteria; detailed TDD steps are added to each chunk when execution reaches it, to avoid lock-in on implementation details that later chunks might revise. This mirrors the successful v1.3.0 plan structure.

**Test concurrency note:** Always use `--test-threads=4` or lower (system resource constraint — see `memory/feedback_test_concurrency.md`). The CI workflow pins this.

---

## Chunk 1 — Crate scaffolding & configuration types

Goal: land a new `datasynth-group` crate in the workspace with the full `group:` config surface deserializing from YAML, the three-level inheritance resolver, and tests demonstrating the spec's Mini-Nestlé config parses cleanly. After this chunk `cargo test -p datasynth-group` passes and the crate exposes `GroupConfig` + `ResolvedEntity` + `ResolvedScopingProfile` as its public surface.

### Task 1.1 — Create `datasynth-group` crate

**Files:**
- Create: `crates/datasynth-group/Cargo.toml`
- Create: `crates/datasynth-group/src/lib.rs`
- Modify: `Cargo.toml` (workspace root — add to `members`)

- [ ] **Step 1: Verify workspace structure**

Run: `ls crates/ | head -20`
Expected: list includes `datasynth-core`, `datasynth-runtime`, etc.

- [ ] **Step 2: Create Cargo.toml**

Create `crates/datasynth-group/Cargo.toml`:

```toml
[package]
name = "datasynth-group"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true

[dependencies]
datasynth-core = { workspace = true }
datasynth-config = { workspace = true }
datasynth-generators = { workspace = true }
datasynth-runtime = { workspace = true }
datasynth-standards = { workspace = true }
datasynth-output = { workspace = true }
datasynth-audit-fsm = { workspace = true }

serde = { workspace = true }
serde_json = { workspace = true }
serde_yaml = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
chrono = { workspace = true }
rust_decimal = { workspace = true }
rand = { workspace = true }
rand_chacha = { workspace = true }
blake3 = "1"

[dev-dependencies]
datasynth-test-utils = { workspace = true }
rust_decimal_macros = { workspace = true }
pretty_assertions = "1"
```

- [ ] **Step 3: Add `blake3` to workspace dependencies if not present**

Check `Cargo.toml` workspace `[workspace.dependencies]`. If `blake3` is missing, add:

```toml
blake3 = "1"
```

Then change the crate's Cargo.toml to use `blake3 = { workspace = true }`.

- [ ] **Step 4: Create src/lib.rs stub**

Create `crates/datasynth-group/src/lib.rs`:

```rust
//! DataSynth group audit simulation engine.
//!
//! Manifest / shard / aggregate three-phase model layered above
//! [`datasynth_runtime::EnhancedOrchestrator`]. See
//! `docs/superpowers/specs/2026-04-23-group-audit-simulation-design.md`.

pub mod config;
pub mod errors;

pub use config::GroupConfig;
pub use errors::{GroupError, GroupResult};
```

- [ ] **Step 5: Create src/errors.rs stub**

```rust
//! Error types for the group engine.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GroupError {
    #[error("config error: {0}")]
    Config(String),
    #[error("manifest error: {0}")]
    Manifest(String),
    #[error("shard error: {0}")]
    Shard(String),
    #[error("aggregate error: {0}")]
    Aggregate(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(String),
}

impl From<serde_yaml::Error> for GroupError {
    fn from(e: serde_yaml::Error) -> Self {
        Self::Serde(format!("yaml: {e}"))
    }
}

impl From<serde_json::Error> for GroupError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serde(format!("json: {e}"))
    }
}

pub type GroupResult<T> = Result<T, GroupError>;
```

- [ ] **Step 6: Create src/config.rs stub**

```rust
//! `group:` YAML configuration types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupConfig {
    pub id: String,
}
```

(Fleshed out in Task 1.2.)

- [ ] **Step 7: Add crate to workspace members**

Edit root `Cargo.toml`, find the `[workspace] members = [ ... ]` list, and add `"crates/datasynth-group"`.

- [ ] **Step 8: Verify compilation**

Run: `cargo check -p datasynth-group`
Expected: Compiles with no errors.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock crates/datasynth-group/
git commit -m "feat(group): scaffold datasynth-group crate

New crate for the v5.0 group engine: manifest / shard / aggregate
phases. This commit just lands the crate skeleton and workspace wiring."
```

---

### Task 1.2 — Define `GroupConfig` top-level structure

**Files:**
- Modify: `crates/datasynth-group/src/config.rs`
- Create: `crates/datasynth-group/tests/config_parse.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/datasynth-group/tests/config_parse.rs`:

```rust
//! Spec §3.1 — `group:` config YAML parses into GroupConfig.

use datasynth_group::GroupConfig;

#[test]
fn test_mini_nestle_parses() {
    let yaml = include_str!("fixtures/mini_nestle_minimal.yaml");
    let cfg: GroupConfig = serde_yaml::from_str(yaml)
        .expect("mini_nestle fixture must parse");

    assert_eq!(cfg.id, "MINI_NESTLE_2024_Q1");
    assert_eq!(cfg.presentation_currency, "CHF");
    assert_eq!(cfg.ownership.parent_entity_code, "NESTLE_SA");
    assert!(!cfg.ownership.entities.is_empty());
}
```

Create fixture `crates/datasynth-group/tests/fixtures/mini_nestle_minimal.yaml`:

```yaml
id: "MINI_NESTLE_2024_Q1"
name: "Mini Nestlé Reference Group"
presentation_currency: "CHF"
period:
  start_date: "2024-01-01"
  length: quarterly
  fiscal_year_end: "2024-12-31"
seed: 0x1234567890ABCDEF

ownership:
  parent_entity_code: NESTLE_SA
  entities:
    - code: NESTLE_SA
      country: CH
      functional_currency: CHF
      scoping_profile: significant
      consolidation_method: parent
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p datasynth-group --test config_parse`
Expected: FAIL — `GroupConfig` has no such fields.

- [ ] **Step 3: Flesh out `GroupConfig` types in `src/config.rs`**

Replace the stub with the full shape (matches spec §3.1 + §15):

```rust
//! `group:` YAML configuration types (spec §3).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level group engagement config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupConfig {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub presentation_currency: String,
    pub period: PeriodConfig,
    pub seed: u64,

    #[serde(default)]
    pub defaults: serde_yaml::Value, // inherited into each entity; opaque at this layer

    #[serde(default)]
    pub scoping_profiles: BTreeMap<String, serde_yaml::Value>,

    pub ownership: OwnershipConfig,

    #[serde(default)]
    pub intercompany: IntercompanyConfig,

    pub fx: FxConfig,

    #[serde(default)]
    pub audit: AuditEngagementConfig,

    #[serde(default)]
    pub tax: TaxGroupConfig,

    #[serde(default)]
    pub output: OutputLayoutConfig,

    #[serde(default)]
    pub fleet: Option<FleetConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeriodConfig {
    pub start_date: NaiveDate,
    pub length: PeriodLength,
    #[serde(default)]
    pub fiscal_year_end: Option<NaiveDate>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PeriodLength {
    Monthly,
    Quarterly,
    SemiAnnual,
    Annual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnershipConfig {
    pub parent_entity_code: String,
    #[serde(default)]
    pub entities: Vec<EntityConfig>,
    #[serde(default)]
    pub generated: Vec<GeneratedEntityBlock>,
    #[serde(default)]
    pub entities_from: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityConfig {
    pub code: String,
    #[serde(default)]
    pub name: Option<String>,
    pub country: String,
    pub functional_currency: String,
    pub scoping_profile: String,
    pub consolidation_method: ConsolidationMethod,
    #[serde(default)]
    pub ownership_percent: Option<Decimal>,
    #[serde(default)]
    pub parent_code: Option<String>,
    #[serde(default)]
    pub acquisition_date: Option<NaiveDate>,
    #[serde(default)]
    pub accounting_framework: Option<String>,
    #[serde(default)]
    pub industry: Option<String>,
    #[serde(default)]
    pub rows: Option<u64>,
    #[serde(default, flatten)]
    pub overrides: BTreeMap<String, serde_yaml::Value>, // generic per-entity overrides
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationMethod {
    Parent,
    Full,
    EquityMethod,
    Proportional,
    FairValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedEntityBlock {
    pub count: u32,
    pub code_prefix: String,
    #[serde(default)]
    pub country: Vec<String>,
    #[serde(default)]
    pub functional_currency: Option<String>,
    pub scoping_profile: String,
    pub consolidation_method: ConsolidationMethod,
    #[serde(default)]
    pub ownership_percent_range: Option<[Decimal; 2]>,
    #[serde(default)]
    pub parent_code: Option<String>,
    #[serde(default)]
    pub accounting_framework: Option<String>,
    #[serde(default)]
    pub industry: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntercompanyConfig {
    #[serde(default)]
    pub relationships: Vec<IcRelationshipConfig>,
    #[serde(default)]
    pub matching: IcMatchingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IcRelationshipConfig {
    Explicit(IcRelationshipExplicit),
    Pattern(IcRelationshipPattern),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcRelationshipExplicit {
    pub seller: String,
    pub buyer: String,
    pub types: Vec<IcTransactionType>,
    pub annual_volume: Decimal,
    #[serde(default)]
    pub transfer_pricing: Option<TransferPricingMethod>,
    #[serde(default)]
    pub markup_percent: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcRelationshipPattern {
    pub pattern: IcPattern,
    pub types: Vec<IcTransactionType>,
    pub per_pair_volume: Decimal,
    #[serde(default)]
    pub transfer_pricing: Option<TransferPricingMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcPattern {
    #[serde(default)]
    pub seller_scoping_profile: Option<String>,
    #[serde(default)]
    pub buyer_scoping_profile: Option<String>,
    #[serde(default)]
    pub seller: Option<String>,
    #[serde(default)]
    pub buyer: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IcTransactionType {
    GoodsSale,
    ServiceProvided,
    ManagementFee,
    Royalty,
    CostSharing,
    LoanInterest,
    Dividend,
    ExpenseRecharge,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferPricingMethod {
    CostPlus,
    ComparableUncontrolled,
    ResalePrice,
    TransactionalNetMargin,
    ProfitSplit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcMatchingConfig {
    #[serde(default = "default_strategy")]
    pub strategy: IcMatchingStrategy,
    #[serde(default = "default_coverage_target")]
    pub coverage_target: f64,
}

fn default_strategy() -> IcMatchingStrategy { IcMatchingStrategy::ManifestDriven }
fn default_coverage_target() -> f64 { 0.98 }

impl Default for IcMatchingConfig {
    fn default() -> Self {
        Self { strategy: default_strategy(), coverage_target: default_coverage_target() }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IcMatchingStrategy {
    ManifestDriven,
    EmergentFuzzy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxConfig {
    pub base_currency: String,
    pub rate_source: FxRateSource,
    #[serde(default)]
    pub rates: BTreeMap<String, BTreeMap<NaiveDate, Decimal>>,
    pub policy: FxPolicyConfig,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FxRateSource {
    Inline,
    UserSupplied,
    HistoricalSeries,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxPolicyConfig {
    pub balance_sheet: FxRateBasis,
    pub income_statement: FxRateBasis,
    pub equity: FxRateBasis,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FxRateBasis {
    Closing,
    Average,
    Historical,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditEngagementConfig {
    #[serde(default)]
    pub engagement_id: Option<String>,
    #[serde(default)]
    pub lead_auditor: Option<String>,
    #[serde(default)]
    pub framework: Option<String>, // isa | pcaob | dual
    #[serde(default)]
    pub fsm_blueprint: Option<String>,
    #[serde(default)]
    pub group_materiality: Option<GroupMaterialityConfig>,
    #[serde(default)]
    pub component_scope_thresholds: Option<ComponentScopeThresholds>,
    #[serde(default)]
    pub generate_kams: bool,
    #[serde(default)]
    pub generate_group_opinion: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMaterialityConfig {
    pub basis: MaterialityBasis,
    pub percent: Decimal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MaterialityBasis {
    Revenue,
    Assets,
    PretaxIncome,
    Equity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentScopeThresholds {
    pub full_scope: Decimal,
    pub specific_scope: Decimal,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaxGroupConfig {
    #[serde(default)]
    pub pillar_two: Option<PillarTwoConfig>,
    #[serde(default)]
    pub cbc_report: Option<CbcReportConfig>,
    #[serde(default)]
    pub transfer_pricing: Option<TpConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PillarTwoConfig {
    pub enabled: bool,
    #[serde(default)]
    pub jurisdictions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CbcReportConfig {
    pub enabled: bool,
    #[serde(default)]
    pub reporting_jurisdiction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TpConfig {
    #[serde(default)]
    pub master_file: bool,
    #[serde(default)]
    pub local_files_for: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutputLayoutConfig {
    #[serde(default = "default_layout")]
    pub layout: OutputLayout,
    #[serde(default = "default_true")]
    pub shared_masters_at_root: bool,
    #[serde(default)]
    pub compression: Option<OutputCompression>,
}

fn default_layout() -> OutputLayout { OutputLayout::PerEntitySubtree }
fn default_true() -> bool { true }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputLayout {
    PerEntitySubtree,
    Flat,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputCompression {
    Json,
    Csv,
    Parquet,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FleetConfig {
    #[serde(default)]
    pub dispatcher: Option<String>, // v5.3: in_process | subprocess | remote
    #[serde(default)]
    pub max_concurrent_shards: Option<u32>,
    #[serde(default)]
    pub per_shard_timeout_seconds: Option<u64>,
}
```

Then update `src/lib.rs` to re-export key types:

```rust
pub use config::{
    AuditEngagementConfig, ComponentScopeThresholds, ConsolidationMethod, EntityConfig,
    FxConfig, FxPolicyConfig, FxRateBasis, FxRateSource, GeneratedEntityBlock,
    GroupConfig, GroupMaterialityConfig, IcMatchingConfig, IcMatchingStrategy,
    IcPattern, IcRelationshipConfig, IcRelationshipExplicit, IcRelationshipPattern,
    IcTransactionType, IntercompanyConfig, MaterialityBasis, OutputLayout,
    OutputLayoutConfig, OwnershipConfig, PeriodConfig, PeriodLength, PillarTwoConfig,
    TaxGroupConfig, TpConfig, TransferPricingMethod,
};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p datasynth-group --test config_parse test_mini_nestle_parses`
Expected: PASS.

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p datasynth-group -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/datasynth-group/src/config.rs \
       crates/datasynth-group/src/lib.rs \
       crates/datasynth-group/tests/config_parse.rs \
       crates/datasynth-group/tests/fixtures/mini_nestle_minimal.yaml
git commit -m "feat(group): define GroupConfig types matching spec §3

Full YAML surface for group: section, including ownership, IC
relationships (explicit + pattern), FX policy, audit engagement
plan, tax group plan, and output layout. Minimal fixture parses."
```

---

### Task 1.3 — Full Mini-Nestlé config parses (spec §15)

**Files:**
- Modify: `crates/datasynth-group/tests/config_parse.rs`
- Create: `crates/datasynth-group/tests/fixtures/mini_nestle.yaml`

- [ ] **Step 1: Copy Mini-Nestlé config from spec**

Create `crates/datasynth-group/tests/fixtures/mini_nestle.yaml` with the full appendix A config from the spec (spec §15, Mini-Nestlé reference config — copy verbatim, including `defaults`, `scoping_profiles`, `ownership`, `intercompany`, `fx`, `audit`, `tax`, `output`).

- [ ] **Step 2: Add test for the full fixture**

Append to `crates/datasynth-group/tests/config_parse.rs`:

```rust
#[test]
fn test_full_mini_nestle_parses() {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let cfg: datasynth_group::GroupConfig = serde_yaml::from_str(yaml)
        .expect("full mini_nestle must parse");

    // Spot-check all sections are present.
    assert_eq!(cfg.id, "MINI_NESTLE_2024_Q1");
    assert_eq!(cfg.scoping_profiles.len(), 2, "significant + material");
    assert_eq!(cfg.ownership.entities.len(), 5, "parent + 4 subsidiaries/JV");

    // Verify consolidation methods exercised.
    use datasynth_group::ConsolidationMethod::*;
    let methods: std::collections::BTreeSet<_> = cfg.ownership.entities.iter()
        .map(|e| e.consolidation_method).collect();
    assert!(methods.contains(&Parent));
    assert!(methods.contains(&Full));
    assert!(methods.contains(&EquityMethod));

    // Multi-GAAP entity present.
    assert!(cfg.ownership.entities.iter()
        .any(|e| e.accounting_framework.as_deref() == Some("us_gaap")));

    // NCI-bearing entity (80% owned).
    assert!(cfg.ownership.entities.iter()
        .any(|e| e.ownership_percent == Some(rust_decimal_macros::dec!(0.80))));

    // IC relationships: explicit + pattern
    assert!(cfg.intercompany.relationships.len() >= 3);
    assert!(matches!(cfg.intercompany.matching.strategy,
        datasynth_group::IcMatchingStrategy::ManifestDriven));

    // FX rates for 3 pairs
    assert_eq!(cfg.fx.rates.len(), 3);

    // Audit + tax both configured
    assert_eq!(cfg.audit.engagement_id.as_deref(), Some("EY_MINI_NESTLE_2024_Q1"));
    assert!(cfg.tax.pillar_two.as_ref().map(|p| p.enabled).unwrap_or(false));
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p datasynth-group --test config_parse`
Expected: both tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/datasynth-group/tests/fixtures/mini_nestle.yaml \
       crates/datasynth-group/tests/config_parse.rs
git commit -m "test(group): full Mini-Nestlé config from spec §15 parses"
```

---

### Task 1.4 — Scoping profile inheritance resolver

**Files:**
- Create: `crates/datasynth-group/src/resolve.rs`
- Modify: `crates/datasynth-group/src/lib.rs`
- Create: `crates/datasynth-group/tests/resolve.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/datasynth-group/tests/resolve.rs`:

```rust
//! Spec §3.2 — three-level inheritance: defaults → scoping_profile → per-entity override.

use datasynth_group::{GroupConfig, resolve::resolve_entity};

#[test]
fn test_entity_inherits_from_defaults_then_profile_then_override() {
    let yaml = r#"
id: T
presentation_currency: CHF
period: { start_date: "2024-01-01", length: quarterly }
seed: 1

defaults:
  accounting_framework: ifrs
  industry: manufacturing
  process_models: [o2c, p2p]
  fraud: { fraud_rate: 0.001 }

scoping_profiles:
  significant:
    process_models: [o2c, p2p, h2r, audit]
    audit: { generate_workpapers: true }
  material:
    process_models: [o2c, p2p, audit]

ownership:
  parent_entity_code: P
  entities:
    - { code: P,   country: CH, functional_currency: CHF, scoping_profile: significant, consolidation_method: parent }
    - { code: S1,  country: US, functional_currency: USD, scoping_profile: significant, consolidation_method: full,
        accounting_framework: us_gaap, parent_code: P, ownership_percent: 1.0 }
    - { code: S2,  country: DE, functional_currency: EUR, scoping_profile: material,    consolidation_method: full,
        parent_code: P, ownership_percent: 0.80 }

fx:
  base_currency: CHF
  rate_source: inline
  rates: {}
  policy: { balance_sheet: closing, income_statement: average, equity: historical }
"#;
    let cfg: GroupConfig = serde_yaml::from_str(yaml).unwrap();

    // P: significant profile (process_models from profile), industry from defaults.
    let p = resolve_entity(&cfg, "P").unwrap();
    assert_eq!(p.accounting_framework, "ifrs", "defaults");
    assert_eq!(p.industry, "manufacturing", "defaults");
    assert_eq!(p.process_models, vec!["o2c", "p2p", "h2r", "audit"], "profile overrides defaults");

    // S1: significant profile + per-entity accounting_framework override.
    let s1 = resolve_entity(&cfg, "S1").unwrap();
    assert_eq!(s1.accounting_framework, "us_gaap", "per-entity override wins");
    assert_eq!(s1.process_models, vec!["o2c", "p2p", "h2r", "audit"], "profile process_models");

    // S2: material profile (different process_models).
    let s2 = resolve_entity(&cfg, "S2").unwrap();
    assert_eq!(s2.process_models, vec!["o2c", "p2p", "audit"]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p datasynth-group --test resolve`
Expected: FAIL — `resolve::resolve_entity` does not exist.

- [ ] **Step 3: Implement `resolve` module**

Create `crates/datasynth-group/src/resolve.rs`:

```rust
//! Three-level inheritance: defaults → scoping_profile → per-entity override.
//! Produces a [`ResolvedEntity`] with every field the orchestrator needs.

use crate::config::{EntityConfig, GroupConfig};
use crate::errors::{GroupError, GroupResult};

/// Fully resolved entity ready for shard execution.
#[derive(Debug, Clone)]
pub struct ResolvedEntity {
    pub code: String,
    pub name: Option<String>,
    pub country: String,
    pub functional_currency: String,
    pub scoping_profile_name: String,
    pub consolidation_method: crate::config::ConsolidationMethod,
    pub ownership_percent: Option<rust_decimal::Decimal>,
    pub parent_code: Option<String>,
    pub accounting_framework: String,
    pub industry: String,
    pub process_models: Vec<String>,
    pub rows: Option<u64>,
    pub merged_config: serde_yaml::Value,
}

/// Resolve a single entity by code.
pub fn resolve_entity(cfg: &GroupConfig, code: &str) -> GroupResult<ResolvedEntity> {
    let entity = cfg.ownership.entities.iter()
        .find(|e| e.code == code)
        .ok_or_else(|| GroupError::Config(format!("entity {code} not found")))?;
    resolve_entity_inner(cfg, entity)
}

fn resolve_entity_inner(cfg: &GroupConfig, entity: &EntityConfig) -> GroupResult<ResolvedEntity> {
    // Start from defaults.
    let mut merged = cfg.defaults.clone();
    if merged.is_null() {
        merged = serde_yaml::Value::Mapping(Default::default());
    }

    // Layer the scoping profile.
    if let Some(profile) = cfg.scoping_profiles.get(&entity.scoping_profile) {
        deep_merge(&mut merged, profile);
    } else {
        return Err(GroupError::Config(format!(
            "entity {} references unknown scoping_profile {}",
            entity.code, entity.scoping_profile
        )));
    }

    // Layer per-entity overrides (the catch-all `overrides` map).
    for (k, v) in &entity.overrides {
        deep_merge_key(&mut merged, k, v);
    }

    // Pull out the fields we need, with fallbacks to per-entity EntityConfig fields
    // (which have precedence over the merged map since they are explicit).
    let accounting_framework = entity.accounting_framework.clone()
        .or_else(|| read_str(&merged, "accounting_framework"))
        .unwrap_or_else(|| "ifrs".to_string());
    let industry = entity.industry.clone()
        .or_else(|| read_str(&merged, "industry"))
        .unwrap_or_else(|| "manufacturing".to_string());
    let process_models = read_str_vec(&merged, "process_models").unwrap_or_default();

    Ok(ResolvedEntity {
        code: entity.code.clone(),
        name: entity.name.clone(),
        country: entity.country.clone(),
        functional_currency: entity.functional_currency.clone(),
        scoping_profile_name: entity.scoping_profile.clone(),
        consolidation_method: entity.consolidation_method,
        ownership_percent: entity.ownership_percent,
        parent_code: entity.parent_code.clone(),
        accounting_framework,
        industry,
        process_models,
        rows: entity.rows,
        merged_config: merged,
    })
}

/// Deep-merge `overlay` into `base` (overlay wins on scalar conflicts).
fn deep_merge(base: &mut serde_yaml::Value, overlay: &serde_yaml::Value) {
    use serde_yaml::Value::*;
    match (base, overlay) {
        (Mapping(bm), Mapping(om)) => {
            for (k, v) in om {
                if let Some(bv) = bm.get_mut(k) {
                    deep_merge(bv, v);
                } else {
                    bm.insert(k.clone(), v.clone());
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

fn deep_merge_key(base: &mut serde_yaml::Value, key: &str, value: &serde_yaml::Value) {
    let k = serde_yaml::Value::String(key.into());
    match base {
        serde_yaml::Value::Mapping(m) => {
            if let Some(existing) = m.get_mut(&k) {
                deep_merge(existing, value);
            } else {
                m.insert(k, value.clone());
            }
        }
        _ => {
            let mut m = serde_yaml::Mapping::new();
            m.insert(k, value.clone());
            *base = serde_yaml::Value::Mapping(m);
        }
    }
}

fn read_str(v: &serde_yaml::Value, key: &str) -> Option<String> {
    v.as_mapping()?.get(serde_yaml::Value::String(key.into()))?.as_str().map(String::from)
}

fn read_str_vec(v: &serde_yaml::Value, key: &str) -> Option<Vec<String>> {
    let seq = v.as_mapping()?.get(serde_yaml::Value::String(key.into()))?.as_sequence()?;
    Some(seq.iter().filter_map(|x| x.as_str().map(String::from)).collect())
}
```

Export in `src/lib.rs`:

```rust
pub mod resolve;
pub use resolve::{resolve_entity, ResolvedEntity};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p datasynth-group --test resolve`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/datasynth-group/src/resolve.rs crates/datasynth-group/src/lib.rs \
       crates/datasynth-group/tests/resolve.rs
git commit -m "feat(group): three-level inheritance resolver

defaults → scoping_profile → per-entity override merge, producing a
ResolvedEntity with every field the orchestrator needs. Tests cover
override precedence and missing-profile errors."
```

---

### Task 1.5 — Validation pass: entity references, profile references, ownership sums

**Files:**
- Create: `crates/datasynth-group/src/validate.rs`
- Modify: `crates/datasynth-group/src/lib.rs`
- Create: `crates/datasynth-group/tests/validate.rs`

- [ ] **Step 1: Write failing tests** for: (a) unknown `parent_code`; (b) unknown `scoping_profile`; (c) ownership percent out of [0.0, 1.0]; (d) `parent_entity_code` not in `entities` list; (e) entity referenced as IC seller/buyer not in entities; (f) sum of ownership_percent for a parent's subsidiaries must not exceed 1.0 per subsidiary (not total — document it correctly).

Create `crates/datasynth-group/tests/validate.rs`:

```rust
use datasynth_group::{GroupConfig, validate::validate};

fn parse(yaml: &str) -> GroupConfig { serde_yaml::from_str(yaml).unwrap() }

#[test]
fn test_unknown_parent_code_fails() {
    let cfg = parse(r#"
id: T
presentation_currency: USD
period: { start_date: "2024-01-01", length: quarterly }
seed: 1
scoping_profiles: { std: {} }
ownership:
  parent_entity_code: UNKNOWN
  entities:
    - { code: P, country: US, functional_currency: USD,
        scoping_profile: std, consolidation_method: parent }
fx:
  base_currency: USD
  rate_source: inline
  rates: {}
  policy: { balance_sheet: closing, income_statement: average, equity: historical }
"#);
    let err = validate(&cfg).unwrap_err();
    assert!(err.to_string().contains("parent_entity_code"));
}

// + 4 more tests per the enumeration above, each failing before validate() exists.
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p datasynth-group --test validate`
Expected: FAIL (module missing).

- [ ] **Step 3: Implement `validate`**

Create `crates/datasynth-group/src/validate.rs` covering all checks from Step 1.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p datasynth-group --test validate`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/datasynth-group/src/validate.rs crates/datasynth-group/src/lib.rs \
       crates/datasynth-group/tests/validate.rs
git commit -m "feat(group): GroupConfig validation pass

Checks: parent_entity_code in entities; scoping_profile references
resolve; ownership_percent ∈ [0,1]; IC seller/buyer resolve to
entities. Actionable error messages with entity codes."
```

---

### Task 1.6 — Chunk 1 integration verification

- [ ] **Step 1: Run full crate tests**

Run: `cargo test -p datasynth-group -- --test-threads=4`
Expected: all pass.

- [ ] **Step 2: Run workspace check**

Run: `cargo check --workspace`
Expected: no errors.

- [ ] **Step 3: Run clippy on the new crate**

Run: `cargo clippy -p datasynth-group -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Run fmt**

Run: `cargo fmt --check -p datasynth-group` and if issues, `cargo fmt -p datasynth-group`.

---

## Chunk 2 — Manifest builder (Phase 1)

Goal: given a `GroupConfig`, produce a `GroupManifest` JSON artifact per spec §4. Includes seed derivation, generated-entity expansion, IC pattern expansion, FX rate master resolution, CoA master selection, materiality allocation, component auditor derivation, tax plan resolution, and shard plan assignment.

### Task 2.1 — Seed derivation (blake3 tree)

**Files:**
- Create: `crates/datasynth-group/src/manifest/mod.rs`
- Create: `crates/datasynth-group/src/manifest/seeds.rs`
- Create: `crates/datasynth-group/tests/seeds.rs`

**Acceptance criteria:**
- `derive_manifest_seed(group_seed, period_start) -> [u8; 32]` uses `blake3("manifest" || group_seed || period_start)`.
- `derive_entity_seed(group_seed, entity_code) -> [u8; 32]` is order-independent.
- `derive_aggregate_seed(group_seed, period_start) -> [u8; 32]`.
- `derive_ic_pair_id(group_seed, ic_relationship_id, pair_index) -> IcPairId` (newtype around `[u8; 32]`).
- Adding/removing an entity from the config does not change any other entity's `entity_seed`.
- Reordering `ownership.entities` in YAML produces identical manifest output.

Detailed TDD steps authored at execution time.

### Task 2.2 — Generated-entity block expansion

**Files:**
- Create: `crates/datasynth-group/src/manifest/expansion.rs`
- Create: `crates/datasynth-group/tests/expansion.rs`

**Acceptance criteria:**
- `GeneratedEntityBlock { count: 200, code_prefix: "NESTLE_EU_", country: [DE, FR, IT], ... }` expands to 200 `ResolvedEntity` instances with deterministic codes (`NESTLE_EU_0000001`..`NESTLE_EU_0000200`), country sampled with `manifest_seed`.
- `ownership_percent_range: [0.85, 1.00]` → uniform sampling within the range (8-dp precision).
- Generated entities must not collide with explicit entities (validate; fail with useful error).
- Stable across reordering of `generated:` blocks.

### Task 2.3 — IC pattern expansion

**Files:**
- Create: `crates/datasynth-group/src/manifest/ic_expansion.rs`
- Create: `crates/datasynth-group/tests/ic_expansion.rs`

**Acceptance criteria:**
- `{pattern: {seller_scoping_profile: significant, buyer_scoping_profile: any}, types: [management_fee]}` expands to all matching (seller, buyer) pairs — excluding self-pairs, excluding pairs where buyer's consolidation_method is `fair_value`.
- Explicit relationships override pattern-derived ones on the same (seller, buyer, type) triple.
- Produces a `ResolvedIcRelationship { id, seller, buyer, types, annual_volume, transfer_pricing, markup_percent }` with a stable `id = blake3("icr" || group_seed || seller || buyer || types[0])`.

### Task 2.4 — Chart of accounts master resolution

**Files:**
- Create: `crates/datasynth-group/src/manifest/coa_master.rs`
- Create: `crates/datasynth-group/tests/coa_master.rs`

**Acceptance criteria:**
- Identifies all distinct `accounting_framework` values used across entities.
- For each, loads the existing framework-specific CoA from `datasynth-core` (existing `ChartOfAccounts::new(...)` pathway).
- Emits `ChartOfAccountsMaster { primary_framework, frameworks: BTreeMap<String, ChartOfAccounts>, coa_id }`.
- `coa_id` is deterministic from framework set + complexity.

### Task 2.5 — FX rate master resolution

**Files:**
- Create: `crates/datasynth-group/src/manifest/fx_master.rs`
- Create: `crates/datasynth-group/tests/fx_master.rs`

**Acceptance criteria:**
- `rate_source: inline` → use `cfg.fx.rates` verbatim (validate completeness for the period).
- `rate_source: user_supplied` → treat as inline (same behavior, opt-in tag for traceability).
- `rate_source: historical_series` → out of scope v5.0 (return `GroupError::Config` with hint).
- Validate: all currency pairs entities need are present in the rate table.
- Emit `FxRateMaster { base_currency, rates, policy, spot_rates_by_date, average_rates_by_period }`.

### Task 2.6 — Audit engagement plan

**Files:**
- Create: `crates/datasynth-group/src/manifest/audit_plan.rs`
- Create: `crates/datasynth-group/tests/audit_plan.rs`

**Acceptance criteria:**
- v5.0 emits the PLAN, not the artifacts (per spec §13 v5.0 scope).
- `group_materiality = basis_value * percent` where `basis_value` is group-wide (estimated from entity-level revenue budgets for v5.0; refined by actual at-aggregate-time in v5.1).
- `performance_materiality = group_materiality * 0.75` by default.
- `clearly_trivial = group_materiality * 0.05`.
- `component_materiality_allocations[entity].scope`: `full` if entity revenue ≥ 15%, `specific` if 5-15%, `analytical` otherwise.
- Component auditor derivation: one per jurisdiction by default (configurable in v5.1).
- Output struct: `AuditEngagementPlan` matching spec §4.1.

### Task 2.7 — Tax group plan

**Files:**
- Create: `crates/datasynth-group/src/manifest/tax_plan.rs`
- Create: `crates/datasynth-group/tests/tax_plan.rs`

**Acceptance criteria:**
- v5.0 emits the PLAN only (scope + jurisdictions); generators are v5.2.
- Plan captures: Pillar 2 in-scope jurisdictions (validated against entities' countries), CbCR reporting jurisdiction, TP master-file flag, local-file jurisdictions list.
- Output struct: `TaxGroupPlan`.

### Task 2.8 — Shard plan assignment

**Files:**
- Create: `crates/datasynth-group/src/manifest/shard_plan.rs`
- Create: `crates/datasynth-group/tests/shard_plan.rs`

**Acceptance criteria:**
- Default: one shard per distinct `scoping_profile`, entities batched to ~1 TB estimated per shard.
- Batching is deterministic: entities sorted by `entity_code` ascending before assignment.
- `shard_id` format: `S_{SCOPING_PROFILE_INITIAL}_{index:04}` (e.g., `S_SIG_0001`, `S_MAT_0001`, `S_CON_0001`).
- Row-budget estimation: `scoping_profile.row_budget * entity_count` (simple for v5.0).
- Each entity's manifest record carries its `shard_id`.

### Task 2.9 — GroupManifest assembly

**Files:**
- Create: `crates/datasynth-group/src/manifest/builder.rs`
- Modify: `crates/datasynth-group/src/manifest/mod.rs` (expose `GroupManifest` + `build_manifest`)
- Create: `crates/datasynth-group/tests/manifest_builder.rs`

**Acceptance criteria:**
- `GroupManifest` struct matches spec §4.1 field-for-field (schema_version, group_id, group_seed, presentation_currency, period, ownership_graph, scoping_profiles, chart_of_accounts_master, fx_rate_master, shared_masters, ic_relationships, audit_engagement_plan, tax_group_plan, shard_plan, aggregate_seed).
- `build_manifest(cfg: &GroupConfig) -> GroupResult<GroupManifest>` orchestrates Tasks 2.1–2.8.
- Serializes to JSON via serde; JSON schema available via `schemars` if we bring it in (otherwise manual roundtrip test).
- Roundtrip property: `build_manifest(cfg)` twice with the same config produces byte-identical JSON.

### Task 2.10 — Mini-Nestlé manifest golden test

**Files:**
- Create: `crates/datasynth-group/tests/golden/mini_nestle_manifest.json` (generated, committed)
- Create: `crates/datasynth-group/tests/manifest_golden.rs`

**Acceptance criteria:**
- Generated manifest from `mini_nestle.yaml` (Task 1.3 fixture) matches the committed golden JSON byte-for-byte.
- Test command to regenerate golden: `cargo test -p datasynth-group --test manifest_golden -- --ignored regenerate_golden`.

---

## Chunk 3 — IC pair derivation + JE injection

Goal: implement the manifest-driven IC matching strategy from spec §5.1 — both shards of an IC pair derive identical pair_id / amount / date deterministically, and inject matching JEs into their respective entity books.

### Task 3.1 — `IcPairId` core type + `ic_pair_id` on `JournalEntryHeader`

**Files:**
- Create: `crates/datasynth-core/src/models/ic_pair.rs`
- Modify: `crates/datasynth-core/src/models/mod.rs`
- Modify: `crates/datasynth-core/src/models/journal_entry.rs` (add optional `ic_pair_id: Option<IcPairId>` and `ic_partner_entity: Option<String>` to `JournalEntryHeader`)
- Modify both `JournalEntryHeader::new()` and `with_deterministic_id()` constructors

**Acceptance criteria:**
- `IcPairId([u8; 32])` newtype with `Display` = hex; `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `Hash`, `Serialize`, `Deserialize`.
- `JournalEntryHeader` has two new optional fields, default `None`, backward-compatible (existing callers don't set them).
- Existing tests across the workspace still pass.

### Task 3.2 — IC pair plan derivation

**Files:**
- Create: `crates/datasynth-group/src/shard/ic_plan.rs`
- Create: `crates/datasynth-group/tests/ic_plan.rs`

**Acceptance criteria:**
- `derive_ic_pair_plans(manifest, entity_code) -> Vec<IcPairPlan>` returns every IC pair plan the given entity participates in (either as seller or buyer).
- Each `IcPairPlan { pair_id, ic_relationship_id, role, partner_entity, transaction_type, amount, date, index }` is fully derived from manifest + group_seed.
- Determinism property: running the derivation on both shards (seller's and buyer's) produces pair plans that are mirror-image matches — same `pair_id`, same `amount`, same `date`, same `transaction_type`, opposite `role`.
- Pair count: `N = round(annual_volume / avg_amount)` where `avg_amount` depends on transaction_type (constants defined in the module; e.g., goods_sale ~ $50k, management_fee ~ $25k, royalty ~ $100k).

### Task 3.3 — IC JE injection hook in the per-entity orchestrator

**Files:**
- Modify: `crates/datasynth-runtime/src/enhanced_orchestrator.rs` — new optional `ShardContext` field on `EnhancedOrchestrator`, `set_shard_context(&mut self, ctx: ShardContext)`
- Create: `crates/datasynth-runtime/src/shard_context.rs` — `ShardContext { entity_seed, ic_pair_plans, fx_rates, coa, shared_masters_ref }`
- Modify: `crates/datasynth-runtime/src/enhanced_orchestrator.rs` — call `inject_ic_journal_entries` after the standard JE generation phase when `ShardContext` is present
- Create: `crates/datasynth-generators/src/intercompany/ic_je_injector.rs`
- Modify: `crates/datasynth-generators/src/intercompany/mod.rs`
- Create: `crates/datasynth-generators/tests/ic_je_injection.rs`

**Acceptance criteria:**
- `inject_ic_journal_entries(plans: &[IcPairPlan], ctx: &InjectionCtx) -> Vec<JournalEntry>`
- For `role = seller` + `type = goods_sale`: DR AR (1100 / local equivalent), CR Revenue (4000 / local equivalent) for the pair amount.
- For `role = buyer` + `type = goods_sale`: DR COGS (5100), CR AP (2000).
- Analogous rules for other transaction_types (management_fee, royalty, loan_interest, dividend, expense_recharge, cost_sharing).
- Every injected JE has `ic_pair_id = plan.pair_id` and `ic_partner_entity = plan.partner_entity`.
- JEs pass the existing `is_balanced()` check.
- `ShardContext == None` (the default) preserves existing single-entity behavior byte-for-byte.

### Task 3.4 — Determinism property test: two shards produce matching pair IDs

**Files:**
- Create: `crates/datasynth-group/tests/ic_matching_property.rs`

**Acceptance criteria:**
- Build a manifest with a 5-entity group and ~10 IC relationships.
- Run `derive_ic_pair_plans` for every entity.
- Assert: for every seller's pair plan, there exists a buyer's pair plan with matching `pair_id`, matching `amount`, matching `date`, and opposite `role`.
- Coverage: 100 % (by construction of manifest-driven strategy).

---

## Chunk 4 — Shard runner (Phase 2)

Goal: given a manifest + shard spec, produce per-entity archives by invoking the unchanged `EnhancedOrchestrator` with the right `ShardContext`, routing output to `entities/{code}/`.

### Task 4.1 — `ShardContext` construction from manifest

**Files:**
- Create: `crates/datasynth-group/src/shard/context.rs`
- Create: `crates/datasynth-group/tests/shard_context.rs`

**Acceptance criteria:**
- `build_shard_context(manifest: &GroupManifest, entity_code: &str) -> ShardContext`
- Populates: entity_seed, IC pair plans (the entity's subset), FX rates projection, CoA for the entity's framework, shared-master pool seeds.

### Task 4.2 — Per-entity `GeneratorConfig` construction

**Files:**
- Create: `crates/datasynth-group/src/shard/per_entity_config.rs`
- Create: `crates/datasynth-group/tests/per_entity_config.rs`

**Acceptance criteria:**
- `build_entity_generator_config(manifest, entity) -> datasynth_config::GeneratorConfig`
- Merges: defaults → scoping_profile → entity overrides (from Task 1.4 resolver).
- Sets: single company, entity's functional currency, entity's framework, entity's process_models, row budget.
- Resulting config is valid against `datasynth-config` schema.

### Task 4.3 — Shard runner

**Files:**
- Create: `crates/datasynth-group/src/shard/runner.rs`
- Modify: `crates/datasynth-group/src/shard/mod.rs`
- Create: `crates/datasynth-group/tests/shard_runner.rs`

**Acceptance criteria:**
- `run_shard(manifest: &GroupManifest, shard_id: &str, out_dir: &Path) -> GroupResult<ShardSummary>`
- For each entity in shard: construct config + ShardContext, instantiate `EnhancedOrchestrator`, call `generate()`, write output to `out_dir/entities/{code}/`.
- Existing orchestrator output shape preserved (same files, same fields).
- `ShardSummary { shard_id, entity_summaries: Vec<EntitySummary> }` written to `out_dir/shard_summary.json`.

### Task 4.4 — Per-entity output routing

**Files:**
- Modify: `crates/datasynth-output/src/lib.rs` — add `OutputRootConfig { root_dir: PathBuf, per_entity_subtree: bool, entity_code: Option<String> }`
- Modify: `crates/datasynth-cli/src/output_writer.rs` — respect `OutputRootConfig` when writing files

**Acceptance criteria:**
- `per_entity_subtree: true` + `entity_code: Some("X")` routes all output under `{root_dir}/entities/X/`.
- `per_entity_subtree: false` (default) preserves today's flat layout.
- Existing single-entity tests still pass unchanged.

### Task 4.5 — Shard end-to-end smoke test

**Files:**
- Create: `crates/datasynth-group/tests/shard_e2e.rs`

**Acceptance criteria:**
- Build Mini-Nestlé manifest.
- Run shard containing 2 entities (one as seller, one as buyer of a goods_sale IC relationship).
- Verify: `entities/{code}/journal_entries.json` exists for each entity, has JEs with `ic_pair_id` populated, pair IDs mirror across entities.
- Verify: `shard_summary.json` has both entities.

---

## Chunk 5 — Consolidation engine (Phase 3a steps 1-2, 7)

Goal: aggregate per-entity trial balances, match IC pairs, apply eliminations, and produce a post-elimination consolidated trial balance. (Translation, NCI, FS generation in Chunks 6-8.)

### Task 5.1 — Per-entity TB loader

**Files:**
- Create: `crates/datasynth-group/src/aggregate/tb_loader.rs`
- Create: `crates/datasynth-group/tests/tb_loader.rs`

**Acceptance criteria:**
- `load_entity_trial_balance(entity_dir: &Path) -> GroupResult<TrialBalance>`
- Reads `entity_dir/subledger/trial_balance.json` (or the equivalent existing path).
- Validates total debit = total credit.

### Task 5.2 — Pre-elimination TB aggregation

**Files:**
- Create: `crates/datasynth-group/src/aggregate/pre_elim.rs`
- Create: `crates/datasynth-group/tests/pre_elim.rs`

**Acceptance criteria:**
- `aggregate_pre_elimination(manifest, entity_tbs) -> AggregatedTb`
- Only entities with `consolidation_method ∈ {parent, full}` contribute to the sum.
- Equity-method / fair-value / proportional entities are held separately (handled in Chunk 7).
- Per-account totals = sum of entity amounts (in presentation currency if same; handled by Chunk 6 otherwise).

### Task 5.3 — IC pair matcher

**Files:**
- Create: `crates/datasynth-group/src/aggregate/ic_matcher.rs`
- Create: `crates/datasynth-group/tests/ic_matcher.rs`

**Acceptance criteria:**
- `match_ic_pairs(entity_jes) -> IcMatchResult { matched: Vec<IcMatchedPair>, unmatched: Vec<UnmatchedSide> }`
- Joins JEs on `ic_pair_id`; produces matched pairs with both sides.
- Unmatched reported with reason (`missing_buyer_side`, `missing_seller_side`, `amount_drift_above_tolerance` — latter is no-op for manifest-driven; populated by emergent-fuzzy in v5.3).
- Coverage calculation: `matched / total_planned`.

### Task 5.4 — `EliminationEntry` generation from matched pairs

**Files:**
- Create: `crates/datasynth-group/src/aggregate/elimination.rs`
- Create: `crates/datasynth-group/tests/elimination.rs`

**Acceptance criteria:**
- For each matched pair, generate an `EliminationEntry` (from `datasynth-core::models::intercompany::elimination`).
- AR↔AP pair → DR AP (buyer side), CR AR (seller side).
- Revenue↔COGS pair → DR Revenue, CR COGS.
- Loan↔Borrowing → DR Borrowing, CR Loan.
- Dividend paid ↔ received → DR Dividend income, CR Dividend paid.
- Management recharge → DR Expense, CR Management income.
- All eliminations balance (DR = CR).

### Task 5.5 — Elimination → JE conversion

**Files:**
- Modify: `crates/datasynth-group/src/aggregate/elimination.rs`
- Reuse: `datasynth-generators::intercompany::elimination_to_je::elimination_to_journal_entries` (existing from v1.3.0 Tier 0)

**Acceptance criteria:**
- Convert elimination entries to JEs.
- Each JE has `header.is_elimination = true`, `document_type = "ELIMINATION"`, `source = "CONSOLIDATION"`.

### Task 5.6 — Post-elimination TB

**Files:**
- Create: `crates/datasynth-group/src/aggregate/post_elim.rs`
- Create: `crates/datasynth-group/tests/post_elim.rs`

**Acceptance criteria:**
- Apply elimination JEs to pre-elimination TB.
- Resulting TB has IC eliminations reflected (e.g., consolidated revenue = sum(external revenue) without double-counting intercompany goods sales).

### Task 5.7 — IC matching coverage report

**Files:**
- Create: `crates/datasynth-group/src/aggregate/coverage_report.rs`

**Acceptance criteria:**
- Emit `ic_eliminations/ic_matching_coverage.json` per spec §5.4.
- `total_pairs_planned`, `matched`, `coverage`, `unmatched_by_reason`, `unmatched_sample[:100]`.

---

## Chunk 6 — IAS 21 translation + CTA (Phase 3a steps 3-4)

Goal: translate each entity's post-elimination TB from functional to presentation currency per IAS 21, accumulate CTA to OCI.

### Task 6.1 — Monetary / non-monetary classification helper

**Files:**
- Create: `crates/datasynth-group/src/aggregate/translation/classify.rs`
- Create: `crates/datasynth-group/tests/classify.rs`

**Acceptance criteria:**
- `classify_account(account_code, framework) -> AccountType` returns `BsMonetary`, `BsNonMonetary`, `PlRevenue`, `PlExpense`, `Equity`, `PlOci`.
- Leverages existing `ChartOfAccounts::get_account(code).account_type` where available.
- BS monetary examples: cash (1000), AR (1100), AP (2000), loans, bonds.
- BS non-monetary examples: inventory, fixed assets, intangibles, goodwill, prepaid expenses.

### Task 6.2 — Per-entity TB translation

**Files:**
- Create: `crates/datasynth-group/src/aggregate/translation/translate.rs`
- Create: `crates/datasynth-group/tests/translate.rs`

**Acceptance criteria:**
- `translate_entity_tb(entity_tb, entity_functional_ccy, manifest.fx_rate_master, period_end) -> TranslatedTb`
- BS monetary items at closing rate.
- BS non-monetary at historical (for v5.0: use transaction-date rate if tracked at source, else weighted-average proxy).
- P&L items at average rate for the period.
- Equity items at historical rate.
- Sum of translated DR != sum of translated CR → residual = CTA.

### Task 6.3 — CTA computation + rollforward

**Files:**
- Create: `crates/datasynth-group/src/aggregate/translation/cta.rs`
- Create: `crates/datasynth-group/tests/cta.rs`

**Acceptance criteria:**
- `compute_cta(translated_tb) -> Decimal` returns DR-CR residual.
- `cta_rollforward(entity_code, opening_cta, period_cta) -> CtaRollforward`
- Opening from prior period's output (if supplied via `--prior-period-aggregate` CLI flag) else 0.
- Closing = opening + period CTA.
- Per-entity emission to `consolidated/cta_rollforward.json` (aggregate file with per-entity rows).

### Task 6.4 — Translation worksheet emission

**Files:**
- Create: `crates/datasynth-group/src/aggregate/translation/worksheet.rs`

**Acceptance criteria:**
- Emit `consolidated/translation_worksheet.json` per spec §9 layout.
- Line-by-line: `account_code, local_amount, ccy_pair, rate, rate_basis, translated_amount`.
- Human-readable for audit traceability.

### Task 6.5 — Integration test: Mini-Nestlé translation round-trip

**Files:**
- Create: `crates/datasynth-group/tests/translation_e2e.rs`

**Acceptance criteria:**
- Build manifest, run shards, run translation on the output.
- Assert: translated consolidated TB includes entities in 3 functional currencies (CHF, USD, EUR, BRL).
- Assert: CTA rollforward has 4 non-parent entities, each with a non-zero period CTA (BR has largest due to BRL volatility).

---

## Chunk 7 — NCI + equity method (Phase 3a steps 5-6)

Goal: for non-wholly-owned fully-consolidated subsidiaries, emit NCI rollforward; for equity-method JVs, emit single-line investment carrying value + P&L pickup.

### Task 7.1 — NCI rollforward per subsidiary

**Files:**
- Create: `crates/datasynth-group/src/aggregate/nci.rs`
- Create: `crates/datasynth-group/tests/nci.rs`

**Acceptance criteria:**
- `compute_nci_rollforward(entity, period_net_income, period_oci, opening_nci) -> NciRollforward`
- `(1 - ownership_percent) * net_income` → NCI share of profit.
- `(1 - ownership_percent) * OCI` → NCI share of OCI.
- Dividends to NCI: `(1 - ownership_percent) * total_dividends_paid` (from per-entity dividend records; if none, 0).
- `closing_nci = opening_nci + nci_share_profit + nci_share_oci - nci_dividends`

### Task 7.2 — NCI opening balance ingestion

**Files:**
- Create: `crates/datasynth-group/src/aggregate/nci/opening.rs`
- Create: `crates/datasynth-group/tests/nci_opening.rs`

**Acceptance criteria:**
- CLI flag: `datasynth-data group aggregate --prior-period-aggregate ./prior_q/`.
- Reads `./prior_q/consolidated/nci_rollforward.json` and extracts closing NCI per entity as this period's opening.
- Missing file → default to 0 per entity, with a warning.

### Task 7.3 — Equity-method investment rollforward

**Files:**
- Create: `crates/datasynth-group/src/aggregate/equity_method.rs`
- Create: `crates/datasynth-group/tests/equity_method.rs`

**Acceptance criteria:**
- For each entity with `consolidation_method = equity_method`:
  - Investment carrying value: opening + (ownership_percent × associate net income) − dividends received − impairment.
  - P&L single-line: "Share of profit of associates" = ownership_percent × associate net income.
- Emission: part of consolidated BS and IS construction in Chunk 8.

### Task 7.4 — NCI + equity method integration in consolidated TB

**Files:**
- Modify: `crates/datasynth-group/src/aggregate/post_elim.rs` (from Chunk 5)

**Acceptance criteria:**
- Post-elimination consolidated TB reflects NCI on equity side.
- Equity-method entities appear as single-line "Investment in associates" on BS + "Share of profit of associates" on IS.

---

## Chunk 8 — Consolidated FS + schedule + notes

Goal: from the translated + NCI-adjusted consolidated TB, produce consolidated BS / IS / CF / Changes in Equity, the consolidation schedule, and notes.

### Task 8.1 — Consolidated BS generator

**Files:**
- Create: `crates/datasynth-group/src/aggregate/fs/balance_sheet.rs`
- Create: `crates/datasynth-group/tests/consolidated_bs.rs`

**Acceptance criteria:**
- Group accounts by BS classification (Current Assets, Non-Current Assets, Current Liabilities, Non-Current Liabilities, Equity, NCI).
- Balance: `Total Assets == Total Liabilities + Total Equity + NCI` within ε (0.01 currency units).
- Emission: part of `consolidated_financial_statements.json`.

### Task 8.2 — Consolidated IS generator

**Files:**
- Create: `crates/datasynth-group/src/aggregate/fs/income_statement.rs`
- Create: `crates/datasynth-group/tests/consolidated_is.rs`

**Acceptance criteria:**
- P&L accounts aggregated, with IC eliminations applied.
- Share of profit of associates as a separate line.
- Net income attributable to Owners vs NCI split.

### Task 8.3 — Consolidated CF generator (indirect method)

**Files:**
- Create: `crates/datasynth-group/src/aggregate/fs/cash_flow.rs`

**Acceptance criteria:**
- Operating (indirect): net income + non-cash (depreciation, amortization, impairment) + WC changes.
- Investing: capex, acquisitions, disposals.
- Financing: debt issuance/repayment, dividends paid (inc. to NCI), equity issuance.
- Reconciles to opening + closing cash.

### Task 8.4 — Changes in Equity

**Files:**
- Create: `crates/datasynth-group/src/aggregate/fs/equity_changes.rs`

**Acceptance criteria:**
- Opening equity + net income + OCI (including CTA) − dividends = closing equity.
- Separate NCI column.

### Task 8.5 — Consolidation schedule

**Files:**
- Create: `crates/datasynth-group/src/aggregate/fs/consolidation_schedule.rs`
- Create: `crates/datasynth-group/tests/consolidation_schedule.rs`

**Acceptance criteria:**
- Per-line: `account_category → entity_amounts (HashMap) → pre_elimination_total → elimination_adjustments → post_elimination_total`.
- Emitted to `consolidated/consolidation_schedule.json`.

### Task 8.6 — Notes to consolidated FS (basic set)

**Files:**
- Create: `crates/datasynth-group/src/aggregate/fs/notes.rs`

**Acceptance criteria:**
- Template-assembled notes covering: significant accounting policies (from framework), basis of consolidation, IC eliminations summary, NCI summary, CTA summary, operating segments (deferred to v5.1 — placeholder note), subsequent events (placeholder), related parties (placeholder).
- Emitted to `consolidated/notes_to_consolidated_fs.json`.

### Task 8.7 — Consolidated FS output assembly

**Files:**
- Create: `crates/datasynth-group/src/aggregate/fs/writer.rs`

**Acceptance criteria:**
- `write_consolidated_fs(fs, out_dir) -> GroupResult<()>` emits:
  - `consolidated/consolidated_financial_statements.json` (BS + IS + CF + Changes in Equity bundled)
  - `consolidated/consolidation_schedule.json`
  - `consolidated/notes_to_consolidated_fs.json`

---

## Chunk 9 — Aggregate phase driver + integration

Goal: tie together Chunks 5–8 into a single `aggregate` phase entrypoint, plus output wiring.

### Task 9.1 — Aggregate phase driver

**Files:**
- Create: `crates/datasynth-group/src/aggregate/mod.rs`
- Create: `crates/datasynth-group/tests/aggregate_e2e.rs`

**Acceptance criteria:**
- `run_aggregate(manifest, shards_dir, out_dir, opts: AggregateOptions) -> GroupResult<AggregateSummary>`
- Runs 5 → 6 → 7 → 8 in order.
- `AggregateOptions { prior_period_aggregate: Option<PathBuf>, tolerate_missing_shards: bool }`.
- Emits all expected files per spec §9.

### Task 9.2 — Standalone `generate` convenience

**Files:**
- Create: `crates/datasynth-group/src/standalone.rs`
- Create: `crates/datasynth-group/tests/standalone_e2e.rs`

**Acceptance criteria:**
- `generate_standalone(cfg: &GroupConfig, out_dir) -> GroupResult<()>` runs manifest + all shards in rayon pool + aggregate in one call.
- Produces bit-identical output to 3-step CLI invocation (Task 11.x property test).

---

## Chunk 10 — CLI integration

Goal: new `datasynth-data group` subcommand with 4 actions: `manifest`, `shard`, `aggregate`, `generate`.

### Task 10.1 — Add `datasynth-group` dependency to CLI crate

**Files:** `crates/datasynth-cli/Cargo.toml`

**Acceptance criteria:** compiles.

### Task 10.2 — `group manifest` subcommand

**Files:** modify `crates/datasynth-cli/src/main.rs`

**Acceptance criteria:**
- `datasynth-data group manifest --config group.yaml --out ./manifest.json` produces manifest.
- Validates before building.
- Exit 0 on success; actionable error on config invalid.

### Task 10.3 — `group shard` subcommand

**Acceptance criteria:**
- `datasynth-data group shard --manifest ./manifest.json --shard S_SIG_0001 --out ./shards/S_SIG_0001/`
- Validates shard_id exists in manifest.
- Produces per-entity archives under `entities/{code}/`.

### Task 10.4 — `group aggregate` subcommand

**Acceptance criteria:**
- `datasynth-data group aggregate --manifest ./manifest.json --shards ./shards/ --out ./group_archive/`
- `--prior-period-aggregate` flag for NCI opening balance ingestion.
- `--tolerate-missing-shards` flag for partial-archive mode.

### Task 10.5 — `group generate` convenience + auto-detection on existing `generate`

**Acceptance criteria:**
- `datasynth-data group generate --config group.yaml --out ./group_archive/` runs 1→2→3 in-process.
- Existing `datasynth-data generate --config group.yaml ...` auto-detects presence of `group:` and dispatches to group engine.

### Task 10.6 — CLI integration tests

**Files:** `crates/datasynth-cli/tests/group_cli.rs`

**Acceptance criteria:**
- Each of the 4 actions runs against Mini-Nestlé config and produces expected output files.
- Exit codes match spec.

---

## Chunk 11 — Property tests + golden fixture

Goal: multiple property tests proving the core guarantees of spec §5 and §10, plus a committed golden archive for the Mini-Nestlé fixture.

### Task 11.1 — Mini-Nestlé golden archive generation + check

**Files:**
- Create: `crates/datasynth-group/tests/golden/mini_nestle/` (directory, committed — use tarball or directory)
- Create: `crates/datasynth-group/tests/golden_archive.rs`

**Acceptance criteria:**
- `cargo test -p datasynth-group --test golden_archive` compares generated vs committed archive file-by-file.
- `--ignored regenerate_golden` mode updates the committed archive.

### Task 11.2 — IC matching coverage ≥ 98 %

**Files:** `crates/datasynth-group/tests/ic_coverage_property.rs`

**Acceptance criteria:**
- Property test over 10 randomized small configs (varied entity counts 3–15, varied IC relationship counts).
- Manifest-driven strategy: coverage = 1.0 exactly.
- Emergent-fuzzy strategy: v5.0 tests only cover manifest-driven (fuzzy is v5.3).

### Task 11.3 — Consolidated balance property: `A = L + E + NCI`

**Files:** `crates/datasynth-group/tests/balance_property.rs`

**Acceptance criteria:**
- For 10 randomized configs: post-elim consolidated BS balances within ε.
- For the Mini-Nestlé fixture: balances to the cent.

### Task 11.4 — Determinism: in-process vs subprocess

**Files:** `crates/datasynth-group/tests/determinism_in_process.rs`

**Acceptance criteria:**
- Run `generate_standalone` in-process twice → byte-identical archives.
- Run manifest → shard(x2) → aggregate manually in 3 subprocesses → byte-identical to standalone.

### Task 11.5 — Order-independence

**Files:** `crates/datasynth-group/tests/order_independence.rs`

**Acceptance criteria:**
- Reorder `ownership.entities` in YAML → byte-identical archive.
- Reorder `intercompany.relationships` → byte-identical archive.

### Task 11.6 — Backward compat: existing `companies:` configs

**Files:** `crates/datasynth-group/tests/backcompat.rs`

**Acceptance criteria:**
- 3 representative existing single-entity configs from `configs/examples/` produce byte-identical output to the current main's orchestrator.
- No `group:` key → no new files; existing shape preserved.

---

## Chunk 12 — Documentation, release wiring, version bump

Goal: CLAUDE.md / README updates, example configs, CHANGELOG, workspace version bump to 5.0.0.

### Task 12.1 — Update `CLAUDE.md`

**Acceptance criteria:**
- New crate `datasynth-group` listed in workspace section.
- New top-level config section `group:` documented with the illustrative example (condensed).
- New CLI subcommand `datasynth-data group {manifest,shard,aggregate,generate}` documented.
- New output archive layout (`entities/{code}/`, `consolidated/`, `ic_eliminations/`, `audit/`, `tax/`) documented.
- New models (IcPairId, GroupManifest, etc.) listed in Key Models section.

### Task 12.2 — Update `README.md`

**Acceptance criteria:**
- Add a "Group audit simulation" section with a 20-line Mini-Nestlé snippet + expected output layout.

### Task 12.3 — Add `configs/examples/group/mini_nestle.yaml`

**Acceptance criteria:**
- Exact copy of the spec §15 appendix.
- Added to `configs/examples/README.md` index.

### Task 12.4 — CHANGELOG entry

**Acceptance criteria:**
- Under `## v5.0.0`:
  - New crate `datasynth-group`
  - New config: `group:` section
  - New CLI: `datasynth-data group {manifest,shard,aggregate,generate}`
  - New output artifacts: consolidated FS, consolidation schedule, NCI rollforward, CTA rollforward, translation worksheet, IC eliminations, IC matching coverage, notes to consolidated FS
  - Backward-compatible: existing single-entity configs unchanged
  - v5.1-5.3 on the roadmap

### Task 12.5 — Workspace version bump to 5.0.0

**Files:** root `Cargo.toml`, `[workspace.package]` version field + all crate-internal workspace dep pins.

**Acceptance criteria:**
- `cargo check --workspace` passes.
- `./scripts/publish.sh --dry-run` passes (if publish script exists).

### Task 12.6 — Final workspace verification

- [ ] Run: `cargo test --workspace -- --test-threads=4`
- [ ] Run: `cargo clippy --workspace -- -D warnings`
- [ ] Run: `cargo fmt --check`
- [ ] Generate Mini-Nestlé: `cargo run -p datasynth-cli --release -- group generate --config configs/examples/group/mini_nestle.yaml --out /tmp/v5.0-test/`
- [ ] Inspect `/tmp/v5.0-test/`: confirm all expected directories and files exist.

### Task 12.7 — Release commit + tag

**Acceptance criteria:**
- Commit message: `release: v5.0.0 — Group audit simulation engine foundation`
- Tag: `v5.0.0`
- All tests green on `main`.

---

## Final verification

- [ ] Demo Mini-Nestlé: `datasynth-data group generate --config configs/examples/group/mini_nestle.yaml --out ./v5.0-mini-nestle/`
- [ ] Verify output layout matches spec §9 (spot-check each top-level directory exists).
- [ ] Verify IC matching coverage in `./v5.0-mini-nestle/ic_eliminations/ic_matching_coverage.json`: `coverage >= 0.98`.
- [ ] Verify consolidated BS balances in `./v5.0-mini-nestle/consolidated/consolidated_financial_statements.json`.
- [ ] Verify backward-compat: pick a representative existing `configs/examples/*.yaml` without `group:`, run `datasynth-data generate --config it.yaml --out ./back/`, confirm output matches pre-v5.0 shape.
- [ ] Run: `cargo test --workspace -- --test-threads=4`
- [ ] Run: `cargo clippy --workspace -- -D warnings`
- [ ] Tag `v5.0.0`, commit, push.

---

## Plan self-review

1. **Spec coverage** — every v5.0 scope item from spec §13 has a task: `datasynth-group` crate (Ch 1), manifest/shard/aggregate (Ch 2/4/5-9), `group:` config (Ch 1), `scoping_profiles` (Ch 1/Task 1.4), IC manifest-driven matching (Ch 3/5), elimination → GL JE (Ch 5), IAS 21 + CTA (Ch 6), NCI rollforward (Ch 7), consolidated FS + schedule (Ch 8), per-entity subtree (Ch 4/Task 4.4), determinism (Ch 11), new CLI (Ch 10), Mini-Nestlé reference + golden (Ch 11 + Ch 12/Task 12.3).
2. **Placeholders** — no "TBD"/"TODO"/"fill in details" left. Chunks 2-12 use task-level granularity with explicit file paths and acceptance criteria per plan convention; detailed TDD is appended at execution time (matches this repo's v1.3.0 pattern).
3. **Type consistency** — `ResolvedEntity`, `GroupManifest`, `ShardContext`, `IcPairId`, `IcPairPlan`, `AggregatedTb`, `NciRollforward`, `CtaRollforward` — referenced names are consistent across chunks.
4. **v5.0 scope discipline** — GAAP bridges, ISA 600 component auditor *generation*, Pillar 2 *calculation*, segment reporting, notes-to-FS full content are all explicitly deferred to v5.1/v5.2 per spec §13. v5.0 emits the PLAN for audit + tax but not the artifacts.

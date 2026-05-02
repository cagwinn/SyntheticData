//! GroupManifest assembly — spec §4.1, Task 2.9.
//!
//! [`build_manifest`] is the top-level orchestrator.  It calls every prior
//! Task 2.1–2.8 builder in dependency order and assembles the results into a
//! single [`GroupManifest`] that drives shard and aggregate phases.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::config::{GroupConfig, OutputLayoutConfig, PeriodLength};
use crate::errors::{GroupError, GroupResult};
use crate::manifest::audit_plan::{build_audit_engagement_plan, AuditEngagementPlan};
use crate::manifest::coa_master::{build_coa_master, ChartOfAccountsMaster};
use crate::manifest::expansion::{expand_ownership, ExpandedEntity};
use crate::manifest::fx_master::{build_fx_master, FxRateMaster};
use crate::manifest::ic_expansion::{expand_ic_relationships, ResolvedIcRelationship};
use crate::manifest::seeds::{derive_aggregate_seed, derive_entity_seed, derive_manifest_seed};
use crate::manifest::shard_plan::{build_shard_plan, ShardPlan};
use crate::manifest::tax_plan::{build_tax_group_plan, TaxGroupPlan};

// ── Schema version ────────────────────────────────────────────────────────────

/// Schema version for the [`GroupManifest`] JSON artifact.
///
/// Bump the major component when the shape changes; aggregate-phase readers
/// must refuse to consume a manifest whose `schema_version` major component
/// differs from theirs.
pub const MANIFEST_SCHEMA_VERSION: &str = "1.0";

// ── Public types ──────────────────────────────────────────────────────────────

/// The full manifest JSON artifact that drives shard and aggregate phases.
///
/// Produced once per engagement period by [`build_manifest`].  All fields
/// are deterministic given identical [`GroupConfig`] inputs — byte-identical
/// serialized JSON is guaranteed for identical configs.
///
/// Note: `PartialEq` is not derived because [`ChartOfAccountsMaster`] wraps
/// [`datasynth_core::models::ChartOfAccounts`] which does not implement it.
/// Use serialized JSON comparison for equality/determinism tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupManifest {
    /// Format version — currently `"1.0"`.
    pub schema_version: String,
    /// Group identifier from [`GroupConfig::id`].
    pub group_id: String,
    /// Raw seed from [`GroupConfig::seed`].
    pub group_seed: u64,
    /// ISO 4217 presentation currency.
    pub presentation_currency: String,
    /// Resolved engagement period.
    pub period: ManifestPeriod,
    /// Hex-encoded blake3 digest of the manifest-phase seed (spec §2.4).
    pub manifest_seed: String,
    /// Hex-encoded blake3 digest of the aggregate-phase seed (spec §2.4).
    pub aggregate_seed: String,
    /// Ownership graph: parent + all expanded entities with per-entity seeds.
    pub ownership_graph: OwnershipGraphSection,
    /// Scoping profiles map (raw YAML values forwarded verbatim from config).
    pub scoping_profiles: BTreeMap<String, serde_yaml::Value>,
    /// Chart of accounts master — one CoA per distinct accounting framework.
    pub chart_of_accounts_master: ChartOfAccountsMaster,
    /// FX rate master — closing/average rates for IAS 21 translation.
    pub fx_rate_master: FxRateMaster,
    /// Flat list of all resolved intercompany relationships.
    pub ic_relationships: Vec<ResolvedIcRelationship>,
    /// ISA 600-level audit engagement plan (materiality, scope, component auditors).
    pub audit_engagement_plan: AuditEngagementPlan,
    /// Pillar Two / CbC / transfer-pricing group tax plan.
    pub tax_group_plan: TaxGroupPlan,
    /// Shard assignment plan — entities batched into ~1 TB shards.
    pub shard_plan: ShardPlan,
    /// Output layout config forwarded verbatim from [`GroupConfig::output`].
    pub output: OutputLayoutConfig,
    /// **v5.3** — IC matching strategy + tolerance.  Forwarded
    /// verbatim from [`GroupConfig::intercompany::matching`].  v5.2
    /// archives that don't carry this field deserialise to the
    /// `Default` (`ManifestDriven`, `tolerance = 0`) — exact-match
    /// behaviour preserved byte-for-byte.
    #[serde(default)]
    pub matching: crate::config::IcMatchingConfig,
}

/// Resolved engagement period.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestPeriod {
    /// First day of the period (inclusive).
    pub start: NaiveDate,
    /// Last day of the period (inclusive).
    pub end: NaiveDate,
    /// Period length from config.
    pub length: PeriodLength,
}

/// Ownership graph section of the manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OwnershipGraphSection {
    /// Top-level parent entity code.
    pub parent_entity_code: String,
    /// All entities (explicit + generated), enriched with `entity_seed` and `shard_id`.
    pub entities: Vec<ManifestEntity>,
}

/// An entity record in the manifest.
///
/// Combines the expanded entity data with the per-entity seed (hex-encoded)
/// and shard assignment for direct shard consumption.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestEntity {
    pub code: String,
    pub name: Option<String>,
    pub country: String,
    pub functional_currency: String,
    pub scoping_profile: String,
    pub consolidation_method: crate::config::ConsolidationMethod,
    pub ownership_percent: Option<rust_decimal::Decimal>,
    pub parent_code: Option<String>,
    pub accounting_framework: Option<String>,
    pub industry: Option<String>,
    /// **v5.2** — IAS 29 hyperinflationary status of this entity's
    /// functional currency.  `#[serde(default)]` so v5.0 / v5.1
    /// archives without the field deserialise to
    /// `NotHyperinflationary` (the default), preserving backwards
    /// compatibility byte-for-byte.
    #[serde(default)]
    pub hyperinflation_status: datasynth_core::models::HyperinflationStatus,
    /// Hex-encoded blake3 digest of the per-entity seed (spec §2.4).
    pub entity_seed: String,
    /// Shard identifier assigned by the shard plan (e.g. `"S_SIG_0001"`).
    pub shard_id: String,
}

// ── Top-level builder ─────────────────────────────────────────────────────────

/// Build the complete [`GroupManifest`] from a [`GroupConfig`].
///
/// This is the single entry point for the manifest phase.  It orchestrates all
/// Task 2.1–2.8 builders in dependency order:
///
/// 1. Period computation (start → end).
/// 2. Manifest-phase and aggregate-phase seed derivation.
/// 3. Ownership expansion (explicit + generated entities).
/// 4. Shard plan — must precede per-entity `shard_id` stamping.
/// 5. Chart of accounts master.
/// 6. FX rate master.
/// 7. Intercompany relationship expansion.
/// 8. Audit engagement plan.
/// 9. Tax group plan.
/// 10. ManifestEntity assembly (entity_seed + shard_id per entity).
///
/// # Errors
/// Propagates all errors from sub-builders.  The most common failure modes are:
/// - [`GroupError::Config`] for invalid period dates, missing materiality config,
///   or bad FX rate coverage.
/// - [`GroupError::Manifest`] for CoA load failures.
pub fn build_manifest(cfg: &GroupConfig) -> GroupResult<GroupManifest> {
    // ── 1. Resolve the engagement period ────────────────────────────────────
    let period = compute_period(&cfg.period)?;

    // ── 2. Derive manifest and aggregate seeds ───────────────────────────────
    let manifest_seed = derive_manifest_seed(cfg.seed, period.start);
    let aggregate_seed = derive_aggregate_seed(cfg.seed, period.start);

    // ── 3. Expand ownership ──────────────────────────────────────────────────
    let expanded: Vec<ExpandedEntity> = expand_ownership(&cfg.ownership, cfg.seed, period.start)?;

    // ── 4. Shard plan (must precede per-entity shard_id assignment) ──────────
    let shard_plan = build_shard_plan(&expanded, &cfg.scoping_profiles)?;
    let shard_by_code = shard_plan.shard_by_code();

    // ── 5. Chart of accounts master ──────────────────────────────────────────
    let coa_master = build_coa_master(&expanded, &cfg.defaults, &cfg.id)?;

    // ── 6. FX rate master ────────────────────────────────────────────────────
    let fx_master = build_fx_master(
        &cfg.fx,
        &cfg.presentation_currency,
        period.start,
        period.end,
        &expanded,
    )?;

    // ── 7. Intercompany relationship expansion ───────────────────────────────
    let ic_relationships = expand_ic_relationships(&cfg.intercompany, &expanded, cfg.seed)?;

    // ── 8. Audit engagement plan ─────────────────────────────────────────────
    let audit_engagement_plan =
        build_audit_engagement_plan(&cfg.audit, &expanded, &cfg.id, &aggregate_seed)?;

    // ── 9. Tax group plan ────────────────────────────────────────────────────
    let tax_group_plan = build_tax_group_plan(&cfg.tax, &expanded)?;

    // ── 10. ManifestEntity: enrich each ExpandedEntity ────────────────────────
    let entities: Vec<ManifestEntity> = expanded
        .iter()
        .map(|e| ManifestEntity {
            code: e.code.clone(),
            name: e.name.clone(),
            country: e.country.clone(),
            functional_currency: e.functional_currency.clone(),
            scoping_profile: e.scoping_profile.clone(),
            consolidation_method: e.consolidation_method,
            ownership_percent: e.ownership_percent,
            parent_code: e.parent_code.clone(),
            accounting_framework: e.accounting_framework.clone(),
            industry: e.industry.clone(),
            hyperinflation_status: e.hyperinflation_status,
            entity_seed: hex::encode(derive_entity_seed(cfg.seed, &e.code)),
            shard_id: shard_by_code
                .get(e.code.as_str())
                .cloned()
                .unwrap_or_default(),
        })
        .collect();

    Ok(GroupManifest {
        schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
        group_id: cfg.id.clone(),
        group_seed: cfg.seed,
        presentation_currency: cfg.presentation_currency.clone(),
        period,
        manifest_seed: hex::encode(manifest_seed),
        aggregate_seed: hex::encode(aggregate_seed),
        ownership_graph: OwnershipGraphSection {
            parent_entity_code: cfg.ownership.parent_entity_code.clone(),
            entities,
        },
        scoping_profiles: cfg.scoping_profiles.clone(),
        chart_of_accounts_master: coa_master,
        fx_rate_master: fx_master,
        ic_relationships,
        audit_engagement_plan,
        tax_group_plan,
        shard_plan,
        output: cfg.output.clone(),
        // v5.3: IC matching strategy + fuzzy tolerance.  Defaults
        // produced by `IcMatchingConfig::default()` match the v5.0
        // exact-match behaviour byte-for-byte.
        matching: cfg.intercompany.matching.clone(),
    })
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Compute the [`ManifestPeriod`] from a [`crate::config::PeriodConfig`].
///
/// The period `end` date is the last calendar day *within* the period:
/// - Monthly  → start + 1 month − 1 day
/// - Quarterly → start + 3 months − 1 day
/// - SemiAnnual → start + 6 months − 1 day
/// - Annual  → start + 12 months − 1 day
fn compute_period(cfg: &crate::config::PeriodConfig) -> GroupResult<ManifestPeriod> {
    compute_period_pub(cfg)
}

/// Public re-export of period computation for integration testing.
///
/// Exposed so `tests/manifest_builder.rs` can exercise period-end edge cases
/// (leap-year February, quarter boundaries, etc.) without going through the
/// full `build_manifest` stack.
pub fn compute_period_pub(cfg: &crate::config::PeriodConfig) -> GroupResult<ManifestPeriod> {
    let start = cfg.start_date;
    let end = match cfg.length {
        PeriodLength::Monthly => start.checked_add_months(chrono::Months::new(1)),
        PeriodLength::Quarterly => start.checked_add_months(chrono::Months::new(3)),
        PeriodLength::SemiAnnual => start.checked_add_months(chrono::Months::new(6)),
        PeriodLength::Annual => start.checked_add_months(chrono::Months::new(12)),
    }
    .and_then(|d| d.pred_opt())
    .ok_or_else(|| {
        GroupError::Config(format!(
            "invalid period start_date '{}' — cannot compute period end",
            start
        ))
    })?;

    Ok(ManifestPeriod {
        start,
        end,
        length: cfg.length,
    })
}

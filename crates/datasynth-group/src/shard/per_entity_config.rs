//! Per-entity [`GeneratorConfig`] builder (Task 4.2).
//!
//! Projects a [`ManifestEntity`] into a single-company [`GeneratorConfig`]
//! suitable for invocation of the standalone `EnhancedOrchestrator`.
//!
//! # Architecture note — ManifestEntity vs ResolvedEntity
//!
//! This builder takes `(&GroupManifest, &ManifestEntity)` — *not*
//! [`crate::resolve::ResolvedEntity`] — because the manifest builder
//! (see `manifest/builder.rs`) has already flattened the three-level
//! merge (`defaults → scoping_profile → entity overrides`) into every
//! [`ManifestEntity`]'s scalar fields.  Re-running
//! [`crate::resolve::resolve_entity`] here would duplicate that work
//! and force callers to carry the full `GroupConfig` alongside the
//! manifest — a layering violation that offers no benefit for v5.0.
//!
//! Everything the builder needs (`code`, `country`, `functional_currency`,
//! `industry`, `name`, `scoping_profile`) is already on
//! [`ManifestEntity`]; the scoping profile's `row_budget` is read from
//! `manifest.scoping_profiles[entity.scoping_profile]["row_budget"]` with
//! a 100,000-row default if the key is absent.
//!
//! # v5.0 scope
//!
//! `accounting_framework` is not threaded through to a dedicated
//! [`GeneratorConfig`] field in v5.0 — it is documented on the
//! [`ManifestEntity`] for aggregate-phase consumption (US GAAP ↔ IFRS
//! bridge packs) but does not gate generator behaviour at the shard
//! level.  `process_models` from the resolved profile are likewise not
//! yet wired into [`GeneratorConfig`]; the preset's
//! `business_processes` weights stand in until the process-selection
//! surface lands in a later task.  This matches the plan's Task 4.2
//! acceptance criteria (a valid config that builds for every entity),
//! not the full enablement matrix.

use datasynth_config::presets::create_preset;
use datasynth_config::{CompanyConfig, GeneratorConfig, TransactionVolume};
use datasynth_core::models::{CoAComplexity, IndustrySector};

use crate::config::PeriodLength;
use crate::errors::{GroupError, GroupResult};
use crate::manifest::builder::{GroupManifest, ManifestEntity};

/// Default row budget applied when a scoping profile omits `row_budget`.
const DEFAULT_ROW_BUDGET: u64 = 100_000;

/// Build the [`GeneratorConfig`] for a single manifest entity.
///
/// The caller already holds both `manifest` and `entity` when dispatching
/// a shard (see spec §5 "Shard phase") — every field needed to drive the
/// orchestrator is either on the [`ManifestEntity`] directly or reachable
/// via the manifest's scoping-profile map.
///
/// The resulting config:
///
/// - carries exactly **one** company keyed by `entity.code`, with
///   `currency` / `functional_currency` / `country` taken verbatim from
///   `entity`;
/// - stamps the manifest's `group_seed` onto `global.seed`, the
///   presentation currency onto `global.group_currency` /
///   `global.presentation_currency`, and `period.start` onto
///   `global.start_date`;
/// - sizes the company's `annual_transaction_volume` off the scoping
///   profile's `row_budget` via the standard [`TransactionVolume`]
///   buckets (see [`volume_from_rows`]);
/// - validates against the `datasynth-config` schema before returning —
///   downstream callers can rely on it being valid.
///
/// # Errors
///
/// Returns [`GroupError::Config`] if the synthesized [`GeneratorConfig`]
/// fails [`datasynth_config::validate_config`] — this should only
/// happen if one of the manifest-provided scalars (start_date,
/// functional_currency, country) is itself malformed, which the
/// manifest builder should already have caught.
pub fn build_entity_generator_config(
    manifest: &GroupManifest,
    entity: &ManifestEntity,
) -> GroupResult<GeneratorConfig> {
    // 1. Derive industry, period months, volume, and complexity.
    let industry = map_industry(entity.industry.as_deref());
    let period_months = period_months_from_length(manifest.period.length);
    let row_budget = lookup_row_budget(manifest, &entity.scoping_profile);
    let volume = volume_from_rows(row_budget);
    let complexity = CoAComplexity::Medium;

    // 2. Start from a fully-populated preset template so every nested
    //    config struct has sensible defaults.  We seed it with `1` company
    //    because we immediately replace the companies vector below.
    let mut cfg = create_preset(industry, 1, period_months, complexity, volume);

    // 3. Global overrides — stamp the manifest-derived values.
    cfg.global.seed = Some(manifest.group_seed);
    cfg.global.industry = industry;
    cfg.global.start_date = manifest.period.start.format("%Y-%m-%d").to_string();
    cfg.global.period_months = period_months;
    cfg.global.group_currency = manifest.presentation_currency.clone();
    cfg.global.presentation_currency = Some(manifest.presentation_currency.clone());

    // 3a. Enable phases the v5.0 aggregate engine consumes.
    //     The preset's defaults leave `financial_reporting.enabled = false`
    //     and the orchestrator's Phase 15 (financial reporting) is skipped —
    //     which means `result.financial_reporting.trial_balances` stays empty
    //     and `output_writer` never emits `period_close/trial_balances.json`.
    //     The aggregate phase's `tb_loader` reads that exact file, so without
    //     this flag the whole `group generate` pipeline fails at run_aggregate
    //     with "missing shard archive". Force-enable it for every shard.
    cfg.financial_reporting.enabled = true;

    // 3b. Disable banking / KYC / AML generation in shard mode.
    //     `BankingConfig::enabled` defaults to `true` and the orchestrator
    //     emits per-entity banking JSON archives that average ~29 GB each
    //     (driven by `aml_transaction_labels.json` at ~6.5 GB / entity for
    //     a quarterly period). At enterprise-2000 scale that's 58 TB — far
    //     beyond any practical disk budget. The companion
    //     `vynfi-aml-100k` HF dataset is the v5.0 banking showcase; the
    //     group-audit pipeline doesn't consume banking data downstream,
    //     so disabling it for shard generation has zero effect on the
    //     consolidated archive.
    cfg.banking.enabled = false;

    // 3c. Per-entity scoping-budget scale (closes #148 — v5.29 regen of
    //     enterprise-2000 OOM-killed the aggregate phase because each entity
    //     emitted ~100 K JEs regardless of row_budget: document_flows,
    //     manufacturing, period_close, and master_data generators each have
    //     their own per-month / per-year counts that the preset wires
    //     independent of `annual_transaction_volume`. At 2000 entities ×
    //     ~700 MB / entity = ~1 TB raw + the aggregate phase loading 2000
    //     trial balances into memory simultaneously → 228 GB RSS / OOM-kill.
    //
    //     Scale every process-count knob proportional to `row_budget /
    //     REFERENCE_BUDGET` where REFERENCE_BUDGET = 100_000 is the volume
    //     the preset is sized for. A "limited" scoping at row_budget=200
    //     then gets ≈0.002× the chain counts → ~200 P2P-derived JEs
    //     instead of 36 K. The full archive at 2000 entities lands in the
    //     ~30 GB band the v5.10 release targeted.
    const REFERENCE_BUDGET: u64 = 100_000;
    let scale = (row_budget as f64 / REFERENCE_BUDGET as f64).clamp(0.001, 10.0);
    let scale_usize = |n: usize| -> usize { ((n as f64) * scale).round().max(1.0) as usize };
    let scale_u32 = |n: u32| -> u32 { ((n as f64) * scale).round().max(1.0) as u32 };

    // P2P + O2C document-flow chain counts are derived from
    // `master_data.{vendors,customers}.count` × period_months, so scaling
    // master_data drives the chain count down proportionally. (The P2P /
    // O2C structs themselves carry rates, not absolute counts.)
    cfg.master_data.vendors.count = scale_usize(cfg.master_data.vendors.count).max(5);
    cfg.master_data.customers.count = scale_usize(cfg.master_data.customers.count).max(5);
    cfg.master_data.materials.count = scale_usize(cfg.master_data.materials.count).max(5);
    cfg.master_data.fixed_assets.count = scale_usize(cfg.master_data.fixed_assets.count).max(3);
    cfg.master_data.employees.count = scale_usize(cfg.master_data.employees.count).max(3);

    // Manufacturing — disabled below 5K row_budget (limited tier).
    // Above, scale production-order monthly throughput. Quality + cycle
    // counts derive from production orders so scale once at the source.
    if row_budget < 5_000 {
        cfg.manufacturing.enabled = false;
    } else {
        cfg.manufacturing.production_orders.orders_per_month =
            scale_u32(cfg.manufacturing.production_orders.orders_per_month).max(1);
    }

    // Audit workpapers — leave the `generate_workpapers` toggle to the
    // scoping_profile (limited scopes already set it to `false`); no
    // numeric count knob exists on `AuditGenerationConfig` to scale.

    // 3d. Note for v3 sizing: master_data.{vendors,customers,materials}
    //     scaling lands a "significant" entity (row_budget=5_000) at
    //     0.05× → 15 vendors, 30 customers, 75 materials. A "limited"
    //     entity (row_budget=200) lands at 0.002× → the 5-vendor floor.
    //     This brings 2000-entity aggregate raw size from ~800 GB to
    //     an expected ~30-40 GB, matching the v5.10 archive target.

    // 4. Replace companies with a single entry tailored to this shard.
    //    `fiscal_year_variant` defaults to "K4" in the schema — we hard-code
    //    it here since `default_fiscal_variant` is private to datasynth-config.
    cfg.companies = vec![CompanyConfig {
        code: entity.code.clone(),
        name: entity.name.clone().unwrap_or_else(|| entity.code.clone()),
        currency: entity.functional_currency.clone(),
        functional_currency: Some(entity.functional_currency.clone()),
        country: entity.country.clone(),
        fiscal_year_variant: "K4".to_string(),
        annual_transaction_volume: volume,
        volume_weight: 1.0,
    }];

    // 5. Validate.  Any failure here is effectively a builder bug — surface
    //    it with the entity code so the caller can pinpoint the shard.
    datasynth_config::validate_config(&cfg).map_err(|e| {
        GroupError::Config(format!(
            "per-entity GeneratorConfig failed validation for {}: {e}",
            entity.code
        ))
    })?;

    Ok(cfg)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Map the entity's optional `industry` string to an [`IndustrySector`].
///
/// Unknown or absent values fall back to `Manufacturing` — the preset
/// template has the widest coverage there, so downstream generators still
/// produce something sensible even when the YAML carries a typo or
/// leaves the field unset.
fn map_industry(s: Option<&str>) -> IndustrySector {
    match s.map(|v| v.to_ascii_lowercase()).as_deref() {
        Some("manufacturing") => IndustrySector::Manufacturing,
        Some("retail") => IndustrySector::Retail,
        Some("financial_services" | "banking" | "finance") => IndustrySector::FinancialServices,
        Some("healthcare" | "pharma" | "pharmaceutical") => IndustrySector::Healthcare,
        Some("technology" | "tech" | "software") => IndustrySector::Technology,
        Some("professional_services" | "consulting") => IndustrySector::ProfessionalServices,
        Some("energy" | "oil_gas" | "utilities") => IndustrySector::Energy,
        Some("transportation" | "logistics") => IndustrySector::Transportation,
        Some("real_estate") => IndustrySector::RealEstate,
        Some("telecommunications" | "telecom") => IndustrySector::Telecommunications,
        _ => IndustrySector::Manufacturing,
    }
}

/// Convert [`PeriodLength`] into the `period_months` integer the
/// `datasynth-config` schema expects.
fn period_months_from_length(len: PeriodLength) -> u32 {
    match len {
        PeriodLength::Monthly => 1,
        PeriodLength::Quarterly => 3,
        PeriodLength::SemiAnnual => 6,
        PeriodLength::Annual => 12,
    }
}

/// Look up the scoping profile's `row_budget`, defaulting to
/// [`DEFAULT_ROW_BUDGET`] if the key is missing or the profile name
/// doesn't resolve.
fn lookup_row_budget(manifest: &GroupManifest, profile: &str) -> u64 {
    manifest
        .scoping_profiles
        .get(profile)
        .and_then(|v| v.as_mapping())
        .and_then(|m| m.get(serde_yaml::Value::String("row_budget".to_string())))
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_ROW_BUDGET)
}

/// Bucket a row budget into a [`TransactionVolume`] preset.
///
/// The mapping is intentionally lossy — v5.0 doesn't need byte-exact
/// alignment between `row_budget` and the orchestrator's volume counter.
/// Callers who need exact control can bypass this helper entirely and
/// construct `TransactionVolume::Custom(n)` themselves.
fn volume_from_rows(rows: u64) -> TransactionVolume {
    // Sub-TenK budgets (`limited` / `material` scoping tiers in
    // enterprise_2000-class configs) must NOT be rounded up to TenK —
    // that's the original #148 root cause: a limited entity with
    // row_budget=200 emitted ~10 K JEs instead of ~200, and the
    // ensemble of 2000 entities ran the aggregate phase OOM. Sub-TenK
    // values get a `Custom(rows)` volume; bucketed values above TenK
    // continue to use the canonical preset volumes (where the
    // generator has tier-specific tuning baked in).
    if rows < 10_000 {
        TransactionVolume::Custom(rows)
    } else if rows <= 10_000 {
        TransactionVolume::TenK
    } else if rows <= 100_000 {
        TransactionVolume::HundredK
    } else if rows <= 1_000_000 {
        TransactionVolume::OneM
    } else if rows <= 10_000_000 {
        TransactionVolume::TenM
    } else {
        TransactionVolume::HundredM
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── map_industry ──────────────────────────────────────────────────────────

    #[test]
    fn map_industry_covers_every_variant() {
        assert_eq!(
            map_industry(Some("manufacturing")),
            IndustrySector::Manufacturing
        );
        assert_eq!(map_industry(Some("retail")), IndustrySector::Retail);
        assert_eq!(
            map_industry(Some("financial_services")),
            IndustrySector::FinancialServices
        );
        assert_eq!(
            map_industry(Some("banking")),
            IndustrySector::FinancialServices
        );
        assert_eq!(
            map_industry(Some("finance")),
            IndustrySector::FinancialServices
        );
        assert_eq!(map_industry(Some("healthcare")), IndustrySector::Healthcare);
        assert_eq!(map_industry(Some("pharma")), IndustrySector::Healthcare);
        assert_eq!(
            map_industry(Some("pharmaceutical")),
            IndustrySector::Healthcare
        );
        assert_eq!(map_industry(Some("technology")), IndustrySector::Technology);
        assert_eq!(map_industry(Some("tech")), IndustrySector::Technology);
        assert_eq!(map_industry(Some("software")), IndustrySector::Technology);
        assert_eq!(
            map_industry(Some("professional_services")),
            IndustrySector::ProfessionalServices
        );
        assert_eq!(
            map_industry(Some("consulting")),
            IndustrySector::ProfessionalServices
        );
        assert_eq!(map_industry(Some("energy")), IndustrySector::Energy);
        assert_eq!(map_industry(Some("oil_gas")), IndustrySector::Energy);
        assert_eq!(map_industry(Some("utilities")), IndustrySector::Energy);
        assert_eq!(
            map_industry(Some("transportation")),
            IndustrySector::Transportation
        );
        assert_eq!(
            map_industry(Some("logistics")),
            IndustrySector::Transportation
        );
        assert_eq!(
            map_industry(Some("real_estate")),
            IndustrySector::RealEstate
        );
        assert_eq!(
            map_industry(Some("telecommunications")),
            IndustrySector::Telecommunications
        );
        assert_eq!(
            map_industry(Some("telecom")),
            IndustrySector::Telecommunications
        );
    }

    #[test]
    fn map_industry_is_case_insensitive() {
        assert_eq!(
            map_industry(Some("MANUFACTURING")),
            IndustrySector::Manufacturing
        );
        assert_eq!(map_industry(Some("Retail")), IndustrySector::Retail);
        assert_eq!(
            map_industry(Some("FINANCIAL_SERVICES")),
            IndustrySector::FinancialServices
        );
    }

    #[test]
    fn map_industry_unknown_defaults_to_manufacturing() {
        assert_eq!(
            map_industry(Some("spacefaring_megacorp")),
            IndustrySector::Manufacturing
        );
        assert_eq!(map_industry(Some("")), IndustrySector::Manufacturing);
    }

    #[test]
    fn map_industry_none_defaults_to_manufacturing() {
        assert_eq!(map_industry(None), IndustrySector::Manufacturing);
    }

    // ── period_months_from_length ─────────────────────────────────────────────

    #[test]
    fn period_months_covers_every_length() {
        assert_eq!(period_months_from_length(PeriodLength::Monthly), 1);
        assert_eq!(period_months_from_length(PeriodLength::Quarterly), 3);
        assert_eq!(period_months_from_length(PeriodLength::SemiAnnual), 6);
        assert_eq!(period_months_from_length(PeriodLength::Annual), 12);
    }

    // ── volume_from_rows — boundary and inside-bucket coverage ────────────────

    #[test]
    fn volume_from_rows_sub_tenk_uses_custom() {
        // Sub-TenK budgets (limited / material scoping in enterprise_2000-
        // class configs) must NOT round up — that was the #148 root cause.
        // Custom(rows) lets a row_budget=200 limited entity emit ~200 JEs
        // rather than getting bumped to 10_000.
        assert!(matches!(
            volume_from_rows(200),
            TransactionVolume::Custom(200)
        ));
        assert!(matches!(
            volume_from_rows(1_000),
            TransactionVolume::Custom(1_000)
        ));
        assert!(matches!(
            volume_from_rows(5_000),
            TransactionVolume::Custom(5_000)
        ));
        assert!(matches!(
            volume_from_rows(9_999),
            TransactionVolume::Custom(9_999)
        ));
        // Exact TenK boundary stays on the canonical tier.
        assert!(matches!(volume_from_rows(10_000), TransactionVolume::TenK));
    }

    #[test]
    fn volume_from_rows_honours_bucket_boundaries() {
        // Each bucket's upper edge goes *into* that bucket; one above flips
        // to the next bucket.
        assert!(matches!(
            volume_from_rows(10_001),
            TransactionVolume::HundredK
        ));
        assert!(matches!(
            volume_from_rows(100_000),
            TransactionVolume::HundredK
        ));
        assert!(matches!(volume_from_rows(100_001), TransactionVolume::OneM));
        assert!(matches!(
            volume_from_rows(1_000_000),
            TransactionVolume::OneM
        ));
        assert!(matches!(
            volume_from_rows(1_000_001),
            TransactionVolume::TenM
        ));
        assert!(matches!(
            volume_from_rows(10_000_000),
            TransactionVolume::TenM
        ));
        assert!(matches!(
            volume_from_rows(10_000_001),
            TransactionVolume::HundredM
        ));
    }

    #[test]
    fn volume_from_rows_interior_samples() {
        // Inside-bucket samples — useful as a sanity check that no off-by-one
        // pulled a mid-bucket value into an adjacent bucket.
        assert!(matches!(
            volume_from_rows(50_000),
            TransactionVolume::HundredK
        ));
        assert!(matches!(volume_from_rows(500_000), TransactionVolume::OneM));
        assert!(matches!(
            volume_from_rows(5_000_000),
            TransactionVolume::TenM
        ));
        assert!(matches!(
            volume_from_rows(50_000_000),
            TransactionVolume::HundredM
        ));
    }

    #[test]
    fn volume_from_rows_saturates_at_hundredm() {
        // Anything above 10M lands in the top bucket — no Custom(n) promotion.
        assert!(matches!(
            volume_from_rows(u64::MAX),
            TransactionVolume::HundredM
        ));
    }
}

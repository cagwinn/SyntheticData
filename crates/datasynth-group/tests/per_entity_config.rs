//! Task 4.2 — `build_entity_generator_config` integration tests.
//!
//! These tests drive `datasynth_group::shard::build_entity_generator_config`
//! from the mini_acme reference fixture and verify that the per-entity
//! [`GeneratorConfig`] it produces:
//!
//! - validates against the `datasynth-config` schema;
//! - reflects the manifest's global fields (period, seed, presentation
//!   currency);
//! - carries exactly one `CompanyConfig` tailored to the entity;
//! - respects the scoping profile's `row_budget` via the [`TransactionVolume`]
//!   bucketing;
//! - remains deterministic across repeated calls with the same inputs.
//!
//! [`GeneratorConfig`]: datasynth_config::GeneratorConfig
//! [`TransactionVolume`]: datasynth_config::TransactionVolume

use datasynth_config::{GeneratorConfig, TransactionVolume};
use datasynth_core::models::IndustrySector;
use datasynth_group::manifest::builder::{GroupManifest, ManifestEntity};
use datasynth_group::shard::build_entity_generator_config;
use datasynth_group::{build_manifest, GroupConfig};

// ── Fixture loading ───────────────────────────────────────────────────────────

/// Load the mini_acme fixture's [`GroupManifest`] — all integration tests
/// here drive a real manifest rather than a hand-built one because the
/// builder relies on the `scoping_profiles` map being populated.
fn load_mini_acme_manifest() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_acme.yaml");
    let cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_acme.yaml must parse into GroupConfig");
    build_manifest(&cfg).expect("mini_acme.yaml must build a manifest")
}

/// Look up a manifest entity by code — every integration test below starts
/// from a named entity, so a small helper keeps the tests readable.
fn entity_by_code<'a>(manifest: &'a GroupManifest, code: &str) -> &'a ManifestEntity {
    manifest
        .ownership_graph
        .entities
        .iter()
        .find(|e| e.code == code)
        .unwrap_or_else(|| panic!("mini_acme fixture must carry entity {code}"))
}

// ── Integration tests ────────────────────────────────────────────────────────

/// Every entity in the mini_acme fixture must produce a valid
/// [`GeneratorConfig`] with a single company keyed by the entity's own
/// code, currency, and country.
#[test]
fn test_builds_valid_config_for_every_entity() {
    let manifest = load_mini_acme_manifest();
    assert!(
        !manifest.ownership_graph.entities.is_empty(),
        "mini_acme fixture must have entities"
    );

    for entity in &manifest.ownership_graph.entities {
        let cfg = build_entity_generator_config(&manifest, entity)
            .unwrap_or_else(|e| panic!("build must succeed for {}: {e}", entity.code));

        datasynth_config::validate_config(&cfg)
            .unwrap_or_else(|e| panic!("validate_config must succeed for {}: {e}", entity.code));

        assert_eq!(
            cfg.companies.len(),
            1,
            "{} must produce exactly one company; got {}",
            entity.code,
            cfg.companies.len()
        );

        let company = &cfg.companies[0];
        assert_eq!(
            company.code, entity.code,
            "{} company.code must match entity code",
            entity.code
        );
        assert_eq!(
            company.currency, entity.functional_currency,
            "{} company.currency must equal entity functional_currency",
            entity.code
        );
        assert_eq!(
            company.functional_currency,
            Some(entity.functional_currency.clone()),
            "{} company.functional_currency mirror must match",
            entity.code
        );
        assert_eq!(
            company.country, entity.country,
            "{} company.country must match entity country",
            entity.code
        );
    }
}

/// ACME_SA is the IFRS-reporting parent; its per-entity config must
/// carry the manifest's quarterly period, CHF presentation currency, and
/// fixture seed verbatim.
#[test]
fn test_period_and_currency_flow_through() {
    let manifest = load_mini_acme_manifest();
    let entity = entity_by_code(&manifest, "ACME_SA");
    let cfg = build_entity_generator_config(&manifest, entity).expect("ACME_SA must build");

    assert_eq!(cfg.global.start_date, "2024-01-01");
    assert_eq!(cfg.global.period_months, 3, "Quarterly → 3 months");
    assert_eq!(cfg.global.group_currency, "CHF");
    assert_eq!(cfg.global.presentation_currency, Some("CHF".to_string()));
    assert_eq!(cfg.global.seed, Some(0x1234567890ABCDEF));
    assert_eq!(cfg.global.seed, Some(manifest.group_seed));
    // mini_acme's `defaults.industry = manufacturing` stamps this.
    assert_eq!(cfg.global.industry, IndustrySector::Manufacturing);
}

/// Every mini_acme entity inherits `defaults.industry = manufacturing`,
/// so every built config must report Manufacturing.  As a negative
/// control, a synthetic entity with `industry: None` must still default
/// to Manufacturing rather than erroring out.
#[test]
fn test_industry_mapping_defaults_to_manufacturing_when_unset() {
    let manifest = load_mini_acme_manifest();

    // Every real entity in the fixture already has `industry = Some("manufacturing")`
    // after three-level resolution, so they all map to Manufacturing.
    for entity in &manifest.ownership_graph.entities {
        let cfg = build_entity_generator_config(&manifest, entity)
            .unwrap_or_else(|e| panic!("build must succeed for {}: {e}", entity.code));
        assert_eq!(
            cfg.global.industry,
            IndustrySector::Manufacturing,
            "{} should inherit Manufacturing from defaults.industry",
            entity.code,
        );
    }

    // Synthetic entity with industry=None — the builder must still accept
    // it and default to Manufacturing.  We clone from a real fixture entity
    // to pick up a real `scoping_profile`, `entity_seed`, `shard_id` — the
    // only field we flip is `industry`.
    let synthetic = ManifestEntity {
        industry: None,
        hyperinflation_status: datasynth_core::models::HyperinflationStatus::NotHyperinflationary,
        ..entity_by_code(&manifest, "ACME_SA").clone()
    };
    let cfg = build_entity_generator_config(&manifest, &synthetic)
        .expect("synthetic entity with industry=None must build");
    assert_eq!(cfg.global.industry, IndustrySector::Manufacturing);
}

/// `row_budget` drives [`TransactionVolume`] bucketing.  The mini_acme
/// fixture has two profiles: `significant` (100_000 rows → `HundredK`)
/// and `material` (25_000 rows → `HundredK`; `25_000 ≤ 100_000`).  Both
/// should produce a company with `annual_transaction_volume == HundredK`.
#[test]
fn test_row_budget_controls_volume() {
    let manifest = load_mini_acme_manifest();

    // ACME_SA is in `significant` (row_budget = 100_000).
    let sig_entity = entity_by_code(&manifest, "ACME_SA");
    assert_eq!(sig_entity.scoping_profile, "significant");
    let sig_cfg = build_entity_generator_config(&manifest, sig_entity).expect("must build");
    assert_eq!(
        sig_cfg.companies[0].annual_transaction_volume.count(),
        TransactionVolume::HundredK.count(),
        "significant (row_budget=100_000) → HundredK",
    );

    // ACME_BR is in `material` (row_budget = 25_000).
    let mat_entity = entity_by_code(&manifest, "ACME_BR");
    assert_eq!(mat_entity.scoping_profile, "material");
    let mat_cfg = build_entity_generator_config(&manifest, mat_entity).expect("must build");
    assert_eq!(
        mat_cfg.companies[0].annual_transaction_volume.count(),
        TransactionVolume::HundredK.count(),
        "material (row_budget=25_000) → HundredK (25_000 ≤ 100_000)",
    );
}

/// Two calls with identical manifest + entity inputs must produce
/// configs that agree on every field this builder is responsible for
/// setting — the global section (seed, period, industry, currencies)
/// and the companies vector (code, name, currency, functional currency,
/// country, fiscal variant, volume, weight).
///
/// We deliberately don't compare the full serialized YAML — the
/// `BankingConfig` embedded in [`GeneratorConfig`] uses `HashMap`-backed
/// persona-weight maps whose serialization order is not guaranteed.
/// That's a pre-existing upstream quirk, not a defect in this builder.
#[test]
fn test_deterministic_across_calls() {
    let manifest = load_mini_acme_manifest();
    let entity = entity_by_code(&manifest, "ACME_SA");

    let a: GeneratorConfig =
        build_entity_generator_config(&manifest, entity).expect("first call must succeed");
    let b: GeneratorConfig =
        build_entity_generator_config(&manifest, entity).expect("second call must succeed");

    // Global section — the builder sets every field asserted here.
    assert_eq!(a.global.seed, b.global.seed);
    assert_eq!(a.global.industry, b.global.industry);
    assert_eq!(a.global.start_date, b.global.start_date);
    assert_eq!(a.global.period_months, b.global.period_months);
    assert_eq!(a.global.group_currency, b.global.group_currency);
    assert_eq!(
        a.global.presentation_currency,
        b.global.presentation_currency
    );

    // Companies vector — exactly one entry, every field deterministic.
    assert_eq!(a.companies.len(), b.companies.len());
    for (ca, cb) in a.companies.iter().zip(b.companies.iter()) {
        assert_eq!(ca.code, cb.code);
        assert_eq!(ca.name, cb.name);
        assert_eq!(ca.currency, cb.currency);
        assert_eq!(ca.functional_currency, cb.functional_currency);
        assert_eq!(ca.country, cb.country);
        assert_eq!(ca.fiscal_year_variant, cb.fiscal_year_variant);
        assert_eq!(
            ca.annual_transaction_volume.count(),
            cb.annual_transaction_volume.count(),
        );
        assert!(
            (ca.volume_weight - cb.volume_weight).abs() < f64::EPSILON,
            "volume_weight mismatch: {} vs {}",
            ca.volume_weight,
            cb.volume_weight,
        );
    }
}

// ── Sanity: ACME_USA carries US-specific scalars ────────────────────────────

/// ACME_USA has `functional_currency: USD`, `country: US` in the fixture.
/// Even though `accounting_framework: us_gaap` is not threaded through to
/// the v5.0 [`GeneratorConfig`] surface, the company still reflects the
/// US scalars — that's the observable difference from the IFRS parent.
#[test]
fn test_us_entity_carries_us_scalars() {
    let manifest = load_mini_acme_manifest();
    let entity = entity_by_code(&manifest, "ACME_USA");
    let cfg = build_entity_generator_config(&manifest, entity).expect("ACME_USA must build");

    let company = &cfg.companies[0];
    assert_eq!(company.code, "ACME_USA");
    assert_eq!(company.country, "US");
    assert_eq!(company.currency, "USD");
    assert_eq!(company.functional_currency, Some("USD".to_string()));
}

// ── v5.33.2 — opening-balance generation force-enabled ─────────────────────

/// Regression guard for the v5.32 FINDINGS gap: the per-entity chain
/// config must force-enable `balance.generate_opening_balances` so the
/// orchestrator's Phase 3b runs for every shard. Without this, the
/// schema default `false` flows through, Phase 3b silently skips, and
/// `balance/opening_balances.json` never lands on disk — masking the
/// v5.3 ShardContext carry-forward in chain runs.
#[test]
fn test_per_entity_config_force_enables_opening_balance_generation() {
    let manifest = load_mini_acme_manifest();
    for entity in &manifest.ownership_graph.entities {
        let cfg = build_entity_generator_config(&manifest, entity)
            .unwrap_or_else(|e| panic!("build must succeed for {}: {e}", entity.code));
        assert!(
            cfg.balance.generate_opening_balances,
            "{}: opening-balance generation must be force-enabled for chain shards",
            entity.code,
        );
    }
}

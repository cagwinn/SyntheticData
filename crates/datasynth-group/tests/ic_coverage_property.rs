//! Task 11.2 — IC matching coverage property test.
//!
//! In v5.0 the IC matching pipeline is fully manifest-driven: both the
//! seller and the buyer derive the same `pair_id` from
//! [`derive_ic_pair_plans`] and emit byte-identical headers via
//! [`inject_ic_journal_entries`].  No fuzzy matching, no amount or date
//! tolerance — every planned pair must match exactly.  Coverage on the
//! [`match_ic_pairs`] result is therefore **1.0 by construction** for
//! any well-formed [`GroupConfig`].
//!
//! This file randomizes 10 group configurations and asserts that
//! coverage is exactly 1.0 every time:
//!
//! - 10 seeds (`0..10`) drive a deterministic `ChaCha8Rng`.
//! - Entity counts vary across `3, 5, 7, 9, 11, 13, 15, 5, 7, 9`.
//! - IC relationship counts vary across `1, 2, 3, 4, 5, 6, 7, 8, 4, 6`.
//! - All entities share the presentation currency (CHF) so the test
//!   doesn't need an FX rate table.
//! - All consolidations use [`ConsolidationMethod::Full`] except the
//!   parent which is [`ConsolidationMethod::Parent`].
//!
//! # Why no orchestrator
//!
//! The orchestrator path peaks at ~17 GiB RSS per entity.  A 10-seed
//! property test that ran the orchestrator would multiply that by 100×
//! and OOM the host.  Instead we drive the manifest-derived sub-pieces
//! directly:
//!
//! 1. [`build_manifest`] over the synthetic config (deterministic, in
//!    memory, ~10 ms).
//! 2. For each entity, [`derive_ic_pair_plans`] +
//!    [`inject_ic_journal_entries`] (~µs).
//! 3. [`match_ic_pairs`] over the resulting cross-entity JE bag (~ms).
//!
//! Total wall-clock per iteration is well under 100 ms, so all 10 seeds
//! finish in under a second.  Cheap → no `#[ignore]`.

use chrono::NaiveDate;
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

use datasynth_core::models::JournalEntry;
use datasynth_group::shard::{derive_ic_pair_plans, inject_ic_journal_entries, InjectionCtx};
use datasynth_group::{
    build_manifest, match_ic_pairs, ConsolidationMethod, EntityConfig, FxConfig, FxPolicyConfig,
    FxRateBasis, FxRateSource, GroupConfig, GroupMaterialityConfig, IcMatchingConfig,
    IcRelationshipConfig, IcRelationshipExplicit, IcTransactionType, IntercompanyConfig,
    MaterialityBasis, OutputLayoutConfig, OwnershipConfig, PeriodConfig, PeriodLength,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

const TX_TYPES: [IcTransactionType; 8] = [
    IcTransactionType::GoodsSale,
    IcTransactionType::ServiceProvided,
    IcTransactionType::ManagementFee,
    IcTransactionType::Royalty,
    IcTransactionType::CostSharing,
    IcTransactionType::LoanInterest,
    IcTransactionType::Dividend,
    IcTransactionType::ExpenseRecharge,
];

/// Build a deterministic single-currency [`GroupConfig`] suitable for
/// IC-matching property tests.  All entities use CHF (so no FX rate
/// table needed) and use [`ConsolidationMethod::Full`] (so they all
/// land in the IC matcher's scope) except the first entity which acts
/// as the [`ConsolidationMethod::Parent`].
///
/// `entity_count` must be ≥ 2 (parent + at least one child for IC
/// pairs to make sense).
///
/// `ic_relationship_count` controls how many explicit IC relationships
/// land in [`IntercompanyConfig::relationships`].  Each relationship's
/// (seller, buyer) pair is sampled from the entity list; we enforce
/// `seller != buyer` and we enforce that no `(seller, buyer, type)`
/// triple is repeated, because [`expand_ic_relationships`] silently
/// drops duplicate explicit triples but we want every relationship in
/// the config to materialize.
fn random_group_config(
    seed: u64,
    entity_count: usize,
    ic_relationship_count: usize,
) -> GroupConfig {
    assert!(
        entity_count >= 2,
        "property test must have ≥ 2 entities; got {entity_count}",
    );
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    // Currencies — kept narrow so no FX table is needed (all CHF).
    let currency = "CHF";

    // Build entities: E001..E0NN.  First is parent.
    let entities: Vec<EntityConfig> = (0..entity_count)
        .map(|i| {
            let code = format!("E{:03}", i + 1);
            EntityConfig {
                code,
                name: None,
                country: "CH".to_string(),
                functional_currency: currency.to_string(),
                scoping_profile: "significant".to_string(),
                consolidation_method: if i == 0 {
                    ConsolidationMethod::Parent
                } else {
                    ConsolidationMethod::Full
                },
                ownership_percent: if i == 0 { None } else { Some(Decimal::ONE) },
                parent_code: if i == 0 {
                    None
                } else {
                    Some(format!("E{:03}", 1))
                },
                acquisition_date: None,
                accounting_framework: None,
                industry: None,
                rows: None,
                hyperinflation_status:
                    datasynth_core::models::HyperinflationStatus::NotHyperinflationary,
                overrides: BTreeMap::new(),
            }
        })
        .collect();

    let entity_codes: Vec<String> = entities.iter().map(|e| e.code.clone()).collect();

    // Generate IC relationships as explicit (seller, buyer, type) triples
    // with no duplicates.  Bound iterations so a saturated triple-space
    // can't loop forever — for entity_count=2 there are only 2 ordered
    // pairs × 8 types = 16 distinct triples available.
    let mut relationships: Vec<IcRelationshipConfig> = Vec::with_capacity(ic_relationship_count);
    let mut seen_triples: std::collections::BTreeSet<(String, String, IcTransactionType)> =
        Default::default();
    let max_attempts = ic_relationship_count.saturating_mul(64).max(64);
    let mut attempts = 0usize;
    while relationships.len() < ic_relationship_count && attempts < max_attempts {
        attempts += 1;
        let seller_idx = rng.random_range(0..entity_codes.len());
        let buyer_idx = rng.random_range(0..entity_codes.len());
        if seller_idx == buyer_idx {
            continue;
        }
        let tx_type = TX_TYPES[rng.random_range(0..TX_TYPES.len())];
        let seller = entity_codes[seller_idx].clone();
        let buyer = entity_codes[buyer_idx].clone();
        let triple = (seller.clone(), buyer.clone(), tx_type);
        if !seen_triples.insert(triple) {
            continue;
        }
        let annual_volume_units: u64 = rng.random_range(100_000..=5_000_000);
        relationships.push(IcRelationshipConfig::Explicit(IcRelationshipExplicit {
            seller,
            buyer,
            types: vec![tx_type],
            annual_volume: Decimal::from(annual_volume_units),
            transfer_pricing: None,
            markup_percent: None,
        }));
    }
    // Sanity: the loop must have produced at least one relationship for
    // the property to be non-vacuous.  Tests with entity_count ≥ 3 and
    // ic_relationship_count ≥ 1 always satisfy this with the selected
    // seeds — assert defensively.
    assert!(
        !relationships.is_empty(),
        "synthetic config produced zero IC relationships (seed={seed}, entities={entity_count}, rels={ic_relationship_count})",
    );

    let mut scoping_profiles: BTreeMap<String, serde_yaml::Value> = BTreeMap::new();
    let mut profile_map = serde_yaml::Mapping::new();
    profile_map.insert(
        serde_yaml::Value::String("row_budget".to_string()),
        serde_yaml::Value::Number(serde_yaml::Number::from(1_000u64)),
    );
    scoping_profiles.insert(
        "significant".to_string(),
        serde_yaml::Value::Mapping(profile_map),
    );

    GroupConfig {
        id: format!("PROP_TEST_GROUP_{seed:04}"),
        name: Some(format!("Property test seed {seed}")),
        presentation_currency: currency.to_string(),
        period: PeriodConfig {
            start_date: NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date"),
            length: PeriodLength::Quarterly,
            fiscal_year_end: None,
        },
        seed,
        defaults: serde_yaml::Value::Null,
        scoping_profiles,
        ownership: OwnershipConfig {
            parent_entity_code: entity_codes[0].clone(),
            entities,
            generated: Vec::new(),
            entities_from: None,
        },
        intercompany: IntercompanyConfig {
            relationships,
            matching: IcMatchingConfig::default(),
        },
        fx: FxConfig {
            base_currency: currency.to_string(),
            rate_source: FxRateSource::Inline,
            rates: BTreeMap::new(),
            policy: FxPolicyConfig {
                balance_sheet: FxRateBasis::Closing,
                income_statement: FxRateBasis::Average,
                equity: FxRateBasis::Historical,
            },
        },
        audit: datasynth_group::AuditEngagementConfig {
            engagement_id: None,
            lead_auditor: None,
            framework: None,
            fsm_blueprint: None,
            group_materiality: Some(GroupMaterialityConfig {
                basis: MaterialityBasis::Revenue,
                percent: Decimal::new(1, 2), // 0.01
            }),
            component_scope_thresholds: None,
            generate_kams: false,
            generate_group_opinion: false,
        },
        tax: Default::default(),
        cgu: Default::default(),
        output: OutputLayoutConfig::default(),
        fleet: None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Loop 10 randomized configs and assert IC matching coverage is
/// **exactly** 1.0 (within `1e-9`) every time.
///
/// For each iteration:
/// 1. Build a synthetic [`GroupConfig`] from a `(seed, entity_count,
///    ic_relationship_count)` triple.
/// 2. [`build_manifest`] — deterministic, no orchestrator.
/// 3. For each entity: [`derive_ic_pair_plans`] →
///    [`inject_ic_journal_entries`] to produce the JE bag the matcher
///    expects.
/// 4. [`match_ic_pairs`] over the cross-entity JE bag.
/// 5. Assert `coverage == 1.0` and that every planned pair was
///    matched.
///
/// 100 % coverage by construction is the v5.0 contract — see the
/// module rustdoc on [`crate::aggregate::ic_matcher::match_ic_pairs`].
#[test]
fn coverage_is_one_across_random_configs() {
    // (seed, entity_count, ic_relationship_count)
    let cases: [(u64, usize, usize); 10] = [
        (0, 3, 1),
        (1, 5, 2),
        (2, 7, 3),
        (3, 9, 4),
        (4, 11, 5),
        (5, 13, 6),
        (6, 15, 7),
        (7, 5, 8),
        (8, 7, 4),
        (9, 9, 6),
    ];

    for (seed, entity_count, ic_count) in cases {
        let cfg = random_group_config(seed, entity_count, ic_count);
        let manifest = build_manifest(&cfg).unwrap_or_else(|e| {
            panic!("build_manifest failed for seed={seed}, entities={entity_count}, rels={ic_count}: {e}")
        });

        // Sanity: the manifest must have produced at least one resolved
        // IC relationship — synthesizing a config with rels >= 1 should
        // always satisfy this.
        assert!(
            !manifest.ic_relationships.is_empty(),
            "manifest had zero IC relationships for seed={seed} (cfg had {ic_count})",
        );

        // Build per-entity IC JEs.  We only run injection for entities
        // that appear as a participant in some IC relationship — others
        // produce empty plans and contribute nothing to the matcher.
        let entity_jes: Vec<(String, Vec<JournalEntry>)> = manifest
            .ownership_graph
            .entities
            .iter()
            .map(|e| {
                let plans = derive_ic_pair_plans(&manifest, &e.code);
                let jes = inject_ic_journal_entries(
                    &plans,
                    &InjectionCtx {
                        entity_code: e.code.clone(),
                    },
                );
                (e.code.clone(), jes)
            })
            .collect();

        let result = match_ic_pairs(&manifest, &entity_jes)
            .unwrap_or_else(|e| panic!("match_ic_pairs failed for seed={seed}: {e}"));

        // Every planned pair must be matched, no orphans.
        assert!(
            result.unmatched.is_empty(),
            "seed={seed}: {} unmatched sides — manifest-driven matching must produce zero orphans",
            result.unmatched.len(),
        );
        assert_eq!(
            result.matched.len(),
            result.total_planned,
            "seed={seed}: matched {} ≠ total_planned {}",
            result.matched.len(),
            result.total_planned,
        );
        // Coverage = 1.0 to the bit.
        assert!(
            (result.coverage - 1.0).abs() < 1e-9,
            "seed={seed}, entities={entity_count}, rels={ic_count}: \
             coverage {} != 1.0 (matched {}, planned {})",
            result.coverage,
            result.matched.len(),
            result.total_planned,
        );
    }
}

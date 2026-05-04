//! Spec §4.1 — ShardPlan resolution tests (Task 2.8).

use std::collections::BTreeMap;

use datasynth_group::{
    build_shard_plan, manifest::expansion::EntitySource, ConsolidationMethod, ExpandedEntity,
};
use rust_decimal_macros::dec;

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn make_entity(code: &str, profile: &str, rows: Option<u64>) -> ExpandedEntity {
    ExpandedEntity {
        code: code.to_string(),
        name: None,
        country: "US".to_string(),
        functional_currency: "USD".to_string(),
        scoping_profile: profile.to_string(),
        consolidation_method: ConsolidationMethod::Full,
        ownership_percent: Some(dec!(1.0)),
        parent_code: None,
        accounting_framework: None,
        industry: None,
        hyperinflation_status: datasynth_core::models::HyperinflationStatus::NotHyperinflationary,
        ownership_changes: Vec::new(),
        source: EntitySource::Explicit,
        generated_block_index: None,
        rows,
    }
}

fn no_profiles() -> BTreeMap<String, serde_yaml::Value> {
    BTreeMap::new()
}

fn profile_with_budget(budget: u64) -> serde_yaml::Value {
    let mut map = serde_yaml::Mapping::new();
    map.insert(
        serde_yaml::Value::String("row_budget".into()),
        serde_yaml::Value::Number(serde_yaml::Number::from(budget)),
    );
    serde_yaml::Value::Mapping(map)
}

// ── Test 1: single-profile grouping ──────────────────────────────────────────

#[test]
fn test_single_profile_single_shard() {
    let entities = vec![
        make_entity("A", "significant", None),
        make_entity("B", "significant", None),
        make_entity("C", "significant", None),
    ];

    let plan = build_shard_plan(&entities, &no_profiles()).unwrap();

    // All three in one shard (3 M rows << 10 B threshold).
    assert_eq!(plan.shards.len(), 1);
    let shard = &plan.shards[0];
    assert_eq!(shard.scoping_profile, "significant");
    assert_eq!(shard.shard_id, "S_SIG_0001");
    assert_eq!(shard.entity_codes.len(), 3);
}

// ── Test 2: multi-profile split ───────────────────────────────────────────────

#[test]
fn test_multi_profile_split() {
    let entities = vec![
        make_entity("SIG1", "significant", None),
        make_entity("MAT1", "material", None),
        make_entity("CON1", "consolidation_only", None),
    ];

    let plan = build_shard_plan(&entities, &no_profiles()).unwrap();

    // One shard per profile.
    assert_eq!(plan.shards.len(), 3, "three profiles → three shards");

    let profiles: Vec<&str> = plan
        .shards
        .iter()
        .map(|s| s.scoping_profile.as_str())
        .collect();
    assert!(profiles.contains(&"significant"));
    assert!(profiles.contains(&"material"));
    assert!(profiles.contains(&"consolidation_only"));
}

// ── Test 3: shard ID formatting S_SIG_0001 / S_MAT_0001 ─────────────────────

#[test]
fn test_shard_id_format() {
    let entities = vec![
        make_entity("S1", "significant", None),
        make_entity("M1", "material", None),
        make_entity("C1", "consolidation_only", None),
    ];

    let plan = build_shard_plan(&entities, &no_profiles()).unwrap();

    let sig_shard = plan
        .shards
        .iter()
        .find(|s| s.scoping_profile == "significant")
        .unwrap();
    assert_eq!(sig_shard.shard_id, "S_SIG_0001");

    let mat_shard = plan
        .shards
        .iter()
        .find(|s| s.scoping_profile == "material")
        .unwrap();
    assert_eq!(mat_shard.shard_id, "S_MAT_0001");

    let con_shard = plan
        .shards
        .iter()
        .find(|s| s.scoping_profile == "consolidation_only")
        .unwrap();
    assert_eq!(con_shard.shard_id, "S_CON_0001");
}

// ── Test 4: shard IDs stable under entity reordering ─────────────────────────

#[test]
fn test_shard_ids_stable_under_entity_reordering() {
    let entities_ab = vec![
        make_entity("B_ENT", "significant", None),
        make_entity("A_ENT", "significant", None),
    ];
    let entities_ba = vec![
        make_entity("A_ENT", "significant", None),
        make_entity("B_ENT", "significant", None),
    ];

    let plan_ab = build_shard_plan(&entities_ab, &no_profiles()).unwrap();
    let plan_ba = build_shard_plan(&entities_ba, &no_profiles()).unwrap();

    // Both should produce S_SIG_0001 with ["A_ENT", "B_ENT"] (sorted).
    assert_eq!(plan_ab.shards[0].shard_id, "S_SIG_0001");
    assert_eq!(plan_ba.shards[0].shard_id, "S_SIG_0001");
    assert_eq!(
        plan_ab.shards[0].entity_codes,
        plan_ba.shards[0].entity_codes
    );
    assert_eq!(
        plan_ab.shards[0].entity_codes,
        vec!["A_ENT".to_string(), "B_ENT".to_string()],
        "entity codes should be sorted ascending"
    );
}

// ── Test 5: row-budget batching at capacity boundary ──────────────────────────

#[test]
fn test_row_budget_batching_at_capacity() {
    // Each entity has 6 B rows.  Two entities would exceed 10 B → 2 shards.
    let entities = vec![
        make_entity("E1", "significant", Some(6_000_000_000)),
        make_entity("E2", "significant", Some(6_000_000_000)),
        make_entity("E3", "significant", Some(6_000_000_000)),
    ];

    let plan = build_shard_plan(&entities, &no_profiles()).unwrap();

    // E1 alone in shard 1, E2 alone in shard 2, E3 alone in shard 3.
    assert_eq!(
        plan.shards.len(),
        3,
        "each 6 B entity should be its own shard"
    );
    assert_eq!(plan.shards[0].shard_id, "S_SIG_0001");
    assert_eq!(plan.shards[1].shard_id, "S_SIG_0002");
    assert_eq!(plan.shards[2].shard_id, "S_SIG_0003");
    assert_eq!(plan.shards[0].entity_codes, vec!["E1".to_string()]);
    assert_eq!(plan.shards[1].entity_codes, vec!["E2".to_string()]);
    assert_eq!(plan.shards[2].entity_codes, vec!["E3".to_string()]);
}

// ── Test 6: empty entities list produces empty plan ───────────────────────────

#[test]
fn test_empty_entities_produces_empty_plan() {
    let entities: Vec<ExpandedEntity> = vec![];
    let plan = build_shard_plan(&entities, &no_profiles()).unwrap();
    assert!(plan.shards.is_empty(), "no entities → no shards");
}

// ── Test 7: profile row_budget read from scoping_profiles map ─────────────────

#[test]
fn test_row_budget_read_from_scoping_profiles() {
    let entities = vec![
        make_entity("E1", "significant", None), // rows = None → use profile budget
    ];

    let mut profiles: BTreeMap<String, serde_yaml::Value> = BTreeMap::new();
    profiles.insert("significant".to_string(), profile_with_budget(2_000_000));

    let plan = build_shard_plan(&entities, &profiles).unwrap();

    assert_eq!(plan.shards.len(), 1);
    // Row budget from profile: 2 M rows.
    assert_eq!(plan.shards[0].estimated_rows, 2_000_000);
    // Archive size: 2_000_000 × 800 / 1_000_000 = 1_600 MB.
    assert_eq!(plan.shards[0].estimated_archive_size_mb, 1_600);
}

// ── Test 8: unknown profile name falls back to first-3-uppercase ───────────────

#[test]
fn test_unknown_profile_name_initial() {
    let entities = vec![make_entity("E1", "analytical_only", None)];

    let plan = build_shard_plan(&entities, &no_profiles()).unwrap();

    // "analytical_only" → first 3 alpha chars = "ana" → "ANA"
    assert_eq!(plan.shards[0].shard_id, "S_ANA_0001");
}

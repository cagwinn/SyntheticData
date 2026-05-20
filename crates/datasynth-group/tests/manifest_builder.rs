//! Spec §4.1 — GroupManifest assembly tests (Task 2.9).

use chrono::NaiveDate;
use datasynth_group::{build_manifest, GroupConfig, MANIFEST_SCHEMA_VERSION};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn load_mini_acme() -> GroupConfig {
    let yaml = include_str!("fixtures/mini_acme.yaml");
    serde_yaml::from_str(yaml).expect("mini_acme.yaml must parse into GroupConfig")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Test 1: build_manifest succeeds for the mini_acme fixture and the top-level
/// fields match expectations from the YAML.
#[test]
fn test_build_manifest_for_mini_acme() {
    let cfg = load_mini_acme();
    let manifest = build_manifest(&cfg).expect("build_manifest must succeed for mini_acme");

    // Schema version
    assert_eq!(manifest.schema_version, MANIFEST_SCHEMA_VERSION);
    assert_eq!(manifest.schema_version, "1.0");

    // Group identity
    assert_eq!(manifest.group_id, "MINI_ACME_2024_Q1");
    assert_eq!(manifest.presentation_currency, "CHF");

    // 5 entities from mini_acme.yaml
    assert_eq!(
        manifest.ownership_graph.entities.len(),
        5,
        "expected exactly 5 entities; got {}",
        manifest.ownership_graph.entities.len()
    );

    // Parent entity code
    assert_eq!(manifest.ownership_graph.parent_entity_code, "ACME_SA");

    // Every entity has a non-empty entity_seed (64 hex chars = 32 bytes)
    for entity in &manifest.ownership_graph.entities {
        assert!(
            !entity.entity_seed.is_empty(),
            "entity {} has empty entity_seed",
            entity.code
        );
        assert_eq!(
            entity.entity_seed.len(),
            64,
            "entity {} entity_seed should be 64 hex chars (32 bytes)",
            entity.code
        );
    }

    // Every entity's shard_id maps to a shard in the shard_plan
    let all_shard_ids: std::collections::BTreeSet<String> = manifest
        .shard_plan
        .shards
        .iter()
        .map(|s| s.shard_id.clone())
        .collect();

    for entity in &manifest.ownership_graph.entities {
        assert!(
            all_shard_ids.contains(&entity.shard_id),
            "entity {} has shard_id '{}' not found in shard_plan",
            entity.code,
            entity.shard_id
        );
    }
}

/// Test 2: the manifest serializes to JSON without error and top-level fields
/// appear in the output.
#[test]
fn test_manifest_serializes_to_json() {
    let cfg = load_mini_acme();
    let manifest = build_manifest(&cfg).expect("build_manifest must succeed");

    let json = serde_json::to_string(&manifest).expect("manifest must serialize to JSON");

    // Must be a non-trivial JSON document
    assert!(
        json.len() > 100,
        "serialized manifest is suspiciously short"
    );

    // Top-level key presence checks
    assert!(
        json.contains("\"schema_version\""),
        "JSON must contain schema_version"
    );
    assert!(json.contains("\"group_id\""), "JSON must contain group_id");
    assert!(
        json.contains("\"MINI_ACME_2024_Q1\""),
        "JSON must contain the group id value"
    );
    assert!(
        json.contains("\"ownership_graph\""),
        "JSON must contain ownership_graph"
    );
    assert!(
        json.contains("\"shard_plan\""),
        "JSON must contain shard_plan"
    );
    assert!(
        json.contains("\"audit_engagement_plan\""),
        "JSON must contain audit_engagement_plan"
    );
    assert!(
        json.contains("\"fx_rate_master\""),
        "JSON must contain fx_rate_master"
    );
    assert!(
        json.contains("\"chart_of_accounts_master\""),
        "JSON must contain chart_of_accounts_master"
    );
}

/// Test 3: manifest round-trips through JSON.
///
/// Note: `GroupManifest` implements full Serialize + Deserialize.
/// `ChartOfAccountsMaster` wraps `ChartOfAccounts` from datasynth-core which
/// has both derives — so full round-trip is expected.
#[test]
fn test_manifest_round_trips_json() {
    let cfg = load_mini_acme();
    let manifest = build_manifest(&cfg).expect("build_manifest must succeed");

    let json = serde_json::to_string(&manifest).expect("must serialize");
    let recovered: datasynth_group::manifest::builder::GroupManifest =
        serde_json::from_str(&json).expect("must deserialize back from JSON");

    // Cross-check key identity fields that are always plain strings.
    assert_eq!(recovered.schema_version, manifest.schema_version);
    assert_eq!(recovered.group_id, manifest.group_id);
    assert_eq!(recovered.group_seed, manifest.group_seed);
    assert_eq!(
        recovered.presentation_currency,
        manifest.presentation_currency
    );
    assert_eq!(recovered.manifest_seed, manifest.manifest_seed);
    assert_eq!(recovered.aggregate_seed, manifest.aggregate_seed);
    assert_eq!(
        recovered.ownership_graph.parent_entity_code,
        manifest.ownership_graph.parent_entity_code
    );
    assert_eq!(
        recovered.ownership_graph.entities.len(),
        manifest.ownership_graph.entities.len()
    );
    // Shard plan shards count
    assert_eq!(
        recovered.shard_plan.shards.len(),
        manifest.shard_plan.shards.len()
    );
    // IC relationships count
    assert_eq!(
        recovered.ic_relationships.len(),
        manifest.ic_relationships.len()
    );
}

/// Test 4: building the manifest twice with identical config produces byte-identical
/// serialized JSON (determinism guarantee).
#[test]
fn test_manifest_is_deterministic() {
    let cfg = load_mini_acme();

    let m1 = build_manifest(&cfg).expect("first build must succeed");
    let m2 = build_manifest(&cfg).expect("second build must succeed");

    let j1 = serde_json::to_string(&m1).expect("must serialize first");
    let j2 = serde_json::to_string(&m2).expect("must serialize second");

    assert_eq!(
        j1, j2,
        "two builds of the same config must produce byte-identical JSON"
    );
}

/// Test 5: period end date is computed correctly for each length variant.
///
/// Quarterly period starting 2024-01-01 → ends 2024-03-31.
/// Annual period starting 2024-01-01 → ends 2024-12-31.
/// Monthly period starting 2024-02-01 → ends 2024-02-29 (2024 is a leap year).
#[test]
fn test_period_end_computed_correctly() {
    use datasynth_group::config::{PeriodConfig, PeriodLength};
    use datasynth_group::manifest::builder::compute_period_pub;

    // Quarterly: 2024-01-01 → 2024-03-31
    let quarterly = PeriodConfig {
        start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        length: PeriodLength::Quarterly,
        fiscal_year_end: None,
    };
    let period = compute_period_pub(&quarterly).expect("quarterly period must compute");
    assert_eq!(period.start, NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
    assert_eq!(period.end, NaiveDate::from_ymd_opt(2024, 3, 31).unwrap());

    // Annual: 2024-01-01 → 2024-12-31
    let annual = PeriodConfig {
        start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        length: PeriodLength::Annual,
        fiscal_year_end: None,
    };
    let period = compute_period_pub(&annual).expect("annual period must compute");
    assert_eq!(period.end, NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());

    // Monthly (leap year Feb): 2024-02-01 → 2024-02-29
    let monthly_feb = PeriodConfig {
        start_date: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        length: PeriodLength::Monthly,
        fiscal_year_end: None,
    };
    let period = compute_period_pub(&monthly_feb).expect("monthly Feb period must compute");
    assert_eq!(period.end, NaiveDate::from_ymd_opt(2024, 2, 29).unwrap());

    // SemiAnnual: 2024-01-01 → 2024-06-30
    let semi = PeriodConfig {
        start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        length: PeriodLength::SemiAnnual,
        fiscal_year_end: None,
    };
    let period = compute_period_pub(&semi).expect("semi-annual period must compute");
    assert_eq!(period.end, NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());
}

/// Test 6: every entity's shard_id is present in shard_plan.shards[*].entity_codes.
#[test]
fn test_shard_assignment_propagated_to_entities() {
    let cfg = load_mini_acme();
    let manifest = build_manifest(&cfg).expect("build_manifest must succeed");

    // Build a reverse lookup: entity_code → shard_id from the shard plan itself.
    let mut code_to_shard: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for shard in &manifest.shard_plan.shards {
        for code in &shard.entity_codes {
            code_to_shard.insert(code.clone(), shard.shard_id.clone());
        }
    }

    for entity in &manifest.ownership_graph.entities {
        let expected_shard = code_to_shard
            .get(&entity.code)
            .unwrap_or_else(|| panic!("entity {} is not in any shard's entity_codes", entity.code));
        assert_eq!(
            &entity.shard_id, expected_shard,
            "entity {} shard_id '{}' does not match shard_plan entry '{}'",
            entity.code, entity.shard_id, expected_shard
        );
    }
}

/// Test 7: manifest_seed and aggregate_seed are distinct non-empty hex strings.
#[test]
fn test_seeds_are_distinct_and_valid_hex() {
    let cfg = load_mini_acme();
    let manifest = build_manifest(&cfg).expect("build_manifest must succeed");

    // 32 bytes = 64 hex chars
    assert_eq!(
        manifest.manifest_seed.len(),
        64,
        "manifest_seed must be 64 hex chars"
    );
    assert_eq!(
        manifest.aggregate_seed.len(),
        64,
        "aggregate_seed must be 64 hex chars"
    );

    // They should be distinct (different domain-separation tags guarantee this)
    assert_ne!(
        manifest.manifest_seed, manifest.aggregate_seed,
        "manifest_seed and aggregate_seed must differ"
    );

    // Must be valid lowercase hex
    assert!(
        manifest
            .manifest_seed
            .chars()
            .all(|c| c.is_ascii_hexdigit()),
        "manifest_seed must be valid hex"
    );
    assert!(
        manifest
            .aggregate_seed
            .chars()
            .all(|c| c.is_ascii_hexdigit()),
        "aggregate_seed must be valid hex"
    );
}

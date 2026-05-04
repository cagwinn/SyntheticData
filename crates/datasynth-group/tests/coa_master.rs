//! Spec §4.1 — ChartOfAccountsMaster resolution tests.

use datasynth_group::{
    build_coa_master,
    manifest::expansion::{EntitySource, ExpandedEntity},
};
use rust_decimal_macros::dec;

// ── Test fixtures ─────────────────────────────────────────────────────────────

fn make_entity(code: &str, country: &str, framework: Option<&str>) -> ExpandedEntity {
    ExpandedEntity {
        code: code.to_string(),
        name: None,
        country: country.to_string(),
        functional_currency: "USD".to_string(),
        scoping_profile: "std".to_string(),
        consolidation_method: datasynth_group::ConsolidationMethod::Full,
        ownership_percent: Some(dec!(1.0)),
        parent_code: None,
        accounting_framework: framework.map(str::to_string),
        industry: None,
        source: EntitySource::Explicit,
        generated_block_index: None,
        rows: None,
        hyperinflation_status: datasynth_core::models::HyperinflationStatus::NotHyperinflationary,
        ownership_changes: Vec::new(),
    }
}

fn defaults_yaml(framework: Option<&str>, complexity: Option<&str>) -> serde_yaml::Value {
    let mut map = serde_yaml::Mapping::new();
    if let Some(f) = framework {
        map.insert(
            serde_yaml::Value::String("accounting_framework".into()),
            serde_yaml::Value::String(f.into()),
        );
    }
    if let Some(c) = complexity {
        map.insert(
            serde_yaml::Value::String("complexity".into()),
            serde_yaml::Value::String(c.into()),
        );
    }
    serde_yaml::Value::Mapping(map)
}

// ── Test 1: Single-framework group ───────────────────────────────────────────

#[test]
fn test_single_framework_group_ifrs() {
    let entities = vec![
        make_entity("E1", "GB", Some("ifrs")),
        make_entity("E2", "DE", Some("ifrs")),
        make_entity("E3", "FR", Some("ifrs")),
    ];
    let defaults = defaults_yaml(Some("ifrs"), None);
    let master = build_coa_master(&entities, &defaults, "ACME").unwrap();

    assert_eq!(master.frameworks.len(), 1, "only one framework expected");
    assert!(master.frameworks.contains_key("ifrs"));
    assert_eq!(master.primary_framework, "ifrs");
}

// ── Test 2: Multi-framework group ─────────────────────────────────────────────

#[test]
fn test_multi_framework_group() {
    let entities = vec![
        make_entity("E1", "US", Some("us_gaap")),
        make_entity("E2", "GB", Some("ifrs")),
        make_entity("E3", "DE", Some("hgb")),
    ];
    let defaults = defaults_yaml(Some("ifrs"), None);
    let master = build_coa_master(&entities, &defaults, "GLOBAL").unwrap();

    assert_eq!(master.frameworks.len(), 3);
    assert!(master.frameworks.contains_key("ifrs"), "missing ifrs");
    assert!(master.frameworks.contains_key("us_gaap"), "missing us_gaap");
    assert!(master.frameworks.contains_key("hgb"), "missing hgb");
}

// ── Test 3: Default fallback for entities without a framework ─────────────────

#[test]
fn test_none_framework_falls_back_to_defaults() {
    let entities = vec![
        make_entity("E1", "US", None), // no framework — should use defaults
        make_entity("E2", "FR", Some("pcg")),
    ];
    let defaults = defaults_yaml(Some("us_gaap"), None);
    let master = build_coa_master(&entities, &defaults, "TEST").unwrap();

    // E1 should have resolved to "us_gaap" from defaults.
    assert!(
        master.frameworks.contains_key("us_gaap"),
        "us_gaap should be present from fallback; keys: {:?}",
        master.frameworks.keys().collect::<Vec<_>>()
    );
    assert!(master.frameworks.contains_key("pcg"));
    assert_eq!(master.frameworks.len(), 2);
}

// ── Test 3b: If defaults.accounting_framework is also absent → "ifrs" ─────────

#[test]
fn test_none_framework_and_no_defaults_falls_back_to_ifrs() {
    let entities = vec![make_entity("E1", "US", None)];
    let defaults = defaults_yaml(None, None); // no framework key at all
    let master = build_coa_master(&entities, &defaults, "TEST2").unwrap();

    assert_eq!(
        master.frameworks.len(),
        1,
        "should have exactly one framework"
    );
    assert!(
        master.frameworks.contains_key("ifrs"),
        "fallback should be ifrs; keys: {:?}",
        master.frameworks.keys().collect::<Vec<_>>()
    );
}

// ── Test 4: Primary framework derivation ──────────────────────────────────────

#[test]
fn test_primary_framework_from_defaults_when_set() {
    let entities = vec![
        make_entity("E1", "US", Some("us_gaap")),
        make_entity("E2", "GB", Some("ifrs")),
    ];
    // defaults explicitly specifies ifrs as primary
    let defaults = defaults_yaml(Some("ifrs"), None);
    let master = build_coa_master(&entities, &defaults, "GRP").unwrap();

    assert_eq!(
        master.primary_framework, "ifrs",
        "primary_framework should come from defaults"
    );
}

#[test]
fn test_primary_framework_is_first_sorted_when_defaults_absent() {
    let entities = vec![
        make_entity("E1", "US", Some("us_gaap")),
        make_entity("E2", "GB", Some("ifrs")),
        make_entity("E3", "DE", Some("hgb")),
    ];
    // No defaults.accounting_framework
    let defaults = defaults_yaml(None, None);
    let master = build_coa_master(&entities, &defaults, "GRP").unwrap();

    // BTreeMap sorts lexicographically: "hgb" < "ifrs" < "us_gaap"
    assert_eq!(
        master.primary_framework, "hgb",
        "without defaults, first-sorted key wins; got: {}",
        master.primary_framework
    );
}

// ── Test 5: coa_id determinism + stability ────────────────────────────────────

#[test]
fn test_coa_id_is_deterministic_same_input() {
    let entities = vec![
        make_entity("E1", "US", Some("us_gaap")),
        make_entity("E2", "GB", Some("ifrs")),
    ];
    let defaults = defaults_yaml(Some("ifrs"), Some("medium"));
    let a = build_coa_master(&entities, &defaults, "GRP").unwrap();
    let b = build_coa_master(&entities, &defaults, "GRP").unwrap();
    assert_eq!(a.coa_id, b.coa_id, "same input → same coa_id");
}

#[test]
fn test_coa_id_changes_when_framework_added() {
    let entities_two = vec![
        make_entity("E1", "US", Some("us_gaap")),
        make_entity("E2", "GB", Some("ifrs")),
    ];
    let entities_three = vec![
        make_entity("E1", "US", Some("us_gaap")),
        make_entity("E2", "GB", Some("ifrs")),
        make_entity("E3", "DE", Some("hgb")),
    ];
    let defaults = defaults_yaml(Some("ifrs"), Some("medium"));
    let two = build_coa_master(&entities_two, &defaults, "GRP").unwrap();
    let three = build_coa_master(&entities_three, &defaults, "GRP").unwrap();
    assert_ne!(
        two.coa_id, three.coa_id,
        "adding a framework must change coa_id"
    );
}

#[test]
fn test_coa_id_stable_under_entity_reordering() {
    let entities_ab = vec![
        make_entity("E1", "US", Some("us_gaap")),
        make_entity("E2", "GB", Some("ifrs")),
    ];
    // Same two frameworks, entities in different order.
    let entities_ba = vec![
        make_entity("E2", "GB", Some("ifrs")),
        make_entity("E1", "US", Some("us_gaap")),
    ];
    let defaults = defaults_yaml(Some("ifrs"), Some("medium"));
    let ab = build_coa_master(&entities_ab, &defaults, "GRP").unwrap();
    let ba = build_coa_master(&entities_ba, &defaults, "GRP").unwrap();
    assert_eq!(ab.coa_id, ba.coa_id, "entity order must NOT affect coa_id");
}

// ── Test 6: Each resolved framework CoA is non-empty ─────────────────────────

#[test]
fn test_all_framework_coas_are_non_empty() {
    let entities = vec![
        make_entity("E1", "US", Some("us_gaap")),
        make_entity("E2", "GB", Some("ifrs")),
        make_entity("E3", "DE", Some("hgb")),
        make_entity("E4", "FR", Some("pcg")),
        make_entity("E5", "CH", Some("swiss_or")),
    ];
    let defaults = defaults_yaml(Some("ifrs"), Some("small"));
    let master = build_coa_master(&entities, &defaults, "MULTI").unwrap();

    assert_eq!(master.frameworks.len(), 5, "five distinct frameworks");

    for (fw, coa) in &master.frameworks {
        assert!(
            coa.account_count() > 0,
            "CoA for framework '{fw}' should have >0 accounts, got 0"
        );
        // Spot-check: the coa_id and framework label are group-scoped.
        assert!(
            coa.coa_id.starts_with("MULTI_"),
            "coa.coa_id for '{fw}' should start with 'MULTI_', got: {}",
            coa.coa_id
        );
        assert_eq!(
            coa.accounting_framework.as_deref(),
            Some(fw.as_str()),
            "accounting_framework field should match key"
        );
    }
}

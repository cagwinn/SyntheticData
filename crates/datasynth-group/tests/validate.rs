//! Spec §3.x — structural validation of GroupConfig.
//! Tests are ordered to match the check numbers in the implementation plan.

use datasynth_group::{validate::validate, GroupConfig};

fn parse(yaml: &str) -> GroupConfig {
    serde_yaml::from_str(yaml).expect("YAML must parse")
}

/// Minimal valid base — parent P with scoping_profile std, no IC relationships.
fn base_yaml(extra_entities: &str, extra_sections: &str) -> String {
    format!(
        r#"
id: T
presentation_currency: USD
period: {{ start_date: "2024-01-01", length: quarterly }}
seed: 1
scoping_profiles:
  std: {{}}
ownership:
  parent_entity_code: P
  entities:
    - {{ code: P, country: US, functional_currency: USD, scoping_profile: std, consolidation_method: parent }}
{extra_entities}
fx:
  base_currency: USD
  rate_source: inline
  rates: {{}}
  policy: {{ balance_sheet: closing, income_statement: average, equity: historical }}
{extra_sections}
"#
    )
}

// ---------------------------------------------------------------------------
// Check 1 — parent_entity_code must appear in ownership.entities
// ---------------------------------------------------------------------------

#[test]
fn test_unknown_parent_code_fails() {
    let yaml = r#"
id: T
presentation_currency: USD
period: { start_date: "2024-01-01", length: quarterly }
seed: 1
scoping_profiles: { std: {} }
ownership:
  parent_entity_code: UNKNOWN
  entities:
    - { code: P, country: US, functional_currency: USD, scoping_profile: std, consolidation_method: parent }
fx:
  base_currency: USD
  rate_source: inline
  rates: {}
  policy: { balance_sheet: closing, income_statement: average, equity: historical }
"#;
    let cfg = parse(yaml);
    let err = validate(&cfg).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("parent_entity_code"),
        "error should mention parent_entity_code, got: {msg}"
    );
    assert!(
        msg.contains("UNKNOWN"),
        "error should mention the bad code, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Check 2 — entity.scoping_profile must exist in scoping_profiles
// ---------------------------------------------------------------------------

#[test]
fn test_unknown_scoping_profile_fails() {
    let yaml = r#"
id: T
presentation_currency: USD
period: { start_date: "2024-01-01", length: quarterly }
seed: 1
scoping_profiles: { std: {} }
ownership:
  parent_entity_code: P
  entities:
    - { code: P, country: US, functional_currency: USD, scoping_profile: std, consolidation_method: parent }
    - { code: Q, country: US, functional_currency: USD, scoping_profile: nonexistent, consolidation_method: full, ownership_percent: 1.0, parent_code: P }
fx:
  base_currency: USD
  rate_source: inline
  rates: {}
  policy: { balance_sheet: closing, income_statement: average, equity: historical }
"#;
    let cfg = parse(yaml);
    let err = validate(&cfg).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("scoping_profile"),
        "error should mention scoping_profile, got: {msg}"
    );
    assert!(
        msg.contains("nonexistent"),
        "error should mention the bad profile name, got: {msg}"
    );
    assert!(
        msg.contains('Q'),
        "error should mention the entity code, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Check 3 — entity.parent_code (if Some) must exist in ownership.entities
// ---------------------------------------------------------------------------

#[test]
fn test_unknown_parent_code_on_entity_fails() {
    let yaml = r#"
id: T
presentation_currency: USD
period: { start_date: "2024-01-01", length: quarterly }
seed: 1
scoping_profiles: { std: {} }
ownership:
  parent_entity_code: P
  entities:
    - { code: P, country: US, functional_currency: USD, scoping_profile: std, consolidation_method: parent }
    - { code: Q, country: US, functional_currency: USD, scoping_profile: std, consolidation_method: full, ownership_percent: 1.0, parent_code: GHOST }
fx:
  base_currency: USD
  rate_source: inline
  rates: {}
  policy: { balance_sheet: closing, income_statement: average, equity: historical }
"#;
    let cfg = parse(yaml);
    let err = validate(&cfg).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("GHOST"),
        "error should mention the missing parent_code, got: {msg}"
    );
    assert!(
        msg.contains('Q'),
        "error should mention the entity with bad parent_code, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Check 4 — ownership_percent must be in [0.0, 1.0]
// ---------------------------------------------------------------------------

#[test]
fn test_ownership_percent_out_of_range_fails() {
    // > 1.0
    let yaml_above = r#"
id: T
presentation_currency: USD
period: { start_date: "2024-01-01", length: quarterly }
seed: 1
scoping_profiles: { std: {} }
ownership:
  parent_entity_code: P
  entities:
    - { code: P, country: US, functional_currency: USD, scoping_profile: std, consolidation_method: parent }
    - { code: Q, country: US, functional_currency: USD, scoping_profile: std, consolidation_method: full, ownership_percent: 1.5, parent_code: P }
fx:
  base_currency: USD
  rate_source: inline
  rates: {}
  policy: { balance_sheet: closing, income_statement: average, equity: historical }
"#;
    let cfg = parse(yaml_above);
    let err = validate(&cfg).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("ownership_percent"),
        "error should mention ownership_percent, got: {msg}"
    );
    assert!(
        msg.contains('Q'),
        "error should mention the entity, got: {msg}"
    );

    // Negative value
    let yaml_below = r#"
id: T
presentation_currency: USD
period: { start_date: "2024-01-01", length: quarterly }
seed: 1
scoping_profiles: { std: {} }
ownership:
  parent_entity_code: P
  entities:
    - { code: P, country: US, functional_currency: USD, scoping_profile: std, consolidation_method: parent }
    - { code: R, country: US, functional_currency: USD, scoping_profile: std, consolidation_method: full, ownership_percent: -0.1, parent_code: P }
fx:
  base_currency: USD
  rate_source: inline
  rates: {}
  policy: { balance_sheet: closing, income_statement: average, equity: historical }
"#;
    let cfg2 = parse(yaml_below);
    let err2 = validate(&cfg2).unwrap_err();
    let msg2 = err2.to_string();
    assert!(
        msg2.contains("ownership_percent"),
        "error should mention ownership_percent, got: {msg2}"
    );
}

// ---------------------------------------------------------------------------
// Check 5 — IC Explicit: seller and buyer must be in entities; no self-pairs
// ---------------------------------------------------------------------------

#[test]
fn test_ic_relationship_unknown_entity_fails() {
    let extra_sections = r#"
intercompany:
  relationships:
    - { seller: P, buyer: GHOST_ENTITY, types: [goods_sale], annual_volume: 1000000 }
"#;
    let yaml = base_yaml("", extra_sections);
    let cfg = parse(&yaml);
    let err = validate(&cfg).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("GHOST_ENTITY"),
        "error should mention the unknown entity, got: {msg}"
    );
}

#[test]
fn test_ic_relationship_self_pair_fails() {
    let extra_sections = r#"
intercompany:
  relationships:
    - { seller: P, buyer: P, types: [management_fee], annual_volume: 500000 }
"#;
    let yaml = base_yaml("", extra_sections);
    let cfg = parse(&yaml);
    let err = validate(&cfg).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("seller == buyer") || msg.contains("self"),
        "error should flag self-pair, got: {msg}"
    );
    assert!(
        msg.contains('P'),
        "error should mention the entity code, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Check 6 — IC Pattern: seller/buyer and scoping_profile references resolve
// ---------------------------------------------------------------------------

#[test]
fn test_ic_pattern_unknown_entity_fails() {
    // Pattern with a named seller that doesn't exist
    let extra_sections = r#"
intercompany:
  relationships:
    - pattern: { seller: NONEXISTENT, buyer_scoping_profile: std }
      types: [management_fee]
      per_pair_volume: 100000
"#;
    let yaml = base_yaml("", extra_sections);
    let cfg = parse(&yaml);
    let err = validate(&cfg).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("NONEXISTENT"),
        "error should mention the unknown entity in pattern, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Check 8 (I2) — EntityConfig.overrides: typo detection via Levenshtein ≤ 2
// ---------------------------------------------------------------------------

#[test]
fn test_entity_overrides_typo_detection() {
    // "acconting_framework" is distance 2 from "accounting_framework"
    // (one 'c' removed from 'accounting' — 'accounting' vs 'acconting')
    // Since EntityConfig doesn't have 'acconting_framework' as a named field,
    // serde absorbs it into the overrides map.
    let yaml = r#"
id: T
presentation_currency: USD
period: { start_date: "2024-01-01", length: quarterly }
seed: 1
scoping_profiles: { std: {} }
ownership:
  parent_entity_code: P
  entities:
    - code: P
      country: US
      functional_currency: USD
      scoping_profile: std
      consolidation_method: parent
      acconting_framework: us_gaap
fx:
  base_currency: USD
  rate_source: inline
  rates: {}
  policy: { balance_sheet: closing, income_statement: average, equity: historical }
"#;
    let cfg = parse(yaml);
    let err = validate(&cfg).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("acconting_framework"),
        "error should mention the typo key, got: {msg}"
    );
    assert!(
        msg.contains("accounting_framework"),
        "error should suggest the correct field, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Positive test — full mini_nestle.yaml must pass validation
// ---------------------------------------------------------------------------

#[test]
fn test_valid_config_passes() {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let cfg: GroupConfig = serde_yaml::from_str(yaml).expect("fixture must parse");
    validate(&cfg).expect("mini_nestle.yaml must pass validation");
}

//! Spec §3.2 — three-level inheritance: defaults → scoping_profile → per-entity override.

use datasynth_group::{resolve::resolve_entity, GroupConfig};

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
    assert_eq!(
        p.process_models,
        vec!["o2c", "p2p", "h2r", "audit"],
        "profile overrides defaults"
    );

    // S1: significant profile + per-entity accounting_framework override.
    let s1 = resolve_entity(&cfg, "S1").unwrap();
    assert_eq!(s1.accounting_framework, "us_gaap", "per-entity override wins");
    assert_eq!(
        s1.process_models,
        vec!["o2c", "p2p", "h2r", "audit"],
        "profile process_models"
    );

    // S2: material profile (different process_models).
    let s2 = resolve_entity(&cfg, "S2").unwrap();
    assert_eq!(s2.process_models, vec!["o2c", "p2p", "audit"]);
}

#[test]
fn test_resolve_unknown_entity_is_error() {
    let cfg_yaml = r#"
id: T
presentation_currency: USD
period: { start_date: "2024-01-01", length: quarterly }
seed: 1
scoping_profiles: { std: {} }
ownership:
  parent_entity_code: P
  entities:
    - { code: P, country: US, functional_currency: USD, scoping_profile: std, consolidation_method: parent }
fx:
  base_currency: USD
  rate_source: inline
  rates: {}
  policy: { balance_sheet: closing, income_statement: average, equity: historical }
"#;
    let cfg: GroupConfig = serde_yaml::from_str(cfg_yaml).unwrap();
    let err = resolve_entity(&cfg, "NONEXISTENT").unwrap_err();
    assert!(err.to_string().contains("NONEXISTENT"));
}

#[test]
fn test_resolve_unknown_scoping_profile_is_error() {
    let cfg_yaml = r#"
id: T
presentation_currency: USD
period: { start_date: "2024-01-01", length: quarterly }
seed: 1
scoping_profiles: { std: {} }
ownership:
  parent_entity_code: P
  entities:
    - { code: P, country: US, functional_currency: USD, scoping_profile: BOGUS, consolidation_method: parent }
fx:
  base_currency: USD
  rate_source: inline
  rates: {}
  policy: { balance_sheet: closing, income_statement: average, equity: historical }
"#;
    let cfg: GroupConfig = serde_yaml::from_str(cfg_yaml).unwrap();
    let err = resolve_entity(&cfg, "P").unwrap_err();
    assert!(err.to_string().contains("BOGUS"));
}

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

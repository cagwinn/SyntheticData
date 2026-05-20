//! Spec §3.1 — `group:` config YAML parses into GroupConfig.

use datasynth_group::{validate::validate, GroupConfig};

#[test]
fn test_mini_acme_parses() {
    let yaml = include_str!("fixtures/mini_acme_minimal.yaml");
    let cfg: GroupConfig = serde_yaml::from_str(yaml).expect("mini_acme fixture must parse");

    assert_eq!(cfg.id, "MINI_ACME_2024_Q1");
    assert_eq!(cfg.presentation_currency, "CHF");
    assert_eq!(cfg.ownership.parent_entity_code, "ACME_SA");
    assert!(!cfg.ownership.entities.is_empty());
}

#[test]
fn test_full_mini_acme_parses() {
    use rust_decimal_macros::dec;

    let yaml = include_str!("fixtures/mini_acme.yaml");
    let cfg: datasynth_group::GroupConfig =
        serde_yaml::from_str(yaml).expect("full mini_acme must parse");

    // Spot-check all sections are present.
    assert_eq!(cfg.id, "MINI_ACME_2024_Q1");
    assert_eq!(cfg.scoping_profiles.len(), 2, "significant + material");
    assert_eq!(
        cfg.ownership.entities.len(),
        5,
        "parent + 4 subsidiaries/JV"
    );

    // Verify consolidation methods exercised.
    use datasynth_group::ConsolidationMethod::*;
    let methods: std::collections::BTreeSet<_> = cfg
        .ownership
        .entities
        .iter()
        .map(|e| e.consolidation_method)
        .collect();
    assert!(methods.contains(&Parent), "parent entity present");
    assert!(methods.contains(&Full), "fully-consolidated entity present");
    assert!(methods.contains(&EquityMethod), "equity-method JV present");

    // Multi-GAAP entity present.
    assert!(
        cfg.ownership
            .entities
            .iter()
            .any(|e| e.accounting_framework.as_deref() == Some("us_gaap")),
        "at least one US GAAP entity"
    );

    // NCI-bearing entity (80% owned).
    assert!(
        cfg.ownership
            .entities
            .iter()
            .any(|e| e.ownership_percent == Some(dec!(0.80))),
        "at least one 80%-owned entity (NCI)"
    );

    // IC relationships: explicit + pattern
    assert!(
        cfg.intercompany.relationships.len() >= 3,
        "at least two explicit + one pattern"
    );
    assert!(matches!(
        cfg.intercompany.matching.strategy,
        datasynth_group::IcMatchingStrategy::ManifestDriven
    ));

    // FX rates for 3 pairs (CHF/USD, CHF/EUR, CHF/BRL)
    assert_eq!(cfg.fx.rates.len(), 3, "three currency pairs");

    // Audit + tax both configured
    assert_eq!(
        cfg.audit.engagement_id.as_deref(),
        Some("EY_MINI_ACME_2024_Q1")
    );
    assert!(
        cfg.tax
            .pillar_two
            .as_ref()
            .map(|p| p.enabled)
            .unwrap_or(false),
        "pillar_two enabled"
    );

    // Structural validation must also pass on this fixture.
    validate(&cfg).expect("mini_acme.yaml must pass structural validation");
}

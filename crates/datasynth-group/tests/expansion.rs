//! Spec §3.1 — `generated:` blocks expand to per-entity records deterministically.

use datasynth_group::{
    config::{ConsolidationMethod, GeneratedEntityBlock, OwnershipConfig},
    manifest::expansion::{expand_ownership, EntitySource},
};

fn base_cfg_with_generated(generated: Vec<GeneratedEntityBlock>) -> OwnershipConfig {
    OwnershipConfig {
        parent_entity_code: "P".to_string(),
        entities: vec![datasynth_group::config::EntityConfig {
            code: "P".to_string(),
            name: None,
            country: "CH".to_string(),
            functional_currency: "CHF".to_string(),
            scoping_profile: "std".to_string(),
            consolidation_method: ConsolidationMethod::Parent,
            ownership_percent: None,
            parent_code: None,
            acquisition_date: None,
            accounting_framework: None,
            industry: None,
            rows: None,
            overrides: Default::default(),
        }],
        generated,
        entities_from: None,
    }
}

fn block(count: u32, prefix: &str, countries: &[&str]) -> GeneratedEntityBlock {
    GeneratedEntityBlock {
        count,
        code_prefix: prefix.to_string(),
        country: countries.iter().map(|s| s.to_string()).collect(),
        functional_currency: None,
        scoping_profile: "std".to_string(),
        consolidation_method: ConsolidationMethod::Full,
        ownership_percent_range: None,
        parent_code: Some("P".to_string()),
        accounting_framework: None,
        industry: None,
    }
}

fn d(y: i32, m: u32, d: u32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

#[test]
fn test_expands_explicit_first_then_generated() {
    let ownership = base_cfg_with_generated(vec![block(3, "GEN_", &["DE"])]);
    let out = expand_ownership(&ownership, 42, d(2024, 1, 1)).unwrap();
    assert_eq!(out.len(), 4, "1 explicit + 3 generated");
    assert_eq!(out[0].code, "P");
    assert_eq!(out[0].source, EntitySource::Explicit);
    for i in 1..=3 {
        assert_eq!(out[i].source, EntitySource::Generated);
        assert_eq!(out[i].generated_block_index, Some(0));
    }
}

#[test]
fn test_generated_codes_are_zero_padded() {
    let ownership = base_cfg_with_generated(vec![block(3, "NESTLE_EU_", &["DE"])]);
    let out = expand_ownership(&ownership, 42, d(2024, 1, 1)).unwrap();
    assert_eq!(out[1].code, "NESTLE_EU_0000001");
    assert_eq!(out[2].code, "NESTLE_EU_0000002");
    assert_eq!(out[3].code, "NESTLE_EU_0000003");
}

#[test]
fn test_generated_is_deterministic() {
    let ownership = base_cfg_with_generated(vec![block(20, "G_", &["DE", "FR", "IT"])]);
    let a = expand_ownership(&ownership, 42, d(2024, 1, 1)).unwrap();
    let b = expand_ownership(&ownership, 42, d(2024, 1, 1)).unwrap();
    assert_eq!(a, b, "same seed → same expansion");

    let c = expand_ownership(&ownership, 43, d(2024, 1, 1)).unwrap();
    assert_ne!(a, c, "different seed → different country distribution");
}

#[test]
fn test_country_sampling_uses_block_list_only() {
    let countries = ["DE", "FR", "IT", "ES", "PL"];
    let ownership = base_cfg_with_generated(vec![block(50, "G_", &countries)]);
    let out = expand_ownership(&ownership, 42, d(2024, 1, 1)).unwrap();
    for e in out.iter().filter(|e| e.source == EntitySource::Generated) {
        assert!(
            countries.contains(&e.country.as_str()),
            "generated entity country {} not in block list",
            e.country
        );
    }
}

#[test]
fn test_currency_defaults_from_country_mapping() {
    let ownership = base_cfg_with_generated(vec![block(10, "G_", &["DE"])]);
    let out = expand_ownership(&ownership, 42, d(2024, 1, 1)).unwrap();
    for e in out.iter().filter(|e| e.source == EntitySource::Generated) {
        assert_eq!(e.functional_currency, "EUR", "DE → EUR default");
    }
}

#[test]
fn test_explicit_currency_override_wins() {
    let mut b = block(3, "G_", &["DE"]);
    b.functional_currency = Some("USD".to_string());
    let ownership = base_cfg_with_generated(vec![b]);
    let out = expand_ownership(&ownership, 42, d(2024, 1, 1)).unwrap();
    for e in out.iter().filter(|e| e.source == EntitySource::Generated) {
        assert_eq!(
            e.functional_currency, "USD",
            "explicit override beats mapping"
        );
    }
}

#[test]
fn test_ownership_percent_samples_within_range() {
    use rust_decimal_macros::dec;
    let mut b = block(100, "G_", &["DE"]);
    b.ownership_percent_range = Some([dec!(0.85), dec!(1.00)]);
    let ownership = base_cfg_with_generated(vec![b]);
    let out = expand_ownership(&ownership, 42, d(2024, 1, 1)).unwrap();
    for e in out.iter().filter(|e| e.source == EntitySource::Generated) {
        let pct = e.ownership_percent.expect("should be Some in range");
        assert!(
            pct >= dec!(0.85) && pct <= dec!(1.00),
            "ownership_percent {pct} outside [0.85, 1.00]",
        );
    }
}

#[test]
fn test_collision_between_explicit_and_generated_fails() {
    // Explicit entity "GEN_0000001" collides with block prefix GEN_.
    let mut ownership = base_cfg_with_generated(vec![block(3, "GEN_", &["DE"])]);
    ownership
        .entities
        .push(datasynth_group::config::EntityConfig {
            code: "GEN_0000001".to_string(),
            name: None,
            country: "US".to_string(),
            functional_currency: "USD".to_string(),
            scoping_profile: "std".to_string(),
            consolidation_method: ConsolidationMethod::Full,
            ownership_percent: None,
            parent_code: Some("P".to_string()),
            acquisition_date: None,
            accounting_framework: None,
            industry: None,
            rows: None,
            overrides: Default::default(),
        });
    let err = expand_ownership(&ownership, 42, d(2024, 1, 1)).unwrap_err();
    assert!(err.to_string().contains("GEN_0000001"));
}

#[test]
fn test_explicit_duplicates_fail() {
    let mut ownership = base_cfg_with_generated(vec![]);
    // Push another entity with the same code as P.
    ownership
        .entities
        .push(datasynth_group::config::EntityConfig {
            code: "P".to_string(),
            name: None,
            country: "US".to_string(),
            functional_currency: "USD".to_string(),
            scoping_profile: "std".to_string(),
            consolidation_method: ConsolidationMethod::Full,
            ownership_percent: None,
            parent_code: None,
            acquisition_date: None,
            accounting_framework: None,
            industry: None,
            rows: None,
            overrides: Default::default(),
        });
    let err = expand_ownership(&ownership, 42, d(2024, 1, 1)).unwrap_err();
    assert!(err.to_string().contains('P'));
}

#[test]
fn test_empty_country_list_fails() {
    let b = block(3, "G_", &[]);
    let ownership = base_cfg_with_generated(vec![b]);
    let err = expand_ownership(&ownership, 42, d(2024, 1, 1)).unwrap_err();
    assert!(err.to_string().contains("country"));
}

#[test]
fn test_block_order_independence_of_other_blocks() {
    // Swapping the order of two generated blocks changes their block_idx,
    // which is intended — block_idx IS part of the seed. Not an
    // order-independence guarantee. But within a block, the generated
    // codes/countries/currencies are order-determinate.
    let ownership1 =
        base_cfg_with_generated(vec![block(5, "A_", &["DE"]), block(5, "B_", &["FR"])]);
    let ownership2 =
        base_cfg_with_generated(vec![block(5, "A_", &["DE"]), block(5, "B_", &["FR"])]);
    let a = expand_ownership(&ownership1, 42, d(2024, 1, 1)).unwrap();
    let b = expand_ownership(&ownership2, 42, d(2024, 1, 1)).unwrap();
    assert_eq!(a, b);
}

//! Spec §3.1 + §5.1 — IC pattern expansion with stable IDs.

use datasynth_group::{
    config::{
        ConsolidationMethod, IcMatchingConfig, IcPattern, IcRelationshipConfig,
        IcRelationshipExplicit, IcRelationshipPattern, IcTransactionType, IntercompanyConfig,
        TransferPricingMethod,
    },
    manifest::expansion::{EntitySource, ExpandedEntity},
    manifest::ic_expansion::{expand_ic_relationships, IcSource},
};
use rust_decimal_macros::dec;

fn ent(code: &str, profile: &str, cm: ConsolidationMethod) -> ExpandedEntity {
    ExpandedEntity {
        code: code.into(),
        name: None,
        country: "XX".into(),
        functional_currency: "USD".into(),
        scoping_profile: profile.into(),
        consolidation_method: cm,
        ownership_percent: None,
        parent_code: None,
        accounting_framework: None,
        industry: None,
        source: EntitySource::Explicit,
        generated_block_index: None,
        rows: None,
    }
}

#[test]
fn test_explicit_pair_is_resolved_with_stable_id() {
    let ic = IntercompanyConfig {
        relationships: vec![IcRelationshipConfig::Explicit(IcRelationshipExplicit {
            seller: "A".into(),
            buyer: "B".into(),
            types: vec![IcTransactionType::GoodsSale, IcTransactionType::Royalty],
            annual_volume: dec!(100_000),
            transfer_pricing: Some(TransferPricingMethod::CostPlus),
            markup_percent: Some(dec!(0.05)),
        })],
        matching: IcMatchingConfig::default(),
    };
    let entities = vec![
        ent("A", "sig", ConsolidationMethod::Parent),
        ent("B", "sig", ConsolidationMethod::Full),
    ];
    let out = expand_ic_relationships(&ic, &entities, 42).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].seller, "A");
    assert_eq!(out[0].buyer, "B");
    assert_eq!(
        out[0].types,
        vec![IcTransactionType::GoodsSale, IcTransactionType::Royalty]
    );
    assert_eq!(out[0].annual_volume, dec!(100_000));
    assert_eq!(out[0].source, IcSource::Explicit);
    // Stable id: repeating the derivation yields the same hash.
    let again = expand_ic_relationships(&ic, &entities, 42).unwrap();
    assert_eq!(out[0].id, again[0].id);
    assert!(out[0].id.starts_with("ICR_"));
}

#[test]
fn test_pattern_expands_to_all_pairs() {
    let ic = IntercompanyConfig {
        relationships: vec![IcRelationshipConfig::Pattern(IcRelationshipPattern {
            pattern: IcPattern {
                seller_scoping_profile: Some("sig".into()),
                buyer_scoping_profile: Some("any".into()),
                seller: None,
                buyer: None,
            },
            types: vec![IcTransactionType::ManagementFee],
            per_pair_volume: dec!(50_000),
            transfer_pricing: Some(TransferPricingMethod::CostPlus),
        })],
        matching: IcMatchingConfig::default(),
    };
    let entities = vec![
        ent("A", "sig", ConsolidationMethod::Parent),
        ent("B", "sig", ConsolidationMethod::Full),
        ent("C", "material", ConsolidationMethod::Full),
    ];
    let out = expand_ic_relationships(&ic, &entities, 42).unwrap();
    // sig sellers (A, B) × any buyers (A, B, C) = 6 candidate pairs, minus 2 self-pairs = 4.
    assert_eq!(out.len(), 4);
    for rel in &out {
        assert_eq!(rel.source, IcSource::Pattern);
        assert!(rel.pattern_index.is_some());
        assert_ne!(rel.seller, rel.buyer);
    }
}

#[test]
fn test_pattern_skips_fair_value_buyers() {
    let ic = IntercompanyConfig {
        relationships: vec![IcRelationshipConfig::Pattern(IcRelationshipPattern {
            pattern: IcPattern {
                seller_scoping_profile: Some("sig".into()),
                buyer_scoping_profile: Some("any".into()),
                seller: None,
                buyer: None,
            },
            types: vec![IcTransactionType::ManagementFee],
            per_pair_volume: dec!(50_000),
            transfer_pricing: None,
        })],
        matching: IcMatchingConfig::default(),
    };
    let entities = vec![
        ent("A", "sig", ConsolidationMethod::Parent),
        ent("FV", "material", ConsolidationMethod::FairValue),
    ];
    let out = expand_ic_relationships(&ic, &entities, 42).unwrap();
    // A → FV skipped; only zero pairs remain (A → A is self, skipped; FV can be seller but
    // has profile "material", not "sig", so it's not a seller either).
    assert_eq!(out.len(), 0);
}

#[test]
fn test_explicit_wins_over_pattern_on_same_triple() {
    // Pattern would produce (A, B, GoodsSale); explicit also names (A, B, GoodsSale).
    // Explicit wins (is present in output); pattern-derived (A, B) is suppressed.
    // B has profile "material" so it is NOT a sig seller — only A→B is pattern-derivable,
    // and the explicit pre-empts it. Net output: exactly 1 entry, from the explicit.
    let ic = IntercompanyConfig {
        relationships: vec![
            IcRelationshipConfig::Explicit(IcRelationshipExplicit {
                seller: "A".into(),
                buyer: "B".into(),
                types: vec![IcTransactionType::GoodsSale],
                annual_volume: dec!(999_999),
                transfer_pricing: Some(TransferPricingMethod::CostPlus),
                markup_percent: Some(dec!(0.10)),
            }),
            IcRelationshipConfig::Pattern(IcRelationshipPattern {
                pattern: IcPattern {
                    seller_scoping_profile: Some("sig".into()),
                    buyer_scoping_profile: Some("any".into()),
                    seller: None,
                    buyer: None,
                },
                types: vec![IcTransactionType::GoodsSale],
                per_pair_volume: dec!(100_000),
                transfer_pricing: None,
            }),
        ],
        matching: IcMatchingConfig::default(),
    };
    // A is "sig" (can be seller); B is "material" (cannot be a sig seller, only a buyer).
    // Pattern produces only (A→B); explicit covers (A→B) → explicit wins, 1 total result.
    let entities = vec![
        ent("A", "sig", ConsolidationMethod::Parent),
        ent("B", "material", ConsolidationMethod::Full),
    ];
    let out = expand_ic_relationships(&ic, &entities, 42).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].source, IcSource::Explicit);
    assert_eq!(
        out[0].annual_volume,
        dec!(999_999),
        "explicit annual_volume wins"
    );
}

#[test]
fn test_pattern_with_fixed_seller_and_wildcard_buyer() {
    let ic = IntercompanyConfig {
        relationships: vec![IcRelationshipConfig::Pattern(IcRelationshipPattern {
            pattern: IcPattern {
                seller: Some("A".into()),
                buyer_scoping_profile: Some("any".into()),
                seller_scoping_profile: None,
                buyer: None,
            },
            types: vec![IcTransactionType::ManagementFee],
            per_pair_volume: dec!(25_000),
            transfer_pricing: None,
        })],
        matching: IcMatchingConfig::default(),
    };
    let entities = vec![
        ent("A", "sig", ConsolidationMethod::Parent),
        ent("B", "sig", ConsolidationMethod::Full),
        ent("C", "material", ConsolidationMethod::Full),
    ];
    let out = expand_ic_relationships(&ic, &entities, 42).unwrap();
    // A → B and A → C = 2 pairs.
    assert_eq!(out.len(), 2);
    for rel in &out {
        assert_eq!(rel.seller, "A");
        assert!(rel.buyer == "B" || rel.buyer == "C");
    }
}

#[test]
fn test_unknown_seller_fails() {
    let ic = IntercompanyConfig {
        relationships: vec![IcRelationshipConfig::Explicit(IcRelationshipExplicit {
            seller: "NONEXISTENT".into(),
            buyer: "B".into(),
            types: vec![IcTransactionType::GoodsSale],
            annual_volume: dec!(1000),
            transfer_pricing: None,
            markup_percent: None,
        })],
        matching: IcMatchingConfig::default(),
    };
    let entities = vec![
        ent("A", "sig", ConsolidationMethod::Parent),
        ent("B", "sig", ConsolidationMethod::Full),
    ];
    let err = expand_ic_relationships(&ic, &entities, 42).unwrap_err();
    assert!(err.to_string().contains("NONEXISTENT"));
}

#[test]
fn test_self_pair_explicit_fails() {
    let ic = IntercompanyConfig {
        relationships: vec![IcRelationshipConfig::Explicit(IcRelationshipExplicit {
            seller: "A".into(),
            buyer: "A".into(),
            types: vec![IcTransactionType::GoodsSale],
            annual_volume: dec!(1000),
            transfer_pricing: None,
            markup_percent: None,
        })],
        matching: IcMatchingConfig::default(),
    };
    let entities = vec![ent("A", "sig", ConsolidationMethod::Parent)];
    let err = expand_ic_relationships(&ic, &entities, 42).unwrap_err();
    assert!(err.to_string().contains("seller == buyer"));
}

#[test]
fn test_ids_differ_across_triples_and_seeds() {
    let ic = IntercompanyConfig {
        relationships: vec![IcRelationshipConfig::Explicit(IcRelationshipExplicit {
            seller: "A".into(),
            buyer: "B".into(),
            types: vec![IcTransactionType::GoodsSale],
            annual_volume: dec!(1000),
            transfer_pricing: None,
            markup_percent: None,
        })],
        matching: IcMatchingConfig::default(),
    };
    let entities = vec![
        ent("A", "sig", ConsolidationMethod::Parent),
        ent("B", "sig", ConsolidationMethod::Full),
    ];
    let id_seed_42 = expand_ic_relationships(&ic, &entities, 42).unwrap()[0]
        .id
        .clone();
    let id_seed_43 = expand_ic_relationships(&ic, &entities, 43).unwrap()[0]
        .id
        .clone();
    assert_ne!(id_seed_42, id_seed_43, "different seed → different id");
}

#[test]
fn test_determinism() {
    let ic = IntercompanyConfig {
        relationships: vec![IcRelationshipConfig::Pattern(IcRelationshipPattern {
            pattern: IcPattern {
                seller_scoping_profile: Some("sig".into()),
                buyer_scoping_profile: Some("any".into()),
                seller: None,
                buyer: None,
            },
            types: vec![IcTransactionType::ManagementFee],
            per_pair_volume: dec!(50_000),
            transfer_pricing: None,
        })],
        matching: IcMatchingConfig::default(),
    };
    let entities = vec![
        ent("A", "sig", ConsolidationMethod::Parent),
        ent("B", "sig", ConsolidationMethod::Full),
        ent("C", "material", ConsolidationMethod::Full),
    ];
    let a = expand_ic_relationships(&ic, &entities, 42).unwrap();
    let b = expand_ic_relationships(&ic, &entities, 42).unwrap();
    assert_eq!(a, b);
}

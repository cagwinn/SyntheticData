//! Spec §2.4 — deterministic seed derivation is order-independent.

use chrono::NaiveDate;
use datasynth_group::{
    derive_aggregate_seed, derive_entity_seed, derive_ic_pair_id, derive_manifest_seed,
};

fn d(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

#[test]
fn test_seeds_are_deterministic() {
    let a = derive_manifest_seed(42, d(2024, 1, 1));
    let b = derive_manifest_seed(42, d(2024, 1, 1));
    assert_eq!(a, b);

    let a = derive_entity_seed(42, "NESTLE_SA");
    let b = derive_entity_seed(42, "NESTLE_SA");
    assert_eq!(a, b);

    let a = derive_aggregate_seed(42, d(2024, 1, 1));
    let b = derive_aggregate_seed(42, d(2024, 1, 1));
    assert_eq!(a, b);

    let a = derive_ic_pair_id(42, "ICR_001", 7);
    let b = derive_ic_pair_id(42, "ICR_001", 7);
    assert_eq!(a, b);
}

#[test]
fn test_different_inputs_produce_different_seeds() {
    let a = derive_manifest_seed(42, d(2024, 1, 1));
    let b = derive_manifest_seed(43, d(2024, 1, 1));
    assert_ne!(a, b, "different group_seed");

    let c = derive_manifest_seed(42, d(2024, 4, 1));
    assert_ne!(a, c, "different period_start");

    let d1 = derive_entity_seed(42, "A");
    let e = derive_entity_seed(42, "B");
    assert_ne!(d1, e, "different entity code");

    let f = derive_ic_pair_id(42, "ICR_001", 0);
    let g = derive_ic_pair_id(42, "ICR_001", 1);
    assert_ne!(f, g, "different pair index");

    let h = derive_ic_pair_id(42, "ICR_002", 0);
    assert_ne!(f, h, "different relationship id");
}

#[test]
fn test_entity_seeds_are_order_independent() {
    // Derive seeds for a set of entities in two different orders; compare pairwise.
    let codes_order_a = [
        "NESTLE_SA",
        "NESPRESSO_SA",
        "NESTLE_USA",
        "NESTLE_DE",
        "NESTLE_BR",
    ];
    let codes_order_b = [
        "NESTLE_BR",
        "NESTLE_SA",
        "NESTLE_DE",
        "NESPRESSO_SA",
        "NESTLE_USA",
    ];

    let seeds_a: std::collections::BTreeMap<&str, [u8; 32]> = codes_order_a
        .iter()
        .map(|c| (*c, derive_entity_seed(42, c)))
        .collect();
    let seeds_b: std::collections::BTreeMap<&str, [u8; 32]> = codes_order_b
        .iter()
        .map(|c| (*c, derive_entity_seed(42, c)))
        .collect();

    assert_eq!(
        seeds_a, seeds_b,
        "entity seeds must not depend on iteration order"
    );
}

#[test]
fn test_adding_entity_does_not_perturb_existing_seeds() {
    // The additive property: adding a new entity to the list must not change
    // any of the existing entities' seeds.
    let existing = ["A", "B", "C"];
    let expanded = ["A", "B", "C", "D"];

    for code in &existing {
        let before = derive_entity_seed(42, code);
        let after = derive_entity_seed(42, code);
        assert_eq!(before, after);
        // Presence of "D" in the expanded set cannot influence A/B/C because
        // derive_entity_seed takes only (group_seed, code) — this test codifies
        // the API contract, not the implementation detail.
    }

    // Also: the new entity's seed is different from all existing ones.
    let new_seed = derive_entity_seed(42, "D");
    for code in &existing {
        assert_ne!(new_seed, derive_entity_seed(42, code));
    }

    // Length check: nothing broken (silence unused warning).
    assert_eq!(expanded.len(), existing.len() + 1);
}

#[test]
fn test_ic_pair_id_mirrors_across_roles() {
    // Both the seller shard and the buyer shard derive the same pair_id for
    // the same (relationship_id, pair_index) input. The derivation must NOT
    // depend on entity role — it is purely a function of (group_seed,
    // relationship_id, pair_index).
    let pair_id_seller = derive_ic_pair_id(42, "ICR_NESPRESSO_TO_USA", 17);
    let pair_id_buyer = derive_ic_pair_id(42, "ICR_NESPRESSO_TO_USA", 17);
    assert_eq!(pair_id_seller, pair_id_buyer);
}

#[test]
fn test_chacha_rng_is_deterministic() {
    use datasynth_group::chacha_rng_from_seed;
    use rand::Rng;

    let seed = derive_entity_seed(42, "NESTLE_SA");
    let mut rng1 = chacha_rng_from_seed(&seed);
    let mut rng2 = chacha_rng_from_seed(&seed);

    let samples1: Vec<u64> = (0..16).map(|_| rng1.next_u64()).collect();
    let samples2: Vec<u64> = (0..16).map(|_| rng2.next_u64()).collect();

    assert_eq!(samples1, samples2, "same seed → same RNG stream");
}

//! Task 3.4 — cross-entity determinism property test.
//!
//! Runs [`derive_ic_pair_plans`] for every entity in a 5-entity Mini-Nestlé
//! manifest and asserts that every seller-side plan has a buyer-side mirror
//! with the same `pair_id`, `amount`, `date`, `transaction_type`, and
//! `ic_relationship_id` — and an opposite `role`.
//!
//! This is what makes aggregate-phase IC matching by `pair_id` a no-tiebreak
//! join: it's guaranteed by construction, not by approximate fuzzy logic.
//! 100 % coverage is the bar; any drop here means a deterministic-seed
//! regression that must be fixed before the aggregate phase can trust the
//! invariant.

use std::collections::BTreeMap;

use datasynth_core::models::IcPairId;
use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::shard::{derive_ic_pair_plans, IcPairPlan, IcRole};
use datasynth_group::{build_manifest, GroupConfig};

fn load_manifest() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse into GroupConfig");
    build_manifest(&cfg).expect("mini_nestle.yaml must build a manifest")
}

/// Lookup key used to mirror-match plans across shards. Two plans with
/// identical `(ic_relationship_id, index)` must be halves of the same pair
/// by construction.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PairKey(String, u64);

fn key(p: &IcPairPlan) -> PairKey {
    PairKey(p.ic_relationship_id.clone(), p.index)
}

/// Walk every entity, collect every seller-side plan, and assert that the
/// buyer-side shard of the same relationship has a mirror plan with
/// byte-identical `pair_id`, `amount`, `date`, and `transaction_type`.
///
/// Coverage target: 100 % — every seller plan must find its buyer mirror.
#[test]
fn every_seller_plan_has_a_mirror_buyer_plan() {
    let manifest = load_manifest();

    // All entity codes in ownership graph.
    let entity_codes: Vec<String> = manifest
        .ownership_graph
        .entities
        .iter()
        .map(|e| e.code.clone())
        .collect();
    assert!(
        entity_codes.len() >= 5,
        "fixture sanity: expected at least 5 entities, got {}",
        entity_codes.len()
    );

    // Build a lookup from (relationship_id, index) → (plan, entity_code)
    // for buyer-side plans across every entity. We'll probe this map from
    // the seller side.
    let mut buyer_index: BTreeMap<PairKey, (IcPairPlan, String)> = BTreeMap::new();
    let mut seller_index: BTreeMap<PairKey, (IcPairPlan, String)> = BTreeMap::new();

    for entity in &entity_codes {
        let plans = derive_ic_pair_plans(&manifest, entity);
        for p in plans {
            match p.role {
                IcRole::Seller => {
                    let existing = seller_index.insert(key(&p), (p.clone(), entity.clone()));
                    assert!(
                        existing.is_none(),
                        "duplicate seller plan at {:?} — would break no-tiebreak matching",
                        key(&p)
                    );
                }
                IcRole::Buyer => {
                    let existing = buyer_index.insert(key(&p), (p.clone(), entity.clone()));
                    assert!(
                        existing.is_none(),
                        "duplicate buyer plan at {:?} — would break no-tiebreak matching",
                        key(&p)
                    );
                }
            }
        }
    }

    // Sanity: the fixture must produce at least one pair so the coverage
    // assertion below is non-vacuous.
    assert!(
        !seller_index.is_empty(),
        "fixture sanity: expected at least one seller-side IC pair plan"
    );

    // Coverage: every seller plan must have a matching buyer plan.
    let mut matched = 0usize;
    let mut unmatched: Vec<PairKey> = Vec::new();
    for (k, (seller_plan, seller_entity)) in &seller_index {
        match buyer_index.get(k) {
            Some((buyer_plan, buyer_entity)) => {
                matched += 1;

                // Mirror-image invariants — identical across both shards.
                assert_eq!(
                    seller_plan.pair_id, buyer_plan.pair_id,
                    "pair_id mismatch at {:?} (seller={}, buyer={})",
                    k, seller_entity, buyer_entity
                );
                assert_eq!(
                    seller_plan.amount, buyer_plan.amount,
                    "amount mismatch at {:?}: seller={} vs buyer={}",
                    k, seller_plan.amount, buyer_plan.amount
                );
                assert_eq!(
                    seller_plan.date, buyer_plan.date,
                    "date mismatch at {:?}: seller={} vs buyer={}",
                    k, seller_plan.date, buyer_plan.date
                );
                assert_eq!(
                    seller_plan.transaction_type, buyer_plan.transaction_type,
                    "transaction_type mismatch at {:?}",
                    k
                );

                // Opposite roles + swapped partner_entity.
                assert_eq!(seller_plan.role, IcRole::Seller);
                assert_eq!(buyer_plan.role, IcRole::Buyer);
                assert_eq!(
                    &seller_plan.partner_entity, buyer_entity,
                    "seller's partner_entity must equal the buyer's own entity_code"
                );
                assert_eq!(
                    &buyer_plan.partner_entity, seller_entity,
                    "buyer's partner_entity must equal the seller's own entity_code"
                );
            }
            None => unmatched.push(k.clone()),
        }
    }

    let total_seller_plans = seller_index.len();
    let coverage = matched as f64 / total_seller_plans as f64;

    assert_eq!(
        unmatched,
        Vec::<PairKey>::new(),
        "expected 100% coverage — {} of {} seller plans had no buyer mirror: {:?}",
        unmatched.len(),
        total_seller_plans,
        unmatched
    );
    assert!(
        (coverage - 1.0).abs() < f64::EPSILON,
        "coverage {} < 1.0 ({} matched / {} total)",
        coverage,
        matched,
        total_seller_plans
    );
}

/// Every `pair_id` emitted across the entire group (both sides) must be
/// unique per logical pair — never a collision with a different
/// (relationship_id, index) coordinate. A collision would cause the
/// aggregate phase to join wrong halves.
#[test]
fn pair_ids_do_not_collide_across_entities() {
    let manifest = load_manifest();

    let mut pair_id_to_key: BTreeMap<IcPairId, PairKey> = BTreeMap::new();

    for entity in &manifest.ownership_graph.entities {
        let plans = derive_ic_pair_plans(&manifest, &entity.code);
        for p in plans {
            let k = key(&p);
            if let Some(existing_key) = pair_id_to_key.get(&p.pair_id) {
                // Seeing the same pair_id at the same (rel_id, index) is
                // expected — that's literally the mirror property. Seeing
                // it at a different key is a collision.
                assert_eq!(
                    existing_key, &k,
                    "pair_id {:?} collides across two different (rel_id, index) pairs: {:?} vs {:?}",
                    p.pair_id, existing_key, k
                );
            } else {
                pair_id_to_key.insert(p.pair_id, k);
            }
        }
    }

    assert!(
        !pair_id_to_key.is_empty(),
        "fixture sanity: expected at least one derived pair_id"
    );
}

/// Walking the entire group and aggregating every seller's plan count must
/// equal the aggregate of every buyer's plan count — a basic conservation
/// law. Any asymmetry would indicate an entity being missed on one side.
#[test]
fn seller_and_buyer_totals_are_equal() {
    let manifest = load_manifest();

    let mut total_seller = 0usize;
    let mut total_buyer = 0usize;
    for entity in &manifest.ownership_graph.entities {
        let plans = derive_ic_pair_plans(&manifest, &entity.code);
        for p in plans {
            match p.role {
                IcRole::Seller => total_seller += 1,
                IcRole::Buyer => total_buyer += 1,
            }
        }
    }

    assert_eq!(
        total_seller, total_buyer,
        "conservation violated: {} seller plans vs {} buyer plans",
        total_seller, total_buyer
    );
    assert!(total_seller > 0, "fixture must produce at least one pair");
}

//! Spec §5.2 / Task 3.2 — IC pair plan derivation.
//!
//! These tests verify the mirror-image determinism guarantee: the seller's
//! shard and the buyer's shard derive byte-identical `pair_id`, `amount`,
//! `date`, and `transaction_type` for every index `i`, with opposite
//! `role`s and swapped `partner_entity` fields.

use chrono::NaiveDate;
use datasynth_group::config::IcTransactionType;
use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::shard::ic_plan::{avg_amount, derive_ic_pair_plans, IcPairPlan, IcRole};
use datasynth_group::{build_manifest, GroupConfig};
use rust_decimal::prelude::ToPrimitive;

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn load_mini_nestle_manifest() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse into GroupConfig");
    build_manifest(&cfg).expect("mini_nestle.yaml must build a manifest")
}

/// A manifest with no intercompany relationships at all.  Built by taking
/// the mini_nestle fixture and overwriting `intercompany.relationships`
/// with an empty list.  Used by tests that need a well-formed manifest
/// but want to exercise the no-IC branch of the derivation.
fn load_manifest_without_ic() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let mut cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse into GroupConfig");
    cfg.intercompany.relationships.clear();
    build_manifest(&cfg).expect("mutated mini_nestle must still build a manifest")
}

/// Find a relationship where both endpoints are explicit entities we can
/// call the derivation on from both sides.  Chooses the first one in the
/// manifest so it's deterministic.
fn first_bilateral_relationship(
    manifest: &GroupManifest,
) -> (String, String, String, IcTransactionType) {
    let rel = manifest
        .ic_relationships
        .first()
        .expect("mini_nestle fixture must have at least one IC relationship");
    let tx_type = *rel
        .types
        .first()
        .expect("every IC relationship must have a non-empty types list");
    (
        rel.seller.clone(),
        rel.buyer.clone(),
        rel.id.clone(),
        tx_type,
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Mirror-image determinism: the seller's plans and the buyer's plans for
/// the same relationship must be byte-identical except for `role` and
/// `partner_entity`.  This is the core property that makes aggregate-phase
/// IC matching by `pair_id` a no-tiebreak join.
#[test]
fn test_seller_and_buyer_produce_mirror_plans() {
    let manifest = load_mini_nestle_manifest();
    let (seller, buyer, rel_id, tx_type) = first_bilateral_relationship(&manifest);

    let seller_plans = derive_ic_pair_plans(&manifest, &seller);
    let buyer_plans = derive_ic_pair_plans(&manifest, &buyer);

    // Filter down to just the relationship under test.
    let seller_for_rel: Vec<&IcPairPlan> = seller_plans
        .iter()
        .filter(|p| p.ic_relationship_id == rel_id)
        .collect();
    let buyer_for_rel: Vec<&IcPairPlan> = buyer_plans
        .iter()
        .filter(|p| p.ic_relationship_id == rel_id)
        .collect();

    assert_eq!(
        seller_for_rel.len(),
        buyer_for_rel.len(),
        "seller and buyer must produce the same number of pairs for relationship {}",
        rel_id
    );
    assert!(
        !seller_for_rel.is_empty(),
        "relationship {} must produce at least one pair",
        rel_id
    );

    for (s, b) in seller_for_rel.iter().zip(buyer_for_rel.iter()) {
        // Mirror-image fields — identical on both sides.
        assert_eq!(
            s.pair_id, b.pair_id,
            "pair_id mismatch at index {}",
            s.index
        );
        assert_eq!(s.amount, b.amount, "amount mismatch at index {}", s.index);
        assert_eq!(s.date, b.date, "date mismatch at index {}", s.index);
        assert_eq!(
            s.transaction_type, b.transaction_type,
            "transaction_type mismatch at index {}",
            s.index
        );
        assert_eq!(s.index, b.index, "plans must be paired at the same index");
        assert_eq!(
            s.transaction_type, tx_type,
            "transaction_type must be the dominant type"
        );

        // Opposite roles — the defining asymmetry.
        assert_eq!(
            s.role,
            IcRole::Seller,
            "seller side must report Seller role"
        );
        assert_eq!(b.role, IcRole::Buyer, "buyer side must report Buyer role");

        // Swapped partner_entity — each side knows its counterparty.
        assert_eq!(
            s.partner_entity, buyer,
            "seller side's partner must be the buyer"
        );
        assert_eq!(
            b.partner_entity, seller,
            "buyer side's partner must be the seller"
        );
    }
}

/// Pair count must match `round(annual_volume / avg_amount(tx_type))`,
/// clamped to at least 1.  Verifies the formula the shard phase relies on
/// for work budgeting.
#[test]
fn test_pair_count_matches_formula() {
    let manifest = load_mini_nestle_manifest();
    let (seller, _buyer, rel_id, tx_type) = first_bilateral_relationship(&manifest);

    // Recompute expected N from first principles.
    let rel = manifest
        .ic_relationships
        .iter()
        .find(|r| r.id == rel_id)
        .expect("relationship must exist");
    let avg = avg_amount(tx_type);
    let raw = rel.annual_volume / avg;
    let rounded = raw
        .round()
        .to_u64()
        .expect("must fit in u64 for this fixture");
    let expected_n = rounded.max(1);

    let seller_plans = derive_ic_pair_plans(&manifest, &seller);
    let actual_n = seller_plans
        .iter()
        .filter(|p| p.ic_relationship_id == rel_id)
        .count() as u64;

    assert_eq!(
        actual_n, expected_n,
        "pair count for {} must equal round({} / {}) = {}",
        rel_id, rel.annual_volume, avg, expected_n
    );
}

/// Dates must span the engagement period inclusively: the first plan's
/// date is `period.start` and the last plan's date is `period.end`
/// (when N >= 2).  This is what guarantees aggregate-phase close-date
/// analyses see IC postings on the close date itself.
#[test]
fn test_dates_span_period() {
    let manifest = load_mini_nestle_manifest();
    let period_start = manifest.period.start;
    let period_end = manifest.period.end;
    let (seller, _buyer, rel_id, _tx_type) = first_bilateral_relationship(&manifest);

    let seller_plans = derive_ic_pair_plans(&manifest, &seller);
    let rel_plans: Vec<&IcPairPlan> = seller_plans
        .iter()
        .filter(|p| p.ic_relationship_id == rel_id)
        .collect();

    assert!(
        rel_plans.len() >= 2,
        "test assumes a relationship with N >= 2 pairs; got {}",
        rel_plans.len()
    );

    assert_eq!(
        rel_plans.first().unwrap().date,
        period_start,
        "first plan must be dated period.start"
    );
    assert_eq!(
        rel_plans.last().unwrap().date,
        period_end,
        "last plan must be dated period.end"
    );

    // Every plan must fall within [start, end].
    for p in &rel_plans {
        assert!(
            p.date >= period_start && p.date <= period_end,
            "plan at index {} dated {} is outside [{}, {}]",
            p.index,
            p.date,
            period_start,
            period_end
        );
    }
}

/// An entity that exists in the manifest but is not named in any IC
/// relationship returns an empty plan list.  We build a manifest from
/// the mini_nestle fixture but clear all IC relationships so every
/// entity is a non-participant.
#[test]
fn test_non_participant_entity_returns_empty() {
    let manifest = load_manifest_without_ic();
    // Sanity: we really did strip the relationships.
    assert!(
        manifest.ic_relationships.is_empty(),
        "fixture wiring: manifest.ic_relationships must be empty for this test"
    );
    for entity_code in [
        "NESTLE_SA",
        "NESTLE_USA",
        "NESTLE_DE",
        "NESTLE_BR",
        "NESTLE_JV",
    ] {
        let plans = derive_ic_pair_plans(&manifest, entity_code);
        assert!(
            plans.is_empty(),
            "entity {} with no IC relationships must return an empty plan list; got {} plans",
            entity_code,
            plans.len()
        );
    }
}

/// An entity code that does not appear in the manifest at all returns an
/// empty plan list — no panic, no lookup failure.  The derivation is a
/// plain filter over relationships, so an unknown code simply matches
/// nothing.
#[test]
fn test_entity_not_in_manifest_returns_empty() {
    let manifest = load_mini_nestle_manifest();
    let plans = derive_ic_pair_plans(&manifest, "ENTITY_THAT_DOES_NOT_EXIST");
    assert!(
        plans.is_empty(),
        "unknown entity code must return an empty plan list; got {} plans",
        plans.len()
    );
}

/// Running the derivation twice with the same arguments must produce
/// bitwise-equal `Vec<IcPairPlan>`s.  This guards against accidental
/// introduction of non-determinism (HashMap iteration, wall-clock time,
/// thread ordering).
#[test]
fn test_deterministic_across_calls() {
    let manifest = load_mini_nestle_manifest();

    // Pick an entity that participates in several relationships so we
    // cover multiple branches in the derivation (seller + buyer).
    let a = derive_ic_pair_plans(&manifest, "NESTLE_SA");
    let b = derive_ic_pair_plans(&manifest, "NESTLE_SA");
    assert_eq!(
        a, b,
        "two calls with the same arguments must return equal Vec<IcPairPlan>"
    );

    // Also exercise a buyer-side entity.
    let a = derive_ic_pair_plans(&manifest, "NESTLE_USA");
    let b = derive_ic_pair_plans(&manifest, "NESTLE_USA");
    assert_eq!(a, b);
}

/// Cross-check: all pair_ids across all relationships for a single entity
/// are globally unique.  Collisions would indicate a seed-derivation bug
/// that could produce phantom matches in the aggregate phase.
#[test]
fn test_pair_ids_are_unique_across_all_relationships() {
    let manifest = load_mini_nestle_manifest();
    let plans = derive_ic_pair_plans(&manifest, "NESTLE_SA");
    let mut seen = std::collections::BTreeSet::new();
    for p in &plans {
        assert!(
            seen.insert(p.pair_id),
            "duplicate pair_id {:?} for entity NESTLE_SA — this would break aggregate matching",
            p.pair_id
        );
    }
    assert!(
        !plans.is_empty(),
        "NESTLE_SA must have at least one IC pair plan in the mini_nestle fixture"
    );
}

/// Every plan's `index` must match its position within the filtered
/// per-relationship list (0, 1, 2, ...) and monotonically increase.
#[test]
fn test_indices_are_contiguous_per_relationship() {
    let manifest = load_mini_nestle_manifest();
    let plans = derive_ic_pair_plans(&manifest, "NESTLE_SA");

    // Group by relationship id, preserve order.
    let mut by_rel: std::collections::BTreeMap<String, Vec<&IcPairPlan>> =
        std::collections::BTreeMap::new();
    for p in &plans {
        by_rel
            .entry(p.ic_relationship_id.clone())
            .or_default()
            .push(p);
    }

    for (rel_id, rel_plans) in &by_rel {
        for (expected_i, plan) in rel_plans.iter().enumerate() {
            assert_eq!(
                plan.index, expected_i as u64,
                "plan index for relationship {} at position {} must be {}; got {}",
                rel_id, expected_i, expected_i, plan.index
            );
        }
    }
}

/// Sanity: the period start from the mini_nestle fixture is 2024-01-01
/// and the period is quarterly — pins the test data so later spec tweaks
/// don't silently invalidate this file.
#[test]
fn test_mini_nestle_fixture_period_is_stable() {
    let manifest = load_mini_nestle_manifest();
    assert_eq!(
        manifest.period.start,
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
    );
    assert_eq!(
        manifest.period.end,
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
    );
}

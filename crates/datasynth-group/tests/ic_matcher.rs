//! Task 5.3 — IC pair matcher integration tests.
//!
//! These tests build their fixtures by composing `derive_ic_pair_plans` +
//! `inject_ic_journal_entries` directly, deliberately bypassing the
//! orchestrator.  That gives us full control over which sides exist in
//! `entity_jes` so we can exercise both the happy path and every
//! unmatched / corruption shape without setting up a full shard run.
//!
//! The fixture is the trimmed `mini_acme.yaml` (ACME_SA + ACME_USA
//! kept, every other entity removed from the ownership graph and every
//! relationship not strictly between those two pruned).  This keeps the
//! plan count small but still exercises a real manifest-built path
//! (`build_manifest` → `derive_ic_pair_plans` → `inject_ic_journal_entries`).

use chrono::NaiveDate;
use rust_decimal::Decimal;

use datasynth_core::models::journal_entry::{JournalEntryHeader, JournalEntryLine};
use datasynth_core::models::{IcPairId, JournalEntry};
use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::shard::{
    derive_ic_pair_plans, inject_ic_journal_entries, IcRole, InjectionCtx,
};
use datasynth_group::{build_manifest, match_ic_pairs, GroupConfig, GroupError, UnmatchedReason};

// ── Fixture builders ──────────────────────────────────────────────────────────

/// Trim the full mini_acme YAML to the two-entity universe we use in
/// these tests: ACME_SA (the parent / seller) and ACME_USA (a Full
/// subsidiary).  Drops every other ownership-graph entity and every IC
/// relationship that names a non-retained entity.
fn load_two_entity_manifest() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_acme.yaml");
    let mut cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_acme.yaml must parse into GroupConfig");

    // Keep only ACME_SA and ACME_USA in the ownership graph.
    cfg.ownership
        .entities
        .retain(|e| e.code == "ACME_SA" || e.code == "ACME_USA");

    // Trim explicit IC relationships to ones whose endpoints survived.
    cfg.intercompany.relationships.retain(|rel| {
        use datasynth_group::config::IcRelationshipConfig;
        match rel {
            IcRelationshipConfig::Explicit(e) => {
                (e.seller == "ACME_SA" || e.seller == "ACME_USA")
                    && (e.buyer == "ACME_SA" || e.buyer == "ACME_USA")
            }
            // Patterns that fan out across all entities will reduce to
            // SA<->USA pairs once entities are trimmed.  Keep them.
            IcRelationshipConfig::Pattern(_) => true,
        }
    });

    // The fixture's pillar-two jurisdiction list may reference dropped
    // entities' countries — narrow it to the surviving CH/US set, and
    // similarly trim transfer-pricing local-files to the same set, so
    // `build_manifest` doesn't reject references to gone entities.
    if let Some(p2) = cfg.tax.pillar_two.as_mut() {
        p2.jurisdictions.retain(|j| j == "CH" || j == "US");
    }
    if let Some(tp) = cfg.tax.transfer_pricing.as_mut() {
        tp.local_files_for.retain(|j| j == "CH" || j == "US");
    }

    build_manifest(&cfg).expect("trimmed mini_acme must still build a manifest")
}

/// Like [`load_two_entity_manifest`] but for the no-IC case: clears
/// `intercompany.relationships` entirely.  Used by the empty-manifest
/// coverage test.
fn load_two_entity_manifest_no_ic() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_acme.yaml");
    let mut cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_acme.yaml must parse into GroupConfig");
    cfg.ownership
        .entities
        .retain(|e| e.code == "ACME_SA" || e.code == "ACME_USA");
    cfg.intercompany.relationships.clear();
    if let Some(p2) = cfg.tax.pillar_two.as_mut() {
        p2.jurisdictions.retain(|j| j == "CH" || j == "US");
    }
    if let Some(tp) = cfg.tax.transfer_pricing.as_mut() {
        tp.local_files_for.retain(|j| j == "CH" || j == "US");
    }
    build_manifest(&cfg).expect("manifest with no IC relationships must still build")
}

/// Generate the seller-side (ACME_SA) JEs from the manifest's plans.
fn sa_jes(manifest: &GroupManifest) -> Vec<JournalEntry> {
    let plans = derive_ic_pair_plans(manifest, "ACME_SA");
    inject_ic_journal_entries(
        &plans,
        &InjectionCtx {
            entity_code: "ACME_SA".to_string(),
        },
    )
}

/// Generate the buyer-side (ACME_USA) JEs from the manifest's plans.
fn usa_jes(manifest: &GroupManifest) -> Vec<JournalEntry> {
    let plans = derive_ic_pair_plans(manifest, "ACME_USA");
    inject_ic_journal_entries(
        &plans,
        &InjectionCtx {
            entity_code: "ACME_USA".to_string(),
        },
    )
}

/// Number of pairs the trimmed manifest plans across both entities.
/// Recomputed from `derive_ic_pair_plans` filtered to seller plans.
fn expected_pair_count(manifest: &GroupManifest) -> usize {
    [&"ACME_SA", &"ACME_USA"]
        .iter()
        .map(|code| {
            derive_ic_pair_plans(manifest, code)
                .into_iter()
                .filter(|p| p.role == IcRole::Seller)
                .count()
        })
        .sum()
}

/// Build a 1-line JE that has no `ic_pair_id` set.  Used by the
/// "non-IC JEs are silently ignored" test.
fn non_ic_je() -> JournalEntry {
    let header = JournalEntryHeader::new(
        "ACME_SA".to_string(),
        NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
    );
    let mut je = JournalEntry::new(header);
    let doc_id = je.header.document_id;
    je.add_line(JournalEntryLine::debit(
        doc_id,
        1,
        "1000".to_string(),
        Decimal::from(100),
    ));
    je.add_line(JournalEntryLine::credit(
        doc_id,
        2,
        "4000".to_string(),
        Decimal::from(100),
    ));
    je
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Happy path: every plan has both sides → 100 % coverage, zero
/// unmatched.
#[test]
fn happy_path_full_coverage() {
    let manifest = load_two_entity_manifest();
    let total = expected_pair_count(&manifest);
    assert!(
        total >= 1,
        "fixture sanity: trimmed mini_acme must yield at least one pair"
    );

    let result = match_ic_pairs(
        &manifest,
        &[
            ("ACME_SA".to_string(), sa_jes(&manifest)),
            ("ACME_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");

    assert_eq!(result.matched.len(), total, "every plan must match");
    assert!(result.unmatched.is_empty(), "no unmatched expected");
    assert_eq!(result.total_planned, total);
    assert!(
        (result.coverage - 1.0).abs() < f64::EPSILON,
        "coverage = 1.0 expected; got {}",
        result.coverage
    );

    // Spot-check the matched shape: seller and buyer entity codes line
    // up with the manifest's seller / buyer.
    for pair in &result.matched {
        assert_eq!(pair.seller_entity, "ACME_SA");
        assert_eq!(pair.buyer_entity, "ACME_USA");
        assert_eq!(pair.seller_je.header.ic_pair_id, Some(pair.pair_id));
        assert_eq!(pair.buyer_je.header.ic_pair_id, Some(pair.pair_id));
        assert_eq!(pair.seller_je.header.company_code, "ACME_SA");
        assert_eq!(pair.buyer_je.header.company_code, "ACME_USA");
    }
}

/// Drop the buyer's JEs entirely → every SA-derived seller plan
/// becomes `MissingBuyerSide`.
#[test]
fn missing_buyer_side_becomes_unmatched() {
    let manifest = load_two_entity_manifest();
    let total = expected_pair_count(&manifest);
    assert!(total >= 1, "fixture sanity");

    // Note: ACME_SA might not be the seller for *every* relationship
    // in the trimmed manifest — the pattern-derived relationships could
    // produce USA→SA pairs too.  We split the SA plans into the ones
    // where SA is the seller and the ones where it's the buyer.
    let sa_plans = derive_ic_pair_plans(&manifest, "ACME_SA");
    let sa_seller_count = sa_plans.iter().filter(|p| p.role == IcRole::Seller).count();
    let sa_buyer_count = sa_plans.iter().filter(|p| p.role == IcRole::Buyer).count();

    let result =
        match_ic_pairs(&manifest, &[("ACME_SA".to_string(), sa_jes(&manifest))]).expect("match");

    assert!(result.matched.is_empty(), "no matches possible without USA");
    assert_eq!(
        result.unmatched.len(),
        sa_seller_count + sa_buyer_count,
        "every SA-side plan must be unmatched (one report per plan)"
    );

    // Roles split: SA-as-seller plans report MissingBuyerSide, and
    // SA-as-buyer plans report MissingSellerSide.
    let missing_buyer = result
        .unmatched
        .iter()
        .filter(|u| u.reason == UnmatchedReason::MissingBuyerSide)
        .count();
    let missing_seller = result
        .unmatched
        .iter()
        .filter(|u| u.reason == UnmatchedReason::MissingSellerSide)
        .count();
    assert_eq!(missing_buyer, sa_seller_count);
    assert_eq!(missing_seller, sa_buyer_count);

    assert_eq!(result.total_planned, total);
    assert!(
        result.coverage.abs() < f64::EPSILON,
        "coverage = 0.0 expected; got {}",
        result.coverage
    );
}

/// Drop the seller's JEs entirely → every USA-derived buyer plan
/// becomes `MissingSellerSide`.
#[test]
fn missing_seller_side_becomes_unmatched() {
    let manifest = load_two_entity_manifest();
    let total = expected_pair_count(&manifest);
    assert!(total >= 1, "fixture sanity");

    let usa_plans = derive_ic_pair_plans(&manifest, "ACME_USA");
    let usa_seller_count = usa_plans
        .iter()
        .filter(|p| p.role == IcRole::Seller)
        .count();
    let usa_buyer_count = usa_plans.iter().filter(|p| p.role == IcRole::Buyer).count();

    let result =
        match_ic_pairs(&manifest, &[("ACME_USA".to_string(), usa_jes(&manifest))]).expect("match");

    assert!(result.matched.is_empty());
    assert_eq!(result.unmatched.len(), usa_seller_count + usa_buyer_count);

    let missing_seller = result
        .unmatched
        .iter()
        .filter(|u| u.reason == UnmatchedReason::MissingSellerSide)
        .count();
    let missing_buyer = result
        .unmatched
        .iter()
        .filter(|u| u.reason == UnmatchedReason::MissingBuyerSide)
        .count();
    assert_eq!(missing_seller, usa_buyer_count);
    assert_eq!(missing_buyer, usa_seller_count);

    assert_eq!(result.total_planned, total);
    assert!(result.coverage.abs() < f64::EPSILON);
}

/// Empty `entity_jes` against a manifest with IC relationships → no
/// matched, no unmatched, but `total_planned > 0` and `coverage = 0.0`.
/// This exercises the "invisible loss" semantics documented in the
/// module-level rustdoc.
#[test]
fn empty_input_with_ic_relationships_yields_zero_coverage() {
    let manifest = load_two_entity_manifest();
    let total = expected_pair_count(&manifest);
    assert!(total >= 1);

    let result = match_ic_pairs(&manifest, &[]).expect("match");

    assert!(result.matched.is_empty());
    assert!(result.unmatched.is_empty());
    assert_eq!(result.total_planned, total);
    assert!(result.coverage.abs() < f64::EPSILON);
}

/// Manifest with no IC relationships → `total_planned = 0`, `coverage =
/// 0.0` (not 1.0 — see module rustdoc).
#[test]
fn empty_manifest_yields_zero_coverage_not_one() {
    let manifest = load_two_entity_manifest_no_ic();
    assert!(manifest.ic_relationships.is_empty(), "fixture sanity");

    let result = match_ic_pairs(
        &manifest,
        &[
            ("ACME_SA".to_string(), Vec::new()),
            ("ACME_USA".to_string(), Vec::new()),
        ],
    )
    .expect("match");

    assert!(result.matched.is_empty());
    assert!(result.unmatched.is_empty());
    assert_eq!(result.total_planned, 0);
    assert!(
        result.coverage.abs() < f64::EPSILON,
        "0/0 maps to 0.0 not 1.0; got {}",
        result.coverage
    );
}

/// Non-IC JEs (with `ic_pair_id == None`) mixed into the input must be
/// silently ignored — no impact on matched, unmatched, or counts.
#[test]
fn non_ic_jes_are_silently_ignored() {
    let manifest = load_two_entity_manifest();
    let total = expected_pair_count(&manifest);

    let mut sa_jes_with_noise = sa_jes(&manifest);
    sa_jes_with_noise.push(non_ic_je());
    sa_jes_with_noise.insert(0, non_ic_je());
    let usa_jes = usa_jes(&manifest);

    let result = match_ic_pairs(
        &manifest,
        &[
            ("ACME_SA".to_string(), sa_jes_with_noise),
            ("ACME_USA".to_string(), usa_jes),
        ],
    )
    .expect("match");

    assert_eq!(
        result.matched.len(),
        total,
        "non-IC JEs must not affect the match count"
    );
    assert!(result.unmatched.is_empty());
}

/// Two calls produce byte-identical serialised result.
#[test]
fn deterministic_serialisation() {
    let manifest = load_two_entity_manifest();
    let inputs = vec![
        ("ACME_SA".to_string(), sa_jes(&manifest)),
        ("ACME_USA".to_string(), usa_jes(&manifest)),
    ];

    let a = match_ic_pairs(&manifest, &inputs).expect("match");
    let b = match_ic_pairs(&manifest, &inputs).expect("match");

    let a_json = serde_json::to_vec(&a).expect("serialise");
    let b_json = serde_json::to_vec(&b).expect("serialise");
    assert_eq!(a_json, b_json, "two calls must produce identical bytes");
}

/// Matched output is sorted lexicographically by `pair_id` — sanity-check
/// the determinism contract.
#[test]
fn matched_output_is_sorted_by_pair_id() {
    let manifest = load_two_entity_manifest();
    let result = match_ic_pairs(
        &manifest,
        &[
            ("ACME_SA".to_string(), sa_jes(&manifest)),
            ("ACME_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");

    let pair_ids: Vec<IcPairId> = result.matched.iter().map(|p| p.pair_id).collect();
    let mut sorted = pair_ids.clone();
    sorted.sort();
    assert_eq!(pair_ids, sorted, "matched must be pre-sorted by pair_id");
}

/// Bizarre case: same entity supplies both seller AND buyer side of one
/// pair.  Synthesised by injecting two SA-side JEs that both carry the
/// same pair_id (one Seller plan + a forged Buyer plan with the same
/// pair_id but for SA).  The matcher must reject this with
/// `GroupError::Aggregate` naming the pair.
#[test]
fn duplicate_role_returns_aggregate_error() {
    let manifest = load_two_entity_manifest();
    let total = expected_pair_count(&manifest);
    assert!(total >= 1, "fixture sanity");

    // Real seller-side JE for the first pair.
    let mut sa_jes_buf = sa_jes(&manifest);
    let first_pair_id = sa_jes_buf
        .iter()
        .find_map(|je| je.header.ic_pair_id)
        .expect("at least one SA JE must have ic_pair_id");

    // Forge a second JE with the same pair_id but tag it as if it were
    // posted by ACME_SA in the buyer role of a different relationship
    // (we only need the matcher to see two SA-side observations of the
    // same pair_id to trigger the corruption check).  The simplest way:
    // duplicate the first JE but bump some lines so it's "different"
    // — the matcher only inspects pair_id + entity_code, not lines.
    let dup = sa_jes_buf[0].clone();
    sa_jes_buf.push(dup);

    // Ensure the duplicated JE shares the pair_id of the first.
    assert_eq!(
        sa_jes_buf.last().unwrap().header.ic_pair_id,
        Some(first_pair_id)
    );

    let usa_jes = usa_jes(&manifest);

    let err = match_ic_pairs(
        &manifest,
        &[
            ("ACME_SA".to_string(), sa_jes_buf),
            ("ACME_USA".to_string(), usa_jes),
        ],
    );

    // We've now produced 3 sides for pair_id #1 (SA, USA, SA) — the
    // matcher must reject as "more than 2 sides".
    let err = err.expect_err("must reject corrupt input");
    match err {
        GroupError::Aggregate(msg) => {
            assert!(
                msg.contains(&first_pair_id.to_string()),
                "error must name the offending pair_id: {msg}"
            );
        }
        other => panic!("expected GroupError::Aggregate, got {other:?}"),
    }
}

/// A subtler corruption: exactly two sides observed for one pair, but
/// both with the same role (e.g. the seller's shard re-runs and writes
/// duplicate seller-side JEs under two entity codes).  We synthesise
/// this by taking a real seller-side JE, cloning it under a new entity
/// code that the matcher will lookup as also-seller via the plan cache.
#[test]
fn two_sides_same_role_is_aggregate_error() {
    let manifest = load_two_entity_manifest();
    let sa_je_vec = sa_jes(&manifest);
    assert!(!sa_je_vec.is_empty(), "fixture sanity");

    let first_seller_je = sa_je_vec
        .iter()
        .find(|je| {
            // Find the SA seller-side JE (some relationships might have
            // SA on the buyer side via patterns).
            je.header.ic_pair_id.is_some_and(|pid| {
                derive_ic_pair_plans(&manifest, "ACME_SA")
                    .iter()
                    .find(|p| p.pair_id == pid)
                    .map(|p| p.role == IcRole::Seller)
                    .unwrap_or(false)
            })
        })
        .expect("at least one SA seller-side JE in the trimmed fixture")
        .clone();

    // Now duplicate this exact JE under another ACME_SA shard (i.e.
    // simulate a duplicate-seller corruption).  We pass it twice via the
    // SAME entity_code "ACME_SA" so the plan-cache resolves both as
    // Seller for the same pair_id.
    let result = match_ic_pairs(
        &manifest,
        &[(
            "ACME_SA".to_string(),
            vec![first_seller_je.clone(), first_seller_je],
        )],
    );

    // 2 sides observed, both Seller → must error.  (Three+ would also
    // error via the n-arm of the match; here we hit the same-role arm.)
    let err = result.expect_err("must reject corrupt input");
    assert!(
        matches!(err, GroupError::Aggregate(_)),
        "expected GroupError::Aggregate, got {err:?}"
    );
}

// ── v5.3 fuzzy matching ────────────────────────────────────────────────────

/// Scale every line's debit / credit on `je` by `factor`, in place.
/// Used by the fuzzy-mode tests below to inject amount drift.
fn scale_je_amounts(je: &mut JournalEntry, factor: Decimal) {
    for line in je.lines.iter_mut() {
        line.debit_amount *= factor;
        line.credit_amount *= factor;
    }
}

#[test]
fn manifest_driven_strategy_ignores_amount_drift() {
    // v5.0–v5.2 default: even when buyer amounts drift, manifest-driven
    // matching does NOT compare amounts and treats both sides as
    // matched.  This pins backwards compatibility — if the drift
    // check ever sneaks into the default path it'd break every v5.0
    // archive that happens to round its amounts slightly differently.
    let manifest = load_two_entity_manifest();
    let total = expected_pair_count(&manifest);
    assert!(total >= 1, "fixture sanity");

    let sa = sa_jes(&manifest);
    let mut usa = usa_jes(&manifest);
    // Multiply EVERY USA line by 1.5 — way above any sane tolerance.
    for je in usa.iter_mut() {
        scale_je_amounts(je, Decimal::new(15, 1)); // 1.5
    }

    // Default manifest = ManifestDriven, tolerance = 0.
    let result = match_ic_pairs(
        &manifest,
        &[("ACME_SA".to_string(), sa), ("ACME_USA".to_string(), usa)],
    )
    .expect("match");

    assert_eq!(
        result.matched.len(),
        total,
        "ManifestDriven must match regardless of amount drift"
    );
    assert!(
        result.unmatched.is_empty(),
        "ManifestDriven must NOT emit AmountDriftAboveTolerance"
    );
}

#[test]
fn emergent_fuzzy_strategy_flags_drift_above_tolerance() {
    // v5.3 fuzzy mode: when drift exceeds `tolerance_percent`, both
    // sides land in `unmatched` with `AmountDriftAboveTolerance`.
    use datasynth_group::config::IcMatchingStrategy;

    let mut manifest = load_two_entity_manifest();
    manifest.matching.strategy = IcMatchingStrategy::EmergentFuzzy;
    manifest.matching.tolerance_percent = Decimal::new(5, 3); // 0.005 = 50 bps

    let total = expected_pair_count(&manifest);
    assert!(total >= 1, "fixture sanity");

    let sa = sa_jes(&manifest);
    let mut usa = usa_jes(&manifest);
    // 10% drift on USA — well above the 50 bps tolerance.
    for je in usa.iter_mut() {
        scale_je_amounts(je, Decimal::new(110, 2)); // 1.10
    }

    let result = match_ic_pairs(
        &manifest,
        &[("ACME_SA".to_string(), sa), ("ACME_USA".to_string(), usa)],
    )
    .expect("match");

    assert_eq!(
        result.matched.len(),
        0,
        "every pair must be unmatched under fuzzy mode with 10% drift"
    );
    assert_eq!(
        result.unmatched.len(),
        total * 2,
        "each fuzzy-rejected pair contributes both sides to unmatched"
    );
    for u in &result.unmatched {
        assert_eq!(
            u.reason,
            UnmatchedReason::AmountDriftAboveTolerance,
            "fuzzy-rejected sides must report AmountDriftAboveTolerance"
        );
    }
}

#[test]
fn emergent_fuzzy_strategy_accepts_drift_within_tolerance() {
    // v5.3 fuzzy mode: small drift inside the tolerance still matches.
    // Pin this to make sure we're testing the comparison itself, not
    // an off-by-one on the predicate sign.
    use datasynth_group::config::IcMatchingStrategy;

    let mut manifest = load_two_entity_manifest();
    manifest.matching.strategy = IcMatchingStrategy::EmergentFuzzy;
    manifest.matching.tolerance_percent = Decimal::new(5, 2); // 5%

    let total = expected_pair_count(&manifest);
    assert!(total >= 1, "fixture sanity");

    let sa = sa_jes(&manifest);
    let mut usa = usa_jes(&manifest);
    // 1% drift — below the 5% tolerance.  Must match cleanly.
    for je in usa.iter_mut() {
        scale_je_amounts(je, Decimal::new(101, 2)); // 1.01
    }

    let result = match_ic_pairs(
        &manifest,
        &[("ACME_SA".to_string(), sa), ("ACME_USA".to_string(), usa)],
    )
    .expect("match");

    assert_eq!(
        result.matched.len(),
        total,
        "drift within tolerance must still match"
    );
    assert!(
        result.unmatched.is_empty(),
        "no AmountDriftAboveTolerance expected when drift ≤ tolerance"
    );
}

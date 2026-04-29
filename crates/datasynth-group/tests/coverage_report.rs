//! Task 5.7 — IC matching coverage report integration tests.
//!
//! Mirrors the fixture pattern of `tests/ic_matcher.rs` and
//! `tests/elimination_to_je.rs`: build a trimmed two-entity manifest
//! from `mini_nestle.yaml`, run the matcher end-to-end, then summarise
//! into a [`CoverageReport`] and exercise the on-disk writer.
//!
//! # Test taxonomy
//!
//! 1. **Happy path (full coverage)** — every plan matched, sample empty.
//! 2. **Partial match (missing buyer side)** — drop USA's JEs, every SA
//!    seller-side plan reports `MissingBuyerSide`.
//! 3. **Empty manifest** — no IC relationships → `total_pairs_planned =
//!    0`, `coverage = 0.0`, sample empty.
//! 4. **Sample cap at 100** — synthesise an `IcMatchResult` directly
//!    with 150 unmatched sides and assert the cap.
//! 5. **Writer writes to the spec'd path** — and round-trips equal to
//!    the in-memory report.
//! 6. **Writer returns the path** — and the path equals the actual
//!    file location.
//! 7. **Deterministic on-disk output** — two writes produce
//!    byte-identical files.

use std::fs;

use chrono::NaiveDate;

use datasynth_core::models::journal_entry::JournalEntryHeader;
use datasynth_core::models::{IcPairId, JournalEntry};
use datasynth_group::aggregate::ic_matcher::{IcMatchResult, IcMatchedPair, UnmatchedSide};
use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::shard::{
    derive_ic_pair_plans, inject_ic_journal_entries, IcRole, InjectionCtx,
};
use datasynth_group::{
    build_coverage_report, build_manifest, match_ic_pairs, write_coverage_report, GroupConfig,
    IcRelationshipConfig, UnmatchedReason, COVERAGE_REPORT_FILENAME, COVERAGE_REPORT_SUBDIR,
    UNMATCHED_SAMPLE_CAP,
};

// ── Fixture helpers ───────────────────────────────────────────────────────────

/// Mirror of `tests/ic_matcher.rs::load_two_entity_manifest`.  Keeps
/// the fixture in lockstep so a change to the canonical trim flows
/// through both files.
fn load_two_entity_manifest() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let mut cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse into GroupConfig");

    cfg.ownership
        .entities
        .retain(|e| e.code == "NESTLE_SA" || e.code == "NESTLE_USA");
    cfg.intercompany.relationships.retain(|rel| match rel {
        IcRelationshipConfig::Explicit(e) => {
            (e.seller == "NESTLE_SA" || e.seller == "NESTLE_USA")
                && (e.buyer == "NESTLE_SA" || e.buyer == "NESTLE_USA")
        }
        IcRelationshipConfig::Pattern(_) => true,
    });
    if let Some(p2) = cfg.tax.pillar_two.as_mut() {
        p2.jurisdictions.retain(|j| j == "CH" || j == "US");
    }
    if let Some(tp) = cfg.tax.transfer_pricing.as_mut() {
        tp.local_files_for.retain(|j| j == "CH" || j == "US");
    }
    build_manifest(&cfg).expect("trimmed mini_nestle must still build a manifest")
}

/// Build the no-IC variant: clears `intercompany.relationships`.  Used
/// by the empty-manifest coverage test.
fn load_two_entity_manifest_no_ic() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let mut cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse into GroupConfig");
    cfg.ownership
        .entities
        .retain(|e| e.code == "NESTLE_SA" || e.code == "NESTLE_USA");
    cfg.intercompany.relationships.clear();
    if let Some(p2) = cfg.tax.pillar_two.as_mut() {
        p2.jurisdictions.retain(|j| j == "CH" || j == "US");
    }
    if let Some(tp) = cfg.tax.transfer_pricing.as_mut() {
        tp.local_files_for.retain(|j| j == "CH" || j == "US");
    }
    build_manifest(&cfg).expect("manifest with no IC relationships must still build")
}

fn sa_jes(manifest: &GroupManifest) -> Vec<JournalEntry> {
    let plans = derive_ic_pair_plans(manifest, "NESTLE_SA");
    inject_ic_journal_entries(
        &plans,
        &InjectionCtx {
            entity_code: "NESTLE_SA".to_string(),
        },
    )
}

fn usa_jes(manifest: &GroupManifest) -> Vec<JournalEntry> {
    let plans = derive_ic_pair_plans(manifest, "NESTLE_USA");
    inject_ic_journal_entries(
        &plans,
        &InjectionCtx {
            entity_code: "NESTLE_USA".to_string(),
        },
    )
}

/// Hand-rolled JE with no `ic_pair_id` set — used by the >100 unmatched
/// fixture builder.
fn dummy_je(entity: &str) -> JournalEntry {
    let header = JournalEntryHeader::new(
        entity.to_string(),
        NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
    );
    JournalEntry::new(header)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// 1. Happy path: every plan matched → coverage = 1.0, empty sample,
///    every reason key present at zero.
#[test]
fn happy_path_full_coverage_zero_unmatched() {
    let manifest = load_two_entity_manifest();
    let result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), sa_jes(&manifest)),
            ("NESTLE_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");

    let report = build_coverage_report(&result);

    assert!(
        (report.coverage - 1.0).abs() < f64::EPSILON,
        "happy path: coverage = 1.0; got {}",
        report.coverage
    );
    assert!(report.matched > 0, "happy path: at least one match");
    assert_eq!(report.matched, result.matched.len());
    assert_eq!(report.total_pairs_planned, result.total_planned);
    assert!(
        report.unmatched_sample.is_empty(),
        "happy path: empty unmatched sample"
    );
    // Schema-stability: all three reasons present at zero.
    assert_eq!(report.unmatched_by_reason.len(), 3);
    for r in [
        UnmatchedReason::MissingBuyerSide,
        UnmatchedReason::MissingSellerSide,
        UnmatchedReason::AmountDriftAboveTolerance,
    ] {
        assert_eq!(
            report.unmatched_by_reason.get(&r),
            Some(&0),
            "reason {r:?} must be present at 0"
        );
    }
}

/// 2. Partial match: drop USA's JEs.  Every SA-seller-side plan reports
///    `MissingBuyerSide`.  Coverage drops below 1.0, sample is
///    non-empty, the relevant reason is incremented and others stay
///    at zero (modulo SA-as-buyer plans which become
///    `MissingSellerSide` — see ic_matcher.rs::missing_buyer_side_*).
#[test]
fn partial_match_missing_buyer_side() {
    let manifest = load_two_entity_manifest();
    let sa_plans = derive_ic_pair_plans(&manifest, "NESTLE_SA");
    let sa_seller_count = sa_plans.iter().filter(|p| p.role == IcRole::Seller).count();
    assert!(
        sa_seller_count >= 1,
        "fixture sanity: need at least one SA seller plan"
    );

    let result =
        match_ic_pairs(&manifest, &[("NESTLE_SA".to_string(), sa_jes(&manifest))]).expect("match");
    let report = build_coverage_report(&result);

    assert!(
        report.coverage < 1.0,
        "partial: coverage < 1.0; got {}",
        report.coverage
    );
    assert!(
        !report.unmatched_sample.is_empty(),
        "partial: sample must be populated"
    );
    assert!(
        report
            .unmatched_by_reason
            .get(&UnmatchedReason::MissingBuyerSide)
            .copied()
            .unwrap_or(0)
            >= sa_seller_count,
        "MissingBuyerSide must reflect every SA-seller-side plan",
    );
    // AmountDriftAboveTolerance is the v5.3 placeholder — must be 0
    // in v5.0 because the matcher never emits it.
    assert_eq!(
        report
            .unmatched_by_reason
            .get(&UnmatchedReason::AmountDriftAboveTolerance),
        Some(&0),
        "v5.0 must never emit AmountDriftAboveTolerance"
    );
}

/// 3. Empty manifest (no IC relationships).  `total_pairs_planned = 0`,
///    `matched = 0`, `coverage = 0.0`, sample empty, all reasons zero.
#[test]
fn empty_manifest_zero_coverage() {
    let manifest = load_two_entity_manifest_no_ic();
    assert!(manifest.ic_relationships.is_empty(), "fixture sanity");

    let result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), Vec::new()),
            ("NESTLE_USA".to_string(), Vec::new()),
        ],
    )
    .expect("match");
    let report = build_coverage_report(&result);

    assert_eq!(report.total_pairs_planned, 0);
    assert_eq!(report.matched, 0);
    assert_eq!(report.coverage, 0.0);
    assert!(report.unmatched_sample.is_empty());
    assert_eq!(report.unmatched_by_reason.len(), 3);
    for r in [
        UnmatchedReason::MissingBuyerSide,
        UnmatchedReason::MissingSellerSide,
        UnmatchedReason::AmountDriftAboveTolerance,
    ] {
        assert_eq!(report.unmatched_by_reason.get(&r), Some(&0));
    }
}

/// 4. Sample cap at 100.
///
/// Synthesise an `IcMatchResult` directly with 150 unmatched sides
/// (each carrying a distinct `pair_id`) and assert the writer caps the
/// sample at exactly 100.  The cap is the spec's "[:100]" semantics —
/// see `coverage_report.rs::UNMATCHED_SAMPLE_CAP`.
#[test]
fn unmatched_sample_caps_at_100() {
    let je = dummy_je("ENT");
    let unmatched: Vec<UnmatchedSide> = (0..150u8)
        .map(|i| UnmatchedSide {
            pair_id: IcPairId::from_bytes([i; 32]),
            present_role: IcRole::Seller,
            present_entity: "ENT".to_string(),
            present_je: je.clone(),
            reason: UnmatchedReason::MissingBuyerSide,
        })
        .collect();

    let result = IcMatchResult {
        matched: Vec::new(),
        unmatched,
        total_planned: 150,
        coverage: 0.0,
    };

    let report = build_coverage_report(&result);
    assert_eq!(
        report.unmatched_sample.len(),
        UNMATCHED_SAMPLE_CAP,
        "sample must cap at {UNMATCHED_SAMPLE_CAP} per spec §5.4"
    );
    assert_eq!(
        UNMATCHED_SAMPLE_CAP, 100,
        "spec §5.4 mandates first-100 cap"
    );
    // Histogram still reflects the full 150, not just the sample.
    assert_eq!(
        report
            .unmatched_by_reason
            .get(&UnmatchedReason::MissingBuyerSide),
        Some(&150),
        "histogram counts every unmatched side, not just the sample"
    );
}

/// 5. `write_coverage_report` writes to the spec'd path and the file
///    round-trips equal to the in-memory report.
#[test]
fn writer_emits_at_spec_path_and_roundtrips() {
    let manifest = load_two_entity_manifest();
    let result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), sa_jes(&manifest)),
            ("NESTLE_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");
    let report = build_coverage_report(&result);

    let tmp = tempfile::tempdir().expect("tempdir");
    let written = write_coverage_report(&report, tmp.path()).expect("write");

    let expected = tmp
        .path()
        .join(COVERAGE_REPORT_SUBDIR)
        .join(COVERAGE_REPORT_FILENAME);
    assert_eq!(
        written, expected,
        "writer must return the actual path it wrote to"
    );
    assert!(expected.exists(), "file must exist on disk");

    // Round-trip via serde_json::Value to avoid requiring PartialEq on
    // JournalEntry (which it doesn't derive — IcMatchedPair /
    // UnmatchedSide both embed it).  Comparing the JSON values is
    // semantically equivalent for our purposes.
    let bytes = fs::read(&expected).expect("read");
    let on_disk: serde_json::Value = serde_json::from_slice(&bytes).expect("parse");
    let in_memory: serde_json::Value =
        serde_json::to_value(&report).expect("serialise in-memory report");
    assert_eq!(
        on_disk, in_memory,
        "on-disk JSON must round-trip equal to the in-memory report"
    );
}

/// 6. Path returned by `write_coverage_report` matches the actual
///    location on disk.
///
/// Equivalent to test 5's path assertion but stated as a stand-alone
/// contract so a regression in the return value (e.g. returning the
/// out_dir instead of the file path) fails this test specifically.
#[test]
fn writer_returns_the_written_path() {
    let report = build_coverage_report(&IcMatchResult {
        matched: Vec::new(),
        unmatched: Vec::new(),
        total_planned: 0,
        coverage: 0.0,
    });
    let tmp = tempfile::tempdir().expect("tempdir");
    let returned = write_coverage_report(&report, tmp.path()).expect("write");

    assert_eq!(
        returned.file_name().and_then(|n| n.to_str()),
        Some(COVERAGE_REPORT_FILENAME),
    );
    assert_eq!(
        returned
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str()),
        Some(COVERAGE_REPORT_SUBDIR),
        "returned path's parent dir must be `{COVERAGE_REPORT_SUBDIR}`"
    );
    assert!(
        returned.is_file(),
        "returned path must point at a real file"
    );
}

/// 7. Two calls to `write_coverage_report` produce byte-identical
///    files (modulo file timestamps, which we don't compare).
#[test]
fn deterministic_on_disk_output() {
    let je = dummy_je("ENT");
    // Use a non-trivial fixture so the test catches reordering bugs.
    let unmatched: Vec<UnmatchedSide> = (0..5u8)
        .map(|i| UnmatchedSide {
            pair_id: IcPairId::from_bytes([i; 32]),
            present_role: if i % 2 == 0 {
                IcRole::Seller
            } else {
                IcRole::Buyer
            },
            present_entity: format!("E{i}"),
            present_je: je.clone(),
            reason: if i < 3 {
                UnmatchedReason::MissingBuyerSide
            } else {
                UnmatchedReason::MissingSellerSide
            },
        })
        .collect();
    let pair = IcMatchedPair {
        pair_id: IcPairId::from_bytes([99; 32]),
        seller_entity: "S".to_string(),
        buyer_entity: "B".to_string(),
        seller_je: je.clone(),
        buyer_je: je,
    };
    let result = IcMatchResult {
        matched: vec![pair],
        unmatched,
        total_planned: 6,
        coverage: 1.0 / 6.0,
    };

    let report = build_coverage_report(&result);

    let tmp_a = tempfile::tempdir().expect("tempdir a");
    let tmp_b = tempfile::tempdir().expect("tempdir b");
    let path_a = write_coverage_report(&report, tmp_a.path()).expect("write a");
    let path_b = write_coverage_report(&report, tmp_b.path()).expect("write b");

    let bytes_a = fs::read(&path_a).expect("read a");
    let bytes_b = fs::read(&path_b).expect("read b");
    assert_eq!(
        bytes_a, bytes_b,
        "two writes of the same report must produce byte-identical files"
    );
}

//! Task 5.5 — Elimination → JournalEntry conversion smoke test.
//!
//! Verifies that
//! [`datasynth_group::aggregate::eliminations_to_journal_entries`]
//! preserves every elimination entry's balance and stamps the v5.0
//! consolidation contract on each emitted JE header:
//!
//! - `is_elimination = true`
//! - `document_type = "ELIMINATION"`
//! - `created_by = "CONSOLIDATION"` (the spec's "source = CONSOLIDATION"
//!   — the typed `source` field is `TransactionSource::Automated`,
//!   per the v1.3.0 helper contract)
//!
//! The wrapper's behaviour comes from
//! [`datasynth_generators::elimination_to_journal_entries`] — these tests
//! cover the v5.0 wiring (ensuring the wrapper is correctly hooked up
//! and that an `EliminationResult` round-trips as expected) rather than
//! re-testing the underlying converter, which has its own unit tests in
//! `datasynth-generators`.

use datasynth_core::models::TransactionSource;

use datasynth_group::aggregate::{
    eliminations_to_journal_entries, generate_eliminations, match_ic_pairs,
};
use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::shard::{derive_ic_pair_plans, inject_ic_journal_entries, InjectionCtx};
use datasynth_group::{build_manifest, GroupConfig, IcRelationshipConfig};

// ── Fixture helpers ───────────────────────────────────────────────────────────

/// Mirror of `tests/elimination.rs::load_two_entity_manifest` — keep
/// the test fixture in lockstep so any change to the canonical trim
/// flows through both files.
fn load_two_entity_manifest() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_acme.yaml");
    let mut cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_acme.yaml must parse into GroupConfig");

    cfg.ownership
        .entities
        .retain(|e| matches!(e.code.as_str(), "ACME_SA" | "ACME_USA"));

    cfg.intercompany.relationships.retain(|r| match r {
        IcRelationshipConfig::Explicit(e) => e.seller == "ACME_SA" && e.buyer == "ACME_USA",
        IcRelationshipConfig::Pattern(_) => false,
    });
    assert_eq!(
        cfg.intercompany.relationships.len(),
        1,
        "trim must leave exactly one explicit ACME_SA→ACME_USA relationship",
    );

    if let Some(p2) = cfg.tax.pillar_two.as_mut() {
        p2.jurisdictions
            .retain(|j| matches!(j.as_str(), "CH" | "US"));
    }
    if let Some(tp) = cfg.tax.transfer_pricing.as_mut() {
        tp.local_files_for
            .retain(|j| matches!(j.as_str(), "CH" | "US"));
    }

    build_manifest(&cfg).expect("trimmed mini_acme must still build a manifest")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn every_emitted_je_carries_the_consolidation_contract() {
    let manifest = load_two_entity_manifest();

    let sa_plans = derive_ic_pair_plans(&manifest, "ACME_SA");
    let usa_plans = derive_ic_pair_plans(&manifest, "ACME_USA");
    let sa_jes = inject_ic_journal_entries(
        &sa_plans,
        &InjectionCtx {
            entity_code: "ACME_SA".to_string(),
        },
    );
    let usa_jes = inject_ic_journal_entries(
        &usa_plans,
        &InjectionCtx {
            entity_code: "ACME_USA".to_string(),
        },
    );

    let match_result = match_ic_pairs(
        &manifest,
        &[
            ("ACME_SA".to_string(), sa_jes),
            ("ACME_USA".to_string(), usa_jes),
        ],
    )
    .expect("match");

    let elim =
        generate_eliminations(&match_result.matched, &manifest).expect("generate_eliminations");
    let jes = eliminations_to_journal_entries(&elim);

    assert!(
        !jes.is_empty(),
        "matched pairs must produce at least one elimination JE"
    );
    assert_eq!(
        jes.len(),
        elim.entries.len(),
        "every balanced elimination entry must convert to exactly one JE"
    );

    for je in &jes {
        assert!(
            je.header.is_elimination,
            "elimination JE must carry is_elimination = true"
        );
        assert_eq!(
            je.header.document_type, "ELIMINATION",
            "elimination JE must carry document_type = \"ELIMINATION\""
        );
        assert_eq!(
            je.header.created_by, "CONSOLIDATION",
            "elimination JE must carry created_by = \"CONSOLIDATION\" \
             (spec's source = CONSOLIDATION lands in created_by since \
             header.source is the typed TransactionSource enum)"
        );
        assert_eq!(
            je.header.source,
            TransactionSource::Automated,
            "elimination JE source must be TransactionSource::Automated \
             (the v1.3.0 helper contract)"
        );
        assert!(
            je.is_balanced(),
            "every emitted elimination JE must balance"
        );
    }
}

#[test]
fn empty_result_produces_zero_journal_entries() {
    // Build an empty `EliminationResult` directly — `match_ic_pairs` on
    // an empty input would also work but takes a roundabout route.
    let manifest = load_two_entity_manifest();
    let empty_match = match_ic_pairs(&manifest, &[]).expect("match empty");
    let elim = generate_eliminations(&empty_match.matched, &manifest)
        .expect("generate_eliminations on empty matched");

    let jes = eliminations_to_journal_entries(&elim);
    assert!(
        jes.is_empty(),
        "empty result must produce no journal entries; got {}",
        jes.len()
    );
}

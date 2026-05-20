//! Task 4.1 — `build_shard_context` integration tests.
//!
//! These tests verify that the opaque `ShardContext` assembled by
//! `datasynth_group::shard::build_shard_context` matches the manifest's
//! per-entity seed and carries the correct set of IC journal entries for
//! the requested entity.
//!
//! Full-struct equality across `extra_journal_entries` does not hold —
//! `JournalEntryHeader::new` stamps `document_id` with `Uuid::now_v7()`
//! which is time-based.  The deterministic surface the shard runner relies
//! on (and which these tests assert) is the set of IC-specific header
//! fields, account numbers, and line amounts.

use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::shard::build_shard_context;
use datasynth_group::{build_manifest, GroupConfig};

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn load_mini_acme_manifest() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_acme.yaml");
    let cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_acme.yaml must parse into GroupConfig");
    build_manifest(&cfg).expect("mini_acme.yaml must build a manifest")
}

/// Mirror of `ic_plan.rs::load_manifest_without_ic` — build the mini_acme
/// manifest with every IC relationship stripped so `derive_ic_pair_plans`
/// is guaranteed to return an empty list for every entity.
fn load_manifest_without_ic() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_acme.yaml");
    let mut cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_acme.yaml must parse into GroupConfig");
    cfg.intercompany.relationships.clear();
    build_manifest(&cfg).expect("mutated mini_acme must still build a manifest")
}

/// Look up an entity's hex `entity_seed` from the manifest and decode it
/// back to `[u8; 32]` so tests can compare against the context's decoded
/// bytes directly.
fn expected_entity_seed(manifest: &GroupManifest, entity_code: &str) -> [u8; 32] {
    let hex_seed = manifest
        .ownership_graph
        .entities
        .iter()
        .find(|e| e.code == entity_code)
        .map(|e| e.entity_seed.clone())
        .unwrap_or_else(|| panic!("fixture must contain entity {entity_code}"));
    hex::decode(&hex_seed)
        .expect("manifest entity_seed must be hex")
        .try_into()
        .expect("manifest entity_seed must be 32 bytes")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Happy path: the context for a known entity carries the correct
/// `entity_code` and the per-entity seed decoded from the manifest.
#[test]
fn test_context_for_known_entity_has_correct_code_and_seed() {
    let manifest = load_mini_acme_manifest();
    let ctx =
        build_shard_context(&manifest, "ACME_SA").expect("ACME_SA is in the mini_acme fixture");

    assert_eq!(ctx.entity_code, "ACME_SA");
    assert_eq!(ctx.entity_seed, expected_entity_seed(&manifest, "ACME_SA"));
}

/// Error path: an unknown entity code produces a `GroupError::Config` whose
/// message names the bad code so caller-side logs pinpoint the typo.
#[test]
fn test_context_for_unknown_entity_errors() {
    let manifest = load_mini_acme_manifest();
    let err = build_shard_context(&manifest, "NOT_REAL")
        .expect_err("unknown entity must produce an error");
    let msg = err.to_string();
    assert!(
        msg.contains("NOT_REAL"),
        "error message must mention the bad entity code; got: {msg}"
    );
}

/// ACME_SA is the common seller in the mini_acme fixture — it
/// participates in both explicit relationships *and* the
/// `buyer_scoping_profile: any` pattern.  It must therefore have at least
/// one IC JE in the context, every IC JE must be tagged with the pair /
/// partner metadata, post against its own company code, and balance.
#[test]
fn test_context_includes_ic_journal_entries_for_seller() {
    let manifest = load_mini_acme_manifest();
    let ctx =
        build_shard_context(&manifest, "ACME_SA").expect("ACME_SA is in the mini_acme fixture");

    assert!(
        !ctx.extra_journal_entries.is_empty(),
        "ACME_SA is a common seller in the mini_acme fixture — \
         must produce at least one IC JE; got 0"
    );

    for (i, je) in ctx.extra_journal_entries.iter().enumerate() {
        assert!(
            je.header.ic_pair_id.is_some(),
            "JE at position {i} must carry an ic_pair_id"
        );
        assert!(
            je.header.ic_partner_entity.is_some(),
            "JE at position {i} must carry an ic_partner_entity"
        );
        assert_eq!(
            je.header.company_code, "ACME_SA",
            "JE at position {i} must post against ACME_SA"
        );
        assert!(
            je.is_balanced(),
            "JE at position {i} must balance; got dr={} cr={}",
            je.total_debit(),
            je.total_credit()
        );
    }
}

/// With every IC relationship stripped from the manifest, no entity —
/// seller, buyer, or non-participant — can produce any `extra_journal_entries`.
#[test]
fn test_context_for_non_participant_has_no_extra_jes() {
    let manifest = load_manifest_without_ic();
    // Sanity: the fixture wiring really did strip the relationships.
    assert!(
        manifest.ic_relationships.is_empty(),
        "fixture wiring: manifest.ic_relationships must be empty for this test"
    );

    for entity_code in ["ACME_SA", "ACME_USA", "ACME_DE", "ACME_BR", "ACME_JV"] {
        let ctx = build_shard_context(&manifest, entity_code)
            .unwrap_or_else(|e| panic!("{entity_code} must build successfully: {e}"));
        assert!(
            ctx.extra_journal_entries.is_empty(),
            "{entity_code} with no IC relationships must have no extra JEs; got {}",
            ctx.extra_journal_entries.len()
        );
        // And the seed / code must still be correct even in the no-IC case.
        assert_eq!(ctx.entity_code, entity_code);
        assert_eq!(
            ctx.entity_seed,
            expected_entity_seed(&manifest, entity_code)
        );
    }
}

/// Calling `build_shard_context` twice with the same arguments must
/// produce contexts with byte-identical `entity_code`, `entity_seed`, and
/// an equal number of `extra_journal_entries`, each matching on the
/// deterministic surface (IC header fields + line accounts / amounts).
#[test]
fn test_deterministic_across_calls() {
    let manifest = load_mini_acme_manifest();
    let a = build_shard_context(&manifest, "ACME_SA").expect("first call must succeed");
    let b = build_shard_context(&manifest, "ACME_SA").expect("second call must succeed");

    // Scalar deterministic surface.
    assert_eq!(a.entity_code, b.entity_code);
    assert_eq!(a.entity_seed, b.entity_seed);
    assert_eq!(
        a.extra_journal_entries.len(),
        b.extra_journal_entries.len(),
        "non-deterministic JE count across calls — this would break the shard contract"
    );

    // Per-JE deterministic surface — exclude `document_id` (Uuid::now_v7)
    // and lines' `journal_id` (seeded off the header doc_id).
    for (i, (ja, jb)) in a
        .extra_journal_entries
        .iter()
        .zip(b.extra_journal_entries.iter())
        .enumerate()
    {
        assert_eq!(
            ja.header.ic_pair_id, jb.header.ic_pair_id,
            "ic_pair_id mismatch at JE {i}"
        );
        assert_eq!(
            ja.header.ic_partner_entity, jb.header.ic_partner_entity,
            "ic_partner_entity mismatch at JE {i}"
        );
        assert_eq!(
            ja.header.company_code, jb.header.company_code,
            "company_code mismatch at JE {i}"
        );
        assert_eq!(
            ja.header.posting_date, jb.header.posting_date,
            "posting_date mismatch at JE {i}"
        );
        assert_eq!(
            ja.header.header_text, jb.header.header_text,
            "header_text mismatch at JE {i}"
        );

        assert_eq!(
            ja.lines.len(),
            jb.lines.len(),
            "line count mismatch at JE {i}"
        );
        for (la, lb) in ja.lines.iter().zip(jb.lines.iter()) {
            assert_eq!(la.gl_account, lb.gl_account);
            assert_eq!(la.debit_amount, lb.debit_amount);
            assert_eq!(la.credit_amount, lb.credit_amount);
        }
    }
}

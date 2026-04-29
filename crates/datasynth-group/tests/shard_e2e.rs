//! Task 4.5 — multi-entity shard end-to-end smoke test with IC mirror checks.
//!
//! Drives [`datasynth_group::shard::run_shard`] with a manifest scoped down
//! to **two** entities — `NESTLE_SA` (seller) and `NESTLE_USA` (buyer) —
//! linked by a single explicit `goods_sale` IC relationship.  This is the
//! smallest configuration that lets us assert the v5.0 shard runner's
//! crown-jewel invariant: every IC pair the runner emits is **mirrored**
//! across the two entity outputs (matching `ic_pair_id`, swapped
//! `ic_partner_entity`).
//!
//! # Why scope the manifest down rather than load a separate fixture?
//!
//! Same reasoning as `tests/shard_runner.rs::load_single_entity_manifest`:
//! we mutate the parsed [`GroupConfig`] (rather than maintain a separate
//! two-entity fixture) so the test stays in lockstep with `mini_nestle.yaml`.
//! Every change to the canonical fixture flows through here automatically.
//!
//! The trim retains exactly:
//! - Two ownership entities: `NESTLE_SA` (parent, CH) and `NESTLE_USA` (US).
//! - One IC relationship (the explicit `SA → USA` entry, `types:
//!   [goods_sale, royalty]`) — the second explicit pair (SA → DE) and the
//!   trailing pattern entry are dropped along with the entities they
//!   reference.
//! - Tax jurisdictions narrowed to `CH` + `US` so `pillar_two` and
//!   `transfer_pricing` validate against only the entities we kept.
//!
//! # Why `#[ignore]`?
//!
//! Each [`datasynth_runtime::EnhancedOrchestrator`] run on a `mini_nestle`
//! entity peaks at ~17 GiB RSS and takes 15+ minutes (see
//! `tests/shard_runner.rs` for the same caveat).  This test sequences
//! **two** orchestrator runs back-to-back inside one process — RSS
//! recovers between runs, but combined wallclock approaches half an hour
//! and any GC stragglers from run #1 inflate run #2's headroom.  The XXL
//! Azure VM verification harness
//! (`docs/superpowers/plans/2026-04-23-group-audit-v5.0-xxl-verification.md`
//! §3.2.b) is the canonical place to run this.  Run it locally with
//! `cargo test … -- --ignored` only on a host with ≥64 GiB free RAM.

use std::collections::BTreeMap;
use std::fs;

use tempfile::TempDir;

use datasynth_core::models::{IcPairId, JournalEntry};
use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::shard::{run_shard, ShardSummary};
use datasynth_group::{build_manifest, GroupConfig};

// ── Fixture helpers ───────────────────────────────────────────────────────────

/// Load the full `mini_nestle` fixture and trim it down to a two-entity
/// IC pair (`NESTLE_SA` ↔ `NESTLE_USA`).
///
/// See the module docs for the trim rationale.  After mutation the
/// manifest builder must still accept the config — the `expect` on
/// [`build_manifest`] catches any drift in fixture invariants the next
/// time `mini_nestle.yaml` changes.
fn load_two_entity_manifest() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let mut cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse into GroupConfig");

    // Retain NESTLE_SA + NESTLE_USA — drop NESTLE_DE, NESTLE_BR, NESTLE_JV
    // so the resulting shard plan puts exactly these two entities into
    // their respective shards (both share the `significant` profile, so
    // the shard packer batches them into one shard ≤ 10 B-row cap).
    cfg.ownership
        .entities
        .retain(|e| matches!(e.code.as_str(), "NESTLE_SA" | "NESTLE_USA"));

    // The fixture has three IC entries: explicit SA→USA, explicit SA→DE,
    // and a pattern targeting any `significant`-profile buyer.  We need
    // only the first one — truncating drops the SA→DE explicit entry
    // (refs dropped NESTLE_DE) and the pattern entry (which would
    // otherwise expand against the trimmed entity list and produce an
    // unwanted SA→USA management_fee leg).
    cfg.intercompany.relationships.truncate(1);

    // The fixture's `tax.pillar_two.jurisdictions: [CH, DE, US]` references
    // entities we just dropped.  The manifest's tax-plan builder rejects
    // any jurisdiction that doesn't appear in some retained entity's
    // country, so we narrow the list to `CH` + `US`.  The same applies
    // to `transfer_pricing.local_files_for`.
    if let Some(p2) = cfg.tax.pillar_two.as_mut() {
        p2.jurisdictions
            .retain(|j| matches!(j.as_str(), "CH" | "US"));
    }
    if let Some(tp) = cfg.tax.transfer_pricing.as_mut() {
        tp.local_files_for
            .retain(|j| matches!(j.as_str(), "CH" | "US"));
    }

    build_manifest(&cfg).expect("trimmed mini_nestle must still build a manifest")
}

/// Read every JE the orchestrator emitted for `entity_code` from
/// `out_dir/entities/{entity_code}/journal_entries.json`.
///
/// The runner hard-codes `Nested` layout + `FileFormat::Json` in v5.0
/// (Task 10 plumbs flags through), so this path is stable for the test.
fn read_entity_journal_entries(out_dir: &std::path::Path, entity_code: &str) -> Vec<JournalEntry> {
    let path = out_dir
        .join("entities")
        .join(entity_code)
        .join("journal_entries.json");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read journal_entries.json for {entity_code}: {e}"));
    serde_json::from_str::<Vec<JournalEntry>>(&raw)
        .unwrap_or_else(|e| panic!("parse journal_entries.json for {entity_code}: {e}"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// End-to-end smoke: `run_shard` drives the orchestrator for two entities,
/// the per-entity `journal_entries.json` exists for both, every JE bearing
/// an `ic_pair_id` is **mirrored** across the two entities (same pair_id,
/// swapped `ic_partner_entity`), and the on-disk `shard_summary.json`
/// round-trips back to the in-memory summary.
///
/// See module-level rustdoc for the `#[ignore]` rationale.
#[test]
#[ignore = "drives 2× full EnhancedOrchestrator runs (~17 GiB peak RSS each, 30+ min total) — XXL VM only"]
fn run_shard_two_entities_mirrors_ic_pair_ids() {
    let manifest = load_two_entity_manifest();
    let tmp = TempDir::new().expect("tempdir");
    let out_dir = tmp.path();

    // Both retained entities must share a shard_id under the trimmed
    // manifest's `significant` profile.  Look it up from the manifest
    // (rather than hard-coding "S_SIG_0001") so the test survives shard-id
    // format refactors.  Asserting equality double-checks the shard
    // packer's batching invariant for this size config.
    let entities = &manifest.ownership_graph.entities;
    let nestle_sa = entities
        .iter()
        .find(|e| e.code == "NESTLE_SA")
        .expect("NESTLE_SA must remain after trimming");
    let nestle_usa = entities
        .iter()
        .find(|e| e.code == "NESTLE_USA")
        .expect("NESTLE_USA must remain after trimming");
    assert_eq!(
        nestle_sa.shard_id, nestle_usa.shard_id,
        "NESTLE_SA and NESTLE_USA must land in the same shard under the trimmed `significant` profile; \
         got SA={} vs USA={}",
        nestle_sa.shard_id, nestle_usa.shard_id,
    );
    let shard_id = nestle_sa.shard_id.clone();

    let summary = run_shard(&manifest, &shard_id, out_dir).expect("run_shard must succeed");

    // ── Top-level shape ───────────────────────────────────────────────────
    assert_eq!(summary.shard_id, shard_id);
    assert_eq!(
        summary.entity_summaries.len(),
        2,
        "trimmed fixture must produce exactly two entity summaries; got {}",
        summary.entity_summaries.len()
    );
    let summary_codes: Vec<&str> = summary
        .entity_summaries
        .iter()
        .map(|s| s.entity_code.as_str())
        .collect();
    for expected in ["NESTLE_SA", "NESTLE_USA"] {
        assert!(
            summary_codes.contains(&expected),
            "{expected} missing from entity_summaries; got {summary_codes:?}",
        );
    }

    // ── Per-entity JE files exist with non-zero counts ───────────────────
    for entity_summary in &summary.entity_summaries {
        let je_path = out_dir
            .join(&entity_summary.output_subdir)
            .join("journal_entries.json");
        assert!(
            je_path.is_file(),
            "journal_entries.json missing for {} at {}",
            entity_summary.entity_code,
            je_path.display()
        );
        assert!(
            entity_summary.journal_entry_count > 0,
            "{} produced zero JEs — orchestrator output regression",
            entity_summary.entity_code
        );
        // Each entity must own at least the IC legs it was assigned via
        // `ShardContext.extra_journal_entries` — both halves of every pair
        // are produced (one per entity).
        assert!(
            entity_summary.ic_journal_entry_count > 0,
            "{} produced zero IC JEs despite the explicit SA↔USA relationship; \
             ic_je_injector wiring may have regressed",
            entity_summary.entity_code,
        );
    }

    // ── IC pair_id mirroring ─────────────────────────────────────────────
    // Group every IC-tagged JE on both sides by `ic_pair_id`.  Each pair
    // must have **exactly two** sides — one from each entity — with the
    // counterparty (`ic_partner_entity`) referencing the *other* entity's
    // code, and a matching `posting_date`.  Use `BTreeMap` for
    // deterministic iteration order in any failure messages.
    let sa_jes = read_entity_journal_entries(out_dir, "NESTLE_SA");
    let usa_jes = read_entity_journal_entries(out_dir, "NESTLE_USA");

    type PairSides<'a> = (Vec<&'a JournalEntry>, Vec<&'a JournalEntry>);
    let mut by_pair: BTreeMap<IcPairId, PairSides> = BTreeMap::new();
    for je in &sa_jes {
        if let Some(pid) = je.header.ic_pair_id {
            by_pair.entry(pid).or_default().0.push(je);
        }
    }
    for je in &usa_jes {
        if let Some(pid) = je.header.ic_pair_id {
            by_pair.entry(pid).or_default().1.push(je);
        }
    }
    assert!(
        !by_pair.is_empty(),
        "no JEs with ic_pair_id were emitted on either side — \
         the trimmed manifest's explicit SA↔USA relationship should produce ≥1 pair"
    );

    for (pair_id, (sa_sides, usa_sides)) in &by_pair {
        assert_eq!(
            sa_sides.len(),
            1,
            "pair {pair_id} must have exactly one NESTLE_SA leg; got {}",
            sa_sides.len()
        );
        assert_eq!(
            usa_sides.len(),
            1,
            "pair {pair_id} must have exactly one NESTLE_USA leg; got {}",
            usa_sides.len()
        );

        let sa_je = sa_sides[0];
        let usa_je = usa_sides[0];

        assert_eq!(
            sa_je.header.ic_partner_entity.as_deref(),
            Some("NESTLE_USA"),
            "pair {pair_id}: NESTLE_SA leg's ic_partner_entity must reference NESTLE_USA"
        );
        assert_eq!(
            usa_je.header.ic_partner_entity.as_deref(),
            Some("NESTLE_SA"),
            "pair {pair_id}: NESTLE_USA leg's ic_partner_entity must reference NESTLE_SA"
        );

        assert_eq!(
            sa_je.header.posting_date, usa_je.header.posting_date,
            "pair {pair_id}: the two legs must share posting_date \
             (sa={}, usa={})",
            sa_je.header.posting_date, usa_je.header.posting_date,
        );

        // company_code on each leg is the leg's own entity — surfaced by
        // ic_je_injector::build_je_for_plan.  This guards against a
        // regression that swaps seller/buyer routing inside the runner.
        assert_eq!(
            sa_je.header.company_code, "NESTLE_SA",
            "pair {pair_id}: SA leg.company_code must be NESTLE_SA"
        );
        assert_eq!(
            usa_je.header.company_code, "NESTLE_USA",
            "pair {pair_id}: USA leg.company_code must be NESTLE_USA"
        );
    }

    // ── shard_summary.json round-trip ────────────────────────────────────
    let summary_path = out_dir.join("shard_summary.json");
    assert!(
        summary_path.is_file(),
        "shard_summary.json must be written at {}",
        summary_path.display()
    );
    let parsed: ShardSummary = serde_json::from_str(
        &fs::read_to_string(&summary_path).expect("shard_summary.json must be readable"),
    )
    .expect("shard_summary.json must round-trip back through serde_json");
    assert_eq!(
        parsed, summary,
        "round-tripped ShardSummary must equal the in-memory summary"
    );
}

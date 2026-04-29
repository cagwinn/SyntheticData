//! Task 4.3 — shard runner integration test (single entity).
//!
//! Drives [`datasynth_group::shard::run_shard`] with a manifest scoped down
//! to a single entity (NESTLE_SA) to keep host memory bounded — the
//! orchestrator's per-entity working set is on the order of hundreds of
//! megabytes, so multi-entity tests in this file would risk OOM on the CI
//! host.  Task 4.5 will add a multi-entity + IC-mirror test once the
//! orchestrator's memory profile is verified to fit two concurrent runs.
//!
//! # Why scope the manifest down rather than pick a single-entity shard?
//!
//! The full mini_nestle fixture's `significant` profile contains three
//! entities (NESTLE_SA, NESTLE_USA, NESTLE_DE), and the shard plan batches
//! every entity in a profile into a single shard while the row budget is
//! below the 10 B-row cap.  Calling `run_shard("S_SIG_0001")` against the
//! unmodified fixture would therefore drive the orchestrator three times
//! in one test process — a known OOM risk on the CI host.  We mutate the
//! parsed [`GroupConfig`] to retain only NESTLE_SA and rebuild the manifest
//! before testing so the resulting `S_SIG_0001` shard contains exactly one
//! entity.
//!
//! IC relationships and FX rate columns referencing the dropped entities
//! are stripped at the same time so the manifest builder doesn't reject
//! the trimmed config.

use std::fs;

use tempfile::TempDir;

use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::shard::{run_shard, ShardSummary};
use datasynth_group::{build_manifest, GroupConfig};

// ── Fixture helpers ───────────────────────────────────────────────────────────

/// Load the full mini_nestle fixture, then trim it down to NESTLE_SA only.
///
/// Manipulating the parsed config (rather than maintaining a separate
/// single-entity fixture) keeps this test in lockstep with the multi-entity
/// fixture — every change to mini_nestle.yaml automatically flows through
/// here, and we only have one place to keep up to date.
fn load_single_entity_manifest() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let mut cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse into GroupConfig");

    // Retain only NESTLE_SA — drop the other entities so the resulting
    // shard plan puts exactly one entity into S_SIG_0001.
    cfg.ownership.entities.retain(|e| e.code == "NESTLE_SA");

    // Strip every IC relationship — both explicit pairs and patterns —
    // so the manifest builder doesn't reject references to dropped
    // entities and IC injection produces an empty extra-JE list (the
    // single-entity case has nothing to net against).
    cfg.intercompany.relationships.clear();

    // The fixture's `tax.pillar_two.jurisdictions: [CH, DE, US]` references
    // entities we just dropped — the manifest's tax-plan builder rejects
    // any jurisdiction that doesn't appear in some entity's country, so we
    // narrow the list to just CH (the country of NESTLE_SA, the lone
    // remaining entity).  The cbc_report and transfer_pricing branches
    // tolerate the trimmed entity list as-is.
    if let Some(p2) = cfg.tax.pillar_two.as_mut() {
        p2.jurisdictions.retain(|j| j == "CH");
    }
    if let Some(tp) = cfg.tax.transfer_pricing.as_mut() {
        tp.local_files_for.retain(|j| j == "CH");
    }

    // FX rates against currencies of the dropped entities aren't required
    // either, but the builder tolerates extra rate columns so we leave
    // them — exercising that tolerance in passing.

    build_manifest(&cfg).expect("trimmed mini_nestle must still build a manifest")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Smoke test: `run_shard` runs the orchestrator end-to-end for a single
/// entity, writes per-entity output under `entities/{code}/`, and emits a
/// `shard_summary.json` whose contents round-trip back to the in-memory
/// summary.
///
/// This is the v5.0 happy path.  Multi-entity tests with IC mirror checks
/// land in Task 4.5; production fleet smoke (real two-entity manifest +
/// IC reconciliation) lands later in the v5.0 plan.
///
/// # Why `#[ignore]`?
///
/// Driving [`datasynth_runtime::EnhancedOrchestrator`] for a single
/// `mini_nestle` entity (even after the manifest is trimmed to
/// `NESTLE_SA` and IC relationships stripped) consumes ~17 GiB peak RSS
/// against the workstation's 32 GiB envelope and runs for 15+ minutes —
/// it has triggered host OOMs twice during v5.0 development.  The
/// runner's *wiring* is exercised by `run_shard_unknown_shard_id_errors`
/// (which fails fast before any orchestrator construction); end-to-end
/// orchestrator execution is intentionally deferred to the XXL Azure VM
/// verification harness (see
/// `docs/superpowers/plans/2026-04-23-group-audit-v5.0-xxl-verification.md`).
/// Run it explicitly with `cargo test … -- --ignored` on a host with
/// ≥64 GiB of free RAM.
#[test]
#[ignore = "runs full EnhancedOrchestrator (~17 GiB peak RSS, 15+ min) — XXL VM only"]
fn run_shard_writes_per_entity_output_and_summary() {
    let manifest = load_single_entity_manifest();
    let tmp = TempDir::new().expect("tempdir");
    let out_dir = tmp.path();

    // The single retained entity must land in some shard — that's the one
    // we drive.  Looking it up from the manifest (rather than hard-coding
    // "S_SIG_0001") keeps the test resilient to shard-id format changes.
    let nestle_sa = manifest
        .ownership_graph
        .entities
        .iter()
        .find(|e| e.code == "NESTLE_SA")
        .expect("NESTLE_SA must remain after trimming");
    let shard_id = nestle_sa.shard_id.clone();

    let summary = run_shard(&manifest, &shard_id, out_dir).expect("run_shard must succeed");

    // ── Top-level shape ───────────────────────────────────────────────────
    assert_eq!(summary.shard_id, shard_id);
    assert_eq!(
        summary.entity_summaries.len(),
        1,
        "trimmed fixture must produce exactly one entity summary; got {}",
        summary.entity_summaries.len()
    );

    // ── Per-entity output ─────────────────────────────────────────────────
    for entity_summary in &summary.entity_summaries {
        // Path the runner declared in the summary must actually exist on
        // disk — guards against subtle drift between OutputRootConfig and
        // the EntitySummary.output_subdir.
        let entity_dir = out_dir.join(&entity_summary.output_subdir);
        assert!(
            entity_dir.is_dir(),
            "{} must be a directory after run_shard",
            entity_dir.display()
        );

        // Journal entries are JSON-only in v5.0 (Task 10 will add
        // CSV/Parquet flags).  Every entity must have at least the JE
        // file — that's the orchestrator's primary output.
        let je_file = entity_dir.join("journal_entries.json");
        assert!(
            je_file.is_file(),
            "journal_entries.json missing for {}",
            entity_summary.entity_code
        );

        assert!(
            entity_summary.journal_entry_count > 0,
            "{} produced zero JEs — orchestrator output regression",
            entity_summary.entity_code
        );

        // Single-entity case — no IC partners → no IC-injected JEs.
        assert_eq!(
            entity_summary.ic_journal_entry_count, 0,
            "{} has no IC partners after trimming; ic_journal_entry_count must be 0",
            entity_summary.entity_code
        );

        // The output_subdir field is a stable contract for aggregate
        // readers — assert the format explicitly so a refactor that
        // changes it (e.g. to absolute paths) trips this test.
        assert_eq!(
            entity_summary.output_subdir,
            format!("entities/{}", entity_summary.entity_code),
            "{} output_subdir must be relative to out_dir",
            entity_summary.entity_code
        );
    }

    // ── shard_summary.json ────────────────────────────────────────────────
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

/// Error path: an unknown `shard_id` produces a `GroupError::Shard` whose
/// message names the bad shard so caller logs pinpoint the typo.
#[test]
fn run_shard_unknown_shard_id_errors() {
    let manifest = load_single_entity_manifest();
    let tmp = TempDir::new().expect("tempdir");
    let out_dir = tmp.path();

    let err = run_shard(&manifest, "S_NOT_A_REAL_SHARD", out_dir)
        .expect_err("unknown shard_id must produce an error");

    let msg = err.to_string();
    assert!(
        msg.contains("S_NOT_A_REAL_SHARD"),
        "error message must mention the bad shard_id; got: {msg}"
    );

    // Sanity: no shard_summary.json should have been written when the
    // shard_id check fails up front.
    assert!(
        !out_dir.join("shard_summary.json").exists(),
        "shard_summary.json must not be written when shard_id is invalid"
    );
}

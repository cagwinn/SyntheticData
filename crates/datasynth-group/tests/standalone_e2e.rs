//! Task 9.2 — standalone `generate_standalone` end-to-end smoke test.
//!
//! Drives [`datasynth_group::generate_standalone`] against a trimmed
//! Mini-Nestlé fixture (NESTLE_SA + NESTLE_USA, single explicit
//! `goods_sale` IC relationship — same trim
//! `tests/shard_e2e.rs::load_two_entity_manifest` uses).  The test
//! verifies that the standalone driver:
//!
//! 1. Persists `manifest.json` at the canonical path.
//! 2. Runs every shard via [`datasynth_group::shard::run_shard`] —
//!    each entity gets a `journal_entries.json` archive under
//!    `entities/{code}/`.
//! 3. Runs the aggregate driver on the resulting tree, emitting all
//!    8 consolidated artefacts (BS / IS / CF / changes-in-equity
//!    bundle, schedule, notes, NCI / CTA / equity-method rollforwards,
//!    translation worksheet, IC matching coverage report).
//! 4. Returns a [`StandaloneSummary`] linking the manifest, every
//!    shard's [`ShardSummary`], and the [`AggregateSummary`].
//!
//! # Why `#[ignore]`?
//!
//! [`generate_standalone`] sequences the **full**
//! [`datasynth_runtime::EnhancedOrchestrator::generate`] cycle for every
//! entity in `manifest.shard_plan.shards` — peak ~17 GiB RSS per entity
//! for ~15 minutes (see `tests/shard_e2e.rs` for the same caveat).
//! Running it on the workstation OOMs the host.  The XXL Azure VM
//! verification harness
//! (`docs/superpowers/plans/2026-04-23-group-audit-v5.0-xxl-verification.md`
//! §3.2.b) is the canonical place to run this test.

use std::fs;

use tempfile::TempDir;

use datasynth_group::{
    generate_standalone, GroupConfig, IcRelationshipConfig, StandaloneOptions,
    CONSOLIDATED_FS_FILENAME, CONSOLIDATION_SCHEDULE_FILENAME, COVERAGE_REPORT_FILENAME,
    CTA_ROLLFORWARD_FILENAME, EQUITY_METHOD_INVESTMENTS_FILENAME, NCI_ROLLFORWARD_FILENAME,
    NOTES_FILENAME, TRANSLATION_WORKSHEET_FILENAME,
};

// ── Fixture helpers ───────────────────────────────────────────────────────────

/// Trim the Mini-Nestlé fixture down to NESTLE_SA + NESTLE_USA so the
/// test exercises a single shard with a single IC relationship — the
/// minimum that drives the full pipeline.  Mirrors
/// `tests/shard_e2e.rs::load_two_entity_manifest`'s trim logic.
fn trimmed_two_entity_config() -> GroupConfig {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let mut cfg: GroupConfig = serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse");

    cfg.ownership
        .entities
        .retain(|e| matches!(e.code.as_str(), "NESTLE_SA" | "NESTLE_USA"));

    cfg.intercompany.relationships.retain(|r| match r {
        IcRelationshipConfig::Explicit(e) => e.seller == "NESTLE_SA" && e.buyer == "NESTLE_USA",
        IcRelationshipConfig::Pattern(_) => false,
    });
    assert_eq!(
        cfg.intercompany.relationships.len(),
        1,
        "trim must leave exactly one explicit NESTLE_SA→NESTLE_USA relationship",
    );

    if let Some(p2) = cfg.tax.pillar_two.as_mut() {
        p2.jurisdictions
            .retain(|j| matches!(j.as_str(), "CH" | "US"));
    }
    if let Some(tp) = cfg.tax.transfer_pricing.as_mut() {
        tp.local_files_for
            .retain(|j| matches!(j.as_str(), "CH" | "US"));
    }

    cfg
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// End-to-end smoke: the standalone driver produces a complete
/// archive — manifest persisted, both per-entity shards generated,
/// every consolidated artefact emitted.
///
/// See module-level rustdoc for the `#[ignore]` rationale.
#[test]
#[ignore = "drives 2× full EnhancedOrchestrator runs (~17 GiB peak RSS each, 30+ min total) — XXL VM only"]
fn generate_standalone_produces_full_archive() {
    let cfg = trimmed_two_entity_config();
    let tmp = TempDir::new().expect("tempdir");
    let out_dir = tmp.path();

    // Sequential (`parallel_shards = false`) so peak RSS stays at one
    // orchestrator's worth (~17 GiB) rather than scaling linearly with
    // shard count.  The XXL VM has the headroom for parallel; the
    // determinism harness tests it explicitly.
    let opts = StandaloneOptions {
        parallel_shards: false,
        ..StandaloneOptions::default()
    };
    let summary = generate_standalone(&cfg, out_dir, &opts).expect("generate_standalone");

    // ── 1. Manifest persisted at canonical path ──────────────────────
    let manifest_path = out_dir.join("manifest.json");
    assert!(
        manifest_path.exists(),
        "manifest.json must be persisted at {}",
        manifest_path.display(),
    );
    assert_eq!(summary.manifest_path, manifest_path);

    // ── 2. Per-entity shard archives present ─────────────────────────
    for code in ["NESTLE_SA", "NESTLE_USA"] {
        let je_path = out_dir
            .join("entities")
            .join(code)
            .join("journal_entries.json");
        assert!(
            je_path.exists(),
            "journal_entries.json missing for {code} at {}",
            je_path.display(),
        );
        let tb_path = out_dir
            .join("entities")
            .join(code)
            .join("period_close")
            .join("trial_balances.json");
        assert!(
            tb_path.exists(),
            "trial_balances.json missing for {code} at {}",
            tb_path.display(),
        );
    }

    // ── 3. Every consolidated artefact emitted ───────────────────────
    let consolidated = out_dir.join("consolidated");
    assert!(consolidated.exists(), "consolidated/ must be created");
    assert!(consolidated.join(CONSOLIDATED_FS_FILENAME).exists());
    assert!(consolidated.join(CONSOLIDATION_SCHEDULE_FILENAME).exists());
    assert!(consolidated.join(NOTES_FILENAME).exists());
    assert!(consolidated.join(CTA_ROLLFORWARD_FILENAME).exists());
    assert!(consolidated.join(NCI_ROLLFORWARD_FILENAME).exists());
    assert!(consolidated
        .join(EQUITY_METHOD_INVESTMENTS_FILENAME)
        .exists());
    assert!(consolidated.join(TRANSLATION_WORKSHEET_FILENAME).exists());

    let coverage_path = out_dir
        .join("ic_eliminations")
        .join(COVERAGE_REPORT_FILENAME);
    assert!(coverage_path.exists(), "coverage report must exist");

    // ── 4. Summary content ──────────────────────────────────────────
    assert!(
        !summary.shard_summaries.is_empty(),
        "at least one shard must run",
    );
    assert!(summary
        .aggregate
        .entities_processed
        .contains(&"NESTLE_SA".to_string()));
    assert!(summary
        .aggregate
        .entities_processed
        .contains(&"NESTLE_USA".to_string()));
    assert!(summary.aggregate.entities_missing.is_empty());
    assert!(
        (summary.aggregate.coverage - 1.0).abs() < 1e-9,
        "expected coverage 1.0, got {}",
        summary.aggregate.coverage,
    );

    // ── 5. Manifest round-trip ──────────────────────────────────────
    let manifest_bytes = fs::read(&manifest_path).expect("read manifest");
    assert!(!manifest_bytes.is_empty(), "manifest must be non-empty");
}

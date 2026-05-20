//! Shard runner — Task 4.3.
//!
//! [`run_shard`] is the entry point a fleet dispatcher (or a single-process
//! caller) invokes for each shard in the manifest's
//! [`crate::manifest::ShardPlan`].  It iterates the entities assigned to the
//! shard, drives the standalone [`EnhancedOrchestrator`] for each one, and
//! routes generated output under
//! `{out_dir}/entities/{entity_code}/`.
//!
//! The orchestrator's existing output shape is preserved verbatim — every
//! file the standalone single-entity flow produces lands in the same
//! relative location, just rooted at the per-entity subtree instead of
//! `out_dir` directly.  This keeps downstream consumers (aggregate-phase
//! readers, audit FSM, AssureTwin) byte-compatible with the pre-v5.0 shape
//! within each entity subtree.
//!
//! After every entity in the shard is generated, the runner writes a
//! [`ShardSummary`] manifest at `{out_dir}/shard_summary.json`.  Aggregate
//! phase readers can scan the per-shard summaries before walking the full
//! `entities/` subtree.
//!
//! # v5.0 scope
//!
//! - Output format is hard-coded to JSON via
//!   [`datasynth_config::FileFormat::Json`] and
//!   [`datasynth_config::ExportLayout::Nested`].  Task 10 will plumb the
//!   format/layout flags through the CLI surface so callers can opt into
//!   CSV/Parquet/Flat-JSON output.
//! - Orchestrator failures are wrapped as [`GroupError::Shard`] with the
//!   entity code prepended so a shard log pinpoints exactly which entity
//!   broke without the caller having to grep upstream stack traces.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use datasynth_config::{ExportLayout, FileFormat};
use datasynth_output::OutputRootConfig;
use datasynth_runtime::output_writer::write_all_output_with_root;
use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};

use crate::errors::{GroupError, GroupResult};
use crate::manifest::builder::{GroupManifest, ManifestEntity};
use crate::shard::context::build_shard_context;
use crate::shard::per_entity_config::build_entity_generator_config;

// ── Public types ──────────────────────────────────────────────────────────────

/// Per-entity result captured after a successful orchestrator run.
///
/// `journal_entry_count` is the **total** count of journal entries the
/// orchestrator emitted for this entity, including the IC journal entries
/// the shard runner installed via [`crate::shard::context::build_shard_context`]
/// (Phase 4c hook in [`EnhancedOrchestrator`]).  `ic_journal_entry_count`
/// is the subset that came from `ShardContext.extra_journal_entries` —
/// captured before [`EnhancedOrchestrator::generate`] is called so the
/// number is exact, not a heuristic post-hoc filter.
///
/// `output_subdir` is the relative path beneath the shard's `out_dir`
/// where the entity's archive was written; aggregate-phase readers walk
/// these directly without re-deriving the path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntitySummary {
    pub entity_code: String,
    pub journal_entry_count: usize,
    pub ic_journal_entry_count: usize,
    pub output_subdir: String,
}

/// Top-level shard summary written to `{out_dir}/shard_summary.json`.
///
/// Aggregate-phase readers pick the shard summaries up before walking the
/// `entities/` subtree: it gives them a quick `(shard_id → entity_codes)`
/// map without having to enumerate the directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShardSummary {
    pub shard_id: String,
    pub entity_summaries: Vec<EntitySummary>,
}

// ── Public runner ─────────────────────────────────────────────────────────────

/// Drive the shard for `shard_id`: run the orchestrator once per assigned
/// entity, write each archive under `{out_dir}/entities/{code}/`, and
/// emit a [`ShardSummary`] manifest at `{out_dir}/shard_summary.json`.
///
/// # Errors
///
/// - [`GroupError::Shard`] if `shard_id` matches no entity in the manifest
///   (caller bug: typo in the shard id).
/// - [`GroupError::Shard`] wrapping any orchestrator construction,
///   `set_shard_context`, `generate`, or output-writer failure — the entity
///   code is prefixed onto the message so logs pinpoint the failing shard.
/// - [`GroupError::Io`] when the `out_dir` itself cannot be created (e.g.
///   permission denied).
/// - [`GroupError::Serde`] when the shard summary cannot be serialised
///   (effectively impossible for the [`ShardSummary`] shape, but we
///   surface it cleanly rather than panic).
pub fn run_shard(
    manifest: &GroupManifest,
    shard_id: &str,
    out_dir: &Path,
) -> GroupResult<ShardSummary> {
    run_shard_with_opening_balances(
        manifest,
        shard_id,
        out_dir,
        &std::collections::BTreeMap::new(),
    )
}

/// **v5.3** — Drive the shard with per-entity opening-balance
/// carryovers from a prior period.  Identical to [`run_shard`] except
/// that for each entity in `entity_opening_balances`, the orchestrator's
/// `ShardContext.opening_balances` is pre-populated with the supplied
/// `EntityOpeningBalance` records instead of starting from empty.
/// The orchestrator's Phase 3b consumes those carryovers in place of
/// the industry-mix `OpeningBalanceGenerator`.
///
/// Empty `entity_opening_balances` (or no entry for a given entity)
/// matches `run_shard`'s behaviour byte-for-byte.  This is the path
/// `generate_standalone_chain` uses to thread the prior period's
/// closing TBs into the next period's generation.
pub fn run_shard_with_opening_balances(
    manifest: &GroupManifest,
    shard_id: &str,
    out_dir: &Path,
    entity_opening_balances: &std::collections::BTreeMap<
        String,
        Vec<datasynth_core::models::balance::EntityOpeningBalance>,
    >,
) -> GroupResult<ShardSummary> {
    // ── 1. Filter entities for this shard ────────────────────────────────────
    // Collect into Vec so we can validate emptiness up front and iterate
    // without re-walking the manifest twice.
    let entities: Vec<&ManifestEntity> = manifest
        .ownership_graph
        .entities
        .iter()
        .filter(|e| e.shard_id == shard_id)
        .collect();

    if entities.is_empty() {
        return Err(GroupError::Shard(format!(
            "run_shard: no entities matched shard_id `{shard_id}` — \
             check it against `manifest.shard_plan.shards[*].shard_id`"
        )));
    }

    // ── 2. Ensure the output directory exists before any orchestrator run ───
    fs::create_dir_all(out_dir).map_err(|e| {
        GroupError::Shard(format!(
            "run_shard: failed to create out_dir `{}`: {e}",
            out_dir.display()
        ))
    })?;

    // ── 3. Drive the orchestrator once per entity ────────────────────────────
    let mut entity_summaries: Vec<EntitySummary> = Vec::with_capacity(entities.len());

    for entity in &entities {
        let openings = entity_opening_balances
            .get(&entity.code)
            .cloned()
            .unwrap_or_default();
        let summary = run_one_entity(manifest, entity, out_dir, &openings)?;
        entity_summaries.push(summary);
    }

    // ── 4. Build and persist the shard summary ───────────────────────────────
    let summary = ShardSummary {
        shard_id: shard_id.to_string(),
        entity_summaries,
    };

    let summary_path = out_dir.join("shard_summary.json");
    let summary_json = serde_json::to_string_pretty(&summary)?;
    fs::write(&summary_path, &summary_json).map_err(|e| {
        GroupError::Shard(format!(
            "run_shard: failed to write shard_summary.json at `{}`: {e}",
            summary_path.display()
        ))
    })?;

    Ok(summary)
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Run the orchestrator for a single entity and return its [`EntitySummary`].
///
/// Split out from [`run_shard`] so each step has a clean error surface and
/// the multi-entity loop body stays linear.  Every failure path here wraps
/// into [`GroupError::Shard`] with the entity code prepended so the caller
/// sees `"shard error: ACME_USA: orchestrator setup: …"` rather than
/// having to map error origins back to entities themselves.
fn run_one_entity(
    manifest: &GroupManifest,
    entity: &ManifestEntity,
    out_dir: &Path,
    opening_balances: &[datasynth_core::models::balance::EntityOpeningBalance],
) -> GroupResult<EntitySummary> {
    // 1. Build the per-entity GeneratorConfig.
    let config = build_entity_generator_config(manifest, entity).map_err(|e| {
        GroupError::Shard(format!(
            "{}: build_entity_generator_config failed: {e}",
            entity.code
        ))
    })?;

    // 2. Construct the orchestrator with phase config derived from the
    //    config (canonical constructor — see datasynth-runtime/lib.rs).
    let phase_config = PhaseConfig::from_config(&config);
    let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).map_err(|e| {
        GroupError::Shard(format!(
            "{}: EnhancedOrchestrator::new failed: {e}",
            entity.code
        ))
    })?;

    // 3. Build the shard context for this entity (carries pre-built IC JEs).
    //    Capture the IC JE count *before* the orchestrator consumes it via
    //    `set_shard_context` — that gives us the deterministic post-hoc
    //    `ic_journal_entry_count` without re-classifying entries downstream.
    let mut ctx = build_shard_context(manifest, &entity.code).map_err(|e| {
        GroupError::Shard(format!("{}: build_shard_context failed: {e}", entity.code))
    })?;
    let ic_journal_entry_count = ctx.extra_journal_entries.len();
    // **v5.3** — install the multi-period opening-balance carryover
    // when the caller supplied one for this entity.  Empty `opening_balances`
    // (the v5.0–v5.2 default) leaves `ctx.opening_balances` empty and the
    // orchestrator's Phase 3b falls through to the industry-mix generator.
    if !opening_balances.is_empty() {
        ctx.opening_balances = opening_balances.to_vec();
    }
    orchestrator.set_shard_context(ctx);

    // 4. Drive generation.
    let result = orchestrator
        .generate()
        .map_err(|e| GroupError::Shard(format!("{}: generate failed: {e}", entity.code)))?;
    let journal_entry_count = result.journal_entries.len();

    // 5. Route output under {out_dir}/entities/{entity_code}/.
    //    v5.0 hardcodes Nested layout + JSON-only output — Task 10 will plumb
    //    the format/layout flags through the CLI surface.
    let root = OutputRootConfig::per_entity(out_dir, &entity.code);
    write_all_output_with_root(&result, &root, ExportLayout::Nested, &[FileFormat::Json]).map_err(
        |e| {
            GroupError::Shard(format!(
                "{}: write_all_output_with_root failed: {e}",
                entity.code
            ))
        },
    )?;

    // 6. **v5.2** — emit ownership-change events when the manifest
    //    declares them for this entity.  The orchestrator pipeline
    //    doesn't synthesise these events (they're engagement facts
    //    declared in config, not generated data), so the shard runner
    //    writes them directly from the manifest into the per-entity
    //    archive at `intercompany/ownership_change_events.json`.
    //    No-op when the entity has no events — preserves backwards
    //    compatibility byte-for-byte for v5.0–v5.1 archives.
    if !entity.ownership_changes.is_empty() {
        let oc_dir = out_dir
            .join("entities")
            .join(&entity.code)
            .join("intercompany");
        fs::create_dir_all(&oc_dir).map_err(|e| {
            GroupError::Shard(format!(
                "{}: cannot create intercompany dir `{}`: {e}",
                entity.code,
                oc_dir.display()
            ))
        })?;
        let path = oc_dir.join("ownership_change_events.json");
        let json = serde_json::to_string_pretty(&entity.ownership_changes).map_err(|e| {
            GroupError::Shard(format!(
                "{}: failed to serialise ownership_change_events to JSON: {e}",
                entity.code
            ))
        })?;
        fs::write(&path, json).map_err(|e| {
            GroupError::Shard(format!(
                "{}: cannot write `{}`: {e}",
                entity.code,
                path.display()
            ))
        })?;
    }

    Ok(EntitySummary {
        entity_code: entity.code.clone(),
        journal_entry_count,
        ic_journal_entry_count,
        output_subdir: format!("entities/{}", entity.code),
    })
}

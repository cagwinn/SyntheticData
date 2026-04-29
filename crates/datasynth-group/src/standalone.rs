//! Standalone single-process generation — Task 9.2.
//!
//! [`generate_standalone`] runs the full v5.0 pipeline — manifest +
//! shards + aggregate — in one call, without spawning a subprocess
//! per phase.  It is the in-process equivalent of:
//!
//! ```text
//! datasynth-data group manifest --config group.yaml --out manifest.json
//! datasynth-data group shard    --manifest manifest.json --shard $SHARD_ID --out ./out
//! datasynth-data group aggregate --manifest manifest.json --shards-dir ./out --out ./out
//! ```
//!
//! ...with manifest persistence at `out_dir/manifest.json` for
//! debuggability and for parity with the multi-step CLI flow.
//!
//! # Memory caveat — orchestrator runs are heavy
//!
//! [`run_shard`] drives [`datasynth_runtime::EnhancedOrchestrator::generate`]
//! end-to-end for every entity in the shard.  Each Mini-Nestlé entity
//! peaks at **~17 GiB RSS** for ~15 minutes; running all five entities
//! sequentially takes 60–90 minutes on a single host.
//!
//! When [`StandaloneOptions::parallel_shards = true`] (the default),
//! the driver uses [`rayon`] to schedule shards concurrently.  Peak
//! RSS scales linearly with the number of shards in flight — N shards
//! × 17 GiB ≈ 17·N GiB.  This is fine on the XXL Azure VM (256 GiB)
//! but will OOM a 32 GiB workstation in seconds.  The associated
//! `tests/standalone_e2e.rs` is `#[ignore]`d for exactly this reason —
//! mirror the pattern in [`crate::shard::runner::run_shard`]
//! integration tests.
//!
//! For the determinism harness (`tests/property/determinism.rs`)
//! callers should pass `parallel_shards: false` so two runs over the
//! same input produce byte-identical archives without the rayon
//! scheduler's non-deterministic interleaving in flight (writes to
//! disk are still deterministic per-shard because the runner's per-
//! entity output writer is sync, but the scheduler may flush in a
//! different order).
//!
//! # File layout
//!
//! After a successful run:
//!
//! ```text
//! {out_dir}/
//!   ├── manifest.json
//!   ├── entities/
//!   │   ├── ENTITY_A/        ← per-shard runner output (verbatim)
//!   │   ├── ENTITY_B/
//!   │   └── ...
//!   ├── shard_summary.json   ← from each run_shard call (overwrites
//!   │                         per-shard; the last shard's summary
//!   │                         persists.  Each ShardSummary is also
//!   │                         captured in StandaloneSummary.shard_summaries)
//!   ├── consolidated/        ← from run_aggregate (Chunk 6/7/8 outputs)
//!   └── ic_eliminations/     ← from run_aggregate (coverage report)
//! ```
//!
//! Note: `shard_summary.json` is not race-free across shards — the
//! runner writes it per-shard at `{out_dir}/shard_summary.json`, which
//! means the last shard wins.  This is a known v5.0 limitation; the
//! per-shard summaries are reliably available in
//! [`StandaloneSummary::shard_summaries`].

use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::aggregate::driver::{run_aggregate, AggregateOptions, AggregateSummary};
use crate::config::GroupConfig;
use crate::errors::{GroupError, GroupResult};
use crate::manifest::builder::build_manifest;
use crate::shard::runner::{run_shard, ShardSummary};

// ── Public types ──────────────────────────────────────────────────────────────

/// Knobs the caller can supply to tune [`generate_standalone`]
/// behaviour.
///
/// All fields default to "sensible production": no prior period,
/// fail-fast on missing shards, parallel shard execution.  Override
/// `parallel_shards = false` for determinism harnesses.
#[derive(Debug, Clone)]
pub struct StandaloneOptions {
    /// Forwarded verbatim to
    /// [`crate::aggregate::driver::AggregateOptions::prior_period_aggregate`].
    pub prior_period_aggregate: Option<PathBuf>,
    /// Forwarded verbatim to
    /// [`crate::aggregate::driver::AggregateOptions::tolerate_missing_shards`].
    pub tolerate_missing_shards: bool,
    /// When `true`, run shards in parallel via [`rayon`].  Defaults to
    /// `true`.  Set to `false` for determinism harnesses (sequential
    /// shard execution removes the scheduler's interleaving from the
    /// output trace).
    pub parallel_shards: bool,
}

impl Default for StandaloneOptions {
    fn default() -> Self {
        Self {
            prior_period_aggregate: None,
            tolerate_missing_shards: false,
            parallel_shards: true,
        }
    }
}

/// Top-level result returned by [`generate_standalone`].
///
/// `shard_summaries` is one entry per shard in
/// `manifest.shard_plan.shards`, in the order the manifest declares
/// them (rayon scheduling does not affect the result ordering — we
/// sort post-collect).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StandaloneSummary {
    /// Path to the persisted manifest at `{out_dir}/manifest.json`.
    pub manifest_path: PathBuf,
    /// Per-shard summaries from [`run_shard`], one per shard in the
    /// manifest's [`crate::manifest::shard_plan::ShardPlan`].
    pub shard_summaries: Vec<ShardSummary>,
    /// Aggregate-phase summary from [`run_aggregate`].
    pub aggregate: AggregateSummary,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Drive the full v5.0 pipeline (manifest → shards → aggregate) in one
/// call.
///
/// 1. Build the manifest from `cfg`.
/// 2. Persist it to `{out_dir}/manifest.json`.
/// 3. Run every shard in `manifest.shard_plan.shards` (parallel or
///    sequential per `opts.parallel_shards`), writing per-entity
///    archives under `{out_dir}/entities/{code}/`.
/// 4. Run the aggregate-phase driver on `out_dir` (which now contains
///    every shard's output) and emit consolidated FS artefacts.
/// 5. Return a [`StandaloneSummary`] linking the manifest, every
///    shard's summary, and the aggregate summary.
///
/// # Errors
///
/// - [`GroupError::Manifest`] / [`GroupError::Config`] propagated from
///   [`build_manifest`].
/// - [`GroupError::Shard`] propagated from any [`run_shard`] failure
///   (orchestrator construction, generation, or per-entity output).
/// - [`GroupError::Aggregate`] / [`GroupError::Io`] /
///   [`GroupError::Serde`] propagated from [`run_aggregate`].
/// - [`GroupError::Io`] if the manifest cannot be persisted.
pub fn generate_standalone(
    cfg: &GroupConfig,
    out_dir: &Path,
    opts: &StandaloneOptions,
) -> GroupResult<StandaloneSummary> {
    // ── 1. Manifest ─────────────────────────────────────────────────
    let manifest = build_manifest(cfg)?;

    // ── 2. Persist manifest at out_dir/manifest.json ────────────────
    fs::create_dir_all(out_dir).map_err(GroupError::Io)?;
    let manifest_path = out_dir.join("manifest.json");
    let mut manifest_json = serde_json::to_string_pretty(&manifest)?;
    manifest_json.push('\n');
    fs::write(&manifest_path, manifest_json).map_err(GroupError::Io)?;

    // ── 3. Run every shard ──────────────────────────────────────────
    //
    // Capture the shard ids in declaration order from
    // `shard_plan.shards`, then dispatch via rayon (parallel) or a
    // plain map (sequential).  Both code paths materialise into a
    // `Vec<ShardSummary>` in declaration order so callers receive a
    // deterministic ordering regardless of scheduler interleaving.
    let shard_ids: Vec<String> = manifest
        .shard_plan
        .shards
        .iter()
        .map(|s| s.shard_id.clone())
        .collect();

    let shard_summaries: Vec<ShardSummary> = if opts.parallel_shards {
        // rayon's par_iter preserves the input order in the collected
        // Vec, so the result is still declaration-ordered.
        shard_ids
            .par_iter()
            .map(|sid| run_shard(&manifest, sid, out_dir))
            .collect::<GroupResult<Vec<_>>>()?
    } else {
        let mut out: Vec<ShardSummary> = Vec::with_capacity(shard_ids.len());
        for sid in &shard_ids {
            out.push(run_shard(&manifest, sid, out_dir)?);
        }
        out
    };

    // ── 4. Aggregate phase ──────────────────────────────────────────
    //
    // The runner writes every per-entity archive under
    // `{out_dir}/entities/{code}/`, so `shards_dir == out_dir` for the
    // aggregate driver.
    let agg_opts = AggregateOptions {
        prior_period_aggregate: opts.prior_period_aggregate.clone(),
        tolerate_missing_shards: opts.tolerate_missing_shards,
    };
    let aggregate = run_aggregate(&manifest, out_dir, out_dir, &agg_opts)?;

    Ok(StandaloneSummary {
        manifest_path,
        shard_summaries,
        aggregate,
    })
}

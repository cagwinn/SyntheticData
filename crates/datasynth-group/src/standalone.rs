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
//! `run_shard` drives [`datasynth_runtime::EnhancedOrchestrator::generate`]
//! end-to-end for every entity in the shard.  Each Mini-Acme entity
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
use crate::shard::runner::{run_shard_with_opening_balances, ShardSummary};

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
    /// Forwarded verbatim to
    /// [`crate::aggregate::driver::AggregateOptions::cgu_test_inputs`]
    /// — the per-period CGU goodwill impairment test inputs.  Empty
    /// by default; an engagement that wants annual IAS 36 § 10
    /// impairment testing supplies one entry per CGU under test.
    pub cgu_test_inputs: Vec<crate::aggregate::cgu_impairment::CguTestInputs>,
    /// **v5.5.2** — Forwarded verbatim to
    /// [`crate::aggregate::driver::AggregateOptions::cpi_series_by_currency`].
    /// Per-currency CPI series for IAS 29 § 12 indexed restatement.
    /// Empty by default — single-period engagements without
    /// hyperinflationary subsidiaries see no behaviour change.  When
    /// the chain runner forwards a non-empty map, every period applies
    /// the same series; vary by period at the library level if the
    /// engagement requires per-period CPI overrides.
    pub cpi_series_by_currency: std::collections::BTreeMap<
        String,
        datasynth_core::models::hyperinflation::GeneralPriceIndex,
    >,
    /// **v5.3** — Per-entity opening-balance carryover from a prior
    /// period.  When non-empty, the shard runner pre-populates each
    /// matching entity's `ShardContext.opening_balances` so the
    /// orchestrator's Phase 3b uses these BS positions instead of
    /// generating fresh openings.  Empty by default — single-period
    /// engagements see no behaviour change.
    ///
    /// **v5.31 C2 (#157)** — auto-populated by
    /// [`generate_standalone_chain`] from the prior period's
    /// `entities/{code}/period_close/trial_balances.json` via
    /// [`crate::aggregate::opening_balance::read_prior_period_closing_tbs`]
    /// followed by
    /// [`datasynth_generators::balance::project_closing_to_opening`].
    /// The projection (vs the legacy `extract_opening_balances`) is
    /// what absorbs prior-period net income into Retained Earnings —
    /// the orchestrator emits `Adjusted` TBs (not `PostClosing`), so
    /// dropping P&L without absorbing it would silently lose the
    /// period's earnings on the chain hand-off.
    ///
    /// Callers using `generate_standalone` directly can populate this
    /// manually when they want to drive multi-period continuity
    /// without the chain helper.
    pub entity_opening_balances: std::collections::BTreeMap<
        String,
        Vec<datasynth_core::models::balance::EntityOpeningBalance>,
    >,
    /// **v5.31 C2 (#157)** — accounting framework used to project the
    /// prior period's closing TB onto next-period opening balances.
    /// Selects which account code holds Retained Earnings (US GAAP
    /// `"3200"`, SKR03/04 `"2970"`, IFRS varies).  Only consulted by
    /// [`generate_standalone_chain`] when computing carryover; the
    /// per-period generation itself reads framework off the config.
    /// Defaults to `"us_gaap"`.  Must match how the prior period was
    /// generated — mismatches mean net income lands in the wrong
    /// account.
    pub closing_to_opening_framework: String,
}

impl Default for StandaloneOptions {
    fn default() -> Self {
        Self {
            prior_period_aggregate: None,
            tolerate_missing_shards: false,
            parallel_shards: true,
            cgu_test_inputs: Vec::new(),
            entity_opening_balances: std::collections::BTreeMap::new(),
            cpi_series_by_currency: std::collections::BTreeMap::new(),
            closing_to_opening_framework: "us_gaap".to_string(),
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
    /// Per-shard summaries from `run_shard`, one per shard in the
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
/// - [`GroupError::Shard`] propagated from any `run_shard` failure
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
            .map(|sid| {
                run_shard_with_opening_balances(
                    &manifest,
                    sid,
                    out_dir,
                    &opts.entity_opening_balances,
                )
            })
            .collect::<GroupResult<Vec<_>>>()?
    } else {
        let mut out: Vec<ShardSummary> = Vec::with_capacity(shard_ids.len());
        for sid in &shard_ids {
            out.push(run_shard_with_opening_balances(
                &manifest,
                sid,
                out_dir,
                &opts.entity_opening_balances,
            )?);
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
        cgu_test_inputs: opts.cgu_test_inputs.clone(),
        cpi_series_by_currency: opts.cpi_series_by_currency.clone(),
    };
    let aggregate = run_aggregate(&manifest, out_dir, out_dir, &agg_opts)?;

    Ok(StandaloneSummary {
        manifest_path,
        shard_summaries,
        aggregate,
    })
}

// ── Multi-period chain helper (v5.3) ──────────────────────────────────────────

/// One period in a multi-period engagement chain.  Carries the
/// period-specific config + the output subdirectory the chain
/// runner uses to scope this period's archive.
///
/// `out_subdir` is interpreted relative to the chain's `base_out_dir`
/// (the second parameter of [`generate_standalone_chain`]) — typically
/// something like `"2024_Q1"` or `"period_001"`.  Subdirs must be
/// unique within a chain or [`generate_standalone_chain`] returns
/// [`GroupError::Config`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeriodChainSpec {
    /// Period config (start_date / length / fiscal_year_end) for this
    /// period.  Replaces [`GroupConfig::period`] when the chain runner
    /// invokes the per-period [`generate_standalone`] call.
    pub period: crate::config::PeriodConfig,
    /// Output subdirectory under `base_out_dir`.  Must be unique
    /// within the chain.
    pub out_subdir: String,
}

/// Drive the full v5.0 pipeline once per period, chaining the
/// aggregate-phase prior-period plumbing automatically.
///
/// For each period in `periods`:
///
/// 1. Clone `base_cfg`, override its `.period` with the spec's
///    period config.  All other fields (ownership, fx, audit, tax,
///    cgu, output) flow through unchanged.
/// 2. Compute `out_dir = base_out_dir.join(spec.out_subdir)`.
/// 3. For period 0, use `opts` as-is — caller's
///    `prior_period_aggregate` is preserved (lets engagements seed
///    from an externally produced archive).
/// 4. For periods 1..N, override `opts.prior_period_aggregate` to
///    point to the previous period's `out_dir` so opening NCI / CTA /
///    equity-method values flow forward automatically.
/// 5. Invoke [`generate_standalone`] and append the [`StandaloneSummary`]
///    to the result vector.
///
/// On per-period failure the chain stops; outputs already written for
/// prior periods are preserved on disk.
///
/// # Companion to [`crate::aggregate::run_aggregate_chain`]
///
/// `run_aggregate_chain` (PR #150) handles the same prior-period
/// stitching at the aggregate-only layer (i.e. when shard archives
/// already exist on disk).  This helper extends the pattern to the
/// full pipeline — it generates the data per period **and** chains
/// the aggregate plumbing.
///
/// # Errors
///
/// - [`GroupError::Config`] if `periods` is empty.
/// - [`GroupError::Config`] if any two `out_subdir` values collide.
/// - Any error from the underlying [`generate_standalone`] call.
///
/// # Caveat — orchestrator opening balances
///
/// **v5.3** — This helper now threads **both** the aggregate-phase
/// prior-period plumbing (opening NCI / CTA / equity-method carrying
/// values) **and** the orchestrator-side opening-balance carryover
/// (entity-level opening TB = prior period's closing TB).  Between
/// periods N and N+1:
///
/// 1. After period N's `generate_standalone` returns, walk
///    `period_N_out_dir/entities/{code}/period_close/trial_balances.json`
///    via [`crate::aggregate::opening_balance::read_prior_period_closing_tbs`]
///    for every entity in the manifest.
/// 2. Project each closing TB onto its BS positions via
///    [`crate::aggregate::opening_balance::extract_opening_balances`].
/// 3. Convert the group-side `OpeningBalance` records into core-side
///    `EntityOpeningBalance` records (the runtime-friendly carrier
///    `ShardContext.opening_balances` consumes).
/// 4. Populate period-(N+1)'s `StandaloneOptions.entity_opening_balances`
///    with the per-entity carry-forwards.  The shard runner installs
///    these into each entity's `ShardContext`, and the orchestrator's
///    Phase 3b uses them in place of the industry-mix opening
///    generator.
///
/// On the first period (idx == 0) and when the entity has no prior
/// closing TB on disk (e.g. it was a generated entity that wasn't
/// persisted), the carryover is silently skipped — that entity's
/// orchestrator falls through to fresh generator output.
pub fn generate_standalone_chain(
    base_cfg: &GroupConfig,
    periods: Vec<PeriodChainSpec>,
    base_out_dir: &Path,
    opts: &StandaloneOptions,
) -> GroupResult<Vec<StandaloneSummary>> {
    if periods.is_empty() {
        return Err(GroupError::Config(
            "generate_standalone_chain: periods must be non-empty".to_string(),
        ));
    }

    // Reject duplicate out_subdir values up front — running two
    // periods into the same directory would produce a corrupt archive
    // (the second period's outputs would silently overwrite the first).
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for spec in &periods {
        if !seen.insert(spec.out_subdir.as_str()) {
            return Err(GroupError::Config(format!(
                "generate_standalone_chain: duplicate out_subdir `{}` — every \
                 period must have a unique output directory",
                spec.out_subdir,
            )));
        }
    }

    let mut summaries: Vec<StandaloneSummary> = Vec::with_capacity(periods.len());
    let mut prior_out: Option<PathBuf> = None;

    for (idx, spec) in periods.into_iter().enumerate() {
        let mut cfg = base_cfg.clone();
        cfg.period = spec.period;

        let out_dir = base_out_dir.join(&spec.out_subdir);

        let mut period_opts = opts.clone();
        if idx > 0 {
            period_opts.prior_period_aggregate = prior_out.clone();
            // **v5.3** — auto-thread the orchestrator-side opening-
            // balance carryover from the prior period.  Walk the
            // manifest to know which entity codes to ask for, then
            // load + project each entity's closing TB into runtime-
            // friendly EntityOpeningBalance records.  Entities whose
            // closing TB doesn't exist on disk are silently skipped
            // (best-effort — the orchestrator's industry-mix
            // generator handles them).
            if let Some(prior) = &prior_out {
                let entity_codes: Vec<String> = base_cfg
                    .ownership
                    .entities
                    .iter()
                    .map(|e| e.code.clone())
                    .collect();
                let closing_tbs = crate::aggregate::opening_balance::read_prior_period_closing_tbs(
                    prior,
                    &entity_codes,
                )?;
                // **v5.31 C2 (#157)** — project each closing TB onto its
                // opening positions via `project_closing_to_opening`,
                // which absorbs the period's net income into Retained
                // Earnings. The legacy `extract_opening_balances` used
                // to live here, but it silently dropped P&L lines
                // without absorbing them — correct only when the
                // closing TB is already PostClosing (zero P&L). The
                // orchestrator emits `Adjusted` TBs (still showing
                // period P&L), so the legacy path lost net income on
                // the chain hand-off.
                let mut openings_by_entity: std::collections::BTreeMap<
                    String,
                    Vec<datasynth_core::models::balance::EntityOpeningBalance>,
                > = std::collections::BTreeMap::new();
                for (code, tb) in &closing_tbs {
                    let core_openings = datasynth_generators::balance::project_closing_to_opening(
                        tb,
                        &opts.closing_to_opening_framework,
                    );
                    if !core_openings.is_empty() {
                        openings_by_entity.insert(code.clone(), core_openings);
                    }
                }
                period_opts.entity_opening_balances = openings_by_entity;
            }
        }

        let summary = generate_standalone(&cfg, &out_dir, &period_opts)?;
        prior_out = Some(out_dir);
        summaries.push(summary);
    }

    Ok(summaries)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::path::PathBuf;

    #[test]
    fn period_chain_spec_field_shape() {
        // Pin the public API shape — refactors that hide or rename
        // fields will break the build.
        let spec = PeriodChainSpec {
            period: crate::config::PeriodConfig {
                start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                length: crate::config::PeriodLength::Quarterly,
                fiscal_year_end: None,
            },
            out_subdir: "2024_Q1".to_string(),
        };
        assert_eq!(spec.out_subdir, "2024_Q1");
        assert_eq!(spec.period.length, crate::config::PeriodLength::Quarterly);
    }

    #[test]
    fn empty_periods_rejected() {
        let cfg = sample_min_config();
        let err = generate_standalone_chain(
            &cfg,
            Vec::new(),
            &PathBuf::from("/tmp/never"),
            &StandaloneOptions::default(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("must be non-empty"));
    }

    #[test]
    fn duplicate_out_subdir_rejected() {
        let cfg = sample_min_config();
        let p1 = PeriodChainSpec {
            period: crate::config::PeriodConfig {
                start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                length: crate::config::PeriodLength::Quarterly,
                fiscal_year_end: None,
            },
            out_subdir: "DUP".to_string(),
        };
        let p2 = PeriodChainSpec {
            period: crate::config::PeriodConfig {
                start_date: NaiveDate::from_ymd_opt(2024, 4, 1).unwrap(),
                length: crate::config::PeriodLength::Quarterly,
                fiscal_year_end: None,
            },
            out_subdir: "DUP".to_string(),
        };
        let err = generate_standalone_chain(
            &cfg,
            vec![p1, p2],
            &PathBuf::from("/tmp/never"),
            &StandaloneOptions::default(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("duplicate out_subdir"));
    }

    #[test]
    fn standalone_options_default_has_empty_entity_opening_balances() {
        // v5.3 carryover field defaults to empty — single-period
        // engagements see no behaviour change.
        let opts = StandaloneOptions::default();
        assert!(opts.entity_opening_balances.is_empty());
    }

    #[test]
    fn standalone_options_can_carry_per_entity_openings() {
        use datasynth_core::models::balance::{AccountType, EntityOpeningBalance};
        use rust_decimal::Decimal;
        let mut opts = StandaloneOptions::default();
        opts.entity_opening_balances.insert(
            "SUB".to_string(),
            vec![EntityOpeningBalance {
                account_code: "1000".to_string(),
                account_type: AccountType::Asset,
                debit: Decimal::from(50_000),
                credit: Decimal::ZERO,
            }],
        );
        assert_eq!(opts.entity_opening_balances.len(), 1);
        assert_eq!(opts.entity_opening_balances["SUB"][0].account_code, "1000");
    }

    /// Helper — minimum-viable GroupConfig for shape-only tests.
    /// Doesn't reach the orchestrator (early-validation tests bail
    /// before generate_standalone is called).
    fn sample_min_config() -> GroupConfig {
        GroupConfig {
            id: "TEST".to_string(),
            name: None,
            presentation_currency: "CHF".to_string(),
            period: crate::config::PeriodConfig {
                start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                length: crate::config::PeriodLength::Quarterly,
                fiscal_year_end: None,
            },
            seed: 42,
            defaults: serde_yaml::Value::Null,
            scoping_profiles: Default::default(),
            ownership: crate::config::OwnershipConfig {
                parent_entity_code: "P".to_string(),
                entities: Vec::new(),
                generated: Vec::new(),
                entities_from: None,
            },
            intercompany: Default::default(),
            fx: crate::config::FxConfig {
                base_currency: "CHF".to_string(),
                rate_source: Default::default(),
                rates: Default::default(),
                policy: crate::config::FxPolicyConfig {
                    balance_sheet: crate::config::FxRateBasis::Closing,
                    income_statement: crate::config::FxRateBasis::Average,
                    equity: crate::config::FxRateBasis::Historical,
                },
            },
            audit: Default::default(),
            tax: Default::default(),
            cgu: Default::default(),
            output: Default::default(),
            fleet: None,
        }
    }
}

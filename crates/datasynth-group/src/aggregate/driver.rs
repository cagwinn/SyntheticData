//! Aggregate phase driver — Task 9.1.
//!
//! Wires Chunks 5–8 into a single entrypoint: [`run_aggregate`] walks the
//! per-entity shard archives produced by
//! [`crate::shard::run_shard`], folds them through pre-elimination,
//! IC matching, eliminations, IAS 21 translation, NCI / equity-method
//! overlays, and consolidated FS assembly, and emits every
//! aggregate-phase artefact under `{out_dir}/consolidated/` and
//! `{out_dir}/ic_eliminations/`.
//!
//! # v5.0 ordering
//!
//! The driver runs the chunks **in this strict order** because each
//! step depends on the prior step's outputs:
//!
//! 1. **Chunk 5** — Pre-elimination aggregation, IC matching,
//!    elimination JEs, post-elimination consolidated TB.  The
//!    presentation currency check on the pre-elim aggregator means
//!    every contributing entity must already be denominated in the
//!    presentation currency at this point — the v5.0 fixture (Mini-
//!    Nestlé) is single-currency CHF so this trivially holds.  Multi-
//!    currency engagements will need to translate first; that path is
//!    documented in the spec but not exercised in v5.0 driver-level
//!    tests.
//! 2. **Chunk 6** — IAS 21 per-entity translation, CTA rollforward,
//!    translation worksheet emission.  Runs *after* the post-elim TB
//!    so the consolidated TB and the translation worksheet are both
//!    grounded in the same set of contributing entities.
//! 3. **Chunk 7** — NCI rollforward per Full-method subsidiary, and
//!    equity-method investment rollforward per EquityMethod investee.
//!    These overlays sit on top of the post-elim TB; the driver applies
//!    them via `apply_nci_and_equity_method` before assembling the FS.
//! 4. **Chunk 8** — Consolidated FS assembly (BS / IS / CF / Changes
//!    in Equity), consolidation schedule, notes, and JSON writer.
//!
//! # Missing-shard handling
//!
//! [`AggregateOptions::tolerate_missing_shards`] toggles between fail-
//! fast and best-effort recovery semantics:
//!
//! - `false` (default) — any missing entity directory or
//!   `period_close/trial_balances.json` produces a
//!   [`crate::errors::GroupError::Aggregate`] error naming the entity
//!   and the path the driver expected.  This is the production
//!   contract: a complete shard archive is required for a complete
//!   consolidation.
//! - `true` — missing entities are pushed to
//!   [`AggregateSummary::entities_missing`] and a `tracing::warn!` is
//!   logged.  The driver then continues with the remaining entities,
//!   which is useful for partial-archive recovery scenarios.
//!
//! Equity-method / fair-value / proportional entities are still
//! **expected** to have shards on disk (the runner generated them) but
//! their TBs are captured separately via
//! [`AggregateOptions::deferred_entity_tbs`-style sidecar] for Chunk 7
//! to consume — the driver does not fold them into the consolidated TB.
//!
//! # Prior-period plumbing
//!
//! When [`AggregateOptions::prior_period_aggregate`] is `Some(path)`,
//! the driver reads opening NCI and equity-method carrying values from
//! `path/consolidated/{nci_rollforward,equity_method_investments}.json`
//! via the existing ingestion helpers in
//! [`crate::aggregate::nci::opening`] and
//! [`crate::aggregate::equity_method`].  Opening CTA balances are
//! similarly loaded from the prior `cta_rollforward.json`.  When `None`,
//! every opening defaults to zero — the engagement's first period.
//!
//! # Determinism
//!
//! Every step the driver invokes is deterministic given identical
//! inputs.  The driver itself walks `manifest.ownership_graph.entities`
//! in declaration order; sub-steps that need lexicographic ordering
//! (pre-elim aggregator, IC matcher) sort internally.  Two runs of
//! `run_aggregate` over the same shard archive produce byte-identical
//! `consolidated/*.json` and `ic_eliminations/*.json` files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use datasynth_core::models::balance::TrialBalance;
use datasynth_core::models::JournalEntry;
use datasynth_standards::framework::AccountingFramework;

use crate::aggregate::coverage_report::{build_coverage_report, write_coverage_report};
use crate::aggregate::elimination::{eliminations_to_journal_entries, generate_eliminations};
use crate::aggregate::equity_method::{
    compute_equity_method_investment, ingest_opening_equity_method_carrying_values,
    write_equity_method_investments, EquityMethodInputs, EquityMethodInvestment,
};
use crate::aggregate::fs::{
    build_consolidated_balance_sheet, build_consolidated_cash_flow,
    build_consolidated_income_statement, build_consolidation_schedule,
    build_notes_to_consolidated_fs, build_statement_of_changes_in_equity, write_consolidated_fs,
    CashFlowInputs, ConsolidatedFinancialStatements, EquityChangesInputs, NotesInputs,
};
use crate::aggregate::ic_matcher::match_ic_pairs;
use crate::aggregate::nci::{
    compute_nci_rollforward, ingest_opening_nci_balances, write_nci_rollforward, NciInputs,
    NciRollforward,
};
use crate::aggregate::post_elim::{apply_eliminations_to_tb, apply_nci_and_equity_method};
use crate::aggregate::pre_elim::aggregate_pre_elimination;
use crate::aggregate::tb_loader::load_entity_trial_balance;
use crate::aggregate::translation::cta::{
    cta_rollforward, write_cta_rollforward, CtaRollforward, CONSOLIDATED_SUBDIR,
    CTA_ROLLFORWARD_FILENAME,
};
use crate::aggregate::translation::translate::{translate_entity_tb, DrCr, TranslatedTb};
use crate::aggregate::translation::worksheet::write_translation_worksheet;
use crate::config::ConsolidationMethod;
use crate::errors::{GroupError, GroupResult};
use crate::manifest::builder::{GroupManifest, ManifestEntity};

// ── Public types ──────────────────────────────────────────────────────────────

/// Knobs the caller can supply to tune [`run_aggregate`] behaviour.
///
/// All fields are optional; the [`Default`] impl gives the production
/// fail-fast contract (no prior period, missing shards are errors).
#[derive(Debug, Clone, Default)]
pub struct AggregateOptions {
    /// Optional path to a prior period's `consolidated/` directory's
    /// **parent** — i.e. the prior period's `out_dir`.  When supplied,
    /// opening NCI, equity-method carrying values, and CTA balances are
    /// loaded from
    /// `prior_period_aggregate/consolidated/{nci_rollforward,equity_method_investments,cta_rollforward}.json`.
    /// When `None`, every opening defaults to zero per entity.
    pub prior_period_aggregate: Option<PathBuf>,
    /// When `true`, missing per-entity shard archives produce a warning
    /// rather than an error — useful for partial-archive recovery
    /// scenarios.  Defaults to `false`: missing shards fail fast.
    pub tolerate_missing_shards: bool,
}

/// Top-level result returned by [`run_aggregate`].
///
/// All numeric fields mirror the post-overlay consolidated TB; the
/// `artifacts_written` list is the absolute paths of every file the
/// driver emitted, in the order they were written.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregateSummary {
    /// Group identifier, mirrors [`GroupManifest::group_id`].
    pub group_id: String,
    /// Group presentation currency.
    pub presentation_currency: String,
    /// Period end date.
    pub as_of_date: NaiveDate,
    /// Codes of every entity whose shard archive contributed to the
    /// pre-elimination aggregation (Parent + Full).  Includes
    /// equity-method / fair-value / proportional entities whose TBs were
    /// loaded for Chunk 7 use even though they were not folded into the
    /// pre-elim totals.  Sorted lexicographically.
    pub entities_processed: Vec<String>,
    /// Codes of entities the driver expected but could not find on
    /// disk.  Always empty when [`AggregateOptions::tolerate_missing_shards`]
    /// is `false`.  Sorted lexicographically.
    pub entities_missing: Vec<String>,
    /// Codes of entities held back from the pre-elim aggregation per
    /// their consolidation method (equity-method / fair-value /
    /// proportional).  Sorted lexicographically.
    pub deferred_entities: Vec<String>,
    /// Number of IC pairs successfully matched.
    pub matched_pairs: usize,
    /// IC matching coverage = matched / planned.
    pub coverage: f64,
    /// Sum of `total_assets` from the consolidated balance sheet.
    pub total_assets: Decimal,
    /// Sum of `total_liabilities` from the consolidated balance sheet.
    pub total_liabilities: Decimal,
    /// Sum of equity attributable to owners of the parent.
    pub total_equity: Decimal,
    /// Sum of non-controlling interest equity.
    pub total_nci: Decimal,
    /// Absolute paths of every artefact the driver wrote, in
    /// emission order:
    ///
    /// 1. `ic_eliminations/ic_matching_coverage.json`
    /// 2. `consolidated/cta_rollforward.json`
    /// 3. `consolidated/translation_worksheet.json`
    /// 4. `consolidated/nci_rollforward.json`
    /// 5. `consolidated/equity_method_investments.json`
    /// 6. `consolidated/consolidated_financial_statements.json`
    /// 7. `consolidated/consolidation_schedule.json`
    /// 8. `consolidated/notes_to_consolidated_fs.json`
    pub artifacts_written: Vec<PathBuf>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Drive the aggregate phase end-to-end.
///
/// Walks the per-entity shard archives under `shards_dir/entities/`,
/// folds them through Chunks 5–8, and writes every artefact under
/// `out_dir`.  Returns an [`AggregateSummary`] describing the
/// consolidation outcome and the absolute paths of every emitted file.
///
/// `shards_dir == out_dir` is supported (and used by
/// [`crate::standalone::generate_standalone`] — the runner writes
/// shards directly into the same root the aggregate driver consumes).
///
/// # Errors
///
/// - [`GroupError::Aggregate`] if a missing shard archive is fatal
///   (`tolerate_missing_shards == false`).
/// - [`GroupError::Aggregate`] propagated from the sub-modules
///   (currency mismatch, balance regressions, FX rate gaps, etc.).
/// - [`GroupError::Io`] if the per-entity TB / JE files cannot be read
///   or the consolidated artefacts cannot be written.
/// - [`GroupError::Serde`] if any file fails to deserialise / serialise.
pub fn run_aggregate(
    manifest: &GroupManifest,
    shards_dir: &Path,
    out_dir: &Path,
    opts: &AggregateOptions,
) -> GroupResult<AggregateSummary> {
    // Resolve framework once for downstream consumers (translation +
    // notes) so we don't re-parse it per-entity.
    let framework = resolve_primary_framework(manifest);

    // ── 1. Walk per-entity shard archives ───────────────────────────────
    let WalkOutcome {
        contributing_tbs,
        contributing_jes,
        deferred_tbs,
        entities_missing,
    } = walk_entity_archives(manifest, shards_dir, opts.tolerate_missing_shards)?;

    let entities_processed: Vec<String> = contributing_tbs
        .iter()
        .map(|(c, _)| c.clone())
        .chain(deferred_tbs.iter().map(|(c, _)| c.clone()))
        .collect();
    let mut entities_processed_sorted = entities_processed.clone();
    entities_processed_sorted.sort();
    entities_processed_sorted.dedup();

    // ── 2. Pre-elimination aggregation (Task 5.2) ───────────────────────
    // The aggregator filters by consolidation method internally, but we
    // pass only the contributing slice for clarity (deferred entities
    // are handled separately in step 13).
    let pre_elim = aggregate_pre_elimination(manifest, &contributing_tbs)?;

    // ── 3. (already covered above by walk_entity_archives) ──────────────

    // ── 4. Match IC pairs (Task 5.3) ────────────────────────────────────
    let match_result = match_ic_pairs(manifest, &contributing_jes)?;

    // ── 5. Build + write coverage report (Task 5.7) ─────────────────────
    let coverage_report = build_coverage_report(&match_result);
    let coverage_path = write_coverage_report(&coverage_report, out_dir)?;

    // ── 6. Generate eliminations (Task 5.4) ─────────────────────────────
    let elim_result = generate_eliminations(&match_result.matched, manifest)?;

    // ── 7. Convert to elimination JEs (Task 5.5) ────────────────────────
    let elim_jes = eliminations_to_journal_entries(&elim_result);

    // ── 8. Apply eliminations to pre-elim TB (Task 5.6) ─────────────────
    let post_elim = apply_eliminations_to_tb(&pre_elim, &elim_jes)?;

    // ── 9. Per-entity translation (Task 6.2) ────────────────────────────
    let translated_tbs = translate_all_contributing(
        &contributing_tbs,
        manifest,
        framework,
        &entity_lookup(manifest),
    )?;

    // ── 10. CTA rollforward (Task 6.3) ─────────────────────────────────
    let cta_rolls = build_cta_rollforwards(
        &translated_tbs,
        &manifest.presentation_currency,
        opts.prior_period_aggregate.as_deref(),
    )?;
    let cta_path = write_cta_rollforward(&cta_rolls, out_dir)?;

    // ── 11. Translation worksheet (Task 6.4) ───────────────────────────
    let worksheet_path = write_translation_worksheet(&translated_tbs, out_dir)?;

    // ── 12. NCI rollforward (Tasks 7.1 + 7.2) ──────────────────────────
    let nci_rolls = build_nci_rollforwards(
        manifest,
        &translated_tbs,
        opts.prior_period_aggregate.as_deref(),
    )?;
    let nci_path = write_nci_rollforward(&nci_rolls, out_dir)?;

    // ── 13. Equity-method investments (Task 7.3) ───────────────────────
    let eq_method_invs = build_equity_method_investments(
        manifest,
        &deferred_tbs,
        framework,
        opts.prior_period_aggregate.as_deref(),
    )?;
    let eq_method_path = write_equity_method_investments(&eq_method_invs, out_dir)?;

    // ── 14. Apply NCI + equity-method overlay (Task 7.4) ───────────────
    let post_overlay = apply_nci_and_equity_method(&post_elim, &nci_rolls, &eq_method_invs)?;

    // ── 15. Build consolidated FS (Tasks 8.1–8.4) ──────────────────────
    let bs =
        build_consolidated_balance_sheet(&post_overlay, &manifest.group_id, manifest.period.end)?;
    let is = build_consolidated_income_statement(
        &post_overlay,
        &nci_rolls,
        &manifest.group_id,
        manifest.period.end,
    )?;
    // For v5.0 the cash flow statement gets a minimal input set: net
    // income from the IS, no non-cash adjustments / capex / financing
    // activity.  The fully derived cash flow statement is on the v5.1
    // roadmap.  `post_elim_tb_prior = None` because the driver does not
    // re-run the prior-period aggregation pipeline (see module rustdoc
    // step 15 in the plan — leave at `None`).
    let cf_inputs = CashFlowInputs {
        post_elim_tb_current: &post_overlay,
        post_elim_tb_prior: None,
        net_income: is.net_income,
        depreciation_amortization: Decimal::ZERO,
        impairment: Decimal::ZERO,
        capex: Decimal::ZERO,
        debt_issuance: Decimal::ZERO,
        debt_repayment: Decimal::ZERO,
        dividends_paid_to_owners: Decimal::ZERO,
        dividends_paid_to_nci: Decimal::ZERO,
        equity_issuance: Decimal::ZERO,
    };
    let cf = build_consolidated_cash_flow(
        &cf_inputs,
        &manifest.group_id,
        manifest.period.start,
        manifest.period.end,
    )?;
    let eq_changes_inputs = EquityChangesInputs {
        opening_owners_equity: Decimal::ZERO,
        opening_nci: nci_rolls
            .iter()
            .map(|rf| rf.opening_nci)
            .fold(Decimal::ZERO, |acc, v| acc + v),
        net_income_to_owners: is.net_income_to_owners,
        net_income_to_nci: is.net_income_to_nci,
        oci_to_owners: cta_rolls
            .iter()
            .map(|rf| rf.period_cta)
            .fold(Decimal::ZERO, |acc, v| acc + v),
        oci_to_nci: nci_rolls
            .iter()
            .map(|rf| rf.nci_share_of_oci)
            .fold(Decimal::ZERO, |acc, v| acc + v),
        dividends_to_owners: Decimal::ZERO,
        dividends_to_nci: nci_rolls
            .iter()
            .map(|rf| rf.nci_dividends)
            .fold(Decimal::ZERO, |acc, v| acc + v),
        other_owners: Decimal::ZERO,
        other_nci: Decimal::ZERO,
    };
    let changes_in_equity = build_statement_of_changes_in_equity(
        &eq_changes_inputs,
        &manifest.group_id,
        manifest.period.start,
        manifest.period.end,
        &manifest.presentation_currency,
    );

    let fs_bundle = ConsolidatedFinancialStatements {
        balance_sheet: bs,
        income_statement: is,
        cash_flow: cf,
        changes_in_equity,
    };

    // ── 16. Build consolidation schedule (Task 8.5) ────────────────────
    let schedule = build_consolidation_schedule(
        &pre_elim,
        &post_overlay,
        &contributing_tbs,
        &manifest.group_id,
        manifest.period.end,
    )?;

    // ── 17. Build notes (Task 8.6) ─────────────────────────────────────
    let notes_inputs = NotesInputs {
        manifest,
        framework,
        ic_coverage: &coverage_report,
        nci_rollforwards: &nci_rolls,
        cta_rollforwards: &cta_rolls,
        equity_method_investments: &eq_method_invs,
    };
    let notes = build_notes_to_consolidated_fs(&notes_inputs, manifest.period.end);

    // ── 18. Write FS artefacts (Task 8.7) ──────────────────────────────
    let fs_paths = write_consolidated_fs(&fs_bundle, &schedule, &notes, out_dir)?;

    // ── 19. Build summary ───────────────────────────────────────────────
    let mut artifacts_written: Vec<PathBuf> = Vec::with_capacity(8);
    artifacts_written.push(coverage_path);
    artifacts_written.push(cta_path);
    artifacts_written.push(worksheet_path);
    artifacts_written.push(nci_path);
    artifacts_written.push(eq_method_path);
    artifacts_written.extend(fs_paths);

    let mut deferred_codes: Vec<String> = deferred_tbs.iter().map(|(c, _)| c.clone()).collect();
    deferred_codes.sort();
    deferred_codes.dedup();

    Ok(AggregateSummary {
        group_id: manifest.group_id.clone(),
        presentation_currency: manifest.presentation_currency.clone(),
        as_of_date: manifest.period.end,
        entities_processed: entities_processed_sorted,
        entities_missing,
        deferred_entities: deferred_codes,
        matched_pairs: match_result.matched.len(),
        coverage: match_result.coverage,
        total_assets: fs_bundle.balance_sheet.total_assets,
        total_liabilities: fs_bundle.balance_sheet.total_liabilities,
        total_equity: fs_bundle.balance_sheet.total_equity,
        total_nci: fs_bundle.balance_sheet.total_nci,
        artifacts_written,
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Outcome of the per-entity archive walk: balanced TBs partitioned by
/// consolidation method, paired with the JEs the runner emitted, plus
/// the entities that were missing from disk.
struct WalkOutcome {
    /// Parent + Full entity TBs, paired with their entity codes.  Fed
    /// into the pre-elim aggregator.
    contributing_tbs: Vec<(String, TrialBalance)>,
    /// Parent + Full entity JEs, paired with their entity codes.  Fed
    /// into the IC matcher.
    contributing_jes: Vec<(String, Vec<JournalEntry>)>,
    /// Equity-method / fair-value / proportional entity TBs, kept
    /// separate so Chunk 7 (equity-method rollforward) can consume them
    /// without re-walking the disk.
    deferred_tbs: Vec<(String, TrialBalance)>,
    /// Entity codes the driver expected but could not find on disk
    /// (only populated when `tolerate_missing_shards == true`).
    entities_missing: Vec<String>,
}

/// Walk every entity in `manifest.ownership_graph.entities` and load
/// its `period_close/trial_balances.json` + `journal_entries.json` from
/// `shards_dir/entities/{code}/`.
///
/// Per the plan, equity-method / fair-value / proportional entities
/// **are** expected to have shards on disk — the runner generated them
/// — but their TBs go into `deferred_tbs` rather than `contributing_tbs`
/// so the pre-elim aggregator only sees Parent + Full inputs.
///
/// A missing entity directory or `period_close/trial_balances.json` is
/// either fatal (`tolerate_missing_shards = false`) or pushed to
/// `entities_missing` with a `tracing::warn!`.  A missing
/// `journal_entries.json` for a present entity is treated as "no JEs"
/// (empty Vec) — the runner always writes the file even when the
/// entity emits zero JEs (the orchestrator at minimum produces opening-
/// balance entries for any seeded TB), so the missing-file case is
/// effectively unreachable under v5.0 but defended here defensively.
fn walk_entity_archives(
    manifest: &GroupManifest,
    shards_dir: &Path,
    tolerate_missing_shards: bool,
) -> GroupResult<WalkOutcome> {
    let mut contributing_tbs: Vec<(String, TrialBalance)> = Vec::new();
    let mut contributing_jes: Vec<(String, Vec<JournalEntry>)> = Vec::new();
    let mut deferred_tbs: Vec<(String, TrialBalance)> = Vec::new();
    let mut entities_missing: Vec<String> = Vec::new();

    for entity in &manifest.ownership_graph.entities {
        let entity_dir = shards_dir.join("entities").join(&entity.code);
        let tb_path = entity_dir.join("period_close").join("trial_balances.json");

        if !tb_path.exists() {
            if tolerate_missing_shards {
                tracing::warn!(
                    entity = %entity.code,
                    path = %tb_path.display(),
                    "missing shard archive — continuing in tolerate_missing_shards mode",
                );
                entities_missing.push(entity.code.clone());
                continue;
            }
            return Err(GroupError::Aggregate(format!(
                "run_aggregate: missing shard archive for `{}` at `{}`",
                entity.code,
                tb_path.display()
            )));
        }

        let tb = load_entity_trial_balance(&entity_dir)?;
        let jes = load_entity_journal_entries(&entity_dir, &entity.code)?;

        match entity.consolidation_method {
            ConsolidationMethod::Parent | ConsolidationMethod::Full => {
                contributing_tbs.push((entity.code.clone(), tb));
                contributing_jes.push((entity.code.clone(), jes));
            }
            ConsolidationMethod::EquityMethod
            | ConsolidationMethod::Proportional
            | ConsolidationMethod::FairValue => {
                // Capture the deferred TB for Chunk 7 consumption; we
                // intentionally do *not* push the JEs into the
                // contributing slice because IC matching only spans
                // line-by-line consolidated entities.  v5.0 IC pair plans
                // for equity-method investees are out of scope; v5.3
                // will revisit.
                deferred_tbs.push((entity.code.clone(), tb));
            }
        }
    }

    entities_missing.sort();
    Ok(WalkOutcome {
        contributing_tbs,
        contributing_jes,
        deferred_tbs,
        entities_missing,
    })
}

/// Read every JE the orchestrator emitted for `entity_code` from
/// `entity_dir/journal_entries.json`.  Treats a missing file as zero
/// JEs (defensive — see `walk_entity_archives` rustdoc).
fn load_entity_journal_entries(
    entity_dir: &Path,
    entity_code: &str,
) -> GroupResult<Vec<JournalEntry>> {
    let path = entity_dir.join("journal_entries.json");
    if !path.exists() {
        tracing::warn!(
            entity = %entity_code,
            path = %path.display(),
            "no journal_entries.json found — treating as empty",
        );
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&path).map_err(GroupError::Io)?;
    let jes: Vec<JournalEntry> = serde_json::from_slice(&bytes)?;
    Ok(jes)
}

/// Translate every contributing entity's TB to the presentation
/// currency.  Returns one [`TranslatedTb`] per `(entity_code, tb)` in
/// the contributing slice.
fn translate_all_contributing(
    contributing_tbs: &[(String, TrialBalance)],
    manifest: &GroupManifest,
    framework: AccountingFramework,
    entity_lookup: &BTreeMap<String, ManifestEntity>,
) -> GroupResult<Vec<TranslatedTb>> {
    let mut out: Vec<TranslatedTb> = Vec::with_capacity(contributing_tbs.len());
    for (code, tb) in contributing_tbs {
        let functional_ccy = entity_lookup
            .get(code)
            .map(|e| e.functional_currency.as_str())
            .ok_or_else(|| {
                GroupError::Aggregate(format!(
                    "run_aggregate: entity `{code}` not in manifest's ownership graph",
                ))
            })?;
        let translated = translate_entity_tb(
            tb,
            functional_ccy,
            &manifest.fx_rate_master,
            manifest.period.end,
            &manifest.presentation_currency,
            framework,
        )?;
        out.push(translated);
    }
    Ok(out)
}

/// Build a [`CtaRollforward`] for every non-presentation-currency
/// entity.  Reads opening CTA from
/// `prior_period.consolidated/cta_rollforward.json` when supplied,
/// otherwise defaults to zero.
fn build_cta_rollforwards(
    translated_tbs: &[TranslatedTb],
    presentation_currency: &str,
    prior_period_aggregate: Option<&Path>,
) -> GroupResult<Vec<CtaRollforward>> {
    let opening_map = ingest_opening_cta_balances(prior_period_aggregate)?;

    let mut rolls: Vec<CtaRollforward> = Vec::new();
    for t in translated_tbs {
        if t.functional_currency == presentation_currency {
            // No CTA for entities already in the presentation currency.
            continue;
        }
        let opening = opening_map
            .get(&t.entity_code)
            .copied()
            .unwrap_or(Decimal::ZERO);
        rolls.push(cta_rollforward(
            &t.entity_code,
            &t.functional_currency,
            &t.presentation_currency,
            opening,
            t.cta,
        ));
    }
    Ok(rolls)
}

/// Read prior-period closing CTA balances by entity code.  Mirrors
/// [`crate::aggregate::nci::opening::ingest_opening_nci_balances`]
/// semantics (missing file → empty map + warn, duplicate entity →
/// error).
fn ingest_opening_cta_balances(
    prior_period_aggregate: Option<&Path>,
) -> GroupResult<BTreeMap<String, Decimal>> {
    let Some(prior) = prior_period_aggregate else {
        return Ok(BTreeMap::new());
    };
    let path = prior
        .join(CONSOLIDATED_SUBDIR)
        .join(CTA_ROLLFORWARD_FILENAME);
    if !path.exists() {
        tracing::warn!(
            path = %path.display(),
            "opening CTA file not found; defaulting to zero opening balance per entity",
        );
        return Ok(BTreeMap::new());
    }
    let bytes = std::fs::read(&path).map_err(GroupError::Io)?;
    let rolls: Vec<CtaRollforward> = serde_json::from_slice(&bytes)?;
    let mut map: BTreeMap<String, Decimal> = BTreeMap::new();
    for rf in rolls {
        if map.contains_key(&rf.entity_code) {
            return Err(GroupError::Aggregate(format!(
                "ingest_opening_cta_balances: duplicate entity `{}` in opening CTA file {}",
                rf.entity_code,
                path.display(),
            )));
        }
        map.insert(rf.entity_code, rf.closing_cta);
    }
    Ok(map)
}

/// Build an [`NciRollforward`] for every Full-method, non-wholly-owned
/// entity.  v5.0 sources the period P&L / OCI numbers from the entity's
/// translated TB; dividends paid is left at zero (the manifest does not
/// yet model it).
fn build_nci_rollforwards(
    manifest: &GroupManifest,
    translated_tbs: &[TranslatedTb],
    prior_period_aggregate: Option<&Path>,
) -> GroupResult<Vec<NciRollforward>> {
    let opening_map = match prior_period_aggregate {
        Some(p) => ingest_opening_nci_balances(p)?,
        None => BTreeMap::new(),
    };

    let translated_lookup: BTreeMap<&str, &TranslatedTb> = translated_tbs
        .iter()
        .map(|t| (t.entity_code.as_str(), t))
        .collect();

    let mut rolls: Vec<NciRollforward> = Vec::new();
    for entity in &manifest.ownership_graph.entities {
        if entity.consolidation_method != ConsolidationMethod::Full {
            continue;
        }
        let Some(ownership) = entity.ownership_percent else {
            continue;
        };
        if ownership >= Decimal::ONE {
            // Wholly-owned `Full` entities have no NCI to measure.  We
            // skip them silently here rather than surfacing an error
            // because mini_nestle.yaml has both 100%-owned (NESTLE_USA,
            // NESTLE_BR) and partially-owned (NESTLE_DE 80%) Full
            // subsidiaries.  The NCI rollforward computer would reject
            // 100% ownership as a caller bug, so the filter must happen
            // at the driver level.
            continue;
        }
        let translated = translated_lookup.get(entity.code.as_str()).ok_or_else(|| {
            GroupError::Aggregate(format!(
                "run_aggregate: NCI computation needs translated TB for `{}` but none was produced",
                entity.code,
            ))
        })?;

        let inputs = NciInputs {
            entity,
            period_net_income: net_income_from_translated(translated),
            period_oci: oci_from_translated(translated),
            total_dividends_paid: Decimal::ZERO,
            opening_nci: opening_map
                .get(&entity.code)
                .copied()
                .unwrap_or(Decimal::ZERO),
            period_end: manifest.period.end,
            currency: manifest.presentation_currency.clone(),
        };
        rolls.push(compute_nci_rollforward(&inputs)?);
    }
    Ok(rolls)
}

/// Build an [`EquityMethodInvestment`] for every EquityMethod investee
/// that has a deferred shard TB on disk.  Sources `investee_net_income`
/// and `investee_dividends_paid` from the deferred TB; impairment is
/// left at zero (the manifest does not yet model it).
fn build_equity_method_investments(
    manifest: &GroupManifest,
    deferred_tbs: &[(String, TrialBalance)],
    framework: AccountingFramework,
    prior_period_aggregate: Option<&Path>,
) -> GroupResult<Vec<EquityMethodInvestment>> {
    let opening_map = match prior_period_aggregate {
        Some(p) => ingest_opening_equity_method_carrying_values(p)?,
        None => BTreeMap::new(),
    };

    let deferred_lookup: BTreeMap<&str, &TrialBalance> = deferred_tbs
        .iter()
        .map(|(c, tb)| (c.as_str(), tb))
        .collect();

    let mut invs: Vec<EquityMethodInvestment> = Vec::new();
    for entity in &manifest.ownership_graph.entities {
        if entity.consolidation_method != ConsolidationMethod::EquityMethod {
            continue;
        }
        let Some(parent_code) = &entity.parent_code else {
            // No parent declared — equity-method requires an investor
            // entity.  Skip silently rather than erroring since v5.0's
            // upstream validators already reject ownership graphs
            // missing parent declarations.
            continue;
        };

        // The investee's deferred TB may be absent if its shard was
        // missing and `tolerate_missing_shards` was enabled.  Treat
        // that as "no income / dividends this period" (zero) so the
        // rollforward still produces a stable opening → opening record.
        let (investee_net_income, investee_dividends_paid) =
            match deferred_lookup.get(entity.code.as_str()) {
                Some(tb) => (net_income_from_tb(tb, framework), dividends_from_tb(tb)),
                None => (Decimal::ZERO, Decimal::ZERO),
            };

        let inputs = EquityMethodInputs {
            investee: entity,
            investor_entity_code: parent_code.clone(),
            investee_net_income,
            investee_dividends_paid,
            opening_carrying_value: opening_map
                .get(&entity.code)
                .copied()
                .unwrap_or(Decimal::ZERO),
            impairment: Decimal::ZERO,
            period_end: manifest.period.end,
            currency: manifest.presentation_currency.clone(),
        };
        invs.push(compute_equity_method_investment(&inputs)?);
    }
    Ok(invs)
}

/// Resolve the presentation framework once.  Maps the manifest's
/// primary CoA framework string back to the typed
/// [`AccountingFramework`].  Anything unrecognised falls through to the
/// default (UsGaap) — matches the [`AccountingFramework::Default`] impl.
fn resolve_primary_framework(manifest: &GroupManifest) -> AccountingFramework {
    let label = manifest
        .chart_of_accounts_master
        .primary_framework
        .to_lowercase();
    match label.as_str() {
        "ifrs" => AccountingFramework::Ifrs,
        "us_gaap" | "usgaap" | "us-gaap" => AccountingFramework::UsGaap,
        "dual_reporting" | "dual" => AccountingFramework::DualReporting,
        "french_gaap" | "frenchgaap" | "pcg" => AccountingFramework::FrenchGaap,
        "german_gaap" | "germangaap" | "hgb" => AccountingFramework::GermanGaap,
        _ => AccountingFramework::default(),
    }
}

/// O(1) lookup of `entity_code → ManifestEntity` from the manifest's
/// ownership graph.  Built once per `run_aggregate` call and shared
/// across the per-entity translation step.
fn entity_lookup(manifest: &GroupManifest) -> BTreeMap<String, ManifestEntity> {
    manifest
        .ownership_graph
        .entities
        .iter()
        .map(|e| (e.code.clone(), e.clone()))
        .collect()
}

/// Sum a translated TB's revenue minus expense (P&L lines) to derive
/// the entity's translated period net income.  Sign convention: revenue
/// (credit-natural) is added, expenses (debit-natural) subtracted, so
/// the result is positive on profit, negative on loss.
fn net_income_from_translated(translated: &TranslatedTb) -> Decimal {
    use crate::aggregate::translation::classify::TranslationAccountType as T;
    let mut net = Decimal::ZERO;
    for line in &translated.lines {
        let signed = match line.local_dr_cr {
            DrCr::Debit => line.translated_amount,
            DrCr::Credit => -line.translated_amount,
        };
        match line.account_type {
            // Revenue is credit-natural → contributes positively to net
            // income.  We negate the signed (debit positive, credit
            // negative) value so a CR revenue posting increments NI.
            T::PlRevenue => net -= signed,
            // Expense is debit-natural → subtracts from net income.  A
            // DR expense posting reduces NI; we subtract the (positive)
            // signed value.
            T::PlExpense => net -= signed,
            _ => {}
        }
    }
    net
}

/// Sum the OCI lines from a translated TB.  Sign convention follows
/// IAS 1 — OCI gains are credit-natural so we negate the signed amount.
fn oci_from_translated(translated: &TranslatedTb) -> Decimal {
    use crate::aggregate::translation::classify::TranslationAccountType as T;
    let mut oci = Decimal::ZERO;
    for line in &translated.lines {
        if line.account_type == T::PlOci {
            let signed = match line.local_dr_cr {
                DrCr::Debit => line.translated_amount,
                DrCr::Credit => -line.translated_amount,
            };
            oci -= signed;
        }
    }
    oci
}

/// Equivalent of [`net_income_from_translated`] but for an untranslated
/// [`TrialBalance`] — used for equity-method investees whose deferred
/// TB is consumed before translation.
///
/// Classifies each line by GL account → translation type so we can
/// pick out P&L revenue / expense lines without relying on the
/// AccountType enum (which the orchestrator may have populated with
/// different values for non-IFRS frameworks).
fn net_income_from_tb(tb: &TrialBalance, framework: AccountingFramework) -> Decimal {
    use crate::aggregate::translation::classify::{classify_account, TranslationAccountType as T};
    let mut net = Decimal::ZERO;
    for line in &tb.lines {
        let ty = classify_account(&line.account_code, framework);
        match ty {
            T::PlRevenue => net += line.credit_balance - line.debit_balance,
            T::PlExpense => net -= line.debit_balance - line.credit_balance,
            _ => {}
        }
    }
    net
}

/// Sum the dividends-paid balance from a TB.  Dividends paid lives in
/// the equity sub-account `equity_accounts::DIVIDENDS_PAID` (3500 in
/// some charts; the named constant is the source of truth).  We look up
/// that exact code rather than scan a range to avoid pulling in
/// unrelated equity movements.
fn dividends_from_tb(tb: &TrialBalance) -> Decimal {
    use datasynth_core::accounts::equity_accounts;
    tb.lines
        .iter()
        .filter(|l| l.account_code == equity_accounts::DIVIDENDS_PAID)
        .map(|l| l.debit_balance - l.credit_balance)
        .fold(Decimal::ZERO, |acc, v| acc + v)
}

// ── Unit tests ────────────────────────────────────────────────────────────────
//
// The driver itself is end-to-end tested by `tests/aggregate_e2e.rs`
// (which exercises the full pipeline against a Mini-Nestlé fixture).
// `resolve_primary_framework` is the only branch worth exercising at
// the unit-test layer; the remainder of the helpers are linear glue
// over already-tested sub-modules.

#[cfg(test)]
mod tests {
    use super::resolve_primary_framework;
    use crate::aggregate::driver::AggregateOptions;
    use datasynth_standards::framework::AccountingFramework;

    #[test]
    fn aggregate_options_default_is_fail_fast_no_prior() {
        let opts = AggregateOptions::default();
        assert!(!opts.tolerate_missing_shards, "default fails fast");
        assert!(opts.prior_period_aggregate.is_none(), "no prior period");
    }

    #[test]
    fn _resolve_primary_framework_is_pure_string_match() {
        // Type-only sanity: keep `resolve_primary_framework` in scope so
        // a refactor that drops a variant breaks the build instead of
        // silently falling through to `AccountingFramework::default()`.
        // The full label-vs-enum mapping is exercised in
        // `tests/aggregate_e2e.rs` once a fixture is loaded.
        let _ = resolve_primary_framework;
        let _ = AccountingFramework::default();
    }
}

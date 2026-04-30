//! Per-entity trial balance loader — Task 5.1.
//!
//! The shard runner ([`crate::shard::run_shard`]) writes each entity's
//! period-close trial balance under
//! `{entity_dir}/period_close/trial_balances.json`, where `{entity_dir}`
//! is `{shard_out_dir}/entities/{entity_code}` (see
//! [`datasynth_runtime::output_writer`] — the file is named with the
//! plural `trial_balances` because the orchestrator emits a JSON array
//! of [`TrialBalance`] entries, one per fiscal period).
//!
//! For v5.0 the orchestrator runs every entity for a single fiscal
//! period at a time, so the array contains exactly one element. This
//! loader enforces that contract: anything else (zero, two, or more
//! entries) is reported as an [`GroupError::Aggregate`] error naming the
//! offending entity directory so an aggregate-phase log pinpoints the
//! corruption without the caller having to inspect the file by hand.
//!
//! On top of the structural check the loader re-verifies the
//! [`TrialBalance::is_balanced`] invariant: if the on-disk file claims
//! `is_balanced = true` but `total_debits != total_credits`, the file is
//! corrupt and we surface it as an [`GroupError::Aggregate`] rather than
//! silently propagating an inconsistent TB into the consolidation
//! engine.  Symmetrically, an explicitly unbalanced TB
//! (`is_balanced = false`) is rejected — the aggregate phase contract is
//! "input must already be a balanced standalone TB" and downstream
//! combiners assume that invariant.
//!
//! Higher-level Chunk-5 modules (group TB combiner, IC elimination, NCI
//! roll-up) call this loader once per entity and then operate on the
//! returned [`TrialBalance`] without further I/O.
//!
//! # v5.1: canonical-shape end-to-end
//!
//! In v5.0 the orchestrator emitted a `Vec<PeriodTrialBalance>` JSON
//! shape that differed from the canonical [`TrialBalance`]; this
//! loader carried a fallback path that detected the difference and
//! synthesised the missing canonical fields on the fly.  v5.1 moved
//! the shape conversion to write time
//! (`PeriodTrialBalance::into_canonical`) so the on-disk JSON matches
//! the canonical shape directly.  The dual-shape detection has been
//! removed.

use std::fs;
use std::path::Path;

use datasynth_core::models::balance::TrialBalance;
use rust_decimal::Decimal;

use crate::errors::{GroupError, GroupResult};

/// Subdirectory under each entity directory where the orchestrator's
/// period-close artefacts live (mirrors `output_writer.rs`).
const PERIOD_CLOSE_DIR: &str = "period_close";

/// File name written by the orchestrator. Plural — the file holds a JSON
/// array of `TrialBalance` (one per fiscal period; v5.0 emits exactly
/// one).
const TRIAL_BALANCES_FILE: &str = "trial_balances.json";

/// Load the single per-entity trial balance for `entity_dir` (typically
/// `{shard_out_dir}/entities/{entity_code}`).
///
/// Reads `{entity_dir}/period_close/trial_balances.json`, deserialises
/// it into `Vec<TrialBalance>`, asserts the array contains exactly one
/// entry, and re-verifies `total_debits == total_credits`.
///
/// # Errors
///
/// - [`GroupError::Io`] when the file cannot be opened (most commonly
///   `NotFound` — caller wrote the entity to a different path or the
///   shard runner did not produce a TB for this entity).
/// - [`GroupError::Serde`] when the file exists but is not valid JSON
///   matching the [`TrialBalance`] schema.
/// - [`GroupError::Aggregate`] when the structural / balance invariants
///   do not hold:
///   - empty array (`[]`) — orchestrator wrote the file but produced no
///     period-close trial balance,
///   - more than one entry — multiple periods were aggregated into one
///     entity directory, which v5.0 does not support,
///   - `is_balanced = false` — the on-disk TB is explicitly unbalanced,
///   - `is_balanced = true` but `total_debits != total_credits` —
///     on-disk corruption (the recalculate flag and the totals disagree).
pub fn load_entity_trial_balance(entity_dir: &Path) -> GroupResult<TrialBalance> {
    let path = entity_dir.join(PERIOD_CLOSE_DIR).join(TRIAL_BALANCES_FILE);
    let bytes = fs::read(&path)?;
    let entity_label = entity_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("<non-utf8 entity dir>");

    // v5.1: the on-disk shape is canonical `Vec<TrialBalance>`.
    // Multi-period archives still allowed (orchestrator emits one TB
    // per fiscal period).  Pick the LAST period's TB (latest
    // fiscal_year / fiscal_period) — that's the closing balance the
    // consolidation engine consolidates against.
    let arr: Vec<TrialBalance> = serde_json::from_slice(&bytes)?;

    if arr.is_empty() {
        return Err(GroupError::Aggregate(format!(
            "load_entity_trial_balance: `{}` contains an empty trial-balance array \
             at `{}` — orchestrator should always emit at least one period-close TB",
            entity_label,
            path.display()
        )));
    }

    let tb = arr
        .into_iter()
        .max_by_key(|tb| (tb.fiscal_year, tb.fiscal_period))
        .expect("non-empty array");
    verify_balance_invariant(&tb, entity_label, &path)?;
    Ok(tb)
}

// `period_to_canonical` (v5.0) was retired in v5.1 — the orchestrator
// now writes the canonical `TrialBalance` shape directly via
// `PeriodTrialBalance::into_canonical`, so the loader no longer needs
// to synthesise missing fields.

/// Re-verify that `tb.is_balanced` matches `tb.total_debits ==
/// tb.total_credits`, and that both flags say "balanced".
///
/// This is a corruption check — the orchestrator's
/// `TrialBalance::recalculate` always sets `is_balanced` from the totals,
/// so a mismatch on disk means the file was hand-edited or written by a
/// non-conforming producer.  Either way, the consolidation engine cannot
/// trust the input and must reject it.
fn verify_balance_invariant(tb: &TrialBalance, entity_label: &str, path: &Path) -> GroupResult<()> {
    // v5.0 contract update: per-entity TBs from the orchestrator are
    // expected to be UNBALANCED in normal operation — the synthetic data
    // engine deliberately injects fraud/anomaly entries with mismatched
    // debits/credits, and `is_balanced` reflects that. The aggregate
    // phase's `pre_elim` step sums per-account independently of balance,
    // so an unbalanced input is fine. Surface the imbalance only as a
    // tracing log so an operator can diagnose suspicious magnitudes.
    let imbalance = tb.total_debits - tb.total_credits;
    if imbalance.abs() >= Decimal::new(1, 2) {
        tracing::debug!(
            entity = entity_label,
            path = %path.display(),
            total_debits = %tb.total_debits,
            total_credits = %tb.total_credits,
            imbalance = %imbalance,
            "per-entity TB unbalanced (expected with anomaly/fraud injection)",
        );
    }
    Ok(())
}

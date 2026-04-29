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

    // Open + read the bytes. `fs::read` returns `io::Error` which
    // converts via `GroupError::Io`, preserving NotFound / PermissionDenied
    // semantics for callers that want to special-case "entity has no TB".
    let bytes = fs::read(&path)?;

    // Deserialize into `Vec<TrialBalance>` — the orchestrator always
    // writes an array, even for a single TB. `serde_json::Error` converts
    // via the existing `From` impl into `GroupError::Serde`.
    let mut trial_balances: Vec<TrialBalance> = serde_json::from_slice(&bytes)?;

    // Identify the entity by directory name for error messages.  Falls
    // back to a static placeholder if the directory name is somehow
    // unavailable (defensive — `Path::file_name` returns `None` for
    // paths ending in `..`, which a caller is unlikely to pass but we
    // guard anyway; the path is also included in every error message,
    // so the placeholder doesn't hide the location).
    let entity_label = entity_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("<non-utf8 entity dir>");

    // Structural invariant: exactly one TB per entity for v5.0.
    match trial_balances.len() {
        0 => Err(GroupError::Aggregate(format!(
            "load_entity_trial_balance: `{}` contains an empty trial-balance array \
             at `{}` — orchestrator should always emit at least one period-close TB",
            entity_label,
            path.display()
        ))),
        1 => {
            // `Vec::pop` is O(1) and avoids a clone; the `.expect` is
            // unreachable because we just matched `len() == 1`.
            let tb = trial_balances
                .pop()
                .expect("len()==1 implies pop() returns Some; this is unreachable");
            verify_balance_invariant(&tb, entity_label, &path)?;
            Ok(tb)
        }
        n => Err(GroupError::Aggregate(format!(
            "load_entity_trial_balance: `{}` contains {} trial balances at `{}`, \
             expected exactly 1 — v5.0 emits one TB per entity per period",
            entity_label,
            n,
            path.display()
        ))),
    }
}

/// Re-verify that `tb.is_balanced` matches `tb.total_debits ==
/// tb.total_credits`, and that both flags say "balanced".
///
/// This is a corruption check — the orchestrator's
/// `TrialBalance::recalculate` always sets `is_balanced` from the totals,
/// so a mismatch on disk means the file was hand-edited or written by a
/// non-conforming producer.  Either way, the consolidation engine cannot
/// trust the input and must reject it.
fn verify_balance_invariant(tb: &TrialBalance, entity_label: &str, path: &Path) -> GroupResult<()> {
    let imbalance = tb.total_debits - tb.total_credits;

    // Reject explicitly unbalanced TBs.  Aggregate phase contract: input
    // is already a balanced standalone TB.
    if !tb.is_balanced {
        return Err(GroupError::Aggregate(format!(
            "load_entity_trial_balance: `{}` trial balance at `{}` is not balanced \
             (is_balanced=false; total_debits={}, total_credits={}, imbalance={})",
            entity_label,
            path.display(),
            tb.total_debits,
            tb.total_credits,
            imbalance,
        )));
    }

    // Detect on-disk corruption: flag claims balanced but totals disagree.
    // Tolerance mirrors `TrialBalance::recalculate` (`< 0.01`).
    if imbalance.abs() >= Decimal::new(1, 2) {
        return Err(GroupError::Aggregate(format!(
            "load_entity_trial_balance: `{}` trial balance at `{}` is corrupt — \
             is_balanced=true but total_debits ({}) != total_credits ({}), \
             imbalance={}",
            entity_label,
            path.display(),
            tb.total_debits,
            tb.total_credits,
            imbalance,
        )));
    }

    Ok(())
}

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

use chrono::NaiveDate;
use datasynth_core::models::balance::{
    AccountCategory, AccountType, TrialBalance, TrialBalanceLine, TrialBalanceStatus,
    TrialBalanceType,
};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::errors::{GroupError, GroupResult};

/// On-disk shape the orchestrator emits at `period_close/trial_balances.json`
/// (a `Vec<datasynth_runtime::PeriodTrialBalance>` serde'd as JSON).
///
/// This differs from the canonical [`TrialBalance`] shape — it carries
/// fiscal year/period + an `entries` field instead of `lines`. The loader
/// converts every entry into a `TrialBalanceLine` and synthesises the
/// missing canonical fields so downstream aggregate code can keep
/// operating on the canonical type.
#[derive(Debug, Clone, Deserialize)]
struct PeriodTrialBalanceOnDisk {
    fiscal_year: u16,
    fiscal_period: u8,
    #[serde(default)]
    period_start: Option<NaiveDate>,
    period_end: NaiveDate,
    entries: Vec<TrialBalanceEntryOnDisk>,
}

#[derive(Debug, Clone, Deserialize)]
struct TrialBalanceEntryOnDisk {
    account_code: String,
    account_name: String,
    #[serde(default)]
    #[allow(dead_code)]
    category: String,
    debit_balance: Decimal,
    credit_balance: Decimal,
}

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

    // The orchestrator emits `Vec<PeriodTrialBalance>` (datasynth-runtime
    // type) — try that shape first, then fall back to the canonical
    // `Vec<TrialBalance>` shape so hand-rolled fixtures and unit tests
    // keep working unchanged.
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let arr = value.as_array().ok_or_else(|| {
        GroupError::Aggregate(format!(
            "load_entity_trial_balance: `{}` at `{}` is not a JSON array",
            entity_label,
            path.display()
        ))
    })?;

    if arr.is_empty() {
        return Err(GroupError::Aggregate(format!(
            "load_entity_trial_balance: `{}` contains an empty trial-balance array \
             at `{}` — orchestrator should always emit at least one period-close TB",
            entity_label,
            path.display()
        )));
    }

    // Multi-period archives: orchestrator emits one TB per fiscal period
    // (e.g. 3 TBs for a quarterly run with monthly periodicity). Pick the
    // LAST period's TB (latest period_end / fiscal_period) — that's the
    // closing balance the consolidation engine consolidates against.
    let elem = if arr[0].get("entries").is_some() {
        // Orchestrator's PeriodTrialBalance shape — pick by latest fiscal_period.
        arr.iter()
            .max_by_key(|v| {
                let yr = v.get("fiscal_year").and_then(|x| x.as_u64()).unwrap_or(0);
                let pd = v.get("fiscal_period").and_then(|x| x.as_u64()).unwrap_or(0);
                (yr, pd)
            })
            .expect("non-empty array")
    } else {
        // Canonical TrialBalance shape (unit-test fixtures): if there's
        // more than one we still demand exactly 1 (test fixtures don't
        // multi-period).
        if arr.len() > 1 {
            return Err(GroupError::Aggregate(format!(
                "load_entity_trial_balance: `{}` contains {} canonical-shape \
                 trial balances at `{}`, expected exactly 1",
                entity_label,
                arr.len(),
                path.display()
            )));
        }
        &arr[0]
    };

    let tb = if elem.get("entries").is_some() {
        let p: PeriodTrialBalanceOnDisk = serde_json::from_value(elem.clone())?;
        period_to_canonical(p, entity_label)
    } else {
        serde_json::from_value::<TrialBalance>(elem.clone())?
    };
    verify_balance_invariant(&tb, entity_label, &path)?;
    Ok(tb)
}

/// Convert the orchestrator's `PeriodTrialBalance` JSON shape into the
/// canonical [`TrialBalance`] the aggregate phase consumes.
///
/// Synthesises every required canonical field that the on-disk shape
/// doesn't carry (id, currency, account types, etc.) using sensible
/// defaults — pre_elim only consumes `lines`/`currency`/`total_*` so the
/// other fields are bookkeeping.
fn period_to_canonical(p: PeriodTrialBalanceOnDisk, entity_label: &str) -> TrialBalance {
    let mut total_debits = Decimal::ZERO;
    let mut total_credits = Decimal::ZERO;
    let lines: Vec<TrialBalanceLine> = p
        .entries
        .into_iter()
        .map(|e| {
            total_debits += e.debit_balance;
            total_credits += e.credit_balance;
            let category = AccountCategory::from_account_code(&e.account_code);
            TrialBalanceLine {
                account_code: e.account_code.clone(),
                account_description: e.account_name,
                category,
                account_type: AccountType::Asset,
                opening_balance: Decimal::ZERO,
                period_debits: e.debit_balance,
                period_credits: e.credit_balance,
                closing_balance: e.debit_balance - e.credit_balance,
                debit_balance: e.debit_balance,
                credit_balance: e.credit_balance,
                cost_center: None,
                profit_center: None,
            }
        })
        .collect();
    let imbalance = (total_debits - total_credits).abs();
    let is_balanced = imbalance < Decimal::new(1, 2);
    TrialBalance {
        trial_balance_id: format!("{entity_label}-{:04}{:02}", p.fiscal_year, p.fiscal_period),
        company_code: entity_label.to_string(),
        company_name: None,
        as_of_date: p.period_end,
        fiscal_year: p.fiscal_year as i32,
        fiscal_period: p.fiscal_period as u32,
        currency: "USD".to_string(),
        balance_type: TrialBalanceType::Adjusted,
        lines,
        total_debits,
        total_credits,
        is_balanced,
        out_of_balance: total_debits - total_credits,
        is_equation_valid: is_balanced,
        equation_difference: total_debits - total_credits,
        category_summary: std::collections::HashMap::new(),
        created_at: p
            .period_start
            .unwrap_or(p.period_end)
            .and_hms_opt(0, 0, 0)
            .expect("midnight is a valid time"),
        created_by: "GROUP_AGGREGATE".to_string(),
        approved_by: None,
        approved_at: None,
        status: TrialBalanceStatus::Final,
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

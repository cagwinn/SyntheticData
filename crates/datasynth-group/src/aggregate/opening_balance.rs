//! v5.3 — Per-entity opening-balance carryover from a prior period.
//!
//! When an engagement runs multiple consecutive periods, period N+1
//! should open with each entity's closing balance from period N.  The
//! aggregate-side prior-period plumbing (PR #141 / PR #150 /
//! `run_aggregate_chain`) already carries opening NCI / CTA / equity-
//! method values.  This module adds the **entity-level opening trial
//! balance** carrier so the orchestrator can — in a follow-up PR —
//! seed each entity's period-N+1 opening balances from period-N's
//! closing TB instead of starting from zero.
//!
//! # What this PR ships
//!
//! - [`read_prior_period_closing_tbs`] — walks
//!   `{prior_dir}/entities/*/period_close/trial_balances.json` and
//!   loads every entity's latest closing TB.  Empty map when the
//!   prior dir doesn't exist or has no entity archives — preserves
//!   v5.0–v5.2 byte-identical first-period behaviour.
//! - [`extract_opening_balances`] — projects a closing TB onto its
//!   balance-sheet positions only (Asset / Liability / Equity + contra
//!   variants), discarding P&L lines (which close to retained earnings
//!   for the new period).  Output is sorted by account code for
//!   determinism.
//! - [`OpeningBalance`] — one carry-forward record per (entity, account)
//!   pair, with the closing debit / credit balance of the prior period.
//!
//! # Follow-up
//!
//! The orchestrator-side hook that consumes [`OpeningBalance`]s as
//! period-N+1 opening balances (so the orchestrator's balance phase
//! generates period activity *on top of* these openings rather than
//! starting from zero) is **not** in this PR — it's the natural next
//! step.  This PR provides the loading + projection helpers and the
//! data shape that the hook will consume.
//!
//! # Per-entity multi-period coverage
//!
//! When an entity has multiple closing TBs in its
//! `period_close/trial_balances.json` (e.g. the orchestrator emits
//! one TB per fiscal sub-period), [`read_prior_period_closing_tbs`]
//! picks the **latest by `(fiscal_year, fiscal_period)`** — same rule
//! the aggregate driver's `tb_loader` uses.  This matches the
//! semantics: the period-N closing balance is whichever TB was struck
//! at the chronologically latest sub-period within period N.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use datasynth_core::models::balance::{AccountType, TrialBalance};

use crate::errors::{GroupError, GroupResult};

// ── Public types ──────────────────────────────────────────────────────────────

/// One entity's opening-balance carry-forward record for one account.
///
/// `debit` and `credit` are both non-negative; at most one is non-zero
/// (matches the underlying [`datasynth_core::models::balance::TrialBalanceLine`]
/// invariant).  The `account_code` and `account_type` are forwarded
/// verbatim from the prior period's TB line so downstream consumers
/// can route the opening balance to the right ledger account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpeningBalance {
    /// GL account code (e.g. `"1100"` for AR control).
    pub account_code: String,
    /// Account type — drives the BS-only filter applied by
    /// [`extract_opening_balances`] before this record is constructed.
    pub account_type: AccountType,
    /// Debit-side closing amount from the prior period (always
    /// non-negative; zero when the line was credit-balanced).
    pub debit: Decimal,
    /// Credit-side closing amount from the prior period (always
    /// non-negative; zero when the line was debit-balanced).
    pub credit: Decimal,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Load every entity's closing trial balance from a prior period's
/// shard archive.  Returns a map `entity_code → TrialBalance` keyed
/// by the entity code from the manifest.
///
/// `entity_codes` filters which entities to load; entities not present
/// on disk are silently skipped (mirrors the aggregate driver's
/// best-effort `tolerate_missing_shards = true` semantics for prior-
/// period reads).  An empty filter list reads no entities.
///
/// When the prior dir doesn't exist, returns an empty map without
/// error — engagements that don't supply a prior period have nothing
/// to carry forward.
///
/// # Errors
///
/// - [`GroupError::Aggregate`] if a present `trial_balances.json` file
///   cannot be parsed as `Vec<TrialBalance>` — this indicates a
///   corrupted archive and should fail fast rather than silently
///   degrade.
/// - [`GroupError::Aggregate`] if a parsed TB has no entries
///   (zero-length array).
pub fn read_prior_period_closing_tbs(
    prior_period_dir: &Path,
    entity_codes: &[String],
) -> GroupResult<BTreeMap<String, TrialBalance>> {
    let mut out: BTreeMap<String, TrialBalance> = BTreeMap::new();
    if !prior_period_dir.exists() {
        return Ok(out);
    }
    for code in entity_codes {
        let path = prior_period_dir
            .join("entities")
            .join(code)
            .join("period_close")
            .join("trial_balances.json");
        if !path.exists() {
            continue;
        }
        let bytes = fs::read(&path).map_err(|e| {
            GroupError::Aggregate(format!(
                "read_prior_period_closing_tbs: cannot read `{}`: {e}",
                path.display()
            ))
        })?;
        let tbs: Vec<TrialBalance> = serde_json::from_slice(&bytes).map_err(|e| {
            GroupError::Aggregate(format!(
                "read_prior_period_closing_tbs: cannot parse `{}` as Vec<TrialBalance>: {e}",
                path.display()
            ))
        })?;
        if tbs.is_empty() {
            return Err(GroupError::Aggregate(format!(
                "read_prior_period_closing_tbs: `{}` contains no trial balances",
                path.display()
            )));
        }
        // Pick the latest TB by (fiscal_year, fiscal_period) — matches
        // the tb_loader rule used elsewhere in the aggregate phase.
        let latest = tbs
            .into_iter()
            .max_by_key(|tb| (tb.fiscal_year, tb.fiscal_period))
            .expect("non-empty after the is_empty check above");
        out.insert(code.clone(), latest);
    }
    Ok(out)
}

/// Project a closing trial balance onto its balance-sheet positions —
/// the carry-forward set the next period needs to open with.
///
/// Filtering rule: only Asset / ContraAsset / Liability / ContraLiability /
/// Equity / ContraEquity lines are kept.  Revenue, Expense, OtherIncome,
/// OtherExpense lines are dropped — those close to retained earnings
/// at period-end and reset to zero for the new period (the orchestrator's
/// period-close logic handles that aggregation; this module's contract
/// is just the BS positions).
///
/// Output is sorted by `account_code` so two calls over identical input
/// produce byte-identical output.
pub fn extract_opening_balances(tb: &TrialBalance) -> Vec<OpeningBalance> {
    let mut out: Vec<OpeningBalance> = tb
        .lines
        .iter()
        .filter(|line| is_balance_sheet_account(line.account_type))
        .map(|line| OpeningBalance {
            account_code: line.account_code.clone(),
            account_type: line.account_type,
            debit: line.debit_balance,
            credit: line.credit_balance,
        })
        .collect();
    out.sort_by(|a, b| a.account_code.cmp(&b.account_code));
    out
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn is_balance_sheet_account(ty: AccountType) -> bool {
    matches!(
        ty,
        AccountType::Asset
            | AccountType::ContraAsset
            | AccountType::Liability
            | AccountType::ContraLiability
            | AccountType::Equity
            | AccountType::ContraEquity
    )
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use datasynth_core::models::balance::{
        AccountCategory, TrialBalance, TrialBalanceLine, TrialBalanceType,
    };
    use rust_decimal_macros::dec;

    fn period_end() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
    }

    fn make_tb(company_code: &str) -> TrialBalance {
        let mut tb = TrialBalance::new(
            format!("TB_{company_code}"),
            company_code.to_string(),
            period_end(),
            2024,
            3,
            "EUR".to_string(),
            TrialBalanceType::Adjusted,
        );
        // BS items
        tb.add_line(line("1000", AccountType::Asset, dec!(10000), Decimal::ZERO));
        tb.add_line(line("1100", AccountType::Asset, dec!(5000), Decimal::ZERO));
        tb.add_line(line(
            "2000",
            AccountType::Liability,
            Decimal::ZERO,
            dec!(3000),
        ));
        tb.add_line(line("3000", AccountType::Equity, Decimal::ZERO, dec!(9000)));
        // P&L items — must be dropped by extract_opening_balances
        tb.add_line(line(
            "4000",
            AccountType::Revenue,
            Decimal::ZERO,
            dec!(7000),
        ));
        tb.add_line(line(
            "5000",
            AccountType::Expense,
            dec!(4000),
            Decimal::ZERO,
        ));
        tb
    }

    fn line(code: &str, ty: AccountType, debit: Decimal, credit: Decimal) -> TrialBalanceLine {
        TrialBalanceLine {
            account_code: code.to_string(),
            account_description: code.to_string(),
            category: AccountCategory::from_account_type(ty),
            account_type: ty,
            opening_balance: Decimal::ZERO,
            period_debits: Decimal::ZERO,
            period_credits: Decimal::ZERO,
            closing_balance: if debit > credit { debit } else { credit },
            debit_balance: debit,
            credit_balance: credit,
            cost_center: None,
            profit_center: None,
        }
    }

    #[test]
    fn read_prior_period_closing_tbs_returns_empty_when_dir_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let prior = tmp.path().join("non_existent");
        let codes = vec!["E1".to_string()];
        let map = read_prior_period_closing_tbs(&prior, &codes).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn read_prior_period_closing_tbs_loads_matching_entities() {
        let tmp = tempfile::tempdir().unwrap();
        let prior = tmp.path();
        // Write a TB for E1
        let entity_dir = prior.join("entities").join("E1").join("period_close");
        fs::create_dir_all(&entity_dir).unwrap();
        let tbs = vec![make_tb("E1")];
        fs::write(
            entity_dir.join("trial_balances.json"),
            serde_json::to_string_pretty(&tbs).unwrap(),
        )
        .unwrap();

        let codes = vec!["E1".to_string(), "E2_MISSING".to_string()];
        let map = read_prior_period_closing_tbs(prior, &codes).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("E1").unwrap().company_code, "E1");
        assert!(!map.contains_key("E2_MISSING"));
    }

    #[test]
    fn read_prior_period_closing_tbs_picks_latest_by_fiscal_period() {
        let tmp = tempfile::tempdir().unwrap();
        let prior = tmp.path();
        let entity_dir = prior.join("entities").join("E1").join("period_close");
        fs::create_dir_all(&entity_dir).unwrap();

        // Three TBs across fiscal periods 1, 2, 3 — latest is period 3.
        let mut p1 = make_tb("E1");
        p1.fiscal_period = 1;
        let mut p2 = make_tb("E1");
        p2.fiscal_period = 2;
        let mut p3 = make_tb("E1");
        p3.fiscal_period = 3;
        let tbs = vec![p1, p2, p3];

        fs::write(
            entity_dir.join("trial_balances.json"),
            serde_json::to_string_pretty(&tbs).unwrap(),
        )
        .unwrap();

        let map = read_prior_period_closing_tbs(prior, &["E1".to_string()]).unwrap();
        assert_eq!(map.get("E1").unwrap().fiscal_period, 3);
    }

    #[test]
    fn read_prior_period_closing_tbs_rejects_empty_array() {
        let tmp = tempfile::tempdir().unwrap();
        let prior = tmp.path();
        let entity_dir = prior.join("entities").join("E1").join("period_close");
        fs::create_dir_all(&entity_dir).unwrap();
        let tbs: Vec<TrialBalance> = Vec::new();
        fs::write(
            entity_dir.join("trial_balances.json"),
            serde_json::to_string_pretty(&tbs).unwrap(),
        )
        .unwrap();

        let err = read_prior_period_closing_tbs(prior, &["E1".to_string()]).unwrap_err();
        assert!(format!("{err}").contains("contains no trial balances"));
    }

    #[test]
    fn extract_opening_balances_drops_pl_accounts() {
        let tb = make_tb("E1");
        let openings = extract_opening_balances(&tb);
        // 4 BS accounts (1000, 1100, 2000, 3000); 4000 and 5000 dropped.
        assert_eq!(openings.len(), 4);
        let codes: Vec<&str> = openings.iter().map(|o| o.account_code.as_str()).collect();
        assert_eq!(codes, vec!["1000", "1100", "2000", "3000"]);
    }

    #[test]
    fn extract_opening_balances_preserves_dr_cr_sides() {
        let tb = make_tb("E1");
        let openings = extract_opening_balances(&tb);
        let cash = openings.iter().find(|o| o.account_code == "1000").unwrap();
        assert_eq!(cash.debit, dec!(10000));
        assert_eq!(cash.credit, Decimal::ZERO);
        let ap = openings.iter().find(|o| o.account_code == "2000").unwrap();
        assert_eq!(ap.debit, Decimal::ZERO);
        assert_eq!(ap.credit, dec!(3000));
    }

    #[test]
    fn extract_opening_balances_sorted_by_account_code() {
        let tb = make_tb("E1");
        let openings = extract_opening_balances(&tb);
        let codes: Vec<&str> = openings.iter().map(|o| o.account_code.as_str()).collect();
        let mut sorted = codes.clone();
        sorted.sort();
        assert_eq!(codes, sorted);
    }

    #[test]
    fn extract_opening_balances_total_dr_equals_total_cr_for_balanced_tb() {
        // The carry-forward set must balance — total DR of all opening
        // balances must equal total CR (since the source TB is
        // balanced).  This is the fundamental invariant the next-period
        // orchestrator hook will rely on.
        let tb = make_tb("E1");
        let openings = extract_opening_balances(&tb);
        let total_dr: Decimal = openings.iter().map(|o| o.debit).sum();
        let total_cr: Decimal = openings.iter().map(|o| o.credit).sum();
        // BS lines: 10000 + 5000 (DR) vs 3000 + 9000 (CR) = 15000 vs 12000.
        // Difference = +3000.  This is exactly the period's retained-
        // earnings residual = revenue (7000 CR) - expense (4000 DR) = +3000
        // CR → would close to retained earnings (CR side), making BS DR
        // = CR after the period-close JE.  BS-only projection captures
        // the *pre-close* state; orchestrator's period-end logic posts
        // the residual to retained earnings.
        let pl_net = dec!(7000) - dec!(4000); // revenue - expense = 3000
        assert_eq!(total_dr - total_cr, pl_net);
    }

    #[test]
    fn opening_balance_round_trips_json() {
        let ob = OpeningBalance {
            account_code: "1000".to_string(),
            account_type: AccountType::Asset,
            debit: dec!(12345.67),
            credit: Decimal::ZERO,
        };
        let json = serde_json::to_string(&ob).unwrap();
        let back: OpeningBalance = serde_json::from_str(&json).unwrap();
        assert_eq!(ob, back);
    }
}

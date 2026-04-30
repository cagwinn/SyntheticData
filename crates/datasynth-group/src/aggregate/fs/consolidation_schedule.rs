//! Consolidation schedule — Task 8.5.
//!
//! Walks the union of accounts across the pre- and post-elimination
//! consolidated trial balances and emits a per-account schedule showing
//! the contribution of each entity, the pre-elimination total, the
//! elimination adjustments, and the post-elimination total.
//!
//! # Layout
//!
//! Per spec §"Consolidated FS outputs":
//!
//! | Column                  | Source                                            |
//! |-------------------------|---------------------------------------------------|
//! | `account_category`      | Coarse bucket (Asset / Liability / Equity / etc.) |
//! | `account_code`          | GL code                                           |
//! | `entity_amounts`        | Per-entity contribution from the standalone TBs   |
//! | `pre_elimination_total` | `pre_elim_tb.account_totals[code].net_balance`    |
//! | `elimination_adjustments` | `post.net_balance - pre.net_balance`            |
//! | `post_elimination_total` | `post_elim_tb.account_totals[code].net_balance`  |
//!
//! Lines are sorted by `account_code` for deterministic output.
//!
//! # Edge cases
//!
//! - **Account only in eliminations** (e.g., the equity-method overlay
//!   posting to `3300` retained earnings — was `3400` in v5.0) —
//!   appears with `pre_elimination_total = 0` and
//!   `elimination_adjustments = post - 0 = post`.
//! - **Account fully eliminated** (e.g., IC AR `1150` after
//!   elimination) — appears with `pre_elimination_total = nonzero`,
//!   `post_elimination_total = 0`, `elimination_adjustments = -pre`.
//! - **Account in entity TBs but missing from consolidated TBs** —
//!   should never happen given the upstream contracts, but defended
//!   here by treating missing post / pre as zero.
//!
//! # `account_category`
//!
//! Coarse bucket derived from the leading digit (Asset / Liability /
//! Equity / Revenue / Expense / Other).  Aligned with the BS / IS
//! sectioning in [`super::balance_sheet`] and [`super::income_statement`]
//! but stays at the top-level category granularity for readability.

use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use datasynth_core::models::balance::TrialBalance;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::aggregate::pre_elim::AggregatedTb;
use crate::errors::GroupResult;

// ── Public types ──────────────────────────────────────────────────────────────

/// Consolidation schedule — one line per GL account showing pre /
/// adjustment / post.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationSchedule {
    /// Group identifier.
    pub group_id: String,
    /// As-of date (period end).
    pub as_of_date: NaiveDate,
    /// Presentation currency.
    pub currency: String,
    /// One line per GL account, sorted by `account_code`.
    pub lines: Vec<ScheduleLine>,
}

/// One line in the consolidation schedule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleLine {
    /// Coarse category — Asset / Liability / Equity / Revenue / Expense / Other.
    pub account_category: String,
    /// GL account code.
    pub account_code: String,
    /// Per-entity contribution from the standalone TBs (entity_code → amount).
    pub entity_amounts: BTreeMap<String, Decimal>,
    /// `pre_elim_tb.account_totals[code].net_balance` (zero if missing).
    pub pre_elimination_total: Decimal,
    /// `post - pre` — the consolidation adjustment for this account.
    pub elimination_adjustments: Decimal,
    /// `post_elim_tb.account_totals[code].net_balance` (zero if missing).
    pub post_elimination_total: Decimal,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Build the consolidation schedule from pre- and post-elimination
/// TBs plus the per-entity standalone TBs.
///
/// Pure function: no I/O, no allocation beyond the schedule itself,
/// no dependence on global state.  Two calls with the same inputs
/// produce equal records.
pub fn build_consolidation_schedule(
    pre_elim_tb: &AggregatedTb,
    post_elim_tb: &AggregatedTb,
    entity_tbs: &[(String, TrialBalance)],
    group_id: &str,
    as_of_date: NaiveDate,
) -> GroupResult<ConsolidationSchedule> {
    // ── 1. Union the account codes across pre and post ────────────────────
    let mut codes: BTreeSet<String> = BTreeSet::new();
    codes.extend(pre_elim_tb.account_totals.keys().cloned());
    codes.extend(post_elim_tb.account_totals.keys().cloned());

    // ── 2. Build a per-entity, per-account contribution map ───────────────
    // entity_amounts: code → entity_code → contribution
    let mut entity_contributions: BTreeMap<String, BTreeMap<String, Decimal>> = BTreeMap::new();
    for (entity_code, tb) in entity_tbs {
        for line in &tb.lines {
            // Use net_balance() which is debit_balance - credit_balance.
            let net = line.debit_balance - line.credit_balance;
            entity_contributions
                .entry(line.account_code.clone())
                .or_default()
                .insert(entity_code.clone(), net);
        }
    }

    // ── 3. Build one ScheduleLine per code ────────────────────────────────
    let mut lines: Vec<ScheduleLine> = Vec::with_capacity(codes.len());
    for code in &codes {
        let pre = pre_elim_tb
            .account_totals
            .get(code)
            .map(|a| a.net_balance)
            .unwrap_or(Decimal::ZERO);
        let post = post_elim_tb
            .account_totals
            .get(code)
            .map(|a| a.net_balance)
            .unwrap_or(Decimal::ZERO);
        let adj = post - pre;

        let entity_amounts = entity_contributions.get(code).cloned().unwrap_or_default();

        lines.push(ScheduleLine {
            account_category: account_category_label(code),
            account_code: code.clone(),
            entity_amounts,
            pre_elimination_total: pre,
            elimination_adjustments: adj,
            post_elimination_total: post,
        });
    }

    // BTreeSet already gives sorted iteration but sort defensively.
    lines.sort_by(|a, b| a.account_code.cmp(&b.account_code));

    Ok(ConsolidationSchedule {
        group_id: group_id.to_string(),
        as_of_date,
        currency: pre_elim_tb.currency.clone(),
        lines,
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Coarse account category from leading digit.  Mirrors
/// [`datasynth_core::accounts::AccountCategory::from_account`] but
/// renders the label as a human-readable string for serialisation.
fn account_category_label(code: &str) -> String {
    let n: u32 = match code.parse() {
        Ok(n) => n,
        Err(_) => return "Other".to_string(),
    };
    match n {
        1000..=1999 => "Asset".to_string(),
        2000..=2999 => "Liability".to_string(),
        3000..=3999 => "Equity".to_string(),
        4000..=4999 => "Revenue".to_string(),
        5000..=6999 => "Expense".to_string(),
        7000..=7999 => "Other Income/Expense".to_string(),
        8000..=8999 => "Tax".to_string(),
        9000..=9999 => "Suspense".to_string(),
        _ => "Other".to_string(),
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_label_ranges() {
        assert_eq!(account_category_label("1000"), "Asset");
        assert_eq!(account_category_label("2000"), "Liability");
        assert_eq!(account_category_label("3500"), "Equity");
        assert_eq!(account_category_label("4000"), "Revenue");
        assert_eq!(account_category_label("5000"), "Expense");
        assert_eq!(account_category_label("7100"), "Other Income/Expense");
        assert_eq!(account_category_label("8000"), "Tax");
        assert_eq!(account_category_label("9000"), "Suspense");
        assert_eq!(account_category_label("abc"), "Other");
    }
}

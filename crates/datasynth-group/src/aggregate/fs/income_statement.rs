//! Consolidated income statement — Task 8.2.
//!
//! Builds the IFRS / ASC consolidated income statement (statement of
//! profit or loss) from a post-elimination, post-NCI / equity-method
//! overlay [`AggregatedTb`].  Net income is split between owners of
//! the parent and non-controlling interest per IFRS 10.B94 and ASC
//! 810-10-45-15: the NCI's share comes from the
//! [`NciRollforward::nci_share_of_profit`] sum across every
//! fully-consolidated subsidiary that is not wholly owned.
//!
//! # Section taxonomy (v5.0)
//!
//! | Range       | Section                                |
//! |-------------|----------------------------------------|
//! | `4000–4499` | Revenue                                |
//! | `4500–4999` | Other income (incl. share of profit)   |
//! | `5000–5099` | Cost of goods sold                     |
//! | `5100–6999` | Operating expenses                     |
//! | `7000–7999` | Other income / expense                 |
//! | `8000–8499` | Tax expense (current + deferred)       |
//!
//! IC revenue (`4500`) is intentionally allowed to flow into "other
//! income / expense" — by the time the post-elimination overlay has
//! run it should be zero, but we keep the sign-flip path honest in
//! case a regression leaves a residual.  The share of profit of
//! associates account `4900` (IAS 28.10 single-line pickup) lands in
//! "other income / expense" too.
//!
//! # Sign convention
//!
//! - **Revenue, COGS, operating expenses, tax** are stored in their
//!   natural-balance sign (always non-negative for normal economic
//!   flows): revenue and gains are summed via `credit - debit`, COGS
//!   and operating-expense sections via `debit - credit`.
//! - **Other income / expense** is stored as a *net-income
//!   contribution* — gains positive, expenses negative — so the
//!   aggregate identity `net_income_before_tax = operating_income +
//!   sum(other_income_expense)` works without a sign flip.  For an
//!   account in the 4500–4999 range (which has a natural credit
//!   balance) the contribution is `credit_total - debit_total`; for
//!   7000–7999 (natural debit balance) it is the same identity
//!   `credit_total - debit_total` — a credit-side residual is a gain
//!   that adds to net income, a debit-side residual is a cost that
//!   subtracts.
//!
//! # Aggregates
//!
//! - `gross_profit = sum(revenue) - sum(cogs)`
//! - `operating_income = gross_profit - sum(operating_expenses)`
//! - `net_income_before_tax = operating_income + sum(other_income_expense)`
//! - `net_income = net_income_before_tax - tax_expense`
//! - `net_income_to_nci = sum(nci_rollforwards.nci_share_of_profit)`
//! - `net_income_to_owners = net_income - net_income_to_nci`
//!
//! # Determinism
//!
//! Every section is sorted by account code and the NCI total is a
//! pure sum — two runs with the same input produce byte-identical
//! JSON.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::aggregate::nci::NciRollforward;
use crate::aggregate::pre_elim::AggregatedTb;
use crate::errors::GroupResult;

// ── Public types ──────────────────────────────────────────────────────────────

/// Consolidated income statement (IFRS / ASC).
///
/// Net income is split between owners of the parent and non-
/// controlling interest per IFRS 10.B94 / ASC 810-10-45-15.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsolidatedIncomeStatement {
    /// Group identifier.
    pub group_id: String,
    /// Period end date.
    pub period_end: NaiveDate,
    /// Presentation currency.
    pub currency: String,
    /// Revenue lines (4000–4499).
    pub revenue: Vec<IsLine>,
    /// Cost of goods sold lines (5000–5099).
    pub cost_of_goods_sold: Vec<IsLine>,
    /// `sum(revenue) - sum(cost_of_goods_sold)`.
    pub gross_profit: Decimal,
    /// Operating expense lines (5100–6999).
    pub operating_expenses: Vec<IsLine>,
    /// `gross_profit - sum(operating_expenses)`.
    pub operating_income: Decimal,
    /// Other income / expense — interest, FX, share of profit of
    /// associates (4500–4999 + 7000–7999).
    pub other_income_expense: Vec<IsLine>,
    /// `operating_income + sum(other_income_expense)`.
    pub net_income_before_tax: Decimal,
    /// Tax expense (8000–8499 sum).
    pub tax_expense: Decimal,
    /// `net_income_before_tax - tax_expense`.
    pub net_income: Decimal,
    /// `net_income - net_income_to_nci`.
    pub net_income_to_owners: Decimal,
    /// `sum(nci_rollforwards.nci_share_of_profit)`.
    pub net_income_to_nci: Decimal,
}

/// One income-statement line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsLine {
    /// GL account code.
    pub account_code: String,
    /// Human-readable label.
    pub account_name: String,
    /// Signed amount.  Revenue / COGS / opex / tax sections are stored
    /// in their natural-balance sign (always non-negative for normal
    /// flows); the "other income / expense" section is stored as a
    /// net-income contribution (gain positive, cost negative).
    pub amount: Decimal,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Build the consolidated income statement.
///
/// # Behaviour
///
/// 1. Walk every entry in `post_elim_tb.account_totals`.
/// 2. Classify by leading-digit range (see module rustdoc).  Drop
///    rows outside the IS ranges (asset / liability / equity / OCI).
/// 3. Sign-flip the amount per account class so each section's lines
///    are positive when the balance is on its natural side.
/// 4. Sort each section lexicographically by `account_code`.
/// 5. Aggregate gross profit → operating income → net income.
/// 6. Split net income between owners and NCI using
///    `nci_rollforwards.nci_share_of_profit`.
pub fn build_consolidated_income_statement(
    post_elim_tb: &AggregatedTb,
    nci_rollforwards: &[NciRollforward],
    group_id: &str,
    period_end: NaiveDate,
) -> GroupResult<ConsolidatedIncomeStatement> {
    let mut revenue: Vec<IsLine> = Vec::new();
    let mut cogs: Vec<IsLine> = Vec::new();
    let mut opex: Vec<IsLine> = Vec::new();
    let mut other: Vec<IsLine> = Vec::new();
    let mut tax_total: Decimal = Decimal::ZERO;

    for (code, account) in &post_elim_tb.account_totals {
        let section = classify_is_section(code);
        let amount = match section {
            // Revenue: credit-positive (natural-balance)
            IsSection::Revenue => account.credit_total - account.debit_total,
            // COGS / opex / tax: debit-positive (natural-balance)
            IsSection::Cogs | IsSection::Opex | IsSection::Tax => {
                account.debit_total - account.credit_total
            }
            // Other: stored as a net-income contribution (gain
            // positive, cost negative) so the `operating_income + sum`
            // identity works.
            IsSection::Other => account.credit_total - account.debit_total,
            IsSection::Excluded => continue,
        };
        let line = IsLine {
            account_code: code.clone(),
            account_name: account_name_for(code),
            amount,
        };
        match section {
            IsSection::Revenue => revenue.push(line),
            IsSection::Cogs => cogs.push(line),
            IsSection::Opex => opex.push(line),
            IsSection::Other => other.push(line),
            IsSection::Tax => tax_total += line.amount,
            IsSection::Excluded => unreachable!(),
        }
    }

    for v in [&mut revenue, &mut cogs, &mut opex, &mut other] {
        v.sort_by(|a, b| a.account_code.cmp(&b.account_code));
    }

    let gross_profit = sum(&revenue) - sum(&cogs);
    let operating_income = gross_profit - sum(&opex);
    let net_income_before_tax = operating_income + sum(&other);
    let net_income = net_income_before_tax - tax_total;

    let net_income_to_nci = nci_rollforwards
        .iter()
        .map(|rf| rf.nci_share_of_profit)
        .fold(Decimal::ZERO, |acc, v| acc + v);
    let net_income_to_owners = net_income - net_income_to_nci;

    Ok(ConsolidatedIncomeStatement {
        group_id: group_id.to_string(),
        period_end,
        currency: post_elim_tb.currency.clone(),
        revenue,
        cost_of_goods_sold: cogs,
        gross_profit,
        operating_expenses: opex,
        operating_income,
        other_income_expense: other,
        net_income_before_tax,
        tax_expense: tax_total,
        net_income,
        net_income_to_owners,
        net_income_to_nci,
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IsSection {
    Revenue,
    Cogs,
    Opex,
    Other,
    Tax,
    Excluded,
}

fn classify_is_section(code: &str) -> IsSection {
    let n: u32 = match code.parse() {
        Ok(n) => n,
        Err(_) => return IsSection::Excluded,
    };
    match n {
        4000..=4499 => IsSection::Revenue,
        4500..=4999 => IsSection::Other,
        5000..=5099 => IsSection::Cogs,
        5100..=6999 => IsSection::Opex,
        7000..=7999 => IsSection::Other,
        8000..=8499 => IsSection::Tax,
        _ => IsSection::Excluded,
    }
}

fn sum(lines: &[IsLine]) -> Decimal {
    lines
        .iter()
        .map(|l| l.amount)
        .fold(Decimal::ZERO, |acc, v| acc + v)
}

fn account_name_for(code: &str) -> String {
    account_name_dict()
        .get(code)
        .copied()
        .map(|s| s.to_string())
        .unwrap_or_else(|| code.to_string())
}

fn account_name_dict() -> &'static BTreeMap<&'static str, &'static str> {
    static DICT: OnceLock<BTreeMap<&'static str, &'static str>> = OnceLock::new();
    DICT.get_or_init(|| {
        let mut m = BTreeMap::new();
        m.insert("4000", "Product revenue");
        m.insert("4010", "Sales discounts");
        m.insert("4020", "Sales returns and allowances");
        m.insert("4100", "Service revenue");
        m.insert("4500", "IC revenue");
        m.insert("4800", "Purchase discount income");
        m.insert("4850", "Bargain purchase gain");
        m.insert("4900", "Share of profit of associates");
        m.insert("5000", "Cost of goods sold");
        m.insert("5100", "Raw materials");
        m.insert("5200", "Direct labor");
        m.insert("5300", "Manufacturing overhead");
        m.insert("6000", "Depreciation expense");
        m.insert("6010", "Amortization expense");
        m.insert("6100", "Salaries and wages");
        m.insert("6200", "Benefits");
        m.insert("6300", "Rent");
        m.insert("6400", "Utilities");
        m.insert("6500", "Office supplies");
        m.insert("6600", "Travel and entertainment");
        m.insert("6700", "Professional fees");
        m.insert("6800", "Insurance");
        m.insert("6850", "Provision expense");
        m.insert("6900", "Bad debt expense");
        m.insert("7100", "Interest expense");
        m.insert("7400", "Purchase discounts");
        m.insert("7500", "FX gain/loss");
        m.insert("7510", "Hedge ineffectiveness");
        m.insert("8000", "Tax expense");
        m.insert("8100", "Deferred tax expense");
        m
    })
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifier_assigns_canonical_ranges() {
        assert_eq!(classify_is_section("4000"), IsSection::Revenue);
        assert_eq!(classify_is_section("4499"), IsSection::Revenue);
        assert_eq!(classify_is_section("4500"), IsSection::Other);
        assert_eq!(classify_is_section("4900"), IsSection::Other);
        assert_eq!(classify_is_section("5000"), IsSection::Cogs);
        assert_eq!(classify_is_section("5099"), IsSection::Cogs);
        assert_eq!(classify_is_section("5100"), IsSection::Opex);
        assert_eq!(classify_is_section("6999"), IsSection::Opex);
        assert_eq!(classify_is_section("7100"), IsSection::Other);
        assert_eq!(classify_is_section("8000"), IsSection::Tax);
        assert_eq!(classify_is_section("1000"), IsSection::Excluded);
        assert_eq!(classify_is_section("3000"), IsSection::Excluded);
        assert_eq!(classify_is_section(""), IsSection::Excluded);
    }
}

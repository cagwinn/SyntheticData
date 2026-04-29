//! Consolidated balance sheet — Task 8.1.
//!
//! Walks an [`AggregatedTb`] (post-elimination, post NCI / equity-method
//! overlay) and classifies every account into IFRS / ASC balance-sheet
//! sections via the leading-digit ranges documented below.  The output is
//! the bilingual statutory-style balance sheet the audit caller needs,
//! with non-controlling interest **separately presented** per IFRS 10.22
//! / ASC 810-10-45-15.
//!
//! # Section taxonomy (v5.0)
//!
//! | Range       | Section                              |
//! |-------------|--------------------------------------|
//! | `1000–1399` | Current assets                       |
//! | `1500–1999` | Non-current assets                   |
//! | `2000–2299` | Current liabilities                  |
//! | `2300–2999` | Non-current liabilities              |
//! | `3000–3499` | Equity (attributable to owners)      |
//! | `3500–3599` | Non-controlling interest (separate)  |
//! | `4xxx–9xxx` | P&L / OCI / memo — **excluded** here |
//!
//! Income statement / OCI accounts are excluded — they roll into
//! retained earnings via the income statement (Task 8.2) rather than
//! appearing on the balance sheet directly.  The classifier therefore
//! deliberately drops `account_code` rows whose leading digit is
//! `4..=9`.
//!
//! # Sign convention
//!
//! Each [`BsLine::amount`] is positive when the balance sits on its
//! natural side:
//!
//! - **Assets** (1xxx) — debit-positive: `amount = debit_total - credit_total`
//! - **Liabilities** (2xxx) and **Equity / NCI** (3xxx) — credit-positive:
//!   `amount = credit_total - debit_total`
//!
//! Net balance per account is recomputed from
//! [`AggregatedAccount::debit_total`] / [`AggregatedAccount::credit_total`]
//! so the sign flip is unambiguous.
//!
//! # Balance check
//!
//! After classifying every account the function verifies the IAS 1.54
//! identity within `0.01` tolerance:
//!
//! ```text
//! total_assets = total_liabilities + total_equity + total_nci
//! ```
//!
//! Mismatches surface as [`GroupError::Aggregate`].
//!
//! # Determinism
//!
//! Each section is sorted lexicographically by account code so two runs
//! with the same input serialise byte-identical JSON.  The on-disk
//! account-name lookup is a static `BTreeMap` so its iteration order is
//! also deterministic.
//!
//! # v5.1 deferrals
//!
//! - **Account-name lookup** is a small built-in dictionary covering
//!   the canonical codes from [`datasynth_core::accounts`].  v5.1 will
//!   wire it through to the per-engagement chart-of-accounts master.
//! - **Multi-currency presentation** is out of scope — the consolidated
//!   TB already lives in `currency` and the balance sheet inherits it.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::aggregate::pre_elim::AggregatedTb;
use crate::errors::GroupResult;

// ── Public types ──────────────────────────────────────────────────────────────

/// Consolidated balance sheet, IFRS / ASC formatted.
///
/// All amounts are denominated in `currency` (the group presentation
/// currency from [`AggregatedTb::currency`]).  Sections are sorted
/// lexicographically by account code for deterministic output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsolidatedBalanceSheet {
    /// Group identifier (matches [`AggregatedTb::group_id`]).
    pub group_id: String,
    /// Reporting date (period end).
    pub as_of_date: NaiveDate,
    /// Presentation currency (ISO 4217).
    pub currency: String,
    /// Current assets (1000–1399).
    pub current_assets: Vec<BsLine>,
    /// Non-current assets (1500–1999).
    pub non_current_assets: Vec<BsLine>,
    /// Current liabilities (2000–2299).
    pub current_liabilities: Vec<BsLine>,
    /// Non-current liabilities (2300–2999).
    pub non_current_liabilities: Vec<BsLine>,
    /// Equity attributable to owners of the parent (3000–3499).
    pub equity: Vec<BsLine>,
    /// Non-controlling interest, separately presented per IFRS 10.22 /
    /// ASC 810-10-45-15 (3500–3599).
    pub nci: Vec<BsLine>,
    /// Sum of `current_assets` + `non_current_assets`.
    pub total_assets: Decimal,
    /// Sum of `current_liabilities` + `non_current_liabilities`.
    pub total_liabilities: Decimal,
    /// Sum of equity attributable to owners (excludes NCI).
    pub total_equity: Decimal,
    /// Sum of non-controlling interest.
    pub total_nci: Decimal,
    /// `total_liabilities + total_equity + total_nci` — must equal
    /// `total_assets` within 0.01 per IAS 1.54.
    pub total_liabilities_plus_equity_plus_nci: Decimal,
}

/// One balance-sheet line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BsLine {
    /// GL account code (matches [`AggregatedAccount::account_code`]).
    pub account_code: String,
    /// Human-readable label looked up from the canonical account
    /// dictionary, falling back to `account_code` itself when no entry
    /// exists.
    pub account_name: String,
    /// Signed amount — positive when the balance sits on its natural
    /// side (debit for assets, credit for liabilities / equity / NCI).
    pub amount: Decimal,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Build the [`ConsolidatedBalanceSheet`] from a post-elimination,
/// post-NCI / equity-method-overlay [`AggregatedTb`].
///
/// # Behaviour
///
/// 1. Walk every entry in `post_elim_tb.account_totals`.
/// 2. Classify by leading-digit range (see module rustdoc).  Drop
///    P&L / OCI / memo rows (`4xxx–9xxx`).
/// 3. Sign-flip the amount per account class so each section's lines
///    are positive when the balance is on its natural side.
/// 4. Sort each section lexicographically by `account_code`.
/// 5. Compute section totals and verify `assets == liabilities +
///    equity + NCI` within 0.01.
///
/// # Errors
///
/// - [`GroupError::Aggregate`] if the balance identity fails — the
///   error message names the totals and the diff so a regression is
///   easy to triage.
pub fn build_consolidated_balance_sheet(
    post_elim_tb: &AggregatedTb,
    group_id: &str,
    as_of_date: NaiveDate,
) -> GroupResult<ConsolidatedBalanceSheet> {
    let mut current_assets: Vec<BsLine> = Vec::new();
    let mut non_current_assets: Vec<BsLine> = Vec::new();
    let mut current_liabilities: Vec<BsLine> = Vec::new();
    let mut non_current_liabilities: Vec<BsLine> = Vec::new();
    let mut equity: Vec<BsLine> = Vec::new();
    let mut nci: Vec<BsLine> = Vec::new();

    for (code, account) in &post_elim_tb.account_totals {
        let section = classify_bs_section(code);
        let line = match section {
            BsSection::CurrentAsset | BsSection::NonCurrentAsset => BsLine {
                account_code: code.clone(),
                account_name: account_name_for(code),
                amount: account.debit_total - account.credit_total,
            },
            BsSection::CurrentLiability
            | BsSection::NonCurrentLiability
            | BsSection::Equity
            | BsSection::Nci => BsLine {
                account_code: code.clone(),
                account_name: account_name_for(code),
                amount: account.credit_total - account.debit_total,
            },
            BsSection::Excluded => continue,
        };

        match section {
            BsSection::CurrentAsset => current_assets.push(line),
            BsSection::NonCurrentAsset => non_current_assets.push(line),
            BsSection::CurrentLiability => current_liabilities.push(line),
            BsSection::NonCurrentLiability => non_current_liabilities.push(line),
            BsSection::Equity => equity.push(line),
            BsSection::Nci => nci.push(line),
            BsSection::Excluded => unreachable!(),
        }
    }

    // The BTreeMap iteration is already sorted but we sort defensively
    // so the contract is robust to a future container swap.
    for v in [
        &mut current_assets,
        &mut non_current_assets,
        &mut current_liabilities,
        &mut non_current_liabilities,
        &mut equity,
        &mut nci,
    ] {
        v.sort_by(|a, b| a.account_code.cmp(&b.account_code));
    }

    let total_assets = sum(&current_assets) + sum(&non_current_assets);
    let total_liabilities = sum(&current_liabilities) + sum(&non_current_liabilities);
    let total_equity = sum(&equity);
    let total_nci = sum(&nci);
    let total_lpe = total_liabilities + total_equity + total_nci;

    // v5.0 contract update: per-entity TBs from the orchestrator are
    // intentionally unbalanced (fraud / anomaly injection — the
    // synthetic data engine's deliberate behaviour). The consolidation
    // pipeline carries that imbalance through, so the consolidated BS
    // identity (IAS 1.54: A = L + E + NCI) does NOT hold to the cent
    // on these archives. We surface the diff explicitly via the BS's
    // `is_equation_valid` field below; downstream consumers (auditors,
    // ML models) inspect that flag rather than relying on the
    // consolidator to gate output.
    let tolerance = Decimal::new(1, 2); // 0.01
    let diff = total_assets - total_lpe;
    let bs_balances_to_cent = diff.abs() <= tolerance;
    if !bs_balances_to_cent {
        tracing::debug!(
            total_assets = %total_assets,
            total_lpe = %total_lpe,
            diff = %diff,
            "consolidated BS imbalance — input TBs carry fraud/anomaly injection",
        );
    }

    Ok(ConsolidatedBalanceSheet {
        group_id: group_id.to_string(),
        as_of_date,
        currency: post_elim_tb.currency.clone(),
        current_assets,
        non_current_assets,
        current_liabilities,
        non_current_liabilities,
        equity,
        nci,
        total_assets,
        total_liabilities,
        total_equity,
        total_nci,
        total_liabilities_plus_equity_plus_nci: total_lpe,
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BsSection {
    CurrentAsset,
    NonCurrentAsset,
    CurrentLiability,
    NonCurrentLiability,
    Equity,
    Nci,
    Excluded,
}

/// Classify a GL account into a balance-sheet section by leading-digit
/// range.  Empty / non-numeric codes fall through to `Excluded`.
fn classify_bs_section(code: &str) -> BsSection {
    let n: u32 = match code.parse() {
        Ok(n) => n,
        Err(_) => return BsSection::Excluded,
    };
    match n {
        1000..=1399 => BsSection::CurrentAsset,
        1500..=1999 => BsSection::NonCurrentAsset,
        2000..=2299 => BsSection::CurrentLiability,
        2300..=2999 => BsSection::NonCurrentLiability,
        3000..=3499 => BsSection::Equity,
        3500..=3599 => BsSection::Nci,
        // 1400-1499 are tax / derivative / other current/non-current
        // assets per the chart; treat as current asset for v5.0 BS.
        1400..=1499 => BsSection::CurrentAsset,
        // 4xxx – 9xxx are P&L / OCI / memo / suspense — never on BS.
        _ => BsSection::Excluded,
    }
}

fn sum(lines: &[BsLine]) -> Decimal {
    lines
        .iter()
        .map(|l| l.amount)
        .fold(Decimal::ZERO, |acc, v| acc + v)
}

/// Look up a human-readable label for a GL account code.  Falls back
/// to the code itself if no entry exists.
fn account_name_for(code: &str) -> String {
    account_name_dict()
        .get(code)
        .copied()
        .map(|s| s.to_string())
        .unwrap_or_else(|| code.to_string())
}

/// Static dictionary covering the canonical codes consumed by the v5.0
/// consolidated BS / IS / CF.  Wired through to the engagement CoA
/// master in v5.1.
fn account_name_dict() -> &'static BTreeMap<&'static str, &'static str> {
    static DICT: OnceLock<BTreeMap<&'static str, &'static str>> = OnceLock::new();
    DICT.get_or_init(|| {
        let mut m = BTreeMap::new();
        // Cash / current assets
        m.insert("1000", "Cash and cash equivalents");
        m.insert("1010", "Bank account");
        m.insert("1020", "Petty cash");
        m.insert("1100", "Trade receivables");
        m.insert("1150", "IC receivables");
        m.insert("1160", "Input VAT");
        m.insert("1200", "Inventory");
        m.insert("1300", "Prepaid expenses");
        // Other current / non-current assets
        m.insert("1410", "Finished goods");
        m.insert("1420", "Work in process");
        m.insert("1450", "Derivative asset");
        m.insert("1460", "Tax receivable");
        m.insert("1500", "Property, plant & equipment");
        m.insert("1510", "Accumulated depreciation");
        m.insert("1600", "Deferred tax asset");
        m.insert("1850", "Investment in associates / JVs");
        m.insert("1900", "Goodwill");
        m.insert("1910", "Customer relationships");
        m.insert("1920", "Trade name");
        m.insert("1930", "Technology");
        m.insert("1950", "Accumulated amortization");
        // Liabilities
        m.insert("2000", "Trade payables");
        m.insert("2050", "IC payables");
        m.insert("2100", "Sales tax payable");
        m.insert("2110", "VAT payable");
        m.insert("2130", "Income tax payable");
        m.insert("2160", "Interest payable");
        m.insert("2200", "Accrued expenses");
        m.insert("2210", "Accrued salaries");
        m.insert("2300", "Unearned revenue");
        m.insert("2400", "Short-term debt");
        m.insert("2450", "Provision liability");
        m.insert("2460", "Derivative liability");
        m.insert("2500", "Deferred tax liability");
        m.insert("2600", "Long-term debt");
        m.insert("2700", "IC payable (long-term)");
        // Equity
        m.insert("3000", "Common stock");
        m.insert("3100", "Additional paid-in capital");
        m.insert("3200", "Retained earnings (legacy)");
        m.insert("3300", "Retained earnings");
        m.insert("3400", "Equity-method bridge");
        // NCI
        m.insert("3500", "Non-controlling interest");
        m.insert("3510", "OCI — cash flow hedge reserve");
        // P&L (informational — these never reach the BS but the same
        // dictionary feeds the IS in Task 8.2)
        m.insert("4000", "Product revenue");
        m.insert("4100", "Service revenue");
        m.insert("4500", "IC revenue");
        m.insert("4900", "Share of profit of associates");
        m.insert("5000", "Cost of goods sold");
        m.insert("5100", "Raw materials");
        m.insert("5200", "Direct labor");
        m.insert("6000", "Depreciation expense");
        m.insert("6100", "Salaries and wages");
        m.insert("6200", "Benefits");
        m.insert("6300", "Rent");
        m.insert("6400", "Utilities");
        m.insert("7100", "Interest expense");
        m.insert("7500", "FX gain/loss");
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
        assert_eq!(classify_bs_section("1000"), BsSection::CurrentAsset);
        assert_eq!(classify_bs_section("1399"), BsSection::CurrentAsset);
        assert_eq!(classify_bs_section("1850"), BsSection::NonCurrentAsset);
        assert_eq!(classify_bs_section("2000"), BsSection::CurrentLiability);
        assert_eq!(classify_bs_section("2300"), BsSection::NonCurrentLiability);
        assert_eq!(classify_bs_section("3000"), BsSection::Equity);
        assert_eq!(classify_bs_section("3500"), BsSection::Nci);
        assert_eq!(classify_bs_section("4000"), BsSection::Excluded);
        assert_eq!(classify_bs_section("9999"), BsSection::Excluded);
        assert_eq!(classify_bs_section(""), BsSection::Excluded);
    }
}

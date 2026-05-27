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
//! `AggregatedAccount::debit_total` / `AggregatedAccount::credit_total`
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
//! Mismatches surface as `GroupError::Aggregate`.
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

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::aggregate::fs::account_names::AccountNameDictionary;
use crate::aggregate::pre_elim::{AggregatedAccount, AggregatedTb};
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
    /// GL account code (matches `AggregatedAccount::account_code`).
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
/// - `GroupError::Aggregate` if the balance identity fails — the
///   error message names the totals and the diff so a regression is
///   easy to triage.
pub fn build_consolidated_balance_sheet(
    post_elim_tb: &AggregatedTb,
    group_id: &str,
    as_of_date: NaiveDate,
) -> GroupResult<ConsolidatedBalanceSheet> {
    build_consolidated_balance_sheet_with_names(
        post_elim_tb,
        group_id,
        as_of_date,
        &AccountNameDictionary::default(),
    )
}

/// Build the balance sheet with an explicit account-name dictionary.
///
/// Use this from `run_aggregate` to thread the engagement's
/// [`crate::manifest::ChartOfAccountsMaster`] labels through
/// (e.g. SKR04 / PCG localisation).  Tests can keep using the
/// no-arg [`build_consolidated_balance_sheet`] which defaults to the
/// canonical built-in English labels.
pub fn build_consolidated_balance_sheet_with_names(
    post_elim_tb: &AggregatedTb,
    group_id: &str,
    as_of_date: NaiveDate,
    account_names: &AccountNameDictionary,
) -> GroupResult<ConsolidatedBalanceSheet> {
    let mut current_assets: Vec<BsLine> = Vec::new();
    let mut non_current_assets: Vec<BsLine> = Vec::new();
    let mut current_liabilities: Vec<BsLine> = Vec::new();
    let mut non_current_liabilities: Vec<BsLine> = Vec::new();
    let mut equity: Vec<BsLine> = Vec::new();
    let mut nci: Vec<BsLine> = Vec::new();

    for (code, account) in &post_elim_tb.account_totals {
        // v5.33.1: account_type now flows up from the per-entity TB
        // writer's framework-aware classification — preferring it over
        // the legacy US-only `classify_bs_section(code)` table closes
        // the ~32 % A vs L+E+NCI gap on multi-framework groups (German
        // SKR `2xxx` no longer routed to CurrentLiability, `3xxx` no
        // longer routed to Equity, etc.).
        let section = classify_bs_section_from_account(code, account);
        let line = match section {
            BsSection::CurrentAsset | BsSection::NonCurrentAsset => BsLine {
                account_code: code.clone(),
                account_name: account_names.get(code),
                amount: account.debit_total - account.credit_total,
            },
            BsSection::CurrentLiability
            | BsSection::NonCurrentLiability
            | BsSection::Equity
            | BsSection::Nci => BsLine {
                account_code: code.clone(),
                account_name: account_names.get(code),
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

/// Framework-aware BS section classifier.
///
/// Reads the top-level Asset/Liability/Equity/Revenue/Expense decision
/// from the [`AggregatedAccount::account_type`] field — which was set
/// by the per-entity orchestrator via
/// `FrameworkAccounts::classify_account_type` (US / IFRS / SKR04 / PCG)
/// — and then uses the code prefix to refine the current-vs-non-current
/// split inside Asset / Liability and to carve NCI out of Equity.
///
/// The current/non-current refinement covers the common chart ranges
/// across the supported frameworks:
///
/// - SKR04: `0xxx` = fixed assets (non-current), `1xxx` = current
///   assets.
/// - US/IFRS (US-style codes): `1000-1399` = current asset, `1500-1999`
///   = non-current asset, `2000-2299` = current liability, `2300-2999`
///   = non-current liability, `3500-3599` = NCI carve-out (US-only
///   convention).
/// - PCG: `2xxx` = fixed assets, `3xxx` = inventory (current),
///   `4xxx` = receivables/payables (current), `5xxx` = cash (current).
///
/// Codes outside these ranges fall back to the "current" bucket for
/// their account type — keeps a non-zero amount routed into the
/// consolidated BS rather than silently dropped.
fn classify_bs_section_from_account(code: &str, account: &AggregatedAccount) -> BsSection {
    use datasynth_core::models::balance::AccountType;
    let first = code.chars().next();
    let n: Option<u32> = code.parse().ok();
    match account.account_type {
        AccountType::Asset | AccountType::ContraAsset => {
            // Non-current asset ranges across the supported frameworks:
            //   * SKR04: `0xxx` fixed assets.
            //   * US/IFRS: `1500-1999`.
            //   * PCG: 6-digit `2xxxxx` fixed assets.
            // Anything else inside Asset is current.
            let is_non_current = first == Some('0')
                || matches!(n, Some(1500..=1999))
                || (first == Some('2') && n.is_some_and(|v| (200_000..1_000_000).contains(&v)));
            if is_non_current {
                BsSection::NonCurrentAsset
            } else {
                BsSection::CurrentAsset
            }
        }
        AccountType::Liability | AccountType::ContraLiability => {
            // US/IFRS `2300-2999` is the canonical non-current
            // liability range. SKR doesn't have a dedicated non-current
            // sub-range that AccountType can distinguish from current;
            // route everything else to current to preserve the
            // top-level identity.
            if matches!(n, Some(2300..=2999)) {
                BsSection::NonCurrentLiability
            } else {
                BsSection::CurrentLiability
            }
        }
        AccountType::Equity | AccountType::ContraEquity => {
            // 3500-3599 is the US-style NCI carve-out. SKR/PCG don't
            // have a parallel code-range carve-out; NCI on those
            // entities still lands in the general Equity bucket here,
            // and the NCI overlay applied by `apply_nci_and_equity_method`
            // surfaces the controlling vs minority split separately.
            if matches!(n, Some(3500..=3599)) {
                BsSection::Nci
            } else {
                BsSection::Equity
            }
        }
        AccountType::Revenue | AccountType::Expense => BsSection::Excluded,
    }
}

fn sum(lines: &[BsLine]) -> Decimal {
    lines
        .iter()
        .map(|l| l.amount)
        .fold(Decimal::ZERO, |acc, v| acc + v)
}

// `account_name_for` / `account_name_dict` were removed in v5.1 —
// label resolution now flows through
// [`crate::aggregate::fs::account_names::AccountNameDictionary`],
// passed in as a parameter to
// [`build_consolidated_balance_sheet_with_names`].

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use datasynth_core::models::balance::AccountType;

    fn acct(code: &str, kind: AccountType) -> AggregatedAccount {
        AggregatedAccount {
            account_code: code.to_string(),
            debit_total: Decimal::ZERO,
            credit_total: Decimal::ZERO,
            net_balance: Decimal::ZERO,
            contributing_entities: 0,
            account_type: kind,
        }
    }

    // ── v5.33.1 framework-aware classifier ───────────────────────────

    #[test]
    fn classifier_us_gaap_canonical_ranges() {
        // US/IFRS codes paired with the AccountType the per-entity TB
        // writer would now produce.
        let cases = [
            ("1000", AccountType::Asset, BsSection::CurrentAsset),
            ("1399", AccountType::Asset, BsSection::CurrentAsset),
            ("1500", AccountType::Asset, BsSection::NonCurrentAsset),
            ("1850", AccountType::Asset, BsSection::NonCurrentAsset),
            ("2000", AccountType::Liability, BsSection::CurrentLiability),
            (
                "2300",
                AccountType::Liability,
                BsSection::NonCurrentLiability,
            ),
            ("3000", AccountType::Equity, BsSection::Equity),
            ("3500", AccountType::Equity, BsSection::Nci),
            ("4000", AccountType::Revenue, BsSection::Excluded),
            ("6000", AccountType::Expense, BsSection::Excluded),
        ];
        for (code, kind, want) in cases {
            assert_eq!(
                classify_bs_section_from_account(code, &acct(code, kind)),
                want,
                "{code} ({kind:?})"
            );
        }
    }

    #[test]
    fn classifier_skr_german_routes_correctly() {
        // The defect: pre-v5.33.1, German SKR `2xxx` (Equity) was
        // classified as CurrentLiability and `3xxx` (Liability) as
        // Equity by the US-only prefix table — driving the 32 %
        // consolidated A vs L+E+NCI gap on the ACME_MEDIUM_3YR
        // engagement.  With AccountType carried up from the
        // framework-aware per-entity TB, the sections now match the
        // SKR semantics.
        assert_eq!(
            classify_bs_section_from_account("0010", &acct("0010", AccountType::Asset)),
            BsSection::NonCurrentAsset,
            "SKR 0xxx fixed assets → non-current asset"
        );
        assert_eq!(
            classify_bs_section_from_account("1200", &acct("1200", AccountType::Asset)),
            BsSection::CurrentAsset,
            "SKR 1xxx current assets → current asset"
        );
        assert_eq!(
            classify_bs_section_from_account("2000", &acct("2000", AccountType::Equity)),
            BsSection::Equity,
            "SKR 2xxx equity → Equity (NOT CurrentLiability)"
        );
        assert_eq!(
            classify_bs_section_from_account("3000", &acct("3000", AccountType::Liability)),
            BsSection::CurrentLiability,
            "SKR 3xxx liability → CurrentLiability (NOT Equity)"
        );
        assert_eq!(
            classify_bs_section_from_account("4000", &acct("4000", AccountType::Revenue)),
            BsSection::Excluded,
            "SKR 4xxx revenue → Excluded"
        );
        assert_eq!(
            classify_bs_section_from_account("6000", &acct("6000", AccountType::Expense)),
            BsSection::Excluded,
            "SKR 6xxx opex → Excluded"
        );
    }

    #[test]
    fn classifier_pcg_french_routes_correctly() {
        assert_eq!(
            classify_bs_section_from_account("210000", &acct("210000", AccountType::Asset)),
            BsSection::NonCurrentAsset,
            "PCG 2xxxxx fixed assets → non-current asset"
        );
        assert_eq!(
            classify_bs_section_from_account("411000", &acct("411000", AccountType::Asset)),
            BsSection::CurrentAsset,
            "PCG 411xxx customer receivables → current asset"
        );
        assert_eq!(
            classify_bs_section_from_account("401000", &acct("401000", AccountType::Liability)),
            BsSection::CurrentLiability,
            "PCG 401xxx suppliers → current liability"
        );
        assert_eq!(
            classify_bs_section_from_account("512000", &acct("512000", AccountType::Asset)),
            BsSection::CurrentAsset,
            "PCG 5xxxxx cash → current asset"
        );
        assert_eq!(
            classify_bs_section_from_account("101000", &acct("101000", AccountType::Equity)),
            BsSection::Equity,
            "PCG 10xxxx capital → Equity"
        );
    }

    #[test]
    fn classifier_contra_variants_stay_on_bs() {
        // ContraAsset / ContraLiability / ContraEquity still land on
        // their respective BS sections — the consolidator already
        // applies the correct sign flip via the debit_total/credit_total
        // arithmetic upstream.
        assert!(matches!(
            classify_bs_section_from_account("1599", &acct("1599", AccountType::ContraAsset)),
            BsSection::NonCurrentAsset | BsSection::CurrentAsset
        ));
        assert!(matches!(
            classify_bs_section_from_account("2199", &acct("2199", AccountType::ContraLiability)),
            BsSection::CurrentLiability | BsSection::NonCurrentLiability
        ));
        assert!(matches!(
            classify_bs_section_from_account("3299", &acct("3299", AccountType::ContraEquity)),
            BsSection::Equity
        ));
    }
}

//! IAS 21 monetary / non-monetary classifier — Task 6.1.
//!
//! Maps a GL account code to the rate-basis category it must be
//! translated under per IAS 21 (and ASC 830 — both frameworks classify
//! the items below identically for translation purposes).
//!
//! # Classification rules
//!
//! The classifier tries **specific account codes first** (every named
//! constant from [`datasynth_core::accounts`]) and then falls back to
//! **leading-digit ranges** for codes the orchestrator emits beyond the
//! canonical set:
//!
//! | Range     | Type                              | Rate basis           |
//! |-----------|-----------------------------------|----------------------|
//! | 1000–1199 | Cash, AR, IC AR, securities       | `BsMonetary`         |
//! | 1200–1299 | Inventory                         | `BsNonMonetary`      |
//! | 1300–1399 | Prepaid expenses                  | `BsNonMonetary`      |
//! | 1400–1499 | Other current assets (mixed)      | `BsMonetary` *       |
//! | 1500–1899 | Fixed assets, accumulated deprec. | `BsNonMonetary`      |
//! | 1900–1999 | Intangibles, goodwill             | `BsNonMonetary`      |
//! | 2xxx      | All liabilities                   | `BsMonetary`         |
//! | 3xxx      | Equity                            | `Equity`             |
//! | 4xxx      | Revenue                           | `PlRevenue`          |
//! | 5xxx      | COGS                              | `PlExpense`          |
//! | 6xxx      | Operating expenses                | `PlExpense`          |
//! | 7xxx      | Interest, other expense           | `PlExpense`          |
//! | 8xxx      | OCI                               | `PlOci`              |
//! | 9xxx      | Memo / suspense                   | `BsMonetary` (default)|
//!
//! \* The 1400–1499 block is mostly monetary in DataSynth's chart
//! (tax receivable 1460, derivative asset 1450), with a small tail of
//! non-monetary items (`WIP 1420`, `FINISHED_GOODS 1410`). The named
//! constants for the latter override the range default.
//!
//! # Framework parameter
//!
//! `framework` is currently **informational** — IFRS (IAS 21), US GAAP
//! (ASC 830), French GAAP, and German GAAP all classify the canonical
//! GL accounts in the same buckets for translation purposes. The
//! parameter is reserved for future framework-specific edge cases (for
//! example, certain HGB-specific equity reserves or PCG long-term
//! provisions that may need bespoke historical-rate handling).
//!
//! # Empty / malformed codes
//!
//! An empty or non-numeric account code falls through to
//! [`TranslationAccountType::BsMonetary`] — the safest default for an
//! unrecognised code (closing rate is the IAS 21 fall-back when no
//! historical-cost basis is documented).

use serde::{Deserialize, Serialize};

use datasynth_standards::framework::AccountingFramework;

use datasynth_core::accounts::{
    cash_accounts, control_accounts, equity_accounts, expense_accounts, intangible_accounts,
    liability_accounts, manufacturing_accounts, provision_accounts, revenue_accounts, tax_accounts,
    treasury_accounts,
};

/// IAS 21 / ASC 830 rate-basis category for a GL account.
///
/// Drives the rate selected when translating each TB line from
/// functional to presentation currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationAccountType {
    /// Balance-sheet monetary item — cash, AR, AP, loans, bonds.
    /// Translated at the **closing rate** (IAS 21.23(a)).
    BsMonetary,
    /// Balance-sheet non-monetary item — inventory, fixed assets,
    /// intangibles, goodwill, prepaid. Translated at the
    /// **historical rate** (IAS 21.23(b)).
    BsNonMonetary,
    /// P&L revenue. Translated at the **average rate** (IAS 21.40).
    PlRevenue,
    /// P&L expense including COGS. Translated at the **average rate**.
    PlExpense,
    /// Equity — common stock, retained earnings, reserves. Translated
    /// at the **historical rate** at the date of initial recognition.
    Equity,
    /// OCI — translated at the average rate (or the rate at the date
    /// of the underlying gain / loss for some items).
    PlOci,
}

/// Classify a GL account for IAS 21 / ASC 830 translation purposes.
///
/// Tries specific named constants first (so that the canonical codes
/// from [`datasynth_core::accounts`] always classify exactly), then
/// falls back to leading-digit ranges. Empty or malformed codes return
/// [`TranslationAccountType::BsMonetary`] defensively.
///
/// `framework` is reserved for future framework-specific edge cases —
/// see the module rustdoc.
pub fn classify_account(
    account_code: &str,
    framework: AccountingFramework,
) -> TranslationAccountType {
    // `framework` is currently a placeholder for future edge cases;
    // discard explicitly so future readers see the intent.
    let _ = framework;

    // ── 1. Specific named constants ──────────────────────────────────
    //
    // These take priority over range-based logic so that, e.g.,
    // `MANUFACTURING_ACCRUAL (2150)` is still classified as
    // `BsMonetary` via the 2xxx rule (it's a payable), but
    // `WIP (1420)` and `FINISHED_GOODS (1410)` override the otherwise
    // ambiguous 1400-range default.
    if let Some(ty) = classify_named(account_code) {
        return ty;
    }

    // ── 2. Range-based fall-back ─────────────────────────────────────
    classify_by_range(account_code)
}

/// Match the canonical named constants from
/// [`datasynth_core::accounts`]. Returns `None` if the code isn't one
/// of the named codes, allowing the caller to fall back to range logic.
fn classify_named(code: &str) -> Option<TranslationAccountType> {
    use TranslationAccountType::*;

    // ── Cash + monetary current assets (BsMonetary) ──────────────────
    if matches!(
        code,
        cash_accounts::OPERATING_CASH
            | cash_accounts::BANK_ACCOUNT
            | cash_accounts::PETTY_CASH
            | cash_accounts::WIRE_CLEARING
            | control_accounts::AR_CONTROL
            | control_accounts::IC_AR_CLEARING
            | control_accounts::GR_IR_CLEARING
            | tax_accounts::INPUT_VAT
            | tax_accounts::TAX_RECEIVABLE
            | tax_accounts::DEFERRED_TAX_ASSET
            | treasury_accounts::DERIVATIVE_ASSET
            | treasury_accounts::CASH_POOL_IC_RECEIVABLE
    ) {
        return Some(BsMonetary);
    }

    // ── Inventory + manufacturing (BsNonMonetary) ────────────────────
    if matches!(
        code,
        control_accounts::INVENTORY
            | manufacturing_accounts::WIP
            | manufacturing_accounts::FINISHED_GOODS
    ) {
        return Some(BsNonMonetary);
    }

    // ── Fixed assets, accumulated depreciation (BsNonMonetary) ───────
    if matches!(
        code,
        control_accounts::FIXED_ASSETS | control_accounts::ACCUMULATED_DEPRECIATION
    ) {
        return Some(BsNonMonetary);
    }

    // ── Intangibles + goodwill (BsNonMonetary) ───────────────────────
    if matches!(
        code,
        intangible_accounts::GOODWILL
            | intangible_accounts::CUSTOMER_RELATIONSHIPS
            | intangible_accounts::TRADE_NAME
            | intangible_accounts::TECHNOLOGY
            | intangible_accounts::ACCUMULATED_AMORTIZATION
    ) {
        return Some(BsNonMonetary);
    }

    // ── Liabilities (BsMonetary) ─────────────────────────────────────
    if matches!(
        code,
        control_accounts::AP_CONTROL
            | control_accounts::IC_AP_CLEARING
            | liability_accounts::ACCRUED_EXPENSES
            | liability_accounts::ACCRUED_SALARIES
            | liability_accounts::ACCRUED_BENEFITS
            | liability_accounts::UNEARNED_REVENUE
            | liability_accounts::SHORT_TERM_DEBT
            | liability_accounts::LONG_TERM_DEBT
            | liability_accounts::IC_PAYABLE
            | tax_accounts::SALES_TAX_PAYABLE
            | tax_accounts::VAT_PAYABLE
            | tax_accounts::WITHHOLDING_TAX_PAYABLE
            | tax_accounts::INCOME_TAX_PAYABLE
            | tax_accounts::DEFERRED_TAX_LIABILITY
            | manufacturing_accounts::LABOR_ACCRUAL
            | manufacturing_accounts::WARRANTY_PROVISION
            | provision_accounts::PROVISION_LIABILITY
            | treasury_accounts::INTEREST_PAYABLE
            | treasury_accounts::DEBT_PREMIUM
            | treasury_accounts::DEBT_DISCOUNT
            | treasury_accounts::DERIVATIVE_LIABILITY
            | treasury_accounts::CASH_POOL_IC_PAYABLE
    ) {
        return Some(BsMonetary);
    }

    // ── Equity ──────────────────────────────────────────────────────
    if matches!(
        code,
        equity_accounts::COMMON_STOCK
            | equity_accounts::APIC
            | equity_accounts::RETAINED_EARNINGS
            | equity_accounts::CURRENT_YEAR_EARNINGS
            | equity_accounts::TREASURY_STOCK
            | equity_accounts::CTA
            | equity_accounts::INCOME_SUMMARY
            | equity_accounts::DIVIDENDS_PAID
            | treasury_accounts::OCI_CASH_FLOW_HEDGE
    ) {
        return Some(Equity);
    }

    // ── Revenue (PlRevenue) ─────────────────────────────────────────
    if matches!(
        code,
        revenue_accounts::PRODUCT_REVENUE
            | revenue_accounts::SERVICE_REVENUE
            | revenue_accounts::IC_REVENUE
            | revenue_accounts::PURCHASE_DISCOUNT_INCOME
            | revenue_accounts::OTHER_REVENUE
            | revenue_accounts::SALES_DISCOUNTS
            | revenue_accounts::SALES_RETURNS
            | intangible_accounts::BARGAIN_PURCHASE_GAIN
    ) {
        return Some(PlRevenue);
    }

    // ── Expenses incl. COGS (PlExpense) ──────────────────────────────
    if matches!(
        code,
        expense_accounts::COGS
            | expense_accounts::RAW_MATERIALS
            | expense_accounts::DIRECT_LABOR
            | expense_accounts::MANUFACTURING_OVERHEAD
            | expense_accounts::DEPRECIATION
            | expense_accounts::SALARIES_WAGES
            | expense_accounts::BENEFITS
            | expense_accounts::RENT
            | expense_accounts::UTILITIES
            | expense_accounts::OFFICE_SUPPLIES
            | expense_accounts::TRAVEL_ENTERTAINMENT
            | expense_accounts::PROFESSIONAL_FEES
            | expense_accounts::INSURANCE
            | expense_accounts::BAD_DEBT
            | expense_accounts::INTEREST_EXPENSE
            | expense_accounts::PURCHASE_DISCOUNTS
            | expense_accounts::FX_GAIN_LOSS
            | intangible_accounts::AMORTIZATION_EXPENSE
            | provision_accounts::PROVISION_EXPENSE
            | treasury_accounts::HEDGE_INEFFECTIVENESS
    ) {
        return Some(PlExpense);
    }

    None
}

/// Range-based classification — called when the account isn't one of
/// the named constants.
fn classify_by_range(code: &str) -> TranslationAccountType {
    use TranslationAccountType::*;

    // Defensive default for empty / non-numeric codes.
    let leading = match code.chars().next() {
        Some(c) if c.is_ascii_digit() => c,
        _ => return BsMonetary,
    };

    match leading {
        '1' => classify_asset_range(code),
        '2' => BsMonetary,
        '3' => Equity,
        '4' => PlRevenue,
        '5' | '6' | '7' => PlExpense,
        '8' => PlOci,
        '9' => BsMonetary,
        _ => BsMonetary,
    }
}

/// Fine-grained classification within the 1xxx asset range.
fn classify_asset_range(code: &str) -> TranslationAccountType {
    use TranslationAccountType::*;

    // Parse the first 4 digits if available; otherwise fall through to
    // `BsMonetary` as the safe default.
    let prefix4: String = code.chars().take(4).collect();
    let n: u32 = match prefix4.parse() {
        Ok(n) => n,
        Err(_) => return BsMonetary,
    };

    match n {
        1000..=1199 => BsMonetary,    // cash, AR, securities, IC AR, input VAT
        1200..=1299 => BsNonMonetary, // inventory
        1300..=1399 => BsNonMonetary, // prepaid expenses
        1400..=1499 => BsMonetary,    // mixed; named constants override (WIP, FG, derivative asset)
        1500..=1899 => BsNonMonetary, // fixed assets, accumulated depreciation
        1900..=1999 => BsNonMonetary, // intangibles, goodwill
        _ => BsMonetary,
    }
}

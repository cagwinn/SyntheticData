//! Integration tests for the IAS 21 monetary / non-monetary classifier
//! (Task 6.1).
//!
//! Two acceptance pillars:
//!
//! 1. Every named constant from [`datasynth_core::accounts`] used in
//!    the IC / FX flow lands in the correct [`TranslationAccountType`].
//! 2. The leading-digit range fall-back covers any unrecognised codes
//!    the orchestrator may emit.

use datasynth_core::accounts::{
    cash_accounts, control_accounts, equity_accounts, expense_accounts, intangible_accounts,
    liability_accounts, manufacturing_accounts, revenue_accounts, tax_accounts, treasury_accounts,
};
use datasynth_group::{classify_account, TranslationAccountType};
use datasynth_standards::framework::AccountingFramework;

/// Convenience: classify with the default framework — IAS 21 itself
/// doesn't differentiate, so framework is irrelevant for v5.0.
fn classify(code: &str) -> TranslationAccountType {
    classify_account(code, AccountingFramework::default())
}

// ── Named-constant classifications ───────────────────────────────────

#[test]
fn cash_accounts_are_bs_monetary() {
    // Cash, bank, petty cash, wire clearing — all closing-rate items.
    assert_eq!(
        classify(cash_accounts::OPERATING_CASH),
        TranslationAccountType::BsMonetary,
        "OPERATING_CASH (1000) must be BsMonetary",
    );
    assert_eq!(
        classify(cash_accounts::BANK_ACCOUNT),
        TranslationAccountType::BsMonetary,
    );
    assert_eq!(
        classify(cash_accounts::PETTY_CASH),
        TranslationAccountType::BsMonetary,
    );
    assert_eq!(
        classify(cash_accounts::WIRE_CLEARING),
        TranslationAccountType::BsMonetary,
    );
}

#[test]
fn ar_and_ic_ar_are_bs_monetary() {
    assert_eq!(
        classify(control_accounts::AR_CONTROL), // "1100"
        TranslationAccountType::BsMonetary,
    );
    assert_eq!(
        classify(control_accounts::IC_AR_CLEARING), // "1150"
        TranslationAccountType::BsMonetary,
    );
}

#[test]
fn inventory_is_bs_non_monetary() {
    assert_eq!(
        classify(control_accounts::INVENTORY), // "1200"
        TranslationAccountType::BsNonMonetary,
    );
    assert_eq!(
        classify(manufacturing_accounts::WIP), // "1420"
        TranslationAccountType::BsNonMonetary,
        "WIP must override the 1400-range BsMonetary default",
    );
    assert_eq!(
        classify(manufacturing_accounts::FINISHED_GOODS), // "1410"
        TranslationAccountType::BsNonMonetary,
    );
}

#[test]
fn fixed_assets_are_bs_non_monetary() {
    assert_eq!(
        classify(control_accounts::FIXED_ASSETS), // "1500"
        TranslationAccountType::BsNonMonetary,
    );
    assert_eq!(
        classify(control_accounts::ACCUMULATED_DEPRECIATION), // "1510"
        TranslationAccountType::BsNonMonetary,
    );
}

#[test]
fn intangibles_and_goodwill_are_bs_non_monetary() {
    assert_eq!(
        classify(intangible_accounts::GOODWILL), // "1900"
        TranslationAccountType::BsNonMonetary,
    );
    assert_eq!(
        classify(intangible_accounts::CUSTOMER_RELATIONSHIPS), // "1910"
        TranslationAccountType::BsNonMonetary,
    );
    assert_eq!(
        classify(intangible_accounts::TRADE_NAME), // "1920"
        TranslationAccountType::BsNonMonetary,
    );
    assert_eq!(
        classify(intangible_accounts::ACCUMULATED_AMORTIZATION), // "1950"
        TranslationAccountType::BsNonMonetary,
    );
}

#[test]
fn ap_and_ic_ap_are_bs_monetary() {
    assert_eq!(
        classify(control_accounts::AP_CONTROL), // "2000"
        TranslationAccountType::BsMonetary,
    );
    assert_eq!(
        classify(control_accounts::IC_AP_CLEARING), // "2050"
        TranslationAccountType::BsMonetary,
    );
}

#[test]
fn debt_and_accrued_liabilities_are_bs_monetary() {
    assert_eq!(
        classify(liability_accounts::SHORT_TERM_DEBT), // "2400"
        TranslationAccountType::BsMonetary,
    );
    assert_eq!(
        classify(liability_accounts::LONG_TERM_DEBT), // "2600"
        TranslationAccountType::BsMonetary,
    );
    assert_eq!(
        classify(liability_accounts::ACCRUED_EXPENSES), // "2200"
        TranslationAccountType::BsMonetary,
    );
    assert_eq!(
        classify(treasury_accounts::INTEREST_PAYABLE), // "2160"
        TranslationAccountType::BsMonetary,
    );
}

#[test]
fn equity_accounts_classify_as_equity() {
    assert_eq!(
        classify(equity_accounts::COMMON_STOCK), // "3000"
        TranslationAccountType::Equity,
    );
    assert_eq!(
        classify(equity_accounts::APIC), // "3100"
        TranslationAccountType::Equity,
    );
    assert_eq!(
        classify(equity_accounts::RETAINED_EARNINGS), // "3200"
        TranslationAccountType::Equity,
    );
    assert_eq!(
        classify(equity_accounts::CTA), // "3500"
        TranslationAccountType::Equity,
    );
}

#[test]
fn revenue_accounts_classify_as_pl_revenue() {
    assert_eq!(
        classify(revenue_accounts::PRODUCT_REVENUE), // "4000"
        TranslationAccountType::PlRevenue,
    );
    assert_eq!(
        classify(revenue_accounts::IC_REVENUE), // "4500"
        TranslationAccountType::PlRevenue,
    );
}

#[test]
fn cogs_and_expenses_classify_as_pl_expense() {
    assert_eq!(
        classify(expense_accounts::COGS), // "5000"
        TranslationAccountType::PlExpense,
    );
    assert_eq!(
        classify(expense_accounts::SALARIES_WAGES), // "6100"
        TranslationAccountType::PlExpense,
    );
    assert_eq!(
        classify(expense_accounts::INTEREST_EXPENSE), // "7100"
        TranslationAccountType::PlExpense,
    );
    assert_eq!(
        classify(expense_accounts::DEPRECIATION), // "6000"
        TranslationAccountType::PlExpense,
    );
}

#[test]
fn tax_accounts_classify_correctly() {
    // Receivable-side → BsMonetary.
    assert_eq!(
        classify(tax_accounts::INPUT_VAT), // "1160"
        TranslationAccountType::BsMonetary,
    );
    assert_eq!(
        classify(tax_accounts::DEFERRED_TAX_ASSET), // "1600"
        TranslationAccountType::BsMonetary,
    );
    // Payable-side → BsMonetary.
    assert_eq!(
        classify(tax_accounts::INCOME_TAX_PAYABLE), // "2130"
        TranslationAccountType::BsMonetary,
    );
    assert_eq!(
        classify(tax_accounts::DEFERRED_TAX_LIABILITY), // "2500"
        TranslationAccountType::BsMonetary,
    );
}

// ── Range-based fall-back ────────────────────────────────────────────

#[test]
fn unknown_1xxx_code_falls_back_by_range() {
    // 1099 — unknown cash-range code → BsMonetary.
    assert_eq!(classify("1099"), TranslationAccountType::BsMonetary);
    // 1250 — unknown inventory-range code → BsNonMonetary.
    assert_eq!(classify("1250"), TranslationAccountType::BsNonMonetary);
    // 1350 — prepaid range → BsNonMonetary.
    assert_eq!(classify("1350"), TranslationAccountType::BsNonMonetary);
    // 1700 — fixed-asset range → BsNonMonetary.
    assert_eq!(classify("1700"), TranslationAccountType::BsNonMonetary);
    // 1990 — intangible range → BsNonMonetary.
    assert_eq!(classify("1990"), TranslationAccountType::BsNonMonetary);
}

#[test]
fn unknown_2xxx_through_8xxx_codes_fall_back_by_range() {
    // 2999 → BsMonetary (liabilities).
    assert_eq!(classify("2999"), TranslationAccountType::BsMonetary);
    // 3999 → Equity.
    assert_eq!(classify("3999"), TranslationAccountType::Equity);
    // 4999 → PlRevenue.
    assert_eq!(classify("4999"), TranslationAccountType::PlRevenue);
    // 5999 → PlExpense.
    assert_eq!(classify("5999"), TranslationAccountType::PlExpense);
    // 6999 → PlExpense.
    assert_eq!(classify("6999"), TranslationAccountType::PlExpense);
    // 7999 → PlExpense.
    assert_eq!(classify("7999"), TranslationAccountType::PlExpense);
    // 8999 → PlOci.
    assert_eq!(classify("8999"), TranslationAccountType::PlOci);
    // 9000 → BsMonetary (memo / suspense default).
    assert_eq!(classify("9000"), TranslationAccountType::BsMonetary);
}

#[test]
fn empty_code_falls_back_to_bs_monetary() {
    assert_eq!(classify(""), TranslationAccountType::BsMonetary);
}

#[test]
fn malformed_code_falls_back_to_bs_monetary() {
    // Non-numeric — defensive default.
    assert_eq!(classify("ABC"), TranslationAccountType::BsMonetary);
    // Mixed prefix — leading char is not a digit.
    assert_eq!(classify("X1100"), TranslationAccountType::BsMonetary);
}

#[test]
fn classification_is_deterministic_across_calls() {
    // Two calls with the same input must produce the same output.
    let a1 = classify(control_accounts::AR_CONTROL);
    let a2 = classify(control_accounts::AR_CONTROL);
    assert_eq!(a1, a2);

    let i1 = classify(control_accounts::INVENTORY);
    let i2 = classify(control_accounts::INVENTORY);
    assert_eq!(i1, i2);
}

#[test]
fn framework_does_not_change_classification_in_v5_0() {
    // Per the module docs, all major frameworks classify these
    // canonical accounts identically for IAS 21 / ASC 830 purposes.
    let ar = control_accounts::AR_CONTROL;
    assert_eq!(
        classify_account(ar, AccountingFramework::UsGaap),
        classify_account(ar, AccountingFramework::Ifrs),
    );
    assert_eq!(
        classify_account(ar, AccountingFramework::FrenchGaap),
        classify_account(ar, AccountingFramework::GermanGaap),
    );
    assert_eq!(
        classify_account(ar, AccountingFramework::UsGaap),
        classify_account(ar, AccountingFramework::DualReporting),
    );
}

//! Integration tests for per-entity TB translation (Task 6.2).
//!
//! Acceptance:
//! - Identity case (functional == presentation) yields rate 1.0 and CTA 0.
//! - Foreign case picks the correct IAS 21 rate basis per account type.
//! - CTA equals total translated DR minus total translated CR.
//! - Missing FX rate surfaces as `GroupError::Aggregate`.
//! - Determinism — two calls produce byte-identical output.
//! - Mini-Nestlé hand-rolled fixture: USD → CHF translation produces
//!   the rates and amounts the Nestlé fixture documents.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_core::accounts::{
    cash_accounts, control_accounts, equity_accounts, expense_accounts, revenue_accounts,
};
use datasynth_core::models::balance::{
    AccountCategory, AccountType, TrialBalance, TrialBalanceLine, TrialBalanceType,
};
use datasynth_group::config::{FxPolicyConfig, FxRateBasis};
use datasynth_group::manifest::FxRateMaster;
use datasynth_group::{
    translate_entity_tb, DrCr, GroupError, RateBasis, TranslationAccountType,
};
use datasynth_standards::framework::AccountingFramework;

// ── Fixture builders ─────────────────────────────────────────────────

fn period_end() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
}

/// Default policy — irrelevant to translation logic but required by
/// `FxRateMaster`.
fn default_policy() -> FxPolicyConfig {
    FxPolicyConfig {
        balance_sheet: FxRateBasis::Closing,
        income_statement: FxRateBasis::Average,
        equity: FxRateBasis::Historical,
    }
}

/// Build an `FxRateMaster` for the Mini-Nestlé fixture's USD/CHF pair.
///
/// CHF/USD rates in the fixture:
///   2024-01-31 → 0.8870
///   2024-02-29 → 0.8812
///   2024-03-31 → 0.9012
///
/// After auto-inversion to canonical USD/CHF (functional / presentation):
///   2024-01-31 → 1/0.8870 ≈ 1.12740...
///   2024-02-29 → 1/0.8812 ≈ 1.13482...
///   2024-03-31 → 1/0.9012 ≈ 1.10963... (also the closing rate)
///   average     ≈ 1.12395...
fn nestle_fx_master_usd_chf() -> FxRateMaster {
    // Use exact rust_decimal math so test is reproducible.
    let rates_chf_usd = vec![
        (NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(), dec!(0.8870)),
        (NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(), dec!(0.8812)),
        (NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(), dec!(0.9012)),
    ];

    // Canonical pair "USD/CHF" — invert each rate.
    let mut usd_chf: BTreeMap<NaiveDate, Decimal> = BTreeMap::new();
    let mut sum = Decimal::ZERO;
    let mut closing = Decimal::ZERO;
    for (d, r) in &rates_chf_usd {
        let inv = Decimal::ONE / r;
        usd_chf.insert(*d, inv);
        sum += inv;
        closing = inv; // last iter wins (sorted by date)
    }
    let n = Decimal::from(rates_chf_usd.len() as u32);
    let average = sum / n;

    let mut rates = BTreeMap::new();
    rates.insert("USD/CHF".to_string(), usd_chf);

    let mut closing_by_pair = BTreeMap::new();
    closing_by_pair.insert("USD/CHF".to_string(), closing);

    let mut average_by_pair = BTreeMap::new();
    average_by_pair.insert("USD/CHF".to_string(), average);

    FxRateMaster {
        base_currency: "CHF".to_string(),
        policy: default_policy(),
        rates,
        closing_by_pair,
        average_by_pair,
    }
}

/// Empty FX master (no needed pairs) — used for identity-case tests.
fn empty_fx_master() -> FxRateMaster {
    FxRateMaster {
        base_currency: "USD".to_string(),
        policy: default_policy(),
        rates: BTreeMap::new(),
        closing_by_pair: BTreeMap::new(),
        average_by_pair: BTreeMap::new(),
    }
}

/// Hand-roll a small, balanced TB for a USD entity:
/// - DR cash 10,000
/// - DR AR 5,000
/// - DR COGS 4,000
/// - CR revenue 7,000
/// - CR AP 3,000
/// - CR retained earnings 9,000
fn build_usd_tb(company_code: &str) -> TrialBalance {
    let mut tb = TrialBalance::new(
        format!("TB_{company_code}"),
        company_code.to_string(),
        period_end(),
        2024,
        3,
        "USD".to_string(),
        TrialBalanceType::Adjusted,
    );

    let push_dr = |tb: &mut TrialBalance, code: &str, ty: AccountType, amt: Decimal| {
        tb.add_line(TrialBalanceLine {
            account_code: code.to_string(),
            account_description: code.to_string(),
            category: AccountCategory::from_account_type(ty),
            account_type: ty,
            opening_balance: Decimal::ZERO,
            period_debits: amt,
            period_credits: Decimal::ZERO,
            closing_balance: amt,
            debit_balance: amt,
            credit_balance: Decimal::ZERO,
            cost_center: None,
            profit_center: None,
        });
    };

    let push_cr = |tb: &mut TrialBalance, code: &str, ty: AccountType, amt: Decimal| {
        tb.add_line(TrialBalanceLine {
            account_code: code.to_string(),
            account_description: code.to_string(),
            category: AccountCategory::from_account_type(ty),
            account_type: ty,
            opening_balance: Decimal::ZERO,
            period_debits: Decimal::ZERO,
            period_credits: amt,
            closing_balance: amt,
            debit_balance: Decimal::ZERO,
            credit_balance: amt,
            cost_center: None,
            profit_center: None,
        });
    };

    push_dr(
        &mut tb,
        cash_accounts::OPERATING_CASH,
        AccountType::Asset,
        dec!(10000),
    );
    push_dr(
        &mut tb,
        control_accounts::AR_CONTROL,
        AccountType::Asset,
        dec!(5000),
    );
    push_dr(
        &mut tb,
        expense_accounts::COGS,
        AccountType::Expense,
        dec!(4000),
    );
    push_cr(
        &mut tb,
        revenue_accounts::PRODUCT_REVENUE,
        AccountType::Revenue,
        dec!(7000),
    );
    push_cr(
        &mut tb,
        control_accounts::AP_CONTROL,
        AccountType::Liability,
        dec!(3000),
    );
    push_cr(
        &mut tb,
        equity_accounts::RETAINED_EARNINGS,
        AccountType::Equity,
        dec!(9000),
    );

    tb
}

// ── Tests ────────────────────────────────────────────────────────────

#[test]
fn identity_translation_yields_rate_one_and_zero_cta() {
    // Functional == presentation (USD == USD).
    let tb = build_usd_tb("TEST_USD");
    let master = empty_fx_master();

    let out = translate_entity_tb(
        &tb,
        "USD",
        &master,
        period_end(),
        "USD",
        AccountingFramework::default(),
    )
    .expect("identity translation must succeed");

    assert_eq!(out.entity_code, "TEST_USD");
    assert_eq!(out.functional_currency, "USD");
    assert_eq!(out.presentation_currency, "USD");
    assert_eq!(out.lines.len(), tb.lines.len());

    // Every rate must be 1.0.
    for line in &out.lines {
        assert_eq!(line.fx_rate, Decimal::ONE, "identity rate must be 1.0");
        assert_eq!(line.translated_amount, line.local_amount);
    }

    // Source TB is balanced ⇒ CTA = 0.
    assert_eq!(out.cta, Decimal::ZERO);
    assert_eq!(
        out.total_translated_debits,
        out.total_translated_credits,
        "identity translation must preserve DR == CR"
    );
}

#[test]
fn foreign_translation_picks_correct_rate_basis() {
    let tb = build_usd_tb("NESTLE_USA");
    let master = nestle_fx_master_usd_chf();

    let out = translate_entity_tb(
        &tb,
        "USD",
        &master,
        period_end(),
        "CHF",
        AccountingFramework::default(),
    )
    .expect("USD → CHF translation must succeed");

    // Each line must carry the rate basis matching its account type.
    for line in &out.lines {
        let expected = match line.account_type {
            TranslationAccountType::BsMonetary => RateBasis::Closing,
            TranslationAccountType::BsNonMonetary | TranslationAccountType::Equity => {
                RateBasis::Historical
            }
            TranslationAccountType::PlRevenue
            | TranslationAccountType::PlExpense
            | TranslationAccountType::PlOci => RateBasis::Average,
        };
        assert_eq!(
            line.rate_basis, expected,
            "account {} ({:?}) must use {:?}",
            line.account_code, line.account_type, expected,
        );
    }

    // BS monetary lines (cash, AR, AP) use the closing rate.
    let cash_line = out
        .lines
        .iter()
        .find(|l| l.account_code == cash_accounts::OPERATING_CASH)
        .unwrap();
    assert_eq!(cash_line.rate_basis, RateBasis::Closing);
    assert_eq!(
        cash_line.fx_rate,
        *master.closing_by_pair.get("USD/CHF").unwrap()
    );

    // Equity line (retained earnings) uses historical (= average proxy).
    let re_line = out
        .lines
        .iter()
        .find(|l| l.account_code == equity_accounts::RETAINED_EARNINGS)
        .unwrap();
    assert_eq!(re_line.rate_basis, RateBasis::Historical);
    assert_eq!(
        re_line.fx_rate,
        *master.average_by_pair.get("USD/CHF").unwrap(),
        "historical rate proxied by average for v5.0"
    );

    // Revenue line uses average.
    let rev_line = out
        .lines
        .iter()
        .find(|l| l.account_code == revenue_accounts::PRODUCT_REVENUE)
        .unwrap();
    assert_eq!(rev_line.rate_basis, RateBasis::Average);
    assert_eq!(
        rev_line.fx_rate,
        *master.average_by_pair.get("USD/CHF").unwrap()
    );
}

#[test]
fn cta_equals_translated_dr_minus_cr() {
    let tb = build_usd_tb("NESTLE_USA");
    let master = nestle_fx_master_usd_chf();

    let out = translate_entity_tb(
        &tb,
        "USD",
        &master,
        period_end(),
        "CHF",
        AccountingFramework::default(),
    )
    .expect("USD → CHF translation must succeed");

    // CTA = DR - CR by definition.
    assert_eq!(
        out.cta,
        out.total_translated_debits - out.total_translated_credits,
    );

    // The DR / CR sums must individually be > 0 (we have both sides).
    assert!(out.total_translated_debits > Decimal::ZERO);
    assert!(out.total_translated_credits > Decimal::ZERO);

    // Closing > average for USD/CHF in this fixture (USD strengthened
    // toward period end), so cash + AR translate at a higher rate than
    // revenue + retained earnings, and CTA should be non-zero.
    assert_ne!(
        out.cta,
        Decimal::ZERO,
        "different rate bases ⇒ non-zero CTA"
    );
}

#[test]
fn missing_fx_rate_returns_aggregate_error() {
    let tb = build_usd_tb("NESTLE_USA");
    // Empty FX master → no rates for USD/CHF.
    let master = empty_fx_master();

    let result = translate_entity_tb(
        &tb,
        "USD",
        &master,
        period_end(),
        "CHF",
        AccountingFramework::default(),
    );

    let err = result.expect_err("missing FX rate must error out");
    match err {
        GroupError::Aggregate(msg) => {
            assert!(
                msg.contains("USD/CHF"),
                "error must name the missing pair: {msg}"
            );
        }
        other => panic!("expected GroupError::Aggregate, got {other:?}"),
    }
}

#[test]
fn determinism_two_calls_produce_byte_identical_output() {
    let tb = build_usd_tb("NESTLE_USA");
    let master = nestle_fx_master_usd_chf();

    let a = translate_entity_tb(
        &tb,
        "USD",
        &master,
        period_end(),
        "CHF",
        AccountingFramework::default(),
    )
    .unwrap();
    let b = translate_entity_tb(
        &tb,
        "USD",
        &master,
        period_end(),
        "CHF",
        AccountingFramework::default(),
    )
    .unwrap();

    let a_json = serde_json::to_string(&a).unwrap();
    let b_json = serde_json::to_string(&b).unwrap();
    assert_eq!(a_json, b_json, "two calls must produce identical output");
}

#[test]
fn mini_nestle_usd_translation_amounts_within_expected_range() {
    // USD entity with 10,000 cash. Closing rate USD/CHF = 1/0.9012
    // ≈ 1.1096... so cash translated ≈ 11,096 CHF.
    let tb = build_usd_tb("NESTLE_USA");
    let master = nestle_fx_master_usd_chf();

    let out = translate_entity_tb(
        &tb,
        "USD",
        &master,
        period_end(),
        "CHF",
        AccountingFramework::default(),
    )
    .expect("USD → CHF translation must succeed");

    let cash_line = out
        .lines
        .iter()
        .find(|l| l.account_code == cash_accounts::OPERATING_CASH)
        .unwrap();
    assert_eq!(cash_line.local_amount, dec!(10000));
    assert_eq!(cash_line.local_dr_cr, DrCr::Debit);
    // 10000 * (1/0.9012) ≈ 11096.32 CHF (rounded to 2 dp).
    assert!(
        cash_line.translated_amount > dec!(11000) && cash_line.translated_amount < dec!(11200),
        "cash translated should be ~11096 CHF, got {}",
        cash_line.translated_amount,
    );

    // Revenue line uses average rate ≈ 1.12395...
    // 7000 * 1.12395 ≈ 7867.66 CHF.
    let rev_line = out
        .lines
        .iter()
        .find(|l| l.account_code == revenue_accounts::PRODUCT_REVENUE)
        .unwrap();
    assert_eq!(rev_line.local_amount, dec!(7000));
    assert_eq!(rev_line.local_dr_cr, DrCr::Credit);
    assert!(
        rev_line.translated_amount > dec!(7800) && rev_line.translated_amount < dec!(7900),
        "revenue translated should be ~7868 CHF, got {}",
        rev_line.translated_amount,
    );

    // Output preserves input order.
    assert_eq!(out.lines.len(), tb.lines.len());
    for (i, line) in out.lines.iter().enumerate() {
        assert_eq!(line.account_code, tb.lines[i].account_code);
    }
}

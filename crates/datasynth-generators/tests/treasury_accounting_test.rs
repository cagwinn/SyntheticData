//! Tests for treasury accounting journal entry pipeline.

use chrono::NaiveDate;
use datasynth_core::accounts::{expense_accounts, treasury_accounts};
use datasynth_core::models::{
    CashPoolSweep, DebtInstrument, DebtType, EffectivenessMethod, HedgeInstrumentType,
    HedgeRelationship, HedgeType, HedgedItemType, HedgingInstrument, InterestRateType,
};
use datasynth_generators::treasury::TreasuryAccounting;
use rust_decimal::Decimal;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn term_loan(id: &str, principal: Decimal, rate: Decimal, maturity: NaiveDate) -> DebtInstrument {
    DebtInstrument::new(
        id,
        "C001",
        DebtType::TermLoan,
        "BigBank Corp",
        principal,
        "USD",
        rate,
        InterestRateType::Fixed,
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        maturity,
    )
}

fn fx_forward(id: &str, fair_value: Decimal) -> HedgingInstrument {
    HedgingInstrument::new(
        id,
        HedgeInstrumentType::FxForward,
        Decimal::from(1_000_000),
        "USD",
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(),
        "Counterparty A",
    )
    .with_fair_value(fair_value)
}

fn cash_flow_hedge_rel(instrument_id: &str, effective: bool, ineff_amt: Decimal) -> HedgeRelationship {
    let ratio = if effective {
        Decimal::new(95, 2) // 0.95 — within 80-125%
    } else {
        Decimal::new(130, 2) // 1.30 — outside corridor
    };
    HedgeRelationship::new(
        format!("HR-{instrument_id}"),
        HedgedItemType::ForecastedTransaction,
        "Forecasted EUR revenue",
        instrument_id,
        HedgeType::CashFlowHedge,
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        EffectivenessMethod::DollarOffset,
        ratio,
    )
    .with_ineffectiveness_amount(ineff_amt)
}

fn fair_value_hedge_rel(instrument_id: &str) -> HedgeRelationship {
    HedgeRelationship::new(
        format!("HR-{instrument_id}"),
        HedgedItemType::RecognizedAsset,
        "Recognized EUR receivable",
        instrument_id,
        HedgeType::FairValueHedge,
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        EffectivenessMethod::DollarOffset,
        Decimal::new(100, 2),
    )
}

// ---------------------------------------------------------------------------
// Debt interest tests
// ---------------------------------------------------------------------------

#[test]
fn test_interest_accrual_je() {
    let period_end = NaiveDate::from_ymd_opt(2024, 3, 31).unwrap();
    let loan = term_loan(
        "DEBT-001",
        Decimal::from(1_000_000),
        Decimal::new(4, 2), // 0.04 = 4%
        NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
    );
    let jes = TreasuryAccounting::generate_debt_jes(&[loan], period_end);

    assert_eq!(jes.len(), 1);
    let je = &jes[0];
    assert_eq!(je.lines.len(), 2);

    // Line 1: DR Interest Expense
    assert_eq!(je.lines[0].gl_account, expense_accounts::INTEREST_EXPENSE);
    assert!(je.lines[0].debit_amount > Decimal::ZERO);

    // Line 2: CR Interest Payable
    assert_eq!(je.lines[1].gl_account, treasury_accounts::INTEREST_PAYABLE);
    assert!(je.lines[1].credit_amount > Decimal::ZERO);
}

#[test]
fn test_interest_amount_calculation() {
    let period_end = NaiveDate::from_ymd_opt(2024, 6, 30).unwrap();
    let principal = Decimal::from(2_000_000);
    let rate = Decimal::new(6, 2); // 6%
    let loan = term_loan(
        "DEBT-002",
        principal,
        rate,
        NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
    );

    let jes = TreasuryAccounting::generate_debt_jes(&[loan], period_end);
    assert_eq!(jes.len(), 1);

    // Expected: 2_000_000 * 0.06 / 4 = 30_000.00
    let expected = Decimal::from(30_000);
    assert_eq!(jes[0].lines[0].debit_amount, expected);
    assert_eq!(jes[0].lines[1].credit_amount, expected);
}

#[test]
fn test_all_debt_jes_balanced() {
    let period_end = NaiveDate::from_ymd_opt(2024, 9, 30).unwrap();
    let loans = vec![
        term_loan(
            "DEBT-A",
            Decimal::from(500_000),
            Decimal::new(5, 2),
            NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(),
        ),
        term_loan(
            "DEBT-B",
            Decimal::from(1_500_000),
            Decimal::new(35, 3), // 0.035 = 3.5%
            NaiveDate::from_ymd_opt(2028, 6, 30).unwrap(),
        ),
        term_loan(
            "DEBT-C",
            Decimal::from(750_000),
            Decimal::new(7, 2),
            NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
        ),
    ];

    let jes = TreasuryAccounting::generate_debt_jes(&loans, period_end);
    assert_eq!(jes.len(), 3);

    for je in &jes {
        assert!(
            je.is_balanced(),
            "JE for {} is unbalanced: DR={} CR={}",
            je.description().unwrap_or("?"),
            je.total_debit(),
            je.total_credit()
        );
    }
}

#[test]
fn test_matured_debt_no_je() {
    // period_end is *after* maturity => no JE
    let period_end = NaiveDate::from_ymd_opt(2025, 6, 30).unwrap();
    let loan = term_loan(
        "DEBT-MATURED",
        Decimal::from(1_000_000),
        Decimal::new(5, 2),
        NaiveDate::from_ymd_opt(2025, 3, 31).unwrap(), // matured before period_end
    );

    let jes = TreasuryAccounting::generate_debt_jes(&[loan], period_end);
    assert!(jes.is_empty(), "Should not generate JE for matured debt");
}

// ---------------------------------------------------------------------------
// Hedge JE tests
// ---------------------------------------------------------------------------

#[test]
fn test_cash_flow_hedge_asset() {
    let period_end = NaiveDate::from_ymd_opt(2024, 6, 30).unwrap();
    let inst = fx_forward("HEDGE-001", Decimal::from(50_000)); // positive = asset
    let rel = cash_flow_hedge_rel("HEDGE-001", true, Decimal::ZERO);

    let jes = TreasuryAccounting::generate_hedge_jes(&[inst], &[rel], period_end);
    assert_eq!(jes.len(), 1);
    let je = &jes[0];
    assert!(je.is_balanced());

    // DR Derivative Asset, CR OCI
    assert_eq!(je.lines[0].gl_account, treasury_accounts::DERIVATIVE_ASSET);
    assert_eq!(je.lines[0].debit_amount, Decimal::from(50_000));
    assert_eq!(
        je.lines[1].gl_account,
        treasury_accounts::OCI_CASH_FLOW_HEDGE
    );
    assert_eq!(je.lines[1].credit_amount, Decimal::from(50_000));
}

#[test]
fn test_fair_value_hedge_liability() {
    let period_end = NaiveDate::from_ymd_opt(2024, 6, 30).unwrap();
    let inst = fx_forward("HEDGE-002", Decimal::from(-30_000)); // negative = liability
    let rel = fair_value_hedge_rel("HEDGE-002");

    let jes = TreasuryAccounting::generate_hedge_jes(&[inst], &[rel], period_end);
    assert_eq!(jes.len(), 1);
    let je = &jes[0];
    assert!(je.is_balanced());

    // DR FX Gain/Loss, CR Derivative Liability
    assert_eq!(je.lines[0].gl_account, expense_accounts::FX_GAIN_LOSS);
    assert_eq!(je.lines[0].debit_amount, Decimal::from(30_000));
    assert_eq!(
        je.lines[1].gl_account,
        treasury_accounts::DERIVATIVE_LIABILITY
    );
    assert_eq!(je.lines[1].credit_amount, Decimal::from(30_000));
}

#[test]
fn test_hedge_ineffectiveness_je() {
    let period_end = NaiveDate::from_ymd_opt(2024, 6, 30).unwrap();
    let inst = fx_forward("HEDGE-003", Decimal::from(40_000));
    let rel = cash_flow_hedge_rel("HEDGE-003", false, Decimal::from(5_000));

    let jes = TreasuryAccounting::generate_hedge_jes(&[inst], &[rel], period_end);
    // Should produce 2 JEs: main hedge + ineffectiveness
    assert_eq!(jes.len(), 2);

    let ineff_je = &jes[1];
    assert!(ineff_je.is_balanced());
    assert_eq!(
        ineff_je.lines[0].gl_account,
        treasury_accounts::HEDGE_INEFFECTIVENESS
    );
    assert_eq!(ineff_je.lines[0].debit_amount, Decimal::from(5_000));
    assert_eq!(
        ineff_je.lines[1].gl_account,
        treasury_accounts::OCI_CASH_FLOW_HEDGE
    );
    assert_eq!(ineff_je.lines[1].credit_amount, Decimal::from(5_000));
}

#[test]
fn test_zero_fair_value_no_je() {
    let period_end = NaiveDate::from_ymd_opt(2024, 6, 30).unwrap();
    let inst = fx_forward("HEDGE-ZERO", Decimal::ZERO);
    let jes = TreasuryAccounting::generate_hedge_jes(&[inst], &[], period_end);
    assert!(jes.is_empty(), "Zero fair value should produce no JE");
}

// ---------------------------------------------------------------------------
// Cash pool sweep tests
// ---------------------------------------------------------------------------

#[test]
fn test_sweep_je() {
    let sweep = CashPoolSweep {
        id: "SWP-001".to_string(),
        pool_id: "POOL-A".to_string(),
        date: NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
        from_account_id: "ACCT-100".to_string(),
        to_account_id: "ACCT-HDR".to_string(),
        amount: Decimal::from(250_000),
        currency: "EUR".to_string(),
    };

    let jes = TreasuryAccounting::generate_cash_pool_sweep_jes(&[sweep], "C001");
    assert_eq!(jes.len(), 1);
    let je = &jes[0];
    assert!(je.is_balanced());
    assert_eq!(je.company_code(), "C001");

    assert_eq!(
        je.lines[0].gl_account,
        treasury_accounts::CASH_POOL_IC_RECEIVABLE
    );
    assert_eq!(je.lines[0].debit_amount, Decimal::from(250_000));
    assert_eq!(
        je.lines[1].gl_account,
        treasury_accounts::CASH_POOL_IC_PAYABLE
    );
    assert_eq!(je.lines[1].credit_amount, Decimal::from(250_000));
}

#[test]
fn test_zero_sweep_no_je() {
    let sweep = CashPoolSweep {
        id: "SWP-ZERO".to_string(),
        pool_id: "POOL-B".to_string(),
        date: NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
        from_account_id: "ACCT-200".to_string(),
        to_account_id: "ACCT-HDR".to_string(),
        amount: Decimal::ZERO,
        currency: "USD".to_string(),
    };

    let jes = TreasuryAccounting::generate_cash_pool_sweep_jes(&[sweep], "C001");
    assert!(jes.is_empty(), "Zero sweep amount should produce no JE");
}

//! Task 8.3 — consolidated cash flow statement integration tests.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_group::{
    build_consolidated_cash_flow, AggregatedAccount, AggregatedTb, CashFlowInputs, CfLine,
};

fn period_start() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
}

fn period_end() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
}

fn aggregate_account(code: &str, debit: Decimal, credit: Decimal) -> AggregatedAccount {
    AggregatedAccount {
        account_code: code.to_string(),
        debit_total: debit,
        credit_total: credit,
        net_balance: debit - credit,
        contributing_entities: 1,
    }
}

/// Build a TB with cash, AR, AP, inventory at the given balances.
fn make_tb(cash: Decimal, ar: Decimal, ap: Decimal, inv: Decimal) -> AggregatedTb {
    let mut totals: BTreeMap<String, AggregatedAccount> = BTreeMap::new();
    if cash != Decimal::ZERO {
        totals.insert(
            "1000".to_string(),
            aggregate_account("1000", cash, Decimal::ZERO),
        );
    }
    if ar != Decimal::ZERO {
        totals.insert(
            "1100".to_string(),
            aggregate_account("1100", ar, Decimal::ZERO),
        );
    }
    if inv != Decimal::ZERO {
        totals.insert(
            "1200".to_string(),
            aggregate_account("1200", inv, Decimal::ZERO),
        );
    }
    if ap != Decimal::ZERO {
        totals.insert(
            "2000".to_string(),
            aggregate_account("2000", Decimal::ZERO, ap),
        );
    }

    let total_debits = cash + ar + inv;
    let total_credits = ap;

    AggregatedTb {
        group_id: "TEST_GROUP".to_string(),
        currency: "CHF".to_string(),
        as_of_date: period_end(),
        account_totals: totals,
        contributing_entities: vec!["E1".to_string()],
        deferred_entities: Vec::new(),
        total_debits,
        total_credits,
    }
}

fn line(label: &str, lines: &[CfLine]) -> Decimal {
    lines
        .iter()
        .find(|l| l.label == label)
        .map(|l| l.amount)
        .unwrap_or_else(|| panic!("expected line: {label}"))
}

#[test]
fn happy_path_with_opening_and_closing_tbs() {
    let prior = make_tb(dec!(500_000), dec!(100_000), dec!(80_000), dec!(50_000));
    // Cash up by 100k, AR up by 30k, AP up by 20k, inventory down by 10k.
    let current = make_tb(
        dec!(600_000),
        dec!(130_000),
        dec!(100_000),
        dec!(40_000),
    );

    let inputs = CashFlowInputs {
        post_elim_tb_current: &current,
        post_elim_tb_prior: Some(&prior),
        net_income: dec!(150_000),
        depreciation_amortization: dec!(20_000),
        impairment: Decimal::ZERO,
        capex: dec!(50_000),
        debt_issuance: dec!(0),
        debt_repayment: dec!(20_000),
        dividends_paid_to_owners: dec!(10_000),
        dividends_paid_to_nci: Decimal::ZERO,
        equity_issuance: Decimal::ZERO,
    };

    let cf = build_consolidated_cash_flow(&inputs, "TEST_GROUP", period_start(), period_end())
        .unwrap();

    // Operating: net income (150) + D&A (20) + ΔAR (-30) + ΔInv (+10) + ΔAP (+20).
    // = 150 + 20 - 30 + 10 + 20 = 170k.
    assert_eq!(cf.operating.subtotal, dec!(170_000));
    assert_eq!(line("Net income", &cf.operating.lines), dec!(150_000));
    assert_eq!(
        line("Depreciation and amortization", &cf.operating.lines),
        dec!(20_000)
    );
    assert_eq!(
        line("Change in trade receivables", &cf.operating.lines),
        dec!(-30_000)
    );
    assert_eq!(
        line("Change in inventory", &cf.operating.lines),
        dec!(10_000)
    );
    assert_eq!(
        line("Change in trade payables", &cf.operating.lines),
        dec!(20_000)
    );

    // Investing: -capex = -50k.
    assert_eq!(cf.investing.subtotal, dec!(-50_000));

    // Financing: -debt_repayment - dividends = -20k - 10k = -30k.
    assert_eq!(cf.financing.subtotal, dec!(-30_000));

    // Opening / closing cash.
    assert_eq!(cf.opening_cash, dec!(500_000));
    assert_eq!(cf.closing_cash, dec!(600_000));
    // Net change = 170 - 50 - 30 = 90k.
    assert_eq!(cf.net_change_in_cash, dec!(90_000));
    // FX effect = 600 - 500 - 90 = 10k.
    assert_eq!(cf.fx_effect_on_cash, dec!(10_000));

    assert_eq!(cf.currency, "CHF");
    assert_eq!(cf.group_id, "TEST_GROUP");
    assert_eq!(cf.period_start, period_start());
    assert_eq!(cf.period_end, period_end());
}

#[test]
fn no_prior_tb_zero_working_capital_and_opening() {
    let current = make_tb(dec!(100_000), dec!(50_000), dec!(20_000), dec!(10_000));
    let inputs = CashFlowInputs {
        post_elim_tb_current: &current,
        post_elim_tb_prior: None,
        net_income: dec!(50_000),
        depreciation_amortization: Decimal::ZERO,
        impairment: Decimal::ZERO,
        capex: Decimal::ZERO,
        debt_issuance: Decimal::ZERO,
        debt_repayment: Decimal::ZERO,
        dividends_paid_to_owners: Decimal::ZERO,
        dividends_paid_to_nci: Decimal::ZERO,
        equity_issuance: Decimal::ZERO,
    };
    let cf = build_consolidated_cash_flow(&inputs, "T", period_start(), period_end()).unwrap();
    // Operating only contains net income — no working-capital lines.
    assert_eq!(cf.operating.lines.len(), 1);
    assert_eq!(cf.operating.subtotal, dec!(50_000));
    // Opening cash = 0.
    assert_eq!(cf.opening_cash, Decimal::ZERO);
    assert_eq!(cf.closing_cash, dec!(100_000));
    // FX effect = 100 - 0 - 50 = 50.
    assert_eq!(cf.fx_effect_on_cash, dec!(50_000));
}

#[test]
fn fx_effect_plug_absorbs_residual() {
    // Synthesize a scenario where cash changes more than the
    // operating / investing / financing flows imply, so the FX plug
    // captures the diff.
    let prior = make_tb(dec!(1_000_000), Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    let current = make_tb(dec!(1_500_000), Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    let inputs = CashFlowInputs {
        post_elim_tb_current: &current,
        post_elim_tb_prior: Some(&prior),
        net_income: dec!(100_000),
        depreciation_amortization: Decimal::ZERO,
        impairment: Decimal::ZERO,
        capex: Decimal::ZERO,
        debt_issuance: Decimal::ZERO,
        debt_repayment: Decimal::ZERO,
        dividends_paid_to_owners: Decimal::ZERO,
        dividends_paid_to_nci: Decimal::ZERO,
        equity_issuance: Decimal::ZERO,
    };
    let cf = build_consolidated_cash_flow(&inputs, "T", period_start(), period_end()).unwrap();
    // Cash up by 500k.  Net change = 100k (just net income).
    // Plug must be 500k - 100k = 400k.
    assert_eq!(cf.fx_effect_on_cash, dec!(400_000));
}

#[test]
fn each_section_subtotal_equals_sum_of_lines() {
    let current = make_tb(dec!(100_000), dec!(50_000), dec!(20_000), dec!(10_000));
    let inputs = CashFlowInputs {
        post_elim_tb_current: &current,
        post_elim_tb_prior: None,
        net_income: dec!(50_000),
        depreciation_amortization: dec!(10_000),
        impairment: dec!(5_000),
        capex: dec!(15_000),
        debt_issuance: dec!(40_000),
        debt_repayment: dec!(5_000),
        dividends_paid_to_owners: dec!(2_000),
        dividends_paid_to_nci: dec!(1_000),
        equity_issuance: dec!(3_000),
    };
    let cf = build_consolidated_cash_flow(&inputs, "T", period_start(), period_end()).unwrap();

    // Each section subtotal must equal the sum of its lines.
    let op_sum = cf
        .operating
        .lines
        .iter()
        .map(|l| l.amount)
        .fold(Decimal::ZERO, |acc, v| acc + v);
    assert_eq!(cf.operating.subtotal, op_sum);
    let inv_sum = cf
        .investing
        .lines
        .iter()
        .map(|l| l.amount)
        .fold(Decimal::ZERO, |acc, v| acc + v);
    assert_eq!(cf.investing.subtotal, inv_sum);
    let fin_sum = cf
        .financing
        .lines
        .iter()
        .map(|l| l.amount)
        .fold(Decimal::ZERO, |acc, v| acc + v);
    assert_eq!(cf.financing.subtotal, fin_sum);
}

#[test]
fn determinism_two_calls_match() {
    let prior = make_tb(dec!(100_000), dec!(50_000), dec!(20_000), dec!(10_000));
    let current = make_tb(dec!(120_000), dec!(60_000), dec!(25_000), dec!(15_000));
    let inputs_a = CashFlowInputs {
        post_elim_tb_current: &current,
        post_elim_tb_prior: Some(&prior),
        net_income: dec!(50_000),
        depreciation_amortization: dec!(10_000),
        impairment: Decimal::ZERO,
        capex: dec!(15_000),
        debt_issuance: Decimal::ZERO,
        debt_repayment: Decimal::ZERO,
        dividends_paid_to_owners: dec!(2_000),
        dividends_paid_to_nci: dec!(1_000),
        equity_issuance: Decimal::ZERO,
    };
    let inputs_b = CashFlowInputs {
        post_elim_tb_current: &current,
        post_elim_tb_prior: Some(&prior),
        net_income: dec!(50_000),
        depreciation_amortization: dec!(10_000),
        impairment: Decimal::ZERO,
        capex: dec!(15_000),
        debt_issuance: Decimal::ZERO,
        debt_repayment: Decimal::ZERO,
        dividends_paid_to_owners: dec!(2_000),
        dividends_paid_to_nci: dec!(1_000),
        equity_issuance: Decimal::ZERO,
    };
    let cf_a = build_consolidated_cash_flow(&inputs_a, "T", period_start(), period_end()).unwrap();
    let cf_b = build_consolidated_cash_flow(&inputs_b, "T", period_start(), period_end()).unwrap();
    assert_eq!(cf_a, cf_b, "two calls with same inputs must match");
}

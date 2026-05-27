//! Task 8.2 — consolidated income statement integration tests.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_group::{
    build_consolidated_income_statement, AggregatedAccount, AggregatedTb, NciRollforward,
};

fn period_end() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
}

fn account_type_for_us_code(code: &str) -> datasynth_core::models::balance::AccountType {
    use datasynth_core::models::balance::AccountType;
    match code.chars().next() {
        Some('1') => AccountType::Asset,
        Some('2') => AccountType::Liability,
        Some('3') => AccountType::Equity,
        Some('4') => AccountType::Revenue,
        Some('5') | Some('6') | Some('7') | Some('8') | Some('9') => AccountType::Expense,
        _ => AccountType::Asset,
    }
}

fn aggregate_account(code: &str, debit: Decimal, credit: Decimal) -> AggregatedAccount {
    AggregatedAccount {
        account_code: code.to_string(),
        debit_total: debit,
        credit_total: credit,
        net_balance: debit - credit,
        contributing_entities: 1,
        account_type: account_type_for_us_code(code),
    }
}

/// Build a TB with revenue, COGS, opex, and tax — a happy P&L shape.
fn happy_pl_tb() -> AggregatedTb {
    let mut totals: BTreeMap<String, AggregatedAccount> = BTreeMap::new();
    // Revenue: 1_000_000 product + 200_000 service
    totals.insert(
        "4000".to_string(),
        aggregate_account("4000", Decimal::ZERO, dec!(1_000_000)),
    );
    totals.insert(
        "4100".to_string(),
        aggregate_account("4100", Decimal::ZERO, dec!(200_000)),
    );
    // COGS: 600_000
    totals.insert(
        "5000".to_string(),
        aggregate_account("5000", dec!(600_000), Decimal::ZERO),
    );
    // Operating expenses: 200_000 salaries
    totals.insert(
        "6100".to_string(),
        aggregate_account("6100", dec!(200_000), Decimal::ZERO),
    );
    // Other: 50_000 share of profit of associates (4900 — credit-positive
    // sign convention: classify_is_section says "other", debit-positive
    // amount = -50000 — so we make 4900 a *gain* by putting it on the
    // credit side, and the amount comes out negative; net contribution
    // to profit is therefore -(-50000) = +50000 added at the IS level).
    //
    // To keep the test arithmetic legible we instead provide a 7100
    // interest expense of 30_000 (debit-positive +30000) and a 4900
    // share-of-profit of 50_000 (credit, amount = -50_000 in the
    // debit-positive sign).  Operating income reduces by sum(other)
    // = -50_000 + 30_000 = -20_000, so net_income_before_tax =
    // operating_income - (-20_000) = operating_income + 20_000.  The
    // separate 4900-only test below verifies the share-of-profit path.
    totals.insert(
        "7100".to_string(),
        aggregate_account("7100", dec!(30_000), Decimal::ZERO),
    );
    // Tax: 50_000
    totals.insert(
        "8000".to_string(),
        aggregate_account("8000", dec!(50_000), Decimal::ZERO),
    );

    // Total debits = 600 + 200 + 30 + 50 = 880_000
    // Total credits = 1_000 + 200 = 1_200_000
    // (P&L isn't required to balance — that only matters in the TB
    // aggregate; here we're just building the post-elim TB shape.)

    AggregatedTb {
        group_id: "TEST_GROUP".to_string(),
        currency: "CHF".to_string(),
        as_of_date: period_end(),
        account_totals: totals,
        contributing_entities: vec!["E1".to_string()],
        deferred_entities: Vec::new(),
        total_debits: dec!(880_000),
        total_credits: dec!(1_200_000),
    }
}

#[test]
fn happy_path_revenue_cogs_opex() {
    let tb = happy_pl_tb();
    let is = build_consolidated_income_statement(&tb, &[], "TEST_GROUP", period_end()).unwrap();

    assert_eq!(is.revenue.len(), 2);
    assert_eq!(is.cost_of_goods_sold.len(), 1);
    assert_eq!(is.operating_expenses.len(), 1);
    // Revenue + COGS (sum revenue = 1.2M, sum cogs = 0.6M, gross = 0.6M).
    assert_eq!(is.gross_profit, dec!(600_000));
    // Operating income = gross - opex = 600k - 200k = 400k.
    assert_eq!(is.operating_income, dec!(400_000));
    // Other includes 7100 interest expense (net-income contribution = -30k).
    assert_eq!(is.other_income_expense.len(), 1);
    // net_income_before_tax = 400k + (-30k) = 370k.
    assert_eq!(is.net_income_before_tax, dec!(370_000));
    // Tax = 50k.
    assert_eq!(is.tax_expense, dec!(50_000));
    // Net income = 370k - 50k = 320k.
    assert_eq!(is.net_income, dec!(320_000));
    // No NCI — entire net income to owners.
    assert_eq!(is.net_income_to_nci, Decimal::ZERO);
    assert_eq!(is.net_income_to_owners, dec!(320_000));

    assert_eq!(is.currency, "CHF");
    assert_eq!(is.group_id, "TEST_GROUP");
    assert_eq!(is.period_end, period_end());
}

#[test]
fn nci_split_sums_to_total_net_income() {
    let tb = happy_pl_tb();
    let nci = vec![
        NciRollforward {
            entity_code: "SUB1".to_string(),
            parent_entity_code: "PARENT".to_string(),
            ownership_percent: dec!(0.80),
            nci_percent: dec!(0.20),
            opening_nci: Decimal::ZERO,
            nci_share_of_profit: dec!(40_000),
            nci_share_of_oci: Decimal::ZERO,
            nci_dividends: Decimal::ZERO,
            equity_transaction_adjustments: Decimal::ZERO,
            pl_remeasurement_gain_or_loss: Decimal::ZERO,
            closing_nci: dec!(40_000),
            period_end: period_end(),
            currency: "CHF".to_string(),
        },
        NciRollforward {
            entity_code: "SUB2".to_string(),
            parent_entity_code: "PARENT".to_string(),
            ownership_percent: dec!(0.70),
            nci_percent: dec!(0.30),
            opening_nci: Decimal::ZERO,
            nci_share_of_profit: dec!(20_000),
            nci_share_of_oci: Decimal::ZERO,
            nci_dividends: Decimal::ZERO,
            equity_transaction_adjustments: Decimal::ZERO,
            pl_remeasurement_gain_or_loss: Decimal::ZERO,
            closing_nci: dec!(20_000),
            period_end: period_end(),
            currency: "CHF".to_string(),
        },
    ];

    let is = build_consolidated_income_statement(&tb, &nci, "TEST_GROUP", period_end()).unwrap();
    // 40k + 20k = 60k NCI.
    assert_eq!(is.net_income_to_nci, dec!(60_000));
    // Owners = 320k - 60k = 260k.
    assert_eq!(is.net_income_to_owners, dec!(260_000));
    // Owners + NCI == total net_income.
    assert_eq!(
        is.net_income_to_owners + is.net_income_to_nci,
        is.net_income
    );
}

#[test]
fn empty_tb_yields_zero_net_income() {
    let tb = AggregatedTb {
        group_id: "EMPTY".to_string(),
        currency: "EUR".to_string(),
        as_of_date: period_end(),
        account_totals: BTreeMap::new(),
        contributing_entities: Vec::new(),
        deferred_entities: Vec::new(),
        total_debits: Decimal::ZERO,
        total_credits: Decimal::ZERO,
    };

    let is = build_consolidated_income_statement(&tb, &[], "EMPTY", period_end()).unwrap();
    assert!(is.revenue.is_empty());
    assert!(is.cost_of_goods_sold.is_empty());
    assert!(is.operating_expenses.is_empty());
    assert!(is.other_income_expense.is_empty());
    assert_eq!(is.gross_profit, Decimal::ZERO);
    assert_eq!(is.operating_income, Decimal::ZERO);
    assert_eq!(is.net_income_before_tax, Decimal::ZERO);
    assert_eq!(is.tax_expense, Decimal::ZERO);
    assert_eq!(is.net_income, Decimal::ZERO);
    assert_eq!(is.net_income_to_owners, Decimal::ZERO);
    assert_eq!(is.net_income_to_nci, Decimal::ZERO);
}

#[test]
fn share_of_profit_of_associates_in_other() {
    let mut totals: BTreeMap<String, AggregatedAccount> = BTreeMap::new();
    // Just one revenue line and one share-of-profit line to isolate
    // 4900's behaviour.
    totals.insert(
        "4000".to_string(),
        aggregate_account("4000", Decimal::ZERO, dec!(100_000)),
    );
    // 4900 (share of profit) on the credit side — under the
    // net-income-contribution convention this becomes amount = +50_000
    // (a gain that adds to net income).
    totals.insert(
        "4900".to_string(),
        aggregate_account("4900", Decimal::ZERO, dec!(50_000)),
    );

    let tb = AggregatedTb {
        group_id: "T".to_string(),
        currency: "CHF".to_string(),
        as_of_date: period_end(),
        account_totals: totals,
        contributing_entities: Vec::new(),
        deferred_entities: Vec::new(),
        total_debits: Decimal::ZERO,
        total_credits: dec!(150_000),
    };
    let is = build_consolidated_income_statement(&tb, &[], "T", period_end()).unwrap();

    // 4900 must be in other_income_expense.
    assert!(
        is.other_income_expense
            .iter()
            .any(|l| l.account_code == "4900"),
        "4900 must appear in other_income_expense"
    );
    let four_nine = is
        .other_income_expense
        .iter()
        .find(|l| l.account_code == "4900")
        .unwrap();
    // Net-income contribution = credit - debit = 50_000.
    assert_eq!(four_nine.amount, dec!(50_000));

    // Operating income = 100_000 (no COGS / opex), then
    // net_income_before_tax = 100_000 + 50_000 = 150_000.
    assert_eq!(is.net_income_before_tax, dec!(150_000));
}

#[test]
fn determinism_two_calls_match() {
    let tb = happy_pl_tb();
    let is_a = build_consolidated_income_statement(&tb, &[], "TEST_GROUP", period_end()).unwrap();
    let is_b = build_consolidated_income_statement(&tb, &[], "TEST_GROUP", period_end()).unwrap();
    assert_eq!(is_a, is_b, "two calls with same input must be identical");
}

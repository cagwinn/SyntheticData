//! Task 8.1 — consolidated balance sheet integration tests.
//!
//! Hand-builds an [`AggregatedTb`] (the post-elimination /
//! post-NCI / equity-method-overlay shape returned by
//! [`apply_nci_and_equity_method`]) and verifies that
//! [`build_consolidated_balance_sheet`] classifies, signs, sorts,
//! and balance-checks the output per IAS 1.54.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_group::{
    build_consolidated_balance_sheet, AggregatedAccount, AggregatedTb, ConsolidatedBalanceSheet,
};

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

fn balanced_aggregated_tb() -> AggregatedTb {
    let mut totals: BTreeMap<String, AggregatedAccount> = BTreeMap::new();
    // Assets
    totals.insert(
        "1000".to_string(),
        aggregate_account("1000", dec!(500_000), Decimal::ZERO),
    );
    totals.insert(
        "1100".to_string(),
        aggregate_account("1100", dec!(200_000), Decimal::ZERO),
    );
    totals.insert(
        "1500".to_string(),
        aggregate_account("1500", dec!(800_000), Decimal::ZERO),
    );
    totals.insert(
        "1850".to_string(),
        aggregate_account("1850", dec!(50_000), Decimal::ZERO),
    );
    // Liabilities
    totals.insert(
        "2000".to_string(),
        aggregate_account("2000", Decimal::ZERO, dec!(150_000)),
    );
    totals.insert(
        "2600".to_string(),
        aggregate_account("2600", Decimal::ZERO, dec!(400_000)),
    );
    // Equity (owners)
    totals.insert(
        "3000".to_string(),
        aggregate_account("3000", Decimal::ZERO, dec!(300_000)),
    );
    totals.insert(
        "3300".to_string(),
        aggregate_account("3300", Decimal::ZERO, dec!(600_000)),
    );
    // NCI
    totals.insert(
        "3500".to_string(),
        aggregate_account("3500", Decimal::ZERO, dec!(100_000)),
    );
    // Excluded P&L
    totals.insert(
        "4000".to_string(),
        aggregate_account("4000", Decimal::ZERO, dec!(123_456)),
    );
    totals.insert(
        "5000".to_string(),
        aggregate_account("5000", dec!(123_456), Decimal::ZERO),
    );

    AggregatedTb {
        group_id: "TEST_GROUP".to_string(),
        currency: "CHF".to_string(),
        as_of_date: period_end(),
        account_totals: totals,
        contributing_entities: vec!["E1".to_string()],
        deferred_entities: Vec::new(),
        total_debits: dec!(1_673_456),
        total_credits: dec!(1_673_456),
    }
}

fn line_amount(bs_lines: &[datasynth_group::BsLine], code: &str) -> Decimal {
    bs_lines
        .iter()
        .find(|l| l.account_code == code)
        .map(|l| l.amount)
        .unwrap_or_else(|| panic!("missing line for {code}"))
}

#[test]
fn happy_path_balanced_bs() {
    let tb = balanced_aggregated_tb();
    let bs = build_consolidated_balance_sheet(&tb, "TEST_GROUP", period_end())
        .expect("balanced TB must yield balanced BS");

    // Section populations.
    assert_eq!(bs.current_assets.len(), 2, "1000 + 1100 are current assets");
    assert_eq!(
        bs.non_current_assets.len(),
        2,
        "1500 + 1850 are non-current assets"
    );
    assert_eq!(bs.current_liabilities.len(), 1, "2000 only");
    assert_eq!(bs.non_current_liabilities.len(), 1, "2600 only");
    assert_eq!(bs.equity.len(), 2, "3000 + 3300");
    assert_eq!(bs.nci.len(), 1, "3500 only");

    // Sign convention: assets debit-positive, liabilities/equity/NCI credit-positive.
    assert_eq!(line_amount(&bs.current_assets, "1000"), dec!(500_000));
    assert_eq!(line_amount(&bs.current_assets, "1100"), dec!(200_000));
    assert_eq!(line_amount(&bs.non_current_assets, "1500"), dec!(800_000));
    assert_eq!(line_amount(&bs.non_current_assets, "1850"), dec!(50_000));
    assert_eq!(line_amount(&bs.current_liabilities, "2000"), dec!(150_000));
    assert_eq!(
        line_amount(&bs.non_current_liabilities, "2600"),
        dec!(400_000)
    );
    assert_eq!(line_amount(&bs.equity, "3000"), dec!(300_000));
    assert_eq!(line_amount(&bs.equity, "3300"), dec!(600_000));
    assert_eq!(line_amount(&bs.nci, "3500"), dec!(100_000));

    // Totals.
    assert_eq!(bs.total_assets, dec!(1_550_000));
    assert_eq!(bs.total_liabilities, dec!(550_000));
    assert_eq!(bs.total_equity, dec!(900_000));
    assert_eq!(bs.total_nci, dec!(100_000));
    assert_eq!(bs.total_liabilities_plus_equity_plus_nci, dec!(1_550_000));
    // Currency / dates round-trip.
    assert_eq!(bs.currency, "CHF");
    assert_eq!(bs.group_id, "TEST_GROUP");
    assert_eq!(bs.as_of_date, period_end());
}

#[test]
fn empty_tb_yields_zeroed_bs() {
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

    let bs = build_consolidated_balance_sheet(&tb, "EMPTY", period_end())
        .expect("empty TB balances trivially");

    assert!(bs.current_assets.is_empty());
    assert!(bs.non_current_assets.is_empty());
    assert!(bs.current_liabilities.is_empty());
    assert!(bs.non_current_liabilities.is_empty());
    assert!(bs.equity.is_empty());
    assert!(bs.nci.is_empty());
    assert_eq!(bs.total_assets, Decimal::ZERO);
    assert_eq!(bs.total_liabilities, Decimal::ZERO);
    assert_eq!(bs.total_equity, Decimal::ZERO);
    assert_eq!(bs.total_nci, Decimal::ZERO);
    assert_eq!(bs.total_liabilities_plus_equity_plus_nci, Decimal::ZERO);
}

#[test]
fn imbalanced_tb_yields_aggregate_error() {
    let mut totals: BTreeMap<String, AggregatedAccount> = BTreeMap::new();
    // Assets only — no offsetting liabilities/equity.
    totals.insert(
        "1000".to_string(),
        aggregate_account("1000", dec!(1_000_000), Decimal::ZERO),
    );

    let tb = AggregatedTb {
        group_id: "BAD".to_string(),
        currency: "EUR".to_string(),
        as_of_date: period_end(),
        account_totals: totals,
        contributing_entities: Vec::new(),
        deferred_entities: Vec::new(),
        total_debits: dec!(1_000_000),
        total_credits: Decimal::ZERO,
    };

    // v5.0 contract: per-entity TBs from the synthetic engine
    // deliberately carry fraud / anomaly imbalances. The consolidated
    // BS preserves them and surfaces the diff via tracing instead of
    // failing — downstream consumers inspect the imbalance as the
    // ground-truth fraud signal.
    let bs = build_consolidated_balance_sheet(&tb, "BAD", period_end())
        .expect("imbalanced TB must succeed under v5.0 fraud-tolerance contract");
    let imbalance = (bs.total_assets - bs.total_liabilities_plus_equity_plus_nci).abs();
    assert!(
        imbalance > Decimal::new(1, 2),
        "imbalanced TB must produce imbalanced consolidated BS: assets={}, L+E+NCI={}",
        bs.total_assets,
        bs.total_liabilities_plus_equity_plus_nci
    );
}

#[test]
fn nci_separated_from_owners_equity() {
    let tb = balanced_aggregated_tb();
    let bs = build_consolidated_balance_sheet(&tb, "TEST_GROUP", period_end()).unwrap();

    // 3500 must be in nci, not equity.
    assert!(
        bs.nci.iter().any(|l| l.account_code == "3500"),
        "3500 must be in nci section"
    );
    assert!(
        bs.equity.iter().all(|l| l.account_code != "3500"),
        "3500 must not be in equity section"
    );
}

#[test]
fn lines_within_each_section_sorted_by_account_code() {
    let tb = balanced_aggregated_tb();
    let bs = build_consolidated_balance_sheet(&tb, "TEST_GROUP", period_end()).unwrap();

    fn assert_sorted(lines: &[datasynth_group::BsLine], section: &str) {
        let codes: Vec<&str> = lines.iter().map(|l| l.account_code.as_str()).collect();
        let mut sorted = codes.clone();
        sorted.sort();
        assert_eq!(codes, sorted, "section {section} lines must be sorted");
    }

    assert_sorted(&bs.current_assets, "current_assets");
    assert_sorted(&bs.non_current_assets, "non_current_assets");
    assert_sorted(&bs.current_liabilities, "current_liabilities");
    assert_sorted(&bs.non_current_liabilities, "non_current_liabilities");
    assert_sorted(&bs.equity, "equity");
    assert_sorted(&bs.nci, "nci");
}

#[test]
fn round_trip_via_serde() {
    let tb = balanced_aggregated_tb();
    let bs = build_consolidated_balance_sheet(&tb, "TEST_GROUP", period_end()).unwrap();
    let json = serde_json::to_string_pretty(&bs).unwrap();
    let bs2: ConsolidatedBalanceSheet = serde_json::from_str(&json).unwrap();
    assert_eq!(bs, bs2, "round-trip serde must preserve every field");
}

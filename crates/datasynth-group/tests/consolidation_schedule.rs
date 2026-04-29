//! Task 8.5 — consolidation schedule integration tests.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_core::models::balance::{
    AccountCategory, AccountType, TrialBalance, TrialBalanceLine, TrialBalanceType,
};

use datasynth_group::{
    build_consolidation_schedule, AggregatedAccount, AggregatedTb, ConsolidationSchedule,
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

fn make_aggregated_tb(currency: &str, accounts: &[(&str, Decimal, Decimal)]) -> AggregatedTb {
    let mut totals: BTreeMap<String, AggregatedAccount> = BTreeMap::new();
    let (mut td, mut tc) = (Decimal::ZERO, Decimal::ZERO);
    for (code, debit, credit) in accounts {
        totals.insert(code.to_string(), aggregate_account(code, *debit, *credit));
        td += *debit;
        tc += *credit;
    }
    AggregatedTb {
        group_id: "TEST_GROUP".to_string(),
        currency: currency.to_string(),
        as_of_date: period_end(),
        account_totals: totals,
        contributing_entities: vec!["E1".to_string(), "E2".to_string()],
        deferred_entities: Vec::new(),
        total_debits: td,
        total_credits: tc,
    }
}

fn make_entity_tb(
    company_code: &str,
    currency: &str,
    lines: &[(&str, Decimal, Decimal)],
) -> TrialBalance {
    let mut tb = TrialBalance::new(
        format!("TB-{company_code}-2024-03"),
        company_code.to_string(),
        period_end(),
        2024,
        3,
        currency.to_string(),
        TrialBalanceType::Adjusted,
    );
    for (account_code, debit, credit) in lines {
        tb.add_line(TrialBalanceLine {
            account_code: (*account_code).to_string(),
            account_description: format!("Line {account_code}"),
            category: AccountCategory::CurrentAssets,
            account_type: AccountType::Asset,
            opening_balance: Decimal::ZERO,
            period_debits: *debit,
            period_credits: *credit,
            closing_balance: *debit - *credit,
            debit_balance: *debit,
            credit_balance: *credit,
            cost_center: None,
            profit_center: None,
        });
    }
    tb
}

fn line_for<'a>(s: &'a ConsolidationSchedule, code: &str) -> &'a datasynth_group::ScheduleLine {
    s.lines
        .iter()
        .find(|l| l.account_code == code)
        .unwrap_or_else(|| panic!("expected line for code {code}"))
}

#[test]
fn happy_path_pre_adjustment_post() {
    // Pre-elim: 1100 = 100k, 2000 = -50k, 3300 = -50k
    let pre = make_aggregated_tb(
        "CHF",
        &[
            ("1100", dec!(100_000), Decimal::ZERO),
            ("2000", Decimal::ZERO, dec!(50_000)),
            ("3300", Decimal::ZERO, dec!(50_000)),
        ],
    );
    // Post-elim: 1100 = 70k (eliminated 30k), 2000 still -50k, 3300 -50k.
    let post = make_aggregated_tb(
        "CHF",
        &[
            ("1100", dec!(70_000), Decimal::ZERO),
            ("2000", Decimal::ZERO, dec!(50_000)),
            ("3300", Decimal::ZERO, dec!(50_000)),
        ],
    );
    // E1 contributed 60k, E2 contributed 40k to 1100.
    let e1 = make_entity_tb(
        "E1",
        "CHF",
        &[
            ("1100", dec!(60_000), Decimal::ZERO),
            ("3300", Decimal::ZERO, dec!(60_000)),
        ],
    );
    let e2 = make_entity_tb(
        "E2",
        "CHF",
        &[
            ("1100", dec!(40_000), Decimal::ZERO),
            ("2000", Decimal::ZERO, dec!(50_000)),
            ("3300", Decimal::ZERO, dec!(-10_000)),
        ],
    );

    let s = build_consolidation_schedule(
        &pre,
        &post,
        &[("E1".to_string(), e1), ("E2".to_string(), e2)],
        "TEST_GROUP",
        period_end(),
    )
    .unwrap();

    // 3 unique account codes.
    assert_eq!(s.lines.len(), 3);

    // 1100: pre 100k, post 70k, adj -30k.  E1 contributed 60k, E2 40k.
    let l = line_for(&s, "1100");
    assert_eq!(l.pre_elimination_total, dec!(100_000));
    assert_eq!(l.post_elimination_total, dec!(70_000));
    assert_eq!(l.elimination_adjustments, dec!(-30_000));
    assert_eq!(l.entity_amounts.get("E1"), Some(&dec!(60_000)));
    assert_eq!(l.entity_amounts.get("E2"), Some(&dec!(40_000)));
    assert_eq!(l.account_category, "Asset");

    assert_eq!(s.currency, "CHF");
    assert_eq!(s.group_id, "TEST_GROUP");
    assert_eq!(s.as_of_date, period_end());
}

#[test]
fn account_only_in_elimination_appears_with_zero_pre() {
    // Pre-elim: 1100 only.
    let pre = make_aggregated_tb("CHF", &[("1100", dec!(100_000), Decimal::ZERO)]);
    // Post-elim: 1100 unchanged + 3400 (equity-method bridge) = 50k credit.
    let post = make_aggregated_tb(
        "CHF",
        &[
            ("1100", dec!(100_000), Decimal::ZERO),
            ("3400", Decimal::ZERO, dec!(50_000)),
        ],
    );

    let s =
        build_consolidation_schedule(&pre, &post, &[], "TEST_GROUP", period_end()).unwrap();

    let l = line_for(&s, "3400");
    assert_eq!(l.pre_elimination_total, Decimal::ZERO);
    assert_eq!(l.post_elimination_total, dec!(-50_000));
    assert_eq!(l.elimination_adjustments, dec!(-50_000));
    assert_eq!(l.account_category, "Equity");
    // No entity contributions for an elimination-only account.
    assert!(l.entity_amounts.is_empty());
}

#[test]
fn account_fully_eliminated_pre_nonzero_post_zero() {
    // 1150 IC AR — pre 100k, post 0 after elimination.
    let pre = make_aggregated_tb("CHF", &[("1150", dec!(100_000), Decimal::ZERO)]);
    let post = make_aggregated_tb(
        "CHF",
        &[("1150", dec!(100_000), dec!(100_000))],
    );

    let s =
        build_consolidation_schedule(&pre, &post, &[], "TEST_GROUP", period_end()).unwrap();

    let l = line_for(&s, "1150");
    assert_eq!(l.pre_elimination_total, dec!(100_000));
    assert_eq!(l.post_elimination_total, Decimal::ZERO);
    assert_eq!(l.elimination_adjustments, dec!(-100_000));
}

#[test]
fn determinism_two_calls_match() {
    let pre = make_aggregated_tb(
        "CHF",
        &[
            ("1100", dec!(100_000), Decimal::ZERO),
            ("2000", Decimal::ZERO, dec!(50_000)),
        ],
    );
    let post = pre.clone();
    let e1 = make_entity_tb("E1", "CHF", &[("1100", dec!(100_000), Decimal::ZERO)]);

    let s1 = build_consolidation_schedule(
        &pre,
        &post,
        &[("E1".to_string(), e1.clone())],
        "T",
        period_end(),
    )
    .unwrap();
    let s2 = build_consolidation_schedule(
        &pre,
        &post,
        &[("E1".to_string(), e1)],
        "T",
        period_end(),
    )
    .unwrap();
    assert_eq!(s1, s2);
}

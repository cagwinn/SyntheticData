//! Task 7.4 — NCI + equity-method overlay on consolidated TB tests.
//!
//! Exercises [`apply_nci_and_equity_method`] for the v5.0
//! Chunk-7 overlay: NCI movement out of retained earnings into the
//! NCI equity component, and the simplified equity-method bridge
//! posting (BS investment + IS share-of-profit).
//!
//! Test taxonomy:
//! 1. Identity — empty NCI / empty equity-method → unchanged TB.
//! 2. NCI only — closing NCI moves from `3300` → `3500`, balance preserved.
//! 3. Equity method only — `1850` and `4900` post against the
//!    `3400` bridge, balance preserved.
//! 4. Both overlays applied together.
//! 5. Currency mismatch → `GroupError::Aggregate`.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_group::{
    apply_nci_and_equity_method, AggregatedAccount, AggregatedTb, EquityMethodInvestment,
    GroupError, NciRollforward,
};

// ── Fixture helpers ───────────────────────────────────────────────────────────

fn period_end() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
}

/// Build a small but balanced post-elimination TB:
/// - 1000 (cash):           1_000_000 dr
/// - 3100 (common stock):     400_000 cr
/// - 3300 (retained earnings):600_000 cr
fn balanced_post_elim_tb() -> AggregatedTb {
    let mut totals: BTreeMap<String, AggregatedAccount> = BTreeMap::new();
    totals.insert(
        "1000".to_string(),
        AggregatedAccount {
            account_code: "1000".to_string(),
            debit_total: dec!(1_000_000),
            credit_total: Decimal::ZERO,
            net_balance: dec!(1_000_000),
            contributing_entities: 1,
        },
    );
    totals.insert(
        "3100".to_string(),
        AggregatedAccount {
            account_code: "3100".to_string(),
            debit_total: Decimal::ZERO,
            credit_total: dec!(400_000),
            net_balance: dec!(-400_000),
            contributing_entities: 1,
        },
    );
    totals.insert(
        "3300".to_string(),
        AggregatedAccount {
            account_code: "3300".to_string(),
            debit_total: Decimal::ZERO,
            credit_total: dec!(600_000),
            net_balance: dec!(-600_000),
            contributing_entities: 1,
        },
    );

    AggregatedTb {
        group_id: "TEST_GROUP".to_string(),
        currency: "CHF".to_string(),
        as_of_date: period_end(),
        account_totals: totals,
        contributing_entities: vec!["E1".to_string()],
        deferred_entities: Vec::new(),
        total_debits: dec!(1_000_000),
        total_credits: dec!(1_000_000),
    }
}

fn nci_rf(entity: &str, closing: Decimal) -> NciRollforward {
    NciRollforward {
        entity_code: entity.to_string(),
        parent_entity_code: "PARENT".to_string(),
        ownership_percent: dec!(0.80),
        nci_percent: dec!(0.20),
        opening_nci: Decimal::ZERO,
        nci_share_of_profit: closing,
        nci_share_of_oci: Decimal::ZERO,
        nci_dividends: Decimal::ZERO,
        closing_nci: closing,
        period_end: period_end(),
        currency: "CHF".to_string(),
    }
}

fn em_inv(investee: &str, closing: Decimal, share_profit: Decimal) -> EquityMethodInvestment {
    EquityMethodInvestment {
        investee_code: investee.to_string(),
        investor_entity_code: "PARENT".to_string(),
        ownership_percent: dec!(0.50),
        opening_carrying_value: Decimal::ZERO,
        opening_suppressed_loss: Decimal::ZERO,
        share_of_profit: share_profit,
        share_of_profit_recognised: share_profit,
        dividends_received: Decimal::ZERO,
        impairment: Decimal::ZERO,
        suppressed_loss_this_period: Decimal::ZERO,
        closing_suppressed_loss: Decimal::ZERO,
        closing_carrying_value: closing,
        period_end: period_end(),
        currency: "CHF".to_string(),
    }
}

fn balance_invariant_holds(tb: &AggregatedTb) {
    let diff = (tb.total_debits - tb.total_credits).abs();
    assert!(
        diff <= Decimal::new(1, 2),
        "balance invariant violated: debits={}, credits={}, diff={}",
        tb.total_debits,
        tb.total_credits,
        diff,
    );
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn identity_no_nci_no_equity_method() {
    let tb = balanced_post_elim_tb();
    let out = apply_nci_and_equity_method(&tb, &[], &[]).expect("identity must succeed");
    assert_eq!(out, tb, "empty overlays must produce identical TB");
}

#[test]
fn nci_only_moves_retained_earnings_to_nci_equity() {
    let tb = balanced_post_elim_tb();
    // 80%-owned NESTLE_DE with 200_000 closing NCI.
    let nci = vec![nci_rf("NESTLE_DE", dec!(200_000))];

    let out = apply_nci_and_equity_method(&tb, &nci, &[]).expect("must succeed");

    // NCI equity (3500) credited 200_000.
    let nci_equity = out.account_totals.get("3500").expect("3500 created");
    assert_eq!(nci_equity.debit_total, Decimal::ZERO);
    assert_eq!(nci_equity.credit_total, dec!(200_000));
    assert_eq!(nci_equity.net_balance, dec!(-200_000));
    assert_eq!(
        nci_equity.contributing_entities, 0,
        "overlay does not bump contributing_entities"
    );

    // Retained earnings (3300) debited 200_000 — net balance reduces
    // from 600_000 cr to 400_000 cr.
    let re = out.account_totals.get("3300").expect("3300 still present");
    assert_eq!(re.debit_total, dec!(200_000));
    assert_eq!(re.credit_total, dec!(600_000));
    assert_eq!(re.net_balance, dec!(-400_000));

    // Balance preserved (overlay is a balanced 2-line posting).
    balance_invariant_holds(&out);
}

#[test]
fn equity_method_only_creates_investment_and_pl_lines() {
    let tb = balanced_post_elim_tb();
    // 50%-owned JV with 1.8m closing carrying value and 400k share of
    // profit.
    let em = vec![em_inv("NESTLE_JV", dec!(1_800_000), dec!(400_000))];

    let out = apply_nci_and_equity_method(&tb, &[], &em).expect("must succeed");

    // 1850 (investment in associates) debited 1_800_000.
    let inv = out
        .account_totals
        .get("1850")
        .expect("investment line created");
    assert_eq!(inv.debit_total, dec!(1_800_000));
    assert_eq!(inv.credit_total, Decimal::ZERO);

    // 4900 (share of profit) credited 400_000.
    let pl = out
        .account_totals
        .get("4900")
        .expect("share of profit line");
    assert_eq!(pl.debit_total, Decimal::ZERO);
    assert_eq!(pl.credit_total, dec!(400_000));

    // v5.1: 3400 bridge retired — overlay now posts to retained
    // earnings (3300).  Cumulative effect on 3300 from this overlay
    // (no NCI in this test):
    //   credit 1_800_000 (BS investment counterparty)
    //   debit  400_000   (IS share-of-profit counterparty)
    // Plus the pre-overlay 3300 carried 600_000 cr.  Net 3300:
    //   credit_total = 600_000 + 1_800_000 = 2_400_000
    //   debit_total  = 400_000
    let re = out.account_totals.get("3300").expect("retained earnings");
    assert_eq!(re.credit_total, dec!(2_400_000));
    assert_eq!(re.debit_total, dec!(400_000));

    // 3400 bridge must NOT exist any more.
    assert!(
        !out.account_totals.contains_key("3400"),
        "v5.1 retired the 3400 bridge — overlay must post to 3300 instead"
    );

    // Each posting is balanced individually so the consolidated
    // invariant still holds.
    balance_invariant_holds(&out);
}

#[test]
fn both_overlays_applied_together() {
    let tb = balanced_post_elim_tb();
    let nci = vec![
        nci_rf("NESTLE_DE", dec!(200_000)),
        nci_rf("NESTLE_BR", dec!(50_000)),
    ];
    let em = vec![em_inv("NESTLE_JV", dec!(1_800_000), dec!(400_000))];

    let out = apply_nci_and_equity_method(&tb, &nci, &em).expect("must succeed");

    // NCI equity (3500) credited Σ closing_nci = 250_000.
    let nci_equity = out.account_totals.get("3500").unwrap();
    assert_eq!(nci_equity.credit_total, dec!(250_000));

    // Retained earnings (3300) is hit by both overlays in v5.1:
    //   - NCI overlay debits 250_000  (Σ closing_nci)
    //   - Equity-method overlay credits 1_800_000 (BS counterparty)
    //                          and debits  400_000 (IS counterparty)
    // Plus the pre-overlay 3300 had 600_000 cr.
    //   credit_total = 600_000 + 1_800_000 = 2_400_000
    //   debit_total  = 250_000 +   400_000 =   650_000
    let re = out.account_totals.get("3300").unwrap();
    assert_eq!(re.debit_total, dec!(650_000));
    assert_eq!(re.credit_total, dec!(2_400_000));

    // Investment (1850) and share of profit (4900) present from the
    // equity-method overlay.
    assert!(out.account_totals.contains_key("1850"));
    assert!(out.account_totals.contains_key("4900"));

    // 3400 bridge retired in v5.1.
    assert!(
        !out.account_totals.contains_key("3400"),
        "v5.1 retired the 3400 bridge"
    );

    balance_invariant_holds(&out);
}

#[test]
fn currency_mismatch_returns_aggregate_error() {
    let tb = balanced_post_elim_tb(); // CHF
                                      // NCI rollforward in EUR — caller forgot to translate.
    let bad_nci = NciRollforward {
        entity_code: "NESTLE_DE".to_string(),
        parent_entity_code: "PARENT".to_string(),
        ownership_percent: dec!(0.80),
        nci_percent: dec!(0.20),
        opening_nci: Decimal::ZERO,
        nci_share_of_profit: dec!(100),
        nci_share_of_oci: Decimal::ZERO,
        nci_dividends: Decimal::ZERO,
        closing_nci: dec!(100),
        period_end: period_end(),
        currency: "EUR".to_string(), // ← mismatch
    };
    let err =
        apply_nci_and_equity_method(&tb, &[bad_nci], &[]).expect_err("currency mismatch must fail");
    match err {
        GroupError::Aggregate(msg) => {
            assert!(msg.contains("EUR"), "msg names src ccy: {msg}");
            assert!(msg.contains("CHF"), "msg names tb ccy: {msg}");
            assert!(msg.contains("NESTLE_DE"), "msg names entity: {msg}");
        }
        other => panic!("expected Aggregate, got {other:?}"),
    }

    // Same for equity-method investment in a different currency.
    let bad_em = EquityMethodInvestment {
        investee_code: "NESTLE_JV".to_string(),
        investor_entity_code: "PARENT".to_string(),
        ownership_percent: dec!(0.50),
        opening_carrying_value: Decimal::ZERO,
        opening_suppressed_loss: Decimal::ZERO,
        share_of_profit: dec!(100),
        share_of_profit_recognised: dec!(100),
        dividends_received: Decimal::ZERO,
        impairment: Decimal::ZERO,
        suppressed_loss_this_period: Decimal::ZERO,
        closing_suppressed_loss: Decimal::ZERO,
        closing_carrying_value: dec!(100),
        period_end: period_end(),
        currency: "USD".to_string(),
    };
    let err =
        apply_nci_and_equity_method(&tb, &[], &[bad_em]).expect_err("currency mismatch must fail");
    match err {
        GroupError::Aggregate(msg) => {
            assert!(msg.contains("USD"));
            assert!(msg.contains("NESTLE_JV"));
        }
        other => panic!("expected Aggregate, got {other:?}"),
    }
}

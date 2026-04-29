//! Task 7.3 — equity-method investment rollforward integration tests.
//!
//! Exercises [`compute_equity_method_investment`] for the IAS 28.10–11
//! / ASC 323 share-of-profit identity, the validation gates for
//! non-`EquityMethod` consolidation methods, ownership boundary values,
//! the IAS 28.38 non-negative carrying-value guard, the
//! write/ingest round-trip, and the determinism contract.

use std::fs;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_group::{
    build_manifest, compute_equity_method_investment,
    ingest_opening_equity_method_carrying_values, write_equity_method_investments,
    ConsolidationMethod, EquityMethodInputs, EquityMethodInvestment, GroupConfig, GroupError,
    EQUITY_METHOD_INVESTMENTS_FILENAME,
};

// ── Fixture helpers ───────────────────────────────────────────────────────────

fn period_end() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
}

/// Pull NESTLE_JV (50%, equity_method) from the real manifest pipeline.
fn nestle_jv() -> datasynth_group::ManifestEntity {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse into GroupConfig");
    let manifest = build_manifest(&cfg).expect("manifest builds");
    manifest
        .ownership_graph
        .entities
        .into_iter()
        .find(|e| e.code == "NESTLE_JV")
        .expect("NESTLE_JV must be present in the fixture")
}

/// Hand-rolled entity builder for tests that vary
/// `consolidation_method` / `ownership_percent`.
fn make_entity(
    code: &str,
    method: ConsolidationMethod,
    ownership: Option<Decimal>,
) -> datasynth_group::ManifestEntity {
    datasynth_group::ManifestEntity {
        code: code.to_string(),
        name: None,
        country: "CH".to_string(),
        functional_currency: "CHF".to_string(),
        scoping_profile: "material".to_string(),
        consolidation_method: method,
        ownership_percent: ownership,
        parent_code: Some("PARENT".to_string()),
        accounting_framework: None,
        industry: None,
        entity_seed: "00".to_string(),
        shard_id: "S_TEST_0001".to_string(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn happy_path_fifty_percent_jv() {
    // 50%-owned NESTLE_JV — the canonical equity-method case.
    let investee = nestle_jv();
    assert_eq!(
        investee.consolidation_method,
        ConsolidationMethod::EquityMethod
    );
    assert_eq!(investee.ownership_percent, Some(dec!(0.50)));

    let inputs = EquityMethodInputs {
        investee: &investee,
        investor_entity_code: "NESTLE_SA".to_string(),
        investee_net_income: dec!(800_000),
        investee_dividends_paid: dec!(200_000),
        opening_carrying_value: dec!(1_500_000),
        impairment: Decimal::ZERO,
        period_end: period_end(),
        currency: "CHF".to_string(),
    };

    let inv = compute_equity_method_investment(&inputs).expect("must succeed");

    assert_eq!(inv.investee_code, "NESTLE_JV");
    assert_eq!(inv.investor_entity_code, "NESTLE_SA");
    assert_eq!(inv.ownership_percent, dec!(0.50));
    assert_eq!(inv.share_of_profit, dec!(400_000.00));
    assert_eq!(inv.dividends_received, dec!(100_000.00));
    assert_eq!(inv.impairment, Decimal::ZERO);
    // 1_500_000 + 400_000 - 100_000 - 0 = 1_800_000
    assert_eq!(inv.closing_carrying_value, dec!(1_800_000.00));
    assert_eq!(inv.period_end, period_end());
    assert_eq!(inv.currency, "CHF");
}

#[test]
fn rejects_non_equity_method_consolidation_methods() {
    for (label, method) in [
        ("Parent", ConsolidationMethod::Parent),
        ("Full", ConsolidationMethod::Full),
        ("Proportional", ConsolidationMethod::Proportional),
        ("FairValue", ConsolidationMethod::FairValue),
    ] {
        let investee = make_entity("INV", method, Some(dec!(0.50)));
        let inputs = EquityMethodInputs {
            investee: &investee,
            investor_entity_code: "PARENT".to_string(),
            investee_net_income: dec!(100),
            investee_dividends_paid: Decimal::ZERO,
            opening_carrying_value: Decimal::ZERO,
            impairment: Decimal::ZERO,
            period_end: period_end(),
            currency: "CHF".to_string(),
        };
        let err =
            compute_equity_method_investment(&inputs).expect_err(&format!("{label} must reject"));
        match err {
            GroupError::Aggregate(msg) => {
                assert!(msg.contains("INV"), "{label}: msg names entity: {msg}");
                assert!(msg.contains(label), "{label}: msg names method: {msg}");
            }
            other => panic!("{label}: expected Aggregate, got {other:?}"),
        }
    }
}

#[test]
fn rejects_ownership_at_boundaries() {
    // Equity-method requires strict 0 < ownership < 1.
    for bad in [Decimal::ZERO, Decimal::ONE, dec!(-0.10), dec!(1.50)] {
        let investee = make_entity("INV", ConsolidationMethod::EquityMethod, Some(bad));
        let inputs = EquityMethodInputs {
            investee: &investee,
            investor_entity_code: "PARENT".to_string(),
            investee_net_income: dec!(100),
            investee_dividends_paid: Decimal::ZERO,
            opening_carrying_value: Decimal::ZERO,
            impairment: Decimal::ZERO,
            period_end: period_end(),
            currency: "CHF".to_string(),
        };
        let err = compute_equity_method_investment(&inputs)
            .expect_err(&format!("ownership={bad} must reject"));
        match err {
            GroupError::Aggregate(msg) => {
                assert!(
                    msg.contains("(0, 1)"),
                    "ownership={bad}: msg explains range: {msg}"
                );
            }
            other => panic!("ownership={bad}: expected Aggregate, got {other:?}"),
        }
    }
}

#[test]
fn negative_carrying_value_triggers_error() {
    // 50%-owned investee with a 1M loss and a 100k opening:
    //   100_000 + (0.5 * -1_000_000) - 0 - 0 = -400_000 → error.
    let investee = make_entity("INV", ConsolidationMethod::EquityMethod, Some(dec!(0.50)));
    let inputs = EquityMethodInputs {
        investee: &investee,
        investor_entity_code: "PARENT".to_string(),
        investee_net_income: dec!(-1_000_000), // huge loss
        investee_dividends_paid: Decimal::ZERO,
        opening_carrying_value: dec!(100_000),
        impairment: Decimal::ZERO,
        period_end: period_end(),
        currency: "CHF".to_string(),
    };
    let err =
        compute_equity_method_investment(&inputs).expect_err("negative carrying must reject");
    match err {
        GroupError::Aggregate(msg) => {
            assert!(msg.contains("INV"), "msg names entity: {msg}");
            assert!(
                msg.contains("negative"),
                "msg explains the violation: {msg}"
            );
            assert!(msg.contains("IAS 28.38"), "msg cites the standard: {msg}");
        }
        other => panic!("expected Aggregate, got {other:?}"),
    }
}

#[test]
fn round_trip_via_writer_and_ingest() {
    let tmp = tempfile::tempdir().expect("tmp");

    let investee = make_entity("JV1", ConsolidationMethod::EquityMethod, Some(dec!(0.40)));
    let inputs = EquityMethodInputs {
        investee: &investee,
        investor_entity_code: "PARENT".to_string(),
        investee_net_income: dec!(500_000),
        investee_dividends_paid: dec!(100_000),
        opening_carrying_value: dec!(2_000_000),
        impairment: Decimal::ZERO,
        period_end: period_end(),
        currency: "CHF".to_string(),
    };
    let inv = compute_equity_method_investment(&inputs).expect("compute");
    // 2_000_000 + 200_000 - 40_000 = 2_160_000
    assert_eq!(inv.closing_carrying_value, dec!(2_160_000.00));

    let path = write_equity_method_investments(std::slice::from_ref(&inv), tmp.path())
        .expect("write must succeed");
    assert!(path.ends_with(EQUITY_METHOD_INVESTMENTS_FILENAME));
    assert!(path.parent().unwrap().ends_with("consolidated"));

    // Direct round-trip via serde.
    let bytes = fs::read(&path).expect("read");
    let round_trip: Vec<EquityMethodInvestment> = serde_json::from_slice(&bytes).expect("parse");
    assert_eq!(round_trip, vec![inv.clone()]);

    let s = String::from_utf8(bytes).expect("utf8");
    assert!(s.ends_with('\n'), "trailing newline for human readers");

    // Round-trip via the public ingest API: closing → opening map.
    let opening_map = ingest_opening_equity_method_carrying_values(tmp.path()).expect("ingest");
    assert_eq!(opening_map.get("JV1"), Some(&dec!(2_160_000.00)));

    // Missing-file path: a fresh tmpdir has no file → empty map.
    let tmp2 = tempfile::tempdir().expect("tmp2");
    let empty = ingest_opening_equity_method_carrying_values(tmp2.path()).expect("missing → ok");
    assert!(empty.is_empty());
}

#[test]
fn determinism_two_calls_produce_identical_records() {
    let investee = make_entity("JV", ConsolidationMethod::EquityMethod, Some(dec!(0.35)));
    let make_inputs = || EquityMethodInputs {
        investee: &investee,
        investor_entity_code: "PARENT".to_string(),
        investee_net_income: dec!(1_234.56),
        investee_dividends_paid: dec!(78.91),
        opening_carrying_value: dec!(10_000),
        impairment: dec!(50),
        period_end: period_end(),
        currency: "CHF".to_string(),
    };
    let a = compute_equity_method_investment(&make_inputs()).expect("a");
    let b = compute_equity_method_investment(&make_inputs()).expect("b");
    assert_eq!(a, b, "two calls must produce equal records");

    let bytes_a = serde_json::to_vec(&a).expect("ser a");
    let bytes_b = serde_json::to_vec(&b).expect("ser b");
    assert_eq!(bytes_a, bytes_b);
}

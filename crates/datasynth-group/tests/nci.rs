//! Task 7.1 — NCI rollforward per-subsidiary integration tests.
//!
//! Exercises [`compute_nci_rollforward`] for the IFRS 10 / ASC 810
//! share-of-equity allocation identity, the validation gates for
//! non-`Full` consolidation methods and bad ownership, and the
//! determinism contract.
//!
//! See `crates/datasynth-group/src/aggregate/nci/rollforward.rs` for the
//! implementation.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_group::{
    build_manifest, compute_nci_rollforward, ConsolidationMethod, GroupConfig, GroupError,
    NciInputs,
};

// ── Fixture helpers ───────────────────────────────────────────────────────────

fn period_end() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
}

/// Build a `ManifestEntity` directly via the manifest pipeline, so we
/// exercise the public `ManifestEntity` shape rather than relying on a
/// hand-rolled stub that could drift from the real builder.
fn nestle_de_entity() -> datasynth_group::ManifestEntity {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse into GroupConfig");
    let manifest = build_manifest(&cfg).expect("manifest builds");
    manifest
        .ownership_graph
        .entities
        .into_iter()
        .find(|e| e.code == "NESTLE_DE")
        .expect("NESTLE_DE must be present in the fixture")
}

/// Direct constructor for tests that need to vary `consolidation_method`
/// or `ownership_percent` against the validation gates.
fn make_entity(
    code: &str,
    method: ConsolidationMethod,
    ownership: Option<Decimal>,
    parent_code: Option<&str>,
) -> datasynth_group::ManifestEntity {
    datasynth_group::ManifestEntity {
        code: code.to_string(),
        name: None,
        country: "DE".to_string(),
        functional_currency: "EUR".to_string(),
        scoping_profile: "significant".to_string(),
        consolidation_method: method,
        ownership_percent: ownership,
        parent_code: parent_code.map(str::to_string),
        accounting_framework: None,
        industry: None,
        entity_seed: "00".to_string(),
        shard_id: "S_TEST_0001".to_string(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn happy_path_eighty_percent_owned_subsidiary() {
    // 80%-owned NESTLE_DE-style subsidiary.
    let entity = make_entity(
        "NESTLE_DE",
        ConsolidationMethod::Full,
        Some(dec!(0.80)),
        Some("NESTLE_SA"),
    );
    let inputs = NciInputs {
        entity: &entity,
        period_net_income: dec!(1_000_000),
        period_oci: dec!(50_000),
        total_dividends_paid: dec!(200_000),
        opening_nci: dec!(800_000),
        period_end: period_end(),
        currency: "CHF".to_string(),
    };

    let rf = compute_nci_rollforward(&inputs).expect("must succeed");

    // 20% NCI share.
    assert_eq!(rf.entity_code, "NESTLE_DE");
    assert_eq!(rf.parent_entity_code, "NESTLE_SA");
    assert_eq!(rf.ownership_percent, dec!(0.80));
    assert_eq!(rf.nci_percent, dec!(0.20));

    // Share-of-* attributions = nci_percent × *
    assert_eq!(rf.nci_share_of_profit, dec!(200_000.00));
    assert_eq!(rf.nci_share_of_oci, dec!(10_000.00));
    assert_eq!(rf.nci_dividends, dec!(40_000.00));

    // Closing identity.
    // 800_000 + 200_000 + 10_000 - 40_000 = 970_000
    assert_eq!(rf.closing_nci, dec!(970_000.00));

    // Carry-through: period_end + currency.
    assert_eq!(rf.period_end, period_end());
    assert_eq!(rf.currency, "CHF");
}

#[test]
fn from_manifest_entity_directly() {
    // Load NESTLE_DE from the real Mini-Nestlé manifest pipeline and
    // verify the rollforward reads ownership_percent correctly.
    let entity = nestle_de_entity();
    assert_eq!(entity.consolidation_method, ConsolidationMethod::Full);
    assert_eq!(entity.ownership_percent, Some(dec!(0.80)));
    assert_eq!(entity.parent_code.as_deref(), Some("NESTLE_SA"));

    let inputs = NciInputs {
        entity: &entity,
        period_net_income: dec!(500_000),
        period_oci: dec!(0),
        total_dividends_paid: dec!(0),
        opening_nci: dec!(0),
        period_end: period_end(),
        currency: "CHF".to_string(),
    };

    let rf = compute_nci_rollforward(&inputs).expect("must succeed");

    // 20% NCI of a 500k profit, no opening / OCI / dividends.
    assert_eq!(rf.nci_percent, dec!(0.20));
    assert_eq!(rf.nci_share_of_profit, dec!(100_000.00));
    assert_eq!(rf.closing_nci, dec!(100_000.00));
    assert_eq!(rf.parent_entity_code, "NESTLE_SA");
}

#[test]
fn rejects_parent_consolidation_method() {
    let entity = make_entity("NESTLE_SA", ConsolidationMethod::Parent, None, None);
    let inputs = NciInputs {
        entity: &entity,
        period_net_income: Decimal::ZERO,
        period_oci: Decimal::ZERO,
        total_dividends_paid: Decimal::ZERO,
        opening_nci: Decimal::ZERO,
        period_end: period_end(),
        currency: "CHF".to_string(),
    };

    let err = compute_nci_rollforward(&inputs).expect_err("Parent must be rejected");
    match err {
        GroupError::Aggregate(msg) => {
            assert!(msg.contains("NESTLE_SA"), "msg names entity: {msg}");
            assert!(msg.contains("Parent"), "msg names method: {msg}");
            assert!(msg.contains("NCI"), "msg explains why: {msg}");
        }
        other => panic!("expected Aggregate, got {other:?}"),
    }
}

#[test]
fn rejects_equity_method() {
    let entity = make_entity(
        "NESTLE_JV",
        ConsolidationMethod::EquityMethod,
        Some(dec!(0.50)),
        Some("NESTLE_SA"),
    );
    let inputs = NciInputs {
        entity: &entity,
        period_net_income: dec!(100),
        period_oci: Decimal::ZERO,
        total_dividends_paid: Decimal::ZERO,
        opening_nci: Decimal::ZERO,
        period_end: period_end(),
        currency: "CHF".to_string(),
    };

    let err = compute_nci_rollforward(&inputs).expect_err("EquityMethod must be rejected");
    match err {
        GroupError::Aggregate(msg) => {
            assert!(msg.contains("NESTLE_JV"));
            assert!(msg.contains("EquityMethod"));
        }
        other => panic!("expected Aggregate, got {other:?}"),
    }
}

#[test]
fn rejects_full_with_one_hundred_percent_ownership() {
    // A `Full`-consolidated entity with 100% ownership is a caller bug
    // — that entity should be `Parent`.  Surface this as a typed error
    // rather than silently produce a zero NCI.
    let entity = make_entity(
        "FULL_BUT_WHOLLY_OWNED",
        ConsolidationMethod::Full,
        Some(Decimal::ONE),
        Some("PARENT"),
    );
    let inputs = NciInputs {
        entity: &entity,
        period_net_income: dec!(100),
        period_oci: Decimal::ZERO,
        total_dividends_paid: Decimal::ZERO,
        opening_nci: Decimal::ZERO,
        period_end: period_end(),
        currency: "CHF".to_string(),
    };

    let err = compute_nci_rollforward(&inputs).expect_err("100%-owned Full must be rejected");
    match err {
        GroupError::Aggregate(msg) => {
            assert!(
                msg.contains("FULL_BUT_WHOLLY_OWNED"),
                "msg names entity: {msg}"
            );
            assert!(msg.contains("Parent"), "msg suggests Parent: {msg}");
        }
        other => panic!("expected Aggregate, got {other:?}"),
    }
}

#[test]
fn determinism_two_calls_produce_identical_records() {
    let entity = make_entity(
        "SUB",
        ConsolidationMethod::Full,
        Some(dec!(0.75)),
        Some("PARENT"),
    );
    let make_inputs = || NciInputs {
        entity: &entity,
        period_net_income: dec!(1_234.56),
        period_oci: dec!(78.91),
        total_dividends_paid: dec!(123.45),
        opening_nci: dec!(2_000),
        period_end: period_end(),
        currency: "CHF".to_string(),
    };

    let rf_a = compute_nci_rollforward(&make_inputs()).expect("a");
    let rf_b = compute_nci_rollforward(&make_inputs()).expect("b");
    assert_eq!(rf_a, rf_b, "two calls must produce equal records");

    // And the JSON serialisation is byte-identical.
    let bytes_a = serde_json::to_vec(&rf_a).expect("ser a");
    let bytes_b = serde_json::to_vec(&rf_b).expect("ser b");
    assert_eq!(bytes_a, bytes_b, "two calls must serialise byte-identical");
}

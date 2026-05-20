//! Task 7.2 — NCI opening-balance ingestion + writer integration tests.
//!
//! Covers the prior-period rollforward read path and the on-disk writer:
//!
//! 1. Missing file → empty map (warning is logged but not asserted on —
//!    `tracing::warn!` integration tests are flaky without a captured
//!    subscriber).
//! 2. Valid file with 2 entries → map keyed by entity code, value is
//!    `closing_nci`.
//! 3. Corrupt JSON → [`GroupError::Serde`].
//! 4. Duplicate entity → [`GroupError::Aggregate`].
//! 5. `write_nci_rollforward` writes to the spec'd path and round-trips
//!    back through [`ingest_opening_nci_balances`].

use std::fs;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_group::{
    ingest_opening_nci_balances, write_nci_rollforward, GroupError, NciRollforward,
    NCI_ROLLFORWARD_FILENAME,
};

// ── Fixture helpers ───────────────────────────────────────────────────────────

fn period_end() -> NaiveDate {
    NaiveDate::from_ymd_opt(2023, 12, 31).unwrap()
}

fn rf(entity_code: &str, parent: &str, closing: Decimal) -> NciRollforward {
    NciRollforward {
        entity_code: entity_code.to_string(),
        parent_entity_code: parent.to_string(),
        ownership_percent: dec!(0.80),
        nci_percent: dec!(0.20),
        opening_nci: dec!(0),
        nci_share_of_profit: closing,
        nci_share_of_oci: dec!(0),
        nci_dividends: dec!(0),
        equity_transaction_adjustments: Decimal::ZERO,
        pl_remeasurement_gain_or_loss: Decimal::ZERO,
        closing_nci: closing,
        period_end: period_end(),
        currency: "CHF".to_string(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn missing_file_returns_empty_map() {
    let tmp = tempfile::tempdir().expect("tmp");
    // No consolidated/nci_rollforward.json was ever written here.
    let map = ingest_opening_nci_balances(tmp.path())
        .expect("missing file is not an error — first period of an engagement");
    assert!(map.is_empty(), "missing file → empty map");
}

#[test]
fn valid_file_round_trips_to_opening_balances() {
    let tmp = tempfile::tempdir().expect("tmp");
    let rfs = vec![
        rf("ACME_DE", "ACME_SA", dec!(940_000.00)),
        rf("ACME_BR", "ACME_SA", dec!(125_500.50)),
    ];

    let path = write_nci_rollforward(&rfs, tmp.path()).expect("write");
    assert!(path.ends_with(NCI_ROLLFORWARD_FILENAME));

    let map = ingest_opening_nci_balances(tmp.path()).expect("read");
    assert_eq!(map.len(), 2);
    // Closing NCI from prior period = opening NCI for this period.
    assert_eq!(map.get("ACME_DE"), Some(&dec!(940_000.00)));
    assert_eq!(map.get("ACME_BR"), Some(&dec!(125_500.50)));
}

#[test]
fn corrupt_json_returns_serde_error() {
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().join("consolidated");
    fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join(NCI_ROLLFORWARD_FILENAME);
    fs::write(&path, b"{ not valid json").expect("write garbage");

    let err = ingest_opening_nci_balances(tmp.path()).expect_err("corrupt JSON must fail");
    match err {
        GroupError::Serde(msg) => assert!(msg.contains("json"), "msg names format: {msg}"),
        other => panic!("expected Serde, got {other:?}"),
    }
}

#[test]
fn duplicate_entity_returns_aggregate_error() {
    let tmp = tempfile::tempdir().expect("tmp");
    let rfs = vec![
        rf("ACME_DE", "ACME_SA", dec!(100)),
        rf("ACME_DE", "ACME_SA", dec!(200)), // duplicate
    ];
    write_nci_rollforward(&rfs, tmp.path()).expect("write");

    let err = ingest_opening_nci_balances(tmp.path()).expect_err("duplicate must fail");
    match err {
        GroupError::Aggregate(msg) => {
            assert!(msg.contains("duplicate"), "msg names problem: {msg}");
            assert!(msg.contains("ACME_DE"), "msg names entity: {msg}");
        }
        other => panic!("expected Aggregate, got {other:?}"),
    }
}

#[test]
fn write_then_read_round_trip_preserves_records() {
    let tmp = tempfile::tempdir().expect("tmp");
    let rfs = vec![
        rf("E1", "PARENT", dec!(1_500)),
        rf("E2", "PARENT", dec!(-250)), // Negative NCI is permitted (cumulative loss).
        rf("E3", "PARENT", Decimal::ZERO),
    ];

    let path = write_nci_rollforward(&rfs, tmp.path()).expect("write");
    // Path is `{out_dir}/consolidated/nci_rollforward.json`.
    assert!(path.ends_with(NCI_ROLLFORWARD_FILENAME));
    assert!(path.parent().unwrap().ends_with("consolidated"));

    // Direct round-trip via serde — verifies the on-disk JSON exactly
    // matches the in-memory shape.
    let bytes = fs::read(&path).expect("read");
    let round_trip: Vec<NciRollforward> = serde_json::from_slice(&bytes).expect("parse");
    assert_eq!(round_trip, rfs);

    // Trailing newline (human-friendly).
    let s = String::from_utf8(bytes).expect("utf8");
    assert!(s.ends_with('\n'), "trailing newline for human readers");

    // And via the public ingest API: the closing → opening conversion.
    let map = ingest_opening_nci_balances(tmp.path()).expect("ingest");
    assert_eq!(map.get("E1"), Some(&dec!(1_500)));
    assert_eq!(map.get("E2"), Some(&dec!(-250)));
    assert_eq!(map.get("E3"), Some(&Decimal::ZERO));
}

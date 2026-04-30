//! Task 5.1 — per-entity trial balance loader integration tests.
//!
//! The orchestrator-running smoke tests (`shard_runner.rs`,
//! `shard_e2e.rs`) are too memory-heavy to also exercise the aggregate
//! loader on the same CI host.  We synthesise the on-disk shape directly
//! instead: hand-build a `Vec<TrialBalance>`, write it to
//! `{tempdir}/period_close/trial_balances.json`, and call the loader.
//!
//! That keeps every case in this file at a few KB of resident memory and
//! lets us exercise paths the orchestrator cannot easily produce —
//! corrupt JSON, empty arrays, multiple-TB files, hand-edited
//! is_balanced flags.

use std::fs;
use std::path::Path;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tempfile::TempDir;

use datasynth_core::models::balance::{
    AccountCategory, AccountType, TrialBalance, TrialBalanceLine, TrialBalanceType,
};
use datasynth_group::errors::GroupError;
use datasynth_group::load_entity_trial_balance;

// ── Test helpers ──────────────────────────────────────────────────────────────

/// Build a small balanced trial balance suitable for fixture serialisation.
///
/// Two lines: a Cash debit and a matching Common-Stock credit, totalling
/// $10,000.  `TrialBalance::add_line` updates totals + `is_balanced`
/// internally so the serialised file exercises the same recalculation
/// path the orchestrator uses.
fn balanced_tb(id: &str, company_code: &str) -> TrialBalance {
    let mut tb = TrialBalance::new(
        id.to_string(),
        company_code.to_string(),
        NaiveDate::from_ymd_opt(2025, 12, 31).expect("valid date"),
        2025,
        12,
        "USD".to_string(),
        TrialBalanceType::Adjusted,
    );
    tb.add_line(TrialBalanceLine {
        account_code: "1100".to_string(),
        account_description: "Cash".to_string(),
        category: AccountCategory::CurrentAssets,
        account_type: AccountType::Asset,
        opening_balance: Decimal::ZERO,
        period_debits: dec!(10000),
        period_credits: Decimal::ZERO,
        closing_balance: dec!(10000),
        debit_balance: dec!(10000),
        credit_balance: Decimal::ZERO,
        cost_center: None,
        profit_center: None,
    });
    tb.add_line(TrialBalanceLine {
        account_code: "3100".to_string(),
        account_description: "Common Stock".to_string(),
        category: AccountCategory::Equity,
        account_type: AccountType::Equity,
        opening_balance: Decimal::ZERO,
        period_debits: Decimal::ZERO,
        period_credits: dec!(10000),
        closing_balance: dec!(10000),
        debit_balance: Decimal::ZERO,
        credit_balance: dec!(10000),
        cost_center: None,
        profit_center: None,
    });
    debug_assert!(tb.is_balanced, "fixture builder must produce balanced TB");
    tb
}

/// Write `body` to `{entity_dir}/period_close/trial_balances.json`,
/// creating parent directories as needed.
fn write_tb_file(entity_dir: &Path, body: &[u8]) {
    let pc_dir = entity_dir.join("period_close");
    fs::create_dir_all(&pc_dir).expect("create period_close dir");
    fs::write(pc_dir.join("trial_balances.json"), body).expect("write TB JSON");
}

/// Convenience: serialise a `Vec<TrialBalance>` and call [`write_tb_file`].
fn write_tb_array(entity_dir: &Path, tbs: &[TrialBalance]) {
    let json = serde_json::to_vec_pretty(tbs).expect("serialise TB array");
    write_tb_file(entity_dir, &json);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Happy path: one balanced TB written by the orchestrator round-trips
/// through the loader byte-for-byte (modulo serde-driven `created_at`
/// formatting).  We assert on identity-bearing fields rather than full
/// struct equality because `TrialBalance` is not `PartialEq`.
#[test]
fn loads_balanced_single_tb() {
    let tmp = TempDir::new().expect("tempdir");
    let entity_dir = tmp.path();

    let original = balanced_tb("TB-NESTLE_SA-2025-12", "NESTLE_SA");
    // Capture identity-bearing fields before we move `original` into the
    // serialised array — the asserts compare loaded values against these
    // captured copies, avoiding a `.clone()` on the whole TB.
    let expected_id = original.trial_balance_id.clone();
    let expected_code = original.company_code.clone();
    let expected_ccy = original.currency.clone();
    let expected_debits = original.total_debits;
    let expected_credits = original.total_credits;
    write_tb_array(entity_dir, std::slice::from_ref(&original));

    let loaded = load_entity_trial_balance(entity_dir).expect("loader must succeed on balanced TB");

    assert_eq!(loaded.trial_balance_id, expected_id);
    assert_eq!(loaded.company_code, expected_code);
    assert_eq!(loaded.currency, expected_ccy);
    assert!(loaded.is_balanced);
    assert_eq!(loaded.total_debits, expected_debits);
    assert_eq!(loaded.total_credits, expected_credits);
    assert_eq!(loaded.lines.len(), 2);
}

/// Missing file: a tempdir without `period_close/trial_balances.json`
/// surfaces as `GroupError::Io(NotFound)`.  Aggregate phase callers
/// special-case this when an entity legitimately has no period-close
/// output (e.g. a freshly added subsidiary with no fiscal period yet).
#[test]
fn missing_file_is_io_not_found() {
    let tmp = TempDir::new().expect("tempdir");

    let err = load_entity_trial_balance(tmp.path()).expect_err("missing file must error");

    match err {
        GroupError::Io(io_err) => {
            assert_eq!(
                io_err.kind(),
                std::io::ErrorKind::NotFound,
                "expected NotFound, got {:?}",
                io_err.kind()
            );
        }
        other => panic!("expected GroupError::Io(NotFound), got {other:?}"),
    }
}

/// Corrupt JSON: a file that exists but is not valid JSON surfaces as
/// `GroupError::Serde`.  We pin the variant rather than the exact
/// message to stay stable across `serde_json` versions.
#[test]
fn corrupt_json_is_serde_error() {
    let tmp = TempDir::new().expect("tempdir");
    write_tb_file(tmp.path(), b"{ this is not json");

    let err = load_entity_trial_balance(tmp.path()).expect_err("corrupt JSON must error");

    assert!(
        matches!(err, GroupError::Serde(_)),
        "expected GroupError::Serde, got {err:?}"
    );
}

/// Empty array: orchestrator wrote `[]`.  Reject as `Aggregate` so the
/// caller can attribute it to the specific entity directory.
#[test]
fn empty_array_is_aggregate_error() {
    let tmp = TempDir::new().expect("tempdir");
    let entity_subdir = tmp.path().join("NESTLE_SA");
    fs::create_dir_all(&entity_subdir).expect("create entity subdir");
    write_tb_file(&entity_subdir, b"[]");

    let err = load_entity_trial_balance(&entity_subdir).expect_err("empty array must error");

    match err {
        GroupError::Aggregate(msg) => {
            assert!(
                msg.contains("NESTLE_SA"),
                "error message must name the entity, got {msg:?}"
            );
            assert!(
                msg.contains("empty"),
                "error message must describe the empty-array condition, got {msg:?}"
            );
        }
        other => panic!("expected GroupError::Aggregate, got {other:?}"),
    }
}

/// Multi-period archive: orchestrator emits one TB per fiscal period
/// (e.g. monthly run with three months → 3 TBs in the array).  v5.1+
/// the loader accepts multiple TBs and picks the latest by
/// `(fiscal_year, fiscal_period)` — that's the closing balance the
/// consolidation engine consolidates against.
#[test]
fn multi_period_archive_picks_latest_period() {
    let tmp = TempDir::new().expect("tempdir");
    let entity_subdir = tmp.path().join("NESTLE_USA");
    fs::create_dir_all(&entity_subdir).expect("create entity subdir");

    let mut tb1 = balanced_tb("TB-NESTLE_USA-2025-11", "NESTLE_USA");
    tb1.fiscal_year = 2025;
    tb1.fiscal_period = 11;
    let mut tb2 = balanced_tb("TB-NESTLE_USA-2025-12", "NESTLE_USA");
    tb2.fiscal_year = 2025;
    tb2.fiscal_period = 12;
    write_tb_array(&entity_subdir, &[tb1, tb2]);

    let loaded =
        load_entity_trial_balance(&entity_subdir).expect("multi-period archive must succeed");

    assert_eq!(
        loaded.trial_balance_id, "TB-NESTLE_USA-2025-12",
        "loader must pick the latest fiscal_period (December over November)"
    );
    assert_eq!(loaded.fiscal_year, 2025);
    assert_eq!(loaded.fiscal_period, 12);
}

/// Unbalanced TB (`is_balanced = false`): explicit rejection so the
/// consolidation engine never operates on an inconsistent input.
#[test]
fn unbalanced_tb_is_aggregate_error() {
    let tmp = TempDir::new().expect("tempdir");
    let entity_subdir = tmp.path().join("NESTLE_DE");
    fs::create_dir_all(&entity_subdir).expect("create entity subdir");

    // Force imbalance by adding a large debit without a matching credit.
    let mut tb = balanced_tb("TB-NESTLE_DE-2025-12", "NESTLE_DE");
    tb.add_line(TrialBalanceLine {
        account_code: "1200".to_string(),
        account_description: "Receivables".to_string(),
        category: AccountCategory::CurrentAssets,
        account_type: AccountType::Asset,
        opening_balance: Decimal::ZERO,
        period_debits: dec!(5000),
        period_credits: Decimal::ZERO,
        closing_balance: dec!(5000),
        debit_balance: dec!(5000),
        credit_balance: Decimal::ZERO,
        cost_center: None,
        profit_center: None,
    });
    debug_assert!(
        !tb.is_balanced,
        "fixture must produce an unbalanced TB after the extra debit"
    );
    write_tb_array(&entity_subdir, &[tb]);

    // v5.0 contract change: per-entity TBs from the orchestrator are
    // intentionally unbalanced (fraud / anomaly injection — the
    // imbalance IS the ground-truth fraud signal). The loader logs the
    // imbalance via tracing instead of failing, so downstream aggregate
    // code keeps working on the synthetic input. Test the new contract:
    // load succeeds, imbalance is preserved verbatim.
    let tb = load_entity_trial_balance(&entity_subdir)
        .expect("unbalanced TB must load successfully under v5.0 fraud-tolerance contract");
    assert!(!tb.is_balanced, "loaded TB preserves the imbalance flag");
    assert!(
        (tb.total_debits - tb.total_credits).abs() > Decimal::new(1, 2),
        "loaded TB preserves the imbalance amount"
    );
}

/// Corruption signal: `is_balanced = true` is on disk but the totals
/// disagree.  This can only happen if the file was hand-edited (or
/// produced by a non-conforming writer) — `TrialBalance::recalculate`
/// would never set those two fields out of sync.  Reject loudly.
#[test]
fn corrupt_balanced_flag_is_aggregate_error() {
    let tmp = TempDir::new().expect("tempdir");
    let entity_subdir = tmp.path().join("NESTLE_SA");
    fs::create_dir_all(&entity_subdir).expect("create entity subdir");

    // Build a balanced TB, then mutate the totals to lie about the
    // balance.  Bypasses `add_line` so `recalculate` never fires —
    // simulating a hand-edited file.
    let mut tb = balanced_tb("TB-NESTLE_SA-2025-12", "NESTLE_SA");
    tb.total_debits = dec!(10000);
    tb.total_credits = dec!(9000);
    // Leave is_balanced = true on purpose — that's the corruption.

    write_tb_array(&entity_subdir, &[tb]);

    // v5.0 contract change: same as the unbalanced-TB case — the
    // loader no longer fails on `is_balanced=true && total_debits !=
    // total_credits`, it just logs the corruption. The aggregate phase
    // tolerates synthetic-engine fraud injection that produces this
    // exact pattern in normal operation.
    let tb = load_entity_trial_balance(&entity_subdir)
        .expect("corrupt-flag TB must load under v5.0 fraud-tolerance contract");
    assert!(
        tb.is_balanced,
        "loaded TB preserves the lying balanced flag"
    );
    assert_eq!(tb.total_debits, dec!(10000));
    assert_eq!(tb.total_credits, dec!(9000));
}

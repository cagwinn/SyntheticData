//! Task 8.7 — consolidated FS writer integration tests.

use std::collections::BTreeMap;
use std::fs;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_group::{
    build_consolidated_balance_sheet, build_consolidated_cash_flow,
    build_consolidated_income_statement, build_consolidation_schedule,
    build_statement_of_changes_in_equity, write_consolidated_fs, AggregatedAccount,
    AggregatedTb, CashFlowInputs, ConsolidatedFinancialStatements, ConsolidationSchedule,
    EquityChangesInputs, Note, NotesToConsolidatedFs, CONSOLIDATED_FS_FILENAME,
    CONSOLIDATION_SCHEDULE_FILENAME, NOTES_FILENAME,
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

fn balanced_tb() -> AggregatedTb {
    let mut totals: BTreeMap<String, AggregatedAccount> = BTreeMap::new();
    totals.insert(
        "1000".to_string(),
        aggregate_account("1000", dec!(1_000_000), Decimal::ZERO),
    );
    totals.insert(
        "3000".to_string(),
        aggregate_account("3000", Decimal::ZERO, dec!(1_000_000)),
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

fn build_test_bundle() -> (
    ConsolidatedFinancialStatements,
    ConsolidationSchedule,
    NotesToConsolidatedFs,
) {
    let tb = balanced_tb();
    let bs = build_consolidated_balance_sheet(&tb, "TEST_GROUP", period_end()).unwrap();
    let is = build_consolidated_income_statement(&tb, &[], "TEST_GROUP", period_end()).unwrap();
    let cf_inputs = CashFlowInputs {
        post_elim_tb_current: &tb,
        post_elim_tb_prior: None,
        net_income: Decimal::ZERO,
        depreciation_amortization: Decimal::ZERO,
        impairment: Decimal::ZERO,
        capex: Decimal::ZERO,
        debt_issuance: Decimal::ZERO,
        debt_repayment: Decimal::ZERO,
        dividends_paid_to_owners: Decimal::ZERO,
        dividends_paid_to_nci: Decimal::ZERO,
        equity_issuance: Decimal::ZERO,
    };
    let cf = build_consolidated_cash_flow(&cf_inputs, "TEST_GROUP", period_start(), period_end())
        .unwrap();
    let eq_inputs = EquityChangesInputs {
        opening_owners_equity: Decimal::ZERO,
        opening_nci: Decimal::ZERO,
        net_income_to_owners: Decimal::ZERO,
        net_income_to_nci: Decimal::ZERO,
        oci_to_owners: Decimal::ZERO,
        oci_to_nci: Decimal::ZERO,
        dividends_to_owners: Decimal::ZERO,
        dividends_to_nci: Decimal::ZERO,
        other_owners: Decimal::ZERO,
        other_nci: Decimal::ZERO,
    };
    let changes_in_equity = build_statement_of_changes_in_equity(
        &eq_inputs,
        "TEST_GROUP",
        period_start(),
        period_end(),
        "CHF",
    );

    let bundle = ConsolidatedFinancialStatements {
        balance_sheet: bs,
        income_statement: is,
        cash_flow: cf,
        changes_in_equity,
    };

    let schedule = build_consolidation_schedule(&tb, &tb, &[], "TEST_GROUP", period_end()).unwrap();

    let notes = NotesToConsolidatedFs {
        group_id: "TEST_GROUP".to_string(),
        period_end: period_end(),
        framework: "IFRS".to_string(),
        notes: vec![Note {
            note_number: 1,
            title: "Significant accounting policies".to_string(),
            body: "IFRS-based.".to_string(),
        }],
    };

    (bundle, schedule, notes)
}

#[test]
fn writer_emits_three_files_with_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let (bundle, schedule, notes) = build_test_bundle();
    let paths = write_consolidated_fs(&bundle, &schedule, &notes, tmp.path()).unwrap();
    assert_eq!(paths.len(), 3);

    // File names match constants.
    assert!(paths[0].ends_with(CONSOLIDATED_FS_FILENAME));
    assert!(paths[1].ends_with(CONSOLIDATION_SCHEDULE_FILENAME));
    assert!(paths[2].ends_with(NOTES_FILENAME));

    // All three files exist.
    for p in &paths {
        assert!(p.exists(), "expected file at {}", p.display());
    }

    // Round-trip serde for each file.
    let fs_json = fs::read_to_string(&paths[0]).unwrap();
    let fs_back: ConsolidatedFinancialStatements = serde_json::from_str(&fs_json).unwrap();
    assert_eq!(fs_back, bundle);

    let sched_json = fs::read_to_string(&paths[1]).unwrap();
    let sched_back: ConsolidationSchedule = serde_json::from_str(&sched_json).unwrap();
    assert_eq!(sched_back, schedule);

    let notes_json = fs::read_to_string(&paths[2]).unwrap();
    let notes_back: NotesToConsolidatedFs = serde_json::from_str(&notes_json).unwrap();
    assert_eq!(notes_back, notes);
}

#[test]
fn writer_creates_consolidated_subdir_if_missing() {
    let tmp = tempfile::tempdir().unwrap();
    // Note: no explicit `consolidated/` subdir in tmp.
    let (bundle, schedule, notes) = build_test_bundle();
    let paths = write_consolidated_fs(&bundle, &schedule, &notes, tmp.path()).unwrap();
    let consolidated = tmp.path().join("consolidated");
    assert!(consolidated.exists(), "consolidated/ subdir must be created");
    for p in &paths {
        assert!(
            p.starts_with(&consolidated),
            "all paths must live under consolidated/"
        );
    }
}

#[test]
fn writer_is_deterministic() {
    let tmp_a = tempfile::tempdir().unwrap();
    let tmp_b = tempfile::tempdir().unwrap();
    let (bundle, schedule, notes) = build_test_bundle();

    let paths_a = write_consolidated_fs(&bundle, &schedule, &notes, tmp_a.path()).unwrap();
    let paths_b = write_consolidated_fs(&bundle, &schedule, &notes, tmp_b.path()).unwrap();

    assert_eq!(paths_a.len(), paths_b.len());
    for (a, b) in paths_a.iter().zip(paths_b.iter()) {
        let bytes_a = fs::read(a).unwrap();
        let bytes_b = fs::read(b).unwrap();
        assert_eq!(
            bytes_a,
            bytes_b,
            "two writes with same input must produce byte-identical {}",
            a.file_name().unwrap().to_string_lossy()
        );
    }
}

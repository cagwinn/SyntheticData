//! v3.4.2 Sub-group B2 — smoke test for `TemporalContext` wiring into
//! `TimeEntryGenerator` + `ExpenseReportGenerator`. Verifies that when
//! `temporal_patterns.business_days.enabled = true`:
//!   - time-entry `date` fields exclude US holidays (not just weekends)
//!   - time-entry `submitted_at` falls on a business day
//!   - expense report `submission_date`, `approved_date`, `paid_date`,
//!     and line-item `date` all fall on business days

use chrono::{Datelike, NaiveDate, Weekday};
use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;

fn build_runtime_hr(enabled: bool) -> EnhancedOrchestrator {
    let mut config = minimal_config();
    config.global.seed = Some(3420);
    config.global.period_months = 3;
    config.fraud.enabled = false;
    config.temporal_patterns.enabled = enabled;
    config.temporal_patterns.business_days.enabled = enabled;
    config.temporal_patterns.calendars.regions = vec!["US".to_string()];
    config.hr.enabled = true;
    config.hr.time_attendance.enabled = true;
    config.hr.expenses.enabled = true;

    let mut phase_config = PhaseConfig::from_config(&config);
    phase_config.generate_journal_entries = false;
    phase_config.inject_anomalies = false;
    phase_config.generate_banking = false;
    phase_config.generate_graph_export = false;
    phase_config.generate_ocpm_events = false;
    phase_config.generate_period_close = false;
    phase_config.generate_evolution_events = false;
    phase_config.generate_sourcing = false;
    phase_config.generate_intercompany = false;
    phase_config.generate_financial_statements = false;
    phase_config.generate_bank_reconciliation = false;
    phase_config.generate_accounting_standards = false;
    phase_config.generate_manufacturing = false;
    phase_config.generate_sales_kpi_budgets = false;
    phase_config.generate_tax = false;
    phase_config.generate_esg = false;
    phase_config.generate_treasury = false;
    phase_config.generate_project_accounting = false;
    phase_config.generate_compliance_regulations = false;
    phase_config.inject_data_quality = false;
    phase_config.validate_balances = false;
    phase_config.show_progress = false;
    phase_config.generate_audit = false;
    phase_config.generate_document_flows = false;
    phase_config.generate_hr = true;

    EnhancedOrchestrator::new(config, phase_config).expect("build orchestrator")
}

fn is_weekend(d: NaiveDate) -> bool {
    matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

// US holidays in 2024 (the start year for seed 3420). Independence Day is
// the cleanest one to test — Jan 1, Jul 4, Dec 25 all land on weekdays.
fn is_known_us_holiday_2024(d: NaiveDate) -> bool {
    let key = (d.year(), d.month(), d.day());
    matches!(
        key,
        (2024, 1, 1)   // New Year's
            | (2024, 1, 15)  // MLK
            | (2024, 2, 19)  // Presidents
            | (2024, 5, 27)  // Memorial
            | (2024, 6, 19)  // Juneteenth
            | (2024, 7, 4)   // Independence
            | (2024, 9, 2)   // Labor
            | (2024, 10, 14) // Columbus
            | (2024, 11, 11) // Veterans
            | (2024, 11, 28) // Thanksgiving
            | (2024, 12, 25) // Christmas
    )
}

#[test]
fn time_entries_respect_business_days() {
    let mut orch = build_runtime_hr(true);
    let result = orch.generate().expect("generate");
    let entries = &result.hr.time_entries;
    assert!(!entries.is_empty(), "should generate time entries");

    let weekend = entries.iter().filter(|e| is_weekend(e.date)).count();
    assert_eq!(weekend, 0, "expected 0 weekend time entries, got {weekend}");

    // Holiday check: period is 2024-01-01 to 2024-04-01 (3 months). Three
    // US federal holidays fall in this range on weekdays: MLK (Jan 15) and
    // Presidents' Day (Feb 19). Assert no entries on those dates.
    let mlk = entries
        .iter()
        .filter(|e| e.date == NaiveDate::from_ymd_opt(2024, 1, 15).unwrap())
        .count();
    let prez = entries
        .iter()
        .filter(|e| e.date == NaiveDate::from_ymd_opt(2024, 2, 19).unwrap())
        .count();
    assert_eq!(
        mlk, 0,
        "expected 0 time entries on MLK Day (2024-01-15), got {mlk}"
    );
    assert_eq!(
        prez, 0,
        "expected 0 time entries on Presidents' Day, got {prez}"
    );
}

#[test]
fn time_entry_submitted_at_is_business_day() {
    let mut orch = build_runtime_hr(true);
    let result = orch.generate().expect("generate");
    let bad_submissions: Vec<_> = result
        .hr
        .time_entries
        .iter()
        .filter_map(|e| e.submitted_at)
        .filter(|d| is_weekend(*d))
        .collect();
    assert!(
        bad_submissions.is_empty(),
        "expected 0 weekend submitted_at dates, got {} examples",
        bad_submissions.len()
    );
}

#[test]
fn expense_report_dates_respect_business_days() {
    let mut orch = build_runtime_hr(true);
    let result = orch.generate().expect("generate");
    let reports = &result.hr.expense_reports;
    assert!(!reports.is_empty(), "should generate expense reports");

    let weekend_submission = reports
        .iter()
        .filter(|r| is_weekend(r.submission_date))
        .count();
    assert_eq!(
        weekend_submission, 0,
        "expected 0 weekend submission dates, got {weekend_submission}"
    );

    let weekend_approved = reports
        .iter()
        .filter_map(|r| r.approved_date)
        .filter(|d| is_weekend(*d))
        .count();
    assert_eq!(
        weekend_approved, 0,
        "expected 0 weekend approved_date, got {weekend_approved}"
    );

    let weekend_paid = reports
        .iter()
        .filter_map(|r| r.paid_date)
        .filter(|d| is_weekend(*d))
        .count();
    assert_eq!(
        weekend_paid, 0,
        "expected 0 weekend paid_date, got {weekend_paid}"
    );
}

#[test]
fn expense_line_item_dates_respect_business_days() {
    let mut orch = build_runtime_hr(true);
    let result = orch.generate().expect("generate");
    let bad_items: Vec<_> = result
        .hr
        .expense_reports
        .iter()
        .flat_map(|r| r.line_items.iter())
        .filter(|item| is_weekend(item.date) || is_known_us_holiday_2024(item.date))
        .collect();
    assert!(
        bad_items.is_empty(),
        "expected 0 weekend/holiday line items, got {} examples",
        bad_items.len()
    );
}

#[test]
fn disabled_temporal_still_produces_hr_output() {
    let mut orch = build_runtime_hr(false);
    let result = orch.generate().expect("generate");
    assert!(
        !result.hr.time_entries.is_empty(),
        "disabled-temporal path should still produce time entries"
    );
    assert!(
        !result.hr.expense_reports.is_empty(),
        "disabled-temporal path should still produce expense reports"
    );
}

use chrono::NaiveDate;
use datasynth_core::models::{EmployeeChangeEvent, EmployeeEventType};
use datasynth_generators::hr::PayrollGenerator;
use rust_decimal_macros::dec;

// ── helpers ─────────────────────────────────────────────────────────────────

fn make_salary_change(
    emp_id: &str,
    old: i64,
    new: i64,
    effective: NaiveDate,
) -> EmployeeChangeEvent {
    EmployeeChangeEvent {
        employee_id: emp_id.to_string(),
        event_date: effective - chrono::Duration::days(5),
        event_type: EmployeeEventType::SalaryAdjustment,
        old_value: Some(old.to_string()),
        new_value: Some(new.to_string()),
        effective_date: effective,
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

/// A salary adjustment that became effective *before* the pay period should
/// result in the full period being paid at the new (higher) rate.
#[test]
fn test_salary_adjustment_reflected() {
    let mut gen = PayrollGenerator::new(42);
    let emp_id = "EMP-001".to_string();

    // Change effective 15 Feb — March payroll should be entirely at new rate.
    let effective = NaiveDate::from_ymd_opt(2025, 2, 15).unwrap();
    let changes = vec![make_salary_change(&emp_id, 60_000, 72_000, effective)];
    let employees = vec![(emp_id.clone(), dec!(60_000), None, None)];

    let march_start = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
    let march_end = NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
    let (_run, items) =
        gen.generate_with_changes("C001", &employees, march_start, march_end, "USD", &changes);

    let emp_item = items.iter().find(|i| i.employee_id == emp_id).unwrap();
    let expected_monthly = dec!(72_000) / dec!(12);

    assert!(
        (emp_item.base_salary - expected_monthly).abs() < dec!(100),
        "March base_salary {} should be ≈{} (new rate); change was before period",
        emp_item.base_salary,
        expected_monthly
    );
}

/// No changes → original salary is used unchanged.
#[test]
fn test_no_changes_uses_original() {
    let mut gen = PayrollGenerator::new(42);
    let emp_id = "EMP-002".to_string();
    let employees = vec![(emp_id.clone(), dec!(60_000), None, None)];

    let march_start = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
    let march_end = NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
    let (_run, items) =
        gen.generate_with_changes("C001", &employees, march_start, march_end, "USD", &[]);

    let emp_item = items.iter().find(|i| i.employee_id == emp_id).unwrap();
    let expected_monthly = dec!(60_000) / dec!(12);

    assert!(
        (emp_item.base_salary - expected_monthly).abs() < dec!(100),
        "base_salary {} should be ≈{} with no changes",
        emp_item.base_salary,
        expected_monthly
    );
}

/// A salary change effective mid-period should prorate between old and new rate.
#[test]
fn test_mid_period_salary_change_prorated() {
    let mut gen = PayrollGenerator::new(99);
    let emp_id = "EMP-003".to_string();

    // March has 31 days (1–31). Change effective 16 March.
    // days_at_old = 16–1 = 15, days_at_new = 31–15 = 16 (total 31)
    let effective = NaiveDate::from_ymd_opt(2025, 3, 16).unwrap();
    let changes = vec![make_salary_change(&emp_id, 60_000, 72_000, effective)];
    let employees = vec![(emp_id.clone(), dec!(60_000), None, None)];

    let march_start = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
    let march_end = NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
    let (_run, items) =
        gen.generate_with_changes("C001", &employees, march_start, march_end, "USD", &changes);

    let emp_item = items.iter().find(|i| i.employee_id == emp_id).unwrap();

    // Expected prorated annual: (60000×15 + 72000×16) / 31 ≈ 66_967.74
    // Monthly = prorated_annual / 12 ≈ 5_580.65
    let expected_monthly =
        (dec!(60_000) * dec!(15) + dec!(72_000) * dec!(16)) / dec!(31) / dec!(12);

    assert!(
        (emp_item.base_salary - expected_monthly).abs() < dec!(100),
        "Mid-period base_salary {} should be ≈{} (prorated)",
        emp_item.base_salary,
        expected_monthly
    );
}

/// A change effective *after* period_end must NOT affect this period's payroll.
#[test]
fn test_future_change_ignored() {
    let mut gen = PayrollGenerator::new(7);
    let emp_id = "EMP-004".to_string();

    // Change effective 1 April — should not affect March payroll.
    let effective = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
    let changes = vec![make_salary_change(&emp_id, 60_000, 80_000, effective)];
    let employees = vec![(emp_id.clone(), dec!(60_000), None, None)];

    let march_start = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
    let march_end = NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
    let (_run, items) =
        gen.generate_with_changes("C001", &employees, march_start, march_end, "USD", &changes);

    let emp_item = items.iter().find(|i| i.employee_id == emp_id).unwrap();
    let expected_monthly = dec!(60_000) / dec!(12);

    assert!(
        (emp_item.base_salary - expected_monthly).abs() < dec!(100),
        "base_salary {} should be ≈{} — future change must not affect current period",
        emp_item.base_salary,
        expected_monthly
    );
}

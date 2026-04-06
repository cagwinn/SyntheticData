//! Tests for ESG ← HR bridge.
//!
//! Verifies that:
//!   - `WorkforceGenerator::generate_diversity_from_employees` correctly
//!     derives department-level diversity metrics from real `Employee` records.
//!   - `WorkforceGenerator::generate_pay_equity_from_payroll` correctly
//!     derives pay-gap ratios from real `PayrollLineItem` records.

#![allow(clippy::unwrap_used)]

use chrono::NaiveDate;
use datasynth_core::models::{Employee, OrganizationLevel, PayrollLineItem};
use datasynth_generators::esg::WorkforceGenerator;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Build a minimal `Employee` with a given department.
fn make_employee(id: &str, department_id: Option<&str>) -> Employee {
    let mut emp = Employee::new(id, id, "Test", "User", "C001");
    emp.department_id = department_id.map(|s| s.to_string());
    emp
}

/// Build a minimal `PayrollLineItem` with a given department and gross pay.
fn make_payroll_item(
    employee_id: &str,
    department: Option<&str>,
    cost_center: Option<&str>,
    gross_pay: Decimal,
) -> PayrollLineItem {
    PayrollLineItem {
        payroll_id: "PR-001".to_string(),
        employee_id: employee_id.to_string(),
        line_id: format!("LI-{employee_id}"),
        gross_pay,
        base_salary: gross_pay,
        overtime_pay: Decimal::ZERO,
        bonus: Decimal::ZERO,
        tax_withholding: Decimal::ZERO,
        social_security: Decimal::ZERO,
        health_insurance: Decimal::ZERO,
        retirement_contribution: Decimal::ZERO,
        other_deductions: Decimal::ZERO,
        net_pay: gross_pay,
        hours_worked: 160.0,
        overtime_hours: 0.0,
        pay_date: date(2025, 1, 31),
        cost_center: cost_center.map(|s| s.to_string()),
        department: department.map(|s| s.to_string()),
        tax_withholding_label: None,
        social_security_label: None,
        health_insurance_label: None,
        retirement_contribution_label: None,
        employer_contribution_label: None,
    }
}

fn gen() -> WorkforceGenerator {
    WorkforceGenerator::new(datasynth_config::schema::SocialConfig::default(), 42)
}

// ---------------------------------------------------------------------------
// generate_diversity_from_employees
// ---------------------------------------------------------------------------

#[test]
fn test_diversity_from_real_employees_department_counts() {
    let employees = vec![
        make_employee("E-001", Some("Finance")),
        make_employee("E-002", Some("Finance")),
        make_employee("E-003", Some("Finance")),
        make_employee("E-004", Some("Engineering")),
        make_employee("E-005", Some("Engineering")),
        make_employee("E-006", Some("HR")),
    ];

    let mut g = gen();
    let metrics = g.generate_diversity_from_employees("C001", &employees, date(2025, 1, 1));

    // 3 departments + 1 corporate total
    assert_eq!(
        metrics.len(),
        4,
        "Should produce 3 department records plus 1 corporate total"
    );

    // Find Finance record
    let finance = metrics
        .iter()
        .find(|m| m.category == "Finance" && m.level == OrganizationLevel::Department)
        .expect("Finance department record should exist");

    assert_eq!(finance.headcount, 3);
    assert_eq!(finance.total_headcount, 6);
    assert_eq!(finance.percentage, dec!(0.5000));

    // Find Engineering record
    let eng = metrics
        .iter()
        .find(|m| m.category == "Engineering" && m.level == OrganizationLevel::Department)
        .expect("Engineering department record should exist");

    assert_eq!(eng.headcount, 2);
    assert_eq!(eng.total_headcount, 6);

    // Headcount sums across department records should equal total
    let dept_sum: u32 = metrics
        .iter()
        .filter(|m| m.level == OrganizationLevel::Department)
        .map(|m| m.headcount)
        .sum();
    assert_eq!(
        dept_sum, 6,
        "Department headcounts should sum to total headcount"
    );
}

#[test]
fn test_diversity_corporate_total_record() {
    let employees = vec![
        make_employee("E-001", Some("Sales")),
        make_employee("E-002", Some("Sales")),
        make_employee("E-003", Some("IT")),
    ];

    let mut g = gen();
    let metrics = g.generate_diversity_from_employees("C001", &employees, date(2025, 3, 31));

    let corporate = metrics
        .iter()
        .find(|m| m.level == OrganizationLevel::Corporate)
        .expect("Corporate total record should exist");

    assert_eq!(corporate.headcount, 3);
    assert_eq!(corporate.total_headcount, 3);
    assert_eq!(corporate.percentage, dec!(1.0000));
    assert_eq!(corporate.category, "All");
}

#[test]
fn test_diversity_employees_without_department_bucketed_as_unknown() {
    let employees = vec![
        make_employee("E-001", None), // no department
        make_employee("E-002", None), // no department
        make_employee("E-003", Some("Finance")),
    ];

    let mut g = gen();
    let metrics = g.generate_diversity_from_employees("C001", &employees, date(2025, 6, 30));

    let unknown = metrics
        .iter()
        .find(|m| m.category == "Unknown" && m.level == OrganizationLevel::Department)
        .expect("'Unknown' bucket should exist for employees without department_id");

    assert_eq!(unknown.headcount, 2);
}

#[test]
fn test_diversity_empty_employees_returns_empty() {
    let mut g = gen();
    let metrics = g.generate_diversity_from_employees("C001", &[], date(2025, 1, 1));
    assert!(
        metrics.is_empty(),
        "Empty employee slice should produce no metrics"
    );
}

// ---------------------------------------------------------------------------
// generate_pay_equity_from_payroll
// ---------------------------------------------------------------------------

#[test]
fn test_pay_equity_from_payroll_ratio_calculation() {
    // Finance avg = (9_000 + 9_000) / 2 = 9_000  → reference (highest)
    // Engineering avg = (6_000 + 6_000) / 2 = 6_000
    // Expected ratio = 6_000 / 9_000 ≈ 0.6667
    let items = vec![
        make_payroll_item("E-001", Some("Finance"), None, dec!(9000)),
        make_payroll_item("E-002", Some("Finance"), None, dec!(9000)),
        make_payroll_item("E-003", Some("Engineering"), None, dec!(6000)),
        make_payroll_item("E-004", Some("Engineering"), None, dec!(6000)),
    ];

    let mut g = gen();
    let metrics = g.generate_pay_equity_from_payroll("C001", &items, date(2025, 1, 31));

    assert_eq!(
        metrics.len(),
        1,
        "Two groups should produce one pay equity metric"
    );

    let m = &metrics[0];
    assert_eq!(m.reference_group, "Finance");
    assert_eq!(m.comparison_group, "Engineering");
    assert_eq!(m.reference_median_salary, dec!(9000.00));
    assert_eq!(m.comparison_median_salary, dec!(6000.00));
    assert_eq!(m.pay_gap_ratio, dec!(0.6667));
    assert_eq!(m.sample_size, 4);
}

#[test]
fn test_pay_equity_cost_center_fallback() {
    // No department set — should fall back to cost_center
    let items = vec![
        make_payroll_item("E-001", None, Some("CC-100"), dec!(8000)),
        make_payroll_item("E-002", None, Some("CC-100"), dec!(8000)),
        make_payroll_item("E-003", None, Some("CC-200"), dec!(5000)),
    ];

    let mut g = gen();
    let metrics = g.generate_pay_equity_from_payroll("C001", &items, date(2025, 1, 31));

    assert_eq!(
        metrics.len(),
        1,
        "Should fall back to cost_center when department is absent"
    );

    let m = &metrics[0];
    assert_eq!(m.reference_group, "CC-100");
    assert_eq!(m.comparison_group, "CC-200");
}

#[test]
fn test_pay_equity_three_groups_produces_two_comparisons() {
    // Alpha=10_000, Beta=7_000, Gamma=4_000 → 2 metrics (Beta vs Alpha, Gamma vs Alpha)
    let items = vec![
        make_payroll_item("E-001", Some("Alpha"), None, dec!(10000)),
        make_payroll_item("E-002", Some("Beta"), None, dec!(7000)),
        make_payroll_item("E-003", Some("Gamma"), None, dec!(4000)),
    ];

    let mut g = gen();
    let metrics = g.generate_pay_equity_from_payroll("C001", &items, date(2025, 1, 31));

    assert_eq!(
        metrics.len(),
        2,
        "Three groups should produce two pay equity comparisons"
    );

    // Reference group should be Alpha (highest avg pay)
    for m in &metrics {
        assert_eq!(m.reference_group, "Alpha");
        assert!(
            m.pay_gap_ratio < dec!(1.00),
            "Comparison groups earn less than reference"
        );
    }
}

#[test]
fn test_pay_equity_single_group_returns_empty() {
    let items = vec![
        make_payroll_item("E-001", Some("Finance"), None, dec!(8000)),
        make_payroll_item("E-002", Some("Finance"), None, dec!(9000)),
    ];

    let mut g = gen();
    let metrics = g.generate_pay_equity_from_payroll("C001", &items, date(2025, 1, 31));

    assert!(
        metrics.is_empty(),
        "A single group has nothing to compare against"
    );
}

#[test]
fn test_pay_equity_empty_items_returns_empty() {
    let mut g = gen();
    let metrics = g.generate_pay_equity_from_payroll("C001", &[], date(2025, 1, 31));
    assert!(metrics.is_empty(), "Empty payroll items should produce no metrics");
}

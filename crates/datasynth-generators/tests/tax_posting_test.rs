//! Tests verifying that TaxProvisionGenerator scales with realistic pre-tax income values.
//!
//! Ensures the generator does not rely on any hardcoded PTI (previously 1_000_000 in the
//! orchestrator) and that provisions are proportional to the supplied income figure.

#![allow(clippy::unwrap_used)]

use datasynth_generators::tax::TaxProvisionGenerator;
use rust_decimal_macros::dec;

#[test]
fn test_tax_provision_with_realistic_pti() {
    let mut gen = TaxProvisionGenerator::new(42);
    let provision = gen.generate(
        "C001",
        chrono::NaiveDate::from_ymd_opt(2025, 3, 31).unwrap(),
        dec!(500_000), // realistic PTI
        dec!(0.21),
    );
    // Current tax expense should be proportional to PTI (500K * ~21% = ~105K).
    // Reconciliation items can nudge the ETR, so allow a wider band.
    assert!(
        provision.current_tax_expense > dec!(50_000),
        "current_tax_expense {} too low for PTI of 500K",
        provision.current_tax_expense
    );
    assert!(
        provision.current_tax_expense < dec!(200_000),
        "current_tax_expense {} too high for PTI of 500K",
        provision.current_tax_expense
    );
    let diff = (provision.effective_rate - dec!(0.21)).abs();
    assert!(
        diff < dec!(0.10),
        "ETR {} should be near 21%",
        provision.effective_rate
    );
}

#[test]
fn test_tax_provision_negative_pti() {
    let mut gen = TaxProvisionGenerator::new(42);
    let provision = gen.generate(
        "C001",
        chrono::NaiveDate::from_ymd_opt(2025, 3, 31).unwrap(),
        dec!(-100_000), // loss year
        dec!(0.21),
    );
    // With a loss, tax benefit (negative expense) or zero is expected.
    // Most important: no panic and effective rate stays within a reasonable range.
    assert!(
        provision.effective_rate.abs() < dec!(1.0),
        "effective_rate {} out of range for a loss year",
        provision.effective_rate
    );
}

#[test]
fn test_tax_provision_scales_with_pti() {
    // Verify that doubling PTI roughly doubles the tax expense (same seed / reconciliation).
    let mut gen_small = TaxProvisionGenerator::new(99);
    let mut gen_large = TaxProvisionGenerator::new(99);

    let small = gen_small.generate(
        "C001",
        chrono::NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(),
        dec!(250_000),
        dec!(0.21),
    );
    let large = gen_large.generate(
        "C001",
        chrono::NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(),
        dec!(500_000),
        dec!(0.21),
    );

    // Both generators use the same seed so they pick the same reconciliation items and
    // the same effective rate — meaning large.expense should be exactly 2× small.expense.
    let ratio = large.current_tax_expense / small.current_tax_expense;
    let deviation = (ratio - dec!(2.0)).abs();
    assert!(
        deviation < dec!(0.01),
        "expense ratio {} is not ~2× when PTI doubles",
        ratio
    );
}

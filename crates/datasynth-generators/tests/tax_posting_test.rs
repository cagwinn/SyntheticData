//! Tests verifying that TaxProvisionGenerator scales with realistic pre-tax income values.
//!
//! Ensures the generator does not rely on any hardcoded PTI (previously 1_000_000 in the
//! orchestrator) and that provisions are proportional to the supplied income figure.

#![allow(clippy::unwrap_used)]

use datasynth_core::models::{TaxLine, TaxableDocumentType};
use datasynth_generators::tax::{TaxPostingGenerator, TaxProvisionGenerator};
use rust_decimal::Decimal;
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

// ---------------------------------------------------------------------------
// TaxPostingGenerator tests
// ---------------------------------------------------------------------------

#[test]
fn test_output_vat_posting() {
    // Customer invoice → output VAT
    let tax_lines = vec![make_tax_line(
        "TL-001",
        TaxableDocumentType::CustomerInvoice,
        dec!(2000),
        false,
    )];
    let doc_date = chrono::NaiveDate::from_ymd_opt(2025, 1, 15).unwrap();
    let fallback = chrono::NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
    let mut doc_dates = std::collections::HashMap::new();
    doc_dates.insert("DOC-TL-001".to_string(), doc_date);

    let jes =
        TaxPostingGenerator::generate_tax_posting_jes(&tax_lines, "C001", &doc_dates, fallback);
    assert_eq!(jes.len(), 1);
    assert!(jes[0].is_balanced());
    let has_vat_payable = jes[0].lines.iter().any(|l| l.gl_account == "2110");
    assert!(has_vat_payable, "Should credit VAT Payable");
    // The JE date should come from the doc_dates map, not the fallback.
    assert_eq!(
        jes[0].header.posting_date, doc_date,
        "JE date should match document date, not fallback period-end"
    );
}

#[test]
fn test_input_vat_posting() {
    // Vendor invoice (deductible) → input VAT
    let tax_lines = vec![make_tax_line(
        "TL-002",
        TaxableDocumentType::VendorInvoice,
        dec!(1000),
        true,
    )];
    let doc_date = chrono::NaiveDate::from_ymd_opt(2025, 2, 20).unwrap();
    let fallback = chrono::NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
    let mut doc_dates = std::collections::HashMap::new();
    doc_dates.insert("DOC-TL-002".to_string(), doc_date);

    let jes =
        TaxPostingGenerator::generate_tax_posting_jes(&tax_lines, "C001", &doc_dates, fallback);
    assert_eq!(jes.len(), 1);
    assert!(jes[0].is_balanced());
    let has_input_vat = jes[0].lines.iter().any(|l| l.gl_account == "1160");
    assert!(has_input_vat, "Should debit Input VAT");
    assert_eq!(
        jes[0].header.posting_date, doc_date,
        "JE date should match document date, not fallback period-end"
    );
}

#[test]
fn test_non_deductible_skipped() {
    // Vendor invoice (non-deductible) → no separate posting
    let tax_lines = vec![make_tax_line(
        "TL-003",
        TaxableDocumentType::VendorInvoice,
        dec!(500),
        false,
    )];
    let fallback = chrono::NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
    let jes = TaxPostingGenerator::generate_tax_posting_jes(
        &tax_lines,
        "C001",
        &std::collections::HashMap::new(),
        fallback,
    );
    assert!(
        jes.is_empty(),
        "Non-deductible vendor tax should not generate JE"
    );
}

#[test]
fn test_fallback_date_used_when_doc_not_in_map() {
    // When document_id is not in doc_dates, the fallback date is used.
    let tax_lines = vec![make_tax_line(
        "TL-004",
        TaxableDocumentType::CustomerInvoice,
        dec!(750),
        false,
    )];
    let fallback = chrono::NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
    // Empty map — no matching entry.
    let jes = TaxPostingGenerator::generate_tax_posting_jes(
        &tax_lines,
        "C001",
        &std::collections::HashMap::new(),
        fallback,
    );
    assert_eq!(jes.len(), 1);
    assert_eq!(
        jes[0].header.posting_date, fallback,
        "Should use fallback date when document not found in map"
    );
}

fn make_tax_line(
    id: &str,
    doc_type: TaxableDocumentType,
    amount: Decimal,
    deductible: bool,
) -> TaxLine {
    TaxLine {
        id: id.to_string(),
        document_type: doc_type,
        document_id: format!("DOC-{}", id),
        line_number: 1,
        tax_code_id: "VAT-STD-20".to_string(),
        jurisdiction_id: "DE-FED".to_string(),
        taxable_amount: amount * dec!(5), // taxable base
        tax_amount: amount,
        is_deductible: deductible,
        is_reverse_charge: false,
        is_self_assessed: false,
    }
}

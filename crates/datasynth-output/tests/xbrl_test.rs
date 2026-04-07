//! Integration tests for the XBRL exporter.

use chrono::NaiveDate;
use datasynth_core::{
    CashFlowCategory, CashFlowItem, FinancialStatement, FinancialStatementLineItem, StatementBasis,
    StatementType,
};
use datasynth_output::XbrlExporter;
use rust_decimal_macros::dec;

/// Build a minimal FinancialStatement for testing.
fn make_test_statement(
    basis: StatementBasis,
    statement_type: StatementType,
    line_items: Vec<FinancialStatementLineItem>,
    cash_flow_items: Vec<CashFlowItem>,
) -> FinancialStatement {
    FinancialStatement {
        statement_id: "FS-TEST-001".to_string(),
        company_code: "TESTCO".to_string(),
        statement_type,
        basis,
        period_start: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
        period_end: NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(),
        fiscal_year: 2025,
        fiscal_period: 12,
        line_items,
        cash_flow_items,
        currency: "USD".to_string(),
        is_consolidated: false,
        preparer_id: "USR-TEST".to_string(),
    }
}

fn make_line_item(
    line_code: &str,
    label: &str,
    amount: rust_decimal::Decimal,
) -> FinancialStatementLineItem {
    FinancialStatementLineItem {
        line_code: line_code.to_string(),
        label: label.to_string(),
        section: "Test Section".to_string(),
        sort_order: 1,
        amount,
        amount_prior: None,
        prior_year_amount: None,
        assumptions: None,
        indent_level: 0,
        is_total: false,
        gl_accounts: vec![],
    }
}

fn make_cash_flow_item(
    item_code: &str,
    label: &str,
    amount: rust_decimal::Decimal,
) -> CashFlowItem {
    CashFlowItem {
        item_code: item_code.to_string(),
        label: label.to_string(),
        category: CashFlowCategory::Operating,
        amount,
        amount_prior: None,
        sort_order: 1,
        is_total: false,
    }
}

#[test]
fn xbrl_output_contains_xml_header() {
    let stmt = make_test_statement(
        StatementBasis::UsGaap,
        StatementType::BalanceSheet,
        vec![make_line_item("BS-CASH", "Cash", dec!(100000))],
        vec![],
    );
    let xml = XbrlExporter::export(&stmt);
    assert!(
        xml.starts_with("<?xml version"),
        "Output must start with XML declaration"
    );
}

#[test]
fn xbrl_output_contains_namespace_declarations() {
    let stmt = make_test_statement(
        StatementBasis::UsGaap,
        StatementType::BalanceSheet,
        vec![make_line_item("BS-CASH", "Cash", dec!(100000))],
        vec![],
    );
    let xml = XbrlExporter::export(&stmt);
    assert!(xml.contains("xmlns:xbrli=\"http://www.xbrl.org/2003/instance\""));
    assert!(xml.contains("xmlns:us-gaap=\"http://fasb.org/us-gaap/2024\""));
    assert!(xml.contains("xmlns:ifrs-full=\"http://xbrl.ifrs.org/taxonomy/2024\""));
    assert!(xml.contains("xmlns:iso4217=\"http://www.xbrl.org/2003/iso4217\""));
    assert!(xml.contains("xmlns:xlink=\"http://www.w3.org/1999/xlink\""));
}

#[test]
fn xbrl_bs_cash_maps_to_us_gaap_element() {
    let stmt = make_test_statement(
        StatementBasis::UsGaap,
        StatementType::BalanceSheet,
        vec![make_line_item("BS-CASH", "Cash", dec!(250000))],
        vec![],
    );
    let xml = XbrlExporter::export(&stmt);
    assert!(
        xml.contains("us-gaap:CashAndCashEquivalentsAtCarryingValue"),
        "BS-CASH must map to us-gaap:CashAndCashEquivalentsAtCarryingValue"
    );
    assert!(xml.contains(">250000<"));
}

#[test]
fn xbrl_bs_items_use_instant_context() {
    let stmt = make_test_statement(
        StatementBasis::UsGaap,
        StatementType::BalanceSheet,
        vec![make_line_item("BS-AR", "Accounts Receivable", dec!(50000))],
        vec![],
    );
    let xml = XbrlExporter::export(&stmt);
    assert!(
        xml.contains("contextRef=\"FY2025-instant\""),
        "Balance sheet items must use instant context"
    );
}

#[test]
fn xbrl_is_items_use_duration_context() {
    let stmt = make_test_statement(
        StatementBasis::UsGaap,
        StatementType::IncomeStatement,
        vec![make_line_item("IS-REV", "Revenue", dec!(1000000))],
        vec![],
    );
    let xml = XbrlExporter::export(&stmt);
    assert!(
        xml.contains("<us-gaap:Revenues\n    contextRef=\"FY2025\""),
        "Income statement items must use duration context (FY2025, not FY2025-instant)"
    );
}

#[test]
fn xbrl_unmapped_line_codes_are_skipped() {
    let stmt = make_test_statement(
        StatementBasis::UsGaap,
        StatementType::BalanceSheet,
        vec![
            make_line_item("BS-CASH", "Cash", dec!(100000)),
            make_line_item("BS-CUSTOM-XYZ", "Custom Item", dec!(9999)),
        ],
        vec![],
    );
    let xml = XbrlExporter::export(&stmt);
    assert!(
        xml.contains("us-gaap:CashAndCashEquivalentsAtCarryingValue"),
        "Mapped item must appear"
    );
    assert!(
        !xml.contains("9999"),
        "Unmapped item amount must not appear"
    );
    assert!(
        !xml.contains("BS-CUSTOM-XYZ"),
        "Unmapped line code must not appear"
    );
}

#[test]
fn xbrl_ifrs_basis_uses_ifrs_elements() {
    let stmt = make_test_statement(
        StatementBasis::Ifrs,
        StatementType::BalanceSheet,
        vec![
            make_line_item("BS-CASH", "Cash", dec!(200000)),
            make_line_item("IS-REV", "Revenue", dec!(500000)),
        ],
        vec![],
    );
    let xml = XbrlExporter::export(&stmt);
    assert!(xml.contains("ifrs-full:CashAndCashEquivalents"));
    assert!(xml.contains("ifrs-full:Revenue"));
    assert!(
        !xml.contains("us-gaap:"),
        "US GAAP elements must not appear for IFRS statements"
    );
}

#[test]
fn xbrl_full_balance_sheet_roundtrip() {
    let stmt = make_test_statement(
        StatementBasis::UsGaap,
        StatementType::BalanceSheet,
        vec![
            make_line_item("BS-CASH", "Cash", dec!(100000)),
            make_line_item("BS-AR", "Accounts Receivable", dec!(75000)),
            make_line_item("BS-INV", "Inventory", dec!(50000)),
            make_line_item("BS-PPE", "PP&E", dec!(300000)),
            make_line_item("BS-TOTAL-ASSETS", "Total Assets", dec!(525000)),
            make_line_item("BS-AP", "Accounts Payable", dec!(60000)),
            make_line_item("BS-LT-DEBT", "Long-term Debt", dec!(200000)),
            make_line_item("BS-TOTAL-LIAB", "Total Liabilities", dec!(260000)),
            make_line_item("BS-EQUITY", "Stockholders Equity", dec!(265000)),
        ],
        vec![],
    );
    let xml = XbrlExporter::export(&stmt);

    // All mapped elements should be present
    assert!(xml.contains("us-gaap:CashAndCashEquivalentsAtCarryingValue"));
    assert!(xml.contains("us-gaap:AccountsReceivableNetCurrent"));
    assert!(xml.contains("us-gaap:InventoryNet"));
    assert!(xml.contains("us-gaap:PropertyPlantAndEquipmentNet"));
    assert!(xml.contains("us-gaap:Assets"));
    assert!(xml.contains("us-gaap:AccountsPayableCurrent"));
    assert!(xml.contains("us-gaap:LongTermDebt"));
    assert!(xml.contains("us-gaap:Liabilities"));
    assert!(xml.contains("us-gaap:StockholdersEquity"));

    // All should use instant context
    let instant_count = xml.matches("contextRef=\"FY2025-instant\"").count();
    assert_eq!(instant_count, 9, "All 9 BS items must use instant context");
}

#[test]
fn xbrl_cash_flow_items_mapped_and_use_duration_context() {
    let stmt = make_test_statement(
        StatementBasis::UsGaap,
        StatementType::CashFlowStatement,
        vec![],
        vec![make_cash_flow_item(
            "IS-NET-INCOME",
            "Net Income",
            dec!(120000),
        )],
    );
    let xml = XbrlExporter::export(&stmt);
    assert!(xml.contains("us-gaap:NetIncomeLoss"));
    // Cash flow items always use duration context
    assert!(xml.contains("contextRef=\"FY2025\""));
}

//! XBRL 2.1 instance document export.
//!
//! Exports a [`FinancialStatement`] as an XBRL instance document XML string.
//! Supports both US GAAP and IFRS taxonomy element mappings.
//!
//! This is a minimal-but-valid XBRL instance intended for compliance tool
//! validation of taxonomy tagging, not a full iXBRL rendering.

use datasynth_core::{FinancialStatement, StatementBasis};

/// XBRL instance document exporter.
///
/// Takes a [`FinancialStatement`] and produces an XML string following the
/// XBRL 2.1 instance document format with proper context references
/// (instant for balance sheet, duration for income statement items).
pub struct XbrlExporter;

impl XbrlExporter {
    /// Export a financial statement as an XBRL instance document XML string.
    pub fn export(statement: &FinancialStatement) -> String {
        let mut xml = String::with_capacity(4096);

        // XML declaration
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");

        // Root element with namespace declarations
        xml.push_str("<xbrli:xbrl\n");
        xml.push_str("  xmlns:xbrli=\"http://www.xbrl.org/2003/instance\"\n");
        xml.push_str("  xmlns:us-gaap=\"http://fasb.org/us-gaap/2024\"\n");
        xml.push_str("  xmlns:ifrs-full=\"http://xbrl.ifrs.org/taxonomy/2024\"\n");
        xml.push_str("  xmlns:iso4217=\"http://www.xbrl.org/2003/iso4217\"\n");
        xml.push_str("  xmlns:xlink=\"http://www.w3.org/1999/xlink\">\n");

        let context_id = format!("FY{}", statement.fiscal_year);
        let instant_context_id = format!("FY{}-instant", statement.fiscal_year);
        let currency = &statement.currency;
        let company_code = escape_xml(&statement.company_code);
        let period_start = statement.period_start;
        let period_end = statement.period_end;

        // Duration context (for income statement / cash flow items)
        xml.push('\n');
        xml.push_str(&format!("  <xbrli:context id=\"{context_id}\">\n"));
        xml.push_str("    <xbrli:entity>\n");
        xml.push_str(&format!(
            "      <xbrli:identifier scheme=\"http://example.com/entity\">{company_code}</xbrli:identifier>\n"
        ));
        xml.push_str("    </xbrli:entity>\n");
        xml.push_str("    <xbrli:period>\n");
        xml.push_str(&format!(
            "      <xbrli:startDate>{period_start}</xbrli:startDate>\n"
        ));
        xml.push_str(&format!(
            "      <xbrli:endDate>{period_end}</xbrli:endDate>\n"
        ));
        xml.push_str("    </xbrli:period>\n");
        xml.push_str("  </xbrli:context>\n");

        // Instant context (for balance sheet items)
        xml.push('\n');
        xml.push_str(&format!("  <xbrli:context id=\"{instant_context_id}\">\n"));
        xml.push_str("    <xbrli:entity>\n");
        xml.push_str(&format!(
            "      <xbrli:identifier scheme=\"http://example.com/entity\">{company_code}</xbrli:identifier>\n"
        ));
        xml.push_str("    </xbrli:entity>\n");
        xml.push_str("    <xbrli:period>\n");
        xml.push_str(&format!(
            "      <xbrli:instant>{period_end}</xbrli:instant>\n"
        ));
        xml.push_str("    </xbrli:period>\n");
        xml.push_str("  </xbrli:context>\n");

        // Unit
        xml.push('\n');
        xml.push_str(&format!("  <xbrli:unit id=\"{currency}\">\n"));
        xml.push_str(&format!(
            "    <xbrli:measure>iso4217:{currency}</xbrli:measure>\n"
        ));
        xml.push_str("  </xbrli:unit>\n");

        // Determine taxonomy mapper based on basis
        let element_mapper: fn(&str) -> Option<&'static str> = match statement.basis {
            StatementBasis::Ifrs => ifrs_element,
            StatementBasis::UsGaap | StatementBasis::Statutory => us_gaap_element,
        };

        // Emit line items
        xml.push('\n');
        for item in &statement.line_items {
            if let Some(element) = element_mapper(&item.line_code) {
                // BS items use instant context; IS items use duration context
                let ctx_ref = if item.line_code.starts_with("BS-") {
                    &instant_context_id
                } else {
                    &context_id
                };
                xml.push_str(&format!(
                    "  <{element}\n    contextRef=\"{ctx_ref}\" unitRef=\"{currency}\" decimals=\"-3\">{}</{element}>\n",
                    item.amount,
                ));
            }
        }

        // Emit cash flow items (duration context, mapped via line_code-style lookup)
        for cf_item in &statement.cash_flow_items {
            if let Some(element) = element_mapper(&cf_item.item_code) {
                xml.push_str(&format!(
                    "  <{element}\n    contextRef=\"{context_id}\" unitRef=\"{currency}\" decimals=\"-3\">{}</{element}>\n",
                    cf_item.amount,
                ));
            }
        }

        xml.push_str("\n</xbrli:xbrl>\n");
        xml
    }
}

/// Map a line code to a US GAAP XBRL taxonomy element.
fn us_gaap_element(line_code: &str) -> Option<&'static str> {
    match line_code {
        "BS-CASH" => Some("us-gaap:CashAndCashEquivalentsAtCarryingValue"),
        "BS-AR" => Some("us-gaap:AccountsReceivableNetCurrent"),
        "BS-INV" => Some("us-gaap:InventoryNet"),
        "BS-PPE" | "BS-FA" => Some("us-gaap:PropertyPlantAndEquipmentNet"),
        "BS-TOTAL-ASSETS" => Some("us-gaap:Assets"),
        "BS-AP" => Some("us-gaap:AccountsPayableCurrent"),
        "BS-ACCRUED" => Some("us-gaap:AccruedLiabilitiesCurrent"),
        "BS-LT-DEBT" | "BS-DEBT" => Some("us-gaap:LongTermDebt"),
        "BS-TOTAL-LIAB" => Some("us-gaap:Liabilities"),
        "BS-EQUITY" | "BS-TOTAL-EQUITY" => Some("us-gaap:StockholdersEquity"),
        "IS-REV" | "IS-REVENUE" => Some("us-gaap:Revenues"),
        "IS-COGS" => Some("us-gaap:CostOfGoodsAndServicesSold"),
        "IS-GROSS-PROFIT" => Some("us-gaap:GrossProfit"),
        "IS-OPEX" => Some("us-gaap:OperatingExpenses"),
        "IS-OP-INCOME" => Some("us-gaap:OperatingIncomeLoss"),
        "IS-NET-INCOME" | "IS-NI" => Some("us-gaap:NetIncomeLoss"),
        _ => None,
    }
}

/// Map a line code to an IFRS XBRL taxonomy element.
fn ifrs_element(line_code: &str) -> Option<&'static str> {
    match line_code {
        "BS-CASH" => Some("ifrs-full:CashAndCashEquivalents"),
        "BS-AR" => Some("ifrs-full:TradeAndOtherCurrentReceivables"),
        "BS-INV" => Some("ifrs-full:Inventories"),
        "BS-PPE" | "BS-FA" => Some("ifrs-full:PropertyPlantAndEquipment"),
        "BS-TOTAL-ASSETS" => Some("ifrs-full:Assets"),
        "BS-AP" => Some("ifrs-full:TradeAndOtherCurrentPayables"),
        "BS-LT-DEBT" | "BS-DEBT" => Some("ifrs-full:BorrowingsNoncurrent"),
        "BS-TOTAL-LIAB" => Some("ifrs-full:Liabilities"),
        "BS-EQUITY" | "BS-TOTAL-EQUITY" => Some("ifrs-full:Equity"),
        "IS-REV" | "IS-REVENUE" => Some("ifrs-full:Revenue"),
        "IS-COGS" => Some("ifrs-full:CostOfSales"),
        "IS-GROSS-PROFIT" => Some("ifrs-full:GrossProfit"),
        "IS-NET-INCOME" | "IS-NI" => Some("ifrs-full:ProfitLoss"),
        _ => None,
    }
}

/// Escape special XML characters in a string.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use datasynth_core::{
        CashFlowCategory, CashFlowItem, FinancialStatementLineItem, StatementType,
    };
    use rust_decimal_macros::dec;

    /// Build a minimal FinancialStatement for testing.
    fn make_test_statement(
        basis: StatementBasis,
        statement_type: StatementType,
        line_items: Vec<FinancialStatementLineItem>,
        cash_flow_items: Vec<CashFlowItem>,
    ) -> FinancialStatement {
        FinancialStatement {
            statement_id: "FS-001".to_string(),
            company_code: "ACME".to_string(),
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
            preparer_id: "USR-001".to_string(),
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
    fn test_xml_header_present() {
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
    fn test_namespace_declarations() {
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
    fn test_us_gaap_cash_element() {
        let stmt = make_test_statement(
            StatementBasis::UsGaap,
            StatementType::BalanceSheet,
            vec![make_line_item("BS-CASH", "Cash", dec!(250000))],
            vec![],
        );
        let xml = XbrlExporter::export(&stmt);
        assert!(
            xml.contains("us-gaap:CashAndCashEquivalentsAtCarryingValue"),
            "BS-CASH must map to the US GAAP cash element"
        );
        assert!(
            xml.contains(">250000<"),
            "Amount must appear as element content"
        );
    }

    #[test]
    fn test_bs_items_use_instant_context() {
        let stmt = make_test_statement(
            StatementBasis::UsGaap,
            StatementType::BalanceSheet,
            vec![make_line_item("BS-AR", "Accounts Receivable", dec!(50000))],
            vec![],
        );
        let xml = XbrlExporter::export(&stmt);
        assert!(
            xml.contains("contextRef=\"FY2025-instant\""),
            "Balance sheet items must reference the instant context"
        );
    }

    #[test]
    fn test_is_items_use_duration_context() {
        let stmt = make_test_statement(
            StatementBasis::UsGaap,
            StatementType::IncomeStatement,
            vec![make_line_item("IS-REV", "Revenue", dec!(1000000))],
            vec![],
        );
        let xml = XbrlExporter::export(&stmt);

        // The IS item should reference the duration context (FY2025),
        // not the instant context (FY2025-instant).
        assert!(
            xml.contains("<us-gaap:Revenues\n    contextRef=\"FY2025\""),
            "Income statement items must reference the duration context"
        );
    }

    #[test]
    fn test_unmapped_line_codes_are_skipped() {
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
            "Unmapped item amount must not appear in output"
        );
        assert!(
            !xml.contains("BS-CUSTOM-XYZ"),
            "Unmapped line code must not appear in output"
        );
    }

    #[test]
    fn test_ifrs_taxonomy_mapping() {
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
        assert!(
            xml.contains("ifrs-full:CashAndCashEquivalents"),
            "IFRS cash element must be used"
        );
        assert!(
            !xml.contains("us-gaap:"),
            "US GAAP elements must not appear for IFRS statements"
        );
        assert!(
            xml.contains("ifrs-full:Revenue"),
            "IFRS revenue element must be used"
        );
    }

    #[test]
    fn test_context_contains_company_code_and_dates() {
        let stmt = make_test_statement(
            StatementBasis::UsGaap,
            StatementType::BalanceSheet,
            vec![make_line_item("BS-CASH", "Cash", dec!(100000))],
            vec![],
        );
        let xml = XbrlExporter::export(&stmt);
        assert!(
            xml.contains(">ACME<"),
            "Company code must appear in entity identifier"
        );
        assert!(xml.contains("<xbrli:startDate>2025-01-01</xbrli:startDate>"));
        assert!(xml.contains("<xbrli:endDate>2025-12-31</xbrli:endDate>"));
        assert!(xml.contains("<xbrli:instant>2025-12-31</xbrli:instant>"));
    }

    #[test]
    fn test_unit_element() {
        let stmt = make_test_statement(
            StatementBasis::UsGaap,
            StatementType::BalanceSheet,
            vec![make_line_item("BS-CASH", "Cash", dec!(100000))],
            vec![],
        );
        let xml = XbrlExporter::export(&stmt);
        assert!(xml.contains("<xbrli:unit id=\"USD\">"));
        assert!(xml.contains("<xbrli:measure>iso4217:USD</xbrli:measure>"));
    }

    #[test]
    fn test_multiple_line_items() {
        let stmt = make_test_statement(
            StatementBasis::UsGaap,
            StatementType::BalanceSheet,
            vec![
                make_line_item("BS-CASH", "Cash", dec!(100000)),
                make_line_item("BS-AR", "Accounts Receivable", dec!(75000)),
                make_line_item("BS-INV", "Inventory", dec!(50000)),
                make_line_item("IS-REV", "Revenue", dec!(500000)),
                make_line_item("IS-COGS", "Cost of Goods Sold", dec!(300000)),
            ],
            vec![],
        );
        let xml = XbrlExporter::export(&stmt);
        assert!(xml.contains("us-gaap:CashAndCashEquivalentsAtCarryingValue"));
        assert!(xml.contains("us-gaap:AccountsReceivableNetCurrent"));
        assert!(xml.contains("us-gaap:InventoryNet"));
        assert!(xml.contains("us-gaap:Revenues"));
        assert!(xml.contains("us-gaap:CostOfGoodsAndServicesSold"));
    }

    #[test]
    fn test_cash_flow_items_use_duration_context() {
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
        assert!(
            xml.contains("us-gaap:NetIncomeLoss"),
            "Cash flow item with matching code must be mapped"
        );
        assert!(
            xml.contains("contextRef=\"FY2025\""),
            "Cash flow items must use duration context"
        );
    }

    #[test]
    fn test_xml_escape_in_company_code() {
        let mut stmt = make_test_statement(
            StatementBasis::UsGaap,
            StatementType::BalanceSheet,
            vec![make_line_item("BS-CASH", "Cash", dec!(100000))],
            vec![],
        );
        stmt.company_code = "A&B<Corp>".to_string();
        let xml = XbrlExporter::export(&stmt);
        assert!(
            xml.contains("A&amp;B&lt;Corp&gt;"),
            "Special XML characters in company code must be escaped"
        );
        assert!(
            !xml.contains("A&B<Corp>"),
            "Unescaped special characters must not appear"
        );
    }
}

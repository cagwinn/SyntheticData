//! Cash flow enhancer — supplementary CashFlowItem entries from v2.3 data.
//!
//! The existing [`FinancialStatementGenerator`] produces a basic indirect-method
//! cash flow statement.  This module generates **additional** items (from
//! manufacturing, treasury, tax, and dividend modules) that Task 10 merges into
//! the financial statement's `cash_flow_items` vector.

use datasynth_core::models::{CashFlowCategory, CashFlowItem};
use rust_decimal::Decimal;

/// Source data collected from upstream v2.3 generators.
#[derive(Debug, Clone)]
pub struct CashFlowSourceData {
    /// Total depreciation / amortisation for the period (from depreciation runs).
    pub depreciation_total: Decimal,
    /// Net movement in provisions for the period (from provision generator).
    pub provision_movements_net: Decimal,
    /// Change in accounts-receivable balance (positive = AR increased).
    pub delta_ar: Decimal,
    /// Change in accounts-payable balance (positive = AP increased).
    pub delta_ap: Decimal,
    /// Change in inventory balance (positive = inventory increased).
    pub delta_inventory: Decimal,
    /// Capital expenditure paid in the period (non-negative).
    pub capex: Decimal,
    /// Proceeds from new debt issued in the period.
    pub debt_issuance: Decimal,
    /// Principal repaid on debt in the period (non-negative).
    pub debt_repayment: Decimal,
    /// Interest paid in cash during the period (non-negative).
    pub interest_paid: Decimal,
    /// Income tax paid in cash during the period (non-negative).
    pub tax_paid: Decimal,
    /// Dividends paid to shareholders in the period (non-negative).
    pub dividends_paid: Decimal,
    /// Accounting framework: `"US_GAAP"` or `"IFRS"`.
    ///
    /// Under US GAAP, interest paid is an Operating cash flow.
    /// Under IFRS, interest paid is a Financing cash flow.
    pub framework: String,
}

/// Generates supplementary cash flow items from v2.3 source data.
pub struct CashFlowEnhancer;

impl CashFlowEnhancer {
    /// Build the supplementary [`CashFlowItem`] list.
    ///
    /// Items whose computed amount is zero are silently omitted so that the
    /// merged statement is not cluttered with empty lines.
    pub fn generate(data: &CashFlowSourceData) -> Vec<CashFlowItem> {
        let is_ifrs = data.framework.eq_ignore_ascii_case("IFRS");
        let mut items: Vec<CashFlowItem> = Vec::new();
        let mut sort = 100u32; // start above the values the FS generator uses

        // ------------------------------------------------------------------
        // Operating activities — adjustments to net income
        // ------------------------------------------------------------------

        // Depreciation add-back (non-cash charge reversed back)
        if !data.depreciation_total.is_zero() {
            items.push(CashFlowItem {
                item_code: "CF-DEP".to_string(),
                label: "Depreciation and Amortisation".to_string(),
                category: CashFlowCategory::Operating,
                amount: data.depreciation_total,
                amount_prior: None,
                sort_order: sort,
                is_total: false,
            });
            sort += 10;
        }

        // Provision movements (e.g., warranties, bad-debt allowances)
        if !data.provision_movements_net.is_zero() {
            items.push(CashFlowItem {
                item_code: "CF-PROV".to_string(),
                label: "Movement in Provisions".to_string(),
                category: CashFlowCategory::Operating,
                amount: data.provision_movements_net,
                amount_prior: None,
                sort_order: sort,
                is_total: false,
            });
            sort += 10;
        }

        // ΔAR: increase in AR → cash has been tied up → outflow (negative)
        let dar = -data.delta_ar;
        if !dar.is_zero() {
            items.push(CashFlowItem {
                item_code: "CF-DAR".to_string(),
                label: "Change in Trade Receivables".to_string(),
                category: CashFlowCategory::Operating,
                amount: dar,
                amount_prior: None,
                sort_order: sort,
                is_total: false,
            });
            sort += 10;
        }

        // ΔAP: increase in AP → supplier financing received → inflow (positive)
        if !data.delta_ap.is_zero() {
            items.push(CashFlowItem {
                item_code: "CF-DAP".to_string(),
                label: "Change in Trade Payables".to_string(),
                category: CashFlowCategory::Operating,
                amount: data.delta_ap,
                amount_prior: None,
                sort_order: sort,
                is_total: false,
            });
            sort += 10;
        }

        // ΔInventory: increase in inventory → cash consumed → outflow (negative)
        let dinv = -data.delta_inventory;
        if !dinv.is_zero() {
            items.push(CashFlowItem {
                item_code: "CF-DINV".to_string(),
                label: "Change in Inventories".to_string(),
                category: CashFlowCategory::Operating,
                amount: dinv,
                amount_prior: None,
                sort_order: sort,
                is_total: false,
            });
            sort += 10;
        }

        // Interest paid — Operating under US GAAP only
        if !is_ifrs && !data.interest_paid.is_zero() {
            items.push(CashFlowItem {
                item_code: "CF-INT".to_string(),
                label: "Interest Paid".to_string(),
                category: CashFlowCategory::Operating,
                amount: -data.interest_paid.abs(),
                amount_prior: None,
                sort_order: sort,
                is_total: false,
            });
            sort += 10;
        }

        // Tax paid (always Operating under both frameworks)
        if !data.tax_paid.is_zero() {
            items.push(CashFlowItem {
                item_code: "CF-TAX".to_string(),
                label: "Income Tax Paid".to_string(),
                category: CashFlowCategory::Operating,
                amount: -data.tax_paid.abs(),
                amount_prior: None,
                sort_order: sort,
                is_total: false,
            });
            sort += 10;
        }

        // ------------------------------------------------------------------
        // Investing activities
        // ------------------------------------------------------------------

        if !data.capex.is_zero() {
            items.push(CashFlowItem {
                item_code: "CF-CAPEX".to_string(),
                label: "Purchase of Property, Plant and Equipment".to_string(),
                category: CashFlowCategory::Investing,
                amount: -data.capex.abs(),
                amount_prior: None,
                sort_order: sort,
                is_total: false,
            });
            sort += 10;
        }

        // ------------------------------------------------------------------
        // Financing activities
        // ------------------------------------------------------------------

        if !data.debt_issuance.is_zero() {
            items.push(CashFlowItem {
                item_code: "CF-DEBT-IN".to_string(),
                label: "Proceeds from Borrowings".to_string(),
                category: CashFlowCategory::Financing,
                amount: data.debt_issuance,
                amount_prior: None,
                sort_order: sort,
                is_total: false,
            });
            sort += 10;
        }

        if !data.debt_repayment.is_zero() {
            items.push(CashFlowItem {
                item_code: "CF-DEBT-OUT".to_string(),
                label: "Repayment of Borrowings".to_string(),
                category: CashFlowCategory::Financing,
                amount: -data.debt_repayment.abs(),
                amount_prior: None,
                sort_order: sort,
                is_total: false,
            });
            sort += 10;
        }

        // Interest paid — Financing under IFRS only
        if is_ifrs && !data.interest_paid.is_zero() {
            items.push(CashFlowItem {
                item_code: "CF-INT-FIN".to_string(),
                label: "Interest Paid".to_string(),
                category: CashFlowCategory::Financing,
                amount: -data.interest_paid.abs(),
                amount_prior: None,
                sort_order: sort,
                is_total: false,
            });
            sort += 10;
        }

        if !data.dividends_paid.is_zero() {
            items.push(CashFlowItem {
                item_code: "CF-DIV".to_string(),
                label: "Dividends Paid".to_string(),
                category: CashFlowCategory::Financing,
                amount: -data.dividends_paid.abs(),
                amount_prior: None,
                sort_order: sort,
                is_total: false,
            });
        }

        items
    }
}

//! Consolidated income statement — Task 8.2 (stub).
//!
//! Placeholder until Task 8.2 lands.  The full implementation will
//! split net income between owners and NCI per IFRS 10.B94 / ASC
//! 810-10-45-15.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::aggregate::nci::NciRollforward;
use crate::aggregate::pre_elim::AggregatedTb;
use crate::errors::GroupResult;

#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsolidatedIncomeStatement {
    pub group_id: String,
    pub period_end: NaiveDate,
    pub currency: String,
    pub revenue: Vec<IsLine>,
    pub cost_of_goods_sold: Vec<IsLine>,
    pub gross_profit: Decimal,
    pub operating_expenses: Vec<IsLine>,
    pub operating_income: Decimal,
    pub other_income_expense: Vec<IsLine>,
    pub net_income_before_tax: Decimal,
    pub tax_expense: Decimal,
    pub net_income: Decimal,
    pub net_income_to_owners: Decimal,
    pub net_income_to_nci: Decimal,
}

#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsLine {
    pub account_code: String,
    pub account_name: String,
    pub amount: Decimal,
}

/// Build a consolidated income statement (placeholder until Task 8.2).
#[allow(unused_variables)]
pub fn build_consolidated_income_statement(
    post_elim_tb: &AggregatedTb,
    nci_rollforwards: &[NciRollforward],
    group_id: &str,
    period_end: NaiveDate,
) -> GroupResult<ConsolidatedIncomeStatement> {
    Ok(ConsolidatedIncomeStatement {
        group_id: group_id.to_string(),
        period_end,
        currency: post_elim_tb.currency.clone(),
        revenue: Vec::new(),
        cost_of_goods_sold: Vec::new(),
        gross_profit: Decimal::ZERO,
        operating_expenses: Vec::new(),
        operating_income: Decimal::ZERO,
        other_income_expense: Vec::new(),
        net_income_before_tax: Decimal::ZERO,
        tax_expense: Decimal::ZERO,
        net_income: Decimal::ZERO,
        net_income_to_owners: Decimal::ZERO,
        net_income_to_nci: Decimal::ZERO,
    })
}

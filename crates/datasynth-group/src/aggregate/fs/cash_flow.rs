//! Consolidated cash flow statement — Task 8.3 (stub).
//!
//! Placeholder until Task 8.3 lands.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::aggregate::pre_elim::AggregatedTb;
use crate::errors::GroupResult;

#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsolidatedCashFlow {
    pub group_id: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub currency: String,
    pub operating: CfSection,
    pub investing: CfSection,
    pub financing: CfSection,
    pub opening_cash: Decimal,
    pub closing_cash: Decimal,
    pub net_change_in_cash: Decimal,
    pub fx_effect_on_cash: Decimal,
}

#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CfSection {
    pub lines: Vec<CfLine>,
    pub subtotal: Decimal,
}

#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CfLine {
    pub label: String,
    pub amount: Decimal,
}

#[allow(missing_docs)]
pub struct CashFlowInputs<'a> {
    pub post_elim_tb_current: &'a AggregatedTb,
    pub post_elim_tb_prior: Option<&'a AggregatedTb>,
    pub net_income: Decimal,
    pub depreciation_amortization: Decimal,
    pub impairment: Decimal,
    pub capex: Decimal,
    pub debt_issuance: Decimal,
    pub debt_repayment: Decimal,
    pub dividends_paid_to_owners: Decimal,
    pub dividends_paid_to_nci: Decimal,
    pub equity_issuance: Decimal,
}

/// Build a consolidated cash flow (placeholder until Task 8.3).
#[allow(unused_variables)]
pub fn build_consolidated_cash_flow(
    inputs: &CashFlowInputs,
    group_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> GroupResult<ConsolidatedCashFlow> {
    Ok(ConsolidatedCashFlow {
        group_id: group_id.to_string(),
        period_start,
        period_end,
        currency: inputs.post_elim_tb_current.currency.clone(),
        operating: CfSection {
            lines: Vec::new(),
            subtotal: Decimal::ZERO,
        },
        investing: CfSection {
            lines: Vec::new(),
            subtotal: Decimal::ZERO,
        },
        financing: CfSection {
            lines: Vec::new(),
            subtotal: Decimal::ZERO,
        },
        opening_cash: Decimal::ZERO,
        closing_cash: Decimal::ZERO,
        net_change_in_cash: Decimal::ZERO,
        fx_effect_on_cash: Decimal::ZERO,
    })
}

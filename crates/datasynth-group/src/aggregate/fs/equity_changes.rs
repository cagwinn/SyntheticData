//! Statement of changes in equity — Task 8.4 (stub).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatementOfChangesInEquity {
    pub group_id: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub currency: String,
    pub owners_equity: EquityRollforward,
    pub nci: EquityRollforward,
    pub total_equity: EquityRollforward,
}

#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EquityRollforward {
    pub opening: Decimal,
    pub net_income: Decimal,
    pub oci: Decimal,
    pub dividends: Decimal,
    pub other: Decimal,
    pub closing: Decimal,
}

#[allow(missing_docs)]
pub struct EquityChangesInputs {
    pub opening_owners_equity: Decimal,
    pub opening_nci: Decimal,
    pub net_income_to_owners: Decimal,
    pub net_income_to_nci: Decimal,
    pub oci_to_owners: Decimal,
    pub oci_to_nci: Decimal,
    pub dividends_to_owners: Decimal,
    pub dividends_to_nci: Decimal,
    pub other_owners: Decimal,
    pub other_nci: Decimal,
}

/// Build a statement of changes in equity (placeholder until Task 8.4).
#[allow(unused_variables)]
pub fn build_statement_of_changes_in_equity(
    inputs: &EquityChangesInputs,
    group_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
    currency: &str,
) -> StatementOfChangesInEquity {
    let zero = EquityRollforward {
        opening: Decimal::ZERO,
        net_income: Decimal::ZERO,
        oci: Decimal::ZERO,
        dividends: Decimal::ZERO,
        other: Decimal::ZERO,
        closing: Decimal::ZERO,
    };
    StatementOfChangesInEquity {
        group_id: group_id.to_string(),
        period_start,
        period_end,
        currency: currency.to_string(),
        owners_equity: zero.clone(),
        nci: zero.clone(),
        total_equity: zero,
    }
}

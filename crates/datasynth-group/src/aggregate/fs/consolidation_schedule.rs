//! Consolidation schedule — Task 8.5 (stub).

use std::collections::BTreeMap;

use chrono::NaiveDate;
use datasynth_core::models::balance::TrialBalance;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::aggregate::pre_elim::AggregatedTb;
use crate::errors::GroupResult;

#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationSchedule {
    pub group_id: String,
    pub as_of_date: NaiveDate,
    pub currency: String,
    pub lines: Vec<ScheduleLine>,
}

#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleLine {
    pub account_category: String,
    pub account_code: String,
    pub entity_amounts: BTreeMap<String, Decimal>,
    pub pre_elimination_total: Decimal,
    pub elimination_adjustments: Decimal,
    pub post_elimination_total: Decimal,
}

/// Build a consolidation schedule (placeholder until Task 8.5).
#[allow(unused_variables)]
pub fn build_consolidation_schedule(
    pre_elim_tb: &AggregatedTb,
    post_elim_tb: &AggregatedTb,
    entity_tbs: &[(String, TrialBalance)],
    group_id: &str,
    as_of_date: NaiveDate,
) -> GroupResult<ConsolidationSchedule> {
    Ok(ConsolidationSchedule {
        group_id: group_id.to_string(),
        as_of_date,
        currency: pre_elim_tb.currency.clone(),
        lines: Vec::new(),
    })
}

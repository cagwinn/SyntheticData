//! Statement of changes in equity — Task 8.4.
//!
//! Implements the IAS 1 / ASC 220 statement of changes in equity.  It
//! reconciles opening to closing equity for two columns — owners of
//! the parent and non-controlling interest — plus a derived total
//! column.
//!
//! # Identity
//!
//! For each rollforward column:
//!
//! ```text
//! closing = opening + net_income + oci - dividends + other
//! ```
//!
//! `oci` includes the period CTA (IAS 21.39) for translation
//! differences and any cash-flow / fair-value hedge reserves.
//! `other` covers share issuance, treasury repurchases, and other
//! direct-to-equity adjustments.
//!
//! The total column is the per-line sum of the owners-equity and
//! NCI columns — a defensive postcondition the test suite verifies.
//!
//! # v5.1 deferrals
//!
//! - Per-component breakdown of OCI (CTA, hedge reserve, fair-value
//!   reserve, etc.) — currently rolled into a single `oci` line.
//! - Per-class share-issuance breakdown — currently rolled into
//!   `other`.
//! - Reclassifications between equity components.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// ── Public types ──────────────────────────────────────────────────────────────

/// Statement of changes in equity (IAS 1 / ASC 220).
///
/// Two rollforward columns (owners + NCI) plus a derived total
/// column.  Each column reconciles opening → closing per the
/// identity in the module rustdoc.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatementOfChangesInEquity {
    /// Group identifier.
    pub group_id: String,
    /// First day of the period.
    pub period_start: NaiveDate,
    /// Last day of the period.
    pub period_end: NaiveDate,
    /// Presentation currency.
    pub currency: String,
    /// Equity attributable to owners of the parent.
    pub owners_equity: EquityRollforward,
    /// Non-controlling interest equity (separately presented per
    /// IFRS 10.22 / ASC 810-10-45-15).
    pub nci: EquityRollforward,
    /// Per-line sum of `owners_equity` and `nci`.
    pub total_equity: EquityRollforward,
}

/// One column's rollforward.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EquityRollforward {
    /// Opening balance brought forward from the prior period.
    pub opening: Decimal,
    /// Period net income attributable to this column (owners or NCI).
    pub net_income: Decimal,
    /// Period OCI attributable to this column (includes CTA).
    pub oci: Decimal,
    /// Dividends paid out of this column (always supplied positive;
    /// the rollforward subtracts).
    pub dividends: Decimal,
    /// Other direct-to-equity adjustments — share issuance, treasury
    /// repurchases, contributions / distributions of capital.
    pub other: Decimal,
    /// `opening + net_income + oci - dividends + other`.
    pub closing: Decimal,
}

/// Inputs for [`build_statement_of_changes_in_equity`].
///
/// All amounts denominated in the group presentation currency
/// (translation per Chunk 6 must already have been applied).
pub struct EquityChangesInputs {
    /// Opening equity attributable to owners of the parent.
    pub opening_owners_equity: Decimal,
    /// Opening NCI equity.
    pub opening_nci: Decimal,
    /// Period net income attributable to owners.
    pub net_income_to_owners: Decimal,
    /// Period net income attributable to NCI (sum of
    /// `nci_share_of_profit` from the NCI rollforwards).
    pub net_income_to_nci: Decimal,
    /// Period OCI attributable to owners.
    pub oci_to_owners: Decimal,
    /// Period OCI attributable to NCI.
    pub oci_to_nci: Decimal,
    /// Dividends paid to owners (always supplied positive).
    pub dividends_to_owners: Decimal,
    /// Dividends paid to NCI (always supplied positive).
    pub dividends_to_nci: Decimal,
    /// Other direct-to-equity adjustments to owners' equity.
    pub other_owners: Decimal,
    /// Other direct-to-equity adjustments to NCI.
    pub other_nci: Decimal,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Build the statement of changes in equity.
///
/// Pure function: no I/O, no validation — every input is consumed
/// verbatim and the rollforward identity is applied per column.
/// Two calls with the same inputs produce equal records.
pub fn build_statement_of_changes_in_equity(
    inputs: &EquityChangesInputs,
    group_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
    currency: &str,
) -> StatementOfChangesInEquity {
    let owners = rollforward(
        inputs.opening_owners_equity,
        inputs.net_income_to_owners,
        inputs.oci_to_owners,
        inputs.dividends_to_owners,
        inputs.other_owners,
    );
    let nci = rollforward(
        inputs.opening_nci,
        inputs.net_income_to_nci,
        inputs.oci_to_nci,
        inputs.dividends_to_nci,
        inputs.other_nci,
    );
    let total = EquityRollforward {
        opening: owners.opening + nci.opening,
        net_income: owners.net_income + nci.net_income,
        oci: owners.oci + nci.oci,
        dividends: owners.dividends + nci.dividends,
        other: owners.other + nci.other,
        closing: owners.closing + nci.closing,
    };
    StatementOfChangesInEquity {
        group_id: group_id.to_string(),
        period_start,
        period_end,
        currency: currency.to_string(),
        owners_equity: owners,
        nci,
        total_equity: total,
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn rollforward(
    opening: Decimal,
    net_income: Decimal,
    oci: Decimal,
    dividends: Decimal,
    other: Decimal,
) -> EquityRollforward {
    let closing = opening + net_income + oci - dividends + other;
    EquityRollforward {
        opening,
        net_income,
        oci,
        dividends,
        other,
        closing,
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn rollforward_identity() {
        let r = rollforward(dec!(100), dec!(50), dec!(10), dec!(5), dec!(2));
        // 100 + 50 + 10 - 5 + 2 = 157.
        assert_eq!(r.closing, dec!(157));
    }
}

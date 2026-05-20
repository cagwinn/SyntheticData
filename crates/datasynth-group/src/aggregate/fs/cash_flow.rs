//! Consolidated cash flow statement (indirect method) — Task 8.3.
//!
//! Implements the IAS 7 / ASC 230 indirect-method statement of cash
//! flows.  Operating activities start from net income and adjust for
//! non-cash items (depreciation, amortization, impairment) plus
//! working-capital changes (ΔAR, ΔAP, Δinventory).  Investing covers
//! capital expenditure (and disposals — deferred to v5.1).  Financing
//! covers debt issuance / repayment, dividends, and equity issuance.
//!
//! # Standards reference
//!
//! - **IAS 7** *Statement of Cash Flows* — both direct and indirect
//!   methods are permitted; we emit the indirect method.
//! - **ASC 230** — same indirect-method shape under US GAAP.
//!
//! # FX-effect plug
//!
//! Cash held in foreign currency retranslates each period at the
//! closing rate.  The retranslation gain / loss does not flow through
//! the operating / investing / financing sections — IAS 7.28 requires
//! it to be presented as a separate "effect of exchange rate changes
//! on cash" line.  We compute this as a residual:
//!
//! ```text
//! fx_effect = closing_cash - opening_cash
//!           - (operating + investing + financing)
//! ```
//!
//! For a single-currency engagement (Mini-Acme, every entity in
//! CHF) the residual is zero modulo rounding.  Multi-currency
//! engagements pick up the IAS 21 retranslation difference here.
//!
//! # Working-capital changes
//!
//! When `post_elim_tb_prior` is supplied, ΔAR / ΔAP / Δinventory are
//! computed from the change in `1100` (trade receivables), `2000`
//! (trade payables), and `1200` (inventory) between the two TBs.  An
//! *increase* in AR is a *use* of cash (negative), an *increase* in
//! AP is a *source* of cash (positive).  When prior is `None` the
//! engagement is in its first period and working-capital changes are
//! treated as zero (opening balances absorb the change as cash flows
//! cumulatively).
//!
//! # v5.1 deferrals
//!
//! - Disposal proceeds of PP&E (investing inflow).
//! - Acquisitions / disposals of subsidiaries (separate IAS 7.39
//!   line).
//! - Interest paid / received split-out (IAS 7.31 — currently rolled
//!   into operating).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::aggregate::pre_elim::AggregatedTb;
use crate::errors::GroupResult;

// ── Public types ──────────────────────────────────────────────────────────────

/// Consolidated cash flow statement (IAS 7 indirect method).
///
/// Sections balance internally — each `subtotal` equals the sum of
/// its `lines` — and the FX-effect line absorbs any retranslation
/// residual.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsolidatedCashFlow {
    /// Group identifier.
    pub group_id: String,
    /// Start of the period (inclusive).
    pub period_start: NaiveDate,
    /// End of the period (inclusive).
    pub period_end: NaiveDate,
    /// Presentation currency.
    pub currency: String,
    /// Operating activities (indirect method: net income + non-cash
    /// adjustments + working-capital changes).
    pub operating: CfSection,
    /// Investing activities (capex, disposals).
    pub investing: CfSection,
    /// Financing activities (debt, dividends, equity).
    pub financing: CfSection,
    /// Cash and equivalents at start of period.
    pub opening_cash: Decimal,
    /// Cash and equivalents at end of period.
    pub closing_cash: Decimal,
    /// `operating + investing + financing` subtotals.
    pub net_change_in_cash: Decimal,
    /// IAS 7.28 retranslation residual.
    pub fx_effect_on_cash: Decimal,
}

/// One section (operating / investing / financing) of the cash flow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CfSection {
    /// Individual line items in the section.
    pub lines: Vec<CfLine>,
    /// `sum(lines.amount)` — recomputed at construction.
    pub subtotal: Decimal,
}

/// One line within a [`CfSection`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CfLine {
    /// Human-readable label (no GL account — cash flow lines are
    /// derived, not direct ledger postings).
    pub label: String,
    /// Signed amount: positive = source of cash, negative = use of cash.
    pub amount: Decimal,
}

/// Inputs required to derive a [`ConsolidatedCashFlow`].
///
/// The caller supplies the cash-flow-specific aggregates (capex,
/// debt issuance, dividends, etc.) directly — v5.0 does not derive
/// them from journal entries.  v5.1 will wire these to the
/// orchestrator's per-shard subledger aggregates.
pub struct CashFlowInputs<'a> {
    /// Post-elimination consolidated TB at period end.  Used for
    /// closing cash and current working-capital balances.
    pub post_elim_tb_current: &'a AggregatedTb,
    /// Post-elimination consolidated TB at period start.  When
    /// `Some`, working-capital changes and opening cash are derived
    /// from it.  When `None`, working-capital changes are zero and
    /// opening cash is zero (first-period engagement).
    pub post_elim_tb_prior: Option<&'a AggregatedTb>,
    /// Net income for the period (consolidated, attributable to
    /// owners + NCI).  First line of the operating section.
    pub net_income: Decimal,
    /// Depreciation + amortization expense (non-cash add-back).
    pub depreciation_amortization: Decimal,
    /// Impairment loss (non-cash add-back).
    pub impairment: Decimal,
    /// Capital expenditure (investing outflow — supplied as a
    /// positive number; the section line is negative).
    pub capex: Decimal,
    /// Debt issuance (financing inflow).
    pub debt_issuance: Decimal,
    /// Debt repayment (financing outflow — supplied positive).
    pub debt_repayment: Decimal,
    /// Dividends paid to owners of the parent (financing outflow —
    /// supplied positive).
    pub dividends_paid_to_owners: Decimal,
    /// Dividends paid to non-controlling shareholders (financing
    /// outflow — supplied positive).
    pub dividends_paid_to_nci: Decimal,
    /// Equity issuance (financing inflow).
    pub equity_issuance: Decimal,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Build the consolidated cash flow statement.
///
/// Pure function: no I/O, no allocation beyond the result, no
/// dependence on global state.  Two calls with the same inputs
/// produce equal records.
pub fn build_consolidated_cash_flow(
    inputs: &CashFlowInputs,
    group_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> GroupResult<ConsolidatedCashFlow> {
    // ── Operating ──────────────────────────────────────────────────────────
    let mut operating_lines: Vec<CfLine> = Vec::new();
    operating_lines.push(CfLine {
        label: "Net income".to_string(),
        amount: inputs.net_income,
    });
    if inputs.depreciation_amortization != Decimal::ZERO {
        operating_lines.push(CfLine {
            label: "Depreciation and amortization".to_string(),
            amount: inputs.depreciation_amortization,
        });
    }
    if inputs.impairment != Decimal::ZERO {
        operating_lines.push(CfLine {
            label: "Impairment".to_string(),
            amount: inputs.impairment,
        });
    }

    // Working-capital changes from prior TB (if supplied).
    let (delta_ar, delta_ap, delta_inventory) =
        working_capital_changes(inputs.post_elim_tb_current, inputs.post_elim_tb_prior);
    // ΔAR: an increase in AR is a *use* of cash → subtract.
    if delta_ar != Decimal::ZERO {
        operating_lines.push(CfLine {
            label: "Change in trade receivables".to_string(),
            amount: -delta_ar,
        });
    }
    // ΔInventory: an increase is a *use* of cash → subtract.
    if delta_inventory != Decimal::ZERO {
        operating_lines.push(CfLine {
            label: "Change in inventory".to_string(),
            amount: -delta_inventory,
        });
    }
    // ΔAP: an increase is a *source* of cash → add.
    if delta_ap != Decimal::ZERO {
        operating_lines.push(CfLine {
            label: "Change in trade payables".to_string(),
            amount: delta_ap,
        });
    }
    let operating_subtotal = sum_lines(&operating_lines);

    // ── Investing ──────────────────────────────────────────────────────────
    let mut investing_lines: Vec<CfLine> = Vec::new();
    if inputs.capex != Decimal::ZERO {
        investing_lines.push(CfLine {
            label: "Capital expenditure".to_string(),
            amount: -inputs.capex,
        });
    }
    let investing_subtotal = sum_lines(&investing_lines);

    // ── Financing ──────────────────────────────────────────────────────────
    let mut financing_lines: Vec<CfLine> = Vec::new();
    if inputs.debt_issuance != Decimal::ZERO {
        financing_lines.push(CfLine {
            label: "Debt issuance".to_string(),
            amount: inputs.debt_issuance,
        });
    }
    if inputs.debt_repayment != Decimal::ZERO {
        financing_lines.push(CfLine {
            label: "Debt repayment".to_string(),
            amount: -inputs.debt_repayment,
        });
    }
    if inputs.equity_issuance != Decimal::ZERO {
        financing_lines.push(CfLine {
            label: "Equity issuance".to_string(),
            amount: inputs.equity_issuance,
        });
    }
    if inputs.dividends_paid_to_owners != Decimal::ZERO {
        financing_lines.push(CfLine {
            label: "Dividends paid to owners".to_string(),
            amount: -inputs.dividends_paid_to_owners,
        });
    }
    if inputs.dividends_paid_to_nci != Decimal::ZERO {
        financing_lines.push(CfLine {
            label: "Dividends paid to non-controlling interest".to_string(),
            amount: -inputs.dividends_paid_to_nci,
        });
    }
    let financing_subtotal = sum_lines(&financing_lines);

    // ── Cash bridge ────────────────────────────────────────────────────────
    let opening_cash = match inputs.post_elim_tb_prior {
        Some(prior) => sum_cash(prior),
        None => Decimal::ZERO,
    };
    let closing_cash = sum_cash(inputs.post_elim_tb_current);
    let net_change_in_cash = operating_subtotal + investing_subtotal + financing_subtotal;
    // FX-effect plug per IAS 7.28.
    let fx_effect_on_cash = closing_cash - opening_cash - net_change_in_cash;

    Ok(ConsolidatedCashFlow {
        group_id: group_id.to_string(),
        period_start,
        period_end,
        currency: inputs.post_elim_tb_current.currency.clone(),
        operating: CfSection {
            lines: operating_lines,
            subtotal: operating_subtotal,
        },
        investing: CfSection {
            lines: investing_lines,
            subtotal: investing_subtotal,
        },
        financing: CfSection {
            lines: financing_lines,
            subtotal: financing_subtotal,
        },
        opening_cash,
        closing_cash,
        net_change_in_cash,
        fx_effect_on_cash,
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Sum the `amount` field across every line in a [`CfSection`].
fn sum_lines(lines: &[CfLine]) -> Decimal {
    lines
        .iter()
        .map(|l| l.amount)
        .fold(Decimal::ZERO, |acc, v| acc + v)
}

/// Sum cash account balances (1000–1099) on a [`AggregatedTb`].  Cash
/// is debit-natural so the balance is `debit_total - credit_total`.
fn sum_cash(tb: &AggregatedTb) -> Decimal {
    tb.account_totals
        .iter()
        .filter(|(code, _)| is_cash(code))
        .map(|(_, a)| a.debit_total - a.credit_total)
        .fold(Decimal::ZERO, |acc, v| acc + v)
}

fn is_cash(code: &str) -> bool {
    let n: u32 = match code.parse() {
        Ok(n) => n,
        Err(_) => return false,
    };
    (1000..=1099).contains(&n)
}

/// Compute working-capital deltas from prior → current.
///
/// Returns `(ΔAR, ΔAP, ΔInventory)`.  When `prior` is `None`, all
/// three deltas are zero.
fn working_capital_changes(
    current: &AggregatedTb,
    prior: Option<&AggregatedTb>,
) -> (Decimal, Decimal, Decimal) {
    let prior = match prior {
        Some(p) => p,
        None => return (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO),
    };

    let cur_ar = sum_natural_balance(current, "1100");
    let prior_ar = sum_natural_balance(prior, "1100");

    let cur_ap = sum_natural_balance_credit(current, "2000");
    let prior_ap = sum_natural_balance_credit(prior, "2000");

    let cur_inv = sum_natural_balance(current, "1200");
    let prior_inv = sum_natural_balance(prior, "1200");

    (cur_ar - prior_ar, cur_ap - prior_ap, cur_inv - prior_inv)
}

fn sum_natural_balance(tb: &AggregatedTb, code: &str) -> Decimal {
    tb.account_totals
        .get(code)
        .map(|a| a.debit_total - a.credit_total)
        .unwrap_or(Decimal::ZERO)
}

fn sum_natural_balance_credit(tb: &AggregatedTb, code: &str) -> Decimal {
    tb.account_totals
        .get(code)
        .map(|a| a.credit_total - a.debit_total)
        .unwrap_or(Decimal::ZERO)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn is_cash_classifier() {
        assert!(is_cash("1000"));
        assert!(is_cash("1099"));
        assert!(!is_cash("1100"));
        assert!(!is_cash("999"));
        assert!(!is_cash("abc"));
        assert!(!is_cash(""));
    }

    #[test]
    fn sum_lines_zero_when_empty() {
        assert_eq!(sum_lines(&[]), Decimal::ZERO);
    }

    #[test]
    fn sum_lines_signs_preserved() {
        let lines = vec![
            CfLine {
                label: "a".to_string(),
                amount: dec!(100),
            },
            CfLine {
                label: "b".to_string(),
                amount: dec!(-30),
            },
        ];
        assert_eq!(sum_lines(&lines), dec!(70));
    }
}

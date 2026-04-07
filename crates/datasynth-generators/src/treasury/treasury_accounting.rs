//! Treasury accounting journal entry pipeline.
//!
//! Generates journal entries from treasury instruments:
//! - Debt interest accruals (quarterly approximation)
//! - Hedge fair-value / cash-flow accounting with ineffectiveness
//! - Cash pool sweep intercompany entries

use chrono::NaiveDate;
use datasynth_core::accounts::{expense_accounts, treasury_accounts};
use datasynth_core::models::{
    BusinessProcess, CashPoolSweep, DebtInstrument, HedgeRelationship, HedgeType,
    HedgingInstrument, JournalEntry, JournalEntryLine, TransactionSource,
};
use rust_decimal::Decimal;

/// Generates journal entries for treasury instruments.
///
/// Follows the same static-method pattern as `ManufacturingCostAccounting`:
/// a zero-sized struct with associated functions that accept domain objects
/// and return balanced `Vec<JournalEntry>`.
pub struct TreasuryAccounting;

impl TreasuryAccounting {
    // ------------------------------------------------------------------
    // 1. Debt interest accruals
    // ------------------------------------------------------------------

    /// Generate interest accrual JEs for active debt instruments.
    ///
    /// For each instrument where `period_end <= maturity_date`, posts:
    ///   DR Interest Expense ("7100")
    ///   CR Interest Payable ("2160")
    ///
    /// Amount = principal * (annual_interest_rate / 4) — quarterly
    /// approximation.
    pub fn generate_debt_jes(
        instruments: &[DebtInstrument],
        period_end: NaiveDate,
    ) -> Vec<JournalEntry> {
        let mut jes = Vec::new();

        for debt in instruments {
            // Skip matured debt — no further interest accrues.
            if period_end > debt.maturity_date {
                continue;
            }

            let quarterly_interest =
                (debt.principal * debt.interest_rate / Decimal::from(4)).round_dp(2);

            if quarterly_interest == Decimal::ZERO {
                continue;
            }

            let mut je = JournalEntry::new_simple(
                format!("JE-TREAS-INT-{}", debt.id),
                debt.entity_id.clone(),
                period_end,
                format!("Interest accrual on {} from {}", debt.id, debt.lender),
            );
            je.header.currency = debt.currency.clone();
            je.header.business_process = Some(BusinessProcess::Treasury);
            je.header.source = TransactionSource::Automated;
            let doc_id = je.header.document_id;

            je.add_line(JournalEntryLine {
                document_id: doc_id,
                line_number: 1,
                gl_account: expense_accounts::INTEREST_EXPENSE.to_string(),
                account_code: expense_accounts::INTEREST_EXPENSE.to_string(),
                debit_amount: quarterly_interest,
                local_amount: quarterly_interest,
                reference: Some(debt.id.clone()),
                text: Some(format!("Interest expense — {}", debt.lender)),
                ..Default::default()
            });
            je.add_line(JournalEntryLine {
                document_id: doc_id,
                line_number: 2,
                gl_account: treasury_accounts::INTEREST_PAYABLE.to_string(),
                account_code: treasury_accounts::INTEREST_PAYABLE.to_string(),
                credit_amount: quarterly_interest,
                local_amount: -quarterly_interest,
                reference: Some(debt.id.clone()),
                text: Some(format!("Interest payable — {}", debt.lender)),
                ..Default::default()
            });

            jes.push(je);
        }

        jes
    }

    // ------------------------------------------------------------------
    // 2. Hedge accounting JEs
    // ------------------------------------------------------------------

    /// Generate hedge accounting JEs for hedging instruments.
    ///
    /// For each active instrument with non-zero fair value:
    /// - Cash flow hedge: DR/CR Derivative Asset/Liability vs OCI ("3510")
    /// - Fair value hedge: DR/CR Derivative Asset/Liability vs FX Gain/Loss ("7500")
    ///
    /// If the hedge relationship is ineffective *and* `ineffectiveness_amount > 0`,
    /// an additional JE recognises ineffectiveness in P&L.
    pub fn generate_hedge_jes(
        instruments: &[HedgingInstrument],
        relationships: &[HedgeRelationship],
        period_end: NaiveDate,
        entity_id: &str,
    ) -> Vec<JournalEntry> {
        let mut jes = Vec::new();

        for instrument in instruments {
            if !instrument.is_active() {
                continue;
            }
            if instrument.fair_value == Decimal::ZERO {
                continue;
            }

            // Find matching hedge relationship.
            let relationship = relationships
                .iter()
                .find(|r| r.hedging_instrument_id == instrument.id);

            let hedge_type = relationship
                .map(|r| r.hedge_type)
                .unwrap_or(HedgeType::FairValueHedge);

            let abs_fv = instrument.fair_value.abs();
            let is_asset = instrument.fair_value > Decimal::ZERO;

            // Determine the P&L / OCI account based on hedge type.
            let pnl_oci_account = match hedge_type {
                HedgeType::CashFlowHedge | HedgeType::NetInvestmentHedge => {
                    treasury_accounts::OCI_CASH_FLOW_HEDGE
                }
                HedgeType::FairValueHedge => expense_accounts::FX_GAIN_LOSS,
            };

            let mut je = JournalEntry::new_simple(
                format!("JE-TREAS-HEDGE-{}", instrument.id),
                entity_id.to_string(),
                period_end,
                format!(
                    "Hedge accounting — {} ({})",
                    instrument.id,
                    if is_asset { "asset" } else { "liability" }
                ),
            );
            je.header.currency = instrument.currency.clone();
            je.header.business_process = Some(BusinessProcess::Treasury);
            je.header.source = TransactionSource::Automated;
            let doc_id = je.header.document_id;

            if is_asset {
                // DR Derivative Asset, CR OCI/FX Gain-Loss
                je.add_line(JournalEntryLine {
                    document_id: doc_id,
                    line_number: 1,
                    gl_account: treasury_accounts::DERIVATIVE_ASSET.to_string(),
                    account_code: treasury_accounts::DERIVATIVE_ASSET.to_string(),
                    debit_amount: abs_fv,
                    local_amount: abs_fv,
                    reference: Some(instrument.id.clone()),
                    text: Some("Derivative asset — positive MTM".to_string()),
                    ..Default::default()
                });
                je.add_line(JournalEntryLine {
                    document_id: doc_id,
                    line_number: 2,
                    gl_account: pnl_oci_account.to_string(),
                    account_code: pnl_oci_account.to_string(),
                    credit_amount: abs_fv,
                    local_amount: -abs_fv,
                    reference: Some(instrument.id.clone()),
                    text: Some(format!(
                        "{} — hedge gain",
                        if hedge_type == HedgeType::CashFlowHedge
                            || hedge_type == HedgeType::NetInvestmentHedge
                        {
                            "OCI"
                        } else {
                            "FX Gain/Loss"
                        }
                    )),
                    ..Default::default()
                });
            } else {
                // DR OCI/FX Gain-Loss, CR Derivative Liability
                je.add_line(JournalEntryLine {
                    document_id: doc_id,
                    line_number: 1,
                    gl_account: pnl_oci_account.to_string(),
                    account_code: pnl_oci_account.to_string(),
                    debit_amount: abs_fv,
                    local_amount: abs_fv,
                    reference: Some(instrument.id.clone()),
                    text: Some(format!(
                        "{} — hedge loss",
                        if hedge_type == HedgeType::CashFlowHedge
                            || hedge_type == HedgeType::NetInvestmentHedge
                        {
                            "OCI"
                        } else {
                            "FX Gain/Loss"
                        }
                    )),
                    ..Default::default()
                });
                je.add_line(JournalEntryLine {
                    document_id: doc_id,
                    line_number: 2,
                    gl_account: treasury_accounts::DERIVATIVE_LIABILITY.to_string(),
                    account_code: treasury_accounts::DERIVATIVE_LIABILITY.to_string(),
                    credit_amount: abs_fv,
                    local_amount: -abs_fv,
                    reference: Some(instrument.id.clone()),
                    text: Some("Derivative liability — negative MTM".to_string()),
                    ..Default::default()
                });
            }

            jes.push(je);

            // Ineffectiveness JE (if applicable).
            if let Some(rel) = relationship {
                if !rel.is_effective && rel.ineffectiveness_amount > Decimal::ZERO {
                    let ineff = rel.ineffectiveness_amount.round_dp(2);

                    let mut je_ineff = JournalEntry::new_simple(
                        format!("JE-TREAS-INEFF-{}", instrument.id),
                        entity_id.to_string(),
                        period_end,
                        format!("Hedge ineffectiveness — {}", instrument.id),
                    );
                    je_ineff.header.currency = instrument.currency.clone();
                    je_ineff.header.business_process = Some(BusinessProcess::Treasury);
                    je_ineff.header.source = TransactionSource::Automated;
                    let doc_id_ineff = je_ineff.header.document_id;

                    je_ineff.add_line(JournalEntryLine {
                        document_id: doc_id_ineff,
                        line_number: 1,
                        gl_account: treasury_accounts::HEDGE_INEFFECTIVENESS.to_string(),
                        account_code: treasury_accounts::HEDGE_INEFFECTIVENESS.to_string(),
                        debit_amount: ineff,
                        local_amount: ineff,
                        reference: Some(instrument.id.clone()),
                        text: Some("Hedge ineffectiveness expense".to_string()),
                        ..Default::default()
                    });
                    je_ineff.add_line(JournalEntryLine {
                        document_id: doc_id_ineff,
                        line_number: 2,
                        gl_account: treasury_accounts::OCI_CASH_FLOW_HEDGE.to_string(),
                        account_code: treasury_accounts::OCI_CASH_FLOW_HEDGE.to_string(),
                        credit_amount: ineff,
                        local_amount: -ineff,
                        reference: Some(instrument.id.clone()),
                        text: Some("OCI reclassification — ineffectiveness".to_string()),
                        ..Default::default()
                    });

                    jes.push(je_ineff);
                }
            }
        }

        jes
    }

    // ------------------------------------------------------------------
    // 3. Cash pool sweep JEs
    // ------------------------------------------------------------------

    /// Generate intercompany JEs for cash pool sweeps.
    ///
    /// For each sweep with a non-zero amount:
    ///   DR Cash Pool IC Receivable ("1155")
    ///   CR Cash Pool IC Payable ("2055")
    pub fn generate_cash_pool_sweep_jes(
        sweeps: &[CashPoolSweep],
        header_entity: &str,
    ) -> Vec<JournalEntry> {
        let mut jes = Vec::new();

        for sweep in sweeps {
            if sweep.amount == Decimal::ZERO {
                continue;
            }

            let abs_amount = sweep.amount.abs();

            let mut je = JournalEntry::new_simple(
                format!("JE-TREAS-SWEEP-{}", sweep.id),
                header_entity.to_string(),
                sweep.date,
                format!(
                    "Cash pool sweep {} → {} (pool {})",
                    sweep.from_account_id, sweep.to_account_id, sweep.pool_id
                ),
            );
            je.header.currency = sweep.currency.clone();
            je.header.business_process = Some(BusinessProcess::Treasury);
            je.header.source = TransactionSource::Automated;
            let doc_id = je.header.document_id;

            je.add_line(JournalEntryLine {
                document_id: doc_id,
                line_number: 1,
                gl_account: treasury_accounts::CASH_POOL_IC_RECEIVABLE.to_string(),
                account_code: treasury_accounts::CASH_POOL_IC_RECEIVABLE.to_string(),
                debit_amount: abs_amount,
                local_amount: abs_amount,
                reference: Some(sweep.id.clone()),
                text: Some(format!(
                    "IC receivable — sweep from {}",
                    sweep.from_account_id
                )),
                ..Default::default()
            });
            je.add_line(JournalEntryLine {
                document_id: doc_id,
                line_number: 2,
                gl_account: treasury_accounts::CASH_POOL_IC_PAYABLE.to_string(),
                account_code: treasury_accounts::CASH_POOL_IC_PAYABLE.to_string(),
                credit_amount: abs_amount,
                local_amount: -abs_amount,
                reference: Some(sweep.id.clone()),
                text: Some(format!("IC payable — sweep to {}", sweep.to_account_id)),
                ..Default::default()
            });

            jes.push(je);
        }

        jes
    }
}

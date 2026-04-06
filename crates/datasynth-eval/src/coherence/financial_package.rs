//! Capstone financial statement package coherence validators.
//!
//! Validates the financial statements as a coherent whole, including:
//! - Cash flow statement reconciliation (opening → closing cash)
//! - Equity roll-forward (opening → closing equity via NI, OCI, dividends, stock comp)
//! - Segment revenue reconciliation (segment totals net of IC eliminations → consolidated)
//! - Trial balance master proof (opening TB + period JEs = closing TB, debits and credits)

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// ─── CashFlowReconciliationEvaluator ─────────────────────────────────────────

/// Input data for cash flow statement reconciliation.
#[derive(Debug, Clone)]
pub struct CashFlowReconciliationData {
    /// Opening cash balance at the beginning of the period.
    pub opening_cash: Decimal,
    /// Net cash from operating activities.
    pub net_operating: Decimal,
    /// Net cash from investing activities.
    pub net_investing: Decimal,
    /// Net cash from financing activities.
    pub net_financing: Decimal,
    /// Closing cash balance per the GL / balance sheet.
    pub closing_cash_gl: Decimal,
}

/// Results of cash flow statement reconciliation evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CashFlowReconciliationEvaluation {
    /// Whether the computed closing cash reconciles to the GL balance.
    pub reconciled: bool,
    /// Expected closing cash: opening + operating + investing + financing.
    pub expected_closing: Decimal,
    /// Absolute difference between expected and GL closing cash.
    pub difference: Decimal,
    /// Overall pass/fail status.
    pub passes: bool,
    /// Human-readable descriptions of failed checks.
    pub failures: Vec<String>,
}

/// Evaluator that verifies the cash flow statement reconciles end-to-end.
pub struct CashFlowReconciliationEvaluator {
    tolerance: Decimal,
}

impl CashFlowReconciliationEvaluator {
    /// Create a new evaluator with the given absolute tolerance.
    pub fn new(tolerance: Decimal) -> Self {
        Self { tolerance }
    }

    /// Run the cash flow reconciliation check against `data`.
    pub fn evaluate(&self, data: &CashFlowReconciliationData) -> CashFlowReconciliationEvaluation {
        let expected_closing =
            data.opening_cash + data.net_operating + data.net_investing + data.net_financing;
        let difference = (expected_closing - data.closing_cash_gl).abs();
        let reconciled = difference <= self.tolerance;
        let mut failures = Vec::new();
        if !reconciled {
            failures.push(format!(
                "Cash flow reconciliation failed: expected closing cash {} vs GL {} (diff {})",
                expected_closing, data.closing_cash_gl, difference
            ));
        }
        CashFlowReconciliationEvaluation {
            reconciled,
            expected_closing,
            difference,
            passes: reconciled,
            failures,
        }
    }
}

impl Default for CashFlowReconciliationEvaluator {
    fn default() -> Self {
        Self::new(Decimal::new(1, 2)) // 0.01 tolerance
    }
}

// ─── EquityRollforwardEvaluator ───────────────────────────────────────────────

/// Input data for equity roll-forward reconciliation.
#[derive(Debug, Clone)]
pub struct EquityRollforwardData {
    /// Opening total equity at the beginning of the period.
    pub opening_equity: Decimal,
    /// Net income (loss) for the period.
    pub net_income: Decimal,
    /// Other comprehensive income (OCI) movements for the period.
    pub oci_movements: Decimal,
    /// Dividends declared during the period (positive = reduction in equity).
    pub dividends_declared: Decimal,
    /// Stock-based compensation recognised during the period.
    pub stock_comp: Decimal,
    /// Closing total equity per the balance sheet.
    pub closing_equity: Decimal,
}

/// Results of equity roll-forward reconciliation evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquityRollforwardEvaluation {
    /// Whether the computed closing equity reconciles to the balance sheet.
    pub reconciled: bool,
    /// Expected closing equity: opening + NI + OCI − dividends + stock_comp.
    pub expected_closing: Decimal,
    /// Absolute difference between expected and balance sheet closing equity.
    pub difference: Decimal,
    /// Overall pass/fail status.
    pub passes: bool,
    /// Human-readable descriptions of failed checks.
    pub failures: Vec<String>,
}

/// Evaluator that verifies the statement of changes in equity rolls forward correctly.
pub struct EquityRollforwardEvaluator {
    tolerance: Decimal,
}

impl EquityRollforwardEvaluator {
    /// Create a new evaluator with the given absolute tolerance.
    pub fn new(tolerance: Decimal) -> Self {
        Self { tolerance }
    }

    /// Run the equity roll-forward check against `data`.
    pub fn evaluate(&self, data: &EquityRollforwardData) -> EquityRollforwardEvaluation {
        let expected_closing = data.opening_equity
            + data.net_income
            + data.oci_movements
            - data.dividends_declared
            + data.stock_comp;
        let difference = (expected_closing - data.closing_equity).abs();
        let reconciled = difference <= self.tolerance;
        let mut failures = Vec::new();
        if !reconciled {
            failures.push(format!(
                "Equity roll-forward reconciliation failed: expected closing equity {} vs balance sheet {} (diff {})",
                expected_closing, data.closing_equity, difference
            ));
        }
        EquityRollforwardEvaluation {
            reconciled,
            expected_closing,
            difference,
            passes: reconciled,
            failures,
        }
    }
}

impl Default for EquityRollforwardEvaluator {
    fn default() -> Self {
        Self::new(Decimal::new(1, 2)) // 0.01 tolerance
    }
}

// ─── SegmentReconciliationEvaluator ──────────────────────────────────────────

/// Input data for segment revenue reconciliation.
#[derive(Debug, Clone)]
pub struct SegmentReconciliationData {
    /// Sum of revenue reported across all operating segments.
    pub sum_segment_revenue: Decimal,
    /// Intercompany eliminations to be netted from segment totals.
    pub ic_eliminations: Decimal,
    /// Consolidated revenue per the group income statement.
    pub consolidated_revenue: Decimal,
}

/// Results of segment revenue reconciliation evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentReconciliationEvaluation {
    /// Whether the segment totals less eliminations reconcile to consolidated revenue.
    pub reconciled: bool,
    /// Expected consolidated revenue: sum_segment_revenue − ic_eliminations.
    pub expected_consolidated: Decimal,
    /// Absolute difference between expected and reported consolidated revenue.
    pub difference: Decimal,
    /// Overall pass/fail status.
    pub passes: bool,
    /// Human-readable descriptions of failed checks.
    pub failures: Vec<String>,
}

/// Evaluator that verifies segment revenues net of eliminations equal consolidated revenue.
pub struct SegmentReconciliationEvaluator {
    tolerance: Decimal,
}

impl SegmentReconciliationEvaluator {
    /// Create a new evaluator with the given absolute tolerance.
    pub fn new(tolerance: Decimal) -> Self {
        Self { tolerance }
    }

    /// Run the segment reconciliation check against `data`.
    pub fn evaluate(&self, data: &SegmentReconciliationData) -> SegmentReconciliationEvaluation {
        let expected_consolidated = data.sum_segment_revenue - data.ic_eliminations;
        let difference = (expected_consolidated - data.consolidated_revenue).abs();
        let reconciled = difference <= self.tolerance;
        let mut failures = Vec::new();
        if !reconciled {
            failures.push(format!(
                "Segment reconciliation failed: expected consolidated revenue {} vs reported {} (diff {})",
                expected_consolidated, data.consolidated_revenue, difference
            ));
        }
        SegmentReconciliationEvaluation {
            reconciled,
            expected_consolidated,
            difference,
            passes: reconciled,
            failures,
        }
    }
}

impl Default for SegmentReconciliationEvaluator {
    fn default() -> Self {
        Self::new(Decimal::new(1, 2)) // 0.01 tolerance
    }
}

// ─── TrialBalanceMasterProofEvaluator ─────────────────────────────────────────

/// Input data for the trial balance master proof.
///
/// The master proof asserts: opening TB + period JEs = closing TB, for both
/// debits and credits independently.
#[derive(Debug, Clone)]
pub struct TrialBalanceMasterProofData {
    /// Sum of all debit balances on the opening trial balance.
    pub sum_opening_debits: Decimal,
    /// Sum of all credit balances on the opening trial balance.
    pub sum_opening_credits: Decimal,
    /// Sum of all debit sides of journal entries posted during the period.
    pub sum_je_debits: Decimal,
    /// Sum of all credit sides of journal entries posted during the period.
    pub sum_je_credits: Decimal,
    /// Sum of all debit balances on the closing trial balance.
    pub closing_tb_debits: Decimal,
    /// Sum of all credit balances on the closing trial balance.
    pub closing_tb_credits: Decimal,
}

/// Results of the trial balance master proof evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrialBalanceMasterProofEvaluation {
    /// Whether the debit side of the closing TB reconciles to opening + JE debits.
    pub debits_reconciled: bool,
    /// Whether the credit side of the closing TB reconciles to opening + JE credits.
    pub credits_reconciled: bool,
    /// Absolute difference on the debit side.
    pub debit_difference: Decimal,
    /// Absolute difference on the credit side.
    pub credit_difference: Decimal,
    /// Overall pass/fail status (both sides must reconcile).
    pub passes: bool,
    /// Human-readable descriptions of failed checks.
    pub failures: Vec<String>,
}

/// Evaluator that performs the master trial balance proof — the capstone check that
/// every generator's GL output is fully accounted for.
pub struct TrialBalanceMasterProofEvaluator {
    tolerance: Decimal,
}

impl TrialBalanceMasterProofEvaluator {
    /// Create a new evaluator with the given absolute tolerance.
    pub fn new(tolerance: Decimal) -> Self {
        Self { tolerance }
    }

    /// Run the trial balance master proof against `data`.
    pub fn evaluate(
        &self,
        data: &TrialBalanceMasterProofData,
    ) -> TrialBalanceMasterProofEvaluation {
        let expected_closing_debits = data.sum_opening_debits + data.sum_je_debits;
        let expected_closing_credits = data.sum_opening_credits + data.sum_je_credits;

        let debit_difference = (expected_closing_debits - data.closing_tb_debits).abs();
        let credit_difference = (expected_closing_credits - data.closing_tb_credits).abs();

        let debits_reconciled = debit_difference <= self.tolerance;
        let credits_reconciled = credit_difference <= self.tolerance;

        let mut failures = Vec::new();
        if !debits_reconciled {
            failures.push(format!(
                "TB master proof (debits): expected {} vs closing TB {} (diff {})",
                expected_closing_debits, data.closing_tb_debits, debit_difference
            ));
        }
        if !credits_reconciled {
            failures.push(format!(
                "TB master proof (credits): expected {} vs closing TB {} (diff {})",
                expected_closing_credits, data.closing_tb_credits, credit_difference
            ));
        }

        TrialBalanceMasterProofEvaluation {
            debits_reconciled,
            credits_reconciled,
            debit_difference,
            credit_difference,
            passes: debits_reconciled && credits_reconciled,
            failures,
        }
    }
}

impl Default for TrialBalanceMasterProofEvaluator {
    fn default() -> Self {
        Self::new(Decimal::new(1, 2)) // 0.01 tolerance
    }
}

// ─── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    // Cash flow tests

    #[test]
    fn test_cash_flow_reconciliation_balanced() {
        let data = CashFlowReconciliationData {
            opening_cash: dec!(100_000),
            net_operating: dec!(50_000),
            net_investing: dec!(-20_000),
            net_financing: dec!(-10_000),
            closing_cash_gl: dec!(120_000),
        };
        let result = CashFlowReconciliationEvaluator::new(dec!(1)).evaluate(&data);
        assert!(result.passes);
        assert!(result.reconciled);
        assert_eq!(result.expected_closing, dec!(120_000));
        assert!(result.failures.is_empty());
    }

    #[test]
    fn test_cash_flow_reconciliation_imbalanced() {
        let data = CashFlowReconciliationData {
            opening_cash: dec!(100_000),
            net_operating: dec!(50_000),
            net_investing: dec!(-20_000),
            net_financing: dec!(-10_000),
            closing_cash_gl: dec!(200_000), // Way off
        };
        let result = CashFlowReconciliationEvaluator::new(dec!(1)).evaluate(&data);
        assert!(!result.passes);
        assert!(!result.reconciled);
        assert!(!result.failures.is_empty());
    }

    // Equity roll-forward tests

    #[test]
    fn test_equity_rollforward_balanced() {
        // 500_000 + 80_000 + 10_000 - 20_000 + 5_000 = 575_000
        let data = EquityRollforwardData {
            opening_equity: dec!(500_000),
            net_income: dec!(80_000),
            oci_movements: dec!(10_000),
            dividends_declared: dec!(20_000),
            stock_comp: dec!(5_000),
            closing_equity: dec!(575_000),
        };
        let result = EquityRollforwardEvaluator::new(dec!(1)).evaluate(&data);
        assert!(result.passes);
        assert!(result.reconciled);
        assert_eq!(result.expected_closing, dec!(575_000));
        assert!(result.failures.is_empty());
    }

    #[test]
    fn test_equity_rollforward_imbalanced() {
        let data = EquityRollforwardData {
            opening_equity: dec!(500_000),
            net_income: dec!(80_000),
            oci_movements: dec!(10_000),
            dividends_declared: dec!(20_000),
            stock_comp: dec!(5_000),
            closing_equity: dec!(999_999), // Wrong
        };
        let result = EquityRollforwardEvaluator::new(dec!(1)).evaluate(&data);
        assert!(!result.passes);
        assert!(!result.reconciled);
        assert!(!result.failures.is_empty());
    }

    // Segment reconciliation tests

    #[test]
    fn test_segment_reconciliation_balanced() {
        // 1_200_000 - 200_000 = 1_000_000
        let data = SegmentReconciliationData {
            sum_segment_revenue: dec!(1_200_000),
            ic_eliminations: dec!(200_000),
            consolidated_revenue: dec!(1_000_000),
        };
        let result = SegmentReconciliationEvaluator::new(dec!(1)).evaluate(&data);
        assert!(result.passes);
        assert!(result.reconciled);
        assert_eq!(result.expected_consolidated, dec!(1_000_000));
        assert!(result.failures.is_empty());
    }

    #[test]
    fn test_segment_reconciliation_imbalanced() {
        let data = SegmentReconciliationData {
            sum_segment_revenue: dec!(1_200_000),
            ic_eliminations: dec!(200_000),
            consolidated_revenue: dec!(850_000), // Missing 150_000
        };
        let result = SegmentReconciliationEvaluator::new(dec!(1)).evaluate(&data);
        assert!(!result.passes);
        assert!(!result.reconciled);
        assert!(!result.failures.is_empty());
    }

    // Trial balance master proof tests

    #[test]
    fn test_tb_master_proof_both_balanced() {
        let data = TrialBalanceMasterProofData {
            sum_opening_debits: dec!(500_000),
            sum_opening_credits: dec!(500_000),
            sum_je_debits: dec!(100_000),
            sum_je_credits: dec!(100_000),
            closing_tb_debits: dec!(600_000),
            closing_tb_credits: dec!(600_000),
        };
        let result = TrialBalanceMasterProofEvaluator::new(dec!(1)).evaluate(&data);
        assert!(result.passes);
        assert!(result.debits_reconciled);
        assert!(result.credits_reconciled);
        assert!(result.failures.is_empty());
    }

    #[test]
    fn test_tb_master_proof_debits_imbalanced() {
        let data = TrialBalanceMasterProofData {
            sum_opening_debits: dec!(500_000),
            sum_opening_credits: dec!(500_000),
            sum_je_debits: dec!(100_000),
            sum_je_credits: dec!(100_000),
            closing_tb_debits: dec!(550_000), // 50_000 short
            closing_tb_credits: dec!(600_000),
        };
        let result = TrialBalanceMasterProofEvaluator::new(dec!(1)).evaluate(&data);
        assert!(!result.passes);
        assert!(!result.debits_reconciled);
        assert!(result.credits_reconciled);
        assert_eq!(result.failures.len(), 1);
    }

    #[test]
    fn test_tb_master_proof_credits_imbalanced() {
        let data = TrialBalanceMasterProofData {
            sum_opening_debits: dec!(500_000),
            sum_opening_credits: dec!(500_000),
            sum_je_debits: dec!(100_000),
            sum_je_credits: dec!(100_000),
            closing_tb_debits: dec!(600_000),
            closing_tb_credits: dec!(550_000), // 50_000 short
        };
        let result = TrialBalanceMasterProofEvaluator::new(dec!(1)).evaluate(&data);
        assert!(!result.passes);
        assert!(result.debits_reconciled);
        assert!(!result.credits_reconciled);
        assert_eq!(result.failures.len(), 1);
    }

    #[test]
    fn test_tb_master_proof_both_imbalanced() {
        let data = TrialBalanceMasterProofData {
            sum_opening_debits: dec!(500_000),
            sum_opening_credits: dec!(500_000),
            sum_je_debits: dec!(100_000),
            sum_je_credits: dec!(100_000),
            closing_tb_debits: dec!(400_000), // Wrong
            closing_tb_credits: dec!(700_000), // Wrong
        };
        let result = TrialBalanceMasterProofEvaluator::new(dec!(1)).evaluate(&data);
        assert!(!result.passes);
        assert!(!result.debits_reconciled);
        assert!(!result.credits_reconciled);
        assert_eq!(result.failures.len(), 2);
    }
}

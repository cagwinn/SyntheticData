//! Treasury, tax, and payroll coherence validators.
//!
//! Validates cross-domain accounting integrity including:
//! - Interest expense reconciliation (GL vs instrument-level)
//! - Effective tax rate reconciliation
//! - Hedge effectiveness corridor compliance
//! - Payroll/HR salary change traceability

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// ─── InterestExpenseProofEvaluator ────────────────────────────────────────────

/// Input data for interest expense reconciliation.
#[derive(Debug, Clone)]
pub struct InterestExpenseProofData {
    /// Total interest expense posted to the GL.
    pub total_interest_expense_gl: Decimal,
    /// Sum of interest computed from individual debt instruments.
    pub sum_instrument_interest: Decimal,
}

/// Results of interest expense proof evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterestExpenseProofEvaluation {
    /// Whether GL interest expense reconciles to instrument-level interest.
    pub reconciled: bool,
    /// Absolute difference between GL and instrument totals.
    pub difference: Decimal,
    /// Overall pass/fail status.
    pub passes: bool,
    /// Human-readable descriptions of failed checks.
    pub failures: Vec<String>,
}

/// Evaluator that verifies GL interest expense equals the sum of instrument-level interest.
pub struct InterestExpenseProofEvaluator {
    tolerance: Decimal,
}

impl InterestExpenseProofEvaluator {
    /// Create a new evaluator with the given absolute tolerance.
    pub fn new(tolerance: Decimal) -> Self {
        Self { tolerance }
    }

    /// Run the interest expense proof check against `data`.
    pub fn evaluate(&self, data: &InterestExpenseProofData) -> InterestExpenseProofEvaluation {
        let difference =
            (data.total_interest_expense_gl - data.sum_instrument_interest).abs();
        let reconciled = difference <= self.tolerance;
        let mut failures = Vec::new();
        if !reconciled {
            failures.push(format!(
                "Interest expense GL {} vs instruments {} (diff {})",
                data.total_interest_expense_gl, data.sum_instrument_interest, difference
            ));
        }
        InterestExpenseProofEvaluation {
            reconciled,
            difference,
            passes: reconciled,
            failures,
        }
    }
}

impl Default for InterestExpenseProofEvaluator {
    fn default() -> Self {
        Self::new(Decimal::new(1, 2)) // 0.01 tolerance
    }
}

// ─── ETRReconciliationEvaluator ───────────────────────────────────────────────

/// Input data for effective tax rate reconciliation.
#[derive(Debug, Clone)]
pub struct ETRReconciliationData {
    /// Pre-tax income (PTI) for the period.
    pub pre_tax_income: Decimal,
    /// Statutory tax rate as a decimal fraction (e.g. 0.21 for 21%).
    pub statutory_rate: Decimal,
    /// Actual income tax expense recognised in the P&L.
    pub actual_tax_expense: Decimal,
    /// Sum of all ETR reconciling items (permanent differences, credits, etc.).
    pub sum_reconciling_items: Decimal,
}

/// Results of ETR reconciliation evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ETRReconciliationEvaluation {
    /// Whether expected tax reconciles to actual tax expense.
    pub reconciled: bool,
    /// Expected tax expense: PTI × statutory_rate + reconciling items.
    pub expected_tax: Decimal,
    /// Absolute difference between expected and actual tax expense.
    pub difference: Decimal,
    /// Overall pass/fail status.
    pub passes: bool,
    /// Human-readable descriptions of failed checks.
    pub failures: Vec<String>,
}

/// Evaluator that verifies statutory rate × PTI + reconciling items ≈ actual tax expense.
pub struct ETRReconciliationEvaluator {
    tolerance: Decimal,
}

impl ETRReconciliationEvaluator {
    /// Create a new evaluator with the given absolute tolerance.
    pub fn new(tolerance: Decimal) -> Self {
        Self { tolerance }
    }

    /// Run the ETR reconciliation check against `data`.
    pub fn evaluate(&self, data: &ETRReconciliationData) -> ETRReconciliationEvaluation {
        let expected_tax =
            data.pre_tax_income * data.statutory_rate + data.sum_reconciling_items;
        let difference = (expected_tax - data.actual_tax_expense).abs();
        let reconciled = difference <= self.tolerance;
        let mut failures = Vec::new();
        if !reconciled {
            failures.push(format!(
                "ETR reconciliation failed: expected tax {} vs actual {} (diff {})",
                expected_tax, data.actual_tax_expense, difference
            ));
        }
        ETRReconciliationEvaluation {
            reconciled,
            expected_tax,
            difference,
            passes: reconciled,
            failures,
        }
    }
}

impl Default for ETRReconciliationEvaluator {
    fn default() -> Self {
        Self::new(Decimal::new(1, 2)) // 0.01 tolerance
    }
}

// ─── HedgeEffectivenessEvaluator ──────────────────────────────────────────────

/// Input data for hedge effectiveness checks.
#[derive(Debug, Clone)]
pub struct HedgeEffectivenessData {
    /// Total number of hedge relationships in the population.
    pub total_hedges: usize,
    /// Number of hedges that remain within the 80–125% effectiveness corridor.
    pub effective_hedges: usize,
    /// Number of hedges that have been discontinued.
    pub discontinued_hedges: usize,
    /// Number of discontinued hedges that have a P&L reclassification entry.
    pub discontinued_with_pl_entries: usize,
}

/// Results of hedge effectiveness evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HedgeEffectivenessEvaluation {
    /// Fraction of total hedges classified as effective (0.0–1.0).
    pub effectiveness_rate: f64,
    /// Whether every discontinued hedge has a corresponding P&L reclassification entry.
    pub all_discontinued_have_pl: bool,
    /// Overall pass/fail status.
    pub passes: bool,
    /// Human-readable descriptions of failed checks.
    pub failures: Vec<String>,
}

/// Evaluator for hedge effectiveness corridor compliance and discontinuation accounting.
pub struct HedgeEffectivenessEvaluator;

impl HedgeEffectivenessEvaluator {
    /// Run hedge effectiveness checks against `data`.
    pub fn evaluate(&self, data: &HedgeEffectivenessData) -> HedgeEffectivenessEvaluation {
        let effectiveness_rate = if data.total_hedges == 0 {
            1.0
        } else {
            data.effective_hedges as f64 / data.total_hedges as f64
        };

        let all_discontinued_have_pl =
            data.discontinued_with_pl_entries >= data.discontinued_hedges;
        let mut failures = Vec::new();
        if !all_discontinued_have_pl {
            failures.push(format!(
                "Hedge discontinuation incomplete: {}/{} discontinued hedges have P&L reclassification entries",
                data.discontinued_with_pl_entries, data.discontinued_hedges
            ));
        }

        let passes = failures.is_empty();
        HedgeEffectivenessEvaluation {
            effectiveness_rate,
            all_discontinued_have_pl,
            passes,
            failures,
        }
    }
}

// ─── PayrollHRReconciliationEvaluator ─────────────────────────────────────────

/// Input data for payroll/HR reconciliation.
#[derive(Debug, Clone)]
pub struct PayrollHRReconciliationData {
    /// Number of salary change events recorded in the HR master data.
    pub salary_change_count: usize,
    /// Number of payroll variance entries that can be traced to those changes.
    pub payroll_variance_count: usize,
}

/// Results of payroll/HR reconciliation evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayrollHRReconciliationEvaluation {
    /// Whether every salary change has a corresponding payroll variance.
    pub changes_traced: bool,
    /// Overall pass/fail status.
    pub passes: bool,
    /// Human-readable descriptions of failed checks.
    pub failures: Vec<String>,
}

/// Evaluator that verifies salary changes trace to payroll variances.
pub struct PayrollHRReconciliationEvaluator;

impl PayrollHRReconciliationEvaluator {
    /// Run the payroll/HR reconciliation check against `data`.
    pub fn evaluate(
        &self,
        data: &PayrollHRReconciliationData,
    ) -> PayrollHRReconciliationEvaluation {
        let changes_traced = data.payroll_variance_count >= data.salary_change_count;
        let mut failures = Vec::new();
        if !changes_traced {
            failures.push(format!(
                "Payroll/HR reconciliation failed: {} salary changes but only {} payroll variance entries",
                data.salary_change_count, data.payroll_variance_count
            ));
        }
        PayrollHRReconciliationEvaluation {
            changes_traced,
            passes: changes_traced,
            failures,
        }
    }
}

// ─── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_interest_expense_proof_reconciled() {
        let data = InterestExpenseProofData {
            total_interest_expense_gl: dec!(50_000),
            sum_instrument_interest: dec!(50_000),
        };
        let result = InterestExpenseProofEvaluator::new(dec!(100)).evaluate(&data);
        assert!(result.passes);
        assert!(result.reconciled);
        assert!(result.failures.is_empty());
    }

    #[test]
    fn test_interest_expense_proof_unreconciled() {
        let data = InterestExpenseProofData {
            total_interest_expense_gl: dec!(50_000),
            sum_instrument_interest: dec!(30_000),
        };
        let result = InterestExpenseProofEvaluator::new(dec!(100)).evaluate(&data);
        assert!(!result.passes);
        assert!(!result.failures.is_empty());
    }

    #[test]
    fn test_etr_reconciliation_reconciled() {
        // PTI=1_000_000, rate=21%, reconciling=+20_000 → expected=230_000
        let data = ETRReconciliationData {
            pre_tax_income: dec!(1_000_000),
            statutory_rate: dec!(0.21),
            actual_tax_expense: dec!(230_000),
            sum_reconciling_items: dec!(20_000),
        };
        let result = ETRReconciliationEvaluator::new(dec!(1_000)).evaluate(&data);
        assert!(result.passes);
        assert_eq!(result.expected_tax, dec!(230_000));
    }

    #[test]
    fn test_etr_reconciliation_unreconciled() {
        let data = ETRReconciliationData {
            pre_tax_income: dec!(1_000_000),
            statutory_rate: dec!(0.21),
            actual_tax_expense: dec!(999_000), // Way off
            sum_reconciling_items: dec!(0),
        };
        let result = ETRReconciliationEvaluator::new(dec!(1_000)).evaluate(&data);
        assert!(!result.passes);
        assert!(!result.failures.is_empty());
    }

    #[test]
    fn test_hedge_effectiveness_all_compliant() {
        let data = HedgeEffectivenessData {
            total_hedges: 10,
            effective_hedges: 9,
            discontinued_hedges: 1,
            discontinued_with_pl_entries: 1,
        };
        let result = HedgeEffectivenessEvaluator.evaluate(&data);
        assert!(result.passes);
        assert!(result.all_discontinued_have_pl);
        assert!((result.effectiveness_rate - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn test_hedge_effectiveness_missing_pl_entry() {
        let data = HedgeEffectivenessData {
            total_hedges: 10,
            effective_hedges: 8,
            discontinued_hedges: 2,
            discontinued_with_pl_entries: 1, // Missing one
        };
        let result = HedgeEffectivenessEvaluator.evaluate(&data);
        assert!(!result.passes);
        assert!(!result.all_discontinued_have_pl);
    }

    #[test]
    fn test_payroll_hr_changes_traced() {
        let data = PayrollHRReconciliationData {
            salary_change_count: 5,
            payroll_variance_count: 5,
        };
        let result = PayrollHRReconciliationEvaluator.evaluate(&data);
        assert!(result.passes);
        assert!(result.changes_traced);
    }

    #[test]
    fn test_payroll_hr_changes_missing_variances() {
        let data = PayrollHRReconciliationData {
            salary_change_count: 5,
            payroll_variance_count: 3,
        };
        let result = PayrollHRReconciliationEvaluator.evaluate(&data);
        assert!(!result.passes);
        assert!(!result.changes_traced);
        assert!(!result.failures.is_empty());
    }
}

use datasynth_eval::coherence::treasury_tax::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

#[test]
fn test_interest_expense_proof_pass() {
    let data = InterestExpenseProofData {
        total_interest_expense_gl: dec!(50_000),
        sum_instrument_interest: dec!(50_000),
    };
    let eval = InterestExpenseProofEvaluator::new(dec!(100));
    let result = eval.evaluate(&data);
    assert!(result.passes);
}

#[test]
fn test_interest_expense_proof_fail() {
    let data = InterestExpenseProofData {
        total_interest_expense_gl: dec!(50_000),
        sum_instrument_interest: dec!(30_000),
    };
    let eval = InterestExpenseProofEvaluator::new(dec!(100));
    let result = eval.evaluate(&data);
    assert!(!result.passes);
}

#[test]
fn test_etr_reconciliation_pass() {
    let data = ETRReconciliationData {
        pre_tax_income: dec!(1_000_000),
        statutory_rate: dec!(0.21),
        actual_tax_expense: dec!(230_000),
        sum_reconciling_items: dec!(20_000),
    };
    let eval = ETRReconciliationEvaluator::new(dec!(1_000));
    let result = eval.evaluate(&data);
    assert!(result.passes);
}

#[test]
fn test_etr_reconciliation_fail() {
    let data = ETRReconciliationData {
        pre_tax_income: dec!(1_000_000),
        statutory_rate: dec!(0.21),
        actual_tax_expense: dec!(999_000),
        sum_reconciling_items: dec!(0),
    };
    let eval = ETRReconciliationEvaluator::new(dec!(1_000));
    let result = eval.evaluate(&data);
    assert!(!result.passes);
}

#[test]
fn test_hedge_effectiveness_pass() {
    let data = HedgeEffectivenessData {
        total_hedges: 10,
        effective_hedges: 9,
        discontinued_hedges: 1,
        discontinued_with_pl_entries: 1,
    };
    let eval = HedgeEffectivenessEvaluator;
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert!(result.all_discontinued_have_pl);
}

#[test]
fn test_hedge_effectiveness_fail() {
    let data = HedgeEffectivenessData {
        total_hedges: 10,
        effective_hedges: 8,
        discontinued_hedges: 2,
        discontinued_with_pl_entries: 1,
    };
    let eval = HedgeEffectivenessEvaluator;
    let result = eval.evaluate(&data);
    assert!(!result.passes);
    assert!(!result.all_discontinued_have_pl);
}

#[test]
fn test_payroll_hr_reconciliation() {
    let data = PayrollHRReconciliationData {
        salary_change_count: 5,
        payroll_variance_count: 5,
    };
    let eval = PayrollHRReconciliationEvaluator;
    let result = eval.evaluate(&data);
    assert!(result.passes);
}

#[test]
fn test_payroll_hr_reconciliation_fail() {
    let data = PayrollHRReconciliationData {
        salary_change_count: 5,
        payroll_variance_count: 3,
    };
    let eval = PayrollHRReconciliationEvaluator;
    let result = eval.evaluate(&data);
    assert!(!result.passes);
    assert!(!result.changes_traced);
}

#[test]
fn test_interest_expense_within_tolerance() {
    // Just inside tolerance
    let data = InterestExpenseProofData {
        total_interest_expense_gl: dec!(50_000),
        sum_instrument_interest: dec!(49_950),
    };
    let eval = InterestExpenseProofEvaluator::new(dec!(100));
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert_eq!(result.difference, dec!(50));
}

#[test]
fn test_etr_reconciliation_zero_pti() {
    // Zero PTI: expected tax = 0 + reconciling items
    let data = ETRReconciliationData {
        pre_tax_income: Decimal::ZERO,
        statutory_rate: dec!(0.21),
        actual_tax_expense: dec!(5_000),
        sum_reconciling_items: dec!(5_000),
    };
    let eval = ETRReconciliationEvaluator::new(dec!(1));
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert_eq!(result.expected_tax, dec!(5_000));
}

#[test]
fn test_hedge_effectiveness_no_hedges() {
    // Edge case: zero total hedges → effectiveness_rate defaults to 1.0
    let data = HedgeEffectivenessData {
        total_hedges: 0,
        effective_hedges: 0,
        discontinued_hedges: 0,
        discontinued_with_pl_entries: 0,
    };
    let eval = HedgeEffectivenessEvaluator;
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert!((result.effectiveness_rate - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_payroll_hr_surplus_variances_pass() {
    // More variances than changes is fine (>= condition)
    let data = PayrollHRReconciliationData {
        salary_change_count: 3,
        payroll_variance_count: 5,
    };
    let eval = PayrollHRReconciliationEvaluator;
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert!(result.changes_traced);
}

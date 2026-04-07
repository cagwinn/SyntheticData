use datasynth_eval::coherence::financial_package::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

// ─── CashFlowReconciliationEvaluator ─────────────────────────────────────────

#[test]
fn test_cash_flow_reconciliation_pass() {
    // 100_000 + 50_000 - 20_000 - 10_000 = 120_000
    let data = CashFlowReconciliationData {
        opening_cash: dec!(100_000),
        net_operating: dec!(50_000),
        net_investing: dec!(-20_000),
        net_financing: dec!(-10_000),
        closing_cash_gl: dec!(120_000),
    };
    let eval = CashFlowReconciliationEvaluator::new(dec!(0.01));
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert!(result.reconciled);
    assert_eq!(result.expected_closing, dec!(120_000));
    assert!(result.failures.is_empty());
}

#[test]
fn test_cash_flow_reconciliation_fail() {
    let data = CashFlowReconciliationData {
        opening_cash: dec!(100_000),
        net_operating: dec!(50_000),
        net_investing: dec!(-20_000),
        net_financing: dec!(-10_000),
        closing_cash_gl: dec!(200_000), // 80_000 off
    };
    let eval = CashFlowReconciliationEvaluator::new(dec!(0.01));
    let result = eval.evaluate(&data);
    assert!(!result.passes);
    assert!(!result.reconciled);
    assert_eq!(result.difference, dec!(80_000));
    assert!(!result.failures.is_empty());
}

#[test]
fn test_cash_flow_reconciliation_within_tolerance() {
    // Difference of 0.005 is inside a tolerance of 0.01
    let data = CashFlowReconciliationData {
        opening_cash: dec!(100_000),
        net_operating: dec!(50_000),
        net_investing: dec!(-20_000),
        net_financing: dec!(-10_000),
        closing_cash_gl: dec!(119_999.995), // 0.005 under
    };
    let eval = CashFlowReconciliationEvaluator::new(dec!(0.01));
    let result = eval.evaluate(&data);
    assert!(result.passes);
}

#[test]
fn test_cash_flow_reconciliation_all_negative_flows() {
    // Cash burn scenario: all three activity totals negative
    let data = CashFlowReconciliationData {
        opening_cash: dec!(500_000),
        net_operating: dec!(-30_000),
        net_investing: dec!(-100_000),
        net_financing: dec!(-50_000),
        closing_cash_gl: dec!(320_000),
    };
    let eval = CashFlowReconciliationEvaluator::new(dec!(1));
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert_eq!(result.expected_closing, dec!(320_000));
}

#[test]
fn test_cash_flow_reconciliation_zero_opening() {
    let data = CashFlowReconciliationData {
        opening_cash: Decimal::ZERO,
        net_operating: dec!(75_000),
        net_investing: dec!(-25_000),
        net_financing: dec!(10_000),
        closing_cash_gl: dec!(60_000),
    };
    let eval = CashFlowReconciliationEvaluator::new(dec!(1));
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert_eq!(result.expected_closing, dec!(60_000));
}

// ─── EquityRollforwardEvaluator ───────────────────────────────────────────────

#[test]
fn test_equity_rollforward_pass() {
    // 500_000 + 80_000 + 10_000 - 20_000 + 5_000 = 575_000
    let data = EquityRollforwardData {
        opening_equity: dec!(500_000),
        net_income: dec!(80_000),
        oci_movements: dec!(10_000),
        dividends_declared: dec!(20_000),
        stock_comp: dec!(5_000),
        closing_equity: dec!(575_000),
    };
    let eval = EquityRollforwardEvaluator::new(dec!(0.01));
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert!(result.reconciled);
    assert_eq!(result.expected_closing, dec!(575_000));
    assert!(result.failures.is_empty());
}

#[test]
fn test_equity_rollforward_fail() {
    let data = EquityRollforwardData {
        opening_equity: dec!(500_000),
        net_income: dec!(80_000),
        oci_movements: dec!(10_000),
        dividends_declared: dec!(20_000),
        stock_comp: dec!(5_000),
        closing_equity: dec!(400_000), // 175_000 off
    };
    let eval = EquityRollforwardEvaluator::new(dec!(0.01));
    let result = eval.evaluate(&data);
    assert!(!result.passes);
    assert!(!result.reconciled);
    assert_eq!(result.difference, dec!(175_000));
    assert!(!result.failures.is_empty());
}

#[test]
fn test_equity_rollforward_net_loss() {
    // Net loss reduces equity
    // 300_000 - 40_000 + 5_000 - 10_000 + 2_000 = 257_000
    let data = EquityRollforwardData {
        opening_equity: dec!(300_000),
        net_income: dec!(-40_000),
        oci_movements: dec!(5_000),
        dividends_declared: dec!(10_000),
        stock_comp: dec!(2_000),
        closing_equity: dec!(257_000),
    };
    let eval = EquityRollforwardEvaluator::new(dec!(1));
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert_eq!(result.expected_closing, dec!(257_000));
}

#[test]
fn test_equity_rollforward_no_dividends_no_oci() {
    // Simplified: only NI and stock comp move
    // 1_000_000 + 200_000 + 0 - 0 + 15_000 = 1_215_000
    let data = EquityRollforwardData {
        opening_equity: dec!(1_000_000),
        net_income: dec!(200_000),
        oci_movements: Decimal::ZERO,
        dividends_declared: Decimal::ZERO,
        stock_comp: dec!(15_000),
        closing_equity: dec!(1_215_000),
    };
    let eval = EquityRollforwardEvaluator::new(dec!(1));
    let result = eval.evaluate(&data);
    assert!(result.passes);
}

#[test]
fn test_equity_rollforward_negative_oci() {
    // Unrealised FX losses drive negative OCI
    // 800_000 + 50_000 - 30_000 - 25_000 + 10_000 = 805_000
    let data = EquityRollforwardData {
        opening_equity: dec!(800_000),
        net_income: dec!(50_000),
        oci_movements: dec!(-30_000),
        dividends_declared: dec!(25_000),
        stock_comp: dec!(10_000),
        closing_equity: dec!(805_000),
    };
    let eval = EquityRollforwardEvaluator::new(dec!(1));
    let result = eval.evaluate(&data);
    assert!(result.passes);
}

// ─── SegmentReconciliationEvaluator ──────────────────────────────────────────

#[test]
fn test_segment_reconciliation_pass() {
    // 1_200_000 - 200_000 = 1_000_000
    let data = SegmentReconciliationData {
        sum_segment_revenue: dec!(1_200_000),
        ic_eliminations: dec!(200_000),
        consolidated_revenue: dec!(1_000_000),
    };
    let eval = SegmentReconciliationEvaluator::new(dec!(0.01));
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert!(result.reconciled);
    assert_eq!(result.expected_consolidated, dec!(1_000_000));
    assert!(result.failures.is_empty());
}

#[test]
fn test_segment_reconciliation_fail() {
    let data = SegmentReconciliationData {
        sum_segment_revenue: dec!(1_200_000),
        ic_eliminations: dec!(200_000),
        consolidated_revenue: dec!(850_000), // 150_000 short
    };
    let eval = SegmentReconciliationEvaluator::new(dec!(0.01));
    let result = eval.evaluate(&data);
    assert!(!result.passes);
    assert!(!result.reconciled);
    assert_eq!(result.difference, dec!(150_000));
    assert!(!result.failures.is_empty());
}

#[test]
fn test_segment_reconciliation_no_eliminations() {
    // Single-entity: no IC eliminations, segments equal consolidated
    let data = SegmentReconciliationData {
        sum_segment_revenue: dec!(500_000),
        ic_eliminations: Decimal::ZERO,
        consolidated_revenue: dec!(500_000),
    };
    let eval = SegmentReconciliationEvaluator::new(dec!(1));
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert_eq!(result.expected_consolidated, dec!(500_000));
}

#[test]
fn test_segment_reconciliation_within_tolerance() {
    // Difference of 0.005 < tolerance 0.01
    let data = SegmentReconciliationData {
        sum_segment_revenue: dec!(1_000_000),
        ic_eliminations: dec!(100_000),
        consolidated_revenue: dec!(899_999.995),
    };
    let eval = SegmentReconciliationEvaluator::new(dec!(0.01));
    let result = eval.evaluate(&data);
    assert!(result.passes);
}

// ─── TrialBalanceMasterProofEvaluator ─────────────────────────────────────────

#[test]
fn test_tb_master_proof_pass() {
    let data = TrialBalanceMasterProofData {
        sum_opening_debits: dec!(500_000),
        sum_opening_credits: dec!(500_000),
        sum_je_debits: dec!(100_000),
        sum_je_credits: dec!(100_000),
        closing_tb_debits: dec!(600_000),
        closing_tb_credits: dec!(600_000),
    };
    let eval = TrialBalanceMasterProofEvaluator::new(dec!(0.01));
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert!(result.debits_reconciled);
    assert!(result.credits_reconciled);
    assert!(result.failures.is_empty());
}

#[test]
fn test_tb_master_proof_debits_fail() {
    let data = TrialBalanceMasterProofData {
        sum_opening_debits: dec!(500_000),
        sum_opening_credits: dec!(500_000),
        sum_je_debits: dec!(100_000),
        sum_je_credits: dec!(100_000),
        closing_tb_debits: dec!(550_000), // 50_000 short on debits
        closing_tb_credits: dec!(600_000),
    };
    let eval = TrialBalanceMasterProofEvaluator::new(dec!(0.01));
    let result = eval.evaluate(&data);
    assert!(!result.passes);
    assert!(!result.debits_reconciled);
    assert!(result.credits_reconciled);
    assert_eq!(result.debit_difference, dec!(50_000));
    assert_eq!(result.failures.len(), 1);
}

#[test]
fn test_tb_master_proof_credits_fail() {
    let data = TrialBalanceMasterProofData {
        sum_opening_debits: dec!(500_000),
        sum_opening_credits: dec!(500_000),
        sum_je_debits: dec!(100_000),
        sum_je_credits: dec!(100_000),
        closing_tb_debits: dec!(600_000),
        closing_tb_credits: dec!(450_000), // 150_000 short on credits
    };
    let eval = TrialBalanceMasterProofEvaluator::new(dec!(0.01));
    let result = eval.evaluate(&data);
    assert!(!result.passes);
    assert!(result.debits_reconciled);
    assert!(!result.credits_reconciled);
    assert_eq!(result.credit_difference, dec!(150_000));
    assert_eq!(result.failures.len(), 1);
}

#[test]
fn test_tb_master_proof_both_fail() {
    let data = TrialBalanceMasterProofData {
        sum_opening_debits: dec!(500_000),
        sum_opening_credits: dec!(500_000),
        sum_je_debits: dec!(100_000),
        sum_je_credits: dec!(100_000),
        closing_tb_debits: dec!(400_000),  // off
        closing_tb_credits: dec!(700_000), // off
    };
    let eval = TrialBalanceMasterProofEvaluator::new(dec!(0.01));
    let result = eval.evaluate(&data);
    assert!(!result.passes);
    assert!(!result.debits_reconciled);
    assert!(!result.credits_reconciled);
    assert_eq!(result.failures.len(), 2);
}

#[test]
fn test_tb_master_proof_zero_je_activity() {
    // No JEs in the period: closing TB should equal opening TB
    let data = TrialBalanceMasterProofData {
        sum_opening_debits: dec!(1_000_000),
        sum_opening_credits: dec!(1_000_000),
        sum_je_debits: Decimal::ZERO,
        sum_je_credits: Decimal::ZERO,
        closing_tb_debits: dec!(1_000_000),
        closing_tb_credits: dec!(1_000_000),
    };
    let eval = TrialBalanceMasterProofEvaluator::new(dec!(1));
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert_eq!(result.debit_difference, Decimal::ZERO);
    assert_eq!(result.credit_difference, Decimal::ZERO);
}

#[test]
fn test_tb_master_proof_within_tolerance() {
    // Difference of 0.005 < tolerance 0.01
    let data = TrialBalanceMasterProofData {
        sum_opening_debits: dec!(500_000),
        sum_opening_credits: dec!(500_000),
        sum_je_debits: dec!(100_000),
        sum_je_credits: dec!(100_000),
        closing_tb_debits: dec!(599_999.995),
        closing_tb_credits: dec!(600_000),
    };
    let eval = TrialBalanceMasterProofEvaluator::new(dec!(0.01));
    let result = eval.evaluate(&data);
    assert!(result.passes);
}

// ─── CoherenceEvaluation integration ─────────────────────────────────────────

#[test]
fn test_coherence_evaluation_propagates_failures() {
    use datasynth_eval::coherence::CoherenceEvaluation;
    use datasynth_eval::config::EvaluationThresholds;

    let mut eval = CoherenceEvaluation::new();

    // Inject a failing cash flow reconciliation
    eval.cash_flow_reconciliation = Some(CashFlowReconciliationEvaluation {
        reconciled: false,
        expected_closing: dec!(120_000),
        difference: dec!(80_000),
        passes: false,
        failures: vec!["Cash flow reconciliation failed: expected closing cash 120000 vs GL 200000 (diff 80000)".to_string()],
    });

    // Inject a passing equity roll-forward
    eval.equity_rollforward = Some(EquityRollforwardEvaluation {
        reconciled: true,
        expected_closing: dec!(575_000),
        difference: Decimal::ZERO,
        passes: true,
        failures: vec![],
    });

    let thresholds = EvaluationThresholds::default();
    eval.check_thresholds(&thresholds);

    assert!(!eval.passes);
    assert_eq!(eval.failures.len(), 1);
    assert!(eval.failures[0].contains("Cash flow reconciliation failed"));
}

#[test]
fn test_coherence_evaluation_all_pass() {
    use datasynth_eval::coherence::CoherenceEvaluation;
    use datasynth_eval::config::EvaluationThresholds;

    let mut eval = CoherenceEvaluation::new();

    eval.cash_flow_reconciliation = Some(CashFlowReconciliationEvaluation {
        reconciled: true,
        expected_closing: dec!(120_000),
        difference: Decimal::ZERO,
        passes: true,
        failures: vec![],
    });
    eval.equity_rollforward = Some(EquityRollforwardEvaluation {
        reconciled: true,
        expected_closing: dec!(575_000),
        difference: Decimal::ZERO,
        passes: true,
        failures: vec![],
    });
    eval.segment_reconciliation = Some(SegmentReconciliationEvaluation {
        reconciled: true,
        expected_consolidated: dec!(1_000_000),
        difference: Decimal::ZERO,
        passes: true,
        failures: vec![],
    });
    eval.tb_master_proof = Some(TrialBalanceMasterProofEvaluation {
        debits_reconciled: true,
        credits_reconciled: true,
        debit_difference: Decimal::ZERO,
        credit_difference: Decimal::ZERO,
        passes: true,
        failures: vec![],
    });

    let thresholds = EvaluationThresholds::default();
    eval.check_thresholds(&thresholds);

    assert!(eval.passes);
    assert!(eval.failures.is_empty());
}

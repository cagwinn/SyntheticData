use datasynth_eval::coherence::inventory_cogs::*;
use rust_decimal::Decimal;

#[test]
fn test_cogs_reconciliation_balanced() {
    let data = InventoryCOGSData {
        opening_fg: Decimal::new(100_000, 0),
        production_completions: Decimal::new(500_000, 0),
        cogs: Decimal::new(450_000, 0),
        scrap: Decimal::new(10_000, 0),
        closing_fg: Decimal::new(140_000, 0),
        opening_wip: Decimal::new(50_000, 0),
        material_issues: Decimal::new(300_000, 0),
        labor_absorbed: Decimal::new(150_000, 0),
        overhead_applied: Decimal::new(100_000, 0),
        completions_out_of_wip: Decimal::new(500_000, 0),
        wip_scrap: Decimal::new(5_000, 0),
        closing_wip: Decimal::new(95_000, 0),
        total_variance: Decimal::new(15_000, 0),
        sum_of_component_variances: Decimal::new(15_000, 0),
    };
    let eval = InventoryCOGSEvaluator::new(Decimal::new(1, 0));
    let result = eval.evaluate(&data);
    assert!(result.passes);
    assert!(result.fg_reconciled);
    assert!(result.wip_reconciled);
    assert!(result.variances_reconciled);
}

#[test]
fn test_cogs_reconciliation_imbalanced() {
    let data = InventoryCOGSData {
        opening_fg: Decimal::new(100_000, 0),
        production_completions: Decimal::new(500_000, 0),
        cogs: Decimal::new(450_000, 0),
        scrap: Decimal::new(10_000, 0),
        closing_fg: Decimal::new(200_000, 0), // Wrong
        opening_wip: Decimal::ZERO,
        material_issues: Decimal::ZERO,
        labor_absorbed: Decimal::ZERO,
        overhead_applied: Decimal::ZERO,
        completions_out_of_wip: Decimal::ZERO,
        wip_scrap: Decimal::ZERO,
        closing_wip: Decimal::ZERO,
        total_variance: Decimal::ZERO,
        sum_of_component_variances: Decimal::ZERO,
    };
    let eval = InventoryCOGSEvaluator::new(Decimal::new(1, 0));
    let result = eval.evaluate(&data);
    assert!(!result.fg_reconciled);
    assert!(!result.passes);
}

#[test]
fn test_ic_elimination_completeness_pass() {
    let data = ICEliminationData {
        matched_pair_count: 10,
        elimination_entry_count: 10,
        total_ic_amount: Decimal::new(1_000_000, 0),
        total_elimination_amount: Decimal::new(1_000_000, 0),
        pairs_with_eliminations: 10,
    };
    let eval = ICEliminationEvaluator::new(Decimal::new(1, 0));
    let result = eval.evaluate(&data);
    assert!(result.passes);
}

#[test]
fn test_ic_elimination_incomplete() {
    let data = ICEliminationData {
        matched_pair_count: 10,
        elimination_entry_count: 7,
        total_ic_amount: Decimal::new(1_000_000, 0),
        total_elimination_amount: Decimal::new(700_000, 0),
        pairs_with_eliminations: 7,
    };
    let eval = ICEliminationEvaluator::new(Decimal::new(1, 0));
    let result = eval.evaluate(&data);
    assert!(!result.passes);
    assert!(!result.all_pairs_eliminated);
}

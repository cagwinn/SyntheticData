//! COGS and WIP coherence validators.
//!
//! Validates manufacturing cost flow integrity including:
//! - Finished goods inventory reconciliation
//! - Work-in-process reconciliation
//! - Cost variance reconciliation
//! - Intercompany elimination completeness

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// ─── InventoryCOGSEvaluator ───────────────────────────────────────────────────

/// Input data for COGS and inventory reconciliation checks.
#[derive(Debug, Clone)]
pub struct InventoryCOGSData {
    /// Opening finished-goods balance.
    pub opening_fg: Decimal,
    /// Production completions transferred into FG.
    pub production_completions: Decimal,
    /// Cost of goods sold posted to P&L.
    pub cogs: Decimal,
    /// Scrap written off from FG.
    pub scrap: Decimal,
    /// Closing finished-goods balance.
    pub closing_fg: Decimal,

    /// Opening work-in-process balance.
    pub opening_wip: Decimal,
    /// Raw-material issues to production.
    pub material_issues: Decimal,
    /// Labor costs absorbed into WIP.
    pub labor_absorbed: Decimal,
    /// Overhead applied to WIP.
    pub overhead_applied: Decimal,
    /// Completions transferred out of WIP (into FG).
    pub completions_out_of_wip: Decimal,
    /// Scrap written off directly from WIP.
    pub wip_scrap: Decimal,
    /// Closing work-in-process balance.
    pub closing_wip: Decimal,

    /// Total manufacturing variance reported.
    pub total_variance: Decimal,
    /// Sum of individual component variances (material, labor, overhead, etc.).
    pub sum_of_component_variances: Decimal,
}

/// Results of COGS and WIP coherence evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryCOGSEvaluation {
    /// Whether the FG roll-forward reconciles.
    pub fg_reconciled: bool,
    /// Difference between expected and actual closing FG.
    pub fg_imbalance: Decimal,
    /// Expected closing FG derived from the roll-forward equation.
    pub fg_expected_closing: Decimal,

    /// Whether the WIP roll-forward reconciles.
    pub wip_reconciled: bool,
    /// Difference between expected and actual closing WIP.
    pub wip_imbalance: Decimal,
    /// Expected closing WIP derived from the roll-forward equation.
    pub wip_expected_closing: Decimal,

    /// Whether total variance equals the sum of component variances.
    pub variances_reconciled: bool,
    /// Difference between total variance and sum of component variances.
    pub variance_difference: Decimal,

    /// Overall pass/fail status — all three checks must pass.
    pub passes: bool,
    /// Human-readable descriptions of failed checks.
    pub failures: Vec<String>,
}

/// Evaluator for finished-goods, WIP, and variance reconciliation.
pub struct InventoryCOGSEvaluator {
    tolerance: Decimal,
}

impl InventoryCOGSEvaluator {
    /// Create a new evaluator with the given absolute tolerance.
    pub fn new(tolerance: Decimal) -> Self {
        Self { tolerance }
    }

    /// Run all three reconciliation checks against `data`.
    pub fn evaluate(&self, data: &InventoryCOGSData) -> InventoryCOGSEvaluation {
        let mut failures = Vec::new();

        // 1. FG Reconciliation
        //    Opening FG + Completions - COGS - Scrap = Closing FG
        let fg_expected_closing =
            data.opening_fg + data.production_completions - data.cogs - data.scrap;
        let fg_imbalance = (fg_expected_closing - data.closing_fg).abs();
        let fg_reconciled = fg_imbalance <= self.tolerance;
        if !fg_reconciled {
            failures.push(format!(
                "FG reconciliation failed: expected closing FG {} but got {} (imbalance {})",
                fg_expected_closing, data.closing_fg, fg_imbalance
            ));
        }

        // 2. WIP Reconciliation
        //    Opening WIP + Materials + Labor + Overhead - Completions - Scrap = Closing WIP
        let wip_expected_closing = data.opening_wip
            + data.material_issues
            + data.labor_absorbed
            + data.overhead_applied
            - data.completions_out_of_wip
            - data.wip_scrap;
        let wip_imbalance = (wip_expected_closing - data.closing_wip).abs();
        let wip_reconciled = wip_imbalance <= self.tolerance;
        if !wip_reconciled {
            failures.push(format!(
                "WIP reconciliation failed: expected closing WIP {} but got {} (imbalance {})",
                wip_expected_closing, data.closing_wip, wip_imbalance
            ));
        }

        // 3. Variance Reconciliation
        //    Total variance = Sum of component variances
        let variance_difference =
            (data.total_variance - data.sum_of_component_variances).abs();
        let variances_reconciled = variance_difference <= self.tolerance;
        if !variances_reconciled {
            failures.push(format!(
                "Variance reconciliation failed: total variance {} != component sum {} (difference {})",
                data.total_variance, data.sum_of_component_variances, variance_difference
            ));
        }

        let passes = failures.is_empty();

        InventoryCOGSEvaluation {
            fg_reconciled,
            fg_imbalance,
            fg_expected_closing,
            wip_reconciled,
            wip_imbalance,
            wip_expected_closing,
            variances_reconciled,
            variance_difference,
            passes,
            failures,
        }
    }
}

impl Default for InventoryCOGSEvaluator {
    fn default() -> Self {
        Self::new(Decimal::new(1, 2)) // 0.01 tolerance
    }
}

// ─── ICEliminationEvaluator ──────────────────────────────────────────────────

/// Input data for intercompany elimination completeness checks.
#[derive(Debug, Clone)]
pub struct ICEliminationData {
    /// Total number of matched IC pairs.
    pub matched_pair_count: usize,
    /// Total number of elimination journal entries generated.
    pub elimination_entry_count: usize,
    /// Gross IC transaction amount (seller side).
    pub total_ic_amount: Decimal,
    /// Total amount covered by elimination entries.
    pub total_elimination_amount: Decimal,
    /// Number of matched pairs that have at least one elimination entry.
    pub pairs_with_eliminations: usize,
}

/// Results of intercompany elimination completeness evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ICEliminationEvaluation {
    /// Whether every matched pair has a corresponding elimination entry.
    pub all_pairs_eliminated: bool,
    /// Fraction of matched pairs that have eliminations (0.0–1.0).
    pub coverage_rate: f64,
    /// Whether the total elimination amount reconciles to the IC amount.
    pub amount_reconciled: bool,
    /// Absolute difference between IC amount and elimination amount.
    pub amount_difference: Decimal,
    /// Overall pass/fail status.
    pub passes: bool,
    /// Human-readable descriptions of failed checks.
    pub failures: Vec<String>,
}

/// Evaluator for intercompany elimination completeness.
pub struct ICEliminationEvaluator {
    tolerance: Decimal,
}

impl ICEliminationEvaluator {
    /// Create a new evaluator with the given absolute tolerance.
    pub fn new(tolerance: Decimal) -> Self {
        Self { tolerance }
    }

    /// Run elimination coverage and amount reconciliation checks.
    pub fn evaluate(&self, data: &ICEliminationData) -> ICEliminationEvaluation {
        let mut failures = Vec::new();

        // Coverage: every matched pair must have an elimination entry.
        let coverage_rate = if data.matched_pair_count == 0 {
            1.0
        } else {
            data.pairs_with_eliminations as f64 / data.matched_pair_count as f64
        };
        let all_pairs_eliminated = data.pairs_with_eliminations >= data.matched_pair_count;
        if !all_pairs_eliminated {
            failures.push(format!(
                "IC elimination incomplete: {}/{} pairs have eliminations (coverage {:.1}%)",
                data.pairs_with_eliminations,
                data.matched_pair_count,
                coverage_rate * 100.0
            ));
        }

        // Amount reconciliation: elimination amount must equal IC amount.
        let amount_difference =
            (data.total_ic_amount - data.total_elimination_amount).abs();
        let amount_reconciled = amount_difference <= self.tolerance;
        if !amount_reconciled {
            failures.push(format!(
                "IC elimination amount mismatch: IC amount {} vs elimination amount {} (difference {})",
                data.total_ic_amount, data.total_elimination_amount, amount_difference
            ));
        }

        let passes = failures.is_empty();

        ICEliminationEvaluation {
            all_pairs_eliminated,
            coverage_rate,
            amount_reconciled,
            amount_difference,
            passes,
            failures,
        }
    }
}

impl Default for ICEliminationEvaluator {
    fn default() -> Self {
        Self::new(Decimal::new(1, 2)) // 0.01 tolerance
    }
}

// ─── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_cogs_all_balanced() {
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
    fn test_fg_imbalanced() {
        let data = InventoryCOGSData {
            opening_fg: Decimal::new(100_000, 0),
            production_completions: Decimal::new(500_000, 0),
            cogs: Decimal::new(450_000, 0),
            scrap: Decimal::new(10_000, 0),
            closing_fg: Decimal::new(999_000, 0), // Wrong
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
        assert!(!result.failures.is_empty());
    }

    #[test]
    fn test_ic_elimination_complete() {
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
        assert!(result.all_pairs_eliminated);
        assert!((result.coverage_rate - 1.0).abs() < f64::EPSILON);
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
        assert_eq!(result.coverage_rate, 0.7);
    }
}

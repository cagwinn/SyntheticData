//! Covenant compliance evaluator.
//!
//! Evaluates debt covenants against actual financial ratios and produces a
//! [`CovenantEvaluationResult`] summarising compliance status.  Reclassification
//! journal entries for breached covenants are intentionally left empty here;
//! the orchestrator handles reclassification once it has access to principal
//! balances.

use chrono::NaiveDate;
use datasynth_core::models::{CovenantType, DebtCovenant, JournalEntry};
use rust_decimal::Decimal;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Output type
// ---------------------------------------------------------------------------

/// Result returned by [`CovenantEvaluator::evaluate_covenants`].
#[derive(Debug, Default)]
pub struct CovenantEvaluationResult {
    /// `true` when every covenant is in compliance.
    pub all_compliant: bool,
    /// IDs of covenants that breached their threshold.
    pub breached_covenants: Vec<String>,
    /// Reclassification journal entries for breaches (currently empty; reserved
    /// for the orchestrator layer).
    pub breach_jes: Vec<JournalEntry>,
}

// ---------------------------------------------------------------------------
// Evaluator
// ---------------------------------------------------------------------------

/// Stateless evaluator that updates covenant records from actual ratio data.
pub struct CovenantEvaluator;

impl CovenantEvaluator {
    /// Evaluates each covenant against `actual_ratios`, updating compliance
    /// fields in-place, and returns a summary result.
    ///
    /// # Arguments
    ///
    /// * `covenants` – mutable slice of covenants to evaluate.
    /// * `actual_ratios` – map from [`CovenantType`] to the measured ratio.
    /// * `measurement_date` – date at which the ratios were measured.
    pub fn evaluate_covenants(
        covenants: &mut [DebtCovenant],
        actual_ratios: &HashMap<CovenantType, Decimal>,
        measurement_date: NaiveDate,
    ) -> CovenantEvaluationResult {
        let mut breached_covenants: Vec<String> = Vec::new();

        for covenant in covenants.iter_mut() {
            if let Some(&actual_value) = actual_ratios.get(&covenant.covenant_type) {
                covenant.actual_value = actual_value;
                covenant.measurement_date = measurement_date;
                covenant.update_compliance();

                if !covenant.is_compliant {
                    breached_covenants.push(covenant.id.clone());
                }
            }
            // If the covenant type is not in the map we leave the covenant
            // unchanged — it retains its previous compliance state.
        }

        let all_compliant = breached_covenants.is_empty();

        CovenantEvaluationResult {
            all_compliant,
            breached_covenants,
            breach_jes: Vec::new(), // reclassification deferred to orchestrator
        }
    }
}

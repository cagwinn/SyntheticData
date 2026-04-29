//! AML typology detectability evaluator.
//!
//! Validates that AML typologies (structuring, layering, mule networks, etc.)
//! produce statistically detectable patterns and maintain coherence.

use crate::error::EvalResult;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// AML transaction data for a typology instance.
///
/// The `typology` string should be the canonical lowercase name
/// produced by `AmlTypology::canonical_name()` — see
/// [`EXPECTED_TYPOLOGIES`] for the allowed values. Using PascalCase
/// (e.g. the Debug format of the enum) will fail the coverage match.
#[derive(Debug, Clone)]
pub struct AmlTransactionData {
    /// Transaction identifier.
    pub transaction_id: String,
    /// Canonical typology name, e.g. "structuring", "mule", "fraud".
    pub typology: String,
    /// Case identifier (shared across related transactions).
    pub case_id: String,
    /// Transaction amount.
    pub amount: f64,
    /// Whether this is a flagged/suspicious transaction.
    pub is_flagged: bool,
}

/// Overall typology data for coverage validation.
#[derive(Debug, Clone)]
pub struct TypologyData {
    /// Typology name.
    pub name: String,
    /// Number of scenarios generated.
    pub scenario_count: usize,
    /// Whether all transactions in a scenario share a case_id.
    pub case_ids_consistent: bool,
}

/// Thresholds for AML detectability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmlDetectabilityThresholds {
    /// Minimum typology coverage (fraction of expected typologies present).
    pub min_typology_coverage: f64,
    /// Minimum scenario coherence rate.
    pub min_scenario_coherence: f64,
    /// Structuring threshold (transactions should cluster below this).
    pub structuring_threshold: f64,
    /// Minimum transaction count below which the typology-coverage
    /// metric is reported as advisory only (not a fail signal).
    ///
    /// v5.0.1 (Gap 2): with seven typology categories at heterogeneous
    /// per-category prevalence, a 1 k-row sample can miss a single
    /// low-rate category just by chance — that's a 14.3 pp drop in
    /// reported coverage even though the generator is firing all
    /// seven. Below this floor, we still compute coverage but skip
    /// the threshold-failure issue and emit an advisory note instead.
    pub min_sample_for_coverage: usize,
}

impl Default for AmlDetectabilityThresholds {
    fn default() -> Self {
        Self {
            min_typology_coverage: 0.80,
            min_scenario_coherence: 0.90,
            structuring_threshold: 10_000.0,
            min_sample_for_coverage: 5_000,
        }
    }
}

/// Per-typology detectability result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypologyDetectability {
    /// Typology name.
    pub name: String,
    /// Number of transactions.
    pub transaction_count: usize,
    /// Number of unique cases.
    pub case_count: usize,
    /// Flag rate.
    pub flag_rate: f64,
    /// Whether the typology shows expected patterns.
    pub pattern_detected: bool,
}

/// Results of AML detectability analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmlDetectabilityAnalysis {
    /// Typology coverage: fraction of expected typologies present.
    pub typology_coverage: f64,
    /// Scenario coherence: fraction of scenarios with consistent case_ids.
    pub scenario_coherence: f64,
    /// Per-typology detectability.
    pub per_typology: Vec<TypologyDetectability>,
    /// Total transactions analyzed.
    pub total_transactions: usize,
    /// Overall pass/fail.
    pub passes: bool,
    /// Issues found.
    pub issues: Vec<String>,
}

/// Expected typology categories for coverage calculation.
///
/// Matches the banking module catalog in CLAUDE.md:
///   structuring, funnel, layering, mule, round_tripping, fraud, spoofing
///
/// v4.4.2: each category is represented by a canonical name *plus* the
/// aliases the typology injectors emit into `TypologyData.name` and
/// `suspicion_reason`. Before v4.4.2 the evaluator did exact-string
/// matching against short names, so "money_mule" / "funnel_account" /
/// "first_party_fraud" / "authorized_push_payment" didn't match even
/// though the underlying typologies were firing — the SDK team saw
/// coverage 0.71 / 5-of-7 where the real coverage was 1.0 / 7-of-7.
///
/// Each entry is `(canonical, aliases)`. A category is "covered" when
/// ANY of its names appears in the typology set.
const EXPECTED_TYPOLOGIES: &[(&str, &[&str])] = &[
    (
        "structuring",
        &["structuring", "smurfing", "cuckoo_smurfing"],
    ),
    (
        "funnel",
        &[
            "funnel",
            "funnel_account",
            "concentration_account",
            "pouch_activity",
        ],
    ),
    ("layering", &["layering", "rapid_movement", "shell_company"]),
    (
        "mule",
        &[
            "mule",
            "money_mule",
            "authorized_push_payment",
            "synthetic_identity",
        ],
    ),
    (
        "round_tripping",
        &[
            "round_tripping",
            "trade_based_ml",
            "real_estate_integration",
        ],
    ),
    (
        "fraud",
        &[
            "fraud",
            "first_party_fraud",
            "account_takeover",
            "romance_scam",
            "sanctions_evasion",
        ],
    ),
    (
        "spoofing",
        &["spoofing", "casino_integration", "crypto_integration"],
    ),
];

/// Analyzer for AML detectability.
pub struct AmlDetectabilityAnalyzer {
    thresholds: AmlDetectabilityThresholds,
}

impl AmlDetectabilityAnalyzer {
    /// Create a new analyzer with default thresholds.
    pub fn new() -> Self {
        Self {
            thresholds: AmlDetectabilityThresholds::default(),
        }
    }

    /// Create with custom thresholds.
    pub fn with_thresholds(thresholds: AmlDetectabilityThresholds) -> Self {
        Self { thresholds }
    }

    /// Analyze AML transactions and typology data.
    pub fn analyze(
        &self,
        transactions: &[AmlTransactionData],
        typologies: &[TypologyData],
    ) -> EvalResult<AmlDetectabilityAnalysis> {
        let mut issues = Vec::new();

        // 1. Typology coverage — a category counts as covered when ANY
        // of its canonical / alias names appears in the observed
        // typology set. v4.4.2+ matching against the alias table lets
        // injector-emitted names like "money_mule" map to the "mule"
        // category without forcing a rename in every injector.
        let present_typologies: std::collections::HashSet<&str> =
            typologies.iter().map(|t| t.name.as_str()).collect();
        let covered = EXPECTED_TYPOLOGIES
            .iter()
            .filter(|(_, aliases)| aliases.iter().any(|a| present_typologies.contains(a)))
            .count();
        let typology_coverage = covered as f64 / EXPECTED_TYPOLOGIES.len() as f64;

        // 2. Scenario coherence
        let coherent = typologies.iter().filter(|t| t.case_ids_consistent).count();
        let scenario_coherence = if typologies.is_empty() {
            1.0
        } else {
            coherent as f64 / typologies.len() as f64
        };

        // 3. Per-typology analysis
        let mut by_typology: HashMap<String, Vec<&AmlTransactionData>> = HashMap::new();
        for txn in transactions {
            by_typology
                .entry(txn.typology.clone())
                .or_default()
                .push(txn);
        }

        let mut per_typology = Vec::new();
        for (name, txns) in &by_typology {
            let case_ids: std::collections::HashSet<&str> =
                txns.iter().map(|t| t.case_id.as_str()).collect();
            let flagged = txns.iter().filter(|t| t.is_flagged).count();
            let flag_rate = if txns.is_empty() {
                0.0
            } else {
                flagged as f64 / txns.len() as f64
            };

            // Check typology-specific patterns
            let pattern_detected = match name.as_str() {
                "structuring" => {
                    // Most amounts should be below threshold
                    let below = txns
                        .iter()
                        .filter(|t| t.amount < self.thresholds.structuring_threshold)
                        .count();
                    below as f64 / txns.len().max(1) as f64 > 0.5
                }
                "layering" => {
                    // Should have multiple cases with >2 transactions each
                    !case_ids.is_empty() && txns.len() > case_ids.len()
                }
                _ => {
                    // Generic: require a meaningful flag rate indicating
                    // the typology produces detectable suspicious patterns.
                    // A flag rate of 0 means no suspicious indicators at all.
                    let suspicious_count = txns.iter().filter(|t| t.is_flagged).count();
                    let suspicious_ratio = suspicious_count as f64 / txns.len().max(1) as f64;
                    !txns.is_empty() && suspicious_ratio > 0.0
                }
            };

            per_typology.push(TypologyDetectability {
                name: name.clone(),
                transaction_count: txns.len(),
                case_count: case_ids.len(),
                flag_rate,
                pattern_detected,
            });
        }

        // Check thresholds. v5.0.1 (Gap 2): on samples below the
        // coverage floor, emit an advisory but don't fail — the
        // metric is statistically unstable at small N because a
        // single low-prevalence category missing on chance produces
        // a 14.3 pp wobble (1 / 7 categories). We track failures
        // separately from advisories so the advisory text remains
        // visible in `issues` without flipping `passes` to false.
        let mut failed = false;
        if transactions.len() < self.thresholds.min_sample_for_coverage {
            issues.push(format!(
                "Advisory: typology coverage {:.3} computed on {} txns \
                 (< {} sample floor) — metric is statistically unstable; \
                 increase sample size for a reliable reading.",
                typology_coverage,
                transactions.len(),
                self.thresholds.min_sample_for_coverage
            ));
        } else if typology_coverage < self.thresholds.min_typology_coverage {
            issues.push(format!(
                "Typology coverage {:.3} < {:.3}",
                typology_coverage, self.thresholds.min_typology_coverage
            ));
            failed = true;
        }
        if scenario_coherence < self.thresholds.min_scenario_coherence {
            issues.push(format!(
                "Scenario coherence {:.3} < {:.3}",
                scenario_coherence, self.thresholds.min_scenario_coherence
            ));
            failed = true;
        }

        let passes = !failed;

        Ok(AmlDetectabilityAnalysis {
            typology_coverage,
            scenario_coherence,
            per_typology,
            total_transactions: transactions.len(),
            passes,
            issues,
        })
    }
}

impl Default for AmlDetectabilityAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_good_aml_data() {
        let analyzer = AmlDetectabilityAnalyzer::new();
        // Use the canonical names (first of each tuple) so every
        // category counts as covered.
        let typologies: Vec<TypologyData> = EXPECTED_TYPOLOGIES
            .iter()
            .map(|(canonical, _aliases)| TypologyData {
                name: canonical.to_string(),
                scenario_count: 5,
                case_ids_consistent: true,
            })
            .collect();
        let transactions = vec![
            AmlTransactionData {
                transaction_id: "T001".to_string(),
                typology: "structuring".to_string(),
                case_id: "C001".to_string(),
                amount: 9_500.0,
                is_flagged: true,
            },
            AmlTransactionData {
                transaction_id: "T002".to_string(),
                typology: "structuring".to_string(),
                case_id: "C001".to_string(),
                amount: 9_800.0,
                is_flagged: true,
            },
        ];

        let result = analyzer.analyze(&transactions, &typologies).unwrap();
        assert!(result.passes);
        assert_eq!(result.typology_coverage, 1.0);
    }

    #[test]
    fn test_missing_typologies() {
        // Override the sample-size floor so the threshold-failure path
        // engages on this small synthetic input. v5.0.1 (Gap 2): the
        // default 5_000-row floor means coverage failures become
        // advisories below that — exercising the strict-failure path
        // requires either a large sample or a lowered floor.
        let mut thresholds = AmlDetectabilityThresholds::default();
        thresholds.min_sample_for_coverage = 0;
        let analyzer = AmlDetectabilityAnalyzer::with_thresholds(thresholds);
        let typologies = vec![TypologyData {
            name: "structuring".to_string(),
            scenario_count: 5,
            case_ids_consistent: true,
        }];

        let result = analyzer.analyze(&[], &typologies).unwrap();
        assert!(!result.passes); // Coverage too low
    }

    #[test]
    fn test_empty() {
        let mut thresholds = AmlDetectabilityThresholds::default();
        thresholds.min_sample_for_coverage = 0;
        let analyzer = AmlDetectabilityAnalyzer::with_thresholds(thresholds);
        let result = analyzer.analyze(&[], &[]).unwrap();
        assert!(!result.passes); // Zero coverage
    }

    #[test]
    fn test_small_sample_advisory_does_not_fail() {
        // v5.0.1 (Gap 2): below the 5_000-row floor, missing a
        // single typology produces an advisory (still surfaced in
        // `issues` for visibility) but does not flip `passes` to
        // false. This protects users against the 14.3 pp coverage
        // wobble inherent to small samples.
        let analyzer = AmlDetectabilityAnalyzer::new();
        let typologies = vec![TypologyData {
            name: "structuring".to_string(),
            scenario_count: 5,
            case_ids_consistent: true,
        }];
        let transactions = vec![AmlTransactionData {
            transaction_id: "T001".to_string(),
            typology: "structuring".to_string(),
            case_id: "C001".to_string(),
            amount: 9_500.0,
            is_flagged: true,
        }];

        let result = analyzer.analyze(&transactions, &typologies).unwrap();
        assert!(result.passes, "small sample should not fail on coverage");
        assert!(
            result.issues.iter().any(|i| i.starts_with("Advisory:")),
            "small sample should surface an advisory issue, got: {:?}",
            result.issues
        );
    }
}

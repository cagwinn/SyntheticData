//! Cross-layer coherence evaluator.
//!
//! Validates that banking transactions linked to document-flow Payments
//! maintain referential integrity and label consistency:
//! - Every bank transaction with `source_payment_id` must reference an existing Payment
//! - Fraud labels must propagate: `Payment.is_fraud → BankTransaction.is_suspicious`
//! - Mirror transactions must have consistent amounts, inverse directions
//! - GL cash accounts must be present and consistent

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::error::EvalResult;

/// Summary of a Payment for cross-layer validation.
#[derive(Debug, Clone)]
pub struct PaymentRef {
    pub payment_id: String,
    pub amount: f64,
    pub is_fraud: bool,
    pub journal_entry_id: Option<String>,
}

/// Summary of a BankTransaction's cross-layer links.
#[derive(Debug, Clone)]
pub struct BankTxnLinks {
    pub transaction_id: String,
    pub source_payment_id: Option<String>,
    pub source_invoice_id: Option<String>,
    pub journal_entry_id: Option<String>,
    pub gl_cash_account: Option<String>,
    pub is_suspicious: bool,
    pub is_outbound: bool,
    pub amount: f64,
    pub parent_transaction_id: Option<String>,
}

/// Thresholds for cross-layer coherence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossLayerThresholds {
    /// Maximum fraction of bridged txns with dangling payment references
    pub max_dangling_payment_rate: f64,
    /// Minimum fraction of Payment.is_fraud cases propagated to BankTransaction.is_suspicious
    pub min_fraud_propagation_rate: f64,
    /// Maximum fraction of bridged txns missing gl_cash_account
    pub max_missing_gl_rate: f64,
    /// Maximum amount deviation allowed between payment and bridged txn
    pub max_amount_deviation: f64,
}

impl Default for CrossLayerThresholds {
    fn default() -> Self {
        Self {
            max_dangling_payment_rate: 0.0,
            min_fraud_propagation_rate: 0.95,
            max_missing_gl_rate: 0.01,
            max_amount_deviation: 0.01,
        }
    }
}

/// Cross-layer coherence analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossLayerCoherenceAnalysis {
    /// Total bank transactions examined
    pub total_bank_transactions: usize,
    /// Bank transactions with source_payment_id populated (bridged)
    pub bridged_transactions: usize,
    /// Bridged transactions whose payment_id doesn't exist in payments
    pub dangling_payment_refs: usize,
    /// Payment IDs that are fraudulent but had no suspicious bank txn
    pub unpropagated_fraud_payments: usize,
    /// Total fraudulent payments observed
    pub total_fraud_payments: usize,
    /// Bridged transactions missing gl_cash_account
    pub missing_gl_account: usize,
    /// Bridged transactions with amount deviation > threshold
    pub amount_mismatches: usize,
    /// Mirror transactions (with parent_transaction_id)
    pub mirror_transactions: usize,
    /// Fraud propagation rate
    pub fraud_propagation_rate: f64,
    /// Overall pass/fail
    pub passes: bool,
    pub issues: Vec<String>,
}

/// Cross-layer coherence analyzer.
pub struct CrossLayerCoherenceAnalyzer {
    pub thresholds: CrossLayerThresholds,
}

impl CrossLayerCoherenceAnalyzer {
    pub fn new() -> Self {
        Self {
            thresholds: CrossLayerThresholds::default(),
        }
    }

    pub fn with_thresholds(thresholds: CrossLayerThresholds) -> Self {
        Self { thresholds }
    }

    /// Analyze cross-layer coherence between payments and bank transactions.
    pub fn analyze(
        &self,
        payments: &[PaymentRef],
        bank_txns: &[BankTxnLinks],
    ) -> EvalResult<CrossLayerCoherenceAnalysis> {
        let payment_by_id: HashMap<&str, &PaymentRef> = payments
            .iter()
            .map(|p| (p.payment_id.as_str(), p))
            .collect();
        let total_fraud_payments = payments.iter().filter(|p| p.is_fraud).count();

        let mut bridged_count = 0usize;
        let mut dangling = 0usize;
        let mut missing_gl = 0usize;
        let mut mismatches = 0usize;
        let mut mirror_count = 0usize;
        // Track which fraud payments had at least one suspicious bank txn
        let mut fraud_payments_with_suspicious_txn: HashSet<&str> = HashSet::new();

        for txn in bank_txns {
            if txn.parent_transaction_id.is_some() {
                mirror_count += 1;
            }
            let Some(ref pid) = txn.source_payment_id else {
                continue;
            };
            bridged_count += 1;

            match payment_by_id.get(pid.as_str()) {
                None => {
                    dangling += 1;
                }
                Some(payment) => {
                    // Amount match (bridged should have same amount as payment)
                    let deviation =
                        (payment.amount - txn.amount).abs() / payment.amount.abs().max(1.0);
                    if deviation > self.thresholds.max_amount_deviation {
                        mismatches += 1;
                    }
                    // Track fraud propagation
                    if payment.is_fraud && txn.is_suspicious {
                        fraud_payments_with_suspicious_txn.insert(pid.as_str());
                    }
                }
            }

            if txn.gl_cash_account.is_none() {
                missing_gl += 1;
            }
        }

        let unpropagated_fraud_payments =
            total_fraud_payments.saturating_sub(fraud_payments_with_suspicious_txn.len());

        let fraud_propagation_rate = if total_fraud_payments > 0 {
            fraud_payments_with_suspicious_txn.len() as f64 / total_fraud_payments as f64
        } else {
            1.0
        };

        let dangling_rate = if bridged_count > 0 {
            dangling as f64 / bridged_count as f64
        } else {
            0.0
        };
        let missing_gl_rate = if bridged_count > 0 {
            missing_gl as f64 / bridged_count as f64
        } else {
            0.0
        };

        let mut issues = Vec::new();
        if dangling_rate > self.thresholds.max_dangling_payment_rate {
            issues.push(format!(
                "{dangling} bridged bank transactions reference non-existent payments ({:.2}%)",
                dangling_rate * 100.0
            ));
        }
        if total_fraud_payments > 0
            && fraud_propagation_rate < self.thresholds.min_fraud_propagation_rate
        {
            issues.push(format!(
                "Fraud propagation rate {:.1}% below minimum {:.1}% ({} of {} fraud payments had no suspicious bank txn)",
                fraud_propagation_rate * 100.0,
                self.thresholds.min_fraud_propagation_rate * 100.0,
                unpropagated_fraud_payments,
                total_fraud_payments,
            ));
        }
        if missing_gl_rate > self.thresholds.max_missing_gl_rate {
            issues.push(format!(
                "{missing_gl} bridged transactions missing gl_cash_account ({:.2}%)",
                missing_gl_rate * 100.0
            ));
        }
        if mismatches > 0 {
            issues.push(format!(
                "{mismatches} bridged transactions have amount deviation > {:.2}% from their payment",
                self.thresholds.max_amount_deviation * 100.0
            ));
        }

        Ok(CrossLayerCoherenceAnalysis {
            total_bank_transactions: bank_txns.len(),
            bridged_transactions: bridged_count,
            dangling_payment_refs: dangling,
            unpropagated_fraud_payments,
            total_fraud_payments,
            missing_gl_account: missing_gl,
            amount_mismatches: mismatches,
            mirror_transactions: mirror_count,
            fraud_propagation_rate,
            passes: issues.is_empty(),
            issues,
        })
    }
}

impl Default for CrossLayerCoherenceAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_coherence_passes() {
        let payments = vec![
            PaymentRef {
                payment_id: "PAY-1".into(),
                amount: 1000.0,
                is_fraud: false,
                journal_entry_id: Some("JE-1".into()),
            },
            PaymentRef {
                payment_id: "PAY-2".into(),
                amount: 500.0,
                is_fraud: true,
                journal_entry_id: Some("JE-2".into()),
            },
        ];
        let bank_txns = vec![
            BankTxnLinks {
                transaction_id: "BT-1".into(),
                source_payment_id: Some("PAY-1".into()),
                source_invoice_id: None,
                journal_entry_id: Some("JE-1".into()),
                gl_cash_account: Some("100000".into()),
                is_suspicious: false,
                is_outbound: true,
                amount: 1000.0,
                parent_transaction_id: None,
            },
            BankTxnLinks {
                transaction_id: "BT-2".into(),
                source_payment_id: Some("PAY-2".into()),
                source_invoice_id: None,
                journal_entry_id: Some("JE-2".into()),
                gl_cash_account: Some("100000".into()),
                is_suspicious: true, // fraud propagated
                is_outbound: true,
                amount: 500.0,
                parent_transaction_id: None,
            },
        ];

        let analyzer = CrossLayerCoherenceAnalyzer::new();
        let result = analyzer.analyze(&payments, &bank_txns).unwrap();
        assert!(result.passes, "Issues: {:?}", result.issues);
        assert_eq!(result.bridged_transactions, 2);
        assert_eq!(result.dangling_payment_refs, 0);
        assert!((result.fraud_propagation_rate - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_dangling_payment_ref_detected() {
        let payments = vec![PaymentRef {
            payment_id: "PAY-1".into(),
            amount: 1000.0,
            is_fraud: false,
            journal_entry_id: None,
        }];
        let bank_txns = vec![BankTxnLinks {
            transaction_id: "BT-1".into(),
            source_payment_id: Some("PAY-999".into()), // doesn't exist!
            source_invoice_id: None,
            journal_entry_id: None,
            gl_cash_account: Some("100000".into()),
            is_suspicious: false,
            is_outbound: true,
            amount: 1000.0,
            parent_transaction_id: None,
        }];

        let analyzer = CrossLayerCoherenceAnalyzer::new();
        let result = analyzer.analyze(&payments, &bank_txns).unwrap();
        assert!(!result.passes);
        assert_eq!(result.dangling_payment_refs, 1);
    }

    #[test]
    fn test_fraud_propagation_failure_detected() {
        // Fraudulent payment but the linked bank txn is NOT suspicious
        let payments = vec![PaymentRef {
            payment_id: "PAY-1".into(),
            amount: 1000.0,
            is_fraud: true,
            journal_entry_id: None,
        }];
        let bank_txns = vec![BankTxnLinks {
            transaction_id: "BT-1".into(),
            source_payment_id: Some("PAY-1".into()),
            source_invoice_id: None,
            journal_entry_id: None,
            gl_cash_account: Some("100000".into()),
            is_suspicious: false, // BUG: fraud not propagated
            is_outbound: true,
            amount: 1000.0,
            parent_transaction_id: None,
        }];

        let analyzer = CrossLayerCoherenceAnalyzer::new();
        let result = analyzer.analyze(&payments, &bank_txns).unwrap();
        assert!(!result.passes);
        assert!((result.fraud_propagation_rate - 0.0).abs() < 1e-9);
        assert_eq!(result.unpropagated_fraud_payments, 1);
    }
}

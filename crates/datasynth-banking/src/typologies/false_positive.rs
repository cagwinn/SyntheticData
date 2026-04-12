//! False positive injection for realistic AML model training.
//!
//! Tags a configurable fraction of legitimate transactions as "suspicious-looking
//! but actually clean" — the inverse of true positives. This is critical because
//! real AML systems have 95%+ false positive rates.

use chrono::Timelike;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::models::BankTransaction;
use crate::seed_offsets::FALSE_POSITIVE_SEED_OFFSET;

/// False positive injector.
pub struct FalsePositiveInjector {
    rng: ChaCha8Rng,
}

impl FalsePositiveInjector {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(FALSE_POSITIVE_SEED_OFFSET)),
        }
    }

    /// Inject false positive flags on legitimate transactions.
    ///
    /// Selects transactions that look suspicious based on heuristics
    /// (near-threshold, high amount, international, round numbers, off-hours)
    /// but leaves `is_suspicious = false` (the ground truth is clean).
    pub fn inject(&mut self, transactions: &mut [BankTransaction], fp_rate: f64) {
        if fp_rate <= 0.0 {
            return;
        }

        // Only consider non-suspicious transactions as FP candidates
        let candidate_indices: Vec<usize> = transactions
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.is_suspicious)
            .map(|(i, _)| i)
            .collect();

        let target_count = (candidate_indices.len() as f64 * fp_rate) as usize;
        if target_count == 0 {
            return;
        }

        // Score each candidate by how "suspicious-looking" it is
        let mut scored: Vec<(usize, f64, String)> = candidate_indices
            .into_iter()
            .map(|idx| {
                let txn = &transactions[idx];
                let (score, reason) = self.suspicion_score(txn);
                (idx, score, reason)
            })
            .filter(|(_, score, _)| *score > 0.0)
            .collect();

        // Sort by suspicion score descending — tag the most suspicious-looking first
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        for (idx, _score, reason) in scored.into_iter().take(target_count) {
            transactions[idx].is_false_positive = true;
            transactions[idx].false_positive_reason = Some(reason);
        }
    }

    /// Score how suspicious a legitimate transaction looks (0.0 = normal, 1.0+ = very suspicious-looking).
    fn suspicion_score(&self, txn: &BankTransaction) -> (f64, String) {
        let mut score = 0.0;
        let mut reasons = Vec::new();
        let amount_f = txn.amount.to_f64().unwrap_or(0.0);

        // Near CTR threshold ($8,000 - $10,000)
        if (8_000.0..10_000.0).contains(&amount_f) {
            score += 0.4;
            reasons.push(format!("near-threshold amount ${:.0}", amount_f));
        }

        // Large round number
        if amount_f > 1_000.0 && amount_f % 1_000.0 == 0.0 {
            score += 0.2;
            reasons.push(format!("round amount ${:.0}", amount_f));
        }

        // International transfer
        if txn.counterparty.country.is_some() && txn.counterparty.country.as_deref() != Some("US") {
            score += 0.3;
            if let Some(ref country) = txn.counterparty.country {
                reasons.push(format!("cross-border to {country}"));
            }
        }

        // Large amount (> $5,000)
        if amount_f > 5_000.0 {
            score += 0.15;
        }

        // Off-hours transaction (before 7am or after 10pm)
        let hour = txn.timestamp_initiated.time().hour();
        if hour < 7 || hour >= 22 {
            score += 0.2;
            reasons.push("off-hours timing".to_string());
        }

        // Cash transaction
        if txn.is_cash() {
            score += 0.15;
            reasons.push("cash transaction".to_string());
        }

        let reason = if reasons.is_empty() {
            "suspicious pattern combination".to_string()
        } else {
            reasons.join("; ")
        };

        (score, reason)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use datasynth_core::banking::{Direction, TransactionCategory, TransactionChannel};
    use rust_decimal_macros::dec;
    use uuid::Uuid;

    use crate::models::CounterpartyRef;

    #[test]
    fn test_false_positive_injection() {
        let mut injector = FalsePositiveInjector::new(42);

        // Create some normal transactions
        let mut txns: Vec<BankTransaction> = (0..100)
            .map(|i| {
                let amount = if i % 10 == 0 { dec!(9500) } else { dec!(150) };
                BankTransaction::new(
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    amount,
                    "USD",
                    Direction::Outbound,
                    TransactionChannel::CardPresent,
                    TransactionCategory::Shopping,
                    CounterpartyRef::merchant(Uuid::new_v4(), "Store"),
                    "Purchase",
                    Utc.with_ymd_and_hms(2024, 3, 15, 14, 0, 0).unwrap(),
                )
            })
            .collect();

        injector.inject(&mut txns, 0.10);

        let fp_count = txns.iter().filter(|t| t.is_false_positive).count();
        assert!(fp_count > 0, "Should tag some false positives");
        assert!(fp_count <= 15, "Should not over-tag");

        // All FPs should NOT be suspicious (ground truth is clean)
        for t in &txns {
            if t.is_false_positive {
                assert!(!t.is_suspicious, "FP should not be marked suspicious");
                assert!(t.false_positive_reason.is_some(), "FP should have a reason");
            }
        }
    }
}

//! Post-generation velocity feature computation.
//!
//! Runs after all transactions (including typology-injected ones) are generated.
//! Computes rolling-window features for each transaction based on the account's
//! transaction history up to that point.

use std::collections::{HashMap, HashSet};

use chrono::Duration;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::models::{BankTransaction, VelocityFeatures};

/// Compute velocity features for all transactions.
///
/// Transactions are grouped by account, sorted by timestamp, and each
/// transaction receives features computed from its preceding history.
pub fn compute_velocity_features(transactions: &mut [BankTransaction]) {
    // Group transaction indices by account_id
    let mut account_txns: HashMap<uuid::Uuid, Vec<usize>> = HashMap::new();
    for (idx, txn) in transactions.iter().enumerate() {
        account_txns
            .entry(txn.account_id)
            .or_default()
            .push(idx);
    }

    // Process each account's transactions in chronological order
    for (_account_id, indices) in &account_txns {
        // Sort indices by timestamp
        let mut sorted_indices: Vec<usize> = indices.clone();
        sorted_indices.sort_by_key(|&i| transactions[i].timestamp_initiated);

        // Sliding window computation
        for (pos, &current_idx) in sorted_indices.iter().enumerate() {
            let current_ts = transactions[current_idx].timestamp_initiated;
            let current_amount = transactions[current_idx].amount;

            let window_1h = current_ts - Duration::hours(1);
            let window_24h = current_ts - Duration::hours(24);
            let window_7d = current_ts - Duration::days(7);
            let window_30d = current_ts - Duration::days(30);

            let mut count_1h = 0u32;
            let mut count_24h = 0u32;
            let mut count_7d = 0u32;
            let mut count_30d = 0u32;
            let mut sum_24h = Decimal::ZERO;
            let mut sum_7d = Decimal::ZERO;
            let mut sum_30d = Decimal::ZERO;
            let mut max_24h = Decimal::ZERO;
            let mut counterparties_24h = HashSet::new();
            let mut counterparties_7d = HashSet::new();
            let mut countries_7d = HashSet::new();
            let mut amounts_30d = Vec::new();

            // Look back through preceding transactions
            for &prev_idx in sorted_indices[..pos].iter().rev() {
                let prev_ts = transactions[prev_idx].timestamp_initiated;
                let prev_amount = transactions[prev_idx].amount;

                if prev_ts < window_30d {
                    break; // Beyond 30d window, stop
                }

                count_30d += 1;
                sum_30d += prev_amount;
                amounts_30d.push(prev_amount);

                if prev_ts >= window_7d {
                    count_7d += 1;
                    sum_7d += prev_amount;
                    if let Some(ref cp) = transactions[prev_idx].counterparty.counterparty_id {
                        counterparties_7d.insert(*cp);
                    }
                    if let Some(ref country) = transactions[prev_idx].location_country {
                        countries_7d.insert(country.clone());
                    }

                    if prev_ts >= window_24h {
                        count_24h += 1;
                        sum_24h += prev_amount;
                        if prev_amount > max_24h {
                            max_24h = prev_amount;
                        }
                        if let Some(ref cp) = transactions[prev_idx].counterparty.counterparty_id {
                            counterparties_24h.insert(*cp);
                        }

                        if prev_ts >= window_1h {
                            count_1h += 1;
                        }
                    }
                }
            }

            // Compute statistical features
            let avg_30d = if count_30d > 0 {
                sum_30d / Decimal::from(count_30d)
            } else {
                Decimal::ZERO
            };

            let std_30d = if amounts_30d.len() >= 2 {
                let mean_f = avg_30d.to_f64().unwrap_or(0.0);
                let variance: f64 = amounts_30d
                    .iter()
                    .map(|a| {
                        let d = a.to_f64().unwrap_or(0.0) - mean_f;
                        d * d
                    })
                    .sum::<f64>()
                    / (amounts_30d.len() as f64 - 1.0);
                variance.sqrt()
            } else {
                0.0
            };

            let amount_zscore = if std_30d > 0.0 {
                (current_amount.to_f64().unwrap_or(0.0) - avg_30d.to_f64().unwrap_or(0.0)) / std_30d
            } else {
                0.0
            };

            transactions[current_idx].velocity_features = Some(VelocityFeatures {
                txn_count_1h: count_1h,
                txn_count_24h: count_24h,
                txn_count_7d: count_7d,
                txn_count_30d: count_30d,
                amount_sum_24h: sum_24h,
                amount_sum_7d: sum_7d,
                amount_sum_30d: sum_30d,
                amount_max_24h: max_24h,
                unique_counterparties_24h: counterparties_24h.len() as u16,
                unique_counterparties_7d: counterparties_7d.len() as u16,
                unique_countries_7d: countries_7d.len() as u16,
                avg_amount_30d: avg_30d,
                std_amount_30d: std_30d,
                amount_zscore,
            });
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use rust_decimal_macros::dec;
    use uuid::Uuid;

    use crate::models::CounterpartyRef;
    use datasynth_core::banking::{Direction, TransactionCategory, TransactionChannel};

    fn make_txn(account_id: Uuid, amount: Decimal, ts: chrono::DateTime<Utc>) -> BankTransaction {
        BankTransaction::new(
            Uuid::new_v4(),
            account_id,
            amount,
            "USD",
            Direction::Outbound,
            TransactionChannel::CardPresent,
            TransactionCategory::Shopping,
            CounterpartyRef::merchant(Uuid::new_v4(), "Test Merchant"),
            "test",
            ts,
        )
    }

    #[test]
    fn test_velocity_populated() {
        let acct = Uuid::new_v4();
        let base = Utc.with_ymd_and_hms(2024, 3, 15, 10, 0, 0).unwrap();
        let mut txns = vec![
            make_txn(acct, dec!(100), base),
            make_txn(acct, dec!(200), base + Duration::hours(2)),
            make_txn(acct, dec!(300), base + Duration::hours(3)),
        ];
        compute_velocity_features(&mut txns);

        // First transaction has no history
        let v0 = txns[0].velocity_features.as_ref().unwrap();
        assert_eq!(v0.txn_count_24h, 0);

        // Third transaction sees two predecessors within 24h
        let v2 = txns[2].velocity_features.as_ref().unwrap();
        assert_eq!(v2.txn_count_24h, 2);
        assert_eq!(v2.amount_sum_24h, dec!(300)); // 100 + 200
    }

    #[test]
    fn test_zscore_computed() {
        let acct = Uuid::new_v4();
        let base = Utc.with_ymd_and_hms(2024, 3, 1, 10, 0, 0).unwrap();
        // Use varied amounts so std > 0
        let amounts = [dec!(80), dec!(120), dec!(95), dec!(110), dec!(90),
                       dec!(105), dec!(115), dec!(85), dec!(100), dec!(100)];
        let mut txns: Vec<BankTransaction> = amounts
            .iter()
            .enumerate()
            .map(|(i, &amt)| make_txn(acct, amt, base + Duration::days(i as i64)))
            .collect();
        // Add an outlier
        txns.push(make_txn(acct, dec!(10000), base + Duration::days(10)));

        compute_velocity_features(&mut txns);

        let outlier = txns.last().unwrap().velocity_features.as_ref().unwrap();
        assert!(outlier.amount_zscore > 2.0, "Outlier should have high z-score, got {}", outlier.amount_zscore);
    }
}

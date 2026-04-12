//! Synthetic identity fraud typology.
//!
//! Pattern: fabricated identity passes KYC → small legitimate "seasoning"
//! transactions build credit → bust-out with large purchases/cash advances.

use chrono::{Duration, NaiveDate};
use datasynth_core::models::banking::{
    AmlTypology, Direction, LaunderingStage, Sophistication, TransactionCategory,
    TransactionChannel,
};
use datasynth_core::DeterministicUuidFactory;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rust_decimal::Decimal;

use crate::models::{BankAccount, BankTransaction, BankingCustomer, CounterpartyRef};
use crate::seed_offsets::SYNTHETIC_IDENTITY_SEED_OFFSET;

/// Synthetic identity fraud injector.
pub struct SyntheticIdentityInjector {
    rng: ChaCha8Rng,
    uuid_factory: DeterministicUuidFactory,
}

impl SyntheticIdentityInjector {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(SYNTHETIC_IDENTITY_SEED_OFFSET)),
            uuid_factory: DeterministicUuidFactory::new(seed, datasynth_core::GeneratorType::Anomaly),
        }
    }

    /// Generate a synthetic identity bust-out scenario.
    pub fn generate(
        &mut self,
        _customer: &BankingCustomer,
        account: &BankAccount,
        start_date: NaiveDate,
        end_date: NaiveDate,
        sophistication: Sophistication,
    ) -> Vec<BankTransaction> {
        let mut transactions = Vec::new();
        let scenario_id = format!("SYN-{:06}", self.rng.random::<u32>());

        // Seasoning period: small legitimate transactions
        let seasoning_days = match sophistication {
            Sophistication::Basic => 30,
            Sophistication::Standard => 60,
            Sophistication::Professional => 90,
            Sophistication::Advanced => 120,
            Sophistication::StateLevel => 180,
        };

        // Bust-out parameters
        let bust_out_txns = match sophistication {
            Sophistication::Basic => self.rng.random_range(2..4),
            Sophistication::Standard => self.rng.random_range(3..6),
            Sophistication::Professional => self.rng.random_range(5..10),
            Sophistication::Advanced => self.rng.random_range(8..15),
            Sophistication::StateLevel => self.rng.random_range(12..25),
        };

        let bust_out_amount: f64 = match sophistication {
            Sophistication::Basic => self.rng.random_range(5_000.0..15_000.0),
            Sophistication::Standard => self.rng.random_range(10_000.0..30_000.0),
            Sophistication::Professional => self.rng.random_range(25_000.0..75_000.0),
            Sophistication::Advanced => self.rng.random_range(50_000.0..150_000.0),
            Sophistication::StateLevel => self.rng.random_range(100_000.0..500_000.0),
        };

        let available_days = (end_date - start_date).num_days().max(1) as i64;
        let actual_seasoning = (seasoning_days as i64).min(available_days * 2 / 3);
        let seq = &mut 0u32;

        // Phase 1: Seasoning — small legitimate transactions
        let seasoning_count = actual_seasoning as u32 / 5; // ~1 txn per 5 days
        for i in 0..seasoning_count {
            let day_offset = self.rng.random_range(0..actual_seasoning.max(1) as u32);
            let date = start_date + Duration::days(day_offset as i64);
            if date > end_date {
                continue;
            }
            let amount = self.rng.random_range(20.0..200.0);
            let hour = self.rng.random_range(9..18);
            let ts = date
                .and_hms_opt(hour, self.rng.random_range(0..60), 0)
                .map(|dt| dt.and_utc())
                .unwrap_or_else(|| date.and_hms_opt(12, 0, 0).expect("valid time").and_utc());

            let mut txn = BankTransaction::new(
                self.uuid_factory.next(),
                account.account_id,
                Decimal::from_f64_retain(amount).unwrap_or(Decimal::ONE_HUNDRED),
                &account.currency,
                Direction::Outbound,
                TransactionChannel::CardNotPresent,
                TransactionCategory::Shopping,
                CounterpartyRef::merchant(self.uuid_factory.next(), "Online Store"),
                "Regular purchase",
                ts,
            );
            txn = txn.mark_suspicious(AmlTypology::SyntheticIdentity, &scenario_id);
            txn = txn.with_laundering_stage(LaunderingStage::Placement);
            txn = txn.with_scenario(&scenario_id, *seq);
            txn.ground_truth_explanation = Some(format!(
                "Synthetic identity seasoning txn #{} of {}: ${:.2} purchase building credit history",
                i + 1,
                seasoning_count,
                amount,
            ));
            *seq += 1;
            transactions.push(txn);
        }

        // Phase 2: Bust-out — large purchases/cash advances
        let bust_start = start_date + Duration::days(actual_seasoning);
        let bust_days = 3.max((available_days - actual_seasoning).min(7));
        let per_txn_amount = bust_out_amount / bust_out_txns as f64;

        for i in 0..bust_out_txns {
            let day_offset = self.rng.random_range(0..bust_days as u32);
            let date = bust_start + Duration::days(day_offset as i64);
            if date > end_date {
                continue;
            }
            let amount = per_txn_amount * self.rng.random_range(0.7..1.3);
            let hour = self.rng.random_range(0..24);
            let ts = date
                .and_hms_opt(hour, self.rng.random_range(0..60), 0)
                .map(|dt| dt.and_utc())
                .unwrap_or_else(|| date.and_hms_opt(12, 0, 0).expect("valid time").and_utc());

            // Alternate between channels for bust-out
            let (channel, category) = if i % 3 == 0 {
                (TransactionChannel::Atm, TransactionCategory::AtmWithdrawal)
            } else if i % 3 == 1 {
                (TransactionChannel::Wire, TransactionCategory::TransferOut)
            } else {
                (TransactionChannel::CardNotPresent, TransactionCategory::Shopping)
            };

            let mut txn = BankTransaction::new(
                self.uuid_factory.next(),
                account.account_id,
                Decimal::from_f64_retain(amount).unwrap_or(Decimal::ONE_HUNDRED),
                &account.currency,
                Direction::Outbound,
                channel,
                category,
                CounterpartyRef::unknown("Unknown"),
                "Bust-out extraction",
                ts,
            );
            txn = txn.mark_suspicious(AmlTypology::SyntheticIdentity, &scenario_id);
            txn = txn.with_laundering_stage(LaunderingStage::Integration);
            txn = txn.with_scenario(&scenario_id, *seq);
            txn.ground_truth_explanation = Some(format!(
                "Synthetic identity bust-out #{} of {}: ${:.2} extraction after {}d seasoning",
                i + 1,
                bust_out_txns,
                amount,
                actual_seasoning,
            ));
            *seq += 1;
            transactions.push(txn);
        }

        transactions
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_synthetic_identity_generates_transactions() {
        let mut injector = SyntheticIdentityInjector::new(42);
        let customer = BankingCustomer::new_retail(Uuid::new_v4(), "Fake", "Person", "US",
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
        let account = BankAccount::new(Uuid::new_v4(), "ACC-001".into(),
            datasynth_core::models::banking::BankAccountType::Checking,
            customer.customer_id, "USD", NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());

        let txns = injector.generate(&customer, &account,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            Sophistication::Standard);

        assert!(!txns.is_empty(), "Should generate transactions");
        assert!(txns.iter().all(|t| t.is_suspicious), "All should be suspicious");
        assert!(txns.iter().all(|t| t.suspicion_reason == Some(AmlTypology::SyntheticIdentity)));
        // Should have both seasoning (small) and bust-out (large) transactions
        let has_small = txns.iter().any(|t| t.amount < Decimal::from(500));
        let has_large = txns.iter().any(|t| t.amount > Decimal::from(1000));
        assert!(has_small, "Should have seasoning transactions");
        assert!(has_large, "Should have bust-out transactions");
        assert!(txns.iter().all(|t| t.ground_truth_explanation.is_some()));
    }
}

//! Pouch activity typology.
//!
//! Pattern: physical cash collection by couriers/pouches, followed by bulk
//! deposits across multiple branch locations. Common in high-cash industries
//! used as fronts (parking lots, convenience stores, restaurants).

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
use crate::seed_offsets::POUCH_ACTIVITY_SEED_OFFSET;

pub struct PouchActivityInjector {
    rng: ChaCha8Rng,
    uuid_factory: DeterministicUuidFactory,
}

impl PouchActivityInjector {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(POUCH_ACTIVITY_SEED_OFFSET)),
            uuid_factory: DeterministicUuidFactory::new(
                seed,
                datasynth_core::GeneratorType::Anomaly,
            ),
        }
    }

    pub fn generate(
        &mut self,
        _customer: &BankingCustomer,
        account: &BankAccount,
        start_date: NaiveDate,
        end_date: NaiveDate,
        sophistication: Sophistication,
    ) -> Vec<BankTransaction> {
        let mut transactions = Vec::new();
        let scenario_id = format!("POU-{:06}", self.rng.random::<u32>());

        let num_pouches = match sophistication {
            Sophistication::Basic => self.rng.random_range(2..5),
            Sophistication::Standard => self.rng.random_range(4..8),
            Sophistication::Professional => self.rng.random_range(6..12),
            Sophistication::Advanced => self.rng.random_range(10..20),
            Sophistication::StateLevel => self.rng.random_range(15..30),
        };

        let cash_per_pouch: f64 = match sophistication {
            Sophistication::Basic => self.rng.random_range(8_000.0..9_500.0),
            Sophistication::Standard => self.rng.random_range(6_000.0..9_000.0),
            Sophistication::Professional => self.rng.random_range(5_000.0..8_500.0),
            Sophistication::Advanced => self.rng.random_range(4_000.0..7_500.0),
            Sophistication::StateLevel => self.rng.random_range(3_000.0..7_000.0),
        };

        let num_branches = match sophistication {
            Sophistication::Basic => 1,
            Sophistication::Standard => 2,
            Sophistication::Professional => self.rng.random_range(3..5),
            Sophistication::Advanced => self.rng.random_range(4..7),
            Sophistication::StateLevel => self.rng.random_range(6..10),
        };

        let available_days = (end_date - start_date).num_days().max(1);
        let mut seq = 0u32;

        for i in 0..num_pouches {
            let day_offset = self.rng.random_range(0..available_days as u32);
            let date = start_date + Duration::days(day_offset as i64);
            if date > end_date {
                continue;
            }
            let branch_id = self.rng.random_range(1..=num_branches);
            let branch_name = format!("Branch-{branch_id:03}");
            let amount = cash_per_pouch * self.rng.random_range(0.9..1.1);
            let ts = date
                .and_hms_opt(
                    self.rng.random_range(9..17),
                    self.rng.random_range(0..60),
                    0,
                )
                .map(|dt| dt.and_utc())
                .unwrap_or_else(chrono::Utc::now);

            let mut txn = BankTransaction::new(
                self.uuid_factory.next(),
                account.account_id,
                Decimal::from_f64_retain(amount).unwrap_or(Decimal::ONE_HUNDRED),
                &account.currency,
                Direction::Inbound,
                TransactionChannel::Cash,
                TransactionCategory::CashDeposit,
                CounterpartyRef::atm(&branch_name),
                &format!("Cash deposit - pouch #{}", i + 1),
                ts,
            );
            txn = txn.mark_suspicious(AmlTypology::PouchActivity, &scenario_id);
            txn = txn.with_laundering_stage(LaunderingStage::Placement);
            txn = txn.with_scenario(&scenario_id, seq);
            txn.ground_truth_explanation = Some(format!(
                "Pouch activity: ${:.0} cash deposit at {branch_name} (pouch {}/{num_pouches}, {num_branches} branches used)",
                amount,
                i + 1,
            ));
            seq += 1;
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
    fn test_pouch_activity_generates_cash_deposits() {
        let mut inj = PouchActivityInjector::new(42);
        let customer = BankingCustomer::new_business(
            Uuid::new_v4(),
            "Cash Store LLC",
            "US",
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        );
        let account = BankAccount::new(
            Uuid::new_v4(),
            "ACC".into(),
            datasynth_core::models::banking::BankAccountType::BusinessOperating,
            customer.customer_id,
            "USD",
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        );

        let txns = inj.generate(
            &customer,
            &account,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap(),
            Sophistication::Professional,
        );

        assert!(!txns.is_empty());
        assert!(txns
            .iter()
            .all(|t| matches!(t.direction, Direction::Inbound)));
        assert!(txns.iter().all(|t| t.channel == TransactionChannel::Cash));
        assert!(txns
            .iter()
            .all(|t| t.suspicion_reason == Some(AmlTypology::PouchActivity)));
    }
}

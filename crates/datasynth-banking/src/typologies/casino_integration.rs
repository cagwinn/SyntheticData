//! Casino integration typology.
//!
//! Pattern: place illicit cash → purchase casino chips → minimal/no play →
//! cash out with check (appears as "gambling winnings" on the back end).
//! Multiple casinos used for sophistication.

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
use crate::seed_offsets::CASINO_INTEGRATION_SEED_OFFSET;

const CASINOS: &[&str] = &[
    "MGM Grand",
    "Caesars Palace",
    "Bellagio",
    "Wynn Las Vegas",
    "The Venetian",
    "Hard Rock Casino",
    "Borgata Atlantic City",
    "Mohegan Sun",
    "Foxwoods",
    "Resorts World",
];

pub struct CasinoIntegrationInjector {
    rng: ChaCha8Rng,
    uuid_factory: DeterministicUuidFactory,
}

impl CasinoIntegrationInjector {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(CASINO_INTEGRATION_SEED_OFFSET)),
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
        let scenario_id = format!("CAS-{:06}", self.rng.random::<u32>());

        let num_cycles = match sophistication {
            Sophistication::Basic => self.rng.random_range(1..3),
            Sophistication::Standard => self.rng.random_range(2..5),
            Sophistication::Professional => self.rng.random_range(3..7),
            Sophistication::Advanced => self.rng.random_range(5..10),
            Sophistication::StateLevel => self.rng.random_range(8..15),
        };

        let num_casinos = match sophistication {
            Sophistication::Basic => 1,
            Sophistication::Standard => self.rng.random_range(1..3),
            Sophistication::Professional => self.rng.random_range(2..4),
            Sophistication::Advanced => self.rng.random_range(3..6),
            Sophistication::StateLevel => self.rng.random_range(5..8),
        };

        let amount_per_cycle: f64 = match sophistication {
            Sophistication::Basic => self.rng.random_range(5_000.0..15_000.0),
            Sophistication::Standard => self.rng.random_range(10_000.0..30_000.0),
            Sophistication::Professional => self.rng.random_range(20_000.0..75_000.0),
            Sophistication::Advanced => self.rng.random_range(50_000.0..200_000.0),
            Sophistication::StateLevel => self.rng.random_range(150_000.0..1_000_000.0),
        };

        let available_days = (end_date - start_date).num_days().max(1);
        let days_per_cycle = (available_days / num_cycles as i64).max(3);
        let mut seq = 0u32;

        for cycle in 0..num_cycles {
            let cycle_start = start_date + Duration::days(cycle as i64 * days_per_cycle);
            if cycle_start > end_date {
                break;
            }

            let casino = CASINOS[self.rng.random_range(0..CASINOS.len().min(num_casinos))];
            let placement_amount = amount_per_cycle * self.rng.random_range(0.85..1.15);

            // Phase 1: Cash/wire to casino (chip purchase)
            let place_date = cycle_start + Duration::days(self.rng.random_range(0..2));
            if place_date <= end_date {
                let ts = place_date
                    .and_hms_opt(
                        self.rng.random_range(10..23),
                        self.rng.random_range(0..60),
                        0,
                    )
                    .map(|dt| dt.and_utc())
                    .unwrap_or_else(chrono::Utc::now);

                let mut txn = BankTransaction::new(
                    self.uuid_factory.next(),
                    account.account_id,
                    Decimal::from_f64_retain(placement_amount).unwrap_or(Decimal::ONE_HUNDRED),
                    &account.currency,
                    Direction::Outbound,
                    TransactionChannel::Wire,
                    TransactionCategory::Entertainment,
                    CounterpartyRef {
                        counterparty_type: crate::models::CounterpartyType::Merchant,
                        counterparty_id: Some(self.uuid_factory.next()),
                        name: casino.to_string(),
                        account_identifier: None,
                        bank_identifier: None,
                        country: Some("US".to_string()),
                    },
                    &format!("Chip purchase - {casino}"),
                    ts,
                );
                txn = txn.mark_suspicious(AmlTypology::CasinoIntegration, &scenario_id);
                txn = txn.with_laundering_stage(LaunderingStage::Placement);
                txn = txn.with_scenario(&scenario_id, seq);
                txn.ground_truth_explanation = Some(format!(
                    "Casino integration: ${:.0} chip purchase at {casino} (cycle {}/{num_cycles})",
                    placement_amount,
                    cycle + 1,
                ));
                seq += 1;
                transactions.push(txn);
            }

            // Phase 2: Check/wire back from casino (minus small "losses")
            let loss_pct = self.rng.random_range(0.02..0.08); // 2-8% "gambling losses"
            let return_amount = placement_amount * (1.0 - loss_pct);
            let return_date = cycle_start + Duration::days(self.rng.random_range(1..7));
            if return_date <= end_date {
                let ts = return_date
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
                    Decimal::from_f64_retain(return_amount).unwrap_or(Decimal::ONE_HUNDRED),
                    &account.currency,
                    Direction::Inbound,
                    TransactionChannel::Check,
                    TransactionCategory::TransferIn,
                    CounterpartyRef {
                        counterparty_type: crate::models::CounterpartyType::Merchant,
                        counterparty_id: None,
                        name: casino.to_string(),
                        account_identifier: None,
                        bank_identifier: None,
                        country: Some("US".to_string()),
                    },
                    &format!("Casino check - {casino}"),
                    ts,
                );
                txn = txn.mark_suspicious(AmlTypology::CasinoIntegration, &scenario_id);
                txn = txn.with_laundering_stage(LaunderingStage::Integration);
                txn = txn.with_scenario(&scenario_id, seq);
                txn.ground_truth_explanation = Some(format!(
                    "Casino integration: ${:.0} 'winnings' check from {casino} ({:.1}% loss - integration complete)",
                    return_amount, loss_pct * 100.0,
                ));
                seq += 1;
                transactions.push(txn);
            }
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
    fn test_casino_integration_paired_txns() {
        let mut inj = CasinoIntegrationInjector::new(42);
        let customer = BankingCustomer::new_retail(
            Uuid::new_v4(),
            "Test",
            "User",
            "US",
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        );
        let account = BankAccount::new(
            Uuid::new_v4(),
            "ACC".into(),
            datasynth_core::models::banking::BankAccountType::Checking,
            customer.customer_id,
            "USD",
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        );

        let txns = inj.generate(
            &customer,
            &account,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            Sophistication::Professional,
        );

        assert!(!txns.is_empty());
        let out = txns
            .iter()
            .filter(|t| matches!(t.direction, Direction::Outbound))
            .count();
        let inb = txns
            .iter()
            .filter(|t| matches!(t.direction, Direction::Inbound))
            .count();
        assert!(
            out > 0 && inb > 0,
            "Should have both placement and integration legs"
        );
    }
}

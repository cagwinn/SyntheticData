//! Cryptocurrency integration typology.
//!
//! Pattern: fiat → crypto exchange (placement) → time gap (off-chain layering)
//! → crypto exchange → fiat (integration). Peel chains use multiple exchanges.

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
use crate::seed_offsets::CRYPTO_INTEGRATION_SEED_OFFSET;

const EXCHANGES: &[&str] = &[
    "Coinbase", "Kraken", "Binance", "Gemini", "Bitstamp",
    "Crypto.com", "OKX", "Bybit", "KuCoin", "Gate.io",
];

/// Cryptocurrency integration injector.
pub struct CryptoIntegrationInjector {
    rng: ChaCha8Rng,
    uuid_factory: DeterministicUuidFactory,
}

impl CryptoIntegrationInjector {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(CRYPTO_INTEGRATION_SEED_OFFSET)),
            uuid_factory: DeterministicUuidFactory::new(seed, datasynth_core::GeneratorType::Anomaly),
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
        let scenario_id = format!("CRY-{:06}", self.rng.random::<u32>());

        let total_amount: f64 = match sophistication {
            Sophistication::Basic => self.rng.random_range(5_000.0..20_000.0),
            Sophistication::Standard => self.rng.random_range(15_000.0..50_000.0),
            Sophistication::Professional => self.rng.random_range(40_000.0..150_000.0),
            Sophistication::Advanced => self.rng.random_range(100_000.0..500_000.0),
            Sophistication::StateLevel => self.rng.random_range(300_000.0..2_000_000.0),
        };

        // Number of exchanges used (peel chain complexity)
        let num_exchanges = match sophistication {
            Sophistication::Basic => 1,
            Sophistication::Standard => 1,
            Sophistication::Professional => self.rng.random_range(2..3),
            Sophistication::Advanced => self.rng.random_range(2..4),
            Sophistication::StateLevel => self.rng.random_range(3..6),
        };

        // Off-chain gap (days between fiat-out and fiat-in)
        let gap_days = match sophistication {
            Sophistication::Basic => self.rng.random_range(1..3),
            Sophistication::Standard => self.rng.random_range(3..7),
            Sophistication::Professional => self.rng.random_range(7..14),
            Sophistication::Advanced => self.rng.random_range(14..30),
            Sophistication::StateLevel => self.rng.random_range(30..60),
        };

        let available_days = (end_date - start_date).num_days().max(1) as i64;
        let mut seq = 0u32;

        // Phase 1: Fiat → Crypto (placement)
        // Split across multiple outbound transfers to exchanges
        let outbound_count = num_exchanges;
        let per_outbound = total_amount / outbound_count as f64;

        for i in 0..outbound_count {
            let day_offset = self.rng.random_range(0..3.min(available_days as u32).max(1));
            let date = start_date + Duration::days(day_offset as i64);
            if date > end_date {
                continue;
            }
            let exchange = EXCHANGES[self.rng.random_range(0..EXCHANGES.len())];
            let amount = per_outbound * self.rng.random_range(0.85..1.15);
            let ts = date
                .and_hms_opt(self.rng.random_range(9..20), self.rng.random_range(0..60), 0)
                .map(|dt| dt.and_utc())
                .unwrap_or_else(|| date.and_hms_opt(12, 0, 0).expect("valid").and_utc());

            let mut txn = BankTransaction::new(
                self.uuid_factory.next(),
                account.account_id,
                Decimal::from_f64_retain(amount).unwrap_or(Decimal::ONE_HUNDRED),
                &account.currency,
                Direction::Outbound,
                TransactionChannel::Wire,
                TransactionCategory::Investment,
                CounterpartyRef::crypto_exchange(exchange),
                &format!("Crypto purchase - {exchange}"),
                ts,
            );
            txn = txn.mark_suspicious(AmlTypology::CryptoIntegration, &scenario_id);
            txn = txn.with_laundering_stage(LaunderingStage::Placement);
            txn = txn.with_scenario(&scenario_id, seq);
            txn.ground_truth_explanation = Some(format!(
                "Crypto placement: ${:.2} to {exchange} (outbound {}/{outbound_count})",
                amount,
                i + 1,
            ));
            seq += 1;
            transactions.push(txn);
        }

        // Phase 2: Crypto → Fiat (integration) — after the off-chain gap
        // Returns via different exchanges (peel chain)
        let return_count = match sophistication {
            Sophistication::Basic => 1,
            Sophistication::Standard => self.rng.random_range(1..3),
            Sophistication::Professional => self.rng.random_range(2..5),
            Sophistication::Advanced => self.rng.random_range(3..8),
            Sophistication::StateLevel => self.rng.random_range(5..12),
        };

        let loss_pct = 0.02 + self.rng.random_range(0.0..0.05); // 2-7% "exchange fees"
        let return_total = total_amount * (1.0 - loss_pct);
        let per_return = return_total / return_count as f64;

        for i in 0..return_count {
            let return_start = start_date + Duration::days(gap_days as i64);
            let day_offset = self.rng.random_range(0..5.min(available_days as u32).max(1));
            let date = return_start + Duration::days(day_offset as i64);
            if date > end_date {
                continue;
            }
            let exchange = EXCHANGES[self.rng.random_range(0..EXCHANGES.len())];
            let amount = per_return * self.rng.random_range(0.8..1.2);
            let ts = date
                .and_hms_opt(self.rng.random_range(9..20), self.rng.random_range(0..60), 0)
                .map(|dt| dt.and_utc())
                .unwrap_or_else(|| date.and_hms_opt(12, 0, 0).expect("valid").and_utc());

            let mut txn = BankTransaction::new(
                self.uuid_factory.next(),
                account.account_id,
                Decimal::from_f64_retain(amount).unwrap_or(Decimal::ONE_HUNDRED),
                &account.currency,
                Direction::Inbound,
                TransactionChannel::Wire,
                TransactionCategory::TransferIn,
                CounterpartyRef::crypto_exchange(exchange),
                &format!("Crypto sale - {exchange}"),
                ts,
            );
            txn = txn.mark_suspicious(AmlTypology::CryptoIntegration, &scenario_id);
            txn = txn.with_laundering_stage(LaunderingStage::Integration);
            txn = txn.with_scenario(&scenario_id, seq);
            txn.ground_truth_explanation = Some(format!(
                "Crypto integration: ${:.2} from {exchange} after {}d off-chain gap (return {}/{return_count})",
                amount, gap_days, i + 1,
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
    fn test_crypto_generates_placement_and_integration() {
        let mut injector = CryptoIntegrationInjector::new(42);
        let customer = BankingCustomer::new_retail(Uuid::new_v4(), "Test", "User", "US",
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
        let account = BankAccount::new(Uuid::new_v4(), "ACC-001".into(),
            datasynth_core::models::banking::BankAccountType::Checking,
            customer.customer_id, "USD", NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());

        let txns = injector.generate(&customer, &account,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap(),
            Sophistication::Professional);

        assert!(!txns.is_empty());
        let outbound = txns.iter().filter(|t| t.direction == Direction::Outbound).count();
        let inbound = txns.iter().filter(|t| t.direction == Direction::Inbound).count();
        assert!(outbound > 0, "Should have placement (outbound) txns");
        assert!(inbound > 0, "Should have integration (inbound) txns");
        assert!(txns.iter().all(|t| t.ground_truth_explanation.is_some()));
    }
}

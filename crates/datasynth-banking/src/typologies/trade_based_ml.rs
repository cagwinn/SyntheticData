//! Trade-based money laundering typology.
//!
//! Patterns: over/under-invoicing, phantom shipments, multiple invoicing.
//! Business customers only. Uses SWIFT channel for international trade flows.

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
use crate::seed_offsets::TRADE_BASED_ML_SEED_OFFSET;

const TRADE_COUNTRIES: &[&str] = &["CN", "HK", "SG", "AE", "TR", "TH", "VN", "MY", "ID", "PH"];
const GOODS: &[&str] = &[
    "Electronics",
    "Textiles",
    "Machinery",
    "Chemicals",
    "Automotive Parts",
    "Medical Supplies",
    "Agricultural Products",
    "Precious Metals",
    "Construction Materials",
];

/// Trade-based money laundering injector.
pub struct TradeBasedMLInjector {
    rng: ChaCha8Rng,
    uuid_factory: DeterministicUuidFactory,
}

impl TradeBasedMLInjector {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(TRADE_BASED_ML_SEED_OFFSET)),
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
        let scenario_id = format!("TBM-{:06}", self.rng.random::<u32>());

        // Number of trade cycles
        let num_cycles = match sophistication {
            Sophistication::Basic => self.rng.random_range(1..3),
            Sophistication::Standard => self.rng.random_range(2..5),
            Sophistication::Professional => self.rng.random_range(3..7),
            Sophistication::Advanced => self.rng.random_range(5..10),
            Sophistication::StateLevel => self.rng.random_range(8..15),
        };

        // Over-invoicing factor (3.0 = invoice says 3x actual value)
        let invoice_factor: f64 = match sophistication {
            Sophistication::Basic => self.rng.random_range(2.0..4.0),
            Sophistication::Standard => self.rng.random_range(1.5..3.0),
            Sophistication::Professional => self.rng.random_range(1.3..2.0),
            Sophistication::Advanced => self.rng.random_range(1.15..1.5),
            Sophistication::StateLevel => self.rng.random_range(1.05..1.2),
        };

        let available_days = (end_date - start_date).num_days().max(1);
        let days_per_cycle = (available_days / num_cycles as i64).max(7);
        let mut seq = 0u32;

        for cycle in 0..num_cycles {
            let cycle_start = start_date + Duration::days(cycle as i64 * days_per_cycle);
            if cycle_start > end_date {
                break;
            }

            let goods = GOODS[self.rng.random_range(0..GOODS.len())];
            let country = TRADE_COUNTRIES[self.rng.random_range(0..TRADE_COUNTRIES.len())];
            let true_value: f64 = self.rng.random_range(10_000.0..100_000.0);
            let invoiced_value = true_value * invoice_factor;
            let invoice_num = format!(
                "INV-{}-{:04}",
                cycle_start.format("%Y%m"),
                self.rng.random::<u16>()
            );

            // Outbound payment (inflated invoice amount)
            let pay_date = cycle_start + Duration::days(self.rng.random_range(0..3) as i64);
            if pay_date <= end_date {
                let ts = pay_date
                    .and_hms_opt(
                        self.rng.random_range(9..17),
                        self.rng.random_range(0..60),
                        0,
                    )
                    .map(|dt| dt.and_utc())
                    .unwrap_or_else(|| pay_date.and_hms_opt(12, 0, 0).expect("valid").and_utc());

                let mut txn = BankTransaction::new(
                    self.uuid_factory.next(),
                    account.account_id,
                    Decimal::from_f64_retain(invoiced_value).unwrap_or(Decimal::ONE_HUNDRED),
                    &account.currency,
                    Direction::Outbound,
                    TransactionChannel::Swift,
                    TransactionCategory::InternationalTransfer,
                    CounterpartyRef::trade_partner(
                        self.uuid_factory.next(),
                        &format!("{country} Trading Co"),
                        country,
                    ),
                    &format!("{invoice_num} - {goods}"),
                    ts,
                );
                txn = txn.mark_suspicious(AmlTypology::TradeBasedML, &scenario_id);
                txn = txn.with_laundering_stage(LaunderingStage::Layering);
                txn = txn.with_scenario(&scenario_id, seq);
                txn.ground_truth_explanation = Some(format!(
                    "Trade-based ML: over-invoiced {goods} at ${:.0} (true value ${:.0}, factor {:.2}x). {invoice_num} to {country}",
                    invoiced_value, true_value, invoice_factor,
                ));
                seq += 1;
                transactions.push(txn);
            }

            // Inbound "refund" or "rebate" (the laundered difference returns)
            let rebate_days = self.rng.random_range(10..30).min(days_per_cycle as u32);
            let rebate_date = cycle_start + Duration::days(rebate_days as i64);
            if rebate_date <= end_date {
                let rebate_amount =
                    (invoiced_value - true_value) * self.rng.random_range(0.8..0.95);
                let ts = rebate_date
                    .and_hms_opt(
                        self.rng.random_range(9..17),
                        self.rng.random_range(0..60),
                        0,
                    )
                    .map(|dt| dt.and_utc())
                    .unwrap_or_else(|| rebate_date.and_hms_opt(12, 0, 0).expect("valid").and_utc());

                let mut txn = BankTransaction::new(
                    self.uuid_factory.next(),
                    account.account_id,
                    Decimal::from_f64_retain(rebate_amount).unwrap_or(Decimal::ONE_HUNDRED),
                    &account.currency,
                    Direction::Inbound,
                    TransactionChannel::Swift,
                    TransactionCategory::TransferIn,
                    CounterpartyRef::trade_partner(
                        self.uuid_factory.next(),
                        &format!("{country} Rebate Agent"),
                        country,
                    ),
                    &format!("Trade rebate - {invoice_num}"),
                    ts,
                );
                txn = txn.mark_suspicious(AmlTypology::TradeBasedML, &scenario_id);
                txn = txn.with_laundering_stage(LaunderingStage::Integration);
                txn = txn.with_scenario(&scenario_id, seq);
                txn.ground_truth_explanation = Some(format!(
                    "Trade-based ML: laundered difference ${:.0} returned as 'rebate' from {country} (cycle {}/{})",
                    rebate_amount, cycle + 1, num_cycles,
                ));
                seq += 1;
                transactions.push(txn);
            }
        }

        transactions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_trade_based_ml_generates_paired_transactions() {
        let mut injector = TradeBasedMLInjector::new(42);
        let customer = BankingCustomer::new_business(
            Uuid::new_v4(),
            "Import Export LLC",
            "US",
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        );
        let account = BankAccount::new(
            Uuid::new_v4(),
            "ACC-001".into(),
            datasynth_core::models::banking::BankAccountType::BusinessOperating,
            customer.customer_id,
            "USD",
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        );

        let txns = injector.generate(
            &customer,
            &account,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            Sophistication::Professional,
        );

        assert!(!txns.is_empty());
        let outbound = txns
            .iter()
            .filter(|t| t.direction == Direction::Outbound)
            .count();
        let inbound = txns
            .iter()
            .filter(|t| t.direction == Direction::Inbound)
            .count();
        assert!(outbound > 0, "Should have outbound invoice payments");
        assert!(inbound > 0, "Should have inbound rebates");
        // Outbound amounts should be larger than inbound (over-invoicing)
        let out_sum: f64 = txns
            .iter()
            .filter(|t| t.direction == Direction::Outbound)
            .map(|t| t.amount.to_string().parse::<f64>().unwrap_or(0.0))
            .sum();
        let in_sum: f64 = txns
            .iter()
            .filter(|t| t.direction == Direction::Inbound)
            .map(|t| t.amount.to_string().parse::<f64>().unwrap_or(0.0))
            .sum();
        assert!(
            out_sum > in_sum,
            "Outbound should exceed inbound (over-invoicing)"
        );
    }
}

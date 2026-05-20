//! Real estate integration typology.
//!
//! Pattern: large wire transfers to title companies / escrow accounts for
//! property purchases, often cash-heavy or all-cash. Common LLC / trust
//! ownership structures to obscure beneficial owner.

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
use crate::seed_offsets::REAL_ESTATE_INTEGRATION_SEED_OFFSET;

const TITLE_COMPANIES: &[&str] = &[
    "First American Title",
    "Chicago Title Insurance",
    "Stewart Title",
    "Fidelity National Title",
    "Old Republic Title",
    "Premier Escrow Services",
    "Pacific Title & Escrow",
];

const PROPERTY_TYPES: &[&str] = &[
    "luxury condo",
    "single-family residence",
    "commercial building",
    "multi-family investment property",
    "vacation home",
    "mixed-use development",
];

pub struct RealEstateIntegrationInjector {
    rng: ChaCha8Rng,
    uuid_factory: DeterministicUuidFactory,
}

impl RealEstateIntegrationInjector {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(REAL_ESTATE_INTEGRATION_SEED_OFFSET)),
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
        let scenario_id = format!("REI-{:06}", self.rng.random::<u32>());

        let num_properties = match sophistication {
            Sophistication::Basic => 1,
            Sophistication::Standard => self.rng.random_range(1..3),
            Sophistication::Professional => self.rng.random_range(2..4),
            Sophistication::Advanced => self.rng.random_range(3..6),
            Sophistication::StateLevel => self.rng.random_range(5..10),
        };

        let amount_per_property: f64 = match sophistication {
            Sophistication::Basic => self.rng.random_range(250_000.0..500_000.0),
            Sophistication::Standard => self.rng.random_range(400_000.0..1_200_000.0),
            Sophistication::Professional => self.rng.random_range(800_000.0..3_500_000.0),
            Sophistication::Advanced => self.rng.random_range(2_000_000.0..10_000_000.0),
            Sophistication::StateLevel => self.rng.random_range(5_000_000.0..50_000_000.0),
        };

        let available_days = (end_date - start_date).num_days().max(1);
        let days_per_property = (available_days / num_properties.max(1) as i64).max(14);
        let mut seq = 0u32;

        for i in 0..num_properties {
            let property_start = start_date + Duration::days(i as i64 * days_per_property);
            if property_start > end_date {
                break;
            }
            let title_co = TITLE_COMPANIES[self.rng.random_range(0..TITLE_COMPANIES.len())];
            let property = PROPERTY_TYPES[self.rng.random_range(0..PROPERTY_TYPES.len())];
            let property_amount = amount_per_property * self.rng.random_range(0.85..1.15);

            // Earnest money deposit (5-10% upfront)
            let earnest_pct = self.rng.random_range(0.05..0.10);
            let earnest = property_amount * earnest_pct;
            let earnest_ts = property_start
                .and_hms_opt(
                    self.rng.random_range(9..17),
                    self.rng.random_range(0..60),
                    0,
                )
                .map(|dt| dt.and_utc())
                .unwrap_or_else(chrono::Utc::now);

            let mut earnest_txn = BankTransaction::new(
                self.uuid_factory.next(),
                account.account_id,
                Decimal::from_f64_retain(earnest).unwrap_or(Decimal::ONE_HUNDRED),
                &account.currency,
                Direction::Outbound,
                TransactionChannel::Wire,
                TransactionCategory::Housing,
                CounterpartyRef {
                    counterparty_type: crate::models::CounterpartyType::FinancialInstitution,
                    counterparty_id: Some(self.uuid_factory.next()),
                    name: title_co.to_string(),
                    account_identifier: None,
                    bank_identifier: None,
                    country: Some("US".to_string()),
                },
                &format!("Earnest money - {property}"),
                earnest_ts,
            );
            earnest_txn =
                earnest_txn.mark_suspicious(AmlTypology::RealEstateIntegration, &scenario_id);
            earnest_txn = earnest_txn.with_laundering_stage(LaunderingStage::Layering);
            earnest_txn = earnest_txn.with_scenario(&scenario_id, seq);
            earnest_txn.ground_truth_explanation = Some(format!(
                "Real estate integration: ${:.0} earnest money via {title_co} for {property} (property {}/{num_properties})",
                earnest, i + 1,
            ));
            seq += 1;
            transactions.push(earnest_txn);

            // Closing: main balance
            let closing_days = self
                .rng
                .random_range(30..60)
                .min(days_per_property as u32 - 1);
            let closing_date = property_start + Duration::days(closing_days as i64);
            if closing_date > end_date {
                continue;
            }
            let closing_amount = property_amount - earnest;
            let closing_ts = closing_date
                .and_hms_opt(
                    self.rng.random_range(9..17),
                    self.rng.random_range(0..60),
                    0,
                )
                .map(|dt| dt.and_utc())
                .unwrap_or_else(chrono::Utc::now);

            let mut closing_txn = BankTransaction::new(
                self.uuid_factory.next(),
                account.account_id,
                Decimal::from_f64_retain(closing_amount).unwrap_or(Decimal::ONE_HUNDRED),
                &account.currency,
                Direction::Outbound,
                TransactionChannel::Wire,
                TransactionCategory::Housing,
                CounterpartyRef {
                    counterparty_type: crate::models::CounterpartyType::FinancialInstitution,
                    counterparty_id: Some(self.uuid_factory.next()),
                    name: title_co.to_string(),
                    account_identifier: None,
                    bank_identifier: None,
                    country: Some("US".to_string()),
                },
                &format!("Closing - {property} purchase"),
                closing_ts,
            );
            closing_txn =
                closing_txn.mark_suspicious(AmlTypology::RealEstateIntegration, &scenario_id);
            closing_txn = closing_txn.with_laundering_stage(LaunderingStage::Integration);
            closing_txn = closing_txn.with_scenario(&scenario_id, seq);
            closing_txn.ground_truth_explanation = Some(format!(
                "Real estate integration: ${:.0} closing payment via {title_co} for {property} (${:.0} total, ${:.0} earnest+close)",
                closing_amount, property_amount, earnest + closing_amount,
            ));
            seq += 1;
            transactions.push(closing_txn);
        }

        transactions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_real_estate_earnest_plus_closing() {
        let mut inj = RealEstateIntegrationInjector::new(42);
        let customer = BankingCustomer::new_business(
            Uuid::new_v4(),
            "Shell LLC",
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
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            Sophistication::Professional,
        );

        assert!(!txns.is_empty());
        assert!(txns
            .iter()
            .all(|t| matches!(t.direction, Direction::Outbound)));
        // Should have earnest + closing pairs
        let has_earnest = txns.iter().any(|t| t.reference.contains("Earnest"));
        let has_closing = txns.iter().any(|t| t.reference.contains("Closing"));
        assert!(has_earnest && has_closing);
    }
}

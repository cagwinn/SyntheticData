//! Romance scam typology.
//!
//! Pattern: victim (often elderly) sends progressively larger transfers to a
//! "romantic interest" abroad. Starts with small amounts, escalates as trust
//! is built. Often involves "emergency" events triggering urgent transfers.

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
use crate::seed_offsets::ROMANCE_SCAM_SEED_OFFSET;

/// Countries common in romance scam operations.
const SCAM_COUNTRIES: &[&str] = &["NG", "GH", "RU", "UA", "RO", "PH", "MY"];

const SCAM_PERSONAS: &[&str] = &[
    "Marcus Johnson (military)",
    "David Williams (oil engineer)",
    "Robert Taylor (doctor)",
    "James Anderson (engineer)",
    "Elena Petrova (nurse)",
    "Anna Kuznetsova (teacher)",
];

pub struct RomanceScamInjector {
    rng: ChaCha8Rng,
    uuid_factory: DeterministicUuidFactory,
}

impl RomanceScamInjector {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(ROMANCE_SCAM_SEED_OFFSET)),
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
        let scenario_id = format!("ROM-{:06}", self.rng.random::<u32>());

        // Grooming + escalation + urgent emergencies
        let num_transfers = match sophistication {
            Sophistication::Basic => self.rng.random_range(3..6),
            Sophistication::Standard => self.rng.random_range(5..10),
            Sophistication::Professional => self.rng.random_range(8..16),
            Sophistication::Advanced => self.rng.random_range(12..25),
            Sophistication::StateLevel => self.rng.random_range(20..40),
        };

        let persona = SCAM_PERSONAS[self.rng.random_range(0..SCAM_PERSONAS.len())];
        let country = SCAM_COUNTRIES[self.rng.random_range(0..SCAM_COUNTRIES.len())];
        let available_days = (end_date - start_date).num_days().max(1);

        for i in 0..num_transfers {
            let seq = i as u32;
            // Escalating amounts: starts small ($100-500), grows to $5K-50K
            let progress = i as f64 / num_transfers.max(1) as f64;
            let base_amount = 200.0 + progress.powf(2.0) * 10_000.0;
            let amount = base_amount * self.rng.random_range(0.7..1.4);

            // "Emergency" transfers late in the scam are larger and more frequent
            let is_emergency = i > num_transfers / 2 && self.rng.random::<f64>() < 0.3;
            let amount = if is_emergency { amount * 2.5 } else { amount };

            let day_offset = ((available_days as f64) * progress + self.rng.random_range(-3.0..3.0))
                .clamp(0.0, available_days as f64 - 1.0) as i64;
            let date = start_date + Duration::days(day_offset);
            let ts = date
                .and_hms_opt(
                    self.rng.random_range(0..24),
                    self.rng.random_range(0..60),
                    0,
                )
                .map(|dt| dt.and_utc())
                .unwrap_or_else(chrono::Utc::now);

            let (channel, category) = match self.rng.random_range(0..4) {
                0 => (
                    TransactionChannel::Wire,
                    TransactionCategory::InternationalTransfer,
                ),
                1 => (TransactionChannel::Wire, TransactionCategory::TransferOut),
                2 => (
                    TransactionChannel::PeerToPeer,
                    TransactionCategory::P2PPayment,
                ),
                _ => (TransactionChannel::Ach, TransactionCategory::TransferOut),
            };

            let mut txn = BankTransaction::new(
                self.uuid_factory.next(),
                account.account_id,
                Decimal::from_f64_retain(amount).unwrap_or(Decimal::ONE_HUNDRED),
                &account.currency,
                Direction::Outbound,
                channel,
                category,
                CounterpartyRef {
                    counterparty_type: crate::models::CounterpartyType::Peer,
                    counterparty_id: None,
                    name: persona.to_string(),
                    account_identifier: None,
                    bank_identifier: None,
                    country: Some(country.to_string()),
                },
                if is_emergency {
                    "URGENT - hospital emergency"
                } else {
                    "Personal transfer"
                },
                ts,
            );
            txn = txn.mark_suspicious(AmlTypology::RomanceScam, &scenario_id);
            txn = txn.with_laundering_stage(LaunderingStage::Placement);
            txn = txn.with_scenario(&scenario_id, seq);

            let stage_note = if is_emergency {
                format!("EMERGENCY transfer ${:.0}", amount)
            } else if progress < 0.3 {
                format!("grooming-phase transfer ${:.0}", amount)
            } else {
                format!("escalation-phase transfer ${:.0}", amount)
            };
            txn.ground_truth_explanation = Some(format!(
                "Romance scam: {stage_note} to '{persona}' in {country} (transfer {}/{num_transfers})",
                i + 1,
            ));
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
    fn test_romance_scam_escalation() {
        let mut inj = RomanceScamInjector::new(42);
        let customer = BankingCustomer::new_retail(
            Uuid::new_v4(),
            "Mary",
            "Victim",
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
        assert!(txns
            .iter()
            .all(|t| t.suspicion_reason == Some(AmlTypology::RomanceScam)));
        // Amounts should escalate: last quartile should have higher average than first
        let n = txns.len();
        if n >= 8 {
            use rust_decimal::prelude::ToPrimitive;
            let first_quarter: f64 = txns[..n / 4]
                .iter()
                .map(|t| t.amount.to_f64().unwrap_or(0.0))
                .sum::<f64>()
                / (n / 4) as f64;
            let last_quarter: f64 = txns[3 * n / 4..]
                .iter()
                .map(|t| t.amount.to_f64().unwrap_or(0.0))
                .sum::<f64>()
                / (n / 4) as f64;
            assert!(
                last_quarter > first_quarter,
                "Amounts should escalate: first={first_quarter}, last={last_quarter}"
            );
        }
    }
}

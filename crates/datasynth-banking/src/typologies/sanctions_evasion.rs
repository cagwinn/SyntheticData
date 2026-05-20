//! Sanctions evasion typology.
//!
//! Pattern: name variations/aliases bypass screening, funds routed through
//! transshipment countries to reach sanctioned destinations.

use chrono::{Duration, NaiveDate};
use datasynth_core::models::banking::{
    AmlTypology, Direction, LaunderingStage, Sophistication, TransactionCategory,
    TransactionChannel,
};
use datasynth_core::DeterministicUuidFactory;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rust_decimal::Decimal;

use crate::models::{
    BankAccount, BankTransaction, BankingCustomer, CounterpartyRef, SanctionsScreening,
    ScreeningResult,
};
use crate::seed_offsets::SANCTIONS_EVASION_SEED_OFFSET;

/// Transshipment countries used to evade sanctions.
const TRANSSHIPMENT_COUNTRIES: &[&str] = &["TR", "AE", "SG", "MY", "GE", "AM", "KG", "KZ"];

/// Sanctioned destination countries.
const SANCTIONED_DESTINATIONS: &[&str] = &["IR", "KP", "SY", "CU", "VE", "MM", "BY", "RU"];

/// Sanctions evasion injector.
pub struct SanctionsEvasionInjector {
    rng: ChaCha8Rng,
    uuid_factory: DeterministicUuidFactory,
}

impl SanctionsEvasionInjector {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(SANCTIONS_EVASION_SEED_OFFSET)),
            uuid_factory: DeterministicUuidFactory::new(
                seed,
                datasynth_core::GeneratorType::Anomaly,
            ),
        }
    }

    /// Generate name variations for evasion.
    fn generate_name_variations(&mut self, base_name: &str) -> Vec<String> {
        let mut variations = vec![base_name.to_string()];
        let parts: Vec<&str> = base_name.split_whitespace().collect();

        if parts.len() >= 2 {
            // Reversed name
            variations.push(format!(
                "{} {}",
                parts.last().unwrap_or(&""),
                parts.first().unwrap_or(&"")
            ));
            // Initial + last name
            if let Some(first) = parts.first() {
                if let Some(c) = first.chars().next() {
                    variations.push(format!("{}. {}", c, parts.last().unwrap_or(&"")));
                }
            }
        }

        // Transliteration variants
        let translits: &[(&str, &str)] = &[
            ("Mohammad", "Muhammad"),
            ("Muhammad", "Mohamed"),
            ("Ahmed", "Ahmad"),
            ("Ali", "Aly"),
            ("Hussein", "Husein"),
            ("Hassan", "Hasan"),
        ];
        for (from, to) in translits {
            if base_name.contains(from) {
                variations.push(base_name.replace(from, to));
            }
            if base_name.contains(to) {
                variations.push(base_name.replace(to, from));
            }
        }

        variations.truncate(5);
        variations
    }

    pub fn generate(
        &mut self,
        customer: &BankingCustomer,
        account: &BankAccount,
        start_date: NaiveDate,
        end_date: NaiveDate,
        sophistication: Sophistication,
    ) -> Vec<BankTransaction> {
        let mut transactions = Vec::new();
        let scenario_id = format!("SAN-{:06}", self.rng.random::<u32>());

        let num_transfers = match sophistication {
            Sophistication::Basic => self.rng.random_range(1..3),
            Sophistication::Standard => self.rng.random_range(2..5),
            Sophistication::Professional => self.rng.random_range(3..7),
            Sophistication::Advanced => self.rng.random_range(5..10),
            Sophistication::StateLevel => self.rng.random_range(8..15),
        };

        // Number of intermediary hops per transfer
        let hops = match sophistication {
            Sophistication::Basic => 0, // direct
            Sophistication::Standard => 1,
            Sophistication::Professional => self.rng.random_range(1..3),
            Sophistication::Advanced => self.rng.random_range(2..4),
            Sophistication::StateLevel => self.rng.random_range(3..5),
        };

        let available_days = (end_date - start_date).num_days().max(1);
        let name_variations = self.generate_name_variations(&customer.name.legal_name);
        let sanctioned =
            SANCTIONED_DESTINATIONS[self.rng.random_range(0..SANCTIONED_DESTINATIONS.len())];
        let mut seq = 0u32;

        for i in 0..num_transfers {
            let day_offset = self.rng.random_range(0..(available_days as u32).max(1));
            let date = start_date + Duration::days(day_offset as i64);
            if date > end_date {
                continue;
            }

            let amount: f64 = self.rng.random_range(5_000.0..50_000.0);

            if hops == 0 {
                // Direct transfer to sanctioned country (basic evasion using name variation)
                let alias = &name_variations[self.rng.random_range(0..name_variations.len())];
                let ts = date
                    .and_hms_opt(
                        self.rng.random_range(9..17),
                        self.rng.random_range(0..60),
                        0,
                    )
                    .map(|dt| dt.and_utc())
                    .unwrap_or_else(|| date.and_hms_opt(12, 0, 0).expect("valid").and_utc());

                let mut txn = BankTransaction::new(
                    self.uuid_factory.next(),
                    account.account_id,
                    Decimal::from_f64_retain(amount).unwrap_or(Decimal::ONE_HUNDRED),
                    &account.currency,
                    Direction::Outbound,
                    TransactionChannel::Swift,
                    TransactionCategory::InternationalTransfer,
                    CounterpartyRef::sanctioned_entity(self.uuid_factory.next(), alias, sanctioned),
                    &format!("Wire transfer - {alias}"),
                    ts,
                );
                txn = txn.mark_suspicious(AmlTypology::SanctionsEvasion, &scenario_id);
                txn = txn.with_laundering_stage(LaunderingStage::Placement);
                txn = txn.with_scenario(&scenario_id, seq);
                txn.ground_truth_explanation = Some(format!(
                    "Sanctions evasion: direct transfer ${:.0} to {sanctioned} using alias '{alias}' (transfer {}/{})",
                    amount, i + 1, num_transfers,
                ));
                seq += 1;
                transactions.push(txn);
            } else {
                // Multi-hop through transshipment countries
                let mut hop_amount = amount;
                for hop in 0..=hops {
                    let hop_date =
                        date + Duration::days(hop as i64 * self.rng.random_range(1..4) as i64);
                    if hop_date > end_date {
                        break;
                    }
                    let ts = hop_date
                        .and_hms_opt(
                            self.rng.random_range(9..17),
                            self.rng.random_range(0..60),
                            0,
                        )
                        .map(|dt| dt.and_utc())
                        .unwrap_or_else(|| {
                            hop_date.and_hms_opt(12, 0, 0).expect("valid").and_utc()
                        });

                    let (dest_country, entity_name) = if hop < hops {
                        let tc = TRANSSHIPMENT_COUNTRIES
                            [self.rng.random_range(0..TRANSSHIPMENT_COUNTRIES.len())];
                        (tc, format!("{tc} Trading LLC"))
                    } else {
                        (sanctioned, format!("{sanctioned} Final Entity"))
                    };

                    let stage = if hop == 0 {
                        LaunderingStage::Placement
                    } else if hop < hops {
                        LaunderingStage::Layering
                    } else {
                        LaunderingStage::Integration
                    };

                    // Small fee deducted per hop
                    hop_amount *= 0.98;

                    let mut txn = BankTransaction::new(
                        self.uuid_factory.next(),
                        account.account_id,
                        Decimal::from_f64_retain(hop_amount).unwrap_or(Decimal::ONE_HUNDRED),
                        &account.currency,
                        Direction::Outbound,
                        TransactionChannel::Swift,
                        TransactionCategory::InternationalTransfer,
                        CounterpartyRef::sanctioned_entity(
                            self.uuid_factory.next(),
                            &entity_name,
                            dest_country,
                        ),
                        &format!("Wire - {entity_name}"),
                        ts,
                    );
                    txn = txn.mark_suspicious(AmlTypology::SanctionsEvasion, &scenario_id);
                    txn = txn.with_laundering_stage(stage);
                    txn = txn.with_scenario(&scenario_id, seq);
                    txn.ground_truth_explanation = Some(format!(
                        "Sanctions evasion hop {}/{}: ${:.0} via {dest_country} → ultimate dest {sanctioned}",
                        hop + 1, hops + 1, hop_amount,
                    ));
                    seq += 1;
                    transactions.push(txn);
                }
            }
        }

        transactions
    }

    /// Generate sanctions screening data for a customer involved in evasion.
    pub fn generate_evasion_screening(
        &mut self,
        customer: &BankingCustomer,
        screening_date: NaiveDate,
    ) -> SanctionsScreening {
        let name_variations = self.generate_name_variations(&customer.name.legal_name);
        SanctionsScreening {
            last_screened: screening_date,
            screening_result: ScreeningResult::Clear, // Evasion = passed screening despite being a match
            matched_list: None,
            match_score: 0.0,
            name_variations,
            is_true_match: true, // Ground truth: this IS a sanctioned person who evaded screening
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_sanctions_evasion_basic() {
        let mut injector = SanctionsEvasionInjector::new(42);
        let customer = BankingCustomer::new_business(
            Uuid::new_v4(),
            "Ahmad Trading",
            "AE",
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
        assert!(txns
            .iter()
            .all(|t| t.suspicion_reason == Some(AmlTypology::SanctionsEvasion)));
        assert!(txns.iter().all(|t| t.ground_truth_explanation.is_some()));
    }

    #[test]
    fn test_name_variations() {
        let mut injector = SanctionsEvasionInjector::new(42);
        let vars = injector.generate_name_variations("Muhammad Ali Khan");
        assert!(vars.len() >= 2);
        assert!(vars.contains(&"Muhammad Ali Khan".to_string()));
    }

    #[test]
    fn test_evasion_screening() {
        let mut injector = SanctionsEvasionInjector::new(42);
        let customer = BankingCustomer::new_retail(
            Uuid::new_v4(),
            "Test",
            "User",
            "US",
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        );
        let screening = injector
            .generate_evasion_screening(&customer, NaiveDate::from_ymd_opt(2024, 6, 1).unwrap());
        assert_eq!(screening.screening_result, ScreeningResult::Clear);
        assert!(screening.is_true_match); // Evaded screening
    }
}

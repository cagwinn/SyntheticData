//! Multi-party criminal network generator.
//!
//! Creates coordinated scenarios across multiple customers/accounts:
//! - Structuring rings (coordinator + smurfs)
//! - Mule chains (recruiter → middlemen → cash-out)
//! - Shell company pyramids (layered business entities)

use chrono::NaiveDate;
use datasynth_core::models::banking::{AmlTypology, Sophistication};
use datasynth_core::DeterministicUuidFactory;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

use crate::models::{
    BankAccount, BankTransaction, BankingCustomer, NetworkContext, NetworkRole,
};
use crate::seed_offsets::NETWORK_GENERATOR_SEED_OFFSET;

/// Multi-party network scenario generator.
pub struct NetworkGenerator {
    rng: ChaCha8Rng,
    #[allow(dead_code)]
    uuid_factory: DeterministicUuidFactory,
    structuring_injector: super::StructuringInjector,
    mule_injector: super::MuleInjector,
    layering_injector: super::LayeringInjector,
}

impl NetworkGenerator {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(NETWORK_GENERATOR_SEED_OFFSET)),
            uuid_factory: DeterministicUuidFactory::new(seed, datasynth_core::GeneratorType::Anomaly),
            structuring_injector: super::StructuringInjector::new(seed + 100),
            mule_injector: super::MuleInjector::new(seed + 200),
            layering_injector: super::LayeringInjector::new(seed + 300),
        }
    }

    /// Generate a structuring ring: 1 coordinator distributes cash to N smurfs
    /// who each make structuring deposits.
    pub fn generate_structuring_ring(
        &mut self,
        customers: &[BankingCustomer],
        accounts: &[BankAccount],
        start_date: NaiveDate,
        end_date: NaiveDate,
        sophistication: Sophistication,
    ) -> Vec<BankTransaction> {
        let mut transactions = Vec::new();
        let network_id = format!("NET-STR-{:06}", self.rng.random::<u32>());

        // Ring size based on sophistication
        let ring_size = match sophistication {
            Sophistication::Basic => self.rng.random_range(3..5),
            Sophistication::Standard => self.rng.random_range(5..8),
            Sophistication::Professional => self.rng.random_range(7..12),
            Sophistication::Advanced => self.rng.random_range(10..16),
            Sophistication::StateLevel => self.rng.random_range(15..25),
        };

        // Need at least ring_size + 1 customers (coordinator + smurfs)
        if customers.len() < 2 || accounts.is_empty() {
            return transactions;
        }

        let participant_count = ring_size.min(customers.len() - 1);

        // First customer is coordinator, rest are smurfs
        for (idx, customer) in customers.iter().take(participant_count + 1).enumerate() {
            // Find this customer's account
            let account = match accounts.iter().find(|a| a.primary_owner_id == customer.customer_id) {
                Some(a) => a,
                None => continue,
            };

            let role = if idx == 0 {
                NetworkRole::Coordinator
            } else {
                NetworkRole::Smurf
            };

            // Each smurf runs the structuring injector
            if role == NetworkRole::Smurf {
                let mut smurf_txns = self.structuring_injector.generate(
                    customer,
                    account,
                    start_date,
                    end_date,
                    sophistication,
                );

                // Tag with network context
                for txn in &mut smurf_txns {
                    txn.network_context = Some(NetworkContext {
                        network_id: network_id.clone(),
                        network_role: role,
                        co_occurring_typologies: vec![AmlTypology::Structuring],
                        network_size: participant_count as u32 + 1,
                    });
                }

                transactions.extend(smurf_txns);
            }
        }

        transactions
    }

    /// Generate a mule chain: recruiter → N middlemen → cash-out node.
    pub fn generate_mule_chain(
        &mut self,
        customers: &[BankingCustomer],
        accounts: &[BankAccount],
        start_date: NaiveDate,
        end_date: NaiveDate,
        sophistication: Sophistication,
    ) -> Vec<BankTransaction> {
        let mut transactions = Vec::new();
        let network_id = format!("NET-MUL-{:06}", self.rng.random::<u32>());

        let chain_length = match sophistication {
            Sophistication::Basic => 2,
            Sophistication::Standard => self.rng.random_range(2..4),
            Sophistication::Professional => self.rng.random_range(3..5),
            Sophistication::Advanced => self.rng.random_range(4..7),
            Sophistication::StateLevel => self.rng.random_range(5..10),
        };

        if customers.len() < 2 || accounts.is_empty() {
            return transactions;
        }

        let participant_count = chain_length.min(customers.len());

        // Each participant in the chain runs the mule injector
        for (idx, customer) in customers.iter().take(participant_count).enumerate() {
            let account = match accounts.iter().find(|a| a.primary_owner_id == customer.customer_id) {
                Some(a) => a,
                None => continue,
            };

            let role = if idx == 0 {
                NetworkRole::Recruiter
            } else if idx == participant_count - 1 {
                NetworkRole::CashOut
            } else {
                NetworkRole::Middleman
            };

            let mut mule_txns = self.mule_injector.generate(
                customer,
                account,
                start_date,
                end_date,
                sophistication,
            );

            for txn in &mut mule_txns {
                txn.network_context = Some(NetworkContext {
                    network_id: network_id.clone(),
                    network_role: role,
                    co_occurring_typologies: vec![AmlTypology::MoneyMule],
                    network_size: participant_count as u32,
                });
            }

            transactions.extend(mule_txns);
        }

        transactions
    }

    /// Generate a shell company pyramid for layering.
    pub fn generate_shell_pyramid(
        &mut self,
        customers: &[BankingCustomer],
        accounts: &[BankAccount],
        start_date: NaiveDate,
        end_date: NaiveDate,
        sophistication: Sophistication,
    ) -> Vec<BankTransaction> {
        let mut transactions = Vec::new();
        let network_id = format!("NET-SHL-{:06}", self.rng.random::<u32>());

        let layers = match sophistication {
            Sophistication::Basic => 2,
            Sophistication::Standard => 3,
            Sophistication::Professional => self.rng.random_range(3..5),
            Sophistication::Advanced => self.rng.random_range(4..6),
            Sophistication::StateLevel => self.rng.random_range(5..7),
        };

        if customers.len() < 2 || accounts.is_empty() {
            return transactions;
        }

        let participant_count = layers.min(customers.len());

        for (idx, customer) in customers.iter().take(participant_count).enumerate() {
            let account = match accounts.iter().find(|a| a.primary_owner_id == customer.customer_id) {
                Some(a) => a,
                None => continue,
            };

            let role = if idx == 0 {
                NetworkRole::Coordinator
            } else if idx == participant_count - 1 {
                NetworkRole::Beneficiary
            } else {
                NetworkRole::ShellEntity
            };

            let mut layer_txns = self.layering_injector.generate(
                customer,
                account,
                start_date,
                end_date,
                sophistication,
            );

            for txn in &mut layer_txns {
                txn.network_context = Some(NetworkContext {
                    network_id: network_id.clone(),
                    network_role: role,
                    co_occurring_typologies: vec![AmlTypology::Layering, AmlTypology::ShellCompany],
                    network_size: participant_count as u32,
                });
            }

            transactions.extend(layer_txns);
        }

        transactions
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_customer(name: &str) -> BankingCustomer {
        BankingCustomer::new_retail(
            Uuid::new_v4(), name, "User", "US",
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        )
    }

    fn make_account(customer: &BankingCustomer) -> BankAccount {
        BankAccount::new(
            Uuid::new_v4(), format!("ACC-{}", customer.customer_id),
            datasynth_core::models::banking::BankAccountType::Checking,
            customer.customer_id, "USD",
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        )
    }

    #[test]
    fn test_structuring_ring() {
        let mut gen = NetworkGenerator::new(42);
        let customers: Vec<_> = (0..6).map(|i| make_customer(&format!("Smurf{i}"))).collect();
        let accounts: Vec<_> = customers.iter().map(|c| make_account(c)).collect();

        let txns = gen.generate_structuring_ring(
            &customers, &accounts,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap(),
            Sophistication::Standard,
        );

        assert!(!txns.is_empty(), "Should generate ring transactions");
        assert!(txns.iter().all(|t| t.network_context.is_some()));
        let network_ids: std::collections::HashSet<_> = txns.iter()
            .filter_map(|t| t.network_context.as_ref().map(|n| n.network_id.clone()))
            .collect();
        assert_eq!(network_ids.len(), 1, "All txns should share one network_id");
    }

    #[test]
    fn test_mule_chain() {
        let mut gen = NetworkGenerator::new(42);
        let customers: Vec<_> = (0..4).map(|i| make_customer(&format!("Mule{i}"))).collect();
        let accounts: Vec<_> = customers.iter().map(|c| make_account(c)).collect();

        let txns = gen.generate_mule_chain(
            &customers, &accounts,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap(),
            Sophistication::Professional,
        );

        assert!(!txns.is_empty());
        // Should have multiple roles
        let roles: std::collections::HashSet<_> = txns.iter()
            .filter_map(|t| t.network_context.as_ref().map(|n| n.network_role))
            .collect();
        assert!(roles.len() >= 2, "Should have multiple roles, got {:?}", roles);
    }
}

//! Main AML typology injector.

use datasynth_core::models::banking::{AmlTypology, LaunderingStage, Sophistication};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

use crate::config::BankingConfig;
use crate::models::{
    AmlScenario, BankAccount, BankTransaction, BankingCustomer, CaseNarrative, CaseRecommendation,
};

use super::{
    FunnelInjector, LayeringInjector, MuleInjector, NetworkGenerator, SpoofingEngine,
    StructuringInjector,
};
use crate::seed_offsets::TYPOLOGY_INJECTOR_SEED_OFFSET;

/// Main AML typology injector.
pub struct TypologyInjector {
    config: BankingConfig,
    rng: ChaCha8Rng,
    structuring_injector: StructuringInjector,
    funnel_injector: FunnelInjector,
    layering_injector: LayeringInjector,
    mule_injector: MuleInjector,
    network_generator: NetworkGenerator,
    spoofing_engine: SpoofingEngine,
    scenarios: Vec<AmlScenario>,
    scenario_counter: u32,
}

impl TypologyInjector {
    /// Create a new typology injector.
    pub fn new(config: BankingConfig, seed: u64) -> Self {
        Self {
            config: config.clone(),
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(TYPOLOGY_INJECTOR_SEED_OFFSET)),
            structuring_injector: StructuringInjector::new(seed),
            funnel_injector: FunnelInjector::new(seed),
            layering_injector: LayeringInjector::new(seed),
            mule_injector: MuleInjector::new(seed),
            network_generator: NetworkGenerator::new(seed),
            spoofing_engine: SpoofingEngine::new(config.spoofing.clone(), seed),
            scenarios: Vec::new(),
            scenario_counter: 0,
        }
    }

    /// Inject AML patterns into transactions.
    pub fn inject(
        &mut self,
        customers: &mut [BankingCustomer],
        accounts: &mut [BankAccount],
        transactions: &mut Vec<BankTransaction>,
    ) {
        if !self.config.typologies.suspicious_rate.is_finite()
            || self.config.typologies.suspicious_rate <= 0.0
        {
            return;
        }

        let total_customers = customers.len();

        // Calculate number of suspicious customers
        let suspicious_count =
            (total_customers as f64 * self.config.typologies.suspicious_rate) as usize;

        // Select random customers for suspicious activity
        let mut customer_indices: Vec<usize> = (0..total_customers).collect();
        customer_indices.shuffle(&mut self.rng);
        let suspicious_indices: Vec<usize> = customer_indices
            .into_iter()
            .take(suspicious_count)
            .collect();

        // Inject different typologies
        for &idx in &suspicious_indices {
            let typology = self.select_typology();
            let sophistication = self.select_sophistication();

            match typology {
                AmlTypology::Structuring | AmlTypology::Smurfing => {
                    self.inject_structuring(idx, customers, accounts, transactions, sophistication);
                }
                AmlTypology::FunnelAccount => {
                    self.inject_funnel(idx, customers, accounts, transactions, sophistication);
                }
                AmlTypology::Layering => {
                    self.inject_layering(idx, customers, accounts, transactions, sophistication);
                }
                AmlTypology::MoneyMule => {
                    self.inject_mule(idx, customers, accounts, transactions, sophistication);
                }
                _ => {
                    // Generic suspicious marking
                    self.inject_generic(idx, customers, accounts, transactions, typology);
                }
            }
        }

        // Phase 6c: Coordinated criminal network injection (activates
        // `network_typology_rate` — previously a phantom config). Emits
        // structuring rings, mule chains, and shell pyramids where every
        // participating transaction carries a `network_context`, which the
        // relationship-label extractor uses to build mule/shell edges.
        self.inject_network_typologies(customers, accounts, transactions);

        // Apply spoofing to sophisticated typologies
        if self.config.spoofing.enabled {
            self.apply_spoofing(customers, transactions);
        }
    }

    /// Inject coordinated multi-party criminal networks (rings, chains,
    /// pyramids). Sized by `typologies.network_typology_rate`.
    fn inject_network_typologies(
        &mut self,
        customers: &mut [BankingCustomer],
        accounts: &mut [BankAccount],
        transactions: &mut Vec<BankTransaction>,
    ) {
        let rate = self.config.typologies.network_typology_rate;
        if !rate.is_finite() || rate <= 0.0 {
            return;
        }

        let total_customers = customers.len();
        let participants = (total_customers as f64 * rate).round() as usize;
        // Need enough people for at least one small ring.
        if participants < 4 {
            return;
        }

        // Average network size (participants per ring/chain/pyramid).
        let avg_network_size: usize = 7;
        let num_networks = (participants / avg_network_size).max(1);

        let start_date = crate::parse_start_date(&self.config.population.start_date);
        let end_date =
            start_date + chrono::Months::new(self.config.population.period_months.max(1));

        // Eligible customers — must have at least one account.
        let eligible_indices: Vec<usize> = customers
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.account_ids.is_empty())
            .map(|(i, _)| i)
            .collect();
        if eligible_indices.len() < 4 {
            return;
        }
        // Shuffled once; we take a disjoint batch per network by walking
        // forward with a moving window, avoiding a full clone per iteration.
        let mut idx_pool = eligible_indices;
        idx_pool.shuffle(&mut self.rng);
        let mut cursor = 0usize;

        for _ in 0..num_networks {
            let batch_size = self
                .rng
                .random_range(5..=avg_network_size.saturating_add(3))
                .min(idx_pool.len());
            if batch_size < 3 {
                continue;
            }
            if cursor + batch_size > idx_pool.len() {
                // Reshuffle and rewind when the pool is exhausted so
                // subsequent networks still see a random ordering.
                idx_pool.shuffle(&mut self.rng);
                cursor = 0;
            }
            let selected: Vec<usize> = idx_pool[cursor..cursor + batch_size].to_vec();
            cursor += batch_size;

            let batch_customers: Vec<BankingCustomer> =
                selected.iter().map(|&i| customers[i].clone()).collect();
            let sophistication = self.select_sophistication();

            // Pick network kind via weighted selection so the 40/35/25 mix
            // between rings / chains / pyramids lives in one place.
            #[derive(Clone, Copy)]
            enum NetworkKind {
                Ring,
                Chain,
                Pyramid,
            }
            let kind = *datasynth_core::utils::weighted_select(
                &mut self.rng,
                &[
                    (NetworkKind::Ring, 0.40),
                    (NetworkKind::Chain, 0.35),
                    (NetworkKind::Pyramid, 0.25),
                ],
            );
            let (new_txns, typology, stage) = match kind {
                NetworkKind::Ring => (
                    self.network_generator.generate_structuring_ring(
                        &batch_customers,
                        accounts,
                        start_date,
                        end_date,
                        sophistication,
                    ),
                    AmlTypology::Structuring,
                    LaunderingStage::Placement,
                ),
                NetworkKind::Chain => (
                    self.network_generator.generate_mule_chain(
                        &batch_customers,
                        accounts,
                        start_date,
                        end_date,
                        sophistication,
                    ),
                    AmlTypology::MoneyMule,
                    LaunderingStage::Layering,
                ),
                NetworkKind::Pyramid => (
                    self.network_generator.generate_shell_pyramid(
                        &batch_customers,
                        accounts,
                        start_date,
                        end_date,
                        sophistication,
                    ),
                    AmlTypology::Layering,
                    LaunderingStage::Integration,
                ),
            };

            if new_txns.is_empty() {
                continue;
            }

            // Build the scenario, then mark participating customers/accounts.
            let scenario_id = self.next_scenario_id();
            let mut scenario = AmlScenario::new(&scenario_id, typology, start_date, end_date)
                .with_sophistication(sophistication);
            scenario.add_stage(stage);

            for txn in &new_txns {
                scenario.add_transaction(txn.transaction_id, txn.amount);
            }
            for &i in &selected {
                let cust = &mut customers[i];
                scenario.add_customer(cust.customer_id);
                if matches!(typology, AmlTypology::MoneyMule) {
                    cust.is_mule = true;
                }
                for acct_id in cust.account_ids.clone() {
                    scenario.add_account(acct_id);
                    if let Some(acct) = accounts.iter_mut().find(|a| a.account_id == acct_id) {
                        if matches!(typology, AmlTypology::MoneyMule) {
                            acct.is_mule_account = true;
                        }
                        if acct.case_id.is_none() {
                            acct.case_id = Some(scenario_id.clone());
                        }
                    }
                }
            }

            scenario.narrative = CaseNarrative::new(
                "Coordinated multi-party network detected: participants share a common case identifier and transaction patterns consistent with a criminal ring.",
            )
            .with_recommendation(CaseRecommendation::FileSar);

            transactions.extend(new_txns);
            self.scenarios.push(scenario);
        }
    }

    /// Select a typology based on configured rates.
    fn select_typology(&mut self) -> AmlTypology {
        let rates = [
            (
                AmlTypology::Structuring,
                self.config.typologies.structuring_rate,
            ),
            (
                AmlTypology::FunnelAccount,
                self.config.typologies.funnel_rate,
            ),
            (AmlTypology::Layering, self.config.typologies.layering_rate),
            (AmlTypology::MoneyMule, self.config.typologies.mule_rate),
            (
                AmlTypology::AccountTakeover,
                self.config.typologies.fraud_rate * 0.3,
            ),
            (
                AmlTypology::FirstPartyFraud,
                self.config.typologies.fraud_rate * 0.3,
            ),
            (
                AmlTypology::AuthorizedPushPayment,
                self.config.typologies.fraud_rate * 0.4,
            ),
        ];

        let total: f64 = rates.iter().map(|(_, r)| r).sum();
        if total <= 0.0 {
            return AmlTypology::Structuring;
        }

        let roll: f64 = self.rng.random::<f64>() * total;
        let mut cumulative = 0.0;

        for (typology, rate) in rates {
            cumulative += rate;
            if roll < cumulative {
                return typology;
            }
        }

        AmlTypology::Structuring
    }

    /// Select sophistication level.
    fn select_sophistication(&mut self) -> Sophistication {
        let dist = &self.config.typologies.sophistication;
        let total = dist.basic + dist.standard + dist.professional + dist.advanced;
        let roll: f64 = self.rng.random::<f64>() * total;

        let mut cumulative = 0.0;
        cumulative += dist.basic;
        if roll < cumulative {
            return Sophistication::Basic;
        }
        cumulative += dist.standard;
        if roll < cumulative {
            return Sophistication::Standard;
        }
        cumulative += dist.professional;
        if roll < cumulative {
            return Sophistication::Professional;
        }
        Sophistication::Advanced
    }

    /// Inject structuring pattern.
    fn inject_structuring(
        &mut self,
        customer_idx: usize,
        customers: &mut [BankingCustomer],
        accounts: &mut [BankAccount],
        transactions: &mut Vec<BankTransaction>,
        sophistication: Sophistication,
    ) {
        let customer = &mut customers[customer_idx];
        if customer.account_ids.is_empty() {
            return;
        }

        let account_id = customer.account_ids[0];
        let account = accounts.iter_mut().find(|a| a.account_id == account_id);

        if let Some(account) = account {
            let scenario_id = self.next_scenario_id();
            let start_date = crate::parse_start_date(&self.config.population.start_date);
            let end_date = start_date + chrono::Months::new(1);

            let mut scenario =
                AmlScenario::new(&scenario_id, AmlTypology::Structuring, start_date, end_date)
                    .with_sophistication(sophistication);

            // Generate structuring transactions
            let structuring_txns = self.structuring_injector.generate(
                customer,
                account,
                start_date,
                end_date,
                sophistication,
            );

            for txn in structuring_txns {
                scenario.add_transaction(txn.transaction_id, txn.amount);
                transactions.push(txn);
            }

            scenario.add_customer(customer.customer_id);
            scenario.add_account(account.account_id);
            scenario.add_stage(LaunderingStage::Placement);

            // Generate narrative
            scenario.narrative = CaseNarrative::new(
                &format!(
                    "Customer {} conducted multiple cash deposits just below reporting threshold over a short period.",
                    customer.name.display_name()
                ),
            )
            .with_recommendation(CaseRecommendation::FileSar);

            // Mark customer
            customer.is_mule = false;
            account.case_id = Some(scenario_id.clone());

            self.scenarios.push(scenario);
        }
    }

    /// Inject funnel account pattern.
    fn inject_funnel(
        &mut self,
        customer_idx: usize,
        customers: &mut [BankingCustomer],
        accounts: &mut [BankAccount],
        transactions: &mut Vec<BankTransaction>,
        sophistication: Sophistication,
    ) {
        let customer = &mut customers[customer_idx];
        if customer.account_ids.is_empty() {
            return;
        }

        let account_id = customer.account_ids[0];
        if let Some(account) = accounts.iter_mut().find(|a| a.account_id == account_id) {
            let scenario_id = self.next_scenario_id();
            let start_date = crate::parse_start_date(&self.config.population.start_date);
            let end_date = start_date + chrono::Months::new(2);

            let mut scenario = AmlScenario::new(
                &scenario_id,
                AmlTypology::FunnelAccount,
                start_date,
                end_date,
            )
            .with_sophistication(sophistication);

            // Generate funnel transactions
            let funnel_txns = self.funnel_injector.generate(
                customer,
                account,
                start_date,
                end_date,
                sophistication,
            );

            for txn in funnel_txns {
                scenario.add_transaction(txn.transaction_id, txn.amount);
                transactions.push(txn);
            }

            scenario.add_customer(customer.customer_id);
            scenario.add_account(account.account_id);
            scenario.add_stage(LaunderingStage::Layering);
            account.is_funnel_account = true;
            account.case_id = Some(scenario_id.clone());

            scenario.narrative = CaseNarrative::new(
                "Account shows funnel pattern with many inbound transfers rapidly consolidated and moved out.",
            )
            .with_recommendation(CaseRecommendation::FileSar);

            self.scenarios.push(scenario);
        }
    }

    /// Inject layering pattern.
    fn inject_layering(
        &mut self,
        customer_idx: usize,
        customers: &mut [BankingCustomer],
        accounts: &mut [BankAccount],
        transactions: &mut Vec<BankTransaction>,
        sophistication: Sophistication,
    ) {
        let customer = &mut customers[customer_idx];
        if customer.account_ids.is_empty() {
            return;
        }

        let account_id = customer.account_ids[0];
        if let Some(account) = accounts.iter_mut().find(|a| a.account_id == account_id) {
            let scenario_id = self.next_scenario_id();
            let start_date = crate::parse_start_date(&self.config.population.start_date);
            let end_date = start_date + chrono::Months::new(1);

            let mut scenario =
                AmlScenario::new(&scenario_id, AmlTypology::Layering, start_date, end_date)
                    .with_sophistication(sophistication);

            let layering_txns = self.layering_injector.generate(
                customer,
                account,
                start_date,
                end_date,
                sophistication,
            );

            for txn in layering_txns {
                scenario.add_transaction(txn.transaction_id, txn.amount);
                transactions.push(txn);
            }

            scenario.add_customer(customer.customer_id);
            scenario.add_account(account.account_id);
            scenario.add_stage(LaunderingStage::Layering);
            account.case_id = Some(scenario_id.clone());

            scenario.narrative = CaseNarrative::new(
                "Complex layering pattern with rapid multi-hop transfers designed to obscure fund trail.",
            )
            .with_recommendation(CaseRecommendation::FileSar);

            self.scenarios.push(scenario);
        }
    }

    /// Inject mule pattern.
    fn inject_mule(
        &mut self,
        customer_idx: usize,
        customers: &mut [BankingCustomer],
        accounts: &mut [BankAccount],
        transactions: &mut Vec<BankTransaction>,
        sophistication: Sophistication,
    ) {
        let customer = &mut customers[customer_idx];
        if customer.account_ids.is_empty() {
            return;
        }

        let account_id = customer.account_ids[0];
        if let Some(account) = accounts.iter_mut().find(|a| a.account_id == account_id) {
            let scenario_id = self.next_scenario_id();
            let start_date = crate::parse_start_date(&self.config.population.start_date);
            let end_date = start_date + chrono::Months::new(1);

            let mut scenario =
                AmlScenario::new(&scenario_id, AmlTypology::MoneyMule, start_date, end_date)
                    .with_sophistication(sophistication);

            let mule_txns = self.mule_injector.generate(
                customer,
                account,
                start_date,
                end_date,
                sophistication,
            );

            for txn in mule_txns {
                scenario.add_transaction(txn.transaction_id, txn.amount);
                transactions.push(txn);
            }

            scenario.add_customer(customer.customer_id);
            scenario.add_account(account.account_id);
            customer.is_mule = true;
            account.is_mule_account = true;
            account.case_id = Some(scenario_id.clone());

            scenario.narrative = CaseNarrative::new(
                "Account shows classic money mule pattern: inbound transfers followed by rapid cash withdrawals or wire transfers.",
            )
            .with_recommendation(CaseRecommendation::CloseAccount);

            self.scenarios.push(scenario);
        }
    }

    /// Inject generic suspicious pattern.
    fn inject_generic(
        &mut self,
        customer_idx: usize,
        customers: &mut [BankingCustomer],
        accounts: &mut [BankAccount],
        transactions: &mut [BankTransaction],
        typology: AmlTypology,
    ) {
        let customer = &mut customers[customer_idx];
        if customer.account_ids.is_empty() {
            return;
        }

        // Mark random transactions as suspicious
        let account_id = customer.account_ids[0];
        let scenario_id = self.next_scenario_id();

        for txn in transactions.iter_mut() {
            if txn.account_id == account_id && self.rng.random::<f64>() < 0.1 {
                txn.is_suspicious = true;
                txn.suspicion_reason = Some(typology);
                txn.case_id = Some(scenario_id.clone());
            }
        }

        if let Some(account) = accounts.iter_mut().find(|a| a.account_id == account_id) {
            account.case_id = Some(scenario_id);
        }
    }

    /// Apply spoofing to sophisticated transactions.
    fn apply_spoofing(
        &mut self,
        customers: &[BankingCustomer],
        transactions: &mut [BankTransaction],
    ) {
        for txn in transactions.iter_mut() {
            if txn.is_suspicious {
                // Find the customer
                let customer = customers
                    .iter()
                    .find(|c| c.account_ids.contains(&txn.account_id));

                if let Some(customer) = customer {
                    self.spoofing_engine.apply(txn, customer);
                }
            }
        }
    }

    /// Generate next scenario ID.
    fn next_scenario_id(&mut self) -> String {
        self.scenario_counter += 1;
        format!("SC-{:06}", self.scenario_counter)
    }

    /// Get all generated scenarios.
    pub fn get_scenarios(&self) -> &[AmlScenario] {
        &self.scenarios
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_typology_injection() {
        let config = BankingConfig::small();
        let mut injector = TypologyInjector::new(config.clone(), 12345);

        // Generate test data
        let mut customer_gen = crate::generators::CustomerGenerator::new(config.clone(), 12345);
        let mut customers = customer_gen.generate_all();

        let mut account_gen = crate::generators::AccountGenerator::new(config.clone(), 12345);
        let mut accounts = account_gen.generate_for_customers(&mut customers);

        let mut txn_gen = crate::generators::TransactionGenerator::new(config, 12345);
        let mut transactions = txn_gen.generate_all(&customers, &mut accounts);

        let initial_count = transactions.len();

        injector.inject(&mut customers, &mut accounts, &mut transactions);

        // Should have added some suspicious transactions
        let suspicious_count = transactions.iter().filter(|t| t.is_suspicious).count();
        assert!(suspicious_count > 0 || transactions.len() > initial_count);
    }
}

//! Payment bridge generator: links document-flow Payment records to BankTransactions.
//!
//! This closes the critical integration gap between the banking module and the
//! rest of the platform. For each Payment in the P2P/O2C document flows, if the
//! payer or payee has a banking profile (via `enterprise_customer_id` linkage),
//! a corresponding `BankTransaction` is emitted on their bank account.
//!
//! Result: a vendor invoice payment is visible in BOTH the document flow
//! (`Payment-001`) AND the banking layer (`BankTransaction` on the house bank
//! account), with a bi-directional link via `source_payment_id`. Fraud labels
//! propagate across both layers. Bank reconciliation can match real transactions.

use std::collections::HashMap;

use chrono::{TimeZone, Utc};
use datasynth_core::documents::{Payment, PaymentMethod, PaymentType};
use datasynth_core::models::banking::{
    AmlTypology, Direction, LaunderingStage, TransactionCategory, TransactionChannel,
};
use datasynth_core::DeterministicUuidFactory;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use uuid::Uuid;

use crate::models::{BankAccount, BankTransaction, BankingCustomer, CounterpartyRef};

/// Seed offset for the payment bridge generator.
pub const PAYMENT_BRIDGE_SEED_OFFSET: u64 = 7900;

/// Bridges document-flow Payments to BankTransactions.
pub struct PaymentBridgeGenerator {
    rng: ChaCha8Rng,
    uuid_factory: DeterministicUuidFactory,
}

/// Statistics from bridging.
#[derive(Debug, Default, Clone)]
pub struct BridgeStats {
    /// Number of payments successfully bridged
    pub bridged_count: usize,
    /// Number of bank transactions emitted (includes mirror transactions)
    pub transactions_emitted: usize,
    /// Number of payments skipped (no matching bank account)
    pub skipped_no_account: usize,
    /// Number of payments skipped as non-bridgeable (Clearing type, voided)
    pub skipped_non_bridgeable: usize,
    /// Number of payments whose fraud label was propagated to the banking layer
    /// (note: each propagation may tag 1-2 bank transactions — see `ground_truth_explanation`)
    pub fraud_propagated: usize,
}

impl PaymentBridgeGenerator {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(PAYMENT_BRIDGE_SEED_OFFSET)),
            uuid_factory: DeterministicUuidFactory::new(
                seed,
                datasynth_core::GeneratorType::Anomaly,
            ),
        }
    }

    /// Bridge payments to bank transactions.
    ///
    /// Strategy:
    /// - For each Payment, find a house bank account on the enterprise side
    ///   (via `BankAccount` where `primary_owner_id` maps to the enterprise
    ///   customer's banking profile, or the first business banking account
    ///   available as a fallback).
    /// - Emit a `BankTransaction` with:
    ///   - `direction` derived from `payment_type` (AP = outbound, AR = inbound)
    ///   - `channel` derived from `payment_method`
    ///   - `source_payment_id` set to the payment document ID
    ///   - `source_invoice_id` set from the first allocation
    ///   - `journal_entry_id` set from `header.journal_entry_id`
    ///   - Fraud labels propagated if Payment was tagged fraudulent
    /// - If the counterparty (vendor/customer) also has a banking profile,
    ///   optionally emit a second BankTransaction on their side (inverse direction).
    ///
    /// `bridge_rate`: fraction of eligible payments to bridge (0.0-1.0).
    /// `payer_account_lookup`: closure returning a bank account for the payer company code.
    pub fn bridge_payments(
        &mut self,
        payments: &[Payment],
        banking_customers: &[BankingCustomer],
        banking_accounts: &[BankAccount],
        bridge_rate: f64,
    ) -> (Vec<BankTransaction>, BridgeStats) {
        let mut stats = BridgeStats::default();
        let mut transactions = Vec::new();

        if bridge_rate <= 0.0 || payments.is_empty() || banking_accounts.is_empty() {
            return (transactions, stats);
        }

        // Index: business_partner_id (vendor/customer ID) -> banking_customer
        let bp_to_banking: HashMap<&str, &BankingCustomer> = banking_customers
            .iter()
            .filter_map(|bc| bc.enterprise_customer_id.as_deref().map(|id| (id, bc)))
            .collect();

        // Index: banking_customer_id -> primary bank account (first one found)
        let customer_to_account: HashMap<Uuid, &BankAccount> = {
            let mut map = HashMap::new();
            for acct in banking_accounts {
                map.entry(acct.primary_owner_id).or_insert(acct);
            }
            map
        };

        // Select a "house bank" account — the first business account, or any account
        // This represents the enterprise's own bank account for AP/AR flows
        let house_bank_account = banking_accounts
            .iter()
            .find(|a| {
                matches!(
                    a.account_type,
                    datasynth_core::models::banking::BankAccountType::BusinessOperating
                        | datasynth_core::models::banking::BankAccountType::BusinessSavings
                )
            })
            .or_else(|| banking_accounts.first());

        // Look up the house bank owner's residence_country for location tagging
        let house_bank_country: Option<String> = house_bank_account.and_then(|a| {
            banking_customers
                .iter()
                .find(|c| c.customer_id == a.primary_owner_id)
                .map(|c| c.residence_country.clone())
        });

        for payment in payments {
            // Probabilistic bridging based on rate
            if self.rng.random::<f64>() > bridge_rate {
                continue;
            }

            // Only bridge clean payments (clearings, voids are internal)
            if matches!(payment.payment_type, PaymentType::Clearing) || payment.is_voided {
                stats.skipped_non_bridgeable += 1;
                continue;
            }

            // Determine the enterprise-side bank account
            let enterprise_account = house_bank_account;
            let Some(enterprise_account) = enterprise_account else {
                stats.skipped_no_account += 1;
                continue;
            };

            // Determine direction
            let (direction, category) = match payment.payment_type {
                PaymentType::ApPayment | PaymentType::DownPayment | PaymentType::Advance => {
                    (Direction::Outbound, TransactionCategory::TransferOut)
                }
                PaymentType::ArReceipt => (Direction::Inbound, TransactionCategory::TransferIn),
                PaymentType::Refund => {
                    if payment.is_vendor {
                        // Vendor refund = money coming back to us
                        (Direction::Inbound, TransactionCategory::Refund)
                    } else {
                        (Direction::Outbound, TransactionCategory::Refund)
                    }
                }
                PaymentType::Clearing => continue, // already filtered above
            };

            let channel = payment_method_to_channel(payment.payment_method);

            // Look up the counterparty's banking profile (if any)
            let counterparty_banking = bp_to_banking
                .get(payment.business_partner_id.as_str())
                .copied();

            let counterparty_ref = if let Some(bc) = counterparty_banking {
                CounterpartyRef {
                    counterparty_type: if payment.is_vendor {
                        crate::models::CounterpartyType::FinancialInstitution
                    } else {
                        crate::models::CounterpartyType::Peer
                    },
                    counterparty_id: Some(bc.customer_id),
                    name: bc.display_name.clone(),
                    account_identifier: payment.partner_bank_account.clone(),
                    bank_identifier: None,
                    country: Some(bc.residence_country.clone()),
                }
            } else {
                // External counterparty — still reference by business_partner_id
                CounterpartyRef {
                    counterparty_type: if payment.is_vendor {
                        crate::models::CounterpartyType::FinancialInstitution
                    } else {
                        crate::models::CounterpartyType::Peer
                    },
                    counterparty_id: None,
                    name: payment.business_partner_id.clone(),
                    account_identifier: payment.partner_bank_account.clone(),
                    bank_identifier: None,
                    country: None,
                }
            };

            let value_date = payment.value_date;
            let hour = self.rng.random_range(9..17);
            let minute = self.rng.random_range(0..60);
            let ts = value_date
                .and_hms_opt(hour, minute, 0)
                .map(|dt| Utc.from_utc_datetime(&dt))
                .unwrap_or_else(Utc::now);

            let mut txn = BankTransaction::new(
                self.uuid_factory.next(),
                enterprise_account.account_id,
                payment.amount,
                &payment.currency,
                direction,
                channel,
                category,
                counterparty_ref,
                &format!(
                    "{} — {}",
                    payment.header.document_id,
                    payment.wire_reference.as_deref().unwrap_or("payment")
                ),
                ts,
            );

            // Wire up cross-references (compute once, reuse for mirror below)
            let source_payment_id = payment.header.document_id.clone();
            let source_invoice_id = payment.allocations.first().map(|a| a.invoice_id.clone());
            let je_id = payment.header.journal_entry_id.clone();

            txn.source_payment_id = Some(source_payment_id.clone());
            txn.source_invoice_id = source_invoice_id.clone();
            txn.journal_entry_id = je_id.clone();
            txn.gl_cash_account = enterprise_account.gl_account.clone();
            txn.location_country = house_bank_country.clone();

            // Propagate fraud labels from payment header
            if payment.header.is_fraud {
                let fraud_typology = payment
                    .header
                    .fraud_type
                    .and_then(fraud_type_to_aml_typology)
                    .unwrap_or(AmlTypology::FirstPartyFraud); // generic fallback
                txn = txn.mark_suspicious(fraud_typology, &payment.header.document_id);
                txn = txn.with_laundering_stage(LaunderingStage::Integration);
                txn.ground_truth_explanation = Some(format!(
                    "Payment-to-bank bridge: {fraud_typology:?} fraud propagated from document {} (${:.2})",
                    payment.header.document_id,
                    payment.amount,
                ));
                stats.fraud_propagated += 1;
            }

            let primary_txn_id = txn.transaction_id;
            transactions.push(txn);
            stats.bridged_count += 1;
            stats.transactions_emitted += 1;

            // If counterparty has a banking profile, emit a mirror transaction on their side
            if let Some(bc) = counterparty_banking {
                if let Some(counterparty_account) = customer_to_account.get(&bc.customer_id) {
                    let mirror_direction = match direction {
                        Direction::Inbound => Direction::Outbound,
                        Direction::Outbound => Direction::Inbound,
                    };
                    let mirror_counterparty = CounterpartyRef {
                        counterparty_type: crate::models::CounterpartyType::FinancialInstitution,
                        counterparty_id: None,
                        name: format!("House Bank — {}", payment.header.company_code),
                        account_identifier: Some(payment.bank_account_id.clone()),
                        bank_identifier: None,
                        country: house_bank_country.clone(),
                    };

                    let mut mirror_txn = BankTransaction::new(
                        self.uuid_factory.next(),
                        counterparty_account.account_id,
                        payment.amount,
                        &payment.currency,
                        mirror_direction,
                        channel,
                        match mirror_direction {
                            Direction::Inbound => TransactionCategory::TransferIn,
                            Direction::Outbound => TransactionCategory::TransferOut,
                        },
                        mirror_counterparty,
                        &format!("Mirror of {}", payment.header.document_id),
                        ts,
                    );

                    mirror_txn.source_payment_id = Some(source_payment_id.clone());
                    mirror_txn.source_invoice_id = source_invoice_id.clone();
                    mirror_txn.journal_entry_id = je_id.clone();
                    mirror_txn.gl_cash_account = counterparty_account.gl_account.clone();
                    // Link mirror to primary for graph traversal
                    mirror_txn.parent_transaction_id = Some(primary_txn_id);

                    if payment.header.is_fraud {
                        let fraud_typology = payment
                            .header
                            .fraud_type
                            .and_then(fraud_type_to_aml_typology)
                            .unwrap_or(AmlTypology::FirstPartyFraud);
                        mirror_txn =
                            mirror_txn.mark_suspicious(fraud_typology, &payment.header.document_id);
                        mirror_txn = mirror_txn.with_laundering_stage(LaunderingStage::Integration);
                        mirror_txn.ground_truth_explanation = Some(format!(
                            "Payment-to-bank mirror: {fraud_typology:?} fraud propagated from document {} (${:.2}, counterparty side)",
                            payment.header.document_id,
                            payment.amount,
                        ));
                    }

                    transactions.push(mirror_txn);
                    stats.transactions_emitted += 1;
                }
            }
        }

        (transactions, stats)
    }
}

fn payment_method_to_channel(method: PaymentMethod) -> TransactionChannel {
    match method {
        PaymentMethod::BankTransfer => TransactionChannel::Ach,
        PaymentMethod::Wire => TransactionChannel::Wire,
        PaymentMethod::Check => TransactionChannel::Check,
        PaymentMethod::CreditCard => TransactionChannel::CardNotPresent,
        PaymentMethod::DirectDebit => TransactionChannel::Ach,
        PaymentMethod::Cash => TransactionChannel::Cash,
        PaymentMethod::LetterOfCredit => TransactionChannel::Swift,
    }
}

/// Map FraudType (from accounting layer) to AmlTypology (banking layer).
fn fraud_type_to_aml_typology(fraud: datasynth_core::models::FraudType) -> Option<AmlTypology> {
    use datasynth_core::models::FraudType as F;
    Some(match fraud {
        F::FictitiousTransaction | F::FictitiousVendor | F::FictitiousEntry => {
            AmlTypology::ShellCompany
        }
        F::DuplicatePayment => AmlTypology::FirstPartyFraud,
        F::InvoiceManipulation => AmlTypology::InvoiceManipulation,
        F::Kickback => AmlTypology::Corruption,
        F::RevenueManipulation => AmlTypology::FirstPartyFraud,
        F::AssetMisappropriation | F::InventoryTheft => AmlTypology::FirstPartyFraud,
        F::GhostEmployee => AmlTypology::FirstPartyFraud,
        F::UnauthorizedApproval => AmlTypology::AuthorizedPushPayment,
        _ => AmlTypology::FirstPartyFraud,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    fn make_banking_customer(enterprise_id: &str) -> BankingCustomer {
        let mut c = BankingCustomer::new_business(
            Uuid::new_v4(),
            "Test Vendor Corp",
            "US",
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        );
        c.enterprise_customer_id = Some(enterprise_id.to_string());
        c
    }

    fn make_account(
        owner: Uuid,
        account_type: datasynth_core::models::banking::BankAccountType,
    ) -> BankAccount {
        BankAccount::new(
            Uuid::new_v4(),
            format!("ACC-{owner}"),
            account_type,
            owner,
            "USD",
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        )
    }

    fn make_payment(vendor_id: &str, amount: rust_decimal::Decimal, is_fraud: bool) -> Payment {
        let mut p = Payment::new_ap_payment(
            format!("PAY-{}", uuid::Uuid::new_v4()),
            "COMP001",
            vendor_id,
            amount,
            2024,
            6,
            NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
            "john.doe",
        );
        p.header.is_fraud = is_fraud;
        if is_fraud {
            p.header.fraud_type = Some(datasynth_core::models::FraudType::DuplicatePayment);
        }
        p
    }

    #[test]
    fn test_bridge_creates_bank_transactions() {
        let mut bridge = PaymentBridgeGenerator::new(42);

        let house_bank_customer = make_banking_customer("ENT-HOUSE");
        let vendor_banking = make_banking_customer("V-001");
        let customers = vec![house_bank_customer.clone(), vendor_banking.clone()];

        let house_account = make_account(
            house_bank_customer.customer_id,
            datasynth_core::models::banking::BankAccountType::BusinessOperating,
        );
        let vendor_account = make_account(
            vendor_banking.customer_id,
            datasynth_core::models::banking::BankAccountType::BusinessOperating,
        );
        let accounts = vec![house_account.clone(), vendor_account.clone()];

        let payments = vec![make_payment("V-001", dec!(50_000), false)];

        let (txns, stats) = bridge.bridge_payments(&payments, &customers, &accounts, 1.0);

        assert!(stats.bridged_count > 0, "Should bridge the payment");
        assert!(txns.iter().any(|t| t.source_payment_id.is_some()));
        // Should create mirror transaction since vendor has banking profile
        assert_eq!(
            txns.len(),
            2,
            "Should emit 2 transactions: enterprise + mirror"
        );
    }

    #[test]
    fn test_fraud_propagation_across_layers() {
        let mut bridge = PaymentBridgeGenerator::new(42);

        let customers = vec![make_banking_customer("V-001")];
        let accounts = vec![make_account(
            customers[0].customer_id,
            datasynth_core::models::banking::BankAccountType::BusinessOperating,
        )];

        let fraudulent_payment = make_payment("V-001", dec!(100_000), true);
        let payments = vec![fraudulent_payment];

        let (txns, stats) = bridge.bridge_payments(&payments, &customers, &accounts, 1.0);

        assert!(stats.fraud_propagated > 0);
        assert!(txns.iter().any(|t| t.is_suspicious));
        assert!(txns.iter().any(|t| t.ground_truth_explanation.is_some()));
    }

    #[test]
    fn test_external_counterparty_no_mirror() {
        let mut bridge = PaymentBridgeGenerator::new(42);

        // Only house bank has a banking profile; vendor "V-999" does not
        let house = make_banking_customer("ENT-HOUSE");
        let customers = vec![house.clone()];
        let accounts = vec![make_account(
            house.customer_id,
            datasynth_core::models::banking::BankAccountType::BusinessOperating,
        )];

        let payments = vec![make_payment("V-999", dec!(25_000), false)];

        let (txns, _) = bridge.bridge_payments(&payments, &customers, &accounts, 1.0);

        // Only 1 txn (no mirror on vendor side)
        assert_eq!(txns.len(), 1);
        // Counterparty has no banking_id but has the vendor ID in the name
        assert_eq!(txns[0].counterparty.name, "V-999");
    }

    #[test]
    fn test_bridge_rate_zero_produces_nothing() {
        let mut bridge = PaymentBridgeGenerator::new(42);
        let customers = vec![make_banking_customer("V-001")];
        let accounts = vec![make_account(
            customers[0].customer_id,
            datasynth_core::models::banking::BankAccountType::BusinessOperating,
        )];
        let payments = vec![make_payment("V-001", dec!(1000), false)];
        let (txns, _) = bridge.bridge_payments(&payments, &customers, &accounts, 0.0);
        assert!(txns.is_empty());
    }
}

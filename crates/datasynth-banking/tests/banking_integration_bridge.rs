//! End-to-end integration test: verifies banking is coherently connected
//! to document flows, GL accounts, and fraud labels.
//!
//! This is the critical test that proves the banking module is no longer a
//! silo. A vendor payment should appear in BOTH the document flow AND the
//! banking layer with cross-references, and fraud labels should propagate.

use chrono::NaiveDate;
use datasynth_banking::generators::payment_bridge::PaymentBridgeGenerator;
use datasynth_banking::models::{BankAccount, BankingCustomer};
use datasynth_core::documents::Payment;
use datasynth_core::models::banking::BankAccountType;
use datasynth_core::models::FraudType;
use rust_decimal_macros::dec;
use uuid::Uuid;

fn setup_enterprise_customer() -> BankingCustomer {
    let mut c = BankingCustomer::new_business(
        Uuid::new_v4(),
        "Enterprise Holding",
        "US",
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
    );
    c.enterprise_customer_id = Some("ENT-HOUSE".to_string());
    c
}

fn setup_vendor_customer() -> BankingCustomer {
    let mut c = BankingCustomer::new_business(
        Uuid::new_v4(),
        "Acme Supplies",
        "US",
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
    );
    c.enterprise_customer_id = Some("V-001".to_string());
    c
}

fn setup_account(owner: Uuid, account_type: BankAccountType) -> BankAccount {
    BankAccount::new(
        Uuid::new_v4(),
        format!("ACC-{owner}"),
        account_type,
        owner,
        "USD",
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
    )
}

fn make_vendor_payment(
    vendor_id: &str,
    amount: rust_decimal::Decimal,
    fraud_type: Option<FraudType>,
) -> Payment {
    let mut p = Payment::new_ap_payment(
        format!("PAY-{}", Uuid::new_v4()),
        "COMP001",
        vendor_id,
        amount,
        2024,
        6,
        NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        "john.doe",
    );
    p.header.journal_entry_id = Some(format!("JE-{}", Uuid::new_v4()));
    if let Some(ft) = fraud_type {
        p.header.is_fraud = true;
        p.header.fraud_type = Some(ft);
    }
    p
}

#[test]
fn test_bank_accounts_have_gl_codes() {
    // Phase 2 verification: BankAccount should auto-populate gl_account from account type
    let enterprise = setup_enterprise_customer();
    let checking = setup_account(enterprise.customer_id, BankAccountType::Checking);
    let business = setup_account(enterprise.customer_id, BankAccountType::BusinessOperating);
    let trust = setup_account(enterprise.customer_id, BankAccountType::TrustAccount);

    assert!(
        checking.gl_account.is_some(),
        "Checking should have GL account"
    );
    assert!(
        business.gl_account.is_some(),
        "Business should have GL account"
    );
    assert!(trust.gl_account.is_some(), "Trust should have GL account");

    // Different account types should get different GL codes
    assert_ne!(checking.gl_account, trust.gl_account);
}

#[test]
fn test_payment_bridge_creates_cross_referenced_bank_transaction() {
    // Phase 1 verification: A vendor payment produces a bank transaction
    // with bidirectional links
    let enterprise = setup_enterprise_customer();
    let vendor = setup_vendor_customer();
    let customers = vec![enterprise.clone(), vendor.clone()];

    let house_account = setup_account(enterprise.customer_id, BankAccountType::BusinessOperating);
    let vendor_account = setup_account(vendor.customer_id, BankAccountType::BusinessOperating);
    let accounts = vec![house_account.clone(), vendor_account.clone()];

    let payment = make_vendor_payment("V-001", dec!(50_000), None);
    let payment_id = payment.header.document_id.clone();
    let je_id = payment.header.journal_entry_id.clone();

    let mut bridge = PaymentBridgeGenerator::new(42);
    let (txns, stats) = bridge.bridge_payments(&[payment], &customers, &accounts, 1.0);

    // Should produce 2 transactions: one on enterprise side (outbound), one on vendor side (inbound mirror)
    assert_eq!(txns.len(), 2, "Should produce enterprise + vendor mirror");
    assert_eq!(stats.bridged_count, 1);

    // Cross-references should link back to the payment and JE
    for txn in &txns {
        assert_eq!(txn.source_payment_id, Some(payment_id.clone()));
        assert_eq!(txn.journal_entry_id, je_id.clone());
        assert!(txn.gl_cash_account.is_some(), "Should have GL cash account");
    }

    // One should be outbound (AP payment leaving house bank), other inbound (vendor receiving)
    let out_count = txns
        .iter()
        .filter(|t| {
            matches!(
                t.direction,
                datasynth_core::models::banking::Direction::Outbound
            )
        })
        .count();
    let in_count = txns
        .iter()
        .filter(|t| {
            matches!(
                t.direction,
                datasynth_core::models::banking::Direction::Inbound
            )
        })
        .count();
    assert_eq!(out_count, 1, "One outbound from enterprise");
    assert_eq!(in_count, 1, "One inbound to vendor");
}

#[test]
fn test_fraud_labels_propagate_across_layers() {
    // The most important test: a fraudulent payment should produce fraudulent
    // bank transactions, with the fraud visible in BOTH layers
    let enterprise = setup_enterprise_customer();
    let vendor = setup_vendor_customer();
    let customers = vec![enterprise.clone(), vendor.clone()];

    let house_account = setup_account(enterprise.customer_id, BankAccountType::BusinessOperating);
    let vendor_account = setup_account(vendor.customer_id, BankAccountType::BusinessOperating);
    let accounts = vec![house_account, vendor_account];

    // Fraudulent duplicate payment
    let payment = make_vendor_payment("V-001", dec!(100_000), Some(FraudType::DuplicatePayment));

    let mut bridge = PaymentBridgeGenerator::new(42);
    let (txns, stats) = bridge.bridge_payments(&[payment], &customers, &accounts, 1.0);

    assert_eq!(stats.fraud_propagated, 1, "Fraud should propagate once");

    // All resulting bank transactions should be marked suspicious
    for txn in &txns {
        assert!(txn.is_suspicious, "Bank txn should inherit fraud");
        assert!(txn.suspicion_reason.is_some());
        assert!(txn.ground_truth_explanation.is_some());
    }
}

#[test]
fn test_bridge_preserves_payment_reference_chain() {
    // Trace-ability test: from a bank transaction, we should be able to
    // reach back to the payment, invoice, and journal entry
    let enterprise = setup_enterprise_customer();
    let customers = vec![enterprise.clone()];
    let account = setup_account(enterprise.customer_id, BankAccountType::BusinessOperating);
    let accounts = vec![account];

    // Payment with allocation to an invoice
    let mut payment = make_vendor_payment("V-999", dec!(25_000), None);
    payment
        .allocations
        .push(datasynth_core::documents::PaymentAllocation::new(
            "VI-ACME-001",
            datasynth_core::documents::DocumentType::VendorInvoice,
            dec!(25_000),
        ));

    let mut bridge = PaymentBridgeGenerator::new(42);
    let (txns, _) = bridge.bridge_payments(&[payment.clone()], &customers, &accounts, 1.0);

    assert!(!txns.is_empty());
    let bank_txn = &txns[0];

    // Full reference chain: bank → payment → invoice → JE
    assert_eq!(
        bank_txn.source_payment_id,
        Some(payment.header.document_id.clone())
    );
    assert_eq!(bank_txn.source_invoice_id, Some("VI-ACME-001".to_string()));
    assert_eq!(
        bank_txn.journal_entry_id,
        payment.header.journal_entry_id.clone()
    );
}

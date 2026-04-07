//! Integration tests for IC source transaction generation.
//!
//! Verifies that `ICGenerator::generate_ic_document_chains` produces
//! real P2P/O2C documents (PurchaseOrder, GoodsReceipt, VendorInvoice,
//! CustomerInvoice) from IC matched pairs.

use chrono::NaiveDate;
use datasynth_core::models::documents::CustomerInvoiceType;
use datasynth_core::models::intercompany::{
    EliminationType, ICMatchedPair, ICTransactionType, IntercompanyRelationship, OwnershipStructure,
};
use datasynth_generators::intercompany::{
    EliminationConfig, EliminationGenerator, ICGenerator, ICGeneratorConfig, ICMatchingConfig,
    ICMatchingEngine,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::str::FromStr;

fn create_test_ownership_structure() -> OwnershipStructure {
    let mut structure = OwnershipStructure::new("1000".to_string());
    structure.add_relationship(IntercompanyRelationship::new(
        "REL001".to_string(),
        "1000".to_string(),
        "1100".to_string(),
        dec!(100),
        NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(),
    ));
    structure.add_relationship(IntercompanyRelationship::new(
        "REL002".to_string(),
        "1000".to_string(),
        "1200".to_string(),
        dec!(100),
        NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(),
    ));
    structure
}

fn create_test_pairs() -> Vec<ICMatchedPair> {
    let date = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
    let types = [
        ICTransactionType::GoodsSale,
        ICTransactionType::ServiceProvided,
        ICTransactionType::ManagementFee,
        ICTransactionType::Royalty,
        ICTransactionType::ExpenseRecharge,
    ];

    types
        .iter()
        .enumerate()
        .map(|(i, &tx_type)| {
            let mut pair = ICMatchedPair::new(
                format!("IC2024{:06}", i + 1),
                tx_type,
                "1000".to_string(),
                "1100".to_string(),
                dec!(10000) + Decimal::from(i as u32) * dec!(5000),
                "USD".to_string(),
                date,
            );
            pair.seller_document = format!("ICS{:08}", i + 1);
            pair.buyer_document = format!("ICB{:08}", i + 1);
            pair
        })
        .collect()
}

#[test]
fn test_generates_nonempty_documents() {
    let config = ICGeneratorConfig::default();
    let structure = create_test_ownership_structure();
    let mut generator = ICGenerator::new(config, structure, 42);
    let pairs = create_test_pairs();

    let chains = generator.generate_ic_document_chains(&pairs);

    assert_eq!(chains.seller_invoices.len(), 5);
    assert_eq!(chains.buyer_orders.len(), 5);
    assert_eq!(chains.buyer_goods_receipts.len(), 5);
    assert_eq!(chains.buyer_invoices.len(), 5);
}

#[test]
fn test_ineligible_types_are_skipped() {
    let config = ICGeneratorConfig::default();
    let structure = create_test_ownership_structure();
    let mut generator = ICGenerator::new(config, structure, 42);

    let date = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
    // LoanInterest and Dividend are NOT eligible
    let pairs: Vec<ICMatchedPair> = [
        ICTransactionType::LoanInterest,
        ICTransactionType::Dividend,
        ICTransactionType::Loan,
        ICTransactionType::CostSharing,
    ]
    .iter()
    .enumerate()
    .map(|(i, &tx_type)| {
        let mut pair = ICMatchedPair::new(
            format!("IC2024{:06}", i + 100),
            tx_type,
            "1000".to_string(),
            "1100".to_string(),
            dec!(50000),
            "USD".to_string(),
            date,
        );
        pair.seller_document = format!("ICS{:08}", i + 100);
        pair.buyer_document = format!("ICB{:08}", i + 100);
        pair
    })
    .collect();

    let chains = generator.generate_ic_document_chains(&pairs);

    assert!(chains.seller_invoices.is_empty());
    assert!(chains.buyer_orders.is_empty());
    assert!(chains.buyer_goods_receipts.is_empty());
    assert!(chains.buyer_invoices.is_empty());
}

#[test]
fn test_ic_reference_links_documents() {
    let config = ICGeneratorConfig::default();
    let structure = create_test_ownership_structure();
    let mut generator = ICGenerator::new(config, structure, 42);
    let pairs = create_test_pairs();

    let chains = generator.generate_ic_document_chains(&pairs);

    for (i, pair) in pairs.iter().enumerate() {
        let ci = &chains.seller_invoices[i];
        let po = &chains.buyer_orders[i];
        let gr = &chains.buyer_goods_receipts[i];
        let vi = &chains.buyer_invoices[i];

        // All documents carry the IC reference
        assert_eq!(
            ci.header.reference.as_deref(),
            Some(pair.ic_reference.as_str()),
            "seller invoice ic_reference mismatch for pair {i}"
        );
        assert_eq!(
            po.header.reference.as_deref(),
            Some(pair.ic_reference.as_str()),
            "buyer PO ic_reference mismatch for pair {i}"
        );
        assert_eq!(
            gr.header.reference.as_deref(),
            Some(pair.ic_reference.as_str()),
            "buyer GR ic_reference mismatch for pair {i}"
        );
        assert_eq!(
            vi.header.reference.as_deref(),
            Some(pair.ic_reference.as_str()),
            "buyer VI ic_reference mismatch for pair {i}"
        );
    }
}

#[test]
fn test_seller_invoice_is_intercompany() {
    let config = ICGeneratorConfig::default();
    let structure = create_test_ownership_structure();
    let mut generator = ICGenerator::new(config, structure, 42);
    let pairs = create_test_pairs();

    let chains = generator.generate_ic_document_chains(&pairs);

    for ci in &chains.seller_invoices {
        assert_eq!(
            ci.invoice_type,
            CustomerInvoiceType::Intercompany,
            "invoice_type should be Intercompany"
        );
        assert!(ci.is_intercompany, "is_intercompany should be true");
    }
}

#[test]
fn test_company_codes_correct() {
    let config = ICGeneratorConfig::default();
    let structure = create_test_ownership_structure();
    let mut generator = ICGenerator::new(config, structure, 42);
    let pairs = create_test_pairs();

    let chains = generator.generate_ic_document_chains(&pairs);

    for (i, pair) in pairs.iter().enumerate() {
        // Seller invoice belongs to seller company, customer = buyer
        let ci = &chains.seller_invoices[i];
        assert_eq!(ci.header.company_code, pair.seller_company);
        assert_eq!(ci.customer_id, pair.buyer_company);

        // Buyer PO belongs to buyer company, vendor = seller
        let po = &chains.buyer_orders[i];
        assert_eq!(po.header.company_code, pair.buyer_company);
        assert_eq!(po.vendor_id, pair.seller_company);

        // Buyer GR belongs to buyer company
        let gr = &chains.buyer_goods_receipts[i];
        assert_eq!(gr.header.company_code, pair.buyer_company);
        assert_eq!(gr.vendor_id.as_deref(), Some(pair.seller_company.as_str()));

        // Buyer VI belongs to buyer company, vendor = seller
        let vi = &chains.buyer_invoices[i];
        assert_eq!(vi.header.company_code, pair.buyer_company);
        assert_eq!(vi.vendor_id, pair.seller_company);
    }
}

#[test]
fn test_amounts_match_pair() {
    let config = ICGeneratorConfig::default();
    let structure = create_test_ownership_structure();
    let mut generator = ICGenerator::new(config, structure, 42);
    let pairs = create_test_pairs();

    let chains = generator.generate_ic_document_chains(&pairs);

    for (i, pair) in pairs.iter().enumerate() {
        let ci = &chains.seller_invoices[i];
        assert_eq!(
            ci.total_net_amount, pair.amount,
            "seller invoice net amount mismatch for pair {i}"
        );

        let po = &chains.buyer_orders[i];
        assert_eq!(
            po.total_net_amount, pair.amount,
            "buyer PO net amount mismatch for pair {i}"
        );

        let gr = &chains.buyer_goods_receipts[i];
        assert_eq!(
            gr.total_value, pair.amount,
            "buyer GR total value mismatch for pair {i}"
        );

        let vi = &chains.buyer_invoices[i];
        assert_eq!(
            vi.net_amount, pair.amount,
            "buyer VI net amount mismatch for pair {i}"
        );
    }
}

#[test]
fn test_buyer_gr_references_po() {
    let config = ICGeneratorConfig::default();
    let structure = create_test_ownership_structure();
    let mut generator = ICGenerator::new(config, structure, 42);
    let pairs = create_test_pairs();

    let chains = generator.generate_ic_document_chains(&pairs);

    for i in 0..pairs.len() {
        let po = &chains.buyer_orders[i];
        let gr = &chains.buyer_goods_receipts[i];

        assert_eq!(
            gr.purchase_order_id.as_deref(),
            Some(po.header.document_id.as_str()),
            "GR should reference its PO for pair {i}"
        );
    }
}

#[test]
fn test_buyer_vi_references_po_and_gr() {
    let config = ICGeneratorConfig::default();
    let structure = create_test_ownership_structure();
    let mut generator = ICGenerator::new(config, structure, 42);
    let pairs = create_test_pairs();

    let chains = generator.generate_ic_document_chains(&pairs);

    for i in 0..pairs.len() {
        let po = &chains.buyer_orders[i];
        let gr = &chains.buyer_goods_receipts[i];
        let vi = &chains.buyer_invoices[i];

        assert_eq!(
            vi.purchase_order_id.as_deref(),
            Some(po.header.document_id.as_str()),
            "VI should reference PO for pair {i}"
        );
        assert_eq!(
            vi.goods_receipt_id.as_deref(),
            Some(gr.header.document_id.as_str()),
            "VI should reference GR for pair {i}"
        );
    }
}

#[test]
fn test_with_generator_produced_pairs() {
    let config = ICGeneratorConfig {
        ic_transaction_rate: 1.0,
        ..Default::default()
    };
    let structure = create_test_ownership_structure();
    let mut generator = ICGenerator::new(config, structure, 99);

    let start = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2024, 3, 5).unwrap();
    let pairs = generator.generate_transactions_for_period(start, end, 2);

    // With rate=1.0, 5 days * 2/day = 10 pairs
    assert_eq!(pairs.len(), 10);

    let chains = generator.generate_ic_document_chains(&pairs);

    // At least some pairs should be eligible types
    assert!(
        !chains.seller_invoices.is_empty(),
        "expected at least one seller invoice from generated pairs"
    );
    assert_eq!(
        chains.seller_invoices.len(),
        chains.buyer_orders.len(),
        "seller_invoices and buyer_orders counts should match"
    );
    assert_eq!(
        chains.buyer_orders.len(),
        chains.buyer_goods_receipts.len(),
        "buyer_orders and buyer_goods_receipts counts should match"
    );
    assert_eq!(
        chains.buyer_goods_receipts.len(),
        chains.buyer_invoices.len(),
        "buyer_goods_receipts and buyer_invoices counts should match"
    );
}

#[test]
fn test_eliminations_use_actual_ic_amounts() {
    // Generate IC transactions
    let ownership = create_test_ownership_structure();
    let config = ICGeneratorConfig::default();
    let mut gen = ICGenerator::new(config, ownership.clone(), 42);

    let start = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
    let pairs = gen.generate_transactions_for_period(start, end, 2);
    assert!(!pairs.is_empty());

    // Run matching
    let matching_config = ICMatchingConfig::default();
    let mut matching = ICMatchingEngine::new(matching_config);
    matching.load_matched_pairs(&pairs);
    let _result = matching.run_matching(end);

    // Collect balances as owned values for elimination
    let balances: Vec<_> = matching.get_balances().into_iter().cloned().collect();

    // Generate eliminations
    let elim_config = EliminationConfig::default();
    let mut elim_gen = EliminationGenerator::new(elim_config, ownership);

    let journal = elim_gen.generate_eliminations(
        "202501",
        end,
        &balances,
        &pairs,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
    );

    // The IC revenue/expense elimination total should relate to actual IC amounts
    let total_ic: Decimal = pairs
        .iter()
        .filter(|p| {
            matches!(
                p.transaction_type,
                ICTransactionType::GoodsSale | ICTransactionType::ServiceProvided
            )
        })
        .map(|p| p.amount)
        .sum();

    let total_elim: Decimal = journal
        .entries
        .iter()
        .filter(|e| matches!(e.elimination_type, EliminationType::ICRevenueExpense))
        .map(|e| e.total_debit)
        .sum();

    // They should be close (may not be exact due to rounding or netting)
    if total_ic > Decimal::ZERO && total_elim > Decimal::ZERO {
        let ratio = total_elim / total_ic;
        assert!(
            ratio > Decimal::from_str("0.5").unwrap() && ratio < Decimal::from_str("2.0").unwrap(),
            "Elimination total {} should be proportional to IC total {}, ratio={}",
            total_elim,
            total_ic,
            ratio
        );
    }
}

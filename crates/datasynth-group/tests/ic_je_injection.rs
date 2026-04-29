//! Integration tests for
//! `datasynth_group::shard::ic_je_injector::inject_ic_journal_entries`.
//!
//! These tests build minimal `IcPairPlan`s inline rather than going through
//! the full manifest machinery — the injector's contract is "one plan → one
//! balanced JE", and the plan assembly logic is already covered by the
//! existing `shard_plan` tests.

use chrono::NaiveDate;
use rust_decimal::Decimal;

use datasynth_core::models::IcPairId;
use datasynth_group::config::IcTransactionType;
use datasynth_group::shard::{inject_ic_journal_entries, IcPairPlan, IcRole, InjectionCtx};

/// Deterministic stand-in for a blake3-derived IcPairId.  Only used so
/// tests can assert field equality without reaching into the seed tree.
fn fake_pair_id(tag: u8) -> IcPairId {
    let mut bytes = [0u8; 32];
    bytes[0] = tag;
    IcPairId::from_bytes(bytes)
}

fn ctx(entity: &str) -> InjectionCtx {
    InjectionCtx {
        entity_code: entity.to_string(),
    }
}

fn sample_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 6, 15).expect("valid date")
}

fn plan(
    role: IcRole,
    tx_type: IcTransactionType,
    partner: &str,
    amount: Decimal,
    index: u64,
) -> IcPairPlan {
    IcPairPlan {
        pair_id: fake_pair_id(index as u8 + 1),
        ic_relationship_id: format!("REL-{index}"),
        role,
        partner_entity: partner.to_string(),
        transaction_type: tx_type,
        amount,
        date: sample_date(),
        index,
    }
}

// ── Sub-part A tests ──────────────────────────────────────────────────────────

#[test]
fn test_seller_goods_sale_debits_ar_credits_revenue() {
    let plans = vec![plan(
        IcRole::Seller,
        IcTransactionType::GoodsSale,
        "E_BUYER",
        Decimal::from(50_000),
        0,
    )];
    let jes = inject_ic_journal_entries(&plans, &ctx("E_SELLER"));

    assert_eq!(jes.len(), 1);
    let je = &jes[0];
    assert_eq!(je.lines.len(), 2);

    // DR line = IC_AR_CLEARING ("1150") with amount 50000.
    let dr = je
        .lines
        .iter()
        .find(|l| l.debit_amount > Decimal::ZERO)
        .expect("debit line");
    assert_eq!(dr.gl_account, "1150");
    assert_eq!(dr.debit_amount, Decimal::from(50_000));

    // CR line = IC_REVENUE ("4500") with amount 50000.
    let cr = je
        .lines
        .iter()
        .find(|l| l.credit_amount > Decimal::ZERO)
        .expect("credit line");
    assert_eq!(cr.gl_account, "4500");
    assert_eq!(cr.credit_amount, Decimal::from(50_000));

    assert!(je.is_balanced());
}

#[test]
fn test_buyer_goods_sale_debits_cogs_credits_ap() {
    let plans = vec![plan(
        IcRole::Buyer,
        IcTransactionType::GoodsSale,
        "E_SELLER",
        Decimal::from(50_000),
        0,
    )];
    let jes = inject_ic_journal_entries(&plans, &ctx("E_BUYER"));

    assert_eq!(jes.len(), 1);
    let je = &jes[0];
    assert_eq!(je.lines.len(), 2);

    // DR line = COGS ("5000") / CR line = IC_AP_CLEARING ("2050").
    let dr = je
        .lines
        .iter()
        .find(|l| l.debit_amount > Decimal::ZERO)
        .expect("debit line");
    assert_eq!(dr.gl_account, "5000");
    assert_eq!(dr.debit_amount, Decimal::from(50_000));
    let cr = je
        .lines
        .iter()
        .find(|l| l.credit_amount > Decimal::ZERO)
        .expect("credit line");
    assert_eq!(cr.gl_account, "2050");
    assert_eq!(cr.credit_amount, Decimal::from(50_000));
    assert!(je.is_balanced());
}

#[test]
fn test_ic_fields_populated_on_header() {
    let seller_plan = plan(
        IcRole::Seller,
        IcTransactionType::ServiceProvided,
        "E_BUYER",
        Decimal::from(30_000),
        0,
    );
    let buyer_plan = plan(
        IcRole::Buyer,
        IcTransactionType::ServiceProvided,
        "E_SELLER",
        Decimal::from(30_000),
        0,
    );

    let seller_jes =
        inject_ic_journal_entries(std::slice::from_ref(&seller_plan), &ctx("E_SELLER"));
    let buyer_jes = inject_ic_journal_entries(std::slice::from_ref(&buyer_plan), &ctx("E_BUYER"));

    assert_eq!(seller_jes[0].header.ic_pair_id, Some(seller_plan.pair_id));
    assert_eq!(
        seller_jes[0].header.ic_partner_entity.as_deref(),
        Some("E_BUYER")
    );
    assert_eq!(buyer_jes[0].header.ic_pair_id, Some(buyer_plan.pair_id));
    assert_eq!(
        buyer_jes[0].header.ic_partner_entity.as_deref(),
        Some("E_SELLER")
    );
}

#[test]
fn test_all_tx_types_produce_balanced_jes() {
    let all_types = [
        IcTransactionType::GoodsSale,
        IcTransactionType::ServiceProvided,
        IcTransactionType::ManagementFee,
        IcTransactionType::Royalty,
        IcTransactionType::CostSharing,
        IcTransactionType::LoanInterest,
        IcTransactionType::Dividend,
        IcTransactionType::ExpenseRecharge,
    ];
    let all_roles = [IcRole::Seller, IcRole::Buyer];

    let mut combos = 0;
    for tx_type in all_types {
        for role in all_roles {
            let p = plan(role, tx_type, "E_OTHER", Decimal::from(25_000), 0);
            let jes = inject_ic_journal_entries(&[p], &ctx("E_ME"));
            assert_eq!(jes.len(), 1);
            let je = &jes[0];
            assert!(
                je.is_balanced(),
                "JE not balanced for role={role:?} tx_type={tx_type:?}: dr={} cr={}",
                je.total_debit(),
                je.total_credit()
            );
            // Sanity: exactly one debit and one credit line.
            let (dr_count, cr_count) = je.debit_credit_counts();
            assert_eq!(dr_count, 1, "expected 1 debit for {role:?}/{tx_type:?}");
            assert_eq!(cr_count, 1, "expected 1 credit for {role:?}/{tx_type:?}");
            combos += 1;
        }
    }
    assert_eq!(
        combos, 16,
        "expected 8 tx_types x 2 roles = 16 combinations"
    );
}

#[test]
fn test_company_code_matches_entity() {
    let plans = vec![plan(
        IcRole::Seller,
        IcTransactionType::ManagementFee,
        "E_BUYER",
        Decimal::from(25_000),
        0,
    )];
    let jes = inject_ic_journal_entries(&plans, &ctx("E_PARENT"));
    assert_eq!(jes[0].header.company_code, "E_PARENT");
}

#[test]
fn test_empty_plans_returns_empty_vec() {
    let jes = inject_ic_journal_entries(&[], &ctx("E_ANYTHING"));
    assert!(jes.is_empty());
}

#[test]
fn test_posting_date_matches_plan_date() {
    let custom_date = NaiveDate::from_ymd_opt(2024, 3, 7).expect("valid date");
    let p = IcPairPlan {
        pair_id: fake_pair_id(1),
        ic_relationship_id: "REL-X".to_string(),
        role: IcRole::Seller,
        partner_entity: "E_BUYER".to_string(),
        transaction_type: IcTransactionType::Royalty,
        amount: Decimal::from(100_000),
        date: custom_date,
        index: 0,
    };
    let jes = inject_ic_journal_entries(&[p], &ctx("E_SELLER"));
    assert_eq!(jes[0].header.posting_date, custom_date);
}

#[test]
fn test_deterministic_across_calls() {
    // Note: `JournalEntryHeader::new` uses `Uuid::now_v7()` which is
    // time-based, so `document_id` (and the document_date default)
    // will differ between calls.  We assert on the IC-specific header
    // fields and on line accounts / amounts — the deterministic surface
    // the shard-runner relies on.
    let p = plan(
        IcRole::Seller,
        IcTransactionType::GoodsSale,
        "E_BUYER",
        Decimal::from(50_000),
        0,
    );
    let ctx1 = ctx("E_SELLER");
    let a = inject_ic_journal_entries(std::slice::from_ref(&p), &ctx1);
    let b = inject_ic_journal_entries(std::slice::from_ref(&p), &ctx1);

    assert_eq!(a.len(), b.len());
    assert_eq!(a[0].header.ic_pair_id, b[0].header.ic_pair_id);
    assert_eq!(a[0].header.ic_partner_entity, b[0].header.ic_partner_entity);
    assert_eq!(a[0].header.company_code, b[0].header.company_code);
    assert_eq!(a[0].header.posting_date, b[0].header.posting_date);
    assert_eq!(a[0].header.header_text, b[0].header.header_text);

    // Lines should match on (gl_account, debit_amount, credit_amount).
    assert_eq!(a[0].lines.len(), b[0].lines.len());
    for (la, lb) in a[0].lines.iter().zip(b[0].lines.iter()) {
        assert_eq!(la.gl_account, lb.gl_account);
        assert_eq!(la.debit_amount, lb.debit_amount);
        assert_eq!(la.credit_amount, lb.credit_amount);
    }
}

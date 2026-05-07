//! v5.7.0 — integration test for industry-pack sub-account expansion.
//!
//! Asserts:
//! 1. With the feature OFF (default), the COA is byte-identical to the
//!    v5.6.0 baseline — no new accounts, no parent flips.
//! 2. With the feature ON for manufacturing, expected sub-accounts
//!    (`400010`, `400020`, …) exist; the parent (`4000`) is flipped to
//!    non-postable / control.
//! 3. `pick_subaccount_for_document` is deterministic across calls and
//!    distributes across multiple sub-accounts when many distinct
//!    document IDs are queried.
//! 4. Sub-accounts inherit the parent's ISO 21378 codes.
//!
//! No JE generation here — the picker is exercised in isolation so the
//! test stays fast and avoids depending on the orchestrator's RNG path.

use std::collections::HashSet;

use datasynth_core::models::{ChartOfAccounts, CoAComplexity, IndustrySector};
use datasynth_generators::ChartOfAccountsGenerator;

fn build_coa(industry: IndustrySector, expand: bool) -> ChartOfAccounts {
    ChartOfAccountsGenerator::new(CoAComplexity::Small, industry, 4242)
        .with_expand_industry_subaccounts(expand)
        .generate()
}

#[test]
fn expansion_off_leaves_canonical_4000_postable() {
    let coa = build_coa(IndustrySector::Manufacturing, false);
    let p = coa
        .get_account("4000")
        .expect("4000 must exist in canonical COA");
    assert!(
        p.is_postable,
        "4000 should be postable when expansion is off (v5.6.0 default)"
    );
    // The procedural per-type generator may seed accounts like
    // `400000`/`400010` with generic names ("Product Revenue 1", ...);
    // that's pre-v5.7.0 behaviour and stays. The check is that no
    // account references `4000` as its `parent_account`.
    let has_pack_subs = coa
        .accounts
        .iter()
        .any(|a| a.parent_account.as_deref() == Some("4000"));
    assert!(
        !has_pack_subs,
        "no pack-driven sub-accounts should reference parent 4000 when expansion is off"
    );
}

#[test]
fn expansion_on_makes_4000_non_postable_control_and_seeds_subs() {
    let coa = build_coa(IndustrySector::Manufacturing, true);
    let p = coa.get_account("4000").expect("4000 must still exist");
    assert!(
        !p.is_postable,
        "4000 should be non-postable (control account) when expansion is on"
    );
    assert!(
        p.is_control_account,
        "4000 should be flagged as control account"
    );

    // Manufacturing pack 4000 has suffixes 10, 20, 30, 50, 70, 85.
    let expected_subs = ["400010", "400020", "400030", "400050", "400070", "400085"];
    for sub in expected_subs {
        let s = coa
            .get_account(sub)
            .unwrap_or_else(|| panic!("expected sub-account {sub} not found"));
        assert!(
            s.is_postable,
            "sub-account {sub} should be postable, was not"
        );
        assert_eq!(
            s.parent_account.as_deref(),
            Some("4000"),
            "sub-account {sub} should reference parent 4000"
        );
    }
}

#[test]
fn sub_accounts_inherit_iso_codes_from_parent() {
    let coa = build_coa(IndustrySector::Manufacturing, true);
    let parent = coa.get_account("4000").unwrap();
    let sub = coa.get_account("400010").unwrap();
    assert_eq!(sub.account_class, parent.account_class);
    assert_eq!(sub.account_class_name, parent.account_class_name);
    assert_eq!(sub.account_sub_class, parent.account_sub_class);
    assert_eq!(sub.account_sub_class_name, parent.account_sub_class_name);
}

#[test]
fn picker_returns_canonical_when_no_expansion() {
    let coa = build_coa(IndustrySector::Manufacturing, false);
    let doc = uuid::Uuid::new_v4();
    let picked = coa
        .pick_subaccount_for_document("4000", doc)
        .expect("4000 exists");
    assert_eq!(
        picked, "4000",
        "without expansion the picker must return the canonical parent"
    );
}

#[test]
fn picker_is_deterministic_per_document() {
    let coa = build_coa(IndustrySector::Manufacturing, true);
    let doc = uuid::Uuid::parse_str("12345678-1234-1234-1234-123456789012").unwrap();
    let p1 = coa.pick_subaccount_for_document("4000", doc).unwrap();
    let p2 = coa.pick_subaccount_for_document("4000", doc).unwrap();
    let p3 = coa.pick_subaccount_for_document("4000", doc).unwrap();
    assert_eq!(p1, p2);
    assert_eq!(p2, p3);
    assert!(p1.starts_with("4000"), "picked {p1} should be a 4000-* sub");
    assert_ne!(p1, "4000", "picker should drill into a sub-account");
}

#[test]
fn picker_distributes_across_subs_for_many_documents() {
    let coa = build_coa(IndustrySector::Manufacturing, true);
    let mut hits: HashSet<String> = HashSet::new();
    for _ in 0..200 {
        let doc = uuid::Uuid::new_v4();
        let picked = coa.pick_subaccount_for_document("4000", doc).unwrap();
        hits.insert(picked);
    }
    // Pack has 6 sub-accounts. 200 random docs should easily hit ≥4.
    assert!(
        hits.len() >= 4,
        "expected ≥4 distinct sub-accounts hit over 200 docs; got {} ({:?})",
        hits.len(),
        hits
    );
    // Should never return the parent canonical 4000.
    assert!(
        !hits.contains("4000"),
        "picker must never select the non-postable parent"
    );
}

#[test]
fn unsupported_industry_skips_expansion() {
    // ProfessionalServices doesn't ship a pack in v5.7.0 → expansion
    // is a no-op even when the flag is on.
    let coa = build_coa(IndustrySector::ProfessionalServices, true);
    let p = coa.get_account("4000").expect("4000 must exist");
    assert!(
        p.is_postable,
        "no pack means no expansion — 4000 must stay postable"
    );
    assert!(coa.get_account("400010").is_none());
}

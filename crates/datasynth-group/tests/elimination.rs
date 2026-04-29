//! Task 5.4 — `EliminationEntry` generation integration tests.
//!
//! These tests reuse the same trimmed `mini_nestle.yaml` two-entity
//! fixture as the IC matcher tests so the elimination engine sees a
//! realistic SA→USA goods/royalty + management-fee pattern.  Fixture
//! builders are deliberately copy-paste from `ic_matcher.rs` rather
//! than shared via a test-utils module — keeping each integration test
//! file self-contained makes it obvious how the inputs were assembled.

use std::collections::BTreeMap;

use datasynth_core::models::intercompany::EliminationType;
use datasynth_core::models::JournalEntry;
use datasynth_group::config::IcRelationshipConfig;
use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::shard::{
    derive_ic_pair_plans, inject_ic_journal_entries, IcRole, InjectionCtx,
};
use datasynth_group::{build_manifest, generate_eliminations, match_ic_pairs, GroupConfig};

// ── Fixture builders ──────────────────────────────────────────────────────────

fn load_two_entity_manifest() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let mut cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse into GroupConfig");

    cfg.ownership
        .entities
        .retain(|e| e.code == "NESTLE_SA" || e.code == "NESTLE_USA");

    cfg.intercompany.relationships.retain(|rel| match rel {
        IcRelationshipConfig::Explicit(e) => {
            (e.seller == "NESTLE_SA" || e.seller == "NESTLE_USA")
                && (e.buyer == "NESTLE_SA" || e.buyer == "NESTLE_USA")
        }
        IcRelationshipConfig::Pattern(_) => true,
    });

    if let Some(p2) = cfg.tax.pillar_two.as_mut() {
        p2.jurisdictions.retain(|j| j == "CH" || j == "US");
    }
    if let Some(tp) = cfg.tax.transfer_pricing.as_mut() {
        tp.local_files_for.retain(|j| j == "CH" || j == "US");
    }

    build_manifest(&cfg).expect("trimmed mini_nestle must still build a manifest")
}

fn load_two_entity_manifest_no_ic() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let mut cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse into GroupConfig");
    cfg.ownership
        .entities
        .retain(|e| e.code == "NESTLE_SA" || e.code == "NESTLE_USA");
    cfg.intercompany.relationships.clear();
    if let Some(p2) = cfg.tax.pillar_two.as_mut() {
        p2.jurisdictions.retain(|j| j == "CH" || j == "US");
    }
    if let Some(tp) = cfg.tax.transfer_pricing.as_mut() {
        tp.local_files_for.retain(|j| j == "CH" || j == "US");
    }
    build_manifest(&cfg).expect("manifest with no IC relationships must still build")
}

fn sa_jes(manifest: &GroupManifest) -> Vec<JournalEntry> {
    let plans = derive_ic_pair_plans(manifest, "NESTLE_SA");
    inject_ic_journal_entries(
        &plans,
        &InjectionCtx {
            entity_code: "NESTLE_SA".to_string(),
        },
    )
}

fn usa_jes(manifest: &GroupManifest) -> Vec<JournalEntry> {
    let plans = derive_ic_pair_plans(manifest, "NESTLE_USA");
    inject_ic_journal_entries(
        &plans,
        &InjectionCtx {
            entity_code: "NESTLE_USA".to_string(),
        },
    )
}

fn expected_pair_count(manifest: &GroupManifest) -> usize {
    [&"NESTLE_SA", &"NESTLE_USA"]
        .iter()
        .map(|code| {
            derive_ic_pair_plans(manifest, code)
                .into_iter()
                .filter(|p| p.role == IcRole::Seller)
                .count()
        })
        .sum()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Happy path: every matched pair produces exactly two elimination
/// entries (`ICBalances` + `ICRevenueExpense`) for the trimmed
/// SA→USA fixture, which uses goods_sale, royalty, and management_fee
/// — none of which trigger the dividend or interest tracks.
#[test]
fn happy_path_emits_two_entries_per_pair() {
    let manifest = load_two_entity_manifest();
    let total = expected_pair_count(&manifest);
    assert!(
        total >= 1,
        "fixture sanity: trimmed manifest must yield IC pairs"
    );

    let match_result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), sa_jes(&manifest)),
            ("NESTLE_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match_ic_pairs");

    let elim_result =
        generate_eliminations(&match_result.matched, &manifest).expect("generate_eliminations");

    // Trimmed mini_nestle has no LoanInterest / Dividend pairs after
    // pruning, so every matched pair contributes exactly 2 entries
    // (ICBalances + ICRevenueExpense).
    assert_eq!(
        elim_result.entries.len(),
        2 * match_result.matched.len(),
        "expected 2 entries per matched pair (ICBalances + ICRevenueExpense)"
    );

    let bal_count = elim_result
        .entries
        .iter()
        .filter(|e| e.elimination_type == EliminationType::ICBalances)
        .count();
    let rev_count = elim_result
        .entries
        .iter()
        .filter(|e| e.elimination_type == EliminationType::ICRevenueExpense)
        .count();
    assert_eq!(bal_count, match_result.matched.len());
    assert_eq!(rev_count, match_result.matched.len());
}

/// Every emitted entry must be balanced (DR = CR).
#[test]
fn every_entry_is_balanced() {
    let manifest = load_two_entity_manifest();
    let match_result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), sa_jes(&manifest)),
            ("NESTLE_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");
    let elim_result = generate_eliminations(&match_result.matched, &manifest).expect("eliminate");

    assert!(!elim_result.entries.is_empty(), "fixture sanity");
    for entry in &elim_result.entries {
        assert!(
            entry.is_balanced(),
            "entry {} (type {:?}) not balanced: DR={}, CR={}",
            entry.entry_id,
            entry.elimination_type,
            entry.total_debit,
            entry.total_credit
        );
    }
}

/// Aggregate totals across all entries must balance (sum DR == sum CR).
#[test]
fn aggregate_totals_balance() {
    let manifest = load_two_entity_manifest();
    let match_result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), sa_jes(&manifest)),
            ("NESTLE_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");
    let elim_result = generate_eliminations(&match_result.matched, &manifest).expect("eliminate");

    assert_eq!(
        elim_result.total_debit, elim_result.total_credit,
        "aggregate total_debit must equal total_credit"
    );
}

/// `by_type_counts` must reflect the entries that were actually
/// emitted (not the elimination types defined in the enum).
#[test]
fn by_type_counts_reflect_emitted() {
    let manifest = load_two_entity_manifest();
    let match_result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), sa_jes(&manifest)),
            ("NESTLE_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");
    let elim_result = generate_eliminations(&match_result.matched, &manifest).expect("eliminate");

    let mut hand_counted: BTreeMap<EliminationType, usize> = BTreeMap::new();
    for e in &elim_result.entries {
        *hand_counted.entry(e.elimination_type).or_insert(0) += 1;
    }
    assert_eq!(elim_result.by_type_counts, hand_counted);

    // Trimmed mini_nestle is goods_sale/royalty/management_fee only, so:
    // - ICBalances and ICRevenueExpense should be present.
    // - ICDividends, ICInterest, etc. should NOT be present.
    assert!(elim_result
        .by_type_counts
        .contains_key(&EliminationType::ICBalances));
    assert!(elim_result
        .by_type_counts
        .contains_key(&EliminationType::ICRevenueExpense));
    assert!(!elim_result
        .by_type_counts
        .contains_key(&EliminationType::ICDividends));
    assert!(!elim_result
        .by_type_counts
        .contains_key(&EliminationType::ICInterest));
}

/// Empty input → empty result with zero totals.
#[test]
fn empty_matched_input_returns_empty_result() {
    let manifest = load_two_entity_manifest_no_ic();
    let elim_result = generate_eliminations(&[], &manifest).expect("eliminate");

    assert!(elim_result.entries.is_empty());
    assert_eq!(
        elim_result.total_debit,
        rust_decimal::Decimal::ZERO,
        "no entries → zero debit"
    );
    assert_eq!(
        elim_result.total_credit,
        rust_decimal::Decimal::ZERO,
        "no entries → zero credit"
    );
    assert!(elim_result.by_type_counts.is_empty());
}

/// Two calls with the same input must produce byte-identical
/// `serde_json::to_string_pretty` output.
#[test]
fn deterministic_output() {
    let manifest = load_two_entity_manifest();
    let match_result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), sa_jes(&manifest)),
            ("NESTLE_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");

    let a = generate_eliminations(&match_result.matched, &manifest).expect("eliminate a");
    let b = generate_eliminations(&match_result.matched, &manifest).expect("eliminate b");

    let a_json = serde_json::to_string_pretty(&a).expect("serialise a");
    let b_json = serde_json::to_string_pretty(&b).expect("serialise b");
    assert_eq!(
        a_json, b_json,
        "two calls with the same input must produce byte-identical output"
    );
}

/// Account codes: every `ICBalances` entry has lines on
/// IC_AR_CLEARING (1150) and IC_AP_CLEARING (2050); every
/// `ICRevenueExpense` entry has IC_REVENUE (4500) on the seller side
/// and either COGS (5000, for goods_sale) or IC_EXPENSE (6800, for
/// other types) on the buyer side.
#[test]
fn account_codes_match_ic_injector() {
    let manifest = load_two_entity_manifest();
    let match_result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), sa_jes(&manifest)),
            ("NESTLE_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");
    let elim_result = generate_eliminations(&match_result.matched, &manifest).expect("eliminate");

    for entry in &elim_result.entries {
        match entry.elimination_type {
            EliminationType::ICBalances => {
                assert_eq!(entry.lines.len(), 2);
                let ar = &entry.lines[1]; // credit side: IC AR
                let ap = &entry.lines[0]; // debit side: IC AP
                assert_eq!(ap.account, "2050", "ICBalances DR must be IC_AP_CLEARING");
                assert_eq!(ar.account, "1150", "ICBalances CR must be IC_AR_CLEARING");
                assert!(ap.is_debit, "AP line must be DR (reduces liability)");
                assert!(!ar.is_debit, "AR line must be CR (reduces asset)");
            }
            EliminationType::ICRevenueExpense => {
                assert_eq!(entry.lines.len(), 2);
                // create_ic_revenue_expense_elimination emits DR revenue
                // (line 0), CR expense (line 1).
                let revenue = &entry.lines[0];
                let expense = &entry.lines[1];
                assert_eq!(
                    revenue.account, "4500",
                    "ICRevenueExpense DR must be IC_REVENUE"
                );
                assert!(
                    expense.account == "5000" || expense.account == "6800",
                    "ICRevenueExpense CR must be COGS (5000) or IC_EXPENSE (6800), got {}",
                    expense.account,
                );
            }
            other => panic!(
                "trimmed mini_nestle should not produce {:?}; got entry {}",
                other, entry.entry_id
            ),
        }
    }
}

/// Currency consistency: every entry must use the manifest's
/// presentation currency.
#[test]
fn currency_consistency() {
    let manifest = load_two_entity_manifest();
    let match_result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), sa_jes(&manifest)),
            ("NESTLE_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");
    let elim_result = generate_eliminations(&match_result.matched, &manifest).expect("eliminate");

    let expected = &manifest.presentation_currency;
    for entry in &elim_result.entries {
        assert_eq!(
            &entry.currency, expected,
            "entry {} currency must equal presentation_currency",
            entry.entry_id
        );
        for line in &entry.lines {
            assert_eq!(
                &line.currency, expected,
                "line {} of entry {} currency must equal presentation_currency",
                line.line_number, entry.entry_id
            );
        }
    }
}

/// Output is sorted by `(elimination_type, entry_id)` — sanity check
/// the determinism contract.
#[test]
fn output_is_sorted_by_type_then_entry_id() {
    let manifest = load_two_entity_manifest();
    let match_result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), sa_jes(&manifest)),
            ("NESTLE_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");
    let elim_result = generate_eliminations(&match_result.matched, &manifest).expect("eliminate");

    // Stable order: ICBalances entries come before ICRevenueExpense
    // entries (per our internal ordering), and within each type the
    // entries are sorted by entry_id.
    let mut prev: Option<(EliminationType, &str)> = None;
    for entry in &elim_result.entries {
        if let Some((prev_type, prev_id)) = prev {
            // ICBalances < ICRevenueExpense in our ordering.  Inside the
            // same type, entry_id should be sorted ascending.
            assert!(
                (prev_type, prev_id) <= (entry.elimination_type, entry.entry_id.as_str()),
                "entries not sorted: {:?}/{} preceded {:?}/{}",
                prev_type,
                prev_id,
                entry.elimination_type,
                entry.entry_id
            );
        }
        prev = Some((entry.elimination_type, entry.entry_id.as_str()));
    }
}

/// Fiscal period must derive from each entry's posting date in
/// `YYYYMM` format.
#[test]
fn fiscal_period_format() {
    let manifest = load_two_entity_manifest();
    let match_result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), sa_jes(&manifest)),
            ("NESTLE_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");
    let elim_result = generate_eliminations(&match_result.matched, &manifest).expect("eliminate");

    for entry in &elim_result.entries {
        assert_eq!(
            entry.fiscal_period.len(),
            6,
            "fiscal_period must be YYYYMM format"
        );
        assert!(
            entry.fiscal_period.chars().all(|c| c.is_ascii_digit()),
            "fiscal_period must be all digits, got {}",
            entry.fiscal_period
        );
    }
}

/// Entry IDs must be deterministic and follow the `ELIM-<type>-<hex>`
/// format documented in the module rustdoc.
#[test]
fn entry_id_format() {
    let manifest = load_two_entity_manifest();
    let match_result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), sa_jes(&manifest)),
            ("NESTLE_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");
    let elim_result = generate_eliminations(&match_result.matched, &manifest).expect("eliminate");

    for entry in &elim_result.entries {
        let prefix_ok = match entry.elimination_type {
            EliminationType::ICBalances => entry.entry_id.starts_with("ELIM-BAL-"),
            EliminationType::ICRevenueExpense => entry.entry_id.starts_with("ELIM-REV-"),
            EliminationType::ICInterest => entry.entry_id.starts_with("ELIM-INT-"),
            EliminationType::ICDividends => entry.entry_id.starts_with("ELIM-DIV-"),
            _ => true,
        };
        assert!(
            prefix_ok,
            "entry_id {} does not match expected prefix for {:?}",
            entry.entry_id, entry.elimination_type
        );
        // The full ID should be `ELIM-<3>-<8 hex>`, total 17 chars.
        assert_eq!(entry.entry_id.len(), 17);
    }
}

/// Consolidation entity must be the manifest's `group_id` for every
/// emitted entry — this is the row in the consolidation TB they
/// affect.
#[test]
fn consolidation_entity_is_group_id() {
    let manifest = load_two_entity_manifest();
    let match_result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), sa_jes(&manifest)),
            ("NESTLE_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");
    let elim_result = generate_eliminations(&match_result.matched, &manifest).expect("eliminate");

    for entry in &elim_result.entries {
        assert_eq!(
            entry.consolidation_entity, manifest.group_id,
            "consolidation_entity must be the manifest's group_id"
        );
    }
}

/// The seller-side and buyer-side entities on every entry must be the
/// matched pair's `seller_entity` / `buyer_entity` (ic_pair_id appears
/// in `ic_references` for traceability).
#[test]
fn related_companies_and_ic_references() {
    let manifest = load_two_entity_manifest();
    let match_result = match_ic_pairs(
        &manifest,
        &[
            ("NESTLE_SA".to_string(), sa_jes(&manifest)),
            ("NESTLE_USA".to_string(), usa_jes(&manifest)),
        ],
    )
    .expect("match");
    let elim_result = generate_eliminations(&match_result.matched, &manifest).expect("eliminate");

    for entry in &elim_result.entries {
        assert_eq!(entry.related_companies.len(), 2);
        assert!(entry.related_companies.contains(&"NESTLE_SA".to_string()));
        assert!(entry.related_companies.contains(&"NESTLE_USA".to_string()));
        // ic_references carries the pair_id traceability link to the
        // matched pair so the consolidation report can drill back.
        assert!(
            entry
                .ic_references
                .iter()
                .any(|r| r.starts_with("ic_pair_id=")),
            "entry {} missing ic_pair_id reference: {:?}",
            entry.entry_id,
            entry.ic_references
        );
    }
}

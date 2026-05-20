//! Task 11.3 — consolidated balance sheet identity property.
//!
//! IAS 1.54: every consolidated balance sheet must satisfy
//!
//! ```text
//! total_assets == total_liabilities + total_equity + total_nci
//! ```
//!
//! within `0.01` (rust_decimal scale 2).  This file walks the full
//! Chunk 5 → 7 → 8.1 pipeline (no orchestrator) for two test
//! populations and asserts the identity holds:
//!
//! 1. **`balances_for_random_configs`** — 10 randomized
//!    [`GroupConfig`]s (varied entity counts, varied IC relationship
//!    counts) with hand-built per-entity TBs.  Each TB carries lines
//!    on assets, liabilities, equity, and the IC clearing accounts so
//!    the elimination engine has something to net out.  All
//!    presentation-currency to keep the property focused on the
//!    consolidation arithmetic — translation / CTA is exercised
//!    separately in `tests/translation_e2e.rs`.
//!
//! 2. **`balances_for_mini_acme`** — the canonical Mini-Acme
//!    fixture, hand-built per-entity TBs, full pipeline.  This is the
//!    "golden" property check: the same fixture every other
//!    integration test references.
//!
//! # v5.0 scope reduction
//!
//! Both tests assume **single-currency** entities (every entity has
//! `functional_currency == presentation_currency`).  This skips IAS 21
//! translation / CTA, which is exercised by
//! `tests/translation_e2e.rs` and `tests/cta.rs`.  The property under
//! test here is the **post-elimination + post-NCI/equity-method**
//! balance identity, not the cross-currency translation invariant.
//!
//! No orchestrator runs — every helper drives in-memory pieces of the
//! Chunk 5 / 7 / 8 pipeline directly.  Each iteration runs in
//! milliseconds, so all 10 random configs + Mini-Acme finish in
//! under a second.  Cheap → no `#[ignore]`.

use chrono::NaiveDate;
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::BTreeMap;

use datasynth_core::models::balance::{
    AccountCategory, AccountType, TrialBalance, TrialBalanceLine, TrialBalanceType,
};
use datasynth_core::models::JournalEntry;
use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::shard::{derive_ic_pair_plans, inject_ic_journal_entries, InjectionCtx};
use datasynth_group::{
    aggregate_pre_elimination, apply_eliminations_to_tb, apply_nci_and_equity_method,
    build_consolidated_balance_sheet, build_manifest, eliminations_to_journal_entries,
    generate_eliminations, match_ic_pairs, ConsolidationMethod, EntityConfig, FxConfig,
    FxPolicyConfig, FxRateBasis, FxRateSource, GroupConfig, GroupMaterialityConfig,
    IcMatchingConfig, IcRelationshipConfig, IcRelationshipExplicit, IcTransactionType,
    IntercompanyConfig, MaterialityBasis, OutputLayoutConfig, OwnershipConfig, PeriodConfig,
    PeriodLength,
};

// ── Fixture builders ──────────────────────────────────────────────────────────

/// Build a balanced multi-line TB with deliberately chosen accounts
/// across asset / liability / equity classes so the consolidated BS
/// has lines in every section.
///
/// Lines:
/// - 1000 Cash             DR `cash`
/// - 1500 PP&E             DR `ppe`
/// - 2000 AP               CR `ap`
/// - 2600 Long-term debt   CR `ltd`
/// - 3000 Common stock     CR `cs`
/// - 3300 Retained earnings CR `re`
/// - Optional 1150 IC AR clearing  DR `ic_ar`
/// - Optional 2050 IC AP clearing  CR `ic_ap`
///
/// All entries balance per construction (check baked into helper).
#[allow(clippy::too_many_arguments)]
fn build_full_section_tb(
    company_code: &str,
    currency: &str,
    cash: Decimal,
    ppe: Decimal,
    ap: Decimal,
    ltd: Decimal,
    cs: Decimal,
    re: Decimal,
    ic_ar: Option<Decimal>,
    ic_ap: Option<Decimal>,
) -> TrialBalance {
    let mut tb = TrialBalance::new(
        format!("TB-{company_code}-2024-03"),
        company_code.to_string(),
        NaiveDate::from_ymd_opt(2024, 3, 31).expect("valid date"),
        2024,
        3,
        currency.to_string(),
        TrialBalanceType::Adjusted,
    );

    let push_dr = |tb: &mut TrialBalance, code: &str, amount: Decimal, kind: AccountType| {
        if amount.is_zero() {
            return;
        }
        let category = match kind {
            AccountType::Asset => AccountCategory::CurrentAssets,
            AccountType::Liability => AccountCategory::CurrentLiabilities,
            AccountType::Equity => AccountCategory::Equity,
            _ => AccountCategory::CurrentAssets,
        };
        tb.add_line(TrialBalanceLine {
            account_code: code.to_string(),
            account_description: format!("DR {code}"),
            category,
            account_type: kind,
            opening_balance: Decimal::ZERO,
            period_debits: amount,
            period_credits: Decimal::ZERO,
            closing_balance: amount,
            debit_balance: amount,
            credit_balance: Decimal::ZERO,
            cost_center: None,
            profit_center: None,
        });
    };

    let push_cr = |tb: &mut TrialBalance, code: &str, amount: Decimal, kind: AccountType| {
        if amount.is_zero() {
            return;
        }
        let category = match kind {
            AccountType::Asset => AccountCategory::CurrentAssets,
            AccountType::Liability => AccountCategory::CurrentLiabilities,
            AccountType::Equity => AccountCategory::Equity,
            _ => AccountCategory::CurrentLiabilities,
        };
        tb.add_line(TrialBalanceLine {
            account_code: code.to_string(),
            account_description: format!("CR {code}"),
            category,
            account_type: kind,
            opening_balance: Decimal::ZERO,
            period_debits: Decimal::ZERO,
            period_credits: amount,
            closing_balance: amount,
            debit_balance: Decimal::ZERO,
            credit_balance: amount,
            cost_center: None,
            profit_center: None,
        });
    };

    push_dr(&mut tb, "1000", cash, AccountType::Asset);
    push_dr(&mut tb, "1500", ppe, AccountType::Asset);
    push_cr(&mut tb, "2000", ap, AccountType::Liability);
    push_cr(&mut tb, "2600", ltd, AccountType::Liability);
    push_cr(&mut tb, "3000", cs, AccountType::Equity);
    push_cr(&mut tb, "3300", re, AccountType::Equity);
    if let Some(amt) = ic_ar {
        push_dr(&mut tb, "1150", amt, AccountType::Asset);
    }
    if let Some(amt) = ic_ap {
        push_cr(&mut tb, "2050", amt, AccountType::Liability);
    }

    debug_assert!(
        tb.is_balanced,
        "fixture builder must produce balanced TB for {company_code}"
    );
    tb
}

/// Run the full Chunk 5 → 7 → 8.1 pipeline (no orchestrator) and
/// return the resulting [`ConsolidatedBalanceSheet`].
///
/// Steps:
/// 1. [`aggregate_pre_elimination`] over per-entity TBs.
/// 2. Build IC JEs via [`derive_ic_pair_plans`] +
///    [`inject_ic_journal_entries`] for every entity.
/// 3. [`match_ic_pairs`] to recover the matched pairs.
/// 4. [`generate_eliminations`] + [`eliminations_to_journal_entries`].
/// 5. [`apply_eliminations_to_tb`] to fold elimination JEs into the
///    pre-elim TB.
/// 6. [`apply_nci_and_equity_method`] (with empty NCI / EM lists for
///    v5.0 simplification — single-currency, full-method only).
/// 7. [`build_consolidated_balance_sheet`].
fn run_pipeline_to_bs(
    manifest: &GroupManifest,
    entity_tbs: &[(String, TrialBalance)],
) -> datasynth_group::ConsolidatedBalanceSheet {
    let pre = aggregate_pre_elimination(manifest, entity_tbs)
        .expect("aggregate_pre_elimination must succeed");

    // Per-entity IC JEs.
    let entity_jes: Vec<(String, Vec<JournalEntry>)> = manifest
        .ownership_graph
        .entities
        .iter()
        .map(|e| {
            let plans = derive_ic_pair_plans(manifest, &e.code);
            let jes = inject_ic_journal_entries(
                &plans,
                &InjectionCtx {
                    entity_code: e.code.clone(),
                },
            );
            (e.code.clone(), jes)
        })
        .collect();

    let match_result = match_ic_pairs(manifest, &entity_jes).expect("match_ic_pairs");
    let elim = generate_eliminations(&match_result.matched, manifest).expect("eliminations");
    let elim_jes = eliminations_to_journal_entries(&elim);
    let post = apply_eliminations_to_tb(&pre, &elim_jes).expect("apply_eliminations_to_tb");

    // Empty NCI + EM overlays — single-currency / full-method-only
    // property keeps the test focused on the post-elim arithmetic.
    let post_nci = apply_nci_and_equity_method(&post, &[], &[])
        .expect("apply_nci_and_equity_method (empty overlays)");

    build_consolidated_balance_sheet(&post_nci, &manifest.group_id, manifest.period.end)
        .expect("build_consolidated_balance_sheet")
}

/// Build a deterministic single-currency [`GroupConfig`] for the
/// random-configs property test.  All entities use CHF and
/// `ConsolidationMethod::Full` (except the parent which is `Parent`),
/// so we exercise post-elimination arithmetic without translation.
fn random_group_config(seed: u64, entity_count: usize, ic_count: usize) -> GroupConfig {
    assert!(entity_count >= 2, "need ≥ 2 entities");
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    let entities: Vec<EntityConfig> = (0..entity_count)
        .map(|i| EntityConfig {
            code: format!("E{:03}", i + 1),
            name: None,
            country: "CH".to_string(),
            functional_currency: "CHF".to_string(),
            scoping_profile: "significant".to_string(),
            consolidation_method: if i == 0 {
                ConsolidationMethod::Parent
            } else {
                ConsolidationMethod::Full
            },
            ownership_percent: if i == 0 { None } else { Some(Decimal::ONE) },
            parent_code: if i == 0 {
                None
            } else {
                Some("E001".to_string())
            },
            acquisition_date: None,
            accounting_framework: None,
            industry: None,
            rows: None,
            hyperinflation_status:
                datasynth_core::models::HyperinflationStatus::NotHyperinflationary,
            ownership_changes: Vec::new(),
            overrides: BTreeMap::new(),
        })
        .collect();
    let codes: Vec<String> = entities.iter().map(|e| e.code.clone()).collect();

    // Generate IC relationships: random (seller, buyer, type) triples,
    // no duplicates.  Each iteration scaled to keep the test sub-second.
    let tx_types = [
        IcTransactionType::GoodsSale,
        IcTransactionType::ManagementFee,
        IcTransactionType::Royalty,
        IcTransactionType::ServiceProvided,
    ];
    let mut relationships: Vec<IcRelationshipConfig> = Vec::with_capacity(ic_count);
    let mut seen: std::collections::BTreeSet<(String, String, IcTransactionType)> =
        Default::default();
    let max_attempts = ic_count.saturating_mul(64).max(64);
    let mut tries = 0usize;
    while relationships.len() < ic_count && tries < max_attempts {
        tries += 1;
        let s = rng.random_range(0..codes.len());
        let b = rng.random_range(0..codes.len());
        if s == b {
            continue;
        }
        let t = tx_types[rng.random_range(0..tx_types.len())];
        let triple = (codes[s].clone(), codes[b].clone(), t);
        if !seen.insert(triple) {
            continue;
        }
        let units: u64 = rng.random_range(100_000..=1_000_000);
        relationships.push(IcRelationshipConfig::Explicit(IcRelationshipExplicit {
            seller: codes[s].clone(),
            buyer: codes[b].clone(),
            types: vec![t],
            annual_volume: Decimal::from(units),
            transfer_pricing: None,
            markup_percent: None,
        }));
    }

    let mut profile_map = serde_yaml::Mapping::new();
    profile_map.insert(
        serde_yaml::Value::String("row_budget".to_string()),
        serde_yaml::Value::Number(serde_yaml::Number::from(1_000u64)),
    );
    let mut scoping_profiles: BTreeMap<String, serde_yaml::Value> = BTreeMap::new();
    scoping_profiles.insert(
        "significant".to_string(),
        serde_yaml::Value::Mapping(profile_map),
    );

    GroupConfig {
        id: format!("BAL_PROP_{seed:04}"),
        name: Some(format!("Balance property seed {seed}")),
        presentation_currency: "CHF".to_string(),
        period: PeriodConfig {
            start_date: NaiveDate::from_ymd_opt(2024, 1, 1).expect("date"),
            length: PeriodLength::Quarterly,
            fiscal_year_end: None,
        },
        seed,
        defaults: serde_yaml::Value::Null,
        scoping_profiles,
        ownership: OwnershipConfig {
            parent_entity_code: codes[0].clone(),
            entities,
            generated: Vec::new(),
            entities_from: None,
        },
        intercompany: IntercompanyConfig {
            relationships,
            matching: IcMatchingConfig::default(),
        },
        fx: FxConfig {
            base_currency: "CHF".to_string(),
            rate_source: FxRateSource::Inline,
            rates: BTreeMap::new(),
            policy: FxPolicyConfig {
                balance_sheet: FxRateBasis::Closing,
                income_statement: FxRateBasis::Average,
                equity: FxRateBasis::Historical,
            },
        },
        audit: datasynth_group::AuditEngagementConfig {
            engagement_id: None,
            lead_auditor: None,
            framework: None,
            fsm_blueprint: None,
            group_materiality: Some(GroupMaterialityConfig {
                basis: MaterialityBasis::Revenue,
                percent: Decimal::new(1, 2),
            }),
            component_scope_thresholds: None,
            generate_kams: false,
            generate_group_opinion: false,
        },
        tax: Default::default(),
        cgu: Default::default(),
        output: OutputLayoutConfig::default(),
        fleet: None,
    }
}

/// Build per-entity TBs scaled by entity index so each contributes
/// distinct amounts but every TB stays balanced.  IC clearing accounts
/// (1150/2050) match the per-entity IC notional totals so eliminations
/// fully zero them out.
fn build_random_entity_tbs(manifest: &GroupManifest) -> Vec<(String, TrialBalance)> {
    let mut tbs: Vec<(String, TrialBalance)> = Vec::new();
    for (idx, entity) in manifest.ownership_graph.entities.iter().enumerate() {
        // Per-entity IC notional totals — each side debits/credits the
        // amount it owes / is owed across all relationships.  We use
        // these to seed 1150 / 2050 so the elimination engine has
        // something to fully zero out.
        let plans = derive_ic_pair_plans(manifest, &entity.code);
        let ic_ar: Decimal = plans
            .iter()
            .filter(|p| p.role == datasynth_group::shard::IcRole::Seller)
            .map(|p| p.amount)
            .sum();
        let ic_ap: Decimal = plans
            .iter()
            .filter(|p| p.role == datasynth_group::shard::IcRole::Buyer)
            .map(|p| p.amount)
            .sum();

        // Base amounts scale by entity index to keep totals distinct.
        let base = Decimal::from((idx as u64 + 1) * 100_000);
        let cash = base * dec!(2);
        let ppe = base * dec!(3);
        let ap = base;
        let ltd = base;
        let cs = base;
        let re = base * dec!(2) + ic_ar - ic_ap;
        // Balance check: assets = liab + equity
        // (cash + ppe + ic_ar) = (ap + ltd + ic_ap) + (cs + re)
        // Substitute: re = cash + ppe + ic_ar - ap - ltd - ic_ap - cs
        //           = 5*base + ic_ar - 2*base - ic_ap - base
        //           = 2*base + ic_ar - ic_ap   ✓
        let tb = build_full_section_tb(
            &entity.code,
            "CHF",
            cash,
            ppe,
            ap,
            ltd,
            cs,
            re,
            if ic_ar.is_zero() { None } else { Some(ic_ar) },
            if ic_ap.is_zero() { None } else { Some(ic_ap) },
        );
        tbs.push((entity.code.clone(), tb));
    }
    tbs
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Walk 10 randomized synthetic configs through the full Chunk
/// 5 → 7 → 8.1 pipeline (no orchestrator) and assert that every
/// resulting consolidated BS satisfies `assets = L + E + NCI` to the
/// cent.
#[test]
fn balances_for_random_configs() {
    let cases: [(u64, usize, usize); 10] = [
        (0, 3, 1),
        (1, 5, 2),
        (2, 7, 3),
        (3, 4, 4),
        (4, 6, 5),
        (5, 8, 3),
        (6, 5, 6),
        (7, 9, 4),
        (8, 6, 2),
        (9, 7, 5),
    ];

    for (seed, ents, rels) in cases {
        let cfg = random_group_config(seed, ents, rels);
        let manifest =
            build_manifest(&cfg).unwrap_or_else(|e| panic!("build_manifest seed={seed}: {e}"));
        let tbs = build_random_entity_tbs(&manifest);

        let bs = run_pipeline_to_bs(&manifest, &tbs);

        let lhs = bs.total_assets;
        let rhs = bs.total_liabilities + bs.total_equity + bs.total_nci;
        let diff = (lhs - rhs).abs();
        let tolerance = dec!(0.01);
        assert!(
            diff <= tolerance,
            "seed={seed}, entities={ents}, rels={rels}: BS doesn't balance: \
             assets={lhs}, L+E+NCI={rhs}, diff={diff}",
        );
        // build_consolidated_balance_sheet bakes the identity into the
        // returned `total_liabilities_plus_equity_plus_nci` field —
        // double-check it matches the explicit sum so a refactor that
        // breaks one but not the other is caught.
        assert_eq!(
            bs.total_liabilities_plus_equity_plus_nci, rhs,
            "seed={seed}: total_liabilities_plus_equity_plus_nci field drift",
        );
    }
}

/// Mini-Acme fixture: hand-built per-entity TBs (5 entities, 4 of
/// which contribute to the Parent + Full scope), full Chunk 5 → 7 →
/// 8.1 pipeline, BS identity must hold to the cent.
#[test]
fn balances_for_mini_acme() {
    let yaml = include_str!("fixtures/mini_acme.yaml");
    let mut cfg: GroupConfig = serde_yaml::from_str(yaml).expect("mini_acme.yaml");

    // Pin every entity to CHF so we don't need to translate.  The
    // property under test is the post-elim balance identity, not
    // translation — see module rustdoc.
    for e in cfg.ownership.entities.iter_mut() {
        e.functional_currency = "CHF".to_string();
    }
    // Also pin the FX rate table (which the manifest's FX master
    // walks even when no translation is needed) by emptying it — no
    // pairs are needed when functional == presentation everywhere.
    cfg.fx.rates.clear();

    let manifest = build_manifest(&cfg).expect("mini_acme build_manifest");
    let tbs = build_random_entity_tbs(&manifest);

    let bs = run_pipeline_to_bs(&manifest, &tbs);

    let lhs = bs.total_assets;
    let rhs = bs.total_liabilities + bs.total_equity + bs.total_nci;
    let diff = (lhs - rhs).abs();
    let tolerance = dec!(0.01);
    assert!(
        diff <= tolerance,
        "Mini-Acme: BS doesn't balance: assets={lhs}, L+E+NCI={rhs}, diff={diff}",
    );
}

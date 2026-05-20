//! Task 5.6 — post-elimination consolidated TB integration tests.
//!
//! Wire-level tests for [`datasynth_group::apply_eliminations_to_tb`] —
//! the IC-elimination fold that turns the pre-elim consolidated TB
//! into the post-elim consolidated TB.  We build the upstream artefacts
//! (per-entity TBs, manifest, IC matches, eliminations, JEs) using the
//! same patterns as `tb_loader.rs::balanced_tb`,
//! `pre_elim.rs::build_balanced_tb`, and
//! `elimination_to_je.rs::load_two_entity_manifest` so each case stays
//! a few KB of resident memory and the wiring under test is the only
//! moving piece.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_core::models::balance::{
    AccountCategory, AccountType, TrialBalance, TrialBalanceLine, TrialBalanceType,
};
use datasynth_core::models::{JournalEntry, JournalEntryHeader, JournalEntryLine};

use datasynth_group::aggregate::{
    aggregate_pre_elimination, apply_eliminations_to_tb, eliminations_to_journal_entries,
    generate_eliminations, match_ic_pairs,
};
use datasynth_group::config::IcRelationshipConfig;
use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::shard::{derive_ic_pair_plans, inject_ic_journal_entries, InjectionCtx};
use datasynth_group::{build_manifest, GroupConfig, GroupError};

// ── Fixture builders ──────────────────────────────────────────────────────────

/// Mirror of `tests/elimination_to_je.rs::load_two_entity_manifest` —
/// trim mini_acme to ACME_SA + ACME_USA with exactly the explicit
/// SA→USA IC relationship.
fn load_two_entity_manifest() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_acme.yaml");
    let mut cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_acme.yaml must parse into GroupConfig");

    cfg.ownership
        .entities
        .retain(|e| matches!(e.code.as_str(), "ACME_SA" | "ACME_USA"));

    cfg.intercompany.relationships.retain(|r| match r {
        IcRelationshipConfig::Explicit(e) => e.seller == "ACME_SA" && e.buyer == "ACME_USA",
        IcRelationshipConfig::Pattern(_) => false,
    });
    assert_eq!(
        cfg.intercompany.relationships.len(),
        1,
        "trim must leave exactly one explicit ACME_SA→ACME_USA relationship",
    );

    if let Some(p2) = cfg.tax.pillar_two.as_mut() {
        p2.jurisdictions
            .retain(|j| matches!(j.as_str(), "CH" | "US"));
    }
    if let Some(tp) = cfg.tax.transfer_pricing.as_mut() {
        tp.local_files_for
            .retain(|j| matches!(j.as_str(), "CH" | "US"));
    }

    build_manifest(&cfg).expect("trimmed mini_acme must still build a manifest")
}

/// Build a balanced 2-line TB.  Mirrors
/// `tests/pre_elim.rs::build_balanced_tb` — kept here verbatim so each
/// integration test file is self-contained.
fn build_balanced_tb(
    company_code: &str,
    currency: &str,
    debit_account: &str,
    credit_account: &str,
    amount: Decimal,
) -> TrialBalance {
    let mut tb = TrialBalance::new(
        format!("TB-{}-2024-03", company_code),
        company_code.to_string(),
        NaiveDate::from_ymd_opt(2024, 3, 31).expect("valid date"),
        2024,
        3,
        currency.to_string(),
        TrialBalanceType::Adjusted,
    );
    tb.add_line(TrialBalanceLine {
        account_code: debit_account.to_string(),
        account_description: format!("DR {debit_account}"),
        category: AccountCategory::CurrentAssets,
        account_type: AccountType::Asset,
        opening_balance: Decimal::ZERO,
        period_debits: amount,
        period_credits: Decimal::ZERO,
        closing_balance: amount,
        debit_balance: amount,
        credit_balance: Decimal::ZERO,
        cost_center: None,
        profit_center: None,
    });
    tb.add_line(TrialBalanceLine {
        account_code: credit_account.to_string(),
        account_description: format!("CR {credit_account}"),
        category: AccountCategory::Equity,
        account_type: AccountType::Equity,
        opening_balance: Decimal::ZERO,
        period_debits: Decimal::ZERO,
        period_credits: amount,
        closing_balance: amount,
        debit_balance: Decimal::ZERO,
        credit_balance: amount,
        cost_center: None,
        profit_center: None,
    });
    debug_assert!(tb.is_balanced, "fixture builder must produce balanced TB");
    tb
}

/// Convenience: build a TB with N pre-populated debit/credit pairs.
/// Used to seed the IC-side balances on each entity's TB before
/// aggregation so the eliminations have something to zero out.
fn build_multi_line_tb(
    company_code: &str,
    currency: &str,
    lines: &[(&str, Decimal, Decimal)],
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
    for (account_code, debit, credit) in lines {
        tb.add_line(TrialBalanceLine {
            account_code: (*account_code).to_string(),
            account_description: format!("Line {account_code}"),
            category: AccountCategory::CurrentAssets,
            account_type: AccountType::Asset,
            opening_balance: Decimal::ZERO,
            period_debits: *debit,
            period_credits: *credit,
            closing_balance: *debit - *credit,
            debit_balance: *debit,
            credit_balance: *credit,
            cost_center: None,
            profit_center: None,
        });
    }
    tb
}

/// Build the full pipeline state for the trimmed mini_acme fixture:
/// per-entity IC JEs → matched pairs → eliminations → JEs.
struct PipelineState {
    manifest: GroupManifest,
    elim_jes: Vec<JournalEntry>,
}

fn build_pipeline_state() -> PipelineState {
    let manifest = load_two_entity_manifest();

    let sa_plans = derive_ic_pair_plans(&manifest, "ACME_SA");
    let usa_plans = derive_ic_pair_plans(&manifest, "ACME_USA");
    let sa_jes = inject_ic_journal_entries(
        &sa_plans,
        &InjectionCtx {
            entity_code: "ACME_SA".to_string(),
        },
    );
    let usa_jes = inject_ic_journal_entries(
        &usa_plans,
        &InjectionCtx {
            entity_code: "ACME_USA".to_string(),
        },
    );

    let match_result = match_ic_pairs(
        &manifest,
        &[
            ("ACME_SA".to_string(), sa_jes),
            ("ACME_USA".to_string(), usa_jes),
        ],
    )
    .expect("match_ic_pairs");

    let elim =
        generate_eliminations(&match_result.matched, &manifest).expect("generate_eliminations");
    let elim_jes = eliminations_to_journal_entries(&elim);

    PipelineState { manifest, elim_jes }
}

/// Compute the per-entity IC notional totals from the seller-side IC JEs
/// — the sum of every IC AR clearing debit on ACME_SA's books.  This
/// is the elimination amount we expect to see fully reversed in the
/// post-elim TB.
fn ic_notional_totals(manifest: &GroupManifest, entity_code: &str) -> Decimal {
    let plans = derive_ic_pair_plans(manifest, entity_code);
    let jes = inject_ic_journal_entries(
        &plans,
        &InjectionCtx {
            entity_code: entity_code.to_string(),
        },
    );
    jes.iter()
        .flat_map(|je| je.lines.iter())
        .map(|l| l.debit_amount)
        .sum()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Happy path — 2-entity Mini-Acme end-to-end.
///
/// 1. Synthesise per-entity TBs containing the IC clearing balances on
///    both sides (seller has IC_AR + IC_REVENUE, buyer has IC_AP + COGS).
/// 2. Aggregate → pre-elim TB.
/// 3. Build IC pairs → eliminations → JEs.
/// 4. Apply → post-elim TB.
/// 5. Assert: the IC clearing accounts and IC revenue/COGS net to zero
///    (or near-zero — the seller's debits and the eliminations' credits
///    cancel exactly because the IC injector emits flat amounts).
#[test]
fn happy_path_zeros_out_ic_balances_and_revenue() {
    let state = build_pipeline_state();

    // Total IC notional on ACME_SA's books = sum of seller-side debits.
    // For the trimmed fixture this is non-zero (multiple goods_sale +
    // royalty pairs scale to ~5M CHF annual volume).
    let sa_ic_total = ic_notional_totals(&state.manifest, "ACME_SA");
    let usa_ic_total = ic_notional_totals(&state.manifest, "ACME_USA");
    assert!(
        sa_ic_total > Decimal::ZERO,
        "fixture sanity: SA must have non-zero IC notional"
    );
    assert!(
        usa_ic_total > Decimal::ZERO,
        "fixture sanity: USA must have non-zero IC notional"
    );

    // Per-entity TBs: each entity carries the IC-clearing balances that
    // the IC JEs would have left on its books, plus a balancing equity
    // line so the standalone TB is balanced.
    //
    // SA (seller side):
    //   DR 1150 IC_AR_CLEARING  = sa_ic_total   (one debit per goods_sale + royalty pair)
    //   CR 4500 IC_REVENUE      = sa_ic_total
    //
    // USA (buyer side):
    //   DR 5000 COGS            = part_of_usa_ic_total (goods_sale only; injector splits)
    //   DR 6800 IC_EXPENSE      = remainder              (royalty, mgmt fee, ...)
    //   CR 2050 IC_AP_CLEARING  = usa_ic_total
    //
    // Rather than splitting USA's TB by transaction type, we simulate
    // the aggregate state directly: the buyer's IC AP balance equals
    // the seller's IC AR (manifest pairing guarantees it), and we use
    // a single IC_EXPENSE bucket on the buyer side because the
    // post-elim assertion only depends on the sum being zero per
    // account.  The post-elim TB sees the seller's IC_REVENUE and the
    // buyer's IC_EXPENSE / COGS landing in different accounts, so we
    // construct the buyer TB to mirror what the IC injector would have
    // emitted.
    let usa_jes = inject_ic_journal_entries(
        &derive_ic_pair_plans(&state.manifest, "ACME_USA"),
        &InjectionCtx {
            entity_code: "ACME_USA".to_string(),
        },
    );
    let mut usa_per_account: std::collections::BTreeMap<String, (Decimal, Decimal)> =
        std::collections::BTreeMap::new();
    for je in &usa_jes {
        for line in &je.lines {
            let entry = usa_per_account
                .entry(line.gl_account.clone())
                .or_insert((Decimal::ZERO, Decimal::ZERO));
            entry.0 += line.debit_amount;
            entry.1 += line.credit_amount;
        }
    }
    let usa_lines: Vec<(&str, Decimal, Decimal)> = usa_per_account
        .iter()
        .map(|(k, v)| (k.as_str(), v.0, v.1))
        .collect();

    let sa_tb = build_multi_line_tb(
        "ACME_SA",
        "CHF",
        &[
            ("1150", sa_ic_total, Decimal::ZERO),
            ("4500", Decimal::ZERO, sa_ic_total),
        ],
    );
    let usa_tb = build_multi_line_tb("ACME_USA", "CHF", &usa_lines);

    debug_assert!(sa_tb.is_balanced, "SA fixture TB must balance");
    debug_assert!(usa_tb.is_balanced, "USA fixture TB must balance");

    // Step 1: aggregate to pre-elim.
    let pre_elim = aggregate_pre_elimination(
        &state.manifest,
        &[
            ("ACME_SA".to_string(), sa_tb),
            ("ACME_USA".to_string(), usa_tb),
        ],
    )
    .expect("aggregate_pre_elimination");

    // Sanity: the pre-elim TB has IC-clearing balances waiting to be
    // eliminated.
    assert_eq!(
        pre_elim
            .account_totals
            .get("1150")
            .expect("pre-elim must have IC_AR_CLEARING")
            .net_balance,
        sa_ic_total,
        "pre-elim IC_AR_CLEARING net = SA's IC notional"
    );
    assert_eq!(
        pre_elim
            .account_totals
            .get("2050")
            .expect("pre-elim must have IC_AP_CLEARING")
            .net_balance,
        -usa_ic_total,
        "pre-elim IC_AP_CLEARING net = -USA's IC notional"
    );

    // Step 2: apply eliminations.
    let post_elim =
        apply_eliminations_to_tb(&pre_elim, &state.elim_jes).expect("apply_eliminations_to_tb");

    // Post-elim assertions: every IC clearing / IC P&L account nets to
    // zero (within the 0.01 tolerance the elimination engine uses).
    let tolerance = dec!(0.01);
    for account in ["1150", "2050", "4500", "5000", "6800"] {
        if let Some(acct) = post_elim.account_totals.get(account) {
            assert!(
                acct.net_balance.abs() <= tolerance,
                "post-elim {} must net to zero, got {}",
                account,
                acct.net_balance,
            );
        }
    }

    // Aggregate balance preserved.
    let diff = (post_elim.total_debits - post_elim.total_credits).abs();
    assert!(
        diff <= tolerance,
        "post-elim aggregate must balance: DR={}, CR={}, diff={}",
        post_elim.total_debits,
        post_elim.total_credits,
        diff,
    );
}

/// An elimination posting against an account that **no contributing
/// entity** had appears in the post-elim TB with the elimination's
/// balance and `contributing_entities = 0`.
#[test]
fn account_not_in_pre_elim_is_created_from_zero() {
    let state = build_pipeline_state();

    // Empty pre-elim TB so every elimination line creates a new account.
    let pre_elim = aggregate_pre_elimination(&state.manifest, &[]).expect("empty aggregate");

    // Build a synthetic elimination JE with one line on a brand-new
    // account (9999) that no real elimination would touch — guarantees
    // we exercise the new-account creation path.
    let mut je = JournalEntry::new(JournalEntryHeader::new(
        state.manifest.group_id.clone(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    ));
    je.header.is_elimination = true;
    je.header.currency = "CHF".to_string();
    let doc_id = je.header.document_id;
    je.add_line(JournalEntryLine::debit(
        doc_id,
        1,
        "9999".to_string(),
        dec!(123.45),
    ));
    je.add_line(JournalEntryLine::credit(
        doc_id,
        2,
        "8888".to_string(),
        dec!(123.45),
    ));

    let post_elim = apply_eliminations_to_tb(&pre_elim, &[je]).expect("apply must succeed");

    let acct_9999 = post_elim
        .account_totals
        .get("9999")
        .expect("9999 must be created from zero");
    assert_eq!(acct_9999.debit_total, dec!(123.45));
    assert_eq!(acct_9999.credit_total, Decimal::ZERO);
    assert_eq!(acct_9999.net_balance, dec!(123.45));
    assert_eq!(
        acct_9999.contributing_entities, 0,
        "elimination is a group-level adjustment, not an entity contribution"
    );

    let acct_8888 = post_elim
        .account_totals
        .get("8888")
        .expect("8888 must be created from zero");
    assert_eq!(acct_8888.credit_total, dec!(123.45));
    assert_eq!(acct_8888.net_balance, -dec!(123.45));
    assert_eq!(acct_8888.contributing_entities, 0);
}

/// JEs with `is_elimination = false` are silently ignored — defensive
/// filter for callers that pass mixed slices.
#[test]
fn non_elimination_jes_are_silently_ignored() {
    let state = build_pipeline_state();
    let pre_elim = aggregate_pre_elimination(&state.manifest, &[]).expect("empty aggregate");

    // Build a non-elimination JE that should NOT affect the TB.
    let mut bad_je = JournalEntry::new(JournalEntryHeader::new(
        "ACME_SA".to_string(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    ));
    // Important: leave header.is_elimination = false (the default).
    bad_je.header.currency = "CHF".to_string();
    let doc_id = bad_je.header.document_id;
    bad_je.add_line(JournalEntryLine::debit(
        doc_id,
        1,
        "1100".to_string(),
        dec!(99999),
    ));
    bad_je.add_line(JournalEntryLine::credit(
        doc_id,
        2,
        "3100".to_string(),
        dec!(99999),
    ));

    let post_elim =
        apply_eliminations_to_tb(&pre_elim, &[bad_je]).expect("non-elim JE must be ignored");

    assert!(
        post_elim.account_totals.is_empty(),
        "non-elimination JE must not affect post-elim TB"
    );
    assert_eq!(post_elim.total_debits, Decimal::ZERO);
    assert_eq!(post_elim.total_credits, Decimal::ZERO);
}

/// Empty `elim_jes` slice returns the pre-elim TB unchanged (modulo the
/// internal clone).
#[test]
fn empty_elim_jes_returns_pre_elim_unchanged() {
    let state = build_pipeline_state();

    let pre_elim = aggregate_pre_elimination(
        &state.manifest,
        &[
            (
                "ACME_SA".to_string(),
                build_balanced_tb("ACME_SA", "CHF", "1100", "3100", dec!(1000)),
            ),
            (
                "ACME_USA".to_string(),
                build_balanced_tb("ACME_USA", "CHF", "1100", "3100", dec!(2000)),
            ),
        ],
    )
    .expect("aggregate");

    let post_elim = apply_eliminations_to_tb(&pre_elim, &[]).expect("empty elim must pass");

    assert_eq!(
        post_elim, pre_elim,
        "empty elim_jes must round-trip the pre-elim TB"
    );
}

/// Currency mismatch between an elimination JE and the pre-elim TB
/// errors out — translation must happen first (Chunk 6).
#[test]
fn currency_mismatch_errors_out() {
    let state = build_pipeline_state();
    let pre_elim = aggregate_pre_elimination(&state.manifest, &[]).expect("empty aggregate");

    // Pre-elim is CHF (mini_acme.yaml::presentation_currency).  Build
    // an elimination JE in EUR.
    let mut bad_je = JournalEntry::new(JournalEntryHeader::new(
        state.manifest.group_id.clone(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    ));
    bad_je.header.is_elimination = true;
    bad_je.header.currency = "EUR".to_string();
    let doc_id = bad_je.header.document_id;
    bad_je.add_line(JournalEntryLine::debit(
        doc_id,
        1,
        "1150".to_string(),
        dec!(100),
    ));
    bad_je.add_line(JournalEntryLine::credit(
        doc_id,
        2,
        "2050".to_string(),
        dec!(100),
    ));

    let err =
        apply_eliminations_to_tb(&pre_elim, &[bad_je]).expect_err("currency mismatch must error");
    match err {
        GroupError::Aggregate(msg) => {
            assert!(msg.contains("EUR"), "must name the JE currency");
            assert!(msg.contains("CHF"), "must name the pre-elim currency");
            assert!(
                msg.contains("Chunk 6") || msg.contains("translation"),
                "must point at the translation phase, got {msg:?}"
            );
        }
        other => panic!("expected GroupError::Aggregate, got {other:?}"),
    }
}

/// Balance preserved: the post-elim aggregate `total_debits` must equal
/// `total_credits` after folding (within the 0.01 tolerance).
#[test]
fn balance_preserved_across_application() {
    let state = build_pipeline_state();

    let sa_ic = ic_notional_totals(&state.manifest, "ACME_SA");
    let usa_jes = inject_ic_journal_entries(
        &derive_ic_pair_plans(&state.manifest, "ACME_USA"),
        &InjectionCtx {
            entity_code: "ACME_USA".to_string(),
        },
    );
    let mut usa_per_account: std::collections::BTreeMap<String, (Decimal, Decimal)> =
        std::collections::BTreeMap::new();
    for je in &usa_jes {
        for line in &je.lines {
            let entry = usa_per_account
                .entry(line.gl_account.clone())
                .or_insert((Decimal::ZERO, Decimal::ZERO));
            entry.0 += line.debit_amount;
            entry.1 += line.credit_amount;
        }
    }
    let usa_lines: Vec<(&str, Decimal, Decimal)> = usa_per_account
        .iter()
        .map(|(k, v)| (k.as_str(), v.0, v.1))
        .collect();

    let pre_elim = aggregate_pre_elimination(
        &state.manifest,
        &[
            (
                "ACME_SA".to_string(),
                build_multi_line_tb(
                    "ACME_SA",
                    "CHF",
                    &[
                        ("1150", sa_ic, Decimal::ZERO),
                        ("4500", Decimal::ZERO, sa_ic),
                    ],
                ),
            ),
            (
                "ACME_USA".to_string(),
                build_multi_line_tb("ACME_USA", "CHF", &usa_lines),
            ),
        ],
    )
    .expect("aggregate");

    let post_elim = apply_eliminations_to_tb(&pre_elim, &state.elim_jes).expect("apply");

    let tolerance = dec!(0.01);
    let diff = (post_elim.total_debits - post_elim.total_credits).abs();
    assert!(
        diff <= tolerance,
        "post-elim aggregate must balance: DR={}, CR={}, diff={}",
        post_elim.total_debits,
        post_elim.total_credits,
        diff,
    );
}

/// Determinism: two calls with identical input produce byte-identical
/// JSON.  `BTreeMap` and sorted `Vec`s in the public surface make this
/// a strict equality check, no tolerance.
#[test]
fn deterministic_output_across_calls() {
    let state = build_pipeline_state();

    let sa_ic = ic_notional_totals(&state.manifest, "ACME_SA");
    let usa_jes = inject_ic_journal_entries(
        &derive_ic_pair_plans(&state.manifest, "ACME_USA"),
        &InjectionCtx {
            entity_code: "ACME_USA".to_string(),
        },
    );
    let mut usa_per_account: std::collections::BTreeMap<String, (Decimal, Decimal)> =
        std::collections::BTreeMap::new();
    for je in &usa_jes {
        for line in &je.lines {
            let entry = usa_per_account
                .entry(line.gl_account.clone())
                .or_insert((Decimal::ZERO, Decimal::ZERO));
            entry.0 += line.debit_amount;
            entry.1 += line.credit_amount;
        }
    }
    let usa_lines: Vec<(&str, Decimal, Decimal)> = usa_per_account
        .iter()
        .map(|(k, v)| (k.as_str(), v.0, v.1))
        .collect();

    let pre_elim = aggregate_pre_elimination(
        &state.manifest,
        &[
            (
                "ACME_SA".to_string(),
                build_multi_line_tb(
                    "ACME_SA",
                    "CHF",
                    &[
                        ("1150", sa_ic, Decimal::ZERO),
                        ("4500", Decimal::ZERO, sa_ic),
                    ],
                ),
            ),
            (
                "ACME_USA".to_string(),
                build_multi_line_tb("ACME_USA", "CHF", &usa_lines),
            ),
        ],
    )
    .expect("aggregate");

    let a = apply_eliminations_to_tb(&pre_elim, &state.elim_jes).expect("apply a");
    let b = apply_eliminations_to_tb(&pre_elim, &state.elim_jes).expect("apply b");

    let a_json = serde_json::to_vec(&a).expect("serialise a");
    let b_json = serde_json::to_vec(&b).expect("serialise b");
    assert_eq!(
        a_json, b_json,
        "two calls with identical input must produce byte-identical JSON"
    );
    assert_eq!(a, b, "structural equality must also hold");
}

/// Pass-through invariants: post-elim copies pre-elim's identity-bearing
/// fields (`group_id`, `currency`, `as_of_date`, `contributing_entities`,
/// `deferred_entities`) verbatim.
#[test]
fn passes_through_identity_fields_unchanged() {
    let state = build_pipeline_state();
    let pre_elim = aggregate_pre_elimination(
        &state.manifest,
        &[
            (
                "ACME_SA".to_string(),
                build_balanced_tb("ACME_SA", "CHF", "1100", "3100", dec!(1000)),
            ),
            (
                "ACME_USA".to_string(),
                build_balanced_tb("ACME_USA", "CHF", "1100", "3100", dec!(2000)),
            ),
        ],
    )
    .expect("aggregate");

    let post_elim = apply_eliminations_to_tb(&pre_elim, &state.elim_jes).expect("apply");

    assert_eq!(post_elim.group_id, pre_elim.group_id);
    assert_eq!(post_elim.currency, pre_elim.currency);
    assert_eq!(post_elim.as_of_date, pre_elim.as_of_date);
    assert_eq!(
        post_elim.contributing_entities, pre_elim.contributing_entities,
        "contributing_entities must pass through unchanged"
    );
    assert_eq!(
        post_elim.deferred_entities, pre_elim.deferred_entities,
        "deferred_entities must pass through unchanged"
    );
}

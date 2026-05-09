//! Task 9.1 — Aggregate phase driver end-to-end.
//!
//! Exercises [`datasynth_group::run_aggregate`] against a hand-built
//! Mini-Nestlé shard archive — no orchestrator runs.  We synthesise
//! per-entity TBs and JEs by writing them to disk in the same shape the
//! shard runner emits, then drive the aggregate phase against the
//! synthesised tree.
//!
//! # Why hand-rolled fixtures?
//!
//! Driving [`datasynth_runtime::EnhancedOrchestrator`] for two entities
//! peaks at ~17 GiB RSS and 30+ minutes of wallclock — see
//! `tests/shard_e2e.rs`.  The aggregate driver itself is pure Rust
//! over balanced TBs / JEs, so we can test it end-to-end in seconds by
//! materialising the shard archive on disk directly.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tempfile::TempDir;

use datasynth_core::accounts::{cash_accounts, control_accounts, equity_accounts};
use datasynth_core::models::balance::{
    AccountCategory, AccountType, TrialBalance, TrialBalanceLine, TrialBalanceType,
};
use datasynth_core::models::JournalEntry;

use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::shard::{derive_ic_pair_plans, inject_ic_journal_entries, InjectionCtx};
use datasynth_group::{
    build_manifest, run_aggregate, AggregateOptions, GroupConfig, GroupError, IcRelationshipConfig,
    CONSOLIDATED_FS_FILENAME, CONSOLIDATION_SCHEDULE_FILENAME, COVERAGE_REPORT_FILENAME,
    CTA_ROLLFORWARD_FILENAME, EQUITY_METHOD_INVESTMENTS_FILENAME, NCI_ROLLFORWARD_FILENAME,
    NOTES_FILENAME, TRANSLATION_WORKSHEET_FILENAME,
};

// ── Fixture helpers ───────────────────────────────────────────────────────────

/// Mirror of `tests/elimination_to_je.rs::load_two_entity_manifest`.
/// Keep the trim in lockstep so any mini_nestle.yaml change flows
/// through both files.
fn load_two_entity_manifest() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let mut cfg: GroupConfig = serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse");

    cfg.ownership
        .entities
        .retain(|e| matches!(e.code.as_str(), "NESTLE_SA" | "NESTLE_USA"));

    cfg.intercompany.relationships.retain(|r| match r {
        IcRelationshipConfig::Explicit(e) => e.seller == "NESTLE_SA" && e.buyer == "NESTLE_USA",
        IcRelationshipConfig::Pattern(_) => false,
    });
    assert_eq!(
        cfg.intercompany.relationships.len(),
        1,
        "trim must leave exactly one explicit NESTLE_SA→NESTLE_USA relationship",
    );

    if let Some(p2) = cfg.tax.pillar_two.as_mut() {
        p2.jurisdictions
            .retain(|j| matches!(j.as_str(), "CH" | "US"));
    }
    if let Some(tp) = cfg.tax.transfer_pricing.as_mut() {
        tp.local_files_for
            .retain(|j| matches!(j.as_str(), "CH" | "US"));
    }

    build_manifest(&cfg).expect("trimmed manifest must build")
}

fn period_end() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
}

/// Build a balanced standalone TB in CHF for `entity_code`, including
/// the IC posting effects so the consolidated BS identity holds after
/// IC eliminations are applied.
///
/// The trimmed Mini-Nestlé fixture's explicit SA→USA `goods_sale`
/// relationship resolves to `annual_volume / avg_amount(GoodsSale)`
/// = `5_000_000 / 50_000` = **100 pairs**.  Each side's IC JEs total
/// `5,000,000` on the IC clearing accounts (`1150` for seller AR,
/// `2050` for buyer AP) plus the matching revenue / COGS legs.
///
/// To mirror what the orchestrator would produce, we model the closed
/// state — period P&L has been closed into retained earnings, so the
/// TB has no `4xxx`–`9xxx` lines, just BS accounts.  The seller's
/// 5,000,000 IC AR is balanced by 5,000,000 of additional retained
/// earnings (the seller booked revenue and closed it); the buyer's
/// 5,000,000 IC AP is balanced by 5,000,000 of additional inventory
/// (the buyer received goods and hasn't sold them).
///
/// Standalone BS identity holds per entity: A = L + E.  After IC
/// eliminations zero out the `1150` and `2050` legs on consolidation,
/// the consolidated BS identity also holds.
///
/// # Layout (per entity)
///
/// **Seller (NESTLE_SA)**:
/// ```text
/// DR cash                 10,000
/// DR AR                    5,000
/// DR IC AR (1150)      5,000,000
/// CR AP                    3,000
/// CR common stock          3,000
/// CR retained earnings 5,009,000   ← closed-period RE incl. IC sales
/// ```
///
/// **Buyer (NESTLE_USA)**:
/// ```text
/// DR cash                 10,000
/// DR AR                    5,000
/// DR inventory (1200)  5,000,000
/// CR AP                    3,000
/// CR IC AP (2050)      5,000,000
/// CR common stock          3,000
/// CR retained earnings     9,000
/// ```
fn make_balanced_tb(entity_code: &str) -> TrialBalance {
    let mut tb = TrialBalance::new(
        format!("TB_{entity_code}"),
        entity_code.to_string(),
        period_end(),
        2024,
        3,
        "CHF".to_string(),
        TrialBalanceType::Adjusted,
    );

    let push_dr = |tb: &mut TrialBalance, code: &str, ty: AccountType, amt: Decimal| {
        tb.add_line(TrialBalanceLine {
            account_code: code.to_string(),
            account_description: code.to_string(),
            category: AccountCategory::from_account_type(ty),
            account_type: ty,
            opening_balance: Decimal::ZERO,
            period_debits: amt,
            period_credits: Decimal::ZERO,
            closing_balance: amt,
            debit_balance: amt,
            credit_balance: Decimal::ZERO,
            cost_center: None,
            profit_center: None,
        });
    };
    let push_cr = |tb: &mut TrialBalance, code: &str, ty: AccountType, amt: Decimal| {
        tb.add_line(TrialBalanceLine {
            account_code: code.to_string(),
            account_description: code.to_string(),
            category: AccountCategory::from_account_type(ty),
            account_type: ty,
            opening_balance: Decimal::ZERO,
            period_debits: Decimal::ZERO,
            period_credits: amt,
            closing_balance: amt,
            debit_balance: Decimal::ZERO,
            credit_balance: amt,
            cost_center: None,
            profit_center: None,
        });
    };

    // Common BS lines on both sides.
    push_dr(
        &mut tb,
        cash_accounts::OPERATING_CASH,
        AccountType::Asset,
        dec!(10000),
    );
    push_dr(
        &mut tb,
        control_accounts::AR_CONTROL,
        AccountType::Asset,
        dec!(5000),
    );
    push_cr(
        &mut tb,
        control_accounts::AP_CONTROL,
        AccountType::Liability,
        dec!(3000),
    );
    push_cr(
        &mut tb,
        equity_accounts::COMMON_STOCK,
        AccountType::Equity,
        dec!(3000),
    );

    // Per-side IC postings + RE adjustment to keep the standalone TB
    // balanced and the consolidated BS identity honest after IC
    // eliminations.  Total IC volume = 100 pairs × 50_000 each =
    // 5_000_000, mirroring the trimmed manifest's resolved IC plan.
    if entity_code == "NESTLE_SA" {
        // Seller — IC AR (1150) booked, revenue closed into RE.
        push_dr(
            &mut tb,
            control_accounts::IC_AR_CLEARING,
            AccountType::Asset,
            dec!(5_000_000),
        );
        push_cr(
            &mut tb,
            equity_accounts::RETAINED_EARNINGS,
            AccountType::Equity,
            dec!(5_009_000),
        );
    } else {
        // Buyer — IC AP (2050) booked, COGS closed into inventory
        // (no revenue yet, so RE matches the "no IC sales" baseline).
        push_dr(
            &mut tb,
            control_accounts::INVENTORY,
            AccountType::Asset,
            dec!(5_000_000),
        );
        push_cr(
            &mut tb,
            control_accounts::IC_AP_CLEARING,
            AccountType::Liability,
            dec!(5_000_000),
        );
        push_cr(
            &mut tb,
            equity_accounts::RETAINED_EARNINGS,
            AccountType::Equity,
            dec!(9000),
        );
    }

    tb
}

/// Persist a per-entity shard archive at the canonical path the
/// aggregate driver expects:
///
/// ```text
/// {root}/entities/{entity_code}/period_close/trial_balances.json
/// {root}/entities/{entity_code}/journal_entries.json
/// ```
///
/// Mirrors what `crate::shard::run_shard` writes — see
/// `crates/datasynth-runtime/src/output_writer.rs`.
fn write_entity_archive(root: &Path, entity_code: &str, tb: &TrialBalance, jes: &[JournalEntry]) {
    let entity_dir = root.join("entities").join(entity_code);
    fs::create_dir_all(entity_dir.join("period_close")).expect("mkdir period_close");

    let tb_array = vec![tb.clone()];
    let tb_json = serde_json::to_string_pretty(&tb_array).expect("serialize tb");
    fs::write(
        entity_dir.join("period_close").join("trial_balances.json"),
        tb_json,
    )
    .expect("write trial_balances.json");

    let je_json = serde_json::to_string_pretty(jes).expect("serialize jes");
    fs::write(entity_dir.join("journal_entries.json"), je_json)
        .expect("write journal_entries.json");
}

/// Materialise the full Mini-Nestlé two-entity shard archive in `root`.
/// Each entity gets:
/// - one balanced TB at `period_close/trial_balances.json`,
/// - the IC JE legs the manifest derives for that entity at
///   `journal_entries.json`.
fn materialise_two_entity_shards(manifest: &GroupManifest, root: &Path) -> BTreeMap<String, usize> {
    let mut ic_counts: BTreeMap<String, usize> = BTreeMap::new();
    for entity in &manifest.ownership_graph.entities {
        let plans = derive_ic_pair_plans(manifest, &entity.code);
        let jes = inject_ic_journal_entries(
            &plans,
            &InjectionCtx {
                entity_code: entity.code.clone(),
            },
        );
        ic_counts.insert(entity.code.clone(), jes.len());
        let tb = make_balanced_tb(&entity.code);
        write_entity_archive(root, &entity.code, &tb, &jes);
    }
    ic_counts
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn run_aggregate_writes_full_artifact_set_in_emission_order() {
    let manifest = load_two_entity_manifest();
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();

    let ic_counts = materialise_two_entity_shards(&manifest, root);
    // Sanity: each side must have at least one IC leg or the test
    // can't validate matching coverage downstream.
    for (code, n) in &ic_counts {
        assert!(*n > 0, "{code} must produce at least one IC JE");
    }

    let summary = run_aggregate(&manifest, root, root, &AggregateOptions::default())
        .expect("run_aggregate must succeed");

    // ── Top-level shape ─────────────────────────────────────────────
    assert_eq!(summary.group_id, manifest.group_id);
    assert_eq!(summary.presentation_currency, "CHF");
    assert_eq!(summary.as_of_date, manifest.period.end);
    assert_eq!(
        summary.entities_processed,
        vec!["NESTLE_SA".to_string(), "NESTLE_USA".to_string()],
        "both entities present, sorted lexicographically",
    );
    assert!(summary.entities_missing.is_empty());
    assert!(
        summary.deferred_entities.is_empty(),
        "trimmed Mini-Nestlé has no equity-method investees",
    );
    assert!(
        summary.matched_pairs > 0,
        "pre-IC matching must produce ≥1 matched pair from the SA↔USA goods_sale relationship",
    );
    // Coverage = matched / total_planned.  Both sides observed ⇒ 1.0.
    assert!(
        (summary.coverage - 1.0).abs() < 1e-9,
        "expected coverage 1.0, got {}",
        summary.coverage,
    );

    // ── Artifact paths in emission order ───────────────────────────
    // The driver emits 8 files:
    //   1. ic_eliminations/ic_matching_coverage.json
    //   2. consolidated/cta_rollforward.json
    //   3. consolidated/translation_worksheet.json
    //   4. consolidated/nci_rollforward.json
    //   5. consolidated/equity_method_investments.json
    //   6. consolidated/consolidated_financial_statements.json
    //   7. consolidated/consolidation_schedule.json
    //   8. consolidated/notes_to_consolidated_fs.json
    assert_eq!(summary.artifacts_written.len(), 8, "8 artefacts expected");
    assert!(
        summary.artifacts_written[0].ends_with(COVERAGE_REPORT_FILENAME),
        "coverage report first; got {:?}",
        summary.artifacts_written[0],
    );
    assert!(summary.artifacts_written[1].ends_with(CTA_ROLLFORWARD_FILENAME));
    assert!(summary.artifacts_written[2].ends_with(TRANSLATION_WORKSHEET_FILENAME));
    assert!(summary.artifacts_written[3].ends_with(NCI_ROLLFORWARD_FILENAME));
    assert!(summary.artifacts_written[4].ends_with(EQUITY_METHOD_INVESTMENTS_FILENAME));
    assert!(summary.artifacts_written[5].ends_with(CONSOLIDATED_FS_FILENAME));
    assert!(summary.artifacts_written[6].ends_with(CONSOLIDATION_SCHEDULE_FILENAME));
    assert!(summary.artifacts_written[7].ends_with(NOTES_FILENAME));

    // Every artefact must exist on disk.
    for p in &summary.artifacts_written {
        assert!(p.exists(), "artefact must exist on disk: {}", p.display());
    }

    // ── BS identity ────────────────────────────────────────────────
    // assets ≈ liabilities + equity + nci within 0.01 (the BS builder
    // enforces this and would have errored otherwise — re-check the
    // numbers landed on the summary correctly).
    let total_lpe = summary.total_liabilities + summary.total_equity + summary.total_nci;
    let diff = (summary.total_assets - total_lpe).abs();
    assert!(
        diff <= dec!(0.01),
        "summary BS identity broken: A={} L={} E={} NCI={}, diff={}",
        summary.total_assets,
        summary.total_liabilities,
        summary.total_equity,
        summary.total_nci,
        diff,
    );

    // ── v5.10 je_network artefacts ─────────────────────────────────
    // Per-entity files
    for entity in ["NESTLE_SA", "NESTLE_USA"] {
        let entity_csv = root
            .join("entities")
            .join(entity)
            .join("graphs")
            .join("je_network.csv");
        let entity_pq = root
            .join("entities")
            .join(entity)
            .join("graphs")
            .join("je_network.parquet");
        assert!(
            entity_csv.exists(),
            "missing per-entity je_network.csv: {:?}",
            entity_csv
        );
        assert!(
            entity_pq.exists(),
            "missing per-entity je_network.parquet: {:?}",
            entity_pq
        );

        // Header sanity: 15 columns, ic_pair_id + ic_partner_entity present
        let body = std::fs::read_to_string(&entity_csv).expect("read entity csv");
        let header = body.lines().next().expect("non-empty");
        assert!(
            header.contains("ic_pair_id") && header.contains("ic_partner_entity"),
            "entity csv header missing v5.10 IC columns: {header}",
        );
        let line_count = body.lines().count();
        assert!(
            line_count >= 2,
            "expect at least header + 1 row; got {line_count}"
        );
    }

    // Consolidated file
    let consol_csv = root.join("consolidated").join("je_network.csv");
    let consol_pq = root.join("consolidated").join("je_network.parquet");
    assert!(consol_csv.exists(), "missing consolidated je_network.csv");
    assert!(
        consol_pq.exists(),
        "missing consolidated je_network.parquet"
    );

    let consol_body = std::fs::read_to_string(&consol_csv).expect("read consolidated csv");
    let consol_header = consol_body.lines().next().expect("non-empty");
    for col in [
        "edge_id",
        "entity_code",
        "ic_pair_id",
        "ic_partner_entity",
        "is_eliminated",
        "eliminates_ic_pair_id",
    ] {
        assert!(
            consol_header.contains(col),
            "consolidated header missing column `{col}`: {consol_header}",
        );
    }

    // At least one row should have is_eliminated=true (Mini-Nestlé has
    // matched IC pairs that produce eliminations).
    let has_elim = consol_body.lines().skip(1).any(|l| {
        // find `,true,` in the right column position is fragile; just
        // require *any* row to contain the literal `,true,` substring
        // — the only true booleans are is_fraud / is_anomaly /
        // is_eliminated and the test fixture sets neither fraud nor
        // anomaly, so any `,true,` must be is_eliminated.
        l.contains(",true,")
    });
    assert!(
        has_elim,
        "expected at least one is_eliminated=true row in consolidated csv"
    );

    // At least one row should carry an IC pair id (matched seller/buyer).
    let has_ic = consol_body.lines().skip(1).any(|l| {
        l.split(',')
            .nth(14)
            .is_some_and(|c| !c.is_empty() && c != "\"\"")
    });
    assert!(
        has_ic,
        "expected at least one row with non-empty ic_pair_id in consolidated csv"
    );
}

#[test]
fn run_aggregate_strict_mode_errors_on_missing_shard() {
    let manifest = load_two_entity_manifest();
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();

    // Materialise only NESTLE_SA — leave NESTLE_USA missing.
    let plans = derive_ic_pair_plans(&manifest, "NESTLE_SA");
    let jes = inject_ic_journal_entries(
        &plans,
        &InjectionCtx {
            entity_code: "NESTLE_SA".to_string(),
        },
    );
    let tb = make_balanced_tb("NESTLE_SA");
    write_entity_archive(root, "NESTLE_SA", &tb, &jes);

    let opts = AggregateOptions {
        prior_period_aggregate: None,
        tolerate_missing_shards: false,
        cgu_test_inputs: Vec::new(),
        cpi_series_by_currency: std::collections::BTreeMap::new(),
    };
    let err = run_aggregate(&manifest, root, root, &opts).expect_err("strict mode must reject");
    match err {
        GroupError::Aggregate(msg) => {
            assert!(
                msg.contains("NESTLE_USA"),
                "error must name the missing entity: {msg}",
            );
            assert!(
                msg.contains("trial_balances.json"),
                "error must point at the missing path: {msg}",
            );
        }
        other => panic!("expected GroupError::Aggregate, got {other:?}"),
    }
}

#[test]
fn run_aggregate_tolerate_missing_shards_continues_on_partial_archive() {
    let manifest = load_two_entity_manifest();
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();

    // Materialise only NESTLE_SA — leave NESTLE_USA missing.
    let plans = derive_ic_pair_plans(&manifest, "NESTLE_SA");
    let jes = inject_ic_journal_entries(
        &plans,
        &InjectionCtx {
            entity_code: "NESTLE_SA".to_string(),
        },
    );
    let tb = make_balanced_tb("NESTLE_SA");
    write_entity_archive(root, "NESTLE_SA", &tb, &jes);

    let opts = AggregateOptions {
        prior_period_aggregate: None,
        tolerate_missing_shards: true,
        cgu_test_inputs: Vec::new(),
        cpi_series_by_currency: std::collections::BTreeMap::new(),
    };
    let summary =
        run_aggregate(&manifest, root, root, &opts).expect("tolerate-missing must succeed");

    assert_eq!(summary.entities_processed, vec!["NESTLE_SA".to_string()]);
    assert_eq!(summary.entities_missing, vec!["NESTLE_USA".to_string()]);
    // No counterpart for SA's IC legs ⇒ all unmatched, zero matched.
    assert_eq!(
        summary.matched_pairs, 0,
        "no buyer-side observed ⇒ zero matched pairs",
    );
    // SA-only run still emits all 8 artefacts.
    assert_eq!(summary.artifacts_written.len(), 8);
    for p in &summary.artifacts_written {
        assert!(p.exists(), "artefact missing: {}", p.display());
    }
}

#[test]
fn run_aggregate_empty_archive_with_tolerate_missing_produces_zero_summary() {
    // Edge case: no entities written at all, tolerate_missing_shards
    // = true.  The driver's pre-elim aggregator handles empty input
    // gracefully (returns zero-valued AggregatedTb), so the whole
    // pipeline should run end-to-end without error.
    let manifest = load_two_entity_manifest();
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("entities")).expect("mkdir entities/");

    let opts = AggregateOptions {
        prior_period_aggregate: None,
        tolerate_missing_shards: true,
        cgu_test_inputs: Vec::new(),
        cpi_series_by_currency: std::collections::BTreeMap::new(),
    };
    let summary = run_aggregate(&manifest, root, root, &opts).expect("must succeed");

    assert!(summary.entities_processed.is_empty());
    assert_eq!(
        summary.entities_missing,
        vec!["NESTLE_SA".to_string(), "NESTLE_USA".to_string()],
    );
    assert_eq!(summary.matched_pairs, 0);
    assert_eq!(summary.coverage, 0.0);
    // Every artefact still emitted — empty inputs produce empty outputs
    // but the file shape is preserved for downstream consumers.
    assert_eq!(summary.artifacts_written.len(), 8);
}

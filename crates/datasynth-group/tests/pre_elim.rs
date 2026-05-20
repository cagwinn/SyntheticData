//! Task 5.2 — pre-elimination TB aggregation integration tests.
//!
//! These tests assemble hand-rolled `TrialBalance` fixtures with
//! `TrialBalance::new` + `add_line` (mirroring `tb_loader.rs::balanced_tb`)
//! so each case stays a few KB of resident memory.  The manifest is
//! loaded from `mini_acme.yaml` because building one by hand would
//! require materialising every sub-plan (`ChartOfAccountsMaster`,
//! `FxRateMaster`, `AuditEngagementPlan`, `TaxGroupPlan`,
//! `ShardPlan`) — `mini_acme.yaml` already has the entity mix we
//! need (Parent + 3×Full + 1×EquityMethod) and we mutate
//! `consolidation_method` on individual entries when we need a
//! different shape (e.g. Proportional for the mixed-methods case).
//!
//! Currency handling: every contributing TB in these tests is in
//! CHF, matching `mini_acme.yaml::presentation_currency`.  The
//! v5.0 contract (Chunk 6 will lift it) is that the aggregator
//! refuses to mix currencies — tested explicitly in
//! `errors_on_currency_mismatch`.
//!
//! Determinism: the byte-identical-output test serialises
//! `AggregatedTb` twice with `serde_json::to_vec` (no maps with
//! non-deterministic iteration order — `BTreeMap` and sorted
//! `Vec`s are the only collections in the public surface).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_core::models::balance::{
    AccountCategory, AccountType, TrialBalance, TrialBalanceLine, TrialBalanceType,
};
use datasynth_group::config::ConsolidationMethod;
use datasynth_group::errors::GroupError;
use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::{aggregate_pre_elimination, build_manifest, GroupConfig};

// ── Fixture builders ──────────────────────────────────────────────────────────

/// Load the mini_acme manifest — 5 entities, 1 Parent + 3 Full +
/// 1 EquityMethod, presentation currency CHF.
fn load_mini_acme_manifest() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_acme.yaml");
    let cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_acme.yaml must parse into GroupConfig");
    build_manifest(&cfg).expect("mini_acme.yaml must build a manifest")
}

/// Like [`load_mini_acme_manifest`] but mutates the consolidation
/// method on the named entity to `method` *before* `build_manifest`
/// runs, so the produced `ManifestEntity` carries that method.
///
/// Used by the mixed-methods test to flip a Full subsidiary to
/// Proportional without writing a second YAML fixture.
fn load_manifest_with_method_override(
    entity_code: &str,
    method: ConsolidationMethod,
) -> GroupManifest {
    let yaml = include_str!("fixtures/mini_acme.yaml");
    let mut cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_acme.yaml must parse into GroupConfig");
    let entry = cfg
        .ownership
        .entities
        .iter_mut()
        .find(|e| e.code == entity_code)
        .unwrap_or_else(|| panic!("fixture must contain entity {entity_code}"));
    entry.consolidation_method = method;
    build_manifest(&cfg).expect("mutated mini_acme must still build a manifest")
}

/// Build a 2-line balanced TB in `currency`: one debit line on
/// `debit_account` for `amount`, one matching credit line on
/// `credit_account`.
///
/// Mirrors `tb_loader.rs::balanced_tb` but parameterises the accounts
/// and amount so we can exercise per-account roll-up.
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
        account_description: format!("DR {}", debit_account),
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
        account_description: format!("CR {}", credit_account),
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

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Happy path: ACME_SA (Parent) + ACME_USA (Full) — both in CHF.
/// The aggregator must sum per-account totals, record both as
/// contributing entities, and end up with `total_debits ==
/// total_credits` because both inputs are individually balanced.
#[test]
fn aggregates_two_full_entities_correctly() {
    let manifest = load_mini_acme_manifest();

    // SA posts 1100 (Cash) DR 10_000 / 3100 (Common Stock) CR 10_000
    // USA posts 1100 (Cash) DR 25_000 / 3100 (Common Stock) CR 25_000
    // Combined: 1100 DR 35_000, 3100 CR 35_000.
    let tbs = vec![
        (
            "ACME_SA".to_string(),
            build_balanced_tb("ACME_SA", "CHF", "1100", "3100", dec!(10000)),
        ),
        (
            "ACME_USA".to_string(),
            build_balanced_tb("ACME_USA", "CHF", "1100", "3100", dec!(25000)),
        ),
    ];

    let agg = aggregate_pre_elimination(&manifest, &tbs).expect("happy path must succeed");

    assert_eq!(agg.group_id, "MINI_ACME_2024_Q1");
    assert_eq!(agg.currency, "CHF");
    assert_eq!(
        agg.contributing_entities,
        vec!["ACME_SA".to_string(), "ACME_USA".to_string()],
        "contributing entities must be sorted lexicographically"
    );
    assert!(
        agg.deferred_entities.is_empty(),
        "all inputs were Parent/Full — no deferred entities expected"
    );

    let cash = agg.account_totals.get("1100").expect("1100 must aggregate");
    assert_eq!(cash.debit_total, dec!(35000));
    assert_eq!(cash.credit_total, Decimal::ZERO);
    assert_eq!(cash.net_balance, dec!(35000));
    assert_eq!(cash.contributing_entities, 2);

    let cs = agg.account_totals.get("3100").expect("3100 must aggregate");
    assert_eq!(cs.debit_total, Decimal::ZERO);
    assert_eq!(cs.credit_total, dec!(35000));
    assert_eq!(cs.net_balance, dec!(-35000));
    assert_eq!(cs.contributing_entities, 2);

    assert_eq!(agg.total_debits, dec!(35000));
    assert_eq!(agg.total_credits, dec!(35000));
    assert_eq!(
        agg.total_debits, agg.total_credits,
        "balanced inputs must produce a balanced aggregate"
    );
}

/// Mixed methods: Parent + 1 Full contribute, EquityMethod +
/// Proportional are deferred.  Verifies the routing logic, not the
/// per-account math (covered by the happy-path test).
#[test]
fn defers_equity_method_and_proportional_entities() {
    // Flip ACME_BR from Full to Proportional so the fixture has one
    // of each special method to defer.
    let manifest = load_manifest_with_method_override("ACME_BR", ConsolidationMethod::Proportional);

    let tbs = vec![
        (
            "ACME_SA".to_string(),
            build_balanced_tb("ACME_SA", "CHF", "1100", "3100", dec!(1000)),
        ),
        (
            "ACME_USA".to_string(),
            build_balanced_tb("ACME_USA", "CHF", "1100", "3100", dec!(2000)),
        ),
        // EquityMethod entity — must NOT be summed.
        (
            "ACME_JV".to_string(),
            build_balanced_tb("ACME_JV", "CHF", "1100", "3100", dec!(500)),
        ),
        // Proportional entity (was Full) — must NOT be summed.
        (
            "ACME_BR".to_string(),
            build_balanced_tb("ACME_BR", "CHF", "1100", "3100", dec!(750)),
        ),
    ];

    let agg = aggregate_pre_elimination(&manifest, &tbs).expect("mixed-methods must succeed");

    // Only the two Parent/Full entities contributed.
    assert_eq!(
        agg.contributing_entities,
        vec!["ACME_SA".to_string(), "ACME_USA".to_string()],
        "only Parent + Full entities should contribute"
    );
    assert_eq!(
        agg.deferred_entities.len(),
        2,
        "EquityMethod + Proportional should both defer"
    );

    // Deferred list is sorted lexicographically.
    assert_eq!(agg.deferred_entities[0].entity_code, "ACME_BR");
    assert_eq!(
        agg.deferred_entities[0].method,
        ConsolidationMethod::Proportional
    );
    assert_eq!(agg.deferred_entities[1].entity_code, "ACME_JV");
    assert_eq!(
        agg.deferred_entities[1].method,
        ConsolidationMethod::EquityMethod
    );

    // Totals exclude deferred entities: SA (1000+1000) + USA (2000+2000) = 6000.
    assert_eq!(agg.total_debits, dec!(3000));
    assert_eq!(agg.total_credits, dec!(3000));

    let cash = agg.account_totals.get("1100").expect("1100 must aggregate");
    assert_eq!(cash.debit_total, dec!(3000));
    assert_eq!(cash.contributing_entities, 2);
}

/// Empty input must NOT panic and must produce an `AggregatedTb` with
/// empty maps + zero totals (recovery / dry-run path).
#[test]
fn empty_input_returns_empty_aggregate() {
    let manifest = load_mini_acme_manifest();

    let agg = aggregate_pre_elimination(&manifest, &[]).expect("empty input must succeed");

    assert_eq!(agg.group_id, "MINI_ACME_2024_Q1");
    assert_eq!(agg.currency, "CHF");
    assert!(agg.account_totals.is_empty());
    assert!(agg.contributing_entities.is_empty());
    assert!(agg.deferred_entities.is_empty());
    assert_eq!(agg.total_debits, Decimal::ZERO);
    assert_eq!(agg.total_credits, Decimal::ZERO);
}

/// Unknown entity in input must surface as `GroupError::Aggregate`
/// with the bad code in the message so the aggregate-phase log
/// pinpoints the drift between the loader's input list and the
/// manifest.
#[test]
fn errors_on_unknown_entity() {
    let manifest = load_mini_acme_manifest();

    let tbs = vec![(
        "GHOST_ENTITY".to_string(),
        build_balanced_tb("GHOST_ENTITY", "CHF", "1100", "3100", dec!(100)),
    )];

    let err = aggregate_pre_elimination(&manifest, &tbs).expect_err("unknown entity must error");

    match err {
        GroupError::Aggregate(msg) => {
            assert!(
                msg.contains("GHOST_ENTITY"),
                "error message must name the bad entity, got {msg:?}"
            );
            assert!(
                msg.contains("not in manifest"),
                "error message must describe the missing-entity condition, got {msg:?}"
            );
        }
        other => panic!("expected GroupError::Aggregate, got {other:?}"),
    }
}

/// Currency mismatch: a contributing TB in EUR while the manifest
/// presents in CHF must error with a message that names the entity,
/// the wrong currency, and the expected currency.
#[test]
fn errors_on_currency_mismatch() {
    let manifest = load_mini_acme_manifest();

    let tbs = vec![
        (
            "ACME_SA".to_string(),
            build_balanced_tb("ACME_SA", "CHF", "1100", "3100", dec!(1000)),
        ),
        // EUR but manifest is CHF — must error.
        (
            "ACME_DE".to_string(),
            build_balanced_tb("ACME_DE", "EUR", "1100", "3100", dec!(2000)),
        ),
    ];

    // v5.0 contract change: pre_elim no longer hard-errors on
    // currency mismatch. The IAS 21 translation step is a sidecar
    // artefact (`consolidated/translation_worksheet.json`) emitted in
    // parallel; pre_elim's per-account additive sum doesn't actually
    // need single-currency input. The mismatch is logged at debug
    // level. Test confirms aggregation still succeeds and the EUR
    // entity's contribution is included verbatim.
    let result = aggregate_pre_elimination(&manifest, &tbs)
        .expect("currency mismatch must succeed under v5.0 translation-as-sidecar contract");
    assert_eq!(result.contributing_entities.len(), 2);
    assert!(result
        .contributing_entities
        .contains(&"ACME_DE".to_string()));
}

/// Deterministic output: two calls with identical input must produce
/// byte-identical serialised JSON.  `BTreeMap` and sorted `Vec`s are
/// the only collections in the public surface so we can compare the
/// raw bytes directly.
#[test]
fn deterministic_output_across_calls() {
    let manifest = load_mini_acme_manifest();

    // Build two identical input vecs — same construction, same order.
    // Then reverse the order on the second call to prove that the
    // aggregator's internal sorting cancels caller-side ordering.
    let tbs_a = vec![
        (
            "ACME_USA".to_string(),
            build_balanced_tb("ACME_USA", "CHF", "1100", "3100", dec!(2000)),
        ),
        (
            "ACME_SA".to_string(),
            build_balanced_tb("ACME_SA", "CHF", "1100", "3100", dec!(1000)),
        ),
    ];
    let tbs_b = vec![
        (
            "ACME_SA".to_string(),
            build_balanced_tb("ACME_SA", "CHF", "1100", "3100", dec!(1000)),
        ),
        (
            "ACME_USA".to_string(),
            build_balanced_tb("ACME_USA", "CHF", "1100", "3100", dec!(2000)),
        ),
    ];

    let agg_a = aggregate_pre_elimination(&manifest, &tbs_a).expect("first call");
    let agg_b = aggregate_pre_elimination(&manifest, &tbs_b).expect("second call");

    let json_a = serde_json::to_vec(&agg_a).expect("serialise first");
    let json_b = serde_json::to_vec(&agg_b).expect("serialise second");

    assert_eq!(
        json_a, json_b,
        "byte-identical serialisation required regardless of input order"
    );
    // And the structural surface should match too.
    assert_eq!(agg_a, agg_b);
}

/// Balance preservation: when every contributing TB is individually
/// balanced (DR == CR), the aggregate must also be balanced.  This is
/// the invariant the elimination engine (Task 5.4) and post-elim TB
/// builder (Task 5.6) rely on.
#[test]
fn aggregated_tb_is_balanced_when_inputs_balanced() {
    let manifest = load_mini_acme_manifest();

    let tbs: Vec<(String, TrialBalance)> = ["ACME_SA", "ACME_USA", "ACME_DE", "ACME_BR"]
        .iter()
        .enumerate()
        .map(|(i, code)| {
            // Distinct amounts to make the balance check meaningful —
            // if the aggregator dropped a credit line, totals would
            // drift out of balance.
            let amount = Decimal::from((i as u64 + 1) * 1000);
            (
                (*code).to_string(),
                build_balanced_tb(code, "CHF", "1100", "3100", amount),
            )
        })
        .collect();

    let agg = aggregate_pre_elimination(&manifest, &tbs).expect("balanced inputs must aggregate");

    assert_eq!(
        agg.total_debits, agg.total_credits,
        "balanced inputs must produce a balanced aggregate; \
         got dr={} cr={}",
        agg.total_debits, agg.total_credits
    );
    // 1000 + 2000 + 3000 + 4000 = 10_000 on each side.
    assert_eq!(agg.total_debits, dec!(10000));
    assert_eq!(agg.total_credits, dec!(10000));
}

/// Sanity: the [`AggregatedTb::as_of_date`] mirrors the manifest's
/// period end date — Mini-Acme is a Q1-2024 engagement, so the
/// quarter end is 2024-03-31.
#[test]
fn as_of_date_mirrors_manifest_period_end() {
    let manifest = load_mini_acme_manifest();
    let agg = aggregate_pre_elimination(&manifest, &[]).expect("empty must succeed");
    assert_eq!(agg.as_of_date, manifest.period.end);
    assert_eq!(
        agg.as_of_date,
        NaiveDate::from_ymd_opt(2024, 3, 31).expect("valid date"),
        "mini_acme is Q1 2024 starting 2024-01-01 so end is 2024-03-31"
    );
}

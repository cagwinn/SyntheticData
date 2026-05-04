//! Integration tests for FX rate master resolution (Task 2.5).

use chrono::NaiveDate;
use datasynth_group::config::{
    ConsolidationMethod, FxConfig, FxPolicyConfig, FxRateBasis, FxRateSource,
};
use datasynth_group::manifest::expansion::{EntitySource, ExpandedEntity};
use datasynth_group::manifest::fx_master::build_fx_master;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::BTreeMap;

// ── Helpers ────────────────────────────────────────────────────────────────

fn make_entity(code: &str, functional_currency: &str) -> ExpandedEntity {
    ExpandedEntity {
        code: code.to_string(),
        name: None,
        country: "US".to_string(),
        functional_currency: functional_currency.to_string(),
        scoping_profile: "standard".to_string(),
        consolidation_method: ConsolidationMethod::Full,
        ownership_percent: Some(dec!(1.0)),
        parent_code: None,
        accounting_framework: None,
        industry: None,
        source: EntitySource::Explicit,
        generated_block_index: None,
        rows: None,
        hyperinflation_status: datasynth_core::models::HyperinflationStatus::NotHyperinflationary,
        ownership_changes: Vec::new(),
    }
}

fn default_policy() -> FxPolicyConfig {
    FxPolicyConfig {
        balance_sheet: FxRateBasis::Closing,
        income_statement: FxRateBasis::Average,
        equity: FxRateBasis::Historical,
    }
}

fn inline_cfg(
    base_currency: &str,
    rates: BTreeMap<String, BTreeMap<NaiveDate, Decimal>>,
) -> FxConfig {
    FxConfig {
        base_currency: base_currency.to_string(),
        rate_source: FxRateSource::Inline,
        rates,
        policy: default_policy(),
    }
}

// ── Test 1: happy path — single rate at period end ─────────────────────────

#[test]
fn test_happy_path_single_period_end_rate() {
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();

    let mut pair_rates = BTreeMap::new();
    pair_rates.insert(end, dec!(0.89));
    let mut rates = BTreeMap::new();
    rates.insert("USD/CHF".to_string(), pair_rates);

    let cfg = inline_cfg("CHF", rates);
    let entities = vec![make_entity("US01", "USD")];

    let master = build_fx_master(&cfg, "CHF", start, end, &entities).unwrap();

    assert_eq!(master.base_currency, "CHF");
    assert_eq!(master.closing_by_pair["USD/CHF"], dec!(0.89));
    assert_eq!(master.average_by_pair["USD/CHF"], dec!(0.89));
}

// ── Test 2: multiple rates in period — arithmetic average ──────────────────

#[test]
fn test_multiple_rates_averaged() {
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2024, 3, 31).unwrap();

    let mut pair_rates = BTreeMap::new();
    pair_rates.insert(NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(), dec!(0.90));
    pair_rates.insert(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(), dec!(0.88));
    pair_rates.insert(NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(), dec!(0.92));

    let mut rates = BTreeMap::new();
    rates.insert("EUR/CHF".to_string(), pair_rates);

    let cfg = inline_cfg("CHF", rates);
    let entities = vec![make_entity("DE01", "EUR")];

    let master = build_fx_master(&cfg, "CHF", start, end, &entities).unwrap();

    // closing = 0.92 (latest at/before end)
    assert_eq!(master.closing_by_pair["EUR/CHF"], dec!(0.92));
    // average = (0.90 + 0.88 + 0.92) / 3 = 0.9000
    let expected_avg = (dec!(0.90) + dec!(0.88) + dec!(0.92)) / Decimal::from(3u32);
    assert_eq!(master.average_by_pair["EUR/CHF"], expected_avg);
}

// ── Test 3: inverted pair is auto-flipped to canonical direction ────────────

#[test]
fn test_inverted_pair_is_auto_flipped() {
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();

    // User provides CHF/USD (presentation/functional) instead of USD/CHF
    let mut pair_rates = BTreeMap::new();
    pair_rates.insert(end, dec!(0.8870));
    let mut rates = BTreeMap::new();
    rates.insert("CHF/USD".to_string(), pair_rates);

    let cfg = inline_cfg("CHF", rates);
    let entities = vec![make_entity("US01", "USD")];

    let master = build_fx_master(&cfg, "CHF", start, end, &entities).unwrap();

    // Should have canonical key USD/CHF (not CHF/USD)
    assert!(
        !master.rates.contains_key("CHF/USD"),
        "inverted key should not appear"
    );
    assert!(
        master.rates.contains_key("USD/CHF"),
        "canonical key should appear"
    );

    // Rate should be 1/0.8870 ≈ 1.1274...
    let inverted = dec!(1) / dec!(0.8870);
    // Allow a very small tolerance since Decimal division is exact but let's check directly
    let closing = master.closing_by_pair["USD/CHF"];
    assert_eq!(closing, inverted, "inverted rate should equal 1/0.8870");
}

// ── Test 4: missing required pair fails with descriptive error ─────────────

#[test]
fn test_missing_pair_fails() {
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();

    // Only USD/CHF provided, but entity needs EUR/CHF
    let mut pair_rates = BTreeMap::new();
    pair_rates.insert(end, dec!(0.89));
    let mut rates = BTreeMap::new();
    rates.insert("USD/CHF".to_string(), pair_rates);

    let cfg = inline_cfg("CHF", rates);
    let entities = vec![
        make_entity("US01", "USD"),
        make_entity("DE01", "EUR"), // needs EUR/CHF — not in rates
    ];

    let err = build_fx_master(&cfg, "CHF", start, end, &entities).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("EUR/CHF"),
        "error should mention the missing pair EUR/CHF, got: {msg}"
    );
}

// ── Test 5: base_currency mismatch fails ──────────────────────────────────

#[test]
fn test_base_currency_mismatch_fails() {
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();

    let cfg = FxConfig {
        base_currency: "USD".to_string(), // mismatch: presentation is CHF
        rate_source: FxRateSource::Inline,
        rates: BTreeMap::new(),
        policy: default_policy(),
    };

    let entities = vec![make_entity("US01", "USD")];
    let err = build_fx_master(&cfg, "CHF", start, end, &entities).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("USD") && msg.contains("CHF"),
        "error should mention both currency codes, got: {msg}"
    );
}

// ── Test 6: historical_series source is unsupported in v5.0 ───────────────

#[test]
fn test_historical_series_unsupported() {
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();

    let cfg = FxConfig {
        base_currency: "CHF".to_string(),
        rate_source: FxRateSource::HistoricalSeries,
        rates: BTreeMap::new(),
        policy: default_policy(),
    };

    let entities = vec![make_entity("US01", "USD")];
    let err = build_fx_master(&cfg, "CHF", start, end, &entities).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("historical_series"),
        "error should mention 'historical_series', got: {msg}"
    );
    assert!(
        msg.contains("v5.0"),
        "error should mention 'v5.0', got: {msg}"
    );
}

// ── Test 7: single-currency group needs no FX ─────────────────────────────

#[test]
fn test_no_fx_needed_for_single_currency_group() {
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();

    // All entities have same currency as presentation (CHF)
    let cfg = inline_cfg("CHF", BTreeMap::new());
    let entities = vec![
        make_entity("CH01", "CHF"),
        make_entity("CH02", "CHF"),
        make_entity("CH03", "CHF"),
    ];

    let master = build_fx_master(&cfg, "CHF", start, end, &entities).unwrap();

    assert!(
        master.rates.is_empty(),
        "rates should be empty for single-currency group"
    );
    assert!(
        master.closing_by_pair.is_empty(),
        "closing_by_pair should be empty"
    );
    assert!(
        master.average_by_pair.is_empty(),
        "average_by_pair should be empty"
    );
}

// ── Test 8: user_supplied source behaves identically to inline ────────────

#[test]
fn test_user_supplied_source_works_like_inline() {
    let start = NaiveDate::from_ymd_opt(2024, 6, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2024, 6, 30).unwrap();

    let mut pair_rates = BTreeMap::new();
    pair_rates.insert(end, dec!(1.08));
    let mut rates = BTreeMap::new();
    rates.insert("GBP/CHF".to_string(), pair_rates);

    let cfg = FxConfig {
        base_currency: "CHF".to_string(),
        rate_source: FxRateSource::UserSupplied,
        rates,
        policy: default_policy(),
    };
    let entities = vec![make_entity("GB01", "GBP")];

    let master = build_fx_master(&cfg, "CHF", start, end, &entities).unwrap();
    assert_eq!(master.closing_by_pair["GBP/CHF"], dec!(1.08));
}

// ── Test 9: closing rate uses latest rate at-or-before period_end ─────────

#[test]
fn test_closing_rate_uses_latest_before_period_end() {
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2024, 3, 31).unwrap();

    // Rates available: Jan 31 and Feb 29 — no rate on exact period_end (Mar 31)
    let mut pair_rates = BTreeMap::new();
    pair_rates.insert(NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(), dec!(1.10));
    pair_rates.insert(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(), dec!(1.12));

    let mut rates = BTreeMap::new();
    rates.insert("BRL/CHF".to_string(), pair_rates);

    let cfg = inline_cfg("CHF", rates);
    let entities = vec![make_entity("BR01", "BRL")];

    let master = build_fx_master(&cfg, "CHF", start, end, &entities).unwrap();

    // closing = latest at or before end = Feb 29 rate = 1.12
    assert_eq!(master.closing_by_pair["BRL/CHF"], dec!(1.12));
    // average = (1.10 + 1.12) / 2 = 1.11
    let expected_avg = (dec!(1.10) + dec!(1.12)) / Decimal::from(2u32);
    assert_eq!(master.average_by_pair["BRL/CHF"], expected_avg);
}

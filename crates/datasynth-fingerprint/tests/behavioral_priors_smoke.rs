//! Integration smoke for the SP2 behavioral-prior pipeline.

use chrono::{Duration, NaiveDate};
use datasynth_eval::behavioral_fidelity::Record;
use datasynth_fingerprint::aggregation::industry_aggregator::aggregate_industry_priors;
use datasynth_fingerprint::extraction::behavioral_extractor::extract_behavioral_priors;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

pub fn gen_records(seed: u64, n: usize) -> Vec<Record> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let sources = ["KR", "RE", "SA", "DZ", "WE", "IM"];
    let accounts: Vec<String> = (1000..1050).map(|i| format!("A{i}")).collect();
    let ccs: Vec<String> = (100..120).map(|i| format!("CC{i}")).collect();
    let tps: Vec<String> = (1..30).map(|i| format!("TP{i}")).collect();
    let base = NaiveDate::from_ymd_opt(2022, 1, 1).expect("date");

    (0..n)
        .map(|i| Record {
            source: sources[rng.random_range(0..sources.len())].to_string(),
            gl_account: accounts[rng.random_range(0..accounts.len())].clone(),
            cost_center: Some(ccs[rng.random_range(0..ccs.len())].clone()),
            profit_center: Some(ccs[rng.random_range(0..ccs.len())].clone()),
            trading_partner: Some(tps[rng.random_range(0..tps.len())].clone()),
            je_number: format!("J{}-{:06}", seed, i / 3),
            je_line_number: format!("{:03}", (i % 3) + 1),
            effective_date: base + Duration::days(rng.random_range(0..365)),
            entry_date: base + Duration::days(rng.random_range(0..365)),
            created_at: None,
            functional_amount: rng.random_range(-10000.0..10000.0),
            header_text: String::new(),
            line_text: String::new(),
        })
        .collect()
}

#[test]
fn extract_aggregate_inspect_roundtrip() {
    // SP3.8b set `DEFAULT_MIN_SOURCE_OBSERVATIONS = 1000`: each Source code
    // appearing fewer than that many times in a single client's data is
    // dropped from `source_mix`. `gen_records` spreads draws across 6 codes,
    // so each client needs ~6000+ records to keep every code above the
    // threshold. 9000 gives ~1500 per code (≈ ±100 at 2σ) — comfortably
    // above 1000 across reasonable seed variance. The prior fix (9d4caa5)
    // bumped the *unit* test past this threshold but missed the *integration*
    // test here; CI surfaced it again on the SP6 branch.
    let a = extract_behavioral_priors(&gen_records(42, 9000), "test_industry").expect("extract a");
    let b = extract_behavioral_priors(&gen_records(43, 9000), "test_industry").expect("extract b");
    let c = extract_behavioral_priors(&gen_records(44, 9000), "test_industry").expect("extract c");

    let agg = aggregate_industry_priors(&[&a, &b, &c], "test_industry").expect("aggregate");
    assert_eq!(agg.n_client_inputs, 3);
    assert_eq!(agg.n_rows_aggregated, 27_000);
    assert!(!agg.source_mix.probabilities.is_empty());
    assert!(!agg.per_source_iet.by_source.is_empty());
    assert!(agg.lines_per_je.overall.n > 0);
    assert_eq!(agg.fanout.by_attribute.len(), 4);
    assert!(agg.posting_lag.is_some());

    // JSON round-trip.
    let json = serde_json::to_string(&agg).expect("serialize");
    let back: datasynth_fingerprint::models::BehavioralPriors =
        serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.n_client_inputs, 3);
    assert_eq!(back.industry, "test_industry");
}

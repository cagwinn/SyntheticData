// Integration smoke test for behavioral_fidelity.

use chrono::{Duration, NaiveDate};
use datasynth_eval::behavioral_fidelity::{self, BehavioralFidelityConfig, Record};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

pub fn gen_synthetic(seed: u64, n_entries: usize) -> Vec<Record> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let sources = ["KR", "RE", "SA", "DZ", "WE", "IM"];
    let accounts: Vec<String> = (1000..1050).map(|i| format!("A{i}")).collect();
    let ccs: Vec<String> = (100..120).map(|i| format!("CC{i}")).collect();
    let tps: Vec<String> = (1..30).map(|i| format!("TP{i}")).collect();

    let mut out = Vec::with_capacity(n_entries);
    let base = NaiveDate::from_ymd_opt(2022, 1, 1).expect("valid base date");
    for i in 0..n_entries {
        let day_off = rng.random_range(0..365);
        let entry = base + Duration::days(day_off);
        let effective = entry + Duration::days(rng.random_range(-2..14));
        let src = sources[rng.random_range(0..sources.len())];
        let je = format!("J{}-{:06}", seed, i / 3);
        let line = format!("{:03}", (i % 3) + 1);
        out.push(Record {
            source: src.to_string(),
            gl_account: accounts[rng.random_range(0..accounts.len())].clone(),
            cost_center: Some(ccs[rng.random_range(0..ccs.len())].clone()),
            profit_center: Some(ccs[rng.random_range(0..ccs.len())].clone()),
            trading_partner: Some(tps[rng.random_range(0..tps.len())].clone()),
            je_number: je,
            je_line_number: line,
            effective_date: effective,
            entry_date: entry,
            created_at: None,
            functional_amount: rng.random_range(-10000.0..10000.0),
            header_text: String::new(),
            line_text: String::new(),
        });
    }
    out
}

#[test]
fn smoke_report_runs_and_passes_gate_on_similar_data() {
    let real = gen_synthetic(42, 3000);
    let syn = gen_synthetic(43, 3000);
    let mut cfg = BehavioralFidelityConfig::gl_default();
    // Very permissive thresholds: the smoke test only verifies the full pipeline
    // executes end-to-end without errors.  DR across two different seeds (42 vs 43)
    // can spike on burst metrics; we use 50.0 as the ceiling.
    cfg.fail_thresholds.fail_if_dr_above = 50.0;
    cfg.fail_thresholds.fail_if_composite_above = 50.0;

    let report = behavioral_fidelity::compute_report(&cfg, &real, &syn).expect("compute_report");
    assert!(report.per_entity.contains_key("Source"));
    assert!(report.per_entity.contains_key("TradingPartner"));
    assert!(report.composite_bf_score.is_finite());
    assert!(report.composite_bf_score >= 0.0);
    assert!(
        report.gates.passed,
        "smoke gate should pass with permissive thresholds; failures = {:?}",
        report.gates.failures
    );
}

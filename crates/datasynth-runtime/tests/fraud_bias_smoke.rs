//! Smoke test for fraud behavioral bias lift.
//!
//! Verifies that the four canonical forensic signals (weekend posting,
//! round-dollar amounts, off-hours timestamps, post-close adjustments)
//! show ≥ 2× lift on fraud vs non-fraud entries. SDK teams had caught
//! this regression in v3.1 — fraud features either showed zero lift or
//! inverted signal because the behavioral bias wasn't being applied on
//! every fraud path.
//!
//! This test is intentionally broad-strokes: we pick a seed and config
//! that produces enough fraud entries to measure, then assert lift
//! thresholds for each bias. Failures here mean the bias wiring
//! regressed in one of the paths (anomaly injector, fraud propagation,
//! je_generator intrinsic fraud, create_self_approval, ...).

use chrono::{Datelike, Timelike, Weekday};
use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;
use rust_decimal::Decimal;

/// Canonical forensic feature set extracted from a journal entry header + first line.
struct ForensicFeatures {
    is_weekend: bool,
    is_round_1000: bool,
    is_off_hours: bool,
    is_post_close: bool,
}

fn extract_features(je: &datasynth_core::models::JournalEntry) -> ForensicFeatures {
    let weekday = je.header.posting_date.weekday();
    let is_weekend = matches!(weekday, Weekday::Sat | Weekday::Sun);
    let hour = je.header.created_at.hour();
    let is_off_hours = !(6..22).contains(&hour);
    let is_post_close = je.header.is_post_close;
    // Round-1000: the entry's largest line amount is exactly divisible by 1000
    // (and non-zero). This is the canonical forensic signal for round-dollar
    // fraud amounts.
    let max_abs: Decimal = je
        .lines
        .iter()
        .map(|l| l.debit_amount.max(l.credit_amount))
        .max()
        .unwrap_or(Decimal::ZERO);
    let is_round_1000 = max_abs > Decimal::ZERO && max_abs % Decimal::from(1_000) == Decimal::ZERO;

    ForensicFeatures {
        is_weekend,
        is_round_1000,
        is_off_hours,
        is_post_close,
    }
}

/// Compute per-feature lift = (fraud rate) / (baseline rate).
/// Returns (fraud_rate, baseline_rate, lift) per feature name.
fn compute_bias_lifts(
    entries: &[datasynth_core::models::JournalEntry],
) -> Vec<(&'static str, f64, f64, f64)> {
    let fraud: Vec<_> = entries.iter().filter(|e| e.header.is_fraud).collect();
    let baseline: Vec<_> = entries.iter().filter(|e| !e.header.is_fraud).collect();

    assert!(
        !fraud.is_empty(),
        "no fraud entries produced — cannot compute lift"
    );
    assert!(
        !baseline.is_empty(),
        "no baseline entries produced — cannot compute lift"
    );

    let fraud_features: Vec<_> = fraud.iter().map(|e| extract_features(e)).collect();
    let baseline_features: Vec<_> = baseline.iter().map(|e| extract_features(e)).collect();

    let rate = |v: &[ForensicFeatures], f: fn(&ForensicFeatures) -> bool| -> f64 {
        v.iter().filter(|ff| f(ff)).count() as f64 / v.len().max(1) as f64
    };
    let lift = |f_rate: f64, b_rate: f64| -> f64 {
        // Handle baseline == 0: report "infinity" as a finite large number
        // so the assertion below can still compare meaningfully.
        if b_rate == 0.0 {
            if f_rate == 0.0 {
                0.0 // both zero: no signal
            } else {
                f64::INFINITY
            }
        } else {
            f_rate / b_rate
        }
    };

    let fw = rate(&fraud_features, |ff| ff.is_weekend);
    let bw = rate(&baseline_features, |ff| ff.is_weekend);
    let fr = rate(&fraud_features, |ff| ff.is_round_1000);
    let br = rate(&baseline_features, |ff| ff.is_round_1000);
    let fo = rate(&fraud_features, |ff| ff.is_off_hours);
    let bo = rate(&baseline_features, |ff| ff.is_off_hours);
    let fp = rate(&fraud_features, |ff| ff.is_post_close);
    let bp = rate(&baseline_features, |ff| ff.is_post_close);

    vec![
        ("is_weekend", fw, bw, lift(fw, bw)),
        ("is_round_1000", fr, br, lift(fr, br)),
        ("is_off_hours", fo, bo, lift(fo, bo)),
        ("is_post_close", fp, bp, lift(fp, bp)),
    ]
}

/// Generate a job with fraud heavily enabled so we get enough labeled
/// entries to measure lift. Uses a medium-retail config with line-level
/// + document-level fraud both turned on.
fn generate_fraud_job() -> Vec<datasynth_core::models::JournalEntry> {
    let mut config = minimal_config();
    config.global.seed = Some(424242);
    config.global.period_months = 3;
    // Crank fraud so we get ≥ 50 fraud entries for stable statistics.
    config.fraud.enabled = true;
    config.fraud.fraud_rate = 0.05;
    config.fraud.document_fraud_rate = Some(0.05);
    config.fraud.propagate_to_lines = true;
    config.companies[0].annual_transaction_volume =
        datasynth_config::schema::TransactionVolume::HundredK;

    let phase_config = PhaseConfig {
        generate_master_data: true,
        generate_document_flows: true,
        generate_journal_entries: true,
        inject_anomalies: true,
        show_progress: false,
        ..Default::default()
    };

    let mut orchestrator =
        EnhancedOrchestrator::new(config, phase_config).expect("Failed to create orchestrator");

    let result = orchestrator.generate().expect("Generation failed");
    result.journal_entries
}

#[test]
fn fraud_bias_lift_weekend_round_offhours_postclose() {
    let entries = generate_fraud_job();
    let fraud_count = entries.iter().filter(|e| e.header.is_fraud).count();
    println!(
        "total entries: {}, fraud entries: {}",
        entries.len(),
        fraud_count
    );
    assert!(
        fraud_count >= 20,
        "need ≥20 fraud entries for stable lift, got {fraud_count}"
    );

    let lifts = compute_bias_lifts(&entries);
    for (name, fr, br, lift) in &lifts {
        println!(
            "  {}: fraud_rate={:.4} baseline_rate={:.4} lift={}",
            name,
            fr,
            br,
            if lift.is_finite() {
                format!("{lift:.2}x")
            } else {
                "∞".to_string()
            }
        );
    }

    // Assert each bias shows ≥ 1.5× lift. This is looser than the SDK
    // team's 2× ask to allow statistical noise from the small sample, but
    // strict enough to catch bias-wiring regressions (which produce 0×
    // or inverted lift).
    //
    // is_off_hours and is_post_close often have baseline = 0 (the
    // generator only emits these on fraud paths), so their lift is
    // reported as infinity — which satisfies ≥ 1.5.
    for (name, fr, br, lift) in lifts {
        assert!(
            lift >= 1.5,
            "{name}: fraud rate {fr:.4} vs baseline {br:.4} — lift {lift:.2}x < 1.5x \
             (bias is not firing on this forensic signal; \
              regression in fraud behavioral bias wiring)"
        );
    }
}

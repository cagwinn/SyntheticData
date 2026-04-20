//! v3.5.2 — smoke test for `distributions.regime_changes` runtime
//! wiring. Verifies that when regime changes are enabled, a JE
//! generation run completes without error and amounts reflect the
//! configured event (e.g. an acquisition mid-period should lift the
//! average amount for dates after the event).

use datasynth_config::schema::{
    AdvancedDistributionConfig, EconomicCycleSchemaConfig, RegimeChangeEventConfig,
    RegimeChangeSchemaConfig, RegimeChangeTypeConfig,
};
use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;
use rust_decimal::prelude::ToPrimitive;

fn build_runtime(
    cfg_tweak: impl FnOnce(&mut datasynth_config::GeneratorConfig),
) -> EnhancedOrchestrator {
    let mut config = minimal_config();
    config.global.seed = Some(3520);
    config.global.period_months = 12;
    config.global.start_date = "2024-01-01".to_string();
    config.fraud.enabled = false;
    cfg_tweak(&mut config);
    let mut phase_config = PhaseConfig::from_config(&config);
    phase_config.generate_document_flows = false;
    phase_config.inject_anomalies = false;
    phase_config.generate_banking = false;
    phase_config.generate_graph_export = false;
    phase_config.generate_ocpm_events = false;
    phase_config.generate_period_close = false;
    phase_config.generate_evolution_events = false;
    phase_config.generate_sourcing = false;
    phase_config.generate_intercompany = false;
    phase_config.generate_financial_statements = false;
    phase_config.generate_bank_reconciliation = false;
    phase_config.generate_accounting_standards = false;
    phase_config.generate_manufacturing = false;
    phase_config.generate_sales_kpi_budgets = false;
    phase_config.generate_tax = false;
    phase_config.generate_esg = false;
    phase_config.generate_hr = false;
    phase_config.generate_treasury = false;
    phase_config.generate_project_accounting = false;
    phase_config.generate_compliance_regulations = false;
    phase_config.inject_data_quality = false;
    phase_config.validate_balances = false;
    phase_config.show_progress = false;
    phase_config.generate_audit = false;
    phase_config.generate_journal_entries = true;
    EnhancedOrchestrator::new(config, phase_config).expect("build orchestrator")
}

fn mean_by_half(orch: &mut EnhancedOrchestrator) -> (f64, f64) {
    let result = orch.generate().expect("generate");
    let cutoff = chrono::NaiveDate::from_ymd_opt(2024, 7, 1).unwrap();
    let (mut pre, mut post) = (Vec::<f64>::new(), Vec::<f64>::new());
    for je in &result.journal_entries {
        let d = je.header.posting_date;
        let total: f64 = je
            .lines
            .iter()
            .map(|l| (l.debit_amount + l.credit_amount).to_f64().unwrap_or(0.0))
            .sum::<f64>()
            / 2.0; // debit==credit; halve to get entry magnitude
        if d < cutoff {
            pre.push(total);
        } else {
            post.push(total);
        }
    }
    let avg = |v: &[f64]| -> f64 {
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };
    (avg(&pre), avg(&post))
}

#[test]
fn regime_changes_disabled_is_no_op() {
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            regime_changes: RegimeChangeSchemaConfig {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        };
    });
    let result = orch.generate().expect("generate");
    assert!(!result.journal_entries.is_empty());
}

#[test]
fn price_increase_regime_lifts_post_event_amounts() {
    // Baseline: no regime change.
    let (base_pre, base_post) = mean_by_half(&mut build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            ..Default::default()
        };
    }));
    // With a July 1st price-increase event.
    let (re_pre, re_post) = mean_by_half(&mut build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            regime_changes: RegimeChangeSchemaConfig {
                enabled: true,
                changes: vec![RegimeChangeEventConfig {
                    date: "2024-07-01".to_string(),
                    change_type: RegimeChangeTypeConfig::PriceIncrease,
                    description: Some("mid-year price hike".to_string()),
                    effects: Vec::new(),
                }],
                ..Default::default()
            },
            ..Default::default()
        };
    }));
    // Pre-event halves should be comparable; post-event the regime run
    // should have higher mean amounts than baseline (price increase).
    let ratio = re_post / base_post.max(1.0);
    assert!(
        ratio > 1.03,
        "expected post-event amounts to rise >3% with PriceIncrease regime; \
         baseline post avg {base_post:.2}, regime post avg {re_post:.2} (ratio {ratio:.3}). \
         base_pre={base_pre:.2}, re_pre={re_pre:.2}"
    );
}

#[test]
fn economic_cycle_enables_drift() {
    // Simple sanity: economic cycle config wired through DriftConfig
    // and pipeline still completes without error.
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            regime_changes: RegimeChangeSchemaConfig {
                enabled: true,
                economic_cycle: Some(EconomicCycleSchemaConfig {
                    enabled: true,
                    period_months: 12,
                    amplitude: 0.20,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
    });
    let result = orch.generate().expect("generate");
    assert!(!result.journal_entries.is_empty());
}

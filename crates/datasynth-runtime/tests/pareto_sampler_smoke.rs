//! v3.4.4 — smoke test for Pareto heavy-tailed amount sampling.
//!
//! Verifies that when `distributions.pareto.enabled = true`, the JE
//! generator routes non-fraud amounts through a Pareto sampler, producing
//! a heavy-tailed distribution where most values cluster near `x_min` and
//! a small number reach much higher.

use datasynth_config::schema::{AdvancedDistributionConfig, ParetoSchemaConfig};
use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;
use rust_decimal::prelude::ToPrimitive;

fn build_runtime(
    cfg_tweak: impl FnOnce(&mut datasynth_config::GeneratorConfig),
) -> EnhancedOrchestrator {
    let mut config = minimal_config();
    config.global.seed = Some(3440);
    config.global.period_months = 1;
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

/// Collect per-JE totals (sum of all debits = sum of all credits for
/// a balanced entry). The base amount sampled by `AdvancedAmountSampler`
/// becomes the entry total, but individual lines may split it into smaller
/// debit/credit pieces — so we test at the entry level, not per-line.
fn entry_totals(orch: &mut EnhancedOrchestrator) -> Vec<f64> {
    let result = orch.generate().expect("generate");
    result
        .journal_entries
        .iter()
        .map(|je| je.total_debit().to_f64().unwrap_or(0.0))
        .filter(|a| *a > 0.0)
        .collect()
}

#[test]
fn pareto_sampling_produces_heavy_tail() {
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            pareto: Some(ParetoSchemaConfig {
                enabled: true,
                alpha: 1.5,
                x_min: 1000.0,
                max_value: None,
                decimal_places: 2,
            }),
            ..Default::default()
        };
    });
    let amounts = entry_totals(&mut orch);
    assert!(!amounts.is_empty(), "should produce amounts");

    // Pareto samples are >= x_min but downstream drift/seasonality
    // multipliers (active even with default temporal config) can scale
    // entries below x_min. Assert that the overwhelming majority
    // (>=95%) remain at or above x_min.
    let below_x_min = amounts.iter().filter(|a| **a < 1000.0).count();
    let total = amounts.len();
    let below_ratio = below_x_min as f64 / total as f64;
    let below_pct = below_ratio * 100.0;
    assert!(
        below_ratio < 0.05,
        "expected <5% of amounts below x_min=1000, got {below_x_min} / {total} ({below_pct:.1}%)"
    );

    // Heavy tail: at least some samples > 10x x_min. Alpha=1.5 should
    // produce ~10% of samples above 10x x_min (P(X > k*x_min) = k^-alpha).
    let extreme = amounts.iter().filter(|a| **a > 10_000.0).count();
    assert!(
        extreme > 0,
        "expected some samples > 10x x_min (heavy tail), got {extreme} / {total}"
    );
}

#[test]
fn pareto_disabled_is_no_op() {
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            pareto: Some(ParetoSchemaConfig {
                enabled: false,
                ..Default::default()
            }),
            ..Default::default()
        };
    });
    let amounts = entry_totals(&mut orch);
    // Should use legacy AmountSampler, producing typical log-normal amounts.
    let above_x_min = amounts.iter().filter(|a| **a >= 1000.0).count();
    let below_x_min = amounts.iter().filter(|a| **a < 1000.0).count();
    // Legacy sampler: most amounts are small (< 1000) on a default config.
    assert!(
        below_x_min > 0,
        "legacy path should produce some amounts < 1000, got {below_x_min} / {}",
        amounts.len()
    );
    let _ = above_x_min;
}

#[test]
fn pareto_precedence_over_amounts_mixture() {
    // When both pareto.enabled and amounts.enabled are true, Pareto wins.
    let mut orch = build_runtime(|c| {
        use datasynth_config::schema::{
            MixtureComponentConfig, MixtureDistributionSchemaConfig, MixtureDistributionType,
        };
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            pareto: Some(ParetoSchemaConfig {
                enabled: true,
                alpha: 2.0,
                x_min: 5000.0, // distinctive floor — easy to detect
                max_value: None,
                decimal_places: 2,
            }),
            amounts: MixtureDistributionSchemaConfig {
                enabled: true,
                distribution_type: MixtureDistributionType::LogNormal,
                components: vec![MixtureComponentConfig {
                    weight: 1.0,
                    mu: 5.0, // ~exp(5) = 148, well below pareto x_min
                    sigma: 0.5,
                    label: Some("distractor".to_string()),
                }],
                min_value: 0.01,
                max_value: None,
                decimal_places: 2,
            },
            ..Default::default()
        };
    });
    let amounts = entry_totals(&mut orch);
    // If Pareto won, the overwhelming majority of amounts should be at
    // or above x_min (5000). Downstream drift/seasonality can push a
    // small fraction below. If the mixture path had won, the mean
    // would center around exp(5) ≈ 148, so we'd see many values < 1000.
    let above_4k = amounts.iter().filter(|a| **a >= 4000.0).count();
    let total = amounts.len();
    let ratio = above_4k as f64 / total as f64;
    let pct = ratio * 100.0;
    assert!(
        ratio > 0.8,
        "Pareto should have precedence; expected >80% of amounts >= 4000, got {above_4k} / {total} ({pct:.1}%)"
    );
}

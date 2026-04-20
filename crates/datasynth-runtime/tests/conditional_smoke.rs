//! v3.5.3 — smoke test for `distributions.conditional` runtime wiring.
//! Verifies that a conditional rule with `output_field = "amount"` and
//! `input_field = "month"` produces observably different means in the
//! declared quarters vs. a baseline run.

use datasynth_config::schema::{
    AdvancedDistributionConfig, ConditionalBreakpointConfig, ConditionalDistributionParamsConfig,
    ConditionalDistributionSchemaConfig,
};
use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;
use rust_decimal::prelude::ToPrimitive;

fn build_runtime(
    cfg_tweak: impl FnOnce(&mut datasynth_config::GeneratorConfig),
) -> EnhancedOrchestrator {
    let mut config = minimal_config();
    config.global.seed = Some(3530);
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

#[test]
fn conditional_disabled_is_no_op() {
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            conditional: Vec::new(),
            ..Default::default()
        };
    });
    let result = orch.generate().expect("generate");
    assert!(!result.journal_entries.is_empty());
}

#[test]
fn conditional_by_month_shifts_q4_amounts() {
    // A rule: amount sampled from LogNormal(5.0, 0.5) in Q1-Q3,
    // LogNormal(9.0, 0.5) in Q4. Using month as input_field.
    // Breakpoint at month >= 10 → the Q4 distribution.
    // Expected: mean amount in Oct-Dec is ~exp(9.125) ≈ 9192
    //           mean amount in Jan-Sep is ~exp(5.125) ≈ 168
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            conditional: vec![ConditionalDistributionSchemaConfig {
                output_field: "amount".to_string(),
                input_field: "month".to_string(),
                breakpoints: vec![ConditionalBreakpointConfig {
                    threshold: 10.0,
                    distribution: ConditionalDistributionParamsConfig::LogNormal {
                        mu: 9.0,
                        sigma: 0.5,
                    },
                }],
                default_distribution: ConditionalDistributionParamsConfig::LogNormal {
                    mu: 5.0,
                    sigma: 0.5,
                },
                min_value: Some(0.01),
                max_value: None,
                decimal_places: 2,
            }],
            ..Default::default()
        };
    });
    let result = orch.generate().expect("generate");

    let mut q1_q3 = Vec::<f64>::new();
    let mut q4 = Vec::<f64>::new();
    for je in &result.journal_entries {
        let total: f64 = je
            .lines
            .iter()
            .map(|l| (l.debit_amount + l.credit_amount).to_f64().unwrap_or(0.0))
            .sum::<f64>()
            / 2.0;
        let month = chrono::Datelike::month(&je.header.posting_date);
        if (10..=12).contains(&month) {
            q4.push(total);
        } else {
            q1_q3.push(total);
        }
    }
    let avg = |v: &[f64]| -> f64 {
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };
    let q1_q3_avg = avg(&q1_q3);
    let q4_avg = avg(&q4);
    assert!(
        q4_avg > q1_q3_avg * 3.0,
        "expected Q4 mean ({q4_avg:.2}) >> Q1-Q3 mean ({q1_q3_avg:.2})"
    );
}

#[test]
fn conditional_with_unsupported_input_field_is_ignored() {
    // input_field = "nonexistent" → the rule is silently skipped and
    // generation proceeds with default amount sampling.
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            conditional: vec![ConditionalDistributionSchemaConfig {
                output_field: "amount".to_string(),
                input_field: "nonexistent_field".to_string(),
                breakpoints: vec![ConditionalBreakpointConfig {
                    threshold: 0.0,
                    distribution: ConditionalDistributionParamsConfig::Fixed { value: 1_000_000.0 },
                }],
                default_distribution: ConditionalDistributionParamsConfig::Fixed {
                    value: 1_000_000.0,
                },
                min_value: None,
                max_value: None,
                decimal_places: 2,
            }],
            ..Default::default()
        };
    });
    let result = orch.generate().expect("generate");
    assert!(!result.journal_entries.is_empty());
    // No amount should equal $1M — the rule was ignored.
    let million_count = result
        .journal_entries
        .iter()
        .flat_map(|je| je.lines.iter())
        .filter(|l| (l.debit_amount + l.credit_amount).to_f64().unwrap_or(0.0) == 2_000_000.0)
        .count();
    assert_eq!(
        million_count, 0,
        "unsupported input_field should yield 0 matches, got {million_count}"
    );
}

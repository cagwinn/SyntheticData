//! v3.5.1 — smoke test for `distributions.validation` runtime wiring.
//!
//! Verifies that when `distributions.validation.enabled = true`, the
//! generator emits a `StatisticalValidationReport` in the result and
//! that the configured tests actually execute over the generated
//! amounts.

use datasynth_config::schema::{
    AdvancedDistributionConfig, StatisticalTestConfig, StatisticalValidationSchemaConfig,
};
use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;

fn build_runtime(
    cfg_tweak: impl FnOnce(&mut datasynth_config::GeneratorConfig),
) -> EnhancedOrchestrator {
    let mut config = minimal_config();
    config.global.seed = Some(3510);
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

#[test]
fn validation_disabled_returns_none() {
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            validation: StatisticalValidationSchemaConfig {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        };
    });
    let result = orch.generate().expect("generate");
    assert!(
        result.statistical_validation.is_none(),
        "disabled validation should yield None, got Some"
    );
}

#[test]
fn validation_enabled_runs_benford() {
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            validation: StatisticalValidationSchemaConfig {
                enabled: true,
                tests: vec![StatisticalTestConfig::BenfordFirstDigit {
                    threshold_mad: 0.030, // loose, we just want to observe a result
                    warning_mad: 0.015,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
    });
    let result = orch.generate().expect("generate");
    let report = result
        .statistical_validation
        .as_ref()
        .expect("enabled validation should yield Some");
    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].name, "benford_first_digit");
    assert!(
        report.sample_count > 100,
        "expected ≥100 samples, got {}",
        report.sample_count
    );
}

#[test]
fn validation_runs_multiple_tests() {
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            validation: StatisticalValidationSchemaConfig {
                enabled: true,
                tests: vec![
                    StatisticalTestConfig::BenfordFirstDigit {
                        threshold_mad: 0.030,
                        warning_mad: 0.015,
                    },
                    StatisticalTestConfig::ChiSquared {
                        bins: 10,
                        significance: 0.05,
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
    });
    let result = orch.generate().expect("generate");
    let report = result
        .statistical_validation
        .as_ref()
        .expect("enabled validation should yield Some");
    assert_eq!(report.results.len(), 2);
    let names: Vec<&str> = report.results.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"benford_first_digit"));
    assert!(names.contains(&"chi_squared"));
}

#[test]
fn validation_skips_correlation_and_anderson_darling() {
    use datasynth_config::schema::{DistributionFitMethod, TargetDistributionConfig};

    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            validation: StatisticalValidationSchemaConfig {
                enabled: true,
                tests: vec![
                    StatisticalTestConfig::AndersonDarling {
                        target: TargetDistributionConfig::LogNormal,
                        significance: 0.05,
                    },
                    StatisticalTestConfig::DistributionFit {
                        target: TargetDistributionConfig::LogNormal,
                        ks_significance: 0.05,
                        method: DistributionFitMethod::default(),
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
    });
    let result = orch.generate().expect("generate");
    let report = result
        .statistical_validation
        .as_ref()
        .expect("enabled validation should yield Some");
    // Anderson-Darling is not yet implemented → Skipped.
    let ad = report
        .results
        .iter()
        .find(|r| r.name == "anderson_darling")
        .expect("anderson_darling result present");
    assert!(matches!(
        ad.outcome,
        datasynth_core::distributions::TestOutcome::Skipped
    ));
    // DistributionFit maps to ks_uniform_log in v3.5.1.
    let ks = report
        .results
        .iter()
        .find(|r| r.name == "ks_uniform_log")
        .expect("ks_uniform_log result present");
    // Outcome may be pass/fail/warn depending on the data — we just want it
    // non-skipped to confirm it actually ran.
    assert!(!matches!(
        ks.outcome,
        datasynth_core::distributions::TestOutcome::Skipped
    ));
}

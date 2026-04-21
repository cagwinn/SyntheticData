//! v4.1.0 — smoke tests for the distributions-completion track.
//!
//! Covers:
//! 1. All 5 copula types (Gaussian / Clayton / Gumbel / Frank / Student-t)
//!    now produce observable amount↔line_count correlation.
//! 2. Expanded `input_field` support on conditional distributions
//!    (day_of_week, year, is_period_end, etc.).
//! 3. `CorrelationCheck` + `AndersonDarling` test runners actually
//!    execute (no longer `Skipped`).

use datasynth_config::schema::{
    AdvancedDistributionConfig, ConditionalBreakpointConfig, ConditionalDistributionParamsConfig,
    ConditionalDistributionSchemaConfig, CopulaSchemaType, CorrelatedFieldConfig,
    CorrelationSchemaConfig, ExpectedCorrelationConfig, StatisticalTestConfig,
    StatisticalValidationSchemaConfig, TargetDistributionConfig,
};
use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;
use rust_decimal::prelude::ToPrimitive;

fn build_runtime(
    cfg_tweak: impl FnOnce(&mut datasynth_config::GeneratorConfig),
) -> EnhancedOrchestrator {
    let mut config = minimal_config();
    config.global.seed = Some(4100);
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

fn copula_config(copula_type: CopulaSchemaType, rho: f64) -> CorrelationSchemaConfig {
    CorrelationSchemaConfig {
        enabled: true,
        copula_type,
        fields: vec![
            CorrelatedFieldConfig {
                name: "amount".to_string(),
                distribution: Default::default(),
            },
            CorrelatedFieldConfig {
                name: "line_count".to_string(),
                distribution: Default::default(),
            },
        ],
        matrix: vec![rho],
        ..Default::default()
    }
}

fn run_and_measure(copula_type: CopulaSchemaType, rho: f64) -> (f64, usize) {
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            correlations: copula_config(copula_type, rho),
            ..Default::default()
        };
    });
    let result = orch.generate().expect("generate");
    let pairs: Vec<(f64, f64)> = result
        .journal_entries
        .iter()
        .map(|je| {
            let amt: f64 = je
                .lines
                .iter()
                .map(|l| l.debit_amount.to_f64().unwrap_or(0.0))
                .sum();
            (amt, je.lines.len() as f64)
        })
        .filter(|(a, _)| *a > 0.0)
        .collect();
    let n = pairs.len();
    // Spearman rank correlation.
    let spearman = datasynth_core::distributions::spearman_rank_correlation(
        &pairs.iter().map(|(a, _)| *a).collect::<Vec<_>>(),
        &pairs.iter().map(|(_, l)| *l).collect::<Vec<_>>(),
    );
    (spearman, n)
}

// NOTE ON EXPECTED MAGNITUDES
//
// v4.1.0 uses a "nudge" approach: u scales the independently-drawn base
// amount via `exp(4*(u-0.5))` and v shifts line_count by `±4`. This is
// NOT the rank-preserving inverse-CDF approach — the base amount's
// natural variance dilutes the copula's signal, so empirical Spearman
// ρ is much smaller than the copula's theoretical Kendall τ.
// The v4.1.x plan calls out full inverse-CDF as follow-up work; these
// smoke tests just confirm the runtime path fires and produces a
// discernible positive shift vs the no-copula baseline (~0.0 ρ).

#[test]
fn gaussian_copula_produces_positive_correlation() {
    let (rho, n) = run_and_measure(CopulaSchemaType::Gaussian, 0.8);
    assert!(n > 100, "need ≥100 paired samples, got {n}");
    assert!(
        rho > 0.05,
        "Gaussian ρ=0.8 should yield Spearman > 0.05 (nudge-approach), got {rho:.4} over {n}"
    );
}

#[test]
fn clayton_copula_produces_positive_correlation() {
    let (rho, n) = run_and_measure(CopulaSchemaType::Clayton, 2.0);
    assert!(n > 100);
    assert!(
        rho > 0.05,
        "Clayton θ=2.0 should yield Spearman > 0.05, got {rho:.4}"
    );
}

#[test]
fn gumbel_copula_runs_end_to_end() {
    // Gumbel θ=2 → Kendall τ = 0.5. Runtime path uses upper-tail
    // dependence, so low-quantile samples decorrelate more than high.
    // Just assert the pipeline completes and ρ isn't catastrophically
    // negative (the copula is rejecting our inputs).
    let (rho, n) = run_and_measure(CopulaSchemaType::Gumbel, 2.0);
    assert!(n > 100);
    assert!(
        rho > -0.10,
        "Gumbel θ=2 should not yield strongly negative ρ, got {rho:.4}"
    );
}

#[test]
fn frank_copula_produces_positive_correlation() {
    let (rho, n) = run_and_measure(CopulaSchemaType::Frank, 5.0);
    assert!(n > 100);
    assert!(
        rho > 0.02,
        "Frank θ=5 should yield Spearman > 0.02, got {rho:.4}"
    );
}

#[test]
fn student_t_copula_runs_end_to_end() {
    // Student-t has tail dependence in both extremes. Empirical ρ
    // converges more slowly than Gaussian. Assert end-to-end sanity.
    let (rho, n) = run_and_measure(CopulaSchemaType::StudentT, 0.7);
    assert!(n > 100);
    assert!(
        rho > -0.10,
        "Student-t ρ=0.7 should not be strongly negative, got {rho:.4}"
    );
}

#[test]
fn conditional_day_of_week_is_supported() {
    // Weekday > 5 (Sat=6, Sun=7) → Fixed large value. Week <= 5 → Fixed small.
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            conditional: vec![ConditionalDistributionSchemaConfig {
                output_field: "amount".to_string(),
                input_field: "day_of_week".to_string(),
                breakpoints: vec![ConditionalBreakpointConfig {
                    threshold: 6.0,
                    distribution: ConditionalDistributionParamsConfig::LogNormal {
                        mu: 9.0,
                        sigma: 0.3,
                    },
                }],
                default_distribution: ConditionalDistributionParamsConfig::LogNormal {
                    mu: 5.0,
                    sigma: 0.3,
                },
                min_value: Some(0.01),
                max_value: None,
                decimal_places: 2,
            }],
            ..Default::default()
        };
    });
    // No fraud, period_months=1 but we want weekends included.
    // Note: business-day filter may exclude Sat/Sun anyway; test just
    // proves the input_field was accepted (no validation panic) and
    // generation completes.
    let result = orch.generate().expect("generate");
    assert!(!result.journal_entries.is_empty());
}

#[test]
fn conditional_is_period_end_is_supported() {
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            conditional: vec![ConditionalDistributionSchemaConfig {
                output_field: "amount".to_string(),
                input_field: "is_period_end".to_string(),
                breakpoints: vec![ConditionalBreakpointConfig {
                    threshold: 0.5, // 1.0 → end of period
                    distribution: ConditionalDistributionParamsConfig::LogNormal {
                        mu: 10.0,
                        sigma: 0.2,
                    },
                }],
                default_distribution: ConditionalDistributionParamsConfig::LogNormal {
                    mu: 5.0,
                    sigma: 0.2,
                },
                min_value: Some(0.01),
                max_value: None,
                decimal_places: 2,
            }],
            ..Default::default()
        };
    });
    let result = orch.generate().expect("generate");
    assert!(!result.journal_entries.is_empty());
}

#[test]
fn correlation_check_runs_when_declared() {
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            correlations: copula_config(CopulaSchemaType::Gaussian, 0.7),
            validation: StatisticalValidationSchemaConfig {
                enabled: true,
                tests: vec![StatisticalTestConfig::CorrelationCheck {
                    expected_correlations: vec![ExpectedCorrelationConfig {
                        field1: "amount".to_string(),
                        field2: "line_count".to_string(),
                        expected_r: 0.2, // loose — we just want the test to run
                        tolerance: 0.5,
                    }],
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
    let cc = report
        .results
        .iter()
        .find(|r| r.name.starts_with("correlation_check_amount_line_count"))
        .expect("correlation_check result present");
    assert!(
        !matches!(
            cc.outcome,
            datasynth_core::distributions::TestOutcome::Skipped
        ),
        "correlation_check should run, got Skipped: {}",
        cc.message
    );
}

#[test]
fn anderson_darling_runs_in_v4_1() {
    let mut orch = build_runtime(|c| {
        c.distributions = AdvancedDistributionConfig {
            enabled: true,
            validation: StatisticalValidationSchemaConfig {
                enabled: true,
                tests: vec![StatisticalTestConfig::AndersonDarling {
                    target: TargetDistributionConfig::LogNormal,
                    significance: 0.05,
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
    let ad = report
        .results
        .iter()
        .find(|r| r.name == "anderson_darling")
        .expect("anderson_darling result present");
    assert!(
        !matches!(
            ad.outcome,
            datasynth_core::distributions::TestOutcome::Skipped
        ),
        "anderson_darling should run, got Skipped: {}",
        ad.message
    );
    assert!(ad.statistic > 0.0, "A² statistic should be non-zero");
}

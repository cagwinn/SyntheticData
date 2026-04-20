//! v3.3.1 — smoke tests for the 3 new accounting-standards generators
//! (Lease / FairValue / FrameworkReconciliation) wired into
//! `phase_accounting_standards`.

use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;

fn build_runtime(
    cfg_tweak: impl FnOnce(&mut datasynth_config::GeneratorConfig),
) -> EnhancedOrchestrator {
    let mut config = minimal_config();
    config.global.seed = Some(3310);
    config.global.period_months = 1;
    config.accounting_standards.enabled = true;
    cfg_tweak(&mut config);
    let mut phase_config = PhaseConfig::from_config(&config);
    // Narrow phase set to keep test memory bounded.
    phase_config.generate_document_flows = false;
    phase_config.generate_journal_entries = false;
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
    // Keep the standards phase on.
    phase_config.generate_accounting_standards = true;
    EnhancedOrchestrator::new(config, phase_config).expect("build orchestrator")
}

#[test]
fn leases_emitted_when_flag_enabled() {
    let mut orch = build_runtime(|c| {
        c.accounting_standards.leases.enabled = true;
        c.accounting_standards.leases.lease_count = 10;
    });
    let result = orch.generate().expect("generate");
    assert!(
        !result.accounting_standards.leases.is_empty(),
        "leases should be populated"
    );
    assert_eq!(
        result.accounting_standards.lease_count,
        result.accounting_standards.leases.len()
    );
}

#[test]
fn fair_value_measurements_emitted_when_flag_enabled() {
    let mut orch = build_runtime(|c| {
        c.accounting_standards.fair_value.enabled = true;
        c.accounting_standards.fair_value.measurement_count = 15;
        c.accounting_standards
            .fair_value
            .include_sensitivity_analysis = true;
    });
    let result = orch.generate().expect("generate");
    assert!(
        !result
            .accounting_standards
            .fair_value_measurements
            .is_empty(),
        "FV measurements should be populated"
    );
}

#[test]
fn framework_reconciliation_only_for_dual_reporting() {
    // Dual reporting + generate_differences → should produce records.
    let mut orch = build_runtime(|c| {
        c.accounting_standards.framework =
            Some(datasynth_config::schema::AccountingFrameworkConfig::DualReporting);
        c.accounting_standards.generate_differences = true;
    });
    let result = orch.generate().expect("generate");
    assert!(
        !result.accounting_standards.framework_differences.is_empty(),
        "dual reporting + generate_differences should emit framework differences"
    );
    assert!(
        !result
            .accounting_standards
            .framework_reconciliations
            .is_empty(),
        "should emit at least one reconciliation per company"
    );
}

#[test]
fn framework_reconciliation_skipped_for_single_framework() {
    // US GAAP alone → no differences.
    let mut orch = build_runtime(|c| {
        c.accounting_standards.framework =
            Some(datasynth_config::schema::AccountingFrameworkConfig::UsGaap);
        c.accounting_standards.generate_differences = true;
    });
    let result = orch.generate().expect("generate");
    assert!(
        result.accounting_standards.framework_differences.is_empty(),
        "single-framework config must not emit framework differences"
    );
}

#[test]
fn all_flags_off_leaves_snapshot_fields_empty() {
    let mut orch = build_runtime(|_c| {});
    let result = orch.generate().expect("generate");
    assert!(result.accounting_standards.leases.is_empty());
    assert!(result
        .accounting_standards
        .fair_value_measurements
        .is_empty());
    assert!(result.accounting_standards.framework_differences.is_empty());
    assert!(result
        .accounting_standards
        .framework_reconciliations
        .is_empty());
}

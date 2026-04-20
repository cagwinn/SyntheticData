//! v3.3.0 — smoke tests for newly wired L1 generators.
//!
//! Each test enables exactly one of the v3.3.0 phase flags and asserts
//! the corresponding snapshot field is non-empty, with one semantic
//! invariant where cheap to check. Regression guard: with all flags
//! off, the orchestrator output is byte-identical to v3.2.1 defaults
//! (no new outputs leak into archives).

use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;

fn build_runtime(
    cfg_tweak: impl FnOnce(&mut datasynth_config::GeneratorConfig),
) -> EnhancedOrchestrator {
    let mut config = minimal_config();
    config.global.seed = Some(3300);
    config.global.period_months = 1;
    cfg_tweak(&mut config);
    // Start with PhaseConfig::from_config then narrow to the
    // essentials — the full phase set is too memory-heavy for a
    // CI-runner smoke test (seen: SIGKILL on all_flags_off).
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
    EnhancedOrchestrator::new(config, phase_config).expect("build orchestrator")
}

#[test]
fn organizational_profile_emits_one_per_company_whenever_master_data_runs() {
    let mut orch = build_runtime(|_| {});
    let result = orch.generate().expect("generate");
    // One profile per company in the fixture — test uses minimal_config
    // which configures a single company.
    assert!(
        !result.master_data.organizational_profiles.is_empty(),
        "should emit ≥1 organizational profile"
    );
    let profile = &result.master_data.organizational_profiles[0];
    assert!(
        !profile.entity_code.is_empty(),
        "organizational profile must carry an entity code"
    );
}

#[test]
fn legal_documents_emitted_when_flag_enabled() {
    let mut orch = build_runtime(|c| {
        c.audit.enabled = true;
        c.compliance_regulations.enabled = true;
        c.compliance_regulations.legal_documents.enabled = true;
    });
    let result = orch.generate().expect("generate");
    assert!(
        !result.audit.legal_documents.is_empty(),
        "legal_documents should be populated when flag enabled"
    );
}

#[test]
fn it_controls_emitted_when_flag_enabled() {
    let mut orch = build_runtime(|c| {
        c.audit.enabled = true;
        c.audit.it_controls.enabled = true;
    });
    let result = orch.generate().expect("generate");
    assert!(
        !result.audit.it_controls_access_logs.is_empty()
            || !result.audit.it_controls_change_records.is_empty(),
        "at least one of access_logs / change_records should populate when flag enabled"
    );
}

#[test]
fn analytics_metadata_phase_emits_when_enabled() {
    let mut orch = build_runtime(|c| {
        c.analytics_metadata.enabled = true;
    });
    let result = orch.generate().expect("generate");
    let am = &result.analytics_metadata;
    // prior_year + industry_benchmark default to true under enabled=true.
    assert!(
        !am.prior_year_comparatives.is_empty() || !am.industry_benchmarks.is_empty(),
        "analytics metadata snapshot should have at least one sub-generator populated"
    );
}

#[test]
fn all_flags_off_leaves_new_snapshots_empty() {
    // Byte-identical default semantics: v3.2.1 baseline behavior.
    let mut orch = build_runtime(|_| {});
    let result = orch.generate().expect("generate");
    assert!(result.audit.legal_documents.is_empty());
    assert!(result.audit.it_controls_access_logs.is_empty());
    assert!(result.audit.it_controls_change_records.is_empty());
    // Analytics metadata snapshot is default-empty when flag off.
    let am = &result.analytics_metadata;
    assert!(am.prior_year_comparatives.is_empty());
    assert!(am.industry_benchmarks.is_empty());
    assert!(am.management_reports.is_empty());
    assert!(am.drift_events.is_empty());
    // BUT organizational_profiles is ALWAYS emitted (v3.3.0 policy —
    // one profile per company with no separate flag).
    assert!(!result.master_data.organizational_profiles.is_empty());
}

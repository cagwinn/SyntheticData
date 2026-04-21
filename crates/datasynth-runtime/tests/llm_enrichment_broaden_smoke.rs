//! v4.1.1 — smoke test for `phase_llm_enrichment` broadening.
//!
//! Verifies that `llm.enrich_customers` / `llm.enrich_materials`
//! flags actually drive the runtime enrichment path (previously
//! vendors-only).

use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;

fn build_runtime(
    cfg_tweak: impl FnOnce(&mut datasynth_config::GeneratorConfig),
) -> EnhancedOrchestrator {
    let mut config = minimal_config();
    config.global.seed = Some(4110);
    config.global.period_months = 1;
    config.fraud.enabled = false;
    config.llm.enabled = true; // default mock provider
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
    phase_config.generate_journal_entries = false;
    EnhancedOrchestrator::new(config, phase_config).expect("build orchestrator")
}

#[test]
fn customers_enriched_when_flag_set() {
    let mut orch = build_runtime(|c| {
        c.llm.enrich_customers = true;
        c.llm.max_customer_enrichments = 5;
    });
    let result = orch.generate().expect("generate");
    assert!(
        result.statistics.llm_customers_enriched > 0,
        "expected some customer enrichments, got {}",
        result.statistics.llm_customers_enriched
    );
}

#[test]
fn materials_enriched_when_flag_set() {
    let mut orch = build_runtime(|c| {
        c.llm.enrich_materials = true;
        c.llm.max_material_enrichments = 5;
    });
    let result = orch.generate().expect("generate");
    assert!(
        result.statistics.llm_materials_enriched > 0,
        "expected some material enrichments, got {}",
        result.statistics.llm_materials_enriched
    );
}

#[test]
fn vendors_still_enriched_by_default() {
    let mut orch = build_runtime(|c| {
        c.llm.max_vendor_enrichments = 5;
    });
    let result = orch.generate().expect("generate");
    assert!(
        result.statistics.llm_vendors_enriched > 0,
        "vendors should still enrich even without new flags, got {}",
        result.statistics.llm_vendors_enriched
    );
}

#[test]
fn defaults_leave_customers_and_materials_untouched() {
    // llm.enabled=true but no extra flags → only vendors enrich.
    let mut orch = build_runtime(|c| {
        c.llm.max_vendor_enrichments = 3;
    });
    let result = orch.generate().expect("generate");
    assert!(result.statistics.llm_vendors_enriched > 0);
    assert_eq!(result.statistics.llm_customers_enriched, 0);
    assert_eq!(result.statistics.llm_materials_enriched, 0);
}

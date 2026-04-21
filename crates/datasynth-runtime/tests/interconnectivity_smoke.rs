//! v4.1.3 — smoke test for interconnectivity snapshot wiring.
//!
//! Verifies `vendor_network`, `customer_segmentation`, and
//! `industry_specific` sections move from validated-but-inert
//! (prior to v4.1.3) to populating `EnhancedGenerationResult.
//! interconnectivity` with tier / cluster / segment / lifecycle
//! labels.

use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;

fn build_runtime(
    cfg_tweak: impl FnOnce(&mut datasynth_config::GeneratorConfig),
) -> EnhancedOrchestrator {
    let mut config = minimal_config();
    config.global.seed = Some(4130);
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
    phase_config.generate_journal_entries = false;
    EnhancedOrchestrator::new(config, phase_config).expect("build orchestrator")
}

#[test]
fn interconnectivity_empty_when_all_disabled() {
    let mut orch = build_runtime(|c| {
        c.vendor_network.enabled = false;
        c.customer_segmentation.enabled = false;
        c.industry_specific.enabled = false;
    });
    let result = orch.generate().expect("generate");
    let inter = &result.interconnectivity;
    assert!(inter.vendor_tiers.is_empty());
    assert!(inter.vendor_clusters.is_empty());
    assert!(inter.customer_value_segments.is_empty());
    assert!(inter.customer_lifecycle_stages.is_empty());
    assert!(inter.industry_metadata.is_empty());
}

#[test]
fn vendor_network_populates_tiers_when_enabled() {
    let mut orch = build_runtime(|c| {
        c.vendor_network.enabled = true;
        c.vendor_network.depth = 3;
    });
    let result = orch.generate().expect("generate");
    let inter = &result.interconnectivity;
    assert!(
        !inter.vendor_tiers.is_empty(),
        "expected tier assignments when vendor_network.enabled"
    );
    assert!(!inter.vendor_clusters.is_empty());
    // Every vendor should have a tier + cluster assigned.
    assert_eq!(
        inter.vendor_tiers.len(),
        result.master_data.vendors.len(),
        "every vendor should receive a tier"
    );
    // Tiers should span 1..=3 for depth=3 with enough vendors.
    let tiers: std::collections::HashSet<u8> = inter.vendor_tiers.iter().map(|(_, t)| *t).collect();
    assert!(tiers.contains(&1), "expected at least one tier-1 vendor");
}

#[test]
fn customer_segmentation_assigns_segments() {
    let mut orch = build_runtime(|c| {
        c.customer_segmentation.enabled = true;
    });
    let result = orch.generate().expect("generate");
    let inter = &result.interconnectivity;
    assert!(!inter.customer_value_segments.is_empty());
    assert_eq!(
        inter.customer_value_segments.len(),
        result.master_data.customers.len()
    );
    assert_eq!(
        inter.customer_lifecycle_stages.len(),
        result.master_data.customers.len()
    );
    // Segment labels should be one of the 4 known values.
    let known = ["enterprise", "mid_market", "smb", "consumer"];
    for (_, label) in &inter.customer_value_segments {
        assert!(
            known.contains(&label.as_str()),
            "unexpected segment label: {label}"
        );
    }
}

#[test]
fn industry_specific_populates_metadata() {
    let mut orch = build_runtime(|c| {
        c.industry_specific.enabled = true;
    });
    let result = orch.generate().expect("generate");
    assert!(!result.interconnectivity.industry_metadata.is_empty());
}

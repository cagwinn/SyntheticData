//! v3.4.1 Sub-group B1 — smoke test for `TemporalContext` wiring into
//! P2P + O2C document-flow generators. Verifies that when
//! `temporal_patterns.business_days.enabled = true`, purchase-order
//! and sales-order document dates never land on weekends.

use chrono::{Datelike, Weekday};
use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;

fn build_runtime_with_temporal(enabled: bool) -> EnhancedOrchestrator {
    let mut config = minimal_config();
    config.global.seed = Some(3410);
    config.global.period_months = 3;
    config.fraud.enabled = false;
    config.temporal_patterns.enabled = enabled;
    config.temporal_patterns.business_days.enabled = enabled;
    config.temporal_patterns.calendars.regions = vec!["US".to_string()];

    let mut phase_config = PhaseConfig::from_config(&config);
    // Narrow to doc-flow only to bound test memory.
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
    phase_config.generate_document_flows = true;

    EnhancedOrchestrator::new(config, phase_config).expect("build orchestrator")
}

fn is_weekend(wd: Weekday) -> bool {
    matches!(wd, Weekday::Sat | Weekday::Sun)
}

#[test]
fn temporal_context_enabled_excludes_weekend_p2p() {
    let mut orch = build_runtime_with_temporal(true);
    let result = orch.generate().expect("generate");
    let pos = &result.document_flows.purchase_orders;
    assert!(!pos.is_empty(), "should generate some POs");
    let weekend_pos: Vec<_> = pos
        .iter()
        .filter(|po| is_weekend(po.header.document_date.weekday()))
        .collect();
    assert!(
        weekend_pos.is_empty(),
        "Expected 0 weekend POs with temporal_context enabled, got {} / {}",
        weekend_pos.len(),
        pos.len()
    );
}

#[test]
fn temporal_context_enabled_excludes_weekend_o2c() {
    let mut orch = build_runtime_with_temporal(true);
    let result = orch.generate().expect("generate");
    let sos = &result.document_flows.sales_orders;
    assert!(!sos.is_empty(), "should generate some SOs");
    let weekend_sos: Vec<_> = sos
        .iter()
        .filter(|so| is_weekend(so.header.document_date.weekday()))
        .collect();
    assert!(
        weekend_sos.is_empty(),
        "Expected 0 weekend SOs with temporal_context enabled, got {} / {}",
        weekend_sos.len(),
        sos.len()
    );
}

#[test]
fn temporal_context_disabled_shows_weekend_baseline() {
    // Without temporal context, raw rng produces weekend dates at the
    // expected 2/7 ≈ 28% base rate. This test just sanity-checks the
    // disabled path still produces non-empty output (no assertion on
    // weekend count itself — too noisy).
    let mut orch = build_runtime_with_temporal(false);
    let result = orch.generate().expect("generate");
    assert!(
        !result.document_flows.purchase_orders.is_empty(),
        "disabled-temporal path should still produce POs"
    );
    assert!(
        !result.document_flows.sales_orders.is_empty(),
        "disabled-temporal path should still produce SOs"
    );
}

#[test]
fn downstream_gr_invoice_also_weekday_only() {
    // Transitive check: GRs and invoices derived from business-day POs
    // should themselves land on business days (they're snapped in the
    // per-date helpers).
    let mut orch = build_runtime_with_temporal(true);
    let result = orch.generate().expect("generate");

    let grs = &result.document_flows.goods_receipts;
    let weekend_grs = grs
        .iter()
        .filter(|g| is_weekend(g.header.document_date.weekday()))
        .count();
    assert_eq!(
        weekend_grs,
        0,
        "Expected 0 weekend GRs, got {weekend_grs} / {}",
        grs.len()
    );

    let invoices = &result.document_flows.vendor_invoices;
    let weekend_invoices = invoices
        .iter()
        .filter(|v| is_weekend(v.header.document_date.weekday()))
        .count();
    assert_eq!(
        weekend_invoices,
        0,
        "Expected 0 weekend invoices, got {weekend_invoices} / {}",
        invoices.len()
    );
}

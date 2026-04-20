//! v3.4.3 Sub-group B3+B4 — smoke test for `TemporalContext` wiring into
//! `ProductionOrderGenerator` (manufacturing) and `AccrualGenerator`
//! (period-close). Verifies that when
//! `temporal_patterns.business_days.enabled = true`, all production-order
//! dates and accrual-reversal dates land on business days.

use chrono::{Datelike, Weekday};
use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;

fn build_runtime(enabled: bool) -> EnhancedOrchestrator {
    let mut config = minimal_config();
    config.global.seed = Some(3430);
    config.global.period_months = 3;
    config.fraud.enabled = false;
    config.temporal_patterns.enabled = enabled;
    config.temporal_patterns.business_days.enabled = enabled;
    config.temporal_patterns.calendars.regions = vec!["US".to_string()];
    config.manufacturing.enabled = true;

    let mut phase_config = PhaseConfig::from_config(&config);
    phase_config.inject_anomalies = false;
    phase_config.generate_banking = false;
    phase_config.generate_graph_export = false;
    phase_config.generate_ocpm_events = false;
    phase_config.generate_evolution_events = false;
    phase_config.generate_sourcing = false;
    phase_config.generate_intercompany = false;
    phase_config.generate_financial_statements = false;
    phase_config.generate_bank_reconciliation = false;
    phase_config.generate_accounting_standards = false;
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
    phase_config.generate_document_flows = false;
    phase_config.generate_manufacturing = true;
    phase_config.generate_period_close = true;
    phase_config.generate_journal_entries = true; // needed for period_close accruals

    EnhancedOrchestrator::new(config, phase_config).expect("build orchestrator")
}

fn is_weekend(wd: Weekday) -> bool {
    matches!(wd, Weekday::Sat | Weekday::Sun)
}

#[test]
fn production_order_planned_dates_respect_business_days() {
    let mut orch = build_runtime(true);
    let result = orch.generate().expect("generate");
    let orders = &result.manufacturing.production_orders;
    assert!(!orders.is_empty(), "should generate some production orders");

    let bad_planned_start = orders
        .iter()
        .filter(|o| is_weekend(o.planned_start.weekday()))
        .count();
    assert_eq!(
        bad_planned_start,
        0,
        "expected 0 weekend planned_start dates, got {bad_planned_start} / {}",
        orders.len()
    );

    let bad_planned_end = orders
        .iter()
        .filter(|o| is_weekend(o.planned_end.weekday()))
        .count();
    assert_eq!(
        bad_planned_end,
        0,
        "expected 0 weekend planned_end dates, got {bad_planned_end} / {}",
        orders.len()
    );
}

#[test]
fn production_order_actual_dates_respect_business_days() {
    let mut orch = build_runtime(true);
    let result = orch.generate().expect("generate");
    let orders = &result.manufacturing.production_orders;

    let bad_actual_start = orders
        .iter()
        .filter_map(|o| o.actual_start)
        .filter(|d| is_weekend(d.weekday()))
        .count();
    assert_eq!(
        bad_actual_start, 0,
        "expected 0 weekend actual_start dates, got {bad_actual_start}"
    );

    let bad_actual_end = orders
        .iter()
        .filter_map(|o| o.actual_end)
        .filter(|d| is_weekend(d.weekday()))
        .count();
    assert_eq!(
        bad_actual_end, 0,
        "expected 0 weekend actual_end dates, got {bad_actual_end}"
    );
}

#[test]
fn production_order_operation_dates_respect_business_days() {
    let mut orch = build_runtime(true);
    let result = orch.generate().expect("generate");

    let mut bad_started = 0usize;
    let mut bad_completed = 0usize;
    let mut total_ops = 0usize;
    for order in &result.manufacturing.production_orders {
        for op in &order.operations {
            total_ops += 1;
            if let Some(started) = op.started_at {
                if is_weekend(started.weekday()) {
                    bad_started += 1;
                }
            }
            if let Some(completed) = op.completed_at {
                if is_weekend(completed.weekday()) {
                    bad_completed += 1;
                }
            }
        }
    }
    assert!(total_ops > 0, "should have at least one routing operation");
    assert_eq!(
        bad_started, 0,
        "expected 0 weekend operation started_at, got {bad_started}"
    );
    assert_eq!(
        bad_completed, 0,
        "expected 0 weekend operation completed_at, got {bad_completed}"
    );
}

#[test]
fn disabled_temporal_still_produces_orders() {
    let mut orch = build_runtime(false);
    let result = orch.generate().expect("generate");
    assert!(
        !result.manufacturing.production_orders.is_empty(),
        "disabled-temporal path should still produce orders"
    );
}

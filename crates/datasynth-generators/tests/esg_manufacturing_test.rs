//! Tests for ESG ← Manufacturing bridge.
//!
//! Verifies that `EmissionGenerator::energy_from_production` correctly converts
//! completed production orders into `EnergyInput` records for Scope 1 (natural gas)
//! and Scope 2 (electricity) emission estimation.

#![allow(clippy::unwrap_used)]

use chrono::NaiveDate;
use datasynth_core::models::{
    ProductionOrder, ProductionOrderStatus, ProductionOrderType, RoutingOperation,
};
use datasynth_generators::esg::{EmissionGenerator, EnergyInputType};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Build a minimal `ProductionOrder` suitable for testing.
fn make_order(
    order_id: &str,
    status: ProductionOrderStatus,
    machine_hours: f64,
    actual_quantity: Decimal,
    work_center: &str,
    actual_end: Option<NaiveDate>,
    planned_end: NaiveDate,
) -> ProductionOrder {
    ProductionOrder {
        order_id: order_id.to_string(),
        company_code: "C001".to_string(),
        material_id: "MAT-001".to_string(),
        material_description: "Test Material".to_string(),
        order_type: ProductionOrderType::Standard,
        status,
        planned_quantity: actual_quantity,
        actual_quantity,
        scrap_quantity: Decimal::ZERO,
        planned_start: date(2025, 1, 1),
        planned_end,
        actual_start: Some(date(2025, 1, 2)),
        actual_end,
        work_center: work_center.to_string(),
        routing_id: None,
        planned_cost: dec!(10000),
        actual_cost: dec!(10500),
        cost_breakdown: None,
        labor_hours: 8.0,
        machine_hours,
        yield_rate: 0.98,
        batch_number: None,
        operations: Vec::<RoutingOperation>::new(),
    }
}

// ---------------------------------------------------------------------------
// Core conversion accuracy
// ---------------------------------------------------------------------------

#[test]
fn test_electricity_kwh_from_machine_hours() {
    // 10 machine hours × 50 kWh/hr = 500 kWh electricity
    let orders = vec![make_order(
        "PO-001",
        ProductionOrderStatus::Completed,
        10.0,
        dec!(100),
        "WC-PRESS",
        Some(date(2025, 2, 28)),
        date(2025, 2, 28),
    )];

    let kwh_per_machine_hour = dec!(50);
    let gas_kwh_per_unit = dec!(2);

    let inputs =
        EmissionGenerator::energy_from_production(&orders, kwh_per_machine_hour, gas_kwh_per_unit);

    let electricity: Vec<_> = inputs
        .iter()
        .filter(|i| i.energy_type == EnergyInputType::Electricity)
        .collect();

    assert_eq!(
        electricity.len(),
        1,
        "Should produce exactly one electricity input"
    );
    assert_eq!(
        electricity[0].consumption_kwh,
        dec!(500),
        "10 machine_hours × 50 kWh/hr should equal 500 kWh"
    );
    assert_eq!(electricity[0].facility_id, "WC-PRESS");
    assert_eq!(electricity[0].period, date(2025, 2, 28));
}

#[test]
fn test_natural_gas_kwh_from_actual_quantity() {
    // 100 units × 2 kWh/unit = 200 kWh natural gas
    let orders = vec![make_order(
        "PO-001",
        ProductionOrderStatus::Completed,
        10.0,
        dec!(100),
        "WC-PRESS",
        Some(date(2025, 2, 28)),
        date(2025, 2, 28),
    )];

    let kwh_per_machine_hour = dec!(50);
    let gas_kwh_per_unit = dec!(2);

    let inputs =
        EmissionGenerator::energy_from_production(&orders, kwh_per_machine_hour, gas_kwh_per_unit);

    let gas: Vec<_> = inputs
        .iter()
        .filter(|i| i.energy_type == EnergyInputType::NaturalGas)
        .collect();

    assert_eq!(gas.len(), 1, "Should produce exactly one natural gas input");
    assert_eq!(
        gas[0].consumption_kwh,
        dec!(200),
        "100 units × 2 kWh/unit should equal 200 kWh"
    );
    assert_eq!(gas[0].facility_id, "WC-PRESS");
    assert_eq!(gas[0].period, date(2025, 2, 28));
}

// ---------------------------------------------------------------------------
// Status filtering — only Completed and Closed pass through
// ---------------------------------------------------------------------------

#[test]
fn test_only_completed_and_closed_orders_included() {
    let orders = vec![
        make_order(
            "PO-PLANNED",
            ProductionOrderStatus::Planned,
            20.0,
            dec!(50),
            "WC-A",
            None,
            date(2025, 3, 31),
        ),
        make_order(
            "PO-RELEASED",
            ProductionOrderStatus::Released,
            20.0,
            dec!(50),
            "WC-A",
            None,
            date(2025, 3, 31),
        ),
        make_order(
            "PO-INPROCESS",
            ProductionOrderStatus::InProcess,
            20.0,
            dec!(50),
            "WC-A",
            None,
            date(2025, 3, 31),
        ),
        make_order(
            "PO-CANCELLED",
            ProductionOrderStatus::Cancelled,
            20.0,
            dec!(50),
            "WC-A",
            None,
            date(2025, 3, 31),
        ),
        make_order(
            "PO-COMPLETED",
            ProductionOrderStatus::Completed,
            20.0,
            dec!(50),
            "WC-A",
            Some(date(2025, 3, 30)),
            date(2025, 3, 31),
        ),
        make_order(
            "PO-CLOSED",
            ProductionOrderStatus::Closed,
            20.0,
            dec!(50),
            "WC-A",
            Some(date(2025, 3, 28)),
            date(2025, 3, 31),
        ),
    ];

    let inputs = EmissionGenerator::energy_from_production(&orders, dec!(10), dec!(1));

    // Only Completed and Closed contribute → 2 orders × 2 inputs each = 4
    assert_eq!(
        inputs.len(),
        4,
        "Should only include Completed and Closed orders (2 × 2 inputs = 4)"
    );
}

// ---------------------------------------------------------------------------
// Period fallback — planned_end when actual_end is None
// ---------------------------------------------------------------------------

#[test]
fn test_period_falls_back_to_planned_end() {
    let planned_end = date(2025, 4, 30);
    let orders = vec![make_order(
        "PO-CLOSED-NO-ACTUAL-END",
        ProductionOrderStatus::Closed,
        5.0,
        dec!(10),
        "WC-B",
        None, // no actual_end
        planned_end,
    )];

    let inputs = EmissionGenerator::energy_from_production(&orders, dec!(10), dec!(1));

    assert!(!inputs.is_empty());
    for input in &inputs {
        assert_eq!(
            input.period, planned_end,
            "Should fall back to planned_end when actual_end is absent"
        );
    }
}

// ---------------------------------------------------------------------------
// Facility ID fallback — company_code when work_center is empty
// ---------------------------------------------------------------------------

#[test]
fn test_facility_id_falls_back_to_company_code() {
    let orders = vec![make_order(
        "PO-NO-WC",
        ProductionOrderStatus::Completed,
        5.0,
        dec!(10),
        "", // empty work_center
        Some(date(2025, 5, 15)),
        date(2025, 5, 15),
    )];

    let inputs = EmissionGenerator::energy_from_production(&orders, dec!(10), dec!(1));

    assert!(!inputs.is_empty());
    for input in &inputs {
        assert_eq!(
            input.facility_id, "C001",
            "Should use company_code when work_center is empty"
        );
    }
}

// ---------------------------------------------------------------------------
// Multiple orders produce independent input records
// ---------------------------------------------------------------------------

#[test]
fn test_multiple_orders_produce_correct_totals() {
    // Order A: 8 machine_hours, 200 units → 400 kWh elec + 400 kWh gas
    // Order B: 4 machine_hours, 100 units → 200 kWh elec + 200 kWh gas
    let orders = vec![
        make_order(
            "PO-A",
            ProductionOrderStatus::Completed,
            8.0,
            dec!(200),
            "WC-X",
            Some(date(2025, 6, 30)),
            date(2025, 6, 30),
        ),
        make_order(
            "PO-B",
            ProductionOrderStatus::Closed,
            4.0,
            dec!(100),
            "WC-Y",
            Some(date(2025, 6, 29)),
            date(2025, 6, 30),
        ),
    ];

    let kwh_per_machine_hour = dec!(50);
    let gas_kwh_per_unit = dec!(2);

    let inputs =
        EmissionGenerator::energy_from_production(&orders, kwh_per_machine_hour, gas_kwh_per_unit);

    assert_eq!(inputs.len(), 4, "2 orders × 2 energy types = 4 inputs");

    let total_electricity: Decimal = inputs
        .iter()
        .filter(|i| i.energy_type == EnergyInputType::Electricity)
        .map(|i| i.consumption_kwh)
        .sum();
    assert_eq!(
        total_electricity,
        dec!(600),
        "Total electricity: (8+4)×50 = 600 kWh"
    );

    let total_gas: Decimal = inputs
        .iter()
        .filter(|i| i.energy_type == EnergyInputType::NaturalGas)
        .map(|i| i.consumption_kwh)
        .sum();
    assert_eq!(total_gas, dec!(600), "Total gas: (200+100)×2 = 600 kWh");
}

// ---------------------------------------------------------------------------
// Empty input and zero-consumption guards
// ---------------------------------------------------------------------------

#[test]
fn test_empty_orders_produces_empty_inputs() {
    let inputs = EmissionGenerator::energy_from_production(&[], dec!(50), dec!(2));
    assert!(inputs.is_empty(), "No orders → no energy inputs");
}

#[test]
fn test_zero_machine_hours_skips_electricity_input() {
    let orders = vec![make_order(
        "PO-NO-MACHINE",
        ProductionOrderStatus::Completed,
        0.0, // zero machine hours
        dec!(100),
        "WC-Z",
        Some(date(2025, 7, 31)),
        date(2025, 7, 31),
    )];

    let inputs = EmissionGenerator::energy_from_production(&orders, dec!(50), dec!(2));

    // Only natural gas input should appear (electricity skipped due to zero consumption)
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].energy_type, EnergyInputType::NaturalGas);
}

// ---------------------------------------------------------------------------
// Integration: energy_from_production feeds into generate_scope1/scope2
// ---------------------------------------------------------------------------

#[test]
fn test_production_derived_inputs_produce_scope1_and_scope2_emissions() {
    use datasynth_config::schema::EnvironmentalConfig;
    use datasynth_core::models::EmissionScope;

    let orders = vec![make_order(
        "PO-EMIT",
        ProductionOrderStatus::Completed,
        100.0,
        dec!(500),
        "WC-MAIN",
        Some(date(2025, 8, 31)),
        date(2025, 8, 31),
    )];

    let inputs = EmissionGenerator::energy_from_production(&orders, dec!(50), dec!(3));

    // Feeds directly into scope generators
    let mut gen = EmissionGenerator::new(EnvironmentalConfig::default(), 42);

    let scope1 = gen.generate_scope1("C001", &inputs);
    let scope2 = gen.generate_scope2("C001", &inputs);

    assert!(
        !scope1.is_empty(),
        "Should produce Scope 1 records from natural gas"
    );
    assert!(
        !scope2.is_empty(),
        "Should produce Scope 2 records from electricity"
    );
    assert!(
        scope1.iter().all(|r| r.scope == EmissionScope::Scope1),
        "All Scope 1 records should have correct scope"
    );
    assert!(
        scope2.iter().all(|r| r.scope == EmissionScope::Scope2),
        "All Scope 2 records should have correct scope"
    );

    // Verify co2e_tonnes are non-zero
    for r in scope1.iter().chain(scope2.iter()) {
        assert!(
            r.co2e_tonnes > Decimal::ZERO,
            "All emission records should be positive"
        );
    }
}

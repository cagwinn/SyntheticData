//! Integration tests for the warranty provision generator.
//!
//! Validates that warranty provisions are correctly derived from quality
//! inspection failure rates on completed production orders, with balanced
//! journal entries and correct ProvisionMovement identity.

#![allow(clippy::unwrap_used)]

use datasynth_config::schema::{ManufacturingCostingConfig, ProductionOrderConfig, RoutingConfig};
use datasynth_core::models::{InspectionResult, ProductionOrderStatus, ProvisionType};
use datasynth_generators::manufacturing::{
    ProductionOrderGenerator, QualityInspectionGenerator, WarrantyProvisionGenerator,
};
use rust_decimal::Decimal;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_completed_orders() -> Vec<datasynth_core::models::ProductionOrder> {
    let mut gen = ProductionOrderGenerator::new(42);
    let config = ProductionOrderConfig::default();
    let costing = ManufacturingCostingConfig::default();
    let routing = RoutingConfig::default();
    let materials = vec![("MAT-001".to_string(), "Widget".to_string())];
    let start = chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
    let mut orders = gen.generate("C001", &materials, start, end, &config, &costing, &routing);
    for o in &mut orders {
        o.status = ProductionOrderStatus::Completed;
        o.actual_end = Some(end);
    }
    orders
}

fn make_inspections(
    orders: &[datasynth_core::models::ProductionOrder],
) -> Vec<datasynth_core::models::QualityInspection> {
    let mut gen = QualityInspectionGenerator::new(42);
    let tuples: Vec<_> = orders
        .iter()
        .map(|o| {
            (
                o.order_id.clone(),
                o.material_id.clone(),
                o.material_description.clone(),
            )
        })
        .collect();
    let date = chrono::NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
    gen.generate("C001", &tuples, date)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_warranty_provisions_from_quality_failures() {
    let orders = make_completed_orders();
    let mut inspections = make_inspections(&orders);
    // Force some rejections — every 3rd inspection rejected → ~33% defect rate
    for (i, insp) in inspections.iter_mut().enumerate() {
        if i % 3 == 0 {
            insp.result = InspectionResult::Rejected;
        }
    }

    let mut gen = WarrantyProvisionGenerator::new(42);
    let result = gen.generate("C001", &orders, &inspections, "USD", "IFRS");

    assert!(
        !result.provisions.is_empty(),
        "Provisions should be created when defect rate ≥ 1%"
    );
    for prov in &result.provisions {
        assert_eq!(prov.provision_type, ProvisionType::Warranty);
        assert!(
            prov.best_estimate > Decimal::ZERO,
            "best_estimate must be positive"
        );
        assert!(
            prov.range_low <= prov.best_estimate,
            "range_low ({}) must be ≤ best_estimate ({})",
            prov.range_low,
            prov.best_estimate
        );
        assert!(
            prov.range_high >= prov.best_estimate,
            "range_high ({}) must be ≥ best_estimate ({})",
            prov.range_high,
            prov.best_estimate
        );
    }

    assert!(
        !result.journal_entries.is_empty(),
        "JEs should be created with provisions"
    );
    for je in &result.journal_entries {
        assert!(je.is_balanced(), "JE must be balanced (debits = credits)");
    }

    assert_eq!(
        result.movements.len(),
        result.provisions.len(),
        "One movement per provision"
    );
    for mvmt in &result.movements {
        let computed = mvmt.opening + mvmt.additions - mvmt.utilizations - mvmt.reversals
            + mvmt.unwinding_of_discount;
        assert_eq!(
            computed, mvmt.closing,
            "ProvisionMovement identity must hold"
        );
    }
}

#[test]
fn test_no_provisions_below_threshold() {
    let orders = make_completed_orders();
    let inspections = make_inspections(&orders);
    // Force all inspections to Accepted — zero rejection rate < 1% threshold
    let mut all_accepted = inspections;
    for insp in &mut all_accepted {
        insp.result = InspectionResult::Accepted;
    }

    let mut gen = WarrantyProvisionGenerator::new(42);
    let result = gen.generate("C001", &orders, &all_accepted, "USD", "US_GAAP");

    assert!(
        result.provisions.is_empty(),
        "No provisions when all inspections pass (defect rate = 0%)"
    );
    assert!(
        result.journal_entries.is_empty(),
        "No JEs when no provisions"
    );
    assert!(
        result.movements.is_empty(),
        "No movements when no provisions"
    );
}

#[test]
fn test_provision_provision_id_in_movement() {
    let orders = make_completed_orders();
    let mut inspections = make_inspections(&orders);
    for insp in inspections.iter_mut() {
        insp.result = InspectionResult::Rejected;
    }

    let mut gen = WarrantyProvisionGenerator::new(55);
    let result = gen.generate("C001", &orders, &inspections, "USD", "IFRS");

    assert!(!result.provisions.is_empty());
    assert!(!result.movements.is_empty());
    let prov_id = &result.provisions[0].id;
    let mvmt_prov_id = &result.movements[0].provision_id;
    assert_eq!(
        prov_id, mvmt_prov_id,
        "Movement.provision_id must match Provision.id"
    );
}

#[test]
fn test_us_gaap_framework() {
    let orders = make_completed_orders();
    let mut inspections = make_inspections(&orders);
    for (i, insp) in inspections.iter_mut().enumerate() {
        if i % 4 == 0 {
            insp.result = InspectionResult::Rejected;
        }
    }

    let mut gen = WarrantyProvisionGenerator::new(77);
    let result = gen.generate("C001", &orders, &inspections, "EUR", "US_GAAP");

    if !result.provisions.is_empty() {
        for prov in &result.provisions {
            assert_eq!(prov.framework, "US_GAAP");
            assert_eq!(prov.currency, "EUR");
        }
    }
}

#[test]
fn test_deterministic_with_same_seed() {
    let orders = make_completed_orders();
    let mut inspections = make_inspections(&orders);
    for (i, insp) in inspections.iter_mut().enumerate() {
        if i % 3 == 0 {
            insp.result = InspectionResult::Rejected;
        }
    }

    let mut gen1 = WarrantyProvisionGenerator::new(42);
    let result1 = gen1.generate("C001", &orders, &inspections, "USD", "IFRS");

    let mut gen2 = WarrantyProvisionGenerator::new(42);
    let result2 = gen2.generate("C001", &orders, &inspections, "USD", "IFRS");

    assert_eq!(result1.provisions.len(), result2.provisions.len());
    if !result1.provisions.is_empty() {
        assert_eq!(
            result1.provisions[0].best_estimate,
            result2.provisions[0].best_estimate
        );
        assert_eq!(result1.provisions[0].id, result2.provisions[0].id);
    }
}

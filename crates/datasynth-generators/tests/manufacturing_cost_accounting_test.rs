use datasynth_config::schema::{ManufacturingCostingConfig, ProductionOrderConfig, RoutingConfig};
use datasynth_generators::manufacturing::ProductionOrderGenerator;
use rust_decimal::Decimal;

#[test]
fn test_production_order_has_cost_breakdown() {
    let mut gen = ProductionOrderGenerator::new(42);
    let config = ProductionOrderConfig::default();
    let costing = ManufacturingCostingConfig::default();
    let routing = RoutingConfig::default();
    let materials = vec![
        ("MAT-001".to_string(), "Widget A".to_string()),
        ("MAT-002".to_string(), "Widget B".to_string()),
    ];
    let start = chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();

    let orders = gen.generate("C001", &materials, start, end, &config, &costing, &routing);

    assert!(!orders.is_empty());
    for order in &orders {
        let breakdown = order
            .cost_breakdown
            .as_ref()
            .expect("Every order should have a cost breakdown");
        assert_eq!(
            order.actual_cost,
            breakdown.total_actual(),
            "actual_cost must match cost_breakdown.total_actual()"
        );
        assert!(breakdown.material_cost >= Decimal::ZERO);
        assert!(breakdown.labor_cost >= Decimal::ZERO);
        assert!(breakdown.overhead_cost >= Decimal::ZERO);
        assert!(breakdown.standard_unit_cost > Decimal::ZERO);
    }
}

#[test]
fn test_cost_breakdown_labor_from_hours_and_rate() {
    let mut gen = ProductionOrderGenerator::new(99);
    let config = ProductionOrderConfig::default();
    let mut costing = ManufacturingCostingConfig::default();
    costing.labor_rate_per_hour = 50.0;
    costing.overhead_rate = 1.0;
    let routing = RoutingConfig::default();
    let materials = vec![("MAT-001".to_string(), "Part X".to_string())];
    let start = chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2025, 1, 31).unwrap();

    let orders = gen.generate("C001", &materials, start, end, &config, &costing, &routing);

    for order in &orders {
        let bd = order.cost_breakdown.as_ref().unwrap();
        // All cost components should be positive
        assert!(
            bd.material_cost > Decimal::ZERO,
            "Material cost should be positive"
        );
        assert!(
            bd.labor_cost > Decimal::ZERO,
            "Labor cost should be positive"
        );
        assert!(
            bd.overhead_cost > Decimal::ZERO,
            "Overhead cost should be positive"
        );
        // Overhead should be within reasonable range of labor (rate is 1.0, but there are variance factors)
        let ratio = bd.overhead_cost.to_string().parse::<f64>().unwrap()
            / bd.labor_cost.to_string().parse::<f64>().unwrap();
        assert!(
            ratio > 0.5 && ratio < 2.0,
            "Overhead/labor ratio {} should be reasonable at 100% overhead rate",
            ratio
        );
    }
}

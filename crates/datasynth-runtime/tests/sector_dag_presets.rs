//! Regression tests for the sector-specific causal DAG presets introduced
//! in v3.1.0 — `manufacturing`, `retail`, `financial_services`.
//!
//! Scope:
//! 1. Each YAML parses.
//! 2. Each DAG validates (acyclic + node/edge id integrity).
//! 3. Each DAG propagates without error for a baseline (no interventions).
//! 4. Each DAG includes the sector-distinctive nodes that callers depend on.

#![allow(clippy::unwrap_used)]

use datasynth_core::CausalDAG;

fn parse_and_validate(yaml: &str) -> CausalDAG {
    let mut dag: CausalDAG = serde_yaml::from_str(yaml).expect("yaml parses");
    dag.validate().expect("DAG validates");
    dag
}

#[test]
fn manufacturing_dag_parses_and_has_supply_chain_nodes() {
    let yaml = include_str!("../src/causal_dag_manufacturing.yaml");
    let dag = parse_and_validate(yaml);
    let ids: Vec<&str> = dag.nodes.iter().map(|n| n.id.as_str()).collect();
    for expected in [
        "supplier_reliability",
        "raw_material_cost",
        "lead_time_days",
        "bom_accuracy",
        "production_yield",
        "scrap_rate",
        "inventory_obsolescence_risk",
        "quality_escape_rate",
    ] {
        assert!(
            ids.contains(&expected),
            "manufacturing DAG is missing expected node `{expected}`"
        );
    }
    assert!(
        dag.edges.len() >= 10,
        "manufacturing DAG should have ≥ 10 edges"
    );
}

#[test]
fn retail_dag_parses_and_has_o2c_nodes() {
    let yaml = include_str!("../src/causal_dag_retail.yaml");
    let dag = parse_and_validate(yaml);
    let ids: Vec<&str> = dag.nodes.iter().map(|n| n.id.as_str()).collect();
    for expected in [
        "seasonal_demand_multiplier",
        "foot_traffic",
        "promotion_intensity",
        "stockout_rate",
        "return_rate",
        "shrinkage_rate",
        "days_sales_outstanding",
        "revenue_cutoff_risk",
    ] {
        assert!(
            ids.contains(&expected),
            "retail DAG is missing expected node `{expected}`"
        );
    }
    assert!(dag.edges.len() >= 10, "retail DAG should have ≥ 10 edges");
}

#[test]
fn financial_services_dag_parses_and_has_aml_nodes() {
    let yaml = include_str!("../src/causal_dag_financial_services.yaml");
    let dag = parse_and_validate(yaml);
    let ids: Vec<&str> = dag.nodes.iter().map(|n| n.id.as_str()).collect();
    for expected in [
        "correspondent_concentration",
        "regulatory_pressure_index",
        "sanctions_environment",
        "kyc_score",
        "aml_screening_strength",
        "aml_true_positive_rate",
        "aml_false_positive_rate",
        "liquidity_coverage_ratio",
        "npl_ratio",
    ] {
        assert!(
            ids.contains(&expected),
            "financial_services DAG is missing expected node `{expected}`"
        );
    }
    assert!(
        dag.edges.len() >= 12,
        "financial_services DAG should have ≥ 12 edges"
    );
}

#[test]
fn all_presets_propagate_with_no_interventions() {
    for (name, yaml) in [
        (
            "manufacturing",
            include_str!("../src/causal_dag_manufacturing.yaml"),
        ),
        ("retail", include_str!("../src/causal_dag_retail.yaml")),
        (
            "financial_services",
            include_str!("../src/causal_dag_financial_services.yaml"),
        ),
    ] {
        let dag = parse_and_validate(yaml);
        let interventions = std::collections::HashMap::new();
        let result = dag.propagate(&interventions, 3);
        // Baseline values should roundtrip — every node ID maps to some finite f64.
        assert_eq!(
            result.len(),
            dag.nodes.len(),
            "{name}: propagation result missing nodes"
        );
        for (id, v) in &result {
            assert!(
                v.is_finite(),
                "{name}: node `{id}` propagated to non-finite value {v}"
            );
        }
    }
}

#[test]
fn manufacturing_dag_has_topological_order_supplier_before_margin() {
    let yaml = include_str!("../src/causal_dag_manufacturing.yaml");
    let dag = parse_and_validate(yaml);
    let pos = |id: &str| dag.topological_order.iter().position(|n| n == id);
    let supplier = pos("supplier_reliability").unwrap();
    let margin = pos("gross_margin").unwrap();
    assert!(
        supplier < margin,
        "supplier_reliability ({supplier}) must come before gross_margin ({margin})"
    );
}

//! Spec §4.1 — AuditEngagementPlan resolution tests (Task 2.6).

use datasynth_group::{
    build_audit_engagement_plan,
    config::{
        AuditEngagementConfig, ComponentScopeThresholds, GroupMaterialityConfig, MaterialityBasis,
    },
    manifest::{audit_plan::ComponentScope, expansion::EntitySource},
    ConsolidationMethod, ExpandedEntity,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn zero_seed() -> [u8; 32] {
    [0u8; 32]
}

fn make_entity(code: &str, country: &str, rows: Option<u64>) -> ExpandedEntity {
    ExpandedEntity {
        code: code.to_string(),
        name: None,
        country: country.to_string(),
        functional_currency: "USD".to_string(),
        scoping_profile: "significant".to_string(),
        consolidation_method: ConsolidationMethod::Full,
        ownership_percent: Some(dec!(1.0)),
        parent_code: None,
        accounting_framework: None,
        industry: None,
        hyperinflation_status: datasynth_core::models::HyperinflationStatus::NotHyperinflationary,
        source: EntitySource::Explicit,
        generated_block_index: None,
        rows,
    }
}

fn minimal_cfg(basis: MaterialityBasis, percent: Decimal) -> AuditEngagementConfig {
    AuditEngagementConfig {
        group_materiality: Some(GroupMaterialityConfig { basis, percent }),
        ..Default::default()
    }
}

// ── Test 1: group materiality computation ─────────────────────────────────────

#[test]
fn test_group_materiality_computed_from_basis_and_percent() {
    // 5 entities × 1 M rows = 5 M rows.  Revenue proxy = 5 M × $1 000 = $5 B.
    // 1 % of $5 B = $50 M.
    let entities: Vec<_> = (1..=5)
        .map(|i| make_entity(&format!("E{i}"), "US", Some(1_000_000)))
        .collect();
    let cfg = minimal_cfg(MaterialityBasis::Revenue, dec!(0.01));

    let plan = build_audit_engagement_plan(&cfg, &entities, "GRP", &zero_seed()).unwrap();

    // Expected: 5_000_000 rows × 1_000 $/row × 0.01 = 50_000_000
    assert_eq!(plan.group_materiality, dec!(50000000));
}

// ── Test 2: performance_materiality and clearly_trivial derived from group ────

#[test]
fn test_derived_materiality_thresholds() {
    let entities = vec![make_entity("E1", "US", Some(1_000_000))];
    let cfg = minimal_cfg(MaterialityBasis::Revenue, dec!(0.01));

    let plan = build_audit_engagement_plan(&cfg, &entities, "GRP", &zero_seed()).unwrap();

    let gm = plan.group_materiality;
    // performance = gm × 0.75
    let expected_pm = gm * dec!(0.75);
    // clearly trivial = gm × 0.05
    let expected_ct = gm * dec!(0.05);

    assert_eq!(
        plan.performance_materiality, expected_pm,
        "performance_materiality"
    );
    assert_eq!(plan.clearly_trivial, expected_ct, "clearly_trivial");
}

// ── Test 3: component scope thresholds — full / specific / analytical ─────────

#[test]
fn test_component_scope_thresholds() {
    // 10 entities, each with 1 M rows → 10 M total.
    // Revenue share per entity = 10 %.
    // With defaults: full ≥ 15 % → none Full; specific ≥ 5 % → all Specific.
    // But we set custom thresholds: full ≥ 9 %, specific ≥ 5 % so all are Full.
    let entities: Vec<_> = (1..=10)
        .map(|i| make_entity(&format!("E{i:02}"), "US", Some(1_000_000)))
        .collect();
    let mut cfg = minimal_cfg(MaterialityBasis::Revenue, dec!(0.01));
    cfg.component_scope_thresholds = Some(ComponentScopeThresholds {
        full_scope: dec!(0.09),
        specific_scope: dec!(0.05),
    });

    let plan = build_audit_engagement_plan(&cfg, &entities, "GRP", &zero_seed()).unwrap();

    // Each entity has exactly 10 % share → ≥ 9 % → Full scope.
    for alloc in &plan.component_materiality_allocations {
        assert_eq!(
            alloc.scope,
            ComponentScope::Full,
            "entity {} should be Full scope (10 % share ≥ 9 % threshold)",
            alloc.entity_code
        );
    }
}

// ── Test 4: sub-5 % entities are Analytical ───────────────────────────────────

#[test]
fn test_small_entities_are_analytical() {
    // 1 large entity (90 M rows) + 10 tiny entities (1 M rows each).
    // Large entity revenue share ≈ 90 % → Full.
    // Tiny entities share ≈ 1 % each → below 5 % → Analytical.
    let mut entities = vec![make_entity("BIG", "US", Some(90_000_000))];
    for i in 1..=10 {
        entities.push(make_entity(&format!("T{i:02}"), "DE", Some(1_000_000)));
    }
    let cfg = minimal_cfg(MaterialityBasis::Revenue, dec!(0.01));

    let plan = build_audit_engagement_plan(&cfg, &entities, "GRP", &zero_seed()).unwrap();

    let big = plan
        .component_materiality_allocations
        .iter()
        .find(|a| a.entity_code == "BIG")
        .unwrap();
    assert_eq!(big.scope, ComponentScope::Full, "BIG should be Full");

    for alloc in plan
        .component_materiality_allocations
        .iter()
        .filter(|a| a.entity_code.starts_with('T'))
    {
        assert_eq!(
            alloc.scope,
            ComponentScope::Analytical,
            "{} should be Analytical (< 5 % share)",
            alloc.entity_code
        );
    }
}

// ── Test 5: component auditors grouped by country ─────────────────────────────

#[test]
fn test_component_auditors_grouped_by_country() {
    let entities = vec![
        make_entity("US1", "US", None),
        make_entity("US2", "US", None),
        make_entity("DE1", "DE", None),
        make_entity("GB1", "GB", None),
    ];
    let cfg = minimal_cfg(MaterialityBasis::Revenue, dec!(0.01));

    let plan = build_audit_engagement_plan(&cfg, &entities, "GRP", &zero_seed()).unwrap();

    // One ComponentAuditor per distinct country.
    let countries: std::collections::BTreeSet<&str> = plan
        .component_auditors
        .iter()
        .map(|ca| ca.jurisdiction.as_str())
        .collect();
    assert!(countries.contains("US"), "US auditor expected");
    assert!(countries.contains("DE"), "DE auditor expected");
    assert!(countries.contains("GB"), "GB auditor expected");
    assert_eq!(plan.component_auditors.len(), 3, "3 distinct countries");

    let us_ca = plan
        .component_auditors
        .iter()
        .find(|ca| ca.jurisdiction == "US")
        .unwrap();
    assert_eq!(us_ca.entities.len(), 2, "US should have 2 entities");
    assert!(us_ca.entities.contains(&"US1".to_string()));
    assert!(us_ca.entities.contains(&"US2".to_string()));
}

// ── Test 6: missing group_materiality returns error ───────────────────────────

#[test]
fn test_missing_group_materiality_errors() {
    let entities = vec![make_entity("E1", "US", None)];
    let cfg = AuditEngagementConfig {
        group_materiality: None,
        ..Default::default()
    };

    let result = build_audit_engagement_plan(&cfg, &entities, "GRP", &zero_seed());
    assert!(
        result.is_err(),
        "missing group_materiality should return an error"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("group_materiality"),
        "error should mention group_materiality; got: {msg}"
    );
}

// ── Test 7: default field values applied ──────────────────────────────────────

#[test]
fn test_default_fields() {
    let entities = vec![make_entity("E1", "US", None)];
    let cfg = minimal_cfg(MaterialityBasis::Revenue, dec!(0.01));

    let plan = build_audit_engagement_plan(&cfg, &entities, "GRP_X", &zero_seed()).unwrap();

    assert_eq!(plan.engagement_id, "GRP_X_ENGAGEMENT");
    assert_eq!(plan.lead_auditor, "UNDESIGNATED");
    assert_eq!(plan.framework, "isa");
    assert_eq!(plan.fsm_blueprint, "builtin:group_fsa");
}

// ── Test 8: lead country gets UNDESIGNATED-LEAD firm ─────────────────────────

#[test]
fn test_lead_country_firm_designation() {
    // First entity is US → US should be lead jurisdiction.
    let entities = vec![
        make_entity("US1", "US", None),
        make_entity("DE1", "DE", None),
    ];
    let cfg = minimal_cfg(MaterialityBasis::Revenue, dec!(0.01));

    let plan = build_audit_engagement_plan(&cfg, &entities, "GRP", &zero_seed()).unwrap();

    let us_ca = plan
        .component_auditors
        .iter()
        .find(|ca| ca.jurisdiction == "US")
        .unwrap();
    assert_eq!(us_ca.firm, "UNDESIGNATED-LEAD");

    let de_ca = plan
        .component_auditors
        .iter()
        .find(|ca| ca.jurisdiction == "DE")
        .unwrap();
    assert_eq!(de_ca.firm, "UNDESIGNATED");
}

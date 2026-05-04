//! Spec §4.1 — TaxGroupPlan resolution tests (Task 2.7).

use datasynth_group::{
    build_tax_group_plan,
    config::{CbcReportConfig, PillarTwoConfig, TaxGroupConfig, TpConfig},
    manifest::expansion::EntitySource,
    ConsolidationMethod, ExpandedEntity,
};
use rust_decimal_macros::dec;

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn make_entity(code: &str, country: &str) -> ExpandedEntity {
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
        source: EntitySource::Explicit,
        generated_block_index: None,
        rows: None,
        hyperinflation_status: datasynth_core::models::HyperinflationStatus::NotHyperinflationary,
        ownership_changes: Vec::new(),
    }
}

fn empty_tax_cfg() -> TaxGroupConfig {
    TaxGroupConfig {
        pillar_two: None,
        cbc_report: None,
        transfer_pricing: None,
    }
}

// ── Test 1: all sub-configs disabled → empty plan ─────────────────────────────

#[test]
fn test_empty_plan_when_all_disabled() {
    let entities = vec![make_entity("E1", "US"), make_entity("E2", "DE")];
    let cfg = empty_tax_cfg();

    let plan = build_tax_group_plan(&cfg, &entities).unwrap();

    assert!(plan.pillar_two.is_none(), "pillar_two should be None");
    assert!(plan.cbc_report.is_none(), "cbc_report should be None");
    assert!(
        plan.transfer_pricing.is_none(),
        "transfer_pricing should be None"
    );
}

// ── Test 2: Pillar 2 jurisdiction validated against entity countries ───────────

#[test]
fn test_pillar_two_unknown_jurisdiction_errors() {
    let entities = vec![make_entity("E1", "US"), make_entity("E2", "DE")];
    let cfg = TaxGroupConfig {
        pillar_two: Some(PillarTwoConfig {
            enabled: true,
            jurisdictions: vec!["US".to_string(), "FR".to_string()], // FR is unknown
        }),
        ..empty_tax_cfg()
    };

    let result = build_tax_group_plan(&cfg, &entities);
    assert!(
        result.is_err(),
        "unknown Pillar 2 jurisdiction should error"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("FR"),
        "error should mention the offending jurisdiction; got: {msg}"
    );
}

// ── Test 3: Pillar 2 entities_in_scope filtered correctly ─────────────────────

#[test]
fn test_pillar_two_entities_in_scope() {
    let entities = vec![
        make_entity("E_US", "US"),
        make_entity("E_DE1", "DE"),
        make_entity("E_DE2", "DE"),
        make_entity("E_GB", "GB"),
    ];
    let cfg = TaxGroupConfig {
        pillar_two: Some(PillarTwoConfig {
            enabled: true,
            jurisdictions: vec!["US".to_string(), "DE".to_string()],
        }),
        ..empty_tax_cfg()
    };

    let plan = build_tax_group_plan(&cfg, &entities).unwrap();
    let p2 = plan.pillar_two.expect("pillar_two should be Some");

    assert_eq!(
        p2.in_scope_jurisdictions,
        vec!["DE".to_string(), "US".to_string()],
        "jurisdictions should be sorted"
    );
    assert!(p2.entities_in_scope.contains(&"E_US".to_string()));
    assert!(p2.entities_in_scope.contains(&"E_DE1".to_string()));
    assert!(p2.entities_in_scope.contains(&"E_DE2".to_string()));
    assert!(
        !p2.entities_in_scope.contains(&"E_GB".to_string()),
        "GB not in scope"
    );
    // Sorted ascending.
    let mut sorted = p2.entities_in_scope.clone();
    sorted.sort();
    assert_eq!(
        p2.entities_in_scope, sorted,
        "entities_in_scope should be sorted"
    );
}

// ── Test 4: Pillar 2 disabled → None ─────────────────────────────────────────

#[test]
fn test_pillar_two_disabled_produces_none() {
    let entities = vec![make_entity("E1", "US")];
    let cfg = TaxGroupConfig {
        pillar_two: Some(PillarTwoConfig {
            enabled: false,
            jurisdictions: vec!["US".to_string()],
        }),
        ..empty_tax_cfg()
    };

    let plan = build_tax_group_plan(&cfg, &entities).unwrap();
    assert!(
        plan.pillar_two.is_none(),
        "disabled pillar_two should be None"
    );
}

// ── Test 5: CbC requires reporting_jurisdiction ───────────────────────────────

#[test]
fn test_cbc_requires_reporting_jurisdiction() {
    let entities = vec![make_entity("E1", "US")];
    let cfg = TaxGroupConfig {
        cbc_report: Some(CbcReportConfig {
            enabled: true,
            reporting_jurisdiction: None, // missing!
        }),
        ..empty_tax_cfg()
    };

    let result = build_tax_group_plan(&cfg, &entities);
    assert!(
        result.is_err(),
        "missing reporting_jurisdiction should error"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("reporting_jurisdiction"),
        "error should mention reporting_jurisdiction; got: {msg}"
    );
}

// ── Test 6: CbC jurisdictions_to_report = all distinct entity countries ────────

#[test]
fn test_cbc_jurisdictions_to_report() {
    let entities = vec![
        make_entity("E1", "US"),
        make_entity("E2", "DE"),
        make_entity("E3", "US"),
        make_entity("E4", "GB"),
    ];
    let cfg = TaxGroupConfig {
        cbc_report: Some(CbcReportConfig {
            enabled: true,
            reporting_jurisdiction: Some("CH".to_string()),
        }),
        ..empty_tax_cfg()
    };

    let plan = build_tax_group_plan(&cfg, &entities).unwrap();
    let cbc = plan.cbc_report.expect("cbc_report should be Some");

    assert_eq!(cbc.reporting_jurisdiction, "CH");
    // Distinct countries sorted: DE, GB, US.
    assert_eq!(
        cbc.jurisdictions_to_report,
        vec!["DE".to_string(), "GB".to_string(), "US".to_string()],
        "should be sorted set of distinct entity countries"
    );
}

// ── Test 7: TP local_file_jurisdictions validated ─────────────────────────────

#[test]
fn test_tp_unknown_local_file_jurisdiction_errors() {
    let entities = vec![make_entity("E1", "US"), make_entity("E2", "DE")];
    let cfg = TaxGroupConfig {
        transfer_pricing: Some(TpConfig {
            master_file: true,
            local_files_for: vec!["US".to_string(), "JP".to_string()], // JP unknown
        }),
        ..empty_tax_cfg()
    };

    let result = build_tax_group_plan(&cfg, &entities);
    assert!(
        result.is_err(),
        "unknown TP local-file jurisdiction should error"
    );
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("JP"), "error should mention JP; got: {msg}");
}

// ── Test 8: TP plan passes through correctly ──────────────────────────────────

#[test]
fn test_tp_plan_passthrough() {
    let entities = vec![make_entity("E1", "US"), make_entity("E2", "DE")];
    let cfg = TaxGroupConfig {
        transfer_pricing: Some(TpConfig {
            master_file: true,
            local_files_for: vec!["DE".to_string(), "US".to_string()],
        }),
        ..empty_tax_cfg()
    };

    let plan = build_tax_group_plan(&cfg, &entities).unwrap();
    let tp = plan
        .transfer_pricing
        .expect("transfer_pricing should be Some");

    assert!(tp.master_file, "master_file should be true");
    assert_eq!(
        tp.local_file_jurisdictions,
        vec!["DE".to_string(), "US".to_string()],
        "should be sorted"
    );
}

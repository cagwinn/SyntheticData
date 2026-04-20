//! v3.3.2 — smoke tests for the 5 previously `[Not yet wired]` audit
//! sub-config fields in `AuditGenerationConfig`.

use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;

fn build_runtime(
    cfg_tweak: impl FnOnce(&mut datasynth_config::GeneratorConfig),
) -> EnhancedOrchestrator {
    let mut config = minimal_config();
    config.global.seed = Some(3320);
    config.global.period_months = 1;
    config.audit.enabled = true;
    cfg_tweak(&mut config);
    let mut phase_config = PhaseConfig::from_config(&config);
    // Narrow phase set for test-memory bounds.
    phase_config.generate_document_flows = false;
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
    phase_config.generate_audit = true;
    EnhancedOrchestrator::new(config, phase_config).expect("build orchestrator")
}

#[test]
fn generate_workpapers_false_skips_workpapers_and_evidence() {
    let mut orch = build_runtime(|c| {
        c.audit.generate_workpapers = false;
    });
    let result = orch.generate().expect("generate");
    // Engagements should still generate, but workpapers + evidence empty.
    assert!(
        !result.audit.engagements.is_empty(),
        "engagements should generate even when workpapers disabled"
    );
    assert!(
        result.audit.workpapers.is_empty(),
        "workpapers should be empty when generate_workpapers=false"
    );
    assert!(
        result.audit.evidence.is_empty(),
        "evidence should be empty (depends on workpapers)"
    );
}

#[test]
fn generate_workpapers_default_produces_workpapers() {
    // Default is true — should produce workpapers when audit enabled.
    let mut orch = build_runtime(|_c| {});
    let result = orch.generate().expect("generate");
    assert!(
        !result.audit.workpapers.is_empty(),
        "workpapers should be non-empty with default config"
    );
}

#[test]
fn team_config_min_max_respected() {
    let mut orch = build_runtime(|c| {
        c.audit.team.min_team_size = 10;
        c.audit.team.max_team_size = 12;
    });
    let result = orch.generate().expect("generate");
    for engagement in &result.audit.engagements {
        let team_size = engagement.team_member_ids.len();
        assert!(
            (10..=12).contains(&team_size),
            "team size {team_size} outside configured [10, 12] band"
        );
    }
}

#[test]
fn engagement_types_distribution_respects_config() {
    use datasynth_core::models::audit::EngagementType;
    // Force all engagements to be ReviewEngagement.
    let mut orch = build_runtime(|c| {
        c.audit.engagement_types.financial_statement = 0.0;
        c.audit.engagement_types.sox_icfr = 0.0;
        c.audit.engagement_types.integrated = 0.0;
        c.audit.engagement_types.review = 1.0;
        c.audit.engagement_types.agreed_upon_procedures = 0.0;
    });
    let result = orch.generate().expect("generate");
    for engagement in &result.audit.engagements {
        assert_eq!(
            engagement.engagement_type,
            EngagementType::ReviewEngagement,
            "engagement type should follow configured distribution"
        );
    }
}

#[test]
fn workpapers_per_phase_affects_workpaper_count() {
    // Compare two runs with different average_per_phase values.
    let small = {
        let mut orch = build_runtime(|c| {
            c.audit.workpapers.average_per_phase = 2;
        });
        orch.generate().expect("gen").audit.workpapers.len()
    };
    let large = {
        let mut orch = build_runtime(|c| {
            c.audit.workpapers.average_per_phase = 12;
        });
        orch.generate().expect("gen").audit.workpapers.len()
    };
    assert!(
        large > small,
        "workpaper count should grow with average_per_phase ({small} -> {large})"
    );
}

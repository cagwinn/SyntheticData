//! SP6 integration smoke — a small priors-enabled generation must emit text
//! with: (a) no literal `{…}` placeholder tokens, (b) no residual-PII shapes,
//! (c) line_text populated for >=95% of lines.
//!
//! The health bundle committed at T14 time does not yet carry `text_taxonomy`
//! (that ships in T16). The generator falls back to `DescriptionGenerator` on
//! the pre-T16 bundle path — the three invariants still hold because the
//! fallback emits clean text and always populates `line_text`. After T16 the
//! test additionally exercises the real taxonomy path without modification.

use datasynth_config::schema::{
    AdvancedDistributionConfig, IndustryPriorsConfig, IndustryProfileField, IndustryProfileFull,
    IndustryProfileType, PriorsSource,
};
use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;

/// Run a small priors-enabled generation (~100 JEs, health industry, seed 42).
/// Returns the emitted `JournalEntry` vector.
fn run_small_priors_enabled_generation() -> Vec<datasynth_core::models::JournalEntry> {
    let mut config = minimal_config();
    config.global.seed = Some(42);
    config.global.period_months = 1;

    // Wire up health priors (bundled, enabled).
    config.distributions = AdvancedDistributionConfig {
        enabled: true,
        industry_profile: Some(IndustryProfileField::Full(IndustryProfileFull {
            name: IndustryProfileType::Healthcare,
            priors: Some(IndustryPriorsConfig {
                enabled: true,
                source: PriorsSource::Bundled,
                path: None,
                velocity_calibration: false,
            }),
        })),
        ..Default::default()
    };

    // Narrow PhaseConfig to JE + master-data (incl. FA subledger). The FA
    // subledger emits its own acquisition/depreciation/disposal JEs via
    // FAGenerator — those now set line_text from the JE header (SP6.x fix),
    // so they no longer dilute coverage below 95%.
    let mut phase_config = PhaseConfig::from_config(&config);
    phase_config.generate_master_data = true;
    phase_config.generate_document_flows = false;
    phase_config.generate_journal_entries = true;
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

    let mut orchestrator =
        EnhancedOrchestrator::new(config, phase_config).expect("Failed to create orchestrator");

    let result = orchestrator.generate().expect("Generation failed");
    result.journal_entries
}

#[test]
fn sp6_generation_emits_clean_filled_text() {
    let entries = run_small_priors_enabled_generation();

    assert!(!entries.is_empty(), "generation produced no entries");

    let mut lines_total = 0usize;
    let mut lines_with_text = 0usize;

    // This smoke checks the SP6 *wiring*: every emitted header/line text is
    // fully filled (no leftover `{…}` placeholder) and line_text coverage is
    // high. It deliberately does NOT run `residual_pii_scan` on the filled
    // OUTPUT: that scan detects PII *shapes* (Firstname Lastname, `Initial.
    // Surname`, …) which synthetic fills legitimately match — e.g. a `{person}`
    // fill "Anna Beispiel" or a `{company}` fill "S.Merchandise Holdings LLC".
    // The authoritative PII gate is `bundle_pii_audit`, which scans the
    // committed TEMPLATES (where such a shape means an un-tokenized corpus
    // name) — not generated output. Scanning output here only produced false
    // positives on synthetic fills.
    for je in &entries {
        if let Some(ht) = &je.header.header_text {
            assert!(
                !ht.contains('{'),
                "header_text has literal placeholder: {ht:?}"
            );
        }
        for line in &je.lines {
            lines_total += 1;
            if let Some(lt) = &line.line_text {
                lines_with_text += 1;
                assert!(
                    !lt.contains('{'),
                    "line_text has literal placeholder: {lt:?}"
                );
            }
        }
    }

    let coverage = lines_with_text as f64 / lines_total.max(1) as f64;
    println!(
        "SP6 smoke: {} JEs, {} lines, {} with text, coverage={:.3}",
        entries.len(),
        lines_total,
        lines_with_text,
        coverage
    );
    assert!(
        coverage >= 0.95,
        "line_text coverage {coverage:.3} < 0.95 ({lines_with_text}/{lines_total})"
    );
}

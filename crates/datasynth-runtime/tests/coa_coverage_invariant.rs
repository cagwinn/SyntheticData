//! Invariant test: every `gl_account` referenced in generated journal
//! entries must exist in the generated chart of accounts.
//!
//! Historically a handful of generators emitted hardcoded GL strings
//! (e.g. `"1300"` for inventory) that did not appear in the seeded COA,
//! leaving downstream consumers unable to resolve account names. This
//! test guards against that regression along three axes:
//!
//! 1. **`every_je_gl_account_exists_in_coa`** — base pipeline (subledger,
//!    document flows, period close, accounting standards, tax, HR,
//!    treasury, manufacturing) with anomaly injection off.
//! 2. **`every_je_gl_account_exists_in_coa_with_anomalies`** — same
//!    pipeline plus anomaly injection enabled, which exercises strategies
//!    like `DormantAccountActivity` that swap in legacy / blocked accounts.
//!    Without explicit seeding of those targets the COA would be missing
//!    them.

use std::collections::BTreeSet;

use datasynth_runtime::{EnhancedGenerationResult, EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;

/// Build an orchestrator covering JE-emitting code paths likely to leak
/// orphan GL accounts. `inject_anomalies` is the only knob exposed.
fn build_runtime(inject_anomalies: bool) -> EnhancedOrchestrator {
    let mut config = minimal_config();
    config.global.seed = Some(424242);
    config.global.period_months = 2;
    config.fraud.enabled = false;
    // Reconcile subledgers so AR/AP/FA/Inventory JEs flow into the result.
    config.balance.reconcile_subledgers = true;

    let mut phase_config = PhaseConfig::from_config(&config);
    phase_config.show_progress = false;
    phase_config.generate_journal_entries = true;
    phase_config.generate_document_flows = true;
    phase_config.generate_period_close = true;
    phase_config.generate_accounting_standards = true;
    // Enable JE-emitting domains to flush hidden orphan accounts.
    phase_config.generate_tax = true;
    phase_config.generate_hr = true;
    phase_config.generate_treasury = true;
    phase_config.generate_manufacturing = true;
    phase_config.inject_anomalies = inject_anomalies;
    // Off: orthogonal to COA coverage and slow on small fixtures.
    phase_config.generate_intercompany = false;
    phase_config.generate_banking = false;
    phase_config.generate_graph_export = false;
    phase_config.generate_ocpm_events = false;
    phase_config.generate_audit = false;
    phase_config.inject_data_quality = false;
    phase_config.generate_evolution_events = false;
    phase_config.generate_sourcing = false;
    phase_config.generate_sales_kpi_budgets = false;
    phase_config.generate_esg = false;
    phase_config.generate_project_accounting = false;
    phase_config.generate_compliance_regulations = false;
    phase_config.generate_financial_statements = false;
    phase_config.generate_bank_reconciliation = false;
    phase_config.validate_balances = false;

    EnhancedOrchestrator::new(config, phase_config).expect("build orchestrator")
}

/// Collect gl_accounts referenced in JEs that are not present in the COA.
fn collect_missing(result: &EnhancedGenerationResult) -> (BTreeSet<String>, usize, usize) {
    let coa: BTreeSet<&str> = result
        .chart_of_accounts
        .accounts
        .iter()
        .map(|a| a.account_number.as_str())
        .collect();
    let mut missing: BTreeSet<String> = BTreeSet::new();
    for je in &result.journal_entries {
        for line in je.lines.iter() {
            if !coa.contains(line.gl_account.as_str()) {
                missing.insert(line.gl_account.clone());
            }
        }
    }
    (missing, coa.len(), result.journal_entries.len())
}

fn assert_full_coverage(label: &str, result: &EnhancedGenerationResult) {
    let (missing, coa_size, je_count) = collect_missing(result);
    assert!(
        missing.is_empty(),
        "[{label}] JEs reference {} gl_account values that are not in the chart of accounts: {:?}\n\
         (COA has {} accounts; total JEs: {}). Each missing value indicates either\n\
         a generator using a raw string instead of an `accounts.rs` constant, or a\n\
         constant whose module is not seeded by `seed_canonical_accounts`.",
        missing.len(),
        missing,
        coa_size,
        je_count,
    );
}

#[test]
fn every_je_gl_account_exists_in_coa() {
    let mut orch = build_runtime(false);
    let result = orch.generate().expect("generate");
    assert_full_coverage("base", &result);
}

#[test]
fn every_je_gl_account_exists_in_coa_with_anomalies() {
    let mut orch = build_runtime(true);
    let result = orch.generate().expect("generate");
    // Anomaly injection must have actually run (otherwise the test is
    // a tautology against the base case).
    let any_anomaly = result.journal_entries.iter().any(|je| je.header.is_anomaly);
    assert!(
        any_anomaly,
        "anomaly-injection branch produced zero anomaly entries — \
         test fixture probably needs a higher injection rate"
    );
    assert_full_coverage("with-anomalies", &result);
}

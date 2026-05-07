//! Invariant test: every `gl_account` referenced in generated journal
//! entries (and subledger postings that flow into JEs) must exist in the
//! generated chart of accounts.
//!
//! Historically a handful of generators emitted hardcoded GL strings
//! (e.g. `"1300"` for inventory) that did not appear in the seeded COA,
//! leaving downstream consumers unable to resolve account names. This
//! test guards against that regression.

use std::collections::BTreeSet;

use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;

/// Build an orchestrator that exercises the JE-emitting code paths most
/// likely to introduce orphan GL accounts: subledger (AR/AP/FA/Inventory),
/// document flows, period close, and accounting standards.
fn build_runtime() -> EnhancedOrchestrator {
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
    // Off: do not exercise these here (orthogonal to COA coverage and slow).
    phase_config.generate_intercompany = false;
    phase_config.generate_banking = false;
    phase_config.generate_graph_export = false;
    phase_config.generate_ocpm_events = false;
    phase_config.generate_audit = false;
    phase_config.inject_anomalies = false;
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

#[test]
fn every_je_gl_account_exists_in_coa() {
    let mut orch = build_runtime();
    let result = orch.generate().expect("generate");

    let coa_accounts: BTreeSet<&str> = result
        .chart_of_accounts
        .accounts
        .iter()
        .map(|a| a.account_number.as_str())
        .collect();

    let mut missing: BTreeSet<String> = BTreeSet::new();
    for je in &result.journal_entries {
        for line in je.lines.iter() {
            if !coa_accounts.contains(line.gl_account.as_str()) {
                missing.insert(line.gl_account.clone());
            }
        }
    }

    assert!(
        missing.is_empty(),
        "JEs reference {} gl_account values that are not in the chart of accounts: {:?}\n\
         (COA has {} accounts; total JEs: {}). Each missing value indicates either\n\
         a generator using a raw string instead of an `accounts.rs` constant, or a\n\
         constant whose module is not seeded by `seed_canonical_accounts`.",
        missing.len(),
        missing,
        coa_accounts.len(),
        result.journal_entries.len(),
    );
}

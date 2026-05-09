//! Aggregate phase — Chunk 5.
//!
//! After the shard phase has driven [`crate::shard::run_shard`] for every
//! shard in the manifest, the aggregate phase walks the resulting
//! `{out_dir}/entities/{code}/` subtrees, loads each entity's per-entity
//! artefacts back into memory, and combines them into group-level
//! consolidation outputs (group trial balance, eliminations,
//! NCI measurement, segment reporting, FX translation results).
//!
//! Task 5.1 lays the foundation: a per-entity trial balance loader that
//! later modules (group TB combiner, IC elimination engine, NCI roll-up,
//! segment aggregator) will all build on. The loader's contract is
//! deliberately narrow — open one file, deserialise one [`TrialBalance`],
//! re-verify the balance invariant, return it — so the higher-level
//! combiners can assume a clean per-entity TB or a typed
//! [`crate::errors::GroupError::Aggregate`] failure they can attribute to
//! a specific entity directory.
//!
//! See `docs/superpowers/specs/2026-04-23-group-audit-simulation-design.md`
//! §"Aggregate phase" for the full Chunk-5 module layout.

pub mod cgu_impairment;
pub mod coverage_report;
pub mod driver;
pub mod elimination;
pub mod equity_method;
pub mod fs;
pub mod ic_matcher;
pub mod je_network;
pub mod nci;
pub mod opening_balance;
pub mod post_elim;
pub mod pre_elim;
pub mod tb_loader;
pub mod translation;

pub use cgu_impairment::{
    run_cgu_impairment_tests, write_cgu_impairment_tests, CguTestInputs,
    CGU_IMPAIRMENT_TESTS_FILENAME,
};
pub use coverage_report::{
    build_coverage_report, write_coverage_report, CoverageReport, COVERAGE_REPORT_FILENAME,
    COVERAGE_REPORT_SUBDIR, UNMATCHED_SAMPLE_CAP,
};
pub use driver::{
    run_aggregate, run_aggregate_chain, AggregateOptions, AggregateSummary, PeriodSpec,
};
pub use elimination::{eliminations_to_journal_entries, generate_eliminations, EliminationResult};
pub use equity_method::{
    compute_equity_method_investment, ingest_opening_equity_method_carrying_values,
    ingest_opening_suppressed_losses, write_equity_method_investments, write_suppressed_losses,
    EquityMethodInputs, EquityMethodInvestment, EQUITY_METHOD_INVESTMENTS_FILENAME,
    EQUITY_METHOD_SUPPRESSED_LOSSES_FILENAME,
};
pub use fs::{
    build_consolidated_balance_sheet, build_consolidated_balance_sheet_with_names,
    build_consolidated_cash_flow, build_consolidated_income_statement,
    build_consolidated_income_statement_with_names, build_consolidation_schedule,
    build_notes_to_consolidated_fs, build_statement_of_changes_in_equity, write_consolidated_fs,
    AccountNameDictionary, BsLine, CashFlowInputs, CfLine, CfSection, ConsolidatedBalanceSheet,
    ConsolidatedCashFlow, ConsolidatedFinancialStatements, ConsolidatedIncomeStatement,
    ConsolidationSchedule, EquityChangesInputs, EquityRollforward, IsLine, Note, NotesInputs,
    NotesToConsolidatedFs, ScheduleLine, StatementOfChangesInEquity, CONSOLIDATED_FS_FILENAME,
    CONSOLIDATION_SCHEDULE_FILENAME, NOTES_FILENAME,
};
pub use ic_matcher::{
    match_ic_pairs, IcMatchResult, IcMatchedPair, UnmatchedReason, UnmatchedSide,
};
pub use je_network::{write_je_network_artefacts, JeNetworkSummary};
pub use nci::{
    compute_nci_rollforward, ingest_opening_nci_balances, write_nci_rollforward, NciInputs,
    NciRollforward, NCI_ROLLFORWARD_FILENAME,
};
pub use opening_balance::{
    extract_opening_balances, read_prior_period_closing_tbs, OpeningBalance,
};
pub use post_elim::{apply_eliminations_to_tb, apply_nci_and_equity_method};
pub use pre_elim::{aggregate_pre_elimination, AggregatedAccount, AggregatedTb, DeferredEntity};
pub use tb_loader::load_entity_trial_balance;
pub use translation::{
    classify_account, compute_cta, cta_rollforward, select_restatement_path, translate_entity_tb,
    translate_entity_tb_with_hyperinflation, translate_entity_tb_with_indexed_restatement,
    write_cta_rollforward, write_translation_worksheet, CtaRollforward, DrCr, IndexedRestatement,
    RateBasis, RestatementPath, TranslatedLine, TranslatedTb, TranslationAccountType,
    WorksheetEntity, WorksheetLine, CONSOLIDATED_SUBDIR, CTA_ROLLFORWARD_FILENAME,
    TRANSLATION_WORKSHEET_FILENAME,
};

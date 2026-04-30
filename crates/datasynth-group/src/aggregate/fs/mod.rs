//! Consolidated financial-statement assembly — Chunk 8.
//!
//! After [`crate::aggregate::post_elim::apply_eliminations_to_tb`] (Task 5.6)
//! and [`crate::aggregate::post_elim::apply_nci_and_equity_method`]
//! (Task 7.4) have produced a fully balanced post-elimination
//! [`crate::aggregate::pre_elim::AggregatedTb`], the aggregate phase
//! must turn that consolidated trial balance into the human-readable
//! consolidated financial statements that auditors and statutory
//! preparers consume.
//!
//! # Standards reference
//!
//! - **IFRS** — *IAS 1 Presentation of Financial Statements*, *IAS 7
//!   Statement of Cash Flows*, *IFRS 10 Consolidated Financial
//!   Statements* (NCI separately presented), *IAS 28* (equity-method
//!   single-line), *IAS 21* (CTA in OCI).
//! - **US GAAP** — *ASC 205* Presentation, *ASC 230* Cash Flows, *ASC
//!   810* Consolidation (NCI separate equity component).
//!
//! # Module layout
//!
//! - [`balance_sheet`] (Task 8.1) — IFRS / ASC consolidated balance
//!   sheet with NCI separately presented.
//! - [`income_statement`] (Task 8.2) — consolidated income statement
//!   with split between owners and NCI.
//! - [`cash_flow`] (Task 8.3) — IAS 7 indirect-method cash flow
//!   statement with FX-effect plug.
//! - [`equity_changes`] (Task 8.4) — statement of changes in equity,
//!   owners + NCI rollforwards.
//! - [`consolidation_schedule`] (Task 8.5) — pre / adjustment / post
//!   per-account schedule.
//! - [`notes`] (Task 8.6) — basic note set (8 notes) assembled from
//!   manifest + coverage + NCI / CTA / equity-method inputs.
//! - [`writer`] (Task 8.7) — JSON writer that emits the consolidated
//!   FS bundle, schedule, and notes under
//!   `{out_dir}/consolidated/`.
//!
//! # v5.1 status
//!
//! - **Account-name dictionary** ✅ shipped (PR #136) — labels sourced
//!   from the manifest's [`crate::manifest::ChartOfAccountsMaster`]
//!   via [`account_names::AccountNameDictionary`].  Engagement-supplied
//!   labels (e.g. SKR04 / PCG localisation) win over the built-in
//!   canonical English labels.
//! - **Suppressed-loss tracking** ✅ shipped (PR #135) — equity-method
//!   investments now carry an opening / closing suppressed-loss
//!   memorandum per IAS 28.38, with full recovery semantics and a
//!   side-artefact `equity_method_suppressed_losses.json`.
//! - **Operating segments — Note 6** ✅ shipped (PR #137) — Note 6
//!   now emits IFRS 8.13 / ASC 280-10-50-41 entity-wide geographic
//!   disclosure derived from the manifest's ownership graph.
//! - **Retained-earnings integration** ✅ shipped (this PR) — the
//!   equity-method overlay's counterparty side now posts directly to
//!   retained earnings (`3300`), retiring the v5.0 `3400` bridge
//!   account.  See [`crate::aggregate::post_elim`] module rustdoc.
//!
//! # Still deferred
//!
//! - Full per-segment revenue / profit / asset aggregation across
//!   entities (consolidated `OperatingSegment` table from per-entity
//!   `segment_reports.json`).  IFRS 8.5 product-line / business-unit
//!   basis, on the v5.2 roadmap.
//! - Auto-impairment of equity-method investments (caller-supplied in
//!   v5.0).
//! - Subsequent events / related-parties auto-derivation.

pub mod account_names;
pub mod balance_sheet;
pub mod cash_flow;
pub mod consolidation_schedule;
pub mod equity_changes;
pub mod income_statement;
pub mod notes;
pub mod writer;

pub use account_names::AccountNameDictionary;
pub use balance_sheet::{
    build_consolidated_balance_sheet, build_consolidated_balance_sheet_with_names, BsLine,
    ConsolidatedBalanceSheet,
};
pub use cash_flow::{
    build_consolidated_cash_flow, CashFlowInputs, CfLine, CfSection, ConsolidatedCashFlow,
};
pub use consolidation_schedule::{
    build_consolidation_schedule, ConsolidationSchedule, ScheduleLine,
};
pub use equity_changes::{
    build_statement_of_changes_in_equity, EquityChangesInputs, EquityRollforward,
    StatementOfChangesInEquity,
};
pub use income_statement::{
    build_consolidated_income_statement, build_consolidated_income_statement_with_names,
    ConsolidatedIncomeStatement, IsLine,
};
pub use notes::{build_notes_to_consolidated_fs, Note, NotesInputs, NotesToConsolidatedFs};
pub use writer::{
    write_consolidated_fs, ConsolidatedFinancialStatements, CONSOLIDATED_FS_FILENAME,
    CONSOLIDATION_SCHEDULE_FILENAME, NOTES_FILENAME,
};

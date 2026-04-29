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
//! # v5.1 deferrals
//!
//! - Operating segment reporting (note 6 emits a placeholder).
//! - Full retained-earnings integration (the equity-method bridge
//!   account `3400` is a v5.0 simplification — see
//!   [`crate::aggregate::post_elim`] module rustdoc).
//! - Auto-impairment of equity-method investments (caller-supplied in
//!   v5.0).
//! - Subsequent events / related-parties auto-derivation.

pub mod balance_sheet;
pub mod cash_flow;
pub mod consolidation_schedule;
pub mod equity_changes;
pub mod income_statement;
pub mod notes;
pub mod writer;

pub use balance_sheet::{build_consolidated_balance_sheet, BsLine, ConsolidatedBalanceSheet};
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
    build_consolidated_income_statement, ConsolidatedIncomeStatement, IsLine,
};
pub use notes::{build_notes_to_consolidated_fs, Note, NotesInputs, NotesToConsolidatedFs};
pub use writer::{
    write_consolidated_fs, ConsolidatedFinancialStatements, CONSOLIDATED_FS_FILENAME,
    CONSOLIDATION_SCHEDULE_FILENAME, NOTES_FILENAME,
};

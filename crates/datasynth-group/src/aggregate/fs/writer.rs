//! Consolidated FS output assembly + writer — Task 8.7.
//!
//! Bundles the four consolidated statements (BS + IS + CF +
//! Statement of Changes in Equity) into one record, then writes that
//! record alongside the consolidation schedule and notes to
//! `{out_dir}/consolidated/`.
//!
//! # On-disk contract
//!
//! ```text
//! {out_dir}/consolidated/
//!   ├── consolidated_financial_statements.json
//!   ├── consolidation_schedule.json
//!   └── notes_to_consolidated_fs.json
//! ```
//!
//! All three files are pretty-printed JSON with a trailing newline so
//! they're human-readable when opened in an editor and diff cleanly
//! between runs.
//!
//! Returned [`PathBuf`] order is fixed:
//!
//! 1. `consolidated_financial_statements.json`
//! 2. `consolidation_schedule.json`
//! 3. `notes_to_consolidated_fs.json`

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::aggregate::fs::balance_sheet::ConsolidatedBalanceSheet;
use crate::aggregate::fs::cash_flow::ConsolidatedCashFlow;
use crate::aggregate::fs::consolidation_schedule::ConsolidationSchedule;
use crate::aggregate::fs::equity_changes::StatementOfChangesInEquity;
use crate::aggregate::fs::income_statement::ConsolidatedIncomeStatement;
use crate::aggregate::fs::notes::NotesToConsolidatedFs;
use crate::errors::{GroupError, GroupResult};

/// Subdirectory under the group output root for consolidated FS files.
pub const CONSOLIDATED_SUBDIR: &str = "consolidated";

/// File name for the bundled consolidated FS.
pub const CONSOLIDATED_FS_FILENAME: &str = "consolidated_financial_statements.json";

/// File name for the consolidation schedule.
pub const CONSOLIDATION_SCHEDULE_FILENAME: &str = "consolidation_schedule.json";

/// File name for the notes to consolidated FS.
pub const NOTES_FILENAME: &str = "notes_to_consolidated_fs.json";

// ── Public types ──────────────────────────────────────────────────────────────

/// Bundled consolidated financial statements.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsolidatedFinancialStatements {
    /// IFRS / ASC consolidated balance sheet (Task 8.1).
    pub balance_sheet: ConsolidatedBalanceSheet,
    /// IFRS / ASC consolidated income statement (Task 8.2).
    pub income_statement: ConsolidatedIncomeStatement,
    /// IAS 7 / ASC 230 consolidated cash flow (Task 8.3).
    pub cash_flow: ConsolidatedCashFlow,
    /// IAS 1 / ASC 220 statement of changes in equity (Task 8.4).
    pub changes_in_equity: StatementOfChangesInEquity,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Write the consolidated FS bundle, schedule, and notes to
/// `{out_dir}/consolidated/`.
///
/// Creates the subdirectory if it doesn't exist.  Output files are
/// pretty-printed JSON with a trailing newline.  Returns the absolute
/// paths in the order documented in the module rustdoc.
///
/// # Errors
///
/// - [`GroupError::Io`] if the subdirectory creation or any file
///   write fails.
/// - [`GroupError::Serde`] if any of the three records fails to
///   serialise (should be impossible — every field is
///   `Serialize`-friendly).
pub fn write_consolidated_fs(
    fs_bundle: &ConsolidatedFinancialStatements,
    schedule: &ConsolidationSchedule,
    notes: &NotesToConsolidatedFs,
    out_dir: &Path,
) -> GroupResult<Vec<PathBuf>> {
    let dir = out_dir.join(CONSOLIDATED_SUBDIR);
    fs::create_dir_all(&dir).map_err(GroupError::Io)?;

    let fs_path = dir.join(CONSOLIDATED_FS_FILENAME);
    let mut fs_json = serde_json::to_string_pretty(fs_bundle)?;
    fs_json.push('\n');
    fs::write(&fs_path, fs_json).map_err(GroupError::Io)?;

    let sched_path = dir.join(CONSOLIDATION_SCHEDULE_FILENAME);
    let mut sched_json = serde_json::to_string_pretty(schedule)?;
    sched_json.push('\n');
    fs::write(&sched_path, sched_json).map_err(GroupError::Io)?;

    let notes_path = dir.join(NOTES_FILENAME);
    let mut notes_json = serde_json::to_string_pretty(notes)?;
    notes_json.push('\n');
    fs::write(&notes_path, notes_json).map_err(GroupError::Io)?;

    Ok(vec![fs_path, sched_path, notes_path])
}

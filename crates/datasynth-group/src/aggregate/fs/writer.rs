//! Consolidated FS output assembly + writer — Task 8.7 (stub).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::aggregate::fs::balance_sheet::ConsolidatedBalanceSheet;
use crate::aggregate::fs::cash_flow::ConsolidatedCashFlow;
use crate::aggregate::fs::consolidation_schedule::ConsolidationSchedule;
use crate::aggregate::fs::equity_changes::StatementOfChangesInEquity;
use crate::aggregate::fs::income_statement::ConsolidatedIncomeStatement;
use crate::aggregate::fs::notes::NotesToConsolidatedFs;
use crate::errors::GroupResult;

/// Subdirectory under the group output root for consolidated FS files.
pub const CONSOLIDATED_SUBDIR: &str = "consolidated";

/// File name for the bundled consolidated FS.
pub const CONSOLIDATED_FS_FILENAME: &str = "consolidated_financial_statements.json";

/// File name for the consolidation schedule.
pub const CONSOLIDATION_SCHEDULE_FILENAME: &str = "consolidation_schedule.json";

/// File name for the notes to consolidated FS.
pub const NOTES_FILENAME: &str = "notes_to_consolidated_fs.json";

#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsolidatedFinancialStatements {
    pub balance_sheet: ConsolidatedBalanceSheet,
    pub income_statement: ConsolidatedIncomeStatement,
    pub cash_flow: ConsolidatedCashFlow,
    pub changes_in_equity: StatementOfChangesInEquity,
}

/// Write consolidated FS bundle, schedule, and notes (placeholder until Task 8.7).
#[allow(unused_variables)]
pub fn write_consolidated_fs(
    fs: &ConsolidatedFinancialStatements,
    schedule: &ConsolidationSchedule,
    notes: &NotesToConsolidatedFs,
    out_dir: &Path,
) -> GroupResult<Vec<PathBuf>> {
    Ok(Vec::new())
}

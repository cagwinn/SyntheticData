//! Notes to consolidated FS — Task 8.6 (stub).

use chrono::NaiveDate;
use datasynth_standards::framework::AccountingFramework;
use serde::{Deserialize, Serialize};

use crate::aggregate::coverage_report::CoverageReport;
use crate::aggregate::equity_method::EquityMethodInvestment;
use crate::aggregate::nci::NciRollforward;
use crate::aggregate::translation::CtaRollforward;
use crate::manifest::GroupManifest;

#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotesToConsolidatedFs {
    pub group_id: String,
    pub period_end: NaiveDate,
    pub framework: String,
    pub notes: Vec<Note>,
}

#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Note {
    pub note_number: u32,
    pub title: String,
    pub body: String,
}

#[allow(missing_docs)]
pub struct NotesInputs<'a> {
    pub manifest: &'a GroupManifest,
    pub framework: AccountingFramework,
    pub ic_coverage: &'a CoverageReport,
    pub nci_rollforwards: &'a [NciRollforward],
    pub cta_rollforwards: &'a [CtaRollforward],
    pub equity_method_investments: &'a [EquityMethodInvestment],
}

/// Build the notes to consolidated FS (placeholder until Task 8.6).
#[allow(unused_variables)]
pub fn build_notes_to_consolidated_fs(
    inputs: &NotesInputs,
    period_end: NaiveDate,
) -> NotesToConsolidatedFs {
    NotesToConsolidatedFs {
        group_id: inputs.manifest.group_id.clone(),
        period_end,
        framework: inputs.framework.to_string(),
        notes: Vec::new(),
    }
}

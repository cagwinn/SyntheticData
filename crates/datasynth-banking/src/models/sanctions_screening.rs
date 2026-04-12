//! Sanctions screening model for KYC compliance.
//!
//! Tracks the results of sanctions list screening (OFAC, EU, UN, etc.)
//! per customer, including fuzzy match scores and name variations.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Sanctions screening result for a customer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanctionsScreening {
    /// Date of last screening
    pub last_screened: NaiveDate,
    /// Screening result
    pub screening_result: ScreeningResult,
    /// Sanctions list that triggered a match (if any)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_list: Option<String>,
    /// Fuzzy match confidence score (0.0 = no match, 1.0 = exact match)
    #[serde(default)]
    pub match_score: f64,
    /// Name variations / aliases that were checked
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_variations: Vec<String>,
    /// Ground truth: whether this is a genuine match (vs false positive name collision)
    #[serde(default)]
    pub is_true_match: bool,
}

/// Result of a sanctions screening check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScreeningResult {
    /// No matches found on any list
    #[default]
    Clear,
    /// Fuzzy match found, requires manual review
    PotentialMatch,
    /// Confirmed match on a sanctions list
    ConfirmedMatch,
    /// Match was previously cleared by compliance
    FalsePositiveCleared,
}

impl ScreeningResult {
    /// Risk weight for this screening result.
    pub fn risk_weight(&self) -> f64 {
        match self {
            Self::Clear => 0.0,
            Self::FalsePositiveCleared => 0.1,
            Self::PotentialMatch => 0.7,
            Self::ConfirmedMatch => 1.0,
        }
    }
}

/// Known sanctions lists for matching.
pub struct SanctionsLists;

impl SanctionsLists {
    pub const LISTS: &[&str] = &[
        "OFAC SDN",
        "OFAC Consolidated",
        "EU Consolidated",
        "UN Security Council",
        "UK HMT",
        "Australia DFAT",
        "Canada OSFI",
    ];
}

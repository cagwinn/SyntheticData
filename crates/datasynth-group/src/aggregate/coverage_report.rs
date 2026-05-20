//! IC matching coverage report — Task 5.7.
//!
//! After [`crate::aggregate::ic_matcher::match_ic_pairs`] has joined the
//! seller-side and buyer-side journal entries of every IC pair, this
//! module summarises the result into a sidecar diagnostic / observability
//! artefact for auditors and dashboards.
//!
//! # Position in the v5.0 aggregate phase
//!
//! ```text
//!   match_ic_pairs           Task 5.3
//!         │
//!         ▼
//!   IcMatchResult ──────► build_coverage_report ──► CoverageReport
//!         │                                              │
//!         ▼                                              ▼
//!   generate_eliminations   Task 5.4              ic_eliminations/
//!         │                                       ic_matching_coverage.json
//!         ▼
//!   eliminations_to_journal_entries   Task 5.5
//!         │
//!         ▼
//!   apply_eliminations_to_tb          Task 5.6
//! ```
//!
//! The report is intentionally **not** part of the consolidated TB
//! pipeline.  The downstream elimination → JE → post-elim chain consumes
//! [`crate::aggregate::ic_matcher::IcMatchResult`] directly; the coverage
//! report is a sidecar artefact whose only readers are auditors,
//! dashboards, and the "Good engagement" gate (`coverage ≥ 0.98` per spec
//! §5.4).
//!
//! # On-disk contract
//!
//! Per spec §5.4, the report lands at
//! `{out_dir}/ic_eliminations/ic_matching_coverage.json` with these fields:
//!
//! ```json
//! {
//!   "total_pairs_planned": 250000,
//!   "matched": 248750,
//!   "coverage": 0.995,
//!   "unmatched_by_reason": {
//!     "missing_buyer_side": 780,
//!     "missing_seller_side": 350,
//!     "amount_drift_above_tolerance": 120
//!   },
//!   "unmatched_sample": [/* first 100 unmatched sides */]
//! }
//! ```
//!
//! # Sample cap
//!
//! `unmatched_sample` is capped at the first **100** entries from
//! [`crate::aggregate::ic_matcher::IcMatchResult::unmatched`] (which is
//! pre-sorted by `pair_id` for determinism).  The cap matches the spec's
//! "first 100 unmatched pairs for debugging" — the goal is diagnostic
//! triage, not exhaustive replay.  Auditors needing the full unmatched
//! list can re-run [`crate::aggregate::ic_matcher::match_ic_pairs`] in a
//! debugger.
//!
//! # Schema stability
//!
//! `unmatched_by_reason` is always emitted with **all three**
//! [`UnmatchedReason`] variants present (zero-valued when no
//! observations) so dashboards can rely on the keys existing.  The map
//! is a [`BTreeMap`] keyed by the reason enum so the JSON output is
//! deterministically ordered.
//!
//! # Determinism
//!
//! Two calls with identical [`IcMatchResult`] inputs produce
//! byte-identical JSON output.  This is verified by the
//! `deterministic_output` test in `tests/coverage_report.rs`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::aggregate::ic_matcher::{IcMatchResult, UnmatchedReason, UnmatchedSide};
use crate::errors::{GroupError, GroupResult};

/// Maximum number of unmatched sides included in
/// [`CoverageReport::unmatched_sample`].  Mirrors the spec §5.4
/// "\[:100\]" semantics.
pub const UNMATCHED_SAMPLE_CAP: usize = 100;

/// Subdirectory within the group output root where the coverage report
/// lives.  Per spec §5.4, the report sits alongside the elimination JEs.
pub const COVERAGE_REPORT_SUBDIR: &str = "ic_eliminations";

/// File name for the coverage report, per spec §5.4.
pub const COVERAGE_REPORT_FILENAME: &str = "ic_matching_coverage.json";

// ── Public types ──────────────────────────────────────────────────────────────

/// Diagnostic / observability summary of [`IcMatchResult`].
///
/// See spec §5.4 for the field contract.  All counts mirror the
/// matcher's already-computed values — this report is a thin pass-through
/// layer with the histogram + sample for auditors / dashboards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageReport {
    /// Total number of distinct IC pairs the manifest planned (sum
    /// across all entities of seller-side plans).  Mirrors
    /// [`IcMatchResult::total_planned`].
    pub total_pairs_planned: usize,
    /// Number of fully matched pairs (both sides observed).  Equals
    /// `IcMatchResult::matched.len()`.
    pub matched: usize,
    /// `matched / total_pairs_planned`, with `0/0` mapped to `0.0`
    /// (mirrors [`IcMatchResult::coverage`] so the two stay
    /// consistent — see the matcher's module docs for the rationale).
    pub coverage: f64,
    /// Histogram of unmatched sides keyed by reason.
    ///
    /// Always contains all three [`UnmatchedReason`] variants — even
    /// when zero — so the schema is stable across runs and dashboards
    /// can rely on the keys being present.  Keyed by the enum itself
    /// (with snake_case serde rename) and stored in a [`BTreeMap`] so
    /// the JSON output's iteration order is deterministic.
    pub unmatched_by_reason: BTreeMap<UnmatchedReason, usize>,
    /// Up to [`UNMATCHED_SAMPLE_CAP`] unmatched sides for diagnostic
    /// triage.  The sample is the head of
    /// [`IcMatchResult::unmatched`] (already sorted by `pair_id` for
    /// determinism) so the same input always produces the same sample.
    pub unmatched_sample: Vec<UnmatchedSide>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Build a [`CoverageReport`] from an [`IcMatchResult`].
///
/// Pure function: no I/O, no allocation beyond the report itself, no
/// dependence on global state.  Two calls with the same input produce
/// equal reports.
///
/// # Behaviour
///
/// 1. **Counts.**  `total_pairs_planned`, `matched`, and `coverage`
///    are passed through verbatim from `result` — the matcher already
///    did the math, we just relabel.
/// 2. **Histogram.**  Initialise `unmatched_by_reason` with all three
///    [`UnmatchedReason`] variants set to zero (so the schema stays
///    stable for downstream dashboards) and increment the relevant
///    bucket for each entry in `result.unmatched`.
/// 3. **Sample.**  Take the first [`UNMATCHED_SAMPLE_CAP`] entries
///    from `result.unmatched`.  The matcher pre-sorts `unmatched` by
///    `pair_id`, so the sample is deterministic.
pub fn build_coverage_report(result: &IcMatchResult) -> CoverageReport {
    // Initialise the histogram with all three reasons present so
    // downstream dashboards can rely on the keys existing even when a
    // run has zero observations of a given reason.
    let mut unmatched_by_reason: BTreeMap<UnmatchedReason, usize> = BTreeMap::new();
    unmatched_by_reason.insert(UnmatchedReason::MissingBuyerSide, 0);
    unmatched_by_reason.insert(UnmatchedReason::MissingSellerSide, 0);
    unmatched_by_reason.insert(UnmatchedReason::AmountDriftAboveTolerance, 0);

    for side in &result.unmatched {
        *unmatched_by_reason.entry(side.reason).or_insert(0) += 1;
    }

    let unmatched_sample: Vec<UnmatchedSide> = result
        .unmatched
        .iter()
        .take(UNMATCHED_SAMPLE_CAP)
        .cloned()
        .collect();

    CoverageReport {
        total_pairs_planned: result.total_planned,
        matched: result.matched.len(),
        coverage: result.coverage,
        unmatched_by_reason,
        unmatched_sample,
    }
}

/// Write the coverage report to
/// `{out_dir}/ic_eliminations/ic_matching_coverage.json`.
///
/// Creates the `ic_eliminations/` subdirectory if it doesn't exist.
/// Output is pretty-printed JSON with a trailing newline so the file is
/// human-readable when opened in an editor.
///
/// Returns the absolute path of the written file so callers logging
/// "wrote ic_matching_coverage.json to …" don't have to re-derive it.
///
/// # Errors
///
/// - [`GroupError::Io`] if the subdirectory creation, file write, or
///   path resolution fails.
/// - [`GroupError::Serde`] if the report fails to serialise (should be
///   impossible — every field is `Serialize`-friendly).
pub fn write_coverage_report(report: &CoverageReport, out_dir: &Path) -> GroupResult<PathBuf> {
    let dir = out_dir.join(COVERAGE_REPORT_SUBDIR);
    fs::create_dir_all(&dir).map_err(GroupError::Io)?;

    let path = dir.join(COVERAGE_REPORT_FILENAME);

    let mut json = serde_json::to_string_pretty(report)?;
    json.push('\n');
    fs::write(&path, json).map_err(GroupError::Io)?;

    Ok(path)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::ic_matcher::{IcMatchResult, IcMatchedPair, UnmatchedSide};
    use crate::shard::ic_plan::IcRole;
    use chrono::NaiveDate;
    use datasynth_core::models::journal_entry::JournalEntryHeader;
    use datasynth_core::models::{IcPairId, JournalEntry};

    fn empty_match_result() -> IcMatchResult {
        IcMatchResult {
            matched: Vec::new(),
            unmatched: Vec::new(),
            total_planned: 0,
            coverage: 0.0,
        }
    }

    fn dummy_je() -> JournalEntry {
        let header = JournalEntryHeader::new(
            "ENT".to_string(),
            NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        );
        JournalEntry::new(header)
    }

    #[test]
    fn build_from_empty_matches_zero_coverage() {
        let r = build_coverage_report(&empty_match_result());
        assert_eq!(r.total_pairs_planned, 0);
        assert_eq!(r.matched, 0);
        assert_eq!(r.coverage, 0.0);
        assert!(r.unmatched_sample.is_empty());
        assert_eq!(r.unmatched_by_reason.len(), 3);
        assert_eq!(
            r.unmatched_by_reason
                .get(&UnmatchedReason::MissingBuyerSide),
            Some(&0)
        );
        assert_eq!(
            r.unmatched_by_reason
                .get(&UnmatchedReason::MissingSellerSide),
            Some(&0)
        );
        assert_eq!(
            r.unmatched_by_reason
                .get(&UnmatchedReason::AmountDriftAboveTolerance),
            Some(&0)
        );
    }

    #[test]
    fn histogram_keys_always_present_even_when_zero() {
        // Result with one unmatched side using a single reason — the
        // other two reasons must still be present at zero so the
        // schema stays stable.
        let mr = IcMatchResult {
            matched: Vec::new(),
            unmatched: vec![UnmatchedSide {
                pair_id: IcPairId::from_bytes([1; 32]),
                present_role: IcRole::Seller,
                present_entity: "ENT".to_string(),
                present_je: dummy_je(),
                reason: UnmatchedReason::MissingBuyerSide,
            }],
            total_planned: 1,
            coverage: 0.0,
        };
        let r = build_coverage_report(&mr);
        assert_eq!(
            r.unmatched_by_reason
                .get(&UnmatchedReason::MissingBuyerSide),
            Some(&1)
        );
        assert_eq!(
            r.unmatched_by_reason
                .get(&UnmatchedReason::MissingSellerSide),
            Some(&0),
            "MissingSellerSide must be present at 0 for schema stability"
        );
        assert_eq!(
            r.unmatched_by_reason
                .get(&UnmatchedReason::AmountDriftAboveTolerance),
            Some(&0),
            "AmountDriftAboveTolerance must be present at 0 for schema stability"
        );
    }

    #[test]
    fn build_passes_through_matched_count_and_coverage() {
        let je = dummy_je();
        let pair = IcMatchedPair {
            pair_id: IcPairId::from_bytes([2; 32]),
            seller_entity: "S".to_string(),
            buyer_entity: "B".to_string(),
            seller_je: je.clone(),
            buyer_je: je,
        };
        let mr = IcMatchResult {
            matched: vec![pair],
            unmatched: Vec::new(),
            total_planned: 2,
            coverage: 0.5,
        };
        let r = build_coverage_report(&mr);
        assert_eq!(r.total_pairs_planned, 2);
        assert_eq!(r.matched, 1);
        assert_eq!(r.coverage, 0.5);
    }
}

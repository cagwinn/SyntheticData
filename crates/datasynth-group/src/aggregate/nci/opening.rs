//! NCI opening-balance ingestion + writer — Task 7.2.
//!
//! Reads prior-period closing NCI balances from
//! `{prior_period_dir}/consolidated/nci_rollforward.json` (the file
//! emitted by Task 9.1's aggregate driver via [`write_nci_rollforward`])
//! and returns them as the current period's opening balances, mirroring
//! the IFRS 10 / ASC 810 rollforward identity:
//!
//! ```text
//! opening_nci(period N) := closing_nci(period N-1)
//! ```
//!
//! # Standards reference
//!
//! - **IFRS 10.B94 / IFRS 12.12** — the NCI carrying amount must be
//!   rolled forward period-on-period; the prior-period closing balance
//!   *is* the current-period opening balance.
//! - **ASC 810-10-45** — equivalent US GAAP requirement.
//!
//! # File-not-found semantics
//!
//! Missing `consolidated/nci_rollforward.json` is **not** an error — it
//! just means the engagement is in its first period.  We log a
//! `tracing::warn!` (so the audit log captures the choice) and return
//! an empty map.  Callers iterating over their entities default every
//! opening balance to zero.
//!
//! # Corruption / duplicates
//!
//! - Malformed JSON surfaces as [`GroupError::Serde`] (the standard
//!   `From<serde_json::Error>` conversion).
//! - Multiple records for the same `entity_code` surfaces as
//!   [`GroupError::Aggregate`] — silent overwriting would mask a
//!   regression in the rollforward writer.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rust_decimal::Decimal;

use crate::errors::{GroupError, GroupResult};

use super::rollforward::NciRollforward;

/// Subdirectory within the group output root where the consolidated
/// NCI rollforward sits.  Mirrors the spec §"Aggregate phase outputs"
/// layout used by other consolidated artefacts (e.g.
/// `cta_rollforward.json`).
pub const CONSOLIDATED_SUBDIR: &str = "consolidated";

/// File name for the on-disk NCI rollforward array, per spec §"Aggregate
/// phase outputs".
pub const NCI_ROLLFORWARD_FILENAME: &str = "nci_rollforward.json";

// ── Public API ────────────────────────────────────────────────────────────────

/// Read opening NCI balances from a prior period's aggregate output.
///
/// Walks `{prior_period_dir}/consolidated/nci_rollforward.json` (the
/// file emitted by Task 9.1's aggregate driver via
/// [`write_nci_rollforward`]), extracts each entity's `closing_nci`,
/// and returns the `(entity_code, closing_nci)` pairs as the current
/// period's opening balances.
///
/// `Ok(BTreeMap::new())` is returned (with a `tracing::warn!` log) when
/// the file is missing — the spec says "missing file → default to 0
/// per entity, with a warning".  Callers receiving an empty map default
/// every entity's opening NCI to zero.
///
/// # Errors
///
/// - [`GroupError::Serde`] if the file exists but cannot be parsed as a
///   JSON array of [`NciRollforward`].
/// - [`GroupError::Aggregate`] if the file contains two or more
///   records for the same `entity_code` — silent overwriting would
///   mask a regression in the writer.
/// - [`GroupError::Io`] for any I/O failure other than file-not-found
///   (which is converted to an empty map, not an error).
pub fn ingest_opening_nci_balances(
    prior_period_dir: &Path,
) -> GroupResult<BTreeMap<String, Decimal>> {
    let path = prior_period_dir
        .join(CONSOLIDATED_SUBDIR)
        .join(NCI_ROLLFORWARD_FILENAME);

    if !path.exists() {
        tracing::warn!(
            path = %path.display(),
            "opening NCI file not found; defaulting to zero opening balance per entity"
        );
        return Ok(BTreeMap::new());
    }

    let bytes = fs::read(&path).map_err(GroupError::Io)?;
    let rollforwards: Vec<NciRollforward> = serde_json::from_slice(&bytes)?;

    let mut map: BTreeMap<String, Decimal> = BTreeMap::new();
    for rf in rollforwards {
        if map.contains_key(&rf.entity_code) {
            return Err(GroupError::Aggregate(format!(
                "ingest_opening_nci_balances: duplicate entity `{}` in opening \
                 NCI file {} — writer regression?",
                rf.entity_code,
                path.display(),
            )));
        }
        map.insert(rf.entity_code, rf.closing_nci);
    }

    Ok(map)
}

/// Write an array of [`NciRollforward`] records to
/// `{out_dir}/consolidated/nci_rollforward.json`.
///
/// Creates the `consolidated/` subdirectory if it doesn't already
/// exist.  Output is pretty-printed JSON with a trailing newline so
/// the file is human-readable when opened in an editor.  Returns the
/// absolute path of the written file so callers logging "wrote
/// nci_rollforward.json to …" don't have to re-derive it.
///
/// # Errors
///
/// - [`GroupError::Io`] if the subdirectory creation or file write
///   fails.
/// - [`GroupError::Serde`] if the rollforward array fails to serialise
///   (should be impossible — every field is `Serialize`-friendly).
pub fn write_nci_rollforward(
    rollforwards: &[NciRollforward],
    out_dir: &Path,
) -> GroupResult<PathBuf> {
    let dir = out_dir.join(CONSOLIDATED_SUBDIR);
    fs::create_dir_all(&dir).map_err(GroupError::Io)?;

    let path = dir.join(NCI_ROLLFORWARD_FILENAME);

    let mut json = serde_json::to_string_pretty(rollforwards)?;
    json.push('\n');
    fs::write(&path, json).map_err(GroupError::Io)?;

    Ok(path)
}

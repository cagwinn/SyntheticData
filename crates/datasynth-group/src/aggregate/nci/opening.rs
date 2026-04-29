//! NCI opening-balance ingestion + writer — Task 7.2.
//!
//! Ingests prior-period closing NCI balances as the current period's
//! opening balances, and provides the on-disk writer for the
//! `consolidated/nci_rollforward.json` artefact consumed by Task 9.1's
//! aggregate driver.
//!
//! Filled in by Task 7.2 — Task 7.1 only needs the file to exist so the
//! module wiring is complete.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rust_decimal::Decimal;

use crate::errors::GroupResult;

use super::rollforward::NciRollforward;

/// File name for the on-disk NCI rollforward array, per spec §"Aggregate
/// phase outputs".
pub const NCI_ROLLFORWARD_FILENAME: &str = "nci_rollforward.json";

/// **Stub** — Task 7.2 implementation.  Returns an empty map so callers
/// integrating against the public API don't blow up before Task 7.2
/// lands.
pub fn ingest_opening_nci_balances(
    _prior_period_dir: &Path,
) -> GroupResult<BTreeMap<String, Decimal>> {
    Ok(BTreeMap::new())
}

/// **Stub** — Task 7.2 implementation.
pub fn write_nci_rollforward(
    _rollforwards: &[NciRollforward],
    out_dir: &Path,
) -> GroupResult<PathBuf> {
    Ok(out_dir.join("consolidated").join(NCI_ROLLFORWARD_FILENAME))
}

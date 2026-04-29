//! CTA computation + rollforward — Task 6.3.
//!
//! After [`crate::aggregate::translation::translate_entity_tb`] has
//! produced a [`TranslatedTb`] per entity, the aggregate phase summarises
//! the per-entity CTA into a small rollforward record per spec §"IAS 21
//! translation":
//!
//! ```text
//! opening_cta + period_cta = closing_cta
//! ```
//!
//! The rollforward is written as
//! `{out_dir}/consolidated/cta_rollforward.json`. Downstream financial-
//! statement assembly (Chunk 8) and equity-method consolidation (Chunk 7)
//! consume it as input.
//!
//! # Prior-period opening CTA
//!
//! For v5.0 the caller supplies `opening_cta` directly (default 0). Task
//! 7.2 layers in the `--prior-period-aggregate` plumbing that auto-loads
//! prior-period CTA from a previously emitted rollforward file; until
//! that lands, opening CTA is the caller's responsibility.

use std::fs;
use std::path::{Path, PathBuf};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::aggregate::translation::translate::TranslatedTb;
use crate::errors::{GroupError, GroupResult};

/// Subdirectory within the group output root where the consolidated
/// rollforward sits. Mirrors the spec §"Aggregate phase outputs"
/// layout used by other consolidated artefacts.
pub const CONSOLIDATED_SUBDIR: &str = "consolidated";

/// File name for the CTA rollforward, per spec §"IAS 21 translation".
pub const CTA_ROLLFORWARD_FILENAME: &str = "cta_rollforward.json";

// ── Public types ──────────────────────────────────────────────────────────────

/// One entity's CTA rollforward for the period.
///
/// `closing_cta = opening_cta + period_cta` is the IAS 21.39 OCI
/// accumulation identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CtaRollforward {
    /// Entity code (matches the per-entity TB's `company_code`).
    pub entity_code: String,
    /// Functional currency the entity reports in.
    pub functional_currency: String,
    /// Group presentation currency the CTA is denominated in.
    pub presentation_currency: String,
    /// CTA balance carried over from the prior period (zero on first
    /// period of an engagement).
    pub opening_cta: Decimal,
    /// CTA generated this period from the IAS 21 translation residual.
    pub period_cta: Decimal,
    /// Closing CTA = `opening_cta + period_cta`.
    pub closing_cta: Decimal,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Pure CTA computation: `total_translated_debits - total_translated_credits`.
///
/// Mirrors the residual already stored on [`TranslatedTb::cta`]; the
/// helper is exposed for callers that prefer to recompute the residual
/// from the totals (for example, to validate that `cta` and the totals
/// haven't drifted apart).
pub fn compute_cta(translated: &TranslatedTb) -> Decimal {
    translated.total_translated_debits - translated.total_translated_credits
}

/// Build a [`CtaRollforward`] for one entity.
///
/// Pure function: no I/O, no allocation beyond the record itself.
pub fn cta_rollforward(
    entity_code: &str,
    functional_currency: &str,
    presentation_currency: &str,
    opening_cta: Decimal,
    period_cta: Decimal,
) -> CtaRollforward {
    CtaRollforward {
        entity_code: entity_code.to_string(),
        functional_currency: functional_currency.to_string(),
        presentation_currency: presentation_currency.to_string(),
        opening_cta,
        period_cta,
        closing_cta: opening_cta + period_cta,
    }
}

/// Write the rollforward array to
/// `{out_dir}/consolidated/cta_rollforward.json`.
///
/// Output is pretty-printed JSON with a trailing newline so the file is
/// human-readable when opened in an editor.  Returns the absolute path of
/// the written file.
///
/// # Errors
///
/// - [`GroupError::Io`] if the subdirectory creation or file write fails.
/// - [`GroupError::Serde`] if the rollforward fails to serialise (should
///   be impossible — every field is `Serialize`-friendly).
pub fn write_cta_rollforward(
    rollforwards: &[CtaRollforward],
    out_dir: &Path,
) -> GroupResult<PathBuf> {
    let dir = out_dir.join(CONSOLIDATED_SUBDIR);
    fs::create_dir_all(&dir).map_err(GroupError::Io)?;

    let path = dir.join(CTA_ROLLFORWARD_FILENAME);

    let mut json = serde_json::to_string_pretty(rollforwards)?;
    json.push('\n');
    fs::write(&path, json).map_err(GroupError::Io)?;

    Ok(path)
}

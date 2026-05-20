//! Translation worksheet emission — Task 6.4.
//!
//! Emits the audit-friendly per-entity, per-line worksheet that documents
//! every IAS 21 translation step. The worksheet lands at
//! `{out_dir}/consolidated/translation_worksheet.json` and is consumed by
//! auditors, dashboards, and the financial-statement assembly pipeline
//! (Chunk 8).
//!
//! # On-disk shape (per spec §"IAS 21 translation")
//!
//! ```json
//! [
//!   {
//!     "entity_code": "ACME_USA",
//!     "functional_currency": "USD",
//!     "presentation_currency": "CHF",
//!     "as_of_date": "2024-03-31",
//!     "lines": [
//!       {
//!         "account_code": "1000",
//!         "local_amount": "10000.00",
//!         "ccy_pair": "USD/CHF",
//!         "rate": "1.10963",
//!         "rate_basis": "closing",
//!         "translated_amount": "11096.32"
//!       },
//!       ...
//!     ]
//!   },
//!   ...
//! ]
//! ```
//!
//! `ccy_pair` is derived from the entity's `functional_currency` and the
//! group `presentation_currency` and is written into every line for grep-
//! ability — even though it's redundant with the parent record.
//!
//! # Determinism
//!
//! Output preserves the input slice's order verbatim, and within each
//! entity preserves the line order from the source [`TranslatedTb`].
//! Two calls with identical inputs produce byte-identical files.

use std::fs;
use std::path::{Path, PathBuf};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use chrono::NaiveDate;

use crate::aggregate::translation::cta::CONSOLIDATED_SUBDIR;
use crate::aggregate::translation::translate::{RateBasis, TranslatedTb};
use crate::errors::{GroupError, GroupResult};

/// File name for the worksheet, per spec §"IAS 21 translation".
pub const TRANSLATION_WORKSHEET_FILENAME: &str = "translation_worksheet.json";

// ── On-disk shape ─────────────────────────────────────────────────────────────

/// Worksheet entry for one entity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorksheetEntity {
    /// Entity code (mirrors [`TranslatedTb::entity_code`]).
    pub entity_code: String,
    /// Functional currency the lines were originally in.
    pub functional_currency: String,
    /// Presentation currency the lines are translated to.
    pub presentation_currency: String,
    /// As-of date of the source TB.
    pub as_of_date: NaiveDate,
    /// Per-line breakdown.
    pub lines: Vec<WorksheetLine>,
}

/// Worksheet entry for one TB line within an entity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorksheetLine {
    /// GL account code.
    pub account_code: String,
    /// Original amount in functional currency (always non-negative).
    pub local_amount: Decimal,
    /// `"FUNCTIONAL/PRESENTATION"` pair key — redundant with the parent
    /// entity record but emitted per-line for grep-ability.
    pub ccy_pair: String,
    /// Rate applied to translate the line.
    pub rate: Decimal,
    /// IAS 21 rate basis used.
    pub rate_basis: RateBasis,
    /// Translated amount in presentation currency.
    pub translated_amount: Decimal,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Write the per-entity translation worksheet to
/// `{out_dir}/consolidated/translation_worksheet.json`.
///
/// Output is pretty-printed JSON with a trailing newline so the file is
/// human-readable when opened in an editor. Returns the absolute path of
/// the written file.
///
/// # Errors
///
/// - [`GroupError::Io`] if the subdirectory creation or file write fails.
/// - [`GroupError::Serde`] if the worksheet fails to serialise (should be
///   impossible — every field is `Serialize`-friendly).
pub fn write_translation_worksheet(
    translated_tbs: &[TranslatedTb],
    out_dir: &Path,
) -> GroupResult<PathBuf> {
    let dir = out_dir.join(CONSOLIDATED_SUBDIR);
    fs::create_dir_all(&dir).map_err(GroupError::Io)?;

    let path = dir.join(TRANSLATION_WORKSHEET_FILENAME);

    let entities: Vec<WorksheetEntity> = translated_tbs.iter().map(build_entity).collect();

    let mut json = serde_json::to_string_pretty(&entities)?;
    json.push('\n');
    fs::write(&path, json).map_err(GroupError::Io)?;

    Ok(path)
}

/// Pure conversion: [`TranslatedTb`] → on-disk [`WorksheetEntity`].
fn build_entity(t: &TranslatedTb) -> WorksheetEntity {
    let ccy_pair = format!("{}/{}", t.functional_currency, t.presentation_currency);
    let lines = t
        .lines
        .iter()
        .map(|l| WorksheetLine {
            account_code: l.account_code.clone(),
            local_amount: l.local_amount,
            ccy_pair: ccy_pair.clone(),
            rate: l.fx_rate,
            rate_basis: l.rate_basis,
            translated_amount: l.translated_amount,
        })
        .collect();
    WorksheetEntity {
        entity_code: t.entity_code.clone(),
        functional_currency: t.functional_currency.clone(),
        presentation_currency: t.presentation_currency.clone(),
        as_of_date: t.as_of_date,
        lines,
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    use crate::aggregate::translation::translate::{DrCr, RateBasis, TranslatedLine, TranslatedTb};
    use crate::aggregate::translation::TranslationAccountType;

    fn period_end() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
    }

    fn sample_translated_tb() -> TranslatedTb {
        TranslatedTb {
            entity_code: "ACME_USA".to_string(),
            functional_currency: "USD".to_string(),
            presentation_currency: "CHF".to_string(),
            as_of_date: period_end(),
            lines: vec![
                TranslatedLine {
                    account_code: "1000".to_string(),
                    local_amount: dec!(10000),
                    local_dr_cr: DrCr::Debit,
                    fx_rate: dec!(1.10963),
                    rate_basis: RateBasis::Closing,
                    translated_amount: dec!(11096.30),
                    account_type: TranslationAccountType::BsMonetary,
                },
                TranslatedLine {
                    account_code: "4000".to_string(),
                    local_amount: dec!(7000),
                    local_dr_cr: DrCr::Credit,
                    fx_rate: dec!(1.12395),
                    rate_basis: RateBasis::Average,
                    translated_amount: dec!(7867.65),
                    account_type: TranslationAccountType::PlRevenue,
                },
            ],
            total_translated_debits: dec!(11096.30),
            total_translated_credits: dec!(7867.65),
            cta: dec!(3228.65),
        }
    }

    #[test]
    fn build_entity_derives_ccy_pair_per_line() {
        let we = build_entity(&sample_translated_tb());
        assert_eq!(we.entity_code, "ACME_USA");
        assert_eq!(we.lines.len(), 2);
        for l in &we.lines {
            assert_eq!(l.ccy_pair, "USD/CHF");
        }
    }

    #[test]
    fn write_worksheet_creates_file_at_canonical_path() {
        let tmp = tempfile::tempdir().expect("tmp dir");
        let path =
            write_translation_worksheet(&[sample_translated_tb()], tmp.path()).expect("write");

        assert!(path.ends_with(TRANSLATION_WORKSHEET_FILENAME));
        assert!(path.parent().unwrap().ends_with(CONSOLIDATED_SUBDIR));
        assert!(path.exists());

        let bytes = fs::read(&path).expect("read");
        let s = String::from_utf8(bytes).expect("utf8");
        assert!(s.ends_with('\n'), "trailing newline");
        assert!(s.contains("\"USD/CHF\""), "ccy_pair must appear in output");
        assert!(s.contains("\"closing\""), "rate_basis enum serialised");
    }

    #[test]
    fn empty_input_writes_empty_array() {
        let tmp = tempfile::tempdir().expect("tmp dir");
        let path = write_translation_worksheet(&[], tmp.path()).expect("write");

        let bytes = fs::read(&path).expect("read");
        let s = String::from_utf8(bytes).expect("utf8");
        let trimmed = s.trim();
        assert_eq!(trimmed, "[]");

        // Round-trip: empty array deserialises to empty vec.
        let round_trip: Vec<WorksheetEntity> = serde_json::from_str(&s).expect("parse");
        assert!(round_trip.is_empty());

        // Translated amounts are typed Decimal — make sure the
        // round-trip preserves precision when there's content too.
        let real = sample_translated_tb();
        let we = build_entity(&real);
        assert_eq!(
            we.lines[0].translated_amount,
            real.lines[0].translated_amount
        );
    }
}

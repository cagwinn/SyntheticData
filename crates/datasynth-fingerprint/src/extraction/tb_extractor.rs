//! SP4.1 — Extract trial-balance targets from real `TB_XXX.parquet` files.
//!
//! Corpus TB exports carry per-account opening/closing balances for a
//! fiscal year per business unit.  This extractor reads the parquet file,
//! adapts to the schema actually present (column names may vary across clients),
//! and builds a `TbAnchorPrior`.
//!
//! **Observed schema** (from the three largest clients in the current corpus):
//! ```text
//! GL Account Number | Functional Beginning Balance | Functional Ending Balance |
//! Business Unit | Functional Currency Code |
//! Reporting Beginning Balance | Reporting Ending Balance | Reporting Currency Code |
//! Year
//! ```
//!
//! The extractor uses `Functional Beginning Balance` and `Functional Ending Balance`
//! as primary balance columns.  When those are absent it falls back to generic
//! `Opening Balance` / `Closing Balance` / `Beginning Balance` / `Ending Balance` names.
//!
//! Period net activity is derived as `closing − opening` for each account and each
//! business-unit row, then summed to a per-account total.
//!
//! **Asset/liability/equity classification** uses a first-digit heuristic on the
//! GL account number:
//! - Leading digit 1 → Asset (or the most common 0000-1xxx zero-padded format)
//! - Leading digit 2 → Liability
//! - Leading digit 3 → Equity
//! - Leading digit 4 → Revenue (treated as quasi-equity for total-aggregate purposes)
//! - Leading digit 5+ → Expense (omitted from balance-sheet totals)
//!
//! For the zero-padded 10-digit format (`0000xxxxxx`) the effective leading digit
//! is the first non-zero digit.

use std::collections::BTreeMap;
use std::path::Path;

use arrow::array::{Array, Float64Array, Int64Array, LargeStringArray, StringArray};
use arrow::compute::cast;
use arrow::datatypes::DataType;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use datasynth_core::distributions::behavioral_priors::{TbAnchorPrior, TbTarget};

use crate::error::{FingerprintError, FingerprintResult};

// ---------------------------------------------------------------------------
// Column-name candidates (all lower-cased for matching)
// ---------------------------------------------------------------------------

const ACCOUNT_NUMBER_CANDIDATES: &[&str] = &[
    "gl account number",
    "gl_account_number",
    "gl account",
    "gl_account",
    "account_number",
    "account number",
    "account_no",
    "saknr",
    "c",
];

const OPENING_BALANCE_CANDIDATES: &[&str] = &[
    "functional beginning balance",
    "functional_beginning_balance",
    "beginning balance",
    "beginning_balance",
    "opening balance",
    "opening_balance",
    "start balance",
    "start_balance",
];

const CLOSING_BALANCE_CANDIDATES: &[&str] = &[
    "functional ending balance",
    "functional_ending_balance",
    "ending balance",
    "ending_balance",
    "closing balance",
    "closing_balance",
    "end balance",
    "end_balance",
];

// ---------------------------------------------------------------------------
// Main extraction function
// ---------------------------------------------------------------------------

/// Extract TB anchor from a single `TB_XXX.parquet` file.
///
/// Returns a `TbAnchorPrior` with one `TbTarget` per unique GL account number.
/// When an account appears in multiple rows (multiple business units), balances
/// are summed across rows so the result represents the entity-total balance.
///
/// For single-client extraction the `opening_stdev` / `closing_stdev` fields
/// are left at `0.0` — cross-client stdevs are computed in `aggregate_tb_anchor`.
pub fn extract_tb_anchor_from_parquet(path: &Path) -> FingerprintResult<TbAnchorPrior> {
    let file = std::fs::File::open(path).map_err(|e| {
        FingerprintError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("TB parquet open failed: {e}"),
        ))
    })?;

    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
        FingerprintError::InvalidFormat(format!("TB parquet: cannot build reader: {e}"))
    })?;

    let reader = builder.build().map_err(|e| {
        FingerprintError::InvalidFormat(format!("TB parquet: cannot open reader: {e}"))
    })?;

    // Accumulate per-account totals across all rows and batches.
    // (opening_sum, closing_sum, row_count)
    let mut acc: BTreeMap<String, (f64, f64, usize)> = BTreeMap::new();

    for batch_res in reader {
        let batch = batch_res.map_err(|e| {
            FingerprintError::InvalidFormat(format!("TB parquet: batch read error: {e}"))
        })?;

        let schema = batch.schema();
        let col_names: Vec<String> = schema
            .fields()
            .iter()
            .map(|f| f.name().to_lowercase())
            .collect();

        // Resolve column indices once per batch.
        let acct_idx = find_column_index(&col_names, ACCOUNT_NUMBER_CANDIDATES);
        let open_idx = find_column_index(&col_names, OPENING_BALANCE_CANDIDATES);
        let close_idx = find_column_index(&col_names, CLOSING_BALANCE_CANDIDATES);

        let Some(acct_col_idx) = acct_idx else {
            tracing::warn!(
                "extract_tb_anchor_from_parquet: no account-number column found in {:?}; columns={:?}",
                path,
                col_names
            );
            continue;
        };

        let n_rows = batch.num_rows();
        let acct_arr = string_column(&batch, acct_col_idx);
        let open_arr: Option<Vec<Option<f64>>> = open_idx.map(|i| float64_column(&batch, i));
        let close_arr: Option<Vec<Option<f64>>> = close_idx.map(|i| float64_column(&batch, i));

        for row in 0..n_rows {
            // Extract account number.
            let account_number = match &acct_arr {
                Some(arr) => {
                    if arr.is_null(row) {
                        continue;
                    }
                    let v = arr.value(row).trim().to_string();
                    if v.is_empty() {
                        continue;
                    }
                    v
                }
                None => continue,
            };

            let opening = open_arr.as_ref().and_then(|arr| arr[row]).unwrap_or(0.0);
            let closing = close_arr.as_ref().and_then(|arr| arr[row]).unwrap_or(0.0);

            // Sum across business units / years for the same account number.
            let entry = acc.entry(account_number).or_insert((0.0, 0.0, 0));
            entry.0 += opening;
            entry.1 += closing;
            entry.2 += 1;
        }
    }

    if acc.is_empty() {
        tracing::warn!(
            "extract_tb_anchor_from_parquet: no rows extracted from {:?}",
            path
        );
        return Ok(TbAnchorPrior::default());
    }

    // Build TbTarget per account and aggregate balance-sheet totals.
    let mut per_account: BTreeMap<String, TbTarget> = BTreeMap::new();
    let mut total_assets = 0.0_f64;
    let mut total_liabilities = 0.0_f64;
    let mut total_equity = 0.0_f64;

    for (account_number, (opening_sum, closing_sum, _n_rows)) in &acc {
        let net_activity = closing_sum - opening_sum;

        per_account.insert(
            account_number.clone(),
            TbTarget {
                opening_balance: *opening_sum,
                closing_balance: *closing_sum,
                period_net_activity: net_activity,
                opening_stdev: 0.0, // populated during cross-client aggregation
                closing_stdev: 0.0,
                n_clients: 1,
            },
        );

        // Classify account for balance-sheet totals using first-digit heuristic.
        match classify_account_type(account_number) {
            AccountClass::Asset => total_assets += closing_sum,
            AccountClass::Liability => total_liabilities += closing_sum.abs(),
            AccountClass::Equity => total_equity += closing_sum.abs(),
            AccountClass::Revenue | AccountClass::Expense | AccountClass::Unknown => {}
        }
    }

    Ok(TbAnchorPrior {
        per_account,
        total_assets,
        total_liabilities,
        total_equity,
        n_clients: 1,
    })
}

// ---------------------------------------------------------------------------
// Account classification heuristic
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccountClass {
    Asset,
    Liability,
    Equity,
    Revenue,
    Expense,
    Unknown,
}

/// Classify a GL account number into a balance-sheet category using a
/// first-digit heuristic.  Handles both plain numeric (e.g. "1000") and
/// zero-padded 10-digit SAP format (e.g. "0000100100").
fn classify_account_type(account: &str) -> AccountClass {
    // Strip leading zeros to find the effective first digit.
    let effective = account.trim_start_matches('0');
    let first_digit = effective.chars().next().unwrap_or('0');
    match first_digit {
        '1' => AccountClass::Asset,
        '2' => AccountClass::Liability,
        '3' => AccountClass::Equity,
        '4' => AccountClass::Revenue,
        '5' | '6' | '7' | '8' | '9' => AccountClass::Expense,
        _ => AccountClass::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Helpers — column index resolution and type casting
// ---------------------------------------------------------------------------

/// Find the first column whose lower-cased name exactly matches any candidate.
fn find_column_index(col_names: &[String], candidates: &[&str]) -> Option<usize> {
    for &candidate in candidates {
        if let Some(idx) = col_names.iter().position(|n| n == candidate) {
            return Some(idx);
        }
    }
    None
}

/// Extract a string column as a `StringArray`, handling Utf8, LargeUtf8, and
/// Dictionary-encoded variants.
fn string_column(batch: &arrow::record_batch::RecordBatch, col_idx: usize) -> Option<StringArray> {
    let col = batch.column(col_idx);
    match col.data_type() {
        DataType::Utf8 => col.as_any().downcast_ref::<StringArray>().cloned(),
        DataType::LargeUtf8 => {
            let large = col.as_any().downcast_ref::<LargeStringArray>()?;
            let values: Vec<Option<&str>> = (0..large.len())
                .map(|i| {
                    if large.is_null(i) {
                        None
                    } else {
                        Some(large.value(i))
                    }
                })
                .collect();
            Some(StringArray::from(values))
        }
        DataType::Dictionary(_, _) => {
            let utf8 = cast(col.as_ref(), &DataType::Utf8).ok()?;
            utf8.as_any().downcast_ref::<StringArray>().cloned()
        }
        _ => None,
    }
}

/// Extract a numeric column as a `Vec<Option<f64>>`, handling Float64, Float32,
/// Int64, Int32, and other numeric types by casting.
fn float64_column(batch: &arrow::record_batch::RecordBatch, col_idx: usize) -> Vec<Option<f64>> {
    let col = batch.column(col_idx);
    let n = col.len();

    match col.data_type() {
        DataType::Float64 => {
            let arr = col.as_any().downcast_ref::<Float64Array>();
            if let Some(arr) = arr {
                return (0..n)
                    .map(|i| {
                        if arr.is_null(i) {
                            None
                        } else {
                            Some(arr.value(i))
                        }
                    })
                    .collect();
            }
            vec![None; n]
        }
        DataType::Int64 => {
            let arr = col.as_any().downcast_ref::<Int64Array>();
            if let Some(arr) = arr {
                return (0..n)
                    .map(|i| {
                        if arr.is_null(i) {
                            None
                        } else {
                            Some(arr.value(i) as f64)
                        }
                    })
                    .collect();
            }
            vec![None; n]
        }
        _ => {
            // Try casting to Float64 for other numeric types.
            if let Ok(float_col) = cast(col.as_ref(), &DataType::Float64) {
                let arr = float_col.as_any().downcast_ref::<Float64Array>();
                if let Some(arr) = arr {
                    return (0..n)
                        .map(|i| {
                            if arr.is_null(i) {
                                None
                            } else {
                                Some(arr.value(i))
                            }
                        })
                        .collect();
                }
            }
            vec![None; n]
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_account_type_plain_numeric() {
        assert_eq!(classify_account_type("1000"), AccountClass::Asset);
        assert_eq!(classify_account_type("2000"), AccountClass::Liability);
        assert_eq!(classify_account_type("3000"), AccountClass::Equity);
        assert_eq!(classify_account_type("4000"), AccountClass::Revenue);
        assert_eq!(classify_account_type("5000"), AccountClass::Expense);
        assert_eq!(classify_account_type("6100"), AccountClass::Expense);
    }

    #[test]
    fn classify_account_type_zero_padded() {
        // 0000100100 → leading zeros stripped → "100100" → first digit '1' → Asset
        assert_eq!(classify_account_type("0000100100"), AccountClass::Asset);
        // 0000200001 → leading zeros stripped → "200001" → '2' → Liability
        assert_eq!(classify_account_type("0000200001"), AccountClass::Liability);
        // 0000300000 → '3' → Equity
        assert_eq!(classify_account_type("0000300000"), AccountClass::Equity);
        // 0000400000 → '4' → Revenue
        assert_eq!(classify_account_type("0000400000"), AccountClass::Revenue);
        // 0000500000 → '5' → Expense
        assert_eq!(classify_account_type("0000500000"), AccountClass::Expense);
    }

    #[test]
    fn find_column_index_exact_match() {
        let cols = vec![
            "gl account number".to_string(),
            "functional beginning balance".to_string(),
            "functional ending balance".to_string(),
        ];
        assert_eq!(find_column_index(&cols, ACCOUNT_NUMBER_CANDIDATES), Some(0));
        assert_eq!(
            find_column_index(&cols, OPENING_BALANCE_CANDIDATES),
            Some(1)
        );
        assert_eq!(
            find_column_index(&cols, CLOSING_BALANCE_CANDIDATES),
            Some(2)
        );
    }

    #[test]
    fn find_column_index_missing_returns_none() {
        let cols = vec!["something_else".to_string()];
        assert!(find_column_index(&cols, ACCOUNT_NUMBER_CANDIDATES).is_none());
    }
}

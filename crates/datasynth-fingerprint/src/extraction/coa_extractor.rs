//! SP4.2 — Extract CoA semantic content from corpus COA_XXX.parquet files.
//!
//! Corpus exports carry account number → description → account_type →
//! optional hierarchy columns.  This extractor reads the parquet file, adapts
//! to whatever columns are actually present (best-effort string-match against
//! common SAP / generic export column names), and builds a `CoaSemanticPrior`.
//!
//! Column mapping (case-insensitive prefix match):
//! - Account number : first column that is "c", "account", "gl_account",
//!   "account_number", "account_no", "gl account", "glaccount"
//! - Description    : "gl account name", "gl_account_name", "description",
//!   "account_name", "name"
//! - Account type   : "account type", "account_type", "acct_type"
//! - Account class  : "account class", "account_class"
//! - Sub type       : "account sub type", "account_sub_type", "sub_type",
//!   "subtype"
//! - Sub class      : "account sub class", "account_sub_class", "sub_class"

use std::collections::BTreeMap;
use std::path::Path;

use arrow::array::{Array, LargeStringArray, StringArray};
use arrow::compute::cast;
use arrow::datatypes::DataType;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use datasynth_core::distributions::behavioral_priors::{AccountSemantic, CoaSemanticPrior};

use crate::error::{FingerprintError, FingerprintResult};

// ---------------------------------------------------------------------------
// Column-name candidates (all lower-cased for matching)
// ---------------------------------------------------------------------------

const ACCOUNT_NUMBER_CANDIDATES: &[&str] = &[
    "c",
    "account",
    "gl_account",
    "account_number",
    "account_no",
    "glaccount",
    "gl account",
    "saknr",
];

const DESCRIPTION_CANDIDATES: &[&str] = &[
    "gl account name",
    "gl_account_name",
    "description",
    "account_name",
    "account_description",
    "name",
    "text",
    "txt50",
];

const ACCOUNT_TYPE_CANDIDATES: &[&str] =
    &["account type", "account_type", "acct_type", "type", "ktoks"];

const ACCOUNT_CLASS_CANDIDATES: &[&str] = &["account class", "account_class", "class"];

const ACCOUNT_SUB_TYPE_CANDIDATES: &[&str] = &[
    "account sub type",
    "account_sub_type",
    "sub_type",
    "subtype",
];

const ACCOUNT_SUB_CLASS_CANDIDATES: &[&str] = &[
    "account sub class",
    "account_sub_class",
    "sub_class",
    "subclass",
];

// ---------------------------------------------------------------------------
// Main extraction function
// ---------------------------------------------------------------------------

/// Extract semantic CoA content from a single COA_XXX.parquet file.
///
/// Iterates rows, builds `BTreeMap<account_number, AccountSemantic>`.
/// Rows with empty/null account numbers are skipped.
/// When multiple rows share the same account number the first one wins
/// (aggregation across clients happens in `aggregate_coa_semantic`).
pub fn extract_coa_semantic_from_parquet(path: &Path) -> FingerprintResult<CoaSemanticPrior> {
    let file = std::fs::File::open(path).map_err(|e| {
        FingerprintError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("COA parquet open failed: {e}"),
        ))
    })?;

    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
        FingerprintError::InvalidFormat(format!("COA parquet: cannot build reader: {e}"))
    })?;

    let reader = builder.build().map_err(|e| {
        FingerprintError::InvalidFormat(format!("COA parquet: cannot open reader: {e}"))
    })?;

    let mut accounts: BTreeMap<String, AccountSemantic> = BTreeMap::new();

    for batch_res in reader {
        let batch = batch_res.map_err(|e| {
            FingerprintError::InvalidFormat(format!("COA parquet: batch read error: {e}"))
        })?;

        let schema = batch.schema();
        let col_names: Vec<String> = schema
            .fields()
            .iter()
            .map(|f| f.name().to_lowercase())
            .collect();

        // Resolve column indices once per batch (schema doesn't change).
        let acct_idx = find_column_index(&col_names, ACCOUNT_NUMBER_CANDIDATES);
        let desc_idx = find_column_index(&col_names, DESCRIPTION_CANDIDATES);
        let type_idx = find_column_index(&col_names, ACCOUNT_TYPE_CANDIDATES);
        let class_idx = find_column_index(&col_names, ACCOUNT_CLASS_CANDIDATES);
        let sub_type_idx = find_column_index(&col_names, ACCOUNT_SUB_TYPE_CANDIDATES);
        let sub_class_idx = find_column_index(&col_names, ACCOUNT_SUB_CLASS_CANDIDATES);

        let Some(acct_col) = acct_idx else {
            // No account-number column found — skip this batch (defensive).
            tracing::warn!(
                "extract_coa_semantic_from_parquet: no account-number column found in {:?}; columns={:?}",
                path,
                col_names
            );
            continue;
        };

        let n_rows = batch.num_rows();
        let acct_arr: Option<StringArray> = string_column(&batch, acct_col);
        let desc_arr: Option<StringArray> = desc_idx.and_then(|i| string_column(&batch, i));
        let type_arr: Option<StringArray> = type_idx.and_then(|i| string_column(&batch, i));
        let class_arr: Option<StringArray> = class_idx.and_then(|i| string_column(&batch, i));
        let sub_type_arr: Option<StringArray> = sub_type_idx.and_then(|i| string_column(&batch, i));
        let sub_class_arr: Option<StringArray> =
            sub_class_idx.and_then(|i| string_column(&batch, i));

        for row in 0..n_rows {
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

            // Skip duplicate account numbers (first occurrence wins within client).
            if accounts.contains_key(&account_number) {
                continue;
            }

            let description = desc_arr
                .as_ref()
                .map(|arr| {
                    if arr.is_null(row) {
                        String::new()
                    } else {
                        arr.value(row).trim().to_string()
                    }
                })
                .unwrap_or_default();

            let account_type = opt_string(&type_arr, row);
            let account_class = opt_string(&class_arr, row);
            // Use "account sub type" as `account_class_name` — it is a
            // human-readable label for the class in most SAP exports we've seen.
            let account_class_name = opt_string(&sub_type_arr, row);
            let account_sub_class = opt_string(&sub_class_arr, row);
            // Sub-class and its name are often the same column in real exports;
            // for now leave `account_sub_class_name` as a copy of the sub-class value.
            let account_sub_class_name = account_sub_class.clone();

            accounts.insert(
                account_number,
                AccountSemantic {
                    description,
                    account_type,
                    account_class,
                    account_class_name,
                    account_sub_class,
                    account_sub_class_name,
                    parent_account: None,
                },
            );
        }
    }

    Ok(CoaSemanticPrior { accounts })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find the first column whose lower-cased name matches any candidate (exact).
fn find_column_index(col_names: &[String], candidates: &[&str]) -> Option<usize> {
    for &candidate in candidates {
        if let Some(idx) = col_names.iter().position(|n| n == candidate) {
            return Some(idx);
        }
    }
    None
}

/// Extract a column as `StringArray` if the column is a string-type.
/// Returns `None` if the column doesn't exist or isn't a string type.
fn string_column(batch: &arrow::record_batch::RecordBatch, col_idx: usize) -> Option<StringArray> {
    let col = batch.column(col_idx);
    match col.data_type() {
        DataType::Utf8 => {
            let arr = col.as_any().downcast_ref::<StringArray>()?.clone();
            Some(arr)
        }
        DataType::LargeUtf8 => {
            // Downcast to LargeStringArray and convert each value.
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
            let arr = StringArray::from(values);
            Some(arr)
        }
        // Dictionary<Int32, Utf8> is common in parquet.
        DataType::Dictionary(_, _) => {
            let utf8 = cast(col.as_ref(), &DataType::Utf8).ok()?;
            let arr = utf8.as_any().downcast_ref::<StringArray>()?.clone();
            Some(arr)
        }
        _ => None,
    }
}

/// Get an optional non-empty string value from a column at a given row.
fn opt_string(col: &Option<StringArray>, row: usize) -> Option<String> {
    let arr = col.as_ref()?;
    if arr.is_null(row) {
        return None;
    }
    let v = arr.value(row).trim().to_string();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use datasynth_core::distributions::behavioral_priors::CoaSemanticPrior;

    /// Unit test for `aggregate_coa_semantic` — merging two synthetic priors.
    #[test]
    fn aggregate_coa_semantic_merges_disjoint_accounts() {
        use crate::aggregation::industry_aggregator::aggregate_coa_semantic;

        let mut a = CoaSemanticPrior::default();
        a.accounts.insert(
            "1000".to_string(),
            AccountSemantic {
                description: "Cash".to_string(),
                account_type: Some("Assets".to_string()),
                ..Default::default()
            },
        );

        let mut b = CoaSemanticPrior::default();
        b.accounts.insert(
            "2000".to_string(),
            AccountSemantic {
                description: "AP Control".to_string(),
                account_type: Some("Liabilities".to_string()),
                ..Default::default()
            },
        );

        let merged = aggregate_coa_semantic(&[&a, &b]);
        assert_eq!(merged.accounts.len(), 2);
        assert_eq!(merged.accounts["1000"].description, "Cash");
        assert_eq!(merged.accounts["2000"].description, "AP Control");
    }

    /// When two clients describe the same account with different descriptions,
    /// the most frequent one wins.
    #[test]
    fn aggregate_coa_semantic_picks_mode_description() {
        use crate::aggregation::industry_aggregator::aggregate_coa_semantic;

        let make_prior = |desc: &str| {
            let mut p = CoaSemanticPrior::default();
            p.accounts.insert(
                "1000".to_string(),
                AccountSemantic {
                    description: desc.to_string(),
                    ..Default::default()
                },
            );
            p
        };

        let a = make_prior("Kasse");
        let b = make_prior("Kasse");
        let c = make_prior("Petty Cash");

        let merged = aggregate_coa_semantic(&[&a, &b, &c]);
        // "Kasse" appears twice, "Petty Cash" once — mode is "Kasse"
        assert_eq!(merged.accounts["1000"].description, "Kasse");
    }

    /// Optional fields are preserved from the first client that provides them.
    #[test]
    fn aggregate_coa_semantic_preserves_optional_fields() {
        use crate::aggregation::industry_aggregator::aggregate_coa_semantic;

        let mut a = CoaSemanticPrior::default();
        a.accounts.insert(
            "3000".to_string(),
            AccountSemantic {
                description: "Revenue".to_string(),
                account_type: Some("Revenue".to_string()),
                account_class: Some("R _ Revenue".to_string()),
                ..Default::default()
            },
        );

        let mut b = CoaSemanticPrior::default();
        b.accounts.insert(
            "3000".to_string(),
            AccountSemantic {
                description: "Revenue".to_string(),
                // No account_type — should use the one from `a`
                ..Default::default()
            },
        );

        let merged = aggregate_coa_semantic(&[&a, &b]);
        let sem = &merged.accounts["3000"];
        assert_eq!(sem.account_type.as_deref(), Some("Revenue"));
        assert_eq!(sem.account_class.as_deref(), Some("R _ Revenue"));
    }
}

//! Parquet / CSV loader producing canonical `Record`s.

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use arrow::array::{
    Array, Date32Array, Float64Array, StringArray, TimestampMicrosecondArray,
    TimestampMillisecondArray, TimestampNanosecondArray,
};
use arrow::record_batch::RecordBatch;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use super::entity_profile::{reference_corpus_aliases, synthetic_aliases};
use super::error::{BehavioralFidelityError, BehavioralFidelityResult};
use super::types::Record;

/// Load all records from a parquet file (or single-file directory).
pub fn load_parquet_records(path: &Path) -> BehavioralFidelityResult<Vec<Record>> {
    let path = resolve_single_file(path, "parquet")?;
    let file = File::open(&path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| BehavioralFidelityError::Parquet(e.to_string()))?;
    let reader = builder
        .build()
        .map_err(|e| BehavioralFidelityError::Parquet(e.to_string()))?;
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for batch_res in reader {
        let batch = batch_res.map_err(|e| BehavioralFidelityError::Parquet(e.to_string()))?;
        let aliases = pick_alias_map(&batch);
        for row in 0..batch.num_rows() {
            match extract_row(&batch, row, &aliases) {
                Ok(rec) => out.push(rec),
                Err(BehavioralFidelityError::Schema(_)) => skipped += 1,
                Err(e) => return Err(e),
            }
        }
    }
    if skipped > 0 {
        tracing::warn!(
            "load_parquet_records: skipped {} rows with missing/malformed required dates",
            skipped
        );
    }
    Ok(out)
}

/// Load all records from a CSV file. Header line is required and is used
/// to pick the alias map.
pub fn load_csv_records(path: &Path) -> BehavioralFidelityResult<Vec<Record>> {
    let path = resolve_single_file(path, "csv")?;
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(&path)
        .map_err(|e| BehavioralFidelityError::Io(std::io::Error::other(e.to_string())))?;
    let headers: Vec<String> = rdr
        .headers()
        .map_err(|e| BehavioralFidelityError::Io(std::io::Error::other(e.to_string())))?
        .iter()
        .map(|s| s.to_string())
        .collect();
    let aliases = pick_alias_map_from_headers(&headers);
    let header_idx: HashMap<&str, usize> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| (h.as_str(), i))
        .collect();
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for rec in rdr.records() {
        let rec =
            rec.map_err(|e| BehavioralFidelityError::Io(std::io::Error::other(e.to_string())))?;
        match extract_csv_row(&rec, &header_idx, &aliases) {
            Ok(record) => out.push(record),
            Err(BehavioralFidelityError::Schema(_)) => skipped += 1,
            Err(e) => return Err(e),
        }
    }
    if skipped > 0 {
        tracing::warn!(
            "load_csv_records: skipped {} rows with missing/malformed required dates",
            skipped
        );
    }
    Ok(out)
}

fn resolve_single_file(
    path: &Path,
    extension: &str,
) -> BehavioralFidelityResult<std::path::PathBuf> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if entry
                .path()
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(extension))
            {
                return Ok(entry.path());
            }
        }
    }
    Err(BehavioralFidelityError::Io(std::io::Error::other(format!(
        "no {extension} file at {}",
        path.display()
    ))))
}

fn pick_alias_map(batch: &RecordBatch) -> HashMap<&'static str, &'static str> {
    let schema = batch.schema();
    let cols: Vec<String> = schema
        .fields()
        .iter()
        .map(|f| f.name().to_string())
        .collect();
    pick_alias_map_from_headers(&cols)
}

fn pick_alias_map_from_headers(cols: &[String]) -> HashMap<&'static str, &'static str> {
    let has = |needle: &str| cols.iter().any(|c| c == needle);
    if has("Tarding Partner") || has("Functional Amount") {
        reference_corpus_aliases().into_iter().collect()
    } else {
        synthetic_aliases().into_iter().collect()
    }
}

fn extract_row(
    batch: &RecordBatch,
    row: usize,
    aliases: &HashMap<&'static str, &'static str>,
) -> BehavioralFidelityResult<Record> {
    let s = batch.schema();
    let col_idx = |canon: &str| -> Option<usize> {
        let real = aliases.get(canon)?;
        s.fields().iter().position(|f| f.name() == *real)
    };
    let str_at = |canon: &str| -> Option<String> {
        let i = col_idx(canon)?;
        let arr = batch.column(i).as_any().downcast_ref::<StringArray>()?;
        if arr.is_null(row) {
            None
        } else {
            Some(arr.value(row).to_string())
        }
    };
    let f64_at = |canon: &str| -> Option<f64> {
        let i = col_idx(canon)?;
        let arr = batch.column(i).as_any().downcast_ref::<Float64Array>()?;
        if arr.is_null(row) {
            None
        } else {
            Some(arr.value(row))
        }
    };
    let date_at = |canon: &str| -> Option<NaiveDate> {
        let i = col_idx(canon)?;
        if let Some(arr) = batch.column(i).as_any().downcast_ref::<StringArray>() {
            if arr.is_null(row) {
                return None;
            }
            return NaiveDate::parse_from_str(arr.value(row), "%Y-%m-%d").ok();
        }
        if let Some(arr) = batch.column(i).as_any().downcast_ref::<Date32Array>() {
            if arr.is_null(row) {
                return None;
            }
            return arr.value_as_date(row);
        }
        None
    };
    let ts_at = |canon: &str| -> Option<DateTime<Utc>> {
        let i = col_idx(canon)?;
        if let Some(arr) = batch
            .column(i)
            .as_any()
            .downcast_ref::<TimestampMillisecondArray>()
        {
            if arr.is_null(row) {
                return None;
            }
            return Utc.timestamp_millis_opt(arr.value(row)).single();
        }
        if let Some(arr) = batch
            .column(i)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
        {
            if arr.is_null(row) {
                return None;
            }
            return Utc.timestamp_micros(arr.value(row)).single();
        }
        if let Some(arr) = batch
            .column(i)
            .as_any()
            .downcast_ref::<TimestampNanosecondArray>()
        {
            if arr.is_null(row) {
                return None;
            }
            let nanos = arr.value(row);
            return Some(Utc.timestamp_nanos(nanos));
        }
        if let Some(arr) = batch.column(i).as_any().downcast_ref::<StringArray>() {
            if arr.is_null(row) {
                return None;
            }
            return DateTime::parse_from_rfc3339(arr.value(row))
                .ok()
                .map(|dt| dt.with_timezone(&Utc));
        }
        None
    };

    Ok(Record {
        source: str_at("Source").unwrap_or_default(),
        gl_account: str_at("GLAccount").unwrap_or_default(),
        cost_center: str_at("CostCenter"),
        profit_center: str_at("ProfitCenter"),
        trading_partner: str_at("TradingPartner"),
        je_number: str_at("JENumber").unwrap_or_default(),
        je_line_number: str_at("JELineNumber").unwrap_or_default(),
        effective_date: date_at("EffectiveDate")
            .ok_or_else(|| BehavioralFidelityError::Schema("missing EffectiveDate".into()))?,
        entry_date: date_at("EntryDate")
            .ok_or_else(|| BehavioralFidelityError::Schema("missing EntryDate".into()))?,
        created_at: ts_at("CreatedAt"),
        functional_amount: f64_at("FunctionalAmount").unwrap_or(0.0),
        // SP4.4 W7.3 — header/line text from JE Description / JE Line Description columns.
        header_text: str_at("HeaderText").unwrap_or_default(),
        line_text: str_at("LineText").unwrap_or_default(),
    })
}

fn extract_csv_row(
    rec: &csv::StringRecord,
    headers: &HashMap<&str, usize>,
    aliases: &HashMap<&'static str, &'static str>,
) -> BehavioralFidelityResult<Record> {
    let get = |canon: &str| -> Option<String> {
        let real = aliases.get(canon)?;
        let i = headers.get(*real)?;
        let v = rec.get(*i)?;
        if v.is_empty() {
            None
        } else {
            Some(v.to_string())
        }
    };
    Ok(Record {
        source: get("Source").unwrap_or_default(),
        gl_account: get("GLAccount").unwrap_or_default(),
        cost_center: get("CostCenter"),
        profit_center: get("ProfitCenter"),
        trading_partner: get("TradingPartner"),
        je_number: get("JENumber").unwrap_or_default(),
        je_line_number: get("JELineNumber").unwrap_or_default(),
        effective_date: get("EffectiveDate")
            .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
            .ok_or_else(|| BehavioralFidelityError::Schema("missing EffectiveDate".into()))?,
        entry_date: get("EntryDate")
            .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
            .ok_or_else(|| BehavioralFidelityError::Schema("missing EntryDate".into()))?,
        created_at: get("CreatedAt").and_then(|s| {
            DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        }),
        functional_amount: get("FunctionalAmount")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0),
        // SP4.4 W7.3 — header/line text from JE Description / JE Line Description columns.
        header_text: get("HeaderText").unwrap_or_default(),
        line_text: get("LineText").unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use arrow::array::{Float64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use parquet::arrow::ArrowWriter;
    use tempfile::NamedTempFile;

    fn build_reference_corpus_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("JE Number", DataType::Utf8, false),
            Field::new("GL Account Number", DataType::Utf8, false),
            Field::new("Functional Amount", DataType::Float64, false),
            Field::new("Effective Date", DataType::Utf8, false),
            Field::new("Entry Date", DataType::Utf8, false),
            Field::new("Source", DataType::Utf8, false),
            Field::new("Cost Center", DataType::Utf8, true),
            Field::new("Profit Center", DataType::Utf8, true),
            Field::new("Tarding Partner", DataType::Utf8, true),
            Field::new("JE Line Number", DataType::Utf8, false),
        ]));
        let arr_je = StringArray::from(vec!["2022-0090-001", "2022-0090-001"]);
        let arr_gl = StringArray::from(vec!["1100", "2000"]);
        let arr_amt = Float64Array::from(vec![100.0, -100.0]);
        let arr_eff = StringArray::from(vec!["2022-04-25", "2022-04-25"]);
        let arr_ent = StringArray::from(vec!["2022-04-14", "2022-04-14"]);
        let arr_src = StringArray::from(vec!["KR", "KR"]);
        let arr_cc = StringArray::from(vec![Some("CC100"), None]);
        let arr_pc = StringArray::from(vec![Some("PC100"), None]);
        let arr_tp = StringArray::from(vec![Some("TP1"), None]);
        let arr_line = StringArray::from(vec!["001", "002"]);
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(arr_je),
                Arc::new(arr_gl),
                Arc::new(arr_amt),
                Arc::new(arr_eff),
                Arc::new(arr_ent),
                Arc::new(arr_src),
                Arc::new(arr_cc),
                Arc::new(arr_pc),
                Arc::new(arr_tp),
                Arc::new(arr_line),
            ],
        )
        .unwrap()
    }

    #[test]
    fn load_parquet_reference_corpus_shape() {
        let batch = build_reference_corpus_batch();
        let tmp = NamedTempFile::new().unwrap();
        let parquet_path = tmp.path().with_extension("parquet");
        {
            let file = File::create(&parquet_path).unwrap();
            let mut writer = ArrowWriter::try_new(file, batch.schema(), None).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }
        let records = load_parquet_records(&parquet_path).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].source, "KR");
        assert_eq!(records[0].cost_center.as_deref(), Some("CC100"));
        assert_eq!(records[1].cost_center, None);
        assert_eq!(
            records[0].entry_date,
            NaiveDate::from_ymd_opt(2022, 4, 14).unwrap()
        );
        let _ = std::fs::remove_file(&parquet_path);
    }
}

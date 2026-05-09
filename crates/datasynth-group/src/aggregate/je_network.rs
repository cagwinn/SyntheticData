//! v5.10 — group-aware Method-A je_network export.
//!
//! Emits two artefacts during the aggregate phase:
//!
//! 1. **Per-entity** `entities/{code}/graphs/je_network.{csv,parquet}`
//!    — the same Method-A edges the single-entity output_writer would
//!    produce, extended with `ic_pair_id` + `ic_partner_entity` columns
//!    so consumers can join the IC postings back into pairs.
//!
//! 2. **Consolidated** `consolidated/je_network.{csv,parquet}` — every
//!    entity's edges concatenated, plus the elimination JEs (flagged
//!    `is_eliminated=true`), with `entity_code` as a partition column.
//!
//! Method-A semantics — *one edge per 2-line journal entry,
//! confidence = 1.0* — are inherited verbatim from
//! [`datasynth_runtime::je_network::build_je_network_edges`].  IC pairs
//! naturally produce two edges (one seller-side, one buyer-side) that
//! share the same `ic_pair_id`; the consolidated file is therefore a
//! straight concat of the per-entity edges plus the eliminations.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::Arc;

use arrow::array::{ArrayRef, BooleanArray, Float64Array, RecordBatch, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use datasynth_config::JeNetworkMethod;
use datasynth_core::models::JournalEntry;
use datasynth_runtime::je_network::{build_je_network_edges, JeNetworkEdge};
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;
use rust_decimal::Decimal;

use crate::errors::{GroupError, GroupResult};

/// Summary statistics returned by [`write_je_network_artefacts`].
#[derive(Debug, Clone, Default)]
pub struct JeNetworkSummary {
    pub per_entity_edge_count: Vec<(String, usize)>,
    pub elim_edge_count: usize,
    pub consolidated_edge_count: usize,
    pub consolidated_csv_path: Option<std::path::PathBuf>,
    pub consolidated_parquet_path: Option<std::path::PathBuf>,
}

/// Per-entity CSV header (15 columns — single-entity v5.8 schema + 2 IC fields).
const ENTITY_CSV_HEADER: &str = "edge_id,document_id,posting_date,from_account,to_account,\
from_line_id,to_line_id,amount,confidence,predecessor_edge_id,\
business_process,is_fraud,is_anomaly,ic_pair_id,ic_partner_entity";

/// Consolidated CSV header (18 columns — entity-scoped + IC + elimination flags).
const CONSOLIDATED_CSV_HEADER: &str = "edge_id,document_id,entity_code,posting_date,\
from_account,to_account,from_line_id,to_line_id,amount,confidence,\
predecessor_edge_id,business_process,is_fraud,is_anomaly,\
ic_pair_id,ic_partner_entity,is_eliminated,eliminates_ic_pair_id";

/// Emit per-entity + consolidated je_network artefacts.
///
/// # Arguments
///
/// * `contributing_jes` — `(entity_code, JEs)` pairs as walked by
///   `walk_entity_archives` in the aggregate driver.
/// * `elim_jes` — elimination JEs returned by
///   [`crate::aggregate::elimination::eliminations_to_journal_entries`].
///   These appear only in the consolidated output with
///   `is_eliminated = true`.
/// * `out_dir` — group archive root.
pub fn write_je_network_artefacts(
    contributing_jes: &[(String, Vec<JournalEntry>)],
    elim_jes: &[JournalEntry],
    out_dir: &Path,
) -> GroupResult<JeNetworkSummary> {
    let mut summary = JeNetworkSummary::default();

    // Container for the consolidated concat.  Each entry is
    // (entity_code, edge, is_eliminated, eliminates_ic_pair_id).
    let mut all_edges: Vec<(String, JeNetworkEdge, bool, Option<String>)> =
        Vec::with_capacity(contributing_jes.iter().map(|(_, j)| j.len()).sum::<usize>() * 2);

    // ── Per-entity emit ──────────────────────────────────────────────────
    for (entity, jes) in contributing_jes {
        let edges = build_je_network_edges(jes, JeNetworkMethod::A);
        write_entity_csv(out_dir, entity, &edges)?;
        write_entity_parquet(out_dir, entity, &edges)?;
        summary
            .per_entity_edge_count
            .push((entity.clone(), edges.len()));
        for e in edges {
            all_edges.push((entity.clone(), e, false, None));
        }
    }

    // ── Elimination edges ────────────────────────────────────────────────
    let elim_edges = build_je_network_edges(elim_jes, JeNetworkMethod::A);
    summary.elim_edge_count = elim_edges.len();
    for (je, e) in elim_jes.iter().zip(elim_edges) {
        // The elimination JE's company_code is set to "CONSOLIDATION" by
        // the elimination factory (per v5.0 contract); use it as the
        // entity code for the consolidated row.
        let entity_code = je.header.company_code.clone();
        // No direct ic_pair_id link on the synthetic elim JE — the
        // pair-level association lives on the EliminationEntry that
        // produced the JE.  v5.10 leaves this column empty; future
        // releases can plumb it through if needed.
        all_edges.push((entity_code, e, true, None));
    }
    summary.consolidated_edge_count = all_edges.len();

    // ── Consolidated emit ────────────────────────────────────────────────
    let consol_dir = out_dir.join("consolidated");
    std::fs::create_dir_all(&consol_dir).map_err(GroupError::Io)?;

    let csv_path = consol_dir.join("je_network.csv");
    write_consolidated_csv(&csv_path, &all_edges)?;
    summary.consolidated_csv_path = Some(csv_path);

    let parquet_path = consol_dir.join("je_network.parquet");
    write_consolidated_parquet(&parquet_path, &all_edges)?;
    summary.consolidated_parquet_path = Some(parquet_path);

    Ok(summary)
}

// ─── CSV writers ─────────────────────────────────────────────────────────────

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn write_entity_csv(out_dir: &Path, entity: &str, edges: &[JeNetworkEdge]) -> GroupResult<()> {
    let dir = out_dir.join("entities").join(entity).join("graphs");
    std::fs::create_dir_all(&dir).map_err(GroupError::Io)?;
    let path = dir.join("je_network.csv");
    let file = File::create(&path).map_err(GroupError::Io)?;
    let mut w = BufWriter::with_capacity(256 * 1024, file);
    use std::io::Write;
    writeln!(w, "{ENTITY_CSV_HEADER}").map_err(GroupError::Io)?;
    for e in edges {
        write_entity_row(&mut w, e)?;
    }
    w.flush().map_err(GroupError::Io)?;
    Ok(())
}

fn write_entity_row<W: std::io::Write>(w: &mut W, e: &JeNetworkEdge) -> GroupResult<()> {
    writeln!(
        w,
        "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        csv_escape(&e.edge_id),
        csv_escape(&e.document_id.to_string()),
        csv_escape(&e.posting_date.to_string()),
        csv_escape(&e.from_account),
        csv_escape(&e.to_account),
        csv_escape(&e.from_line_id),
        csv_escape(&e.to_line_id),
        e.amount,
        e.confidence,
        csv_escape(&e.predecessor_edge_id),
        csv_escape(&e.business_process),
        e.is_fraud,
        e.is_anomaly,
        csv_escape(e.ic_pair_id.as_deref().unwrap_or("")),
        csv_escape(e.ic_partner_entity.as_deref().unwrap_or("")),
    )
    .map_err(GroupError::Io)?;
    Ok(())
}

fn write_consolidated_csv(
    path: &Path,
    rows: &[(String, JeNetworkEdge, bool, Option<String>)],
) -> GroupResult<()> {
    let file = File::create(path).map_err(GroupError::Io)?;
    let mut w = BufWriter::with_capacity(1024 * 1024, file);
    use std::io::Write;
    writeln!(w, "{CONSOLIDATED_CSV_HEADER}").map_err(GroupError::Io)?;
    for (entity_code, e, is_elim, elim_pair) in rows {
        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            csv_escape(&e.edge_id),
            csv_escape(&e.document_id.to_string()),
            csv_escape(entity_code),
            csv_escape(&e.posting_date.to_string()),
            csv_escape(&e.from_account),
            csv_escape(&e.to_account),
            csv_escape(&e.from_line_id),
            csv_escape(&e.to_line_id),
            e.amount,
            e.confidence,
            csv_escape(&e.predecessor_edge_id),
            csv_escape(&e.business_process),
            e.is_fraud,
            e.is_anomaly,
            csv_escape(e.ic_pair_id.as_deref().unwrap_or("")),
            csv_escape(e.ic_partner_entity.as_deref().unwrap_or("")),
            is_elim,
            csv_escape(elim_pair.as_deref().unwrap_or("")),
        )
        .map_err(GroupError::Io)?;
    }
    w.flush().map_err(GroupError::Io)?;
    Ok(())
}

// ─── Parquet writers ─────────────────────────────────────────────────────────

fn entity_parquet_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("edge_id", DataType::Utf8, false),
        Field::new("document_id", DataType::Utf8, false),
        Field::new("posting_date", DataType::Utf8, false),
        Field::new("from_account", DataType::Utf8, false),
        Field::new("to_account", DataType::Utf8, false),
        Field::new("from_line_id", DataType::Utf8, false),
        Field::new("to_line_id", DataType::Utf8, false),
        // Stored as Utf8 to preserve full Decimal precision (matches
        // datasynth-output's parquet-sink convention for monetary
        // fields).
        Field::new("amount", DataType::Utf8, false),
        Field::new("confidence", DataType::Float64, false),
        Field::new("predecessor_edge_id", DataType::Utf8, false),
        Field::new("business_process", DataType::Utf8, false),
        Field::new("is_fraud", DataType::Boolean, false),
        Field::new("is_anomaly", DataType::Boolean, false),
        Field::new("ic_pair_id", DataType::Utf8, true),
        Field::new("ic_partner_entity", DataType::Utf8, true),
    ]))
}

fn consolidated_parquet_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("edge_id", DataType::Utf8, false),
        Field::new("document_id", DataType::Utf8, false),
        Field::new("entity_code", DataType::Utf8, false),
        Field::new("posting_date", DataType::Utf8, false),
        Field::new("from_account", DataType::Utf8, false),
        Field::new("to_account", DataType::Utf8, false),
        Field::new("from_line_id", DataType::Utf8, false),
        Field::new("to_line_id", DataType::Utf8, false),
        Field::new("amount", DataType::Utf8, false),
        Field::new("confidence", DataType::Float64, false),
        Field::new("predecessor_edge_id", DataType::Utf8, false),
        Field::new("business_process", DataType::Utf8, false),
        Field::new("is_fraud", DataType::Boolean, false),
        Field::new("is_anomaly", DataType::Boolean, false),
        Field::new("ic_pair_id", DataType::Utf8, true),
        Field::new("ic_partner_entity", DataType::Utf8, true),
        Field::new("is_eliminated", DataType::Boolean, false),
        Field::new("eliminates_ic_pair_id", DataType::Utf8, true),
    ]))
}

fn dec_to_str(d: Decimal) -> String {
    d.to_string()
}

fn write_entity_parquet(out_dir: &Path, entity: &str, edges: &[JeNetworkEdge]) -> GroupResult<()> {
    let dir = out_dir.join("entities").join(entity).join("graphs");
    std::fs::create_dir_all(&dir).map_err(GroupError::Io)?;
    let path = dir.join("je_network.parquet");
    let schema = entity_parquet_schema();

    let edge_id: Vec<&str> = edges.iter().map(|e| e.edge_id.as_str()).collect();
    let document_id: Vec<String> = edges.iter().map(|e| e.document_id.to_string()).collect();
    let posting_date: Vec<String> = edges.iter().map(|e| e.posting_date.to_string()).collect();
    let from_account: Vec<&str> = edges.iter().map(|e| e.from_account.as_str()).collect();
    let to_account: Vec<&str> = edges.iter().map(|e| e.to_account.as_str()).collect();
    let from_line_id: Vec<&str> = edges.iter().map(|e| e.from_line_id.as_str()).collect();
    let to_line_id: Vec<&str> = edges.iter().map(|e| e.to_line_id.as_str()).collect();
    let amount: Vec<String> = edges.iter().map(|e| dec_to_str(e.amount)).collect();
    let confidence: Vec<f64> = edges.iter().map(|e| e.confidence).collect();
    let predecessor: Vec<&str> = edges
        .iter()
        .map(|e| e.predecessor_edge_id.as_str())
        .collect();
    let bp: Vec<&str> = edges.iter().map(|e| e.business_process.as_str()).collect();
    let is_fraud: Vec<bool> = edges.iter().map(|e| e.is_fraud).collect();
    let is_anomaly: Vec<bool> = edges.iter().map(|e| e.is_anomaly).collect();
    let ic_pair: Vec<Option<String>> = edges.iter().map(|e| e.ic_pair_id.clone()).collect();
    let ic_partner: Vec<Option<String>> =
        edges.iter().map(|e| e.ic_partner_entity.clone()).collect();

    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(edge_id)),
        Arc::new(StringArray::from(document_id)),
        Arc::new(StringArray::from(posting_date)),
        Arc::new(StringArray::from(from_account)),
        Arc::new(StringArray::from(to_account)),
        Arc::new(StringArray::from(from_line_id)),
        Arc::new(StringArray::from(to_line_id)),
        Arc::new(StringArray::from(amount)),
        Arc::new(Float64Array::from(confidence)),
        Arc::new(StringArray::from(predecessor)),
        Arc::new(StringArray::from(bp)),
        Arc::new(BooleanArray::from(is_fraud)),
        Arc::new(BooleanArray::from(is_anomaly)),
        Arc::new(StringArray::from(ic_pair)),
        Arc::new(StringArray::from(ic_partner)),
    ];

    let batch = RecordBatch::try_new(Arc::clone(&schema), columns)
        .map_err(|e| GroupError::Aggregate(format!("je_network entity arrow build: {e}")))?;

    write_parquet_batch(&path, &schema, &batch)
}

fn write_consolidated_parquet(
    path: &Path,
    rows: &[(String, JeNetworkEdge, bool, Option<String>)],
) -> GroupResult<()> {
    let schema = consolidated_parquet_schema();

    let edge_id: Vec<&str> = rows.iter().map(|(_, e, _, _)| e.edge_id.as_str()).collect();
    let document_id: Vec<String> = rows
        .iter()
        .map(|(_, e, _, _)| e.document_id.to_string())
        .collect();
    let entity_code: Vec<&str> = rows.iter().map(|(c, _, _, _)| c.as_str()).collect();
    let posting_date: Vec<String> = rows
        .iter()
        .map(|(_, e, _, _)| e.posting_date.to_string())
        .collect();
    let from_account: Vec<&str> = rows
        .iter()
        .map(|(_, e, _, _)| e.from_account.as_str())
        .collect();
    let to_account: Vec<&str> = rows
        .iter()
        .map(|(_, e, _, _)| e.to_account.as_str())
        .collect();
    let from_line_id: Vec<&str> = rows
        .iter()
        .map(|(_, e, _, _)| e.from_line_id.as_str())
        .collect();
    let to_line_id: Vec<&str> = rows
        .iter()
        .map(|(_, e, _, _)| e.to_line_id.as_str())
        .collect();
    let amount: Vec<String> = rows
        .iter()
        .map(|(_, e, _, _)| dec_to_str(e.amount))
        .collect();
    let confidence: Vec<f64> = rows.iter().map(|(_, e, _, _)| e.confidence).collect();
    let predecessor: Vec<&str> = rows
        .iter()
        .map(|(_, e, _, _)| e.predecessor_edge_id.as_str())
        .collect();
    let bp: Vec<&str> = rows
        .iter()
        .map(|(_, e, _, _)| e.business_process.as_str())
        .collect();
    let is_fraud: Vec<bool> = rows.iter().map(|(_, e, _, _)| e.is_fraud).collect();
    let is_anomaly: Vec<bool> = rows.iter().map(|(_, e, _, _)| e.is_anomaly).collect();
    let ic_pair: Vec<Option<String>> = rows
        .iter()
        .map(|(_, e, _, _)| e.ic_pair_id.clone())
        .collect();
    let ic_partner: Vec<Option<String>> = rows
        .iter()
        .map(|(_, e, _, _)| e.ic_partner_entity.clone())
        .collect();
    let is_eliminated: Vec<bool> = rows.iter().map(|(_, _, b, _)| *b).collect();
    let eliminates_pair: Vec<Option<String>> = rows.iter().map(|(_, _, _, p)| p.clone()).collect();

    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(edge_id)),
        Arc::new(StringArray::from(document_id)),
        Arc::new(StringArray::from(entity_code)),
        Arc::new(StringArray::from(posting_date)),
        Arc::new(StringArray::from(from_account)),
        Arc::new(StringArray::from(to_account)),
        Arc::new(StringArray::from(from_line_id)),
        Arc::new(StringArray::from(to_line_id)),
        Arc::new(StringArray::from(amount)),
        Arc::new(Float64Array::from(confidence)),
        Arc::new(StringArray::from(predecessor)),
        Arc::new(StringArray::from(bp)),
        Arc::new(BooleanArray::from(is_fraud)),
        Arc::new(BooleanArray::from(is_anomaly)),
        Arc::new(StringArray::from(ic_pair)),
        Arc::new(StringArray::from(ic_partner)),
        Arc::new(BooleanArray::from(is_eliminated)),
        Arc::new(StringArray::from(eliminates_pair)),
    ];

    let batch = RecordBatch::try_new(Arc::clone(&schema), columns)
        .map_err(|e| GroupError::Aggregate(format!("je_network consolidated arrow build: {e}")))?;

    write_parquet_batch(path, &schema, &batch)
}

fn write_parquet_batch(path: &Path, schema: &Arc<Schema>, batch: &RecordBatch) -> GroupResult<()> {
    let file = File::create(path).map_err(GroupError::Io)?;
    let props = WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::default()))
        .build();
    let mut writer = ArrowWriter::try_new(file, Arc::clone(schema), Some(props))
        .map_err(|e| GroupError::Aggregate(format!("parquet writer init: {e}")))?;
    writer
        .write(batch)
        .map_err(|e| GroupError::Aggregate(format!("parquet write: {e}")))?;
    writer
        .close()
        .map_err(|e| GroupError::Aggregate(format!("parquet close: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use datasynth_core::models::{
        BusinessProcess, JournalEntry, JournalEntryHeader, JournalEntryLine,
    };
    use rust_decimal::Decimal;
    use uuid::Uuid;

    fn header_for(doc: Uuid, company: &str) -> JournalEntryHeader {
        let mut h = JournalEntryHeader::new(
            company.to_string(),
            NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        );
        h.document_id = doc;
        h.business_process = Some(BusinessProcess::P2P);
        h
    }

    fn line(doc: Uuid, n: u32, account: &str, debit: i64, credit: i64) -> JournalEntryLine {
        JournalEntryLine {
            document_id: doc,
            line_number: n,
            gl_account: account.into(),
            debit_amount: Decimal::from(debit),
            credit_amount: Decimal::from(credit),
            ..Default::default()
        }
    }

    fn two_line_je(company: &str, debit_acc: &str, credit_acc: &str, amt: i64) -> JournalEntry {
        let doc = Uuid::new_v4();
        let header = header_for(doc, company);
        let lines = smallvec::smallvec![
            line(doc, 1, debit_acc, amt, 0),
            line(doc, 2, credit_acc, 0, amt),
        ];
        JournalEntry { header, lines }
    }

    #[test]
    fn writes_per_entity_and_consolidated_files() {
        let tmp = tempfile::tempdir().unwrap();
        let contributing = vec![
            (
                "ACME_HQ".to_string(),
                vec![
                    two_line_je("ACME_HQ", "1000", "2000", 1000),
                    two_line_je("ACME_HQ", "1000", "4000", 5000),
                ],
            ),
            (
                "ACME_EUR".to_string(),
                vec![two_line_je("ACME_EUR", "1100", "4500", 2500)],
            ),
        ];
        let elim_jes: Vec<JournalEntry> = vec![two_line_je("CONSOLIDATION", "4500", "1100", 2500)];

        let summary = write_je_network_artefacts(&contributing, &elim_jes, tmp.path()).unwrap();

        assert_eq!(summary.per_entity_edge_count.len(), 2);
        assert_eq!(summary.elim_edge_count, 1);
        assert_eq!(summary.consolidated_edge_count, 4);

        // Per-entity files exist
        for entity in ["ACME_HQ", "ACME_EUR"] {
            let csv = tmp
                .path()
                .join("entities")
                .join(entity)
                .join("graphs")
                .join("je_network.csv");
            let pq = tmp
                .path()
                .join("entities")
                .join(entity)
                .join("graphs")
                .join("je_network.parquet");
            assert!(csv.exists(), "missing {csv:?}");
            assert!(pq.exists(), "missing {pq:?}");
        }

        // Consolidated files exist
        let consol_csv = tmp.path().join("consolidated").join("je_network.csv");
        let consol_pq = tmp.path().join("consolidated").join("je_network.parquet");
        assert!(consol_csv.exists());
        assert!(consol_pq.exists());

        let body = std::fs::read_to_string(&consol_csv).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 1 + 4, "header + 4 edges (3 entity + 1 elim)");
        assert!(lines[0].starts_with("edge_id,document_id,entity_code,"));
        assert!(
            lines.iter().any(|l| l.contains(",true,")),
            "at least one row should have is_eliminated=true"
        );
    }
}

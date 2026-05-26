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

/// Per-entity CSV header (16 columns — single-entity v5.27 schema + 2 IC fields).
const ENTITY_CSV_HEADER: &str = "edge_id,document_id,posting_date,from_account,to_account,\
from_line_id,to_line_id,amount,confidence,predecessor_edge_id,\
business_process,is_fraud,is_anomaly,fraud_type,ic_pair_id,ic_partner_entity";

/// Consolidated CSV header (19 columns — entity-scoped + IC + elimination flags).
const CONSOLIDATED_CSV_HEADER: &str = "edge_id,document_id,entity_code,posting_date,\
from_account,to_account,from_line_id,to_line_id,amount,confidence,\
predecessor_edge_id,business_process,is_fraud,is_anomaly,fraud_type,\
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
        "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
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
        csv_escape(e.fraud_type.as_deref().unwrap_or("")),
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
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
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
            csv_escape(e.fraud_type.as_deref().unwrap_or("")),
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
        Field::new("fraud_type", DataType::Utf8, true),
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
        Field::new("fraud_type", DataType::Utf8, true),
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
    let fraud_type: Vec<Option<String>> = edges.iter().map(|e| e.fraud_type.clone()).collect();
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
        Arc::new(StringArray::from(fraud_type)),
        Arc::new(StringArray::from(ic_pair)),
        Arc::new(StringArray::from(ic_partner)),
    ];

    let batch = RecordBatch::try_new(Arc::clone(&schema), columns)
        .map_err(|e| GroupError::Aggregate(format!("je_network entity arrow build: {e}")))?;

    write_parquet_batch(&path, &schema, &batch)
}

/// v5.31 C1 Phase 2: build a single RecordBatch from a slice of
/// consolidated-row tuples. Factored out so the streaming writer can
/// call it on batched buffers without rewriting the array-building
/// logic.
fn build_consolidated_record_batch(
    schema: &Arc<Schema>,
    rows: &[(String, JeNetworkEdge, bool, Option<String>)],
) -> GroupResult<RecordBatch> {
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
    let fraud_type: Vec<Option<String>> = rows
        .iter()
        .map(|(_, e, _, _)| e.fraud_type.clone())
        .collect();
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
        Arc::new(StringArray::from(fraud_type)),
        Arc::new(StringArray::from(ic_pair)),
        Arc::new(StringArray::from(ic_partner)),
        Arc::new(BooleanArray::from(is_eliminated)),
        Arc::new(StringArray::from(eliminates_pair)),
    ];

    let batch = RecordBatch::try_new(Arc::clone(schema), columns)
        .map_err(|e| GroupError::Aggregate(format!("je_network consolidated arrow build: {e}")))?;
    Ok(batch)
}

#[allow(dead_code)]
fn write_consolidated_parquet(
    path: &Path,
    rows: &[(String, JeNetworkEdge, bool, Option<String>)],
) -> GroupResult<()> {
    let schema = consolidated_parquet_schema();
    let batch = build_consolidated_record_batch(&schema, rows)?;
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

// ─── v5.31 C1 Phase 2 — streaming JE network writer ──────────────────────────

/// Streaming JE-network writer for the aggregate phase.
///
/// **Why:** v5.31 C1 Phase 1 eliminated the `contributing_tbs` hold
/// (~200 GB at 2k scale) but the 2026-05-27 2000-entity validation
/// OOM-killed at 228 GB anyway because the *journal-entries* still
/// got loaded into a 700+ GB on-disk → 200-400 GB in-memory vector
/// (`contributing_jes: Vec<(String, Vec<JournalEntry>)>`). Phase 2
/// closes that hold by streaming per-entity edges directly to disk
/// during the aggregate walk — no consolidated edge vec ever
/// materialises in memory.
///
/// Usage pattern from the streaming walk:
/// ```ignore
/// let mut writer = JeNetworkStreamingWriter::open(out_dir)?;
/// for entity in entities {
///     let jes = load_entity_journal_entries(entity)?;
///     writer.write_entity_edges(&entity.code, &jes)?;
///     // jes can be dropped here (after filtering to IC subset)
/// }
/// let summary = writer.finalize(&elim_jes)?;
/// ```
///
/// The CSV writers stream-append row-by-row. The per-entity parquet
/// is written per entity (one file each), so its memory cost is
/// bounded by a single entity's edges. The consolidated parquet is
/// **deferred** to Phase 3 — at 2000-entity scale it would still
/// hit the OOM ceiling when buffering all edges into one Arrow
/// RecordBatch. Consumers who need consolidated parquet can convert
/// from the consolidated CSV with `pyarrow.csv.read_csv` or similar.
pub struct JeNetworkStreamingWriter {
    /// Consolidated CSV writer (open for streaming appends).
    consol_csv: BufWriter<File>,
    consol_csv_path: std::path::PathBuf,
    /// v5.31 C1 Phase 2: Consolidated parquet writer — batches via
    /// `pending_rows` to keep memory bounded by `PARQUET_BATCH_SIZE`.
    consol_parquet: ArrowWriter<File>,
    consol_parquet_path: std::path::PathBuf,
    consol_parquet_schema: Arc<Schema>,
    /// Buffered consolidated rows pending a parquet batch flush.
    pending_rows: Vec<(String, JeNetworkEdge, bool, Option<String>)>,
    /// Output root.
    out_dir: std::path::PathBuf,
    /// Running summary stats.
    summary: JeNetworkSummary,
}

/// v5.31 C1 Phase 2: batch size for streaming parquet writes.
/// At ~200 bytes/row this caps the buffer at ~10 MB regardless of
/// total consolidated edge count.
const PARQUET_BATCH_SIZE: usize = 50_000;

impl JeNetworkStreamingWriter {
    /// Open the streaming writer. Creates `{out_dir}/consolidated/`
    /// and starts both a fresh `je_network.csv` (line-by-line stream)
    /// and a fresh `je_network.parquet` (batched stream via ArrowWriter
    /// with row groups every `PARQUET_BATCH_SIZE` rows).
    pub fn open(out_dir: &Path) -> GroupResult<Self> {
        let consol_dir = out_dir.join("consolidated");
        std::fs::create_dir_all(&consol_dir).map_err(GroupError::Io)?;

        // CSV writer
        let consol_csv_path = consol_dir.join("je_network.csv");
        let csv_file = File::create(&consol_csv_path).map_err(GroupError::Io)?;
        let mut consol_csv = BufWriter::with_capacity(1024 * 1024, csv_file);
        use std::io::Write;
        writeln!(consol_csv, "{CONSOLIDATED_CSV_HEADER}").map_err(GroupError::Io)?;

        // Parquet writer — batched, schema fixed at open()
        let consol_parquet_path = consol_dir.join("je_network.parquet");
        let consol_parquet_schema = consolidated_parquet_schema();
        let pq_file = File::create(&consol_parquet_path).map_err(GroupError::Io)?;
        let pq_props = WriterProperties::builder()
            .set_compression(Compression::ZSTD(ZstdLevel::default()))
            .build();
        let consol_parquet =
            ArrowWriter::try_new(pq_file, Arc::clone(&consol_parquet_schema), Some(pq_props))
                .map_err(|e| {
                    GroupError::Aggregate(format!("je_network parquet writer init: {e}"))
                })?;

        Ok(Self {
            consol_csv,
            consol_csv_path,
            consol_parquet,
            consol_parquet_path,
            consol_parquet_schema,
            pending_rows: Vec::with_capacity(PARQUET_BATCH_SIZE),
            out_dir: out_dir.to_path_buf(),
            summary: JeNetworkSummary::default(),
        })
    }

    /// Emit one entity's edges. Writes per-entity CSV + parquet under
    /// `{out_dir}/entities/{entity}/graphs/` and appends the same
    /// rows (with `is_eliminated=false`) to the consolidated CSV.
    ///
    /// **Caller responsibility:** drop the source JEs (or filter to a
    /// downstream-needed subset) after this call returns — the writer
    /// retains nothing from `jes`.
    pub fn write_entity_edges(
        &mut self,
        entity_code: &str,
        jes: &[JournalEntry],
    ) -> GroupResult<usize> {
        let edges = build_je_network_edges(jes, JeNetworkMethod::A);
        write_entity_csv(&self.out_dir, entity_code, &edges)?;
        write_entity_parquet(&self.out_dir, entity_code, &edges)?;
        // Append to consolidated CSV (is_eliminated=false, no elim pair link).
        for e in &edges {
            self.write_consolidated_row(entity_code, e, false, None)?;
        }
        let n = edges.len();
        self.summary
            .per_entity_edge_count
            .push((entity_code.to_string(), n));
        self.summary.consolidated_edge_count += n;
        Ok(n)
    }

    /// Emit the elimination edges (consolidated CSV only — no per-entity
    /// emit for eliminations). The elimination JE's `header.company_code`
    /// is used as the entity code in the consolidated row, per the
    /// v5.0 contract (elimination factory sets it to "CONSOLIDATION").
    pub fn write_elim_edges(&mut self, elim_jes: &[JournalEntry]) -> GroupResult<()> {
        let elim_edges = build_je_network_edges(elim_jes, JeNetworkMethod::A);
        for (je, e) in elim_jes.iter().zip(elim_edges.iter()) {
            let entity_code = je.header.company_code.clone();
            self.write_consolidated_row(&entity_code, e, true, None)?;
        }
        self.summary.elim_edge_count = elim_edges.len();
        self.summary.consolidated_edge_count += elim_edges.len();
        Ok(())
    }

    /// Flush both writers and return the accumulated summary.
    /// Drains any remaining pending parquet rows, closes the parquet
    /// writer (writes footer + row group index), and flushes the CSV.
    pub fn finalize(mut self) -> GroupResult<JeNetworkSummary> {
        use std::io::Write;
        // Flush any pending parquet rows before closing the writer.
        if !self.pending_rows.is_empty() {
            let batch =
                build_consolidated_record_batch(&self.consol_parquet_schema, &self.pending_rows)?;
            self.consol_parquet
                .write(&batch)
                .map_err(|e| GroupError::Aggregate(format!("je_network parquet flush: {e}")))?;
            self.pending_rows.clear();
        }
        self.consol_parquet
            .close()
            .map_err(|e| GroupError::Aggregate(format!("je_network parquet close: {e}")))?;
        self.consol_csv.flush().map_err(GroupError::Io)?;
        self.summary.consolidated_csv_path = Some(self.consol_csv_path);
        self.summary.consolidated_parquet_path = Some(self.consol_parquet_path);
        Ok(self.summary)
    }

    fn write_consolidated_row(
        &mut self,
        entity_code: &str,
        e: &JeNetworkEdge,
        is_elim: bool,
        elim_pair: Option<&str>,
    ) -> GroupResult<()> {
        use std::io::Write;
        // ── CSV: stream-write line-by-line ─────────────────────────────
        writeln!(
            self.consol_csv,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
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
            csv_escape(e.fraud_type.as_deref().unwrap_or("")),
            csv_escape(e.ic_pair_id.as_deref().unwrap_or("")),
            csv_escape(e.ic_partner_entity.as_deref().unwrap_or("")),
            is_elim,
            csv_escape(elim_pair.unwrap_or("")),
        )
        .map_err(GroupError::Io)?;

        // ── Parquet: push into batch buffer; flush when full ───────────
        self.pending_rows.push((
            entity_code.to_string(),
            e.clone(),
            is_elim,
            elim_pair.map(|s| s.to_string()),
        ));
        if self.pending_rows.len() >= PARQUET_BATCH_SIZE {
            let batch =
                build_consolidated_record_batch(&self.consol_parquet_schema, &self.pending_rows)?;
            self.consol_parquet
                .write(&batch)
                .map_err(|e| GroupError::Aggregate(format!("je_network parquet write: {e}")))?;
            self.pending_rows.clear();
        }
        Ok(())
    }
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

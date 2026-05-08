//! Comprehensive output writer for all generated data.
//!
//! Writes all generated data from the EnhancedGenerationResult to files
//! in the output directory. Uses CSV for flat tabular data (journal entry
//! lines) and JSON for types with nested structures (Vecs, sub-structs).

use std::cell::Cell;
use std::io::Write;
use std::path::Path;

use crate::enhanced_orchestrator::EnhancedGenerationResult;
use datasynth_core::documents::PaymentType;
use datasynth_output::OutputRootConfig;
use tracing::{info, warn};

thread_local! {
    /// Thread-local flat-layout flag. When true, every `write_json_safe` call
    /// routes through `write_json_flat` so nested `{header, lines}` shapes get
    /// flattened. Set by `write_all_output_with_layout` at the top of its body,
    /// reset on exit.
    static FLAT_LAYOUT_ACTIVE: Cell<bool> = const { Cell::new(false) };

    /// Thread-local JSON skip flag. When true, `write_json_safe` becomes a no-op.
    /// Set by `write_all_output_with_layout` when the requested formats don't
    /// include JSON. This avoids wrapping 190+ call sites in `if write_json`.
    static SKIP_JSON: Cell<bool> = const { Cell::new(false) };
}

/// Write a JSON file for any serializable slice. Skips empty slices.
///
/// Streams JSON directly to a buffered file writer instead of allocating
/// the entire JSON string in memory (Phase 3 I/O optimization).
/// Write a JSON array by streaming one record at a time.
///
/// Instead of serializing the entire `&[T]` in one `to_writer_pretty` call
/// (which builds a massive in-memory serde state for large arrays), this
/// writes `[\n` + per-record pretty-printed JSON with commas + `\n]`.
///
/// For 200K+ records this reduces peak memory and improves write throughput
/// by avoiding serde's internal buffering of the full array structure.
fn write_json<T: serde::Serialize>(
    data: &[T],
    path: &Path,
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;

    if data.is_empty() {
        return Ok(());
    }

    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::with_capacity(512 * 1024, file);

    // Stream records one at a time into a JSON array
    writer.write_all(b"[\n")?;
    for (i, item) in data.iter().enumerate() {
        if i > 0 {
            writer.write_all(b",\n")?;
        }
        serde_json::to_writer_pretty(&mut writer, item)?;
    }
    writer.write_all(b"\n]\n")?;
    writer.flush()?;

    info!(
        "  {} written: {} records -> {}",
        label,
        data.len(),
        path.display()
    );
    Ok(())
}

/// Write journal entry lines as a flat CSV file.
///
/// This extracts the key fields from both the header and each line item to
/// produce a single flat CSV that can be loaded directly into dataframes.
fn write_journal_entries_csv(
    result: &EnhancedGenerationResult,
    output_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if result.journal_entries.is_empty() {
        return Ok(());
    }

    let path = output_dir.join("journal_entries.csv");
    let file = std::fs::File::create(&path)?;
    let mut w = std::io::BufWriter::with_capacity(256 * 1024, file);

    // Write header.
    //
    // Schema note: each release that widens the schema appends new
    // columns at the end so existing column-positional consumers keep
    // working.
    //   v5.5.1 added:
    //     is_manual, is_post_close, source_system     (audit / ETL provenance)
    //     account_description                         (joined from CoA)
    //     financial_statement_category                (asset/liability/...)
    //     assignment, value_date, tax_code            (already-populated line fields)
    //     transaction_id                              (stable per-line id)
    //   v5.6.0 added (ISO 21378 Audit Data Collection classification):
    //     account_class, account_class_name           (Level-2 e.g. "A.B" / "Trade Receivables")
    //     account_sub_class, account_sub_class_name   (Level-3 e.g. "A.B.A" / "Trade Accounts Receivable")
    //   v5.8.0 added:
    //     predecessor_line_id                         (UUID v5 of preceding line in document chain;
    //                                                  populated by document_flow_je_generator for
    //                                                  P2P / O2C chains, empty for chain heads and
    //                                                  for purely-GL adjustments)
    writeln!(
        w,
        "document_id,company_code,fiscal_year,fiscal_period,posting_date,document_date,\
         document_type,currency,exchange_rate,reference,header_text,created_by,source,\
         business_process,ledger,is_fraud,is_anomaly,\
         line_number,gl_account,debit_amount,credit_amount,local_amount,\
         cost_center,profit_center,line_text,\
         auxiliary_account_number,auxiliary_account_label,lettrage,lettrage_date,\
         is_manual,is_post_close,source_system,\
         account_description,financial_statement_category,\
         assignment,value_date,tax_code,transaction_id,\
         account_class,account_class_name,account_sub_class,account_sub_class_name,\
         predecessor_line_id"
    )?;

    // Build a CoA → (short_description, ISO class, ISO sub-class) lookup.
    // Empty when no CoA was generated (e.g. some smoke tests); resolution
    // falls back to the line's already-populated `account_description`
    // and to empty ISO codes.
    let coa_index: std::collections::HashMap<&str, (&str, &str, &str, &str, &str)> = result
        .chart_of_accounts
        .accounts
        .iter()
        .map(|a| {
            (
                a.account_number.as_str(),
                (
                    a.short_description.as_str(),
                    a.account_class.as_str(),
                    a.account_class_name.as_str(),
                    a.account_sub_class.as_str(),
                    a.account_sub_class_name.as_str(),
                ),
            )
        })
        .collect();

    for je in &result.journal_entries {
        let h = &je.header;
        for line in &je.lines {
            let lettrage_date_str = line
                .lettrage_date
                .map(|d| d.to_string())
                .unwrap_or_default();
            let value_date_str = line.value_date.map(|d| d.to_string()).unwrap_or_default();
            // Look up CoA-joined fields in one shot.
            let coa_hit = coa_index.get(line.gl_account.as_str()).copied();
            let coa_short_desc = coa_hit.map(|t| t.0).unwrap_or("");
            let coa_class = coa_hit.map(|t| t.1).unwrap_or("");
            let coa_class_name = coa_hit.map(|t| t.2).unwrap_or("");
            let coa_sub_class = coa_hit.map(|t| t.3).unwrap_or("");
            let coa_sub_class_name = coa_hit.map(|t| t.4).unwrap_or("");
            // Prefer the line's own account_description; fall back to the CoA
            // lookup so consumers always get a name even when the generator
            // forgot to populate the field.
            let account_description: &str = line
                .account_description
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(coa_short_desc);
            // Derive the FSA category from the gl_account prefix (1xxx=asset,
            // 2xxx=liability, ...). Cheap, deterministic, no CoA dependency.
            let fsa_category =
                datasynth_core::accounts::AccountCategory::from_account(line.gl_account.as_str())
                    .as_label();
            // Stable per-line identifier (UUID v5 of document_id+line_number).
            let transaction_id = line.transaction_id.clone().unwrap_or_else(|| {
                datasynth_core::models::JournalEntryLine::derive_transaction_id(
                    line.document_id,
                    line.line_number,
                )
            });
            writeln!(
                w,
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                h.document_id,
                csv_escape(&h.company_code),
                h.fiscal_year,
                h.fiscal_period,
                h.posting_date,
                h.document_date,
                csv_escape(&h.document_type),
                csv_escape(&h.currency),
                h.exchange_rate,
                csv_opt_str(&h.reference),
                csv_opt_str(&h.header_text),
                csv_escape(&h.created_by),
                h.source,
                h.business_process
                    .map(|bp| format!("{bp:?}"))
                    .unwrap_or_default(),
                csv_escape(&h.ledger),
                h.is_fraud,
                h.is_anomaly,
                line.line_number,
                csv_escape(&line.gl_account),
                line.debit_amount,
                line.credit_amount,
                line.local_amount,
                csv_opt_str(&line.cost_center),
                csv_opt_str(&line.profit_center),
                csv_opt_str(&line.line_text),
                csv_opt_str(&line.auxiliary_account_number),
                csv_opt_str(&line.auxiliary_account_label),
                csv_opt_str(&line.lettrage),
                lettrage_date_str,
                h.is_manual,
                h.is_post_close,
                csv_escape(&h.source_system),
                csv_escape(account_description),
                fsa_category,
                csv_opt_str(&line.assignment),
                value_date_str,
                csv_opt_str(&line.tax_code),
                csv_escape(&transaction_id),
                csv_escape(coa_class),
                csv_escape(coa_class_name),
                csv_escape(coa_sub_class),
                csv_escape(coa_sub_class_name),
                csv_opt_str(&line.predecessor_line_id),
            )?;
        }
    }

    w.flush()?;
    let total_lines: usize = result.journal_entries.iter().map(|je| je.lines.len()).sum();
    info!(
        "  Journal entries CSV written: {} entries, {} line items -> {}",
        result.journal_entries.len(),
        total_lines,
        path.display()
    );
    Ok(())
}

/// **v5.8.0** — write `graphs/je_network.csv`: a flat edge-list of the
/// accounting network derived from journal entries.
///
/// Each row represents one debit↔credit flow within a single JE,
/// formed via the cartesian product of debit lines × credit lines (the
/// approach in `datasynth-graph::TransactionGraphBuilder`). For a
/// 2-line JE this is exactly the bijective Method-A flow from
/// Ivertowski et al. (2024); for larger JEs it is a Method-B/C
/// approximation with proportional amount allocation.
///
/// Joins back to `journal_entries.csv` via:
///   - `document_id` → JE-level header
///   - `from_line_id` / `to_line_id` → per-line `transaction_id`
///   - `predecessor_edge_id` → previous flow in a document chain
fn write_je_network_csv(
    result: &EnhancedGenerationResult,
    output_dir: &Path,
    method: datasynth_config::JeNetworkMethod,
) -> Result<(), Box<dyn std::error::Error>> {
    use rust_decimal::Decimal;

    if result.journal_entries.is_empty() {
        return Ok(());
    }
    let graphs_dir = output_dir.join("graphs");
    std::fs::create_dir_all(&graphs_dir)?;
    let path = graphs_dir.join("je_network.csv");
    let file = std::fs::File::create(&path)?;
    let mut w = std::io::BufWriter::with_capacity(256 * 1024, file);

    writeln!(
        w,
        "edge_id,document_id,posting_date,from_account,to_account,\
         from_line_id,to_line_id,amount,confidence,\
         predecessor_edge_id,business_process,is_fraud,is_anomaly"
    )?;

    // Build a map line_transaction_id → edge_id of the FIRST out-going
    // edge from that line so predecessor_edge_id can be resolved without
    // a second pass: if a curr-line's predecessor_line_id points at
    // some prev-line's transaction_id, the predecessor edge is the
    // first edge in that prev JE that has this line as its credit
    // ("from") side. This reasonably maps each line to one canonical
    // outgoing edge.
    //
    // We populate the map during the first pass (write the rows) and
    // resolve predecessor_edge_id with the existing entry — JEs are
    // emitted in chain order, so the predecessor will already be in
    // the map by the time we look it up.
    let mut line_id_to_edge_id: std::collections::HashMap<String, String> =
        std::collections::HashMap::with_capacity(result.journal_entries.len() * 2);

    let mut total_edges: usize = 0;

    for je in &result.journal_entries {
        let h = &je.header;
        // Pre-compute transaction_ids for every line (so we can pair
        // them without re-deriving inside the inner loop).
        let line_ids: Vec<String> = je
            .lines
            .iter()
            .map(|l| {
                l.transaction_id.clone().unwrap_or_else(|| {
                    datasynth_core::models::JournalEntryLine::derive_transaction_id(
                        l.document_id,
                        l.line_number,
                    )
                })
            })
            .collect();

        let debits: Vec<usize> = je
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.debit_amount > Decimal::ZERO)
            .map(|(i, _)| i)
            .collect();
        let credits: Vec<usize> = je
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.credit_amount > Decimal::ZERO)
            .map(|(i, _)| i)
            .collect();
        if debits.is_empty() || credits.is_empty() {
            continue;
        }

        // Method A: bijective on 2-line entries only.  Multi-line JEs
        // are skipped under this method — see Ivertowski (2024)
        // Methods A through E.  The full Cartesian product of a
        // multi-line consolidation produces O(n × m) edges per JE,
        // which dominates total dataset size at scale; users who need
        // the multi-line edges should set
        // `graph_export.je_network.method: cartesian` (the default).
        if method == datasynth_config::JeNetworkMethod::A
            && !(debits.len() == 1 && credits.len() == 1)
        {
            continue;
        }

        let total_debit: Decimal = debits.iter().map(|i| je.lines[*i].debit_amount).sum();
        let total_credit: Decimal = credits.iter().map(|i| je.lines[*i].credit_amount).sum();
        if total_debit.is_zero() || total_credit.is_zero() {
            continue;
        }

        // Confidence per the paper: bijective on 2-line entries
        // (Method A), 1/(n*m) approximation otherwise.
        let confidence: f64 = if debits.len() == 1 && credits.len() == 1 {
            1.0
        } else {
            1.0 / (debits.len() * credits.len()) as f64
        };

        let bp = h
            .business_process
            .map(|bp| format!("{bp:?}"))
            .unwrap_or_default();
        let posting_date = h.posting_date.to_string();
        let doc_id = h.document_id.to_string();

        for &di in &debits {
            let debit_line = &je.lines[di];
            let to_line_id = &line_ids[di];
            for &ci in &credits {
                let credit_line = &je.lines[ci];
                let from_line_id = &line_ids[ci];

                // Edge id = UUID v5 of (document_id, debit.line_number,
                // credit.line_number). Stable across regenerations.
                let mut input = Vec::with_capacity(16 + 8);
                input.extend_from_slice(h.document_id.as_bytes());
                input.extend_from_slice(&debit_line.line_number.to_le_bytes());
                input.extend_from_slice(&credit_line.line_number.to_le_bytes());
                let edge_id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, &input).to_string();

                // Proportional allocation of the flow's amount (matches
                // the existing TransactionGraphBuilder::add_journal_entry
                // _debit_credit formula).
                let proportion = (debit_line.debit_amount / total_debit)
                    * (credit_line.credit_amount / total_credit);
                let amount = debit_line.debit_amount * proportion;

                // Resolve predecessor: the predecessor_line_id from the
                // CREDIT side (it represents the "from" of the next
                // edge); fall back to the debit side. Either points at
                // a transaction_id whose first outgoing edge id is the
                // predecessor edge.
                let predecessor_edge_id: String = credit_line
                    .predecessor_line_id
                    .as_ref()
                    .or(debit_line.predecessor_line_id.as_ref())
                    .and_then(|tx_id| line_id_to_edge_id.get(tx_id).cloned())
                    .unwrap_or_default();

                writeln!(
                    w,
                    "{},{},{},{},{},{},{},{},{},{},{},{},{}",
                    csv_escape(&edge_id),
                    csv_escape(&doc_id),
                    csv_escape(&posting_date),
                    csv_escape(&credit_line.gl_account),
                    csv_escape(&debit_line.gl_account),
                    csv_escape(from_line_id),
                    csv_escape(to_line_id),
                    amount,
                    confidence,
                    csv_escape(&predecessor_edge_id),
                    csv_escape(&bp),
                    h.is_fraud,
                    h.is_anomaly,
                )?;

                // Map the credit line ("from" end) to its first edge —
                // this is the canonical outgoing edge subsequent JEs
                // can refer to as their predecessor.
                line_id_to_edge_id
                    .entry(from_line_id.clone())
                    .or_insert_with(|| edge_id.clone());
                total_edges += 1;
            }
        }
    }

    w.flush()?;
    info!(
        "  JE network CSV written: {} edges from {} entries -> {}",
        total_edges,
        result.journal_entries.len(),
        path.display()
    );
    Ok(())
}

/// Write journal entries as flat JSON (header fields merged onto each line).
///
/// Each object in the output array contains all header fields plus all line fields,
/// with no nesting. This is the analytics-friendly format.
fn write_journal_entries_flat_json(
    result: &EnhancedGenerationResult,
    output_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if result.journal_entries.is_empty() {
        return Ok(());
    }

    let path = output_dir.join("journal_entries.json");
    let file = std::fs::File::create(&path)?;
    let mut writer = std::io::BufWriter::with_capacity(256 * 1024, file);

    // Write opening bracket
    writer.write_all(b"[\n")?;

    let mut first = true;
    let mut total_lines = 0usize;
    for je in &result.journal_entries {
        // Serialize header to a JSON map
        let header_value = serde_json::to_value(&je.header)?;

        for line in &je.lines {
            if !first {
                writer.write_all(b",\n")?;
            }
            first = false;
            total_lines += 1;

            // Serialize line to a JSON map, then merge header fields in
            let mut line_value = serde_json::to_value(line)?;

            if let serde_json::Value::Object(ref header_map) = header_value {
                if let serde_json::Value::Object(ref mut line_map) = line_value {
                    for (key, val) in header_map {
                        // Line fields take precedence for shared keys (e.g. document_id)
                        if !line_map.contains_key(key) {
                            line_map.insert(key.clone(), val.clone());
                        }
                    }
                }
            }

            serde_json::to_writer_pretty(&mut writer, &line_value)?;
        }
    }

    writer.write_all(b"\n]\n")?;
    writer.flush()?;
    info!(
        "  Journal entries (flat JSON) written: {} line items -> {}",
        total_lines,
        path.display()
    );
    Ok(())
}

/// v4.4.2 helper — walk a serialized OCEL event-log `Value` tree and
/// mirror `object_type_id` into `object_type` on every
/// `object_refs[*]` entry. The canonical OCEL 2.0 field name is
/// `object_type`; DataSynth's internal model carries it as
/// `object_type_id` for historical reasons. Emitting both keys lets
/// OCEL-spec-compliant consumers (pm4py, Celonis, etc.) see the type
/// without a rename step.
fn add_ocel_object_type_alias(value: &mut serde_json::Value) {
    if let Some(events) = value.get_mut("events").and_then(|v| v.as_array_mut()) {
        for event in events.iter_mut() {
            if let Some(refs) = event.get_mut("object_refs").and_then(|r| r.as_array_mut()) {
                for oref in refs.iter_mut() {
                    if let Some(obj) = oref.as_object_mut() {
                        if let Some(oti) = obj.get("object_type_id").cloned() {
                            obj.entry("object_type").or_insert(oti);
                        }
                    }
                }
            }
        }
    }
}

/// Escape a string for CSV output by quoting if it contains commas or quotes.
fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Format an Option<String> for CSV output (empty string for None).
fn csv_opt_str(opt: &Option<String>) -> String {
    match opt {
        Some(s) => csv_escape(s),
        None => String::new(),
    }
}

/// Write all generated data to the output directory.
///
/// This function exports every non-empty dataset from the generation result.
/// Journal entries are written as a flat CSV file (one row per line item)
/// and as a nested JSON file. Other data is written as JSON files since
/// many model types contain nested structures.
#[allow(dead_code)]
pub fn write_all_output(
    result: &EnhancedGenerationResult,
    output_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    write_all_output_with_layout(
        result,
        output_dir,
        datasynth_config::ExportLayout::Nested,
        &[
            datasynth_config::FileFormat::Csv,
            datasynth_config::FileFormat::Json,
        ],
        datasynth_config::JeNetworkMethod::default(),
    )
}

/// Variant of [`write_all_output_with_layout`] that routes output through
/// an [`OutputRootConfig`] instead of a raw `&Path`.
///
/// Per-entity subtree mode is used by the group-audit shard runner
/// (v5.0+): the runner sets `per_entity_subtree: true` and
/// `entity_code: Some(code)` on `root`, and this helper drops each
/// entity's archive under `{root_dir}/entities/{code}/` so group-wide
/// artifacts can still live at `{root_dir}/`.
///
/// In flat mode (the default for single-entity runs) this is exactly
/// equivalent to calling [`write_all_output_with_layout`] with
/// `output_dir = root.root_dir`, so the signature and behavior of the
/// existing single-entity entrypoints are unchanged.
#[allow(dead_code)]
pub fn write_all_output_with_root(
    result: &EnhancedGenerationResult,
    root: &OutputRootConfig,
    export_layout: datasynth_config::ExportLayout,
    formats: &[datasynth_config::FileFormat],
) -> Result<(), Box<dyn std::error::Error>> {
    let effective = root.effective_dir();
    write_all_output_with_layout(
        result,
        &effective,
        export_layout,
        formats,
        datasynth_config::JeNetworkMethod::default(),
    )
}

/// Write all generated data with a configurable export layout and format set.
///
/// Only writes files for formats present in `formats`. If `formats` is empty,
/// writes both CSV and JSON (backward compatible). This allows skipping JSON
/// when only CSV is needed, which halves output time for large datasets.
pub fn write_all_output_with_layout(
    result: &EnhancedGenerationResult,
    output_dir: &Path,
    export_layout: datasynth_config::ExportLayout,
    formats: &[datasynth_config::FileFormat],
    je_network_method: datasynth_config::JeNetworkMethod,
) -> Result<(), Box<dyn std::error::Error>> {
    let csv_enabled = formats.is_empty()
        || formats.contains(&datasynth_config::FileFormat::Csv)
        || formats.contains(&datasynth_config::FileFormat::Parquet);
    let json_enabled = formats.is_empty()
        || formats.contains(&datasynth_config::FileFormat::Json)
        || formats.contains(&datasynth_config::FileFormat::JsonLines);
    std::fs::create_dir_all(output_dir)?;
    info!("Writing comprehensive output to: {}", output_dir.display());

    // Set flat-layout flag for all `write_json_safe` calls in this pass.
    // Scope guard ensures we reset on return (including error paths).
    struct FlatLayoutGuard;
    impl Drop for FlatLayoutGuard {
        fn drop(&mut self) {
            FLAT_LAYOUT_ACTIVE.with(|c| c.set(false));
        }
    }
    let _flat_guard = if export_layout == datasynth_config::ExportLayout::Flat {
        FLAT_LAYOUT_ACTIVE.with(|c| c.set(true));
        Some(FlatLayoutGuard)
    } else {
        None
    };

    // Set JSON skip flag so `write_json_safe` becomes a no-op when JSON not requested.
    struct SkipJsonGuard;
    impl Drop for SkipJsonGuard {
        fn drop(&mut self) {
            SKIP_JSON.with(|c| c.set(false));
        }
    }
    let _skip_json_guard = if !json_enabled {
        SKIP_JSON.with(|c| c.set(true));
        info!("JSON output skipped (not in requested formats)");
        Some(SkipJsonGuard)
    } else {
        None
    };

    // ========================================================================
    // Journal Entries (CSV + JSON in parallel when both enabled)
    // ========================================================================
    if !result.journal_entries.is_empty() {
        let do_csv = csv_enabled;
        let do_json = json_enabled;
        let is_flat = export_layout == datasynth_config::ExportLayout::Flat;

        std::thread::scope(|s| {
            if do_csv {
                s.spawn(|| {
                    if let Err(e) = write_journal_entries_csv(result, output_dir) {
                        warn!("Failed to write journal_entries.csv: {}", e);
                    }
                });
                // v5.8.0 — flat edge-list for accounting-network construction.
                // Always emit when CSV is requested; cheap relative to the
                // main JE table.
                s.spawn(|| {
                    if let Err(e) = write_je_network_csv(result, output_dir, je_network_method) {
                        warn!("Failed to write graphs/je_network.csv: {}", e);
                    }
                });
            }
            if do_json {
                s.spawn(|| {
                    if is_flat {
                        if let Err(e) = write_journal_entries_flat_json(result, output_dir) {
                            warn!("Failed to write flat journal_entries.json: {}", e);
                        }
                    } else if let Err(e) = write_json(
                        &result.journal_entries,
                        &output_dir.join("journal_entries.json"),
                        "Journal entries (JSON)",
                    ) {
                        warn!("Failed to write journal_entries.json: {}", e);
                    }
                });
            }
        });
    }

    // ========================================================================
    // Master Data
    // ========================================================================
    let md_dir = output_dir.join("master_data");
    if !result.master_data.vendors.is_empty()
        || !result.master_data.customers.is_empty()
        || !result.master_data.materials.is_empty()
        || !result.master_data.assets.is_empty()
        || !result.master_data.employees.is_empty()
        || !result.master_data.cost_centers.is_empty()
        || !result.master_data.profit_centers.is_empty()
    {
        std::fs::create_dir_all(&md_dir)?;
        info!("Writing master data...");

        write_json_safe(
            &result.master_data.vendors,
            &md_dir.join("vendors.json"),
            "Vendors",
        );
        write_json_safe(
            &result.master_data.customers,
            &md_dir.join("customers.json"),
            "Customers",
        );
        write_json_safe(
            &result.master_data.materials,
            &md_dir.join("materials.json"),
            "Materials",
        );
        write_json_safe(
            &result.master_data.assets,
            &md_dir.join("fixed_assets.json"),
            "Fixed assets",
        );
        write_json_safe(
            &result.master_data.employees,
            &md_dir.join("employees.json"),
            "Employees",
        );
        write_json_safe(
            &result.master_data.cost_centers,
            &md_dir.join("cost_centers.json"),
            "Cost centers",
        );
        // v5.1: profit-centre hierarchy (segments + sub-units).
        write_json_safe(
            &result.master_data.profit_centers,
            &md_dir.join("profit_centers.json"),
            "Profit centres",
        );
        // v3.3.0: organizational profiles (one per company)
        write_json_safe(
            &result.master_data.organizational_profiles,
            &md_dir.join("organizational_profiles.json"),
            "Organizational profiles (v3.3.0)",
        );
    }

    // ========================================================================
    // Document Flows
    // ========================================================================
    let df_dir = output_dir.join("document_flows");
    let flat_mode = export_layout == datasynth_config::ExportLayout::Flat;
    if !result.document_flows.purchase_orders.is_empty()
        || !result.document_flows.sales_orders.is_empty()
    {
        std::fs::create_dir_all(&df_dir)?;
        info!("Writing document flows...");

        write_json_auto(
            &result.document_flows.purchase_orders,
            &df_dir.join("purchase_orders.json"),
            "Purchase orders",
            flat_mode,
        );
        write_json_auto(
            &result.document_flows.goods_receipts,
            &df_dir.join("goods_receipts.json"),
            "Goods receipts",
            flat_mode,
        );
        write_json_auto(
            &result.document_flows.vendor_invoices,
            &df_dir.join("vendor_invoices.json"),
            "Vendor invoices",
            flat_mode,
        );
        write_json_auto(
            &result.document_flows.payments,
            &df_dir.join("payments.json"),
            "Payments",
            flat_mode,
        );
        let customer_receipts: Vec<_> = result
            .document_flows
            .payments
            .iter()
            .filter(|p| p.payment_type == PaymentType::ArReceipt)
            .collect();
        write_json_auto(
            &customer_receipts,
            &df_dir.join("customer_receipts.json"),
            "Customer receipts",
            flat_mode,
        );
        write_json_auto(
            &result.document_flows.sales_orders,
            &df_dir.join("sales_orders.json"),
            "Sales orders",
            flat_mode,
        );
        write_json_auto(
            &result.document_flows.deliveries,
            &df_dir.join("deliveries.json"),
            "Deliveries",
            flat_mode,
        );
        write_json_auto(
            &result.document_flows.customer_invoices,
            &df_dir.join("customer_invoices.json"),
            "Customer invoices",
            flat_mode,
        );

        // Document cross-references (PO→GR, GR→Invoice, Invoice→Payment, etc.).
        // v4.4.2+: inject SDK-friendly `from_type`/`from_id`/`to_type`/`to_id`
        // aliases so consumers that follow the graph convention see the
        // types populated. The canonical `source_doc_*`/`target_doc_*`
        // keys continue to emit unchanged for backwards compatibility.
        match serde_json::to_value(&result.document_flows.document_references) {
            Ok(mut v) => {
                if let Some(arr) = v.as_array_mut() {
                    for r in arr.iter_mut() {
                        if let Some(obj) = r.as_object_mut() {
                            if let Some(st) = obj.get("source_doc_type").cloned() {
                                obj.entry("from_type").or_insert(st);
                            }
                            if let Some(si) = obj.get("source_doc_id").cloned() {
                                obj.entry("from_id").or_insert(si);
                            }
                            if let Some(tt) = obj.get("target_doc_type").cloned() {
                                obj.entry("to_type").or_insert(tt);
                            }
                            if let Some(ti) = obj.get("target_doc_id").cloned() {
                                obj.entry("to_id").or_insert(ti);
                            }
                        }
                    }
                }
                match serde_json::to_string_pretty(&v) {
                    Ok(json) => {
                        let path = df_dir.join("document_references.json");
                        if let Err(e) = std::fs::write(&path, json) {
                            warn!("Failed to write document references: {}", e);
                        } else {
                            info!(
                                "  Document references written: {} records -> {}",
                                result.document_flows.document_references.len(),
                                path.display()
                            );
                        }
                    }
                    Err(e) => warn!("Failed to serialize document references: {}", e),
                }
            }
            Err(e) => warn!("Failed to build document references Value: {}", e),
        }

        // Note: P2P/O2C chain types do not implement Serialize, so we log
        // their counts instead. The individual documents above capture all data.
        if !result.document_flows.p2p_chains.is_empty() {
            info!(
                "  P2P chains: {} (data exported via individual document files)",
                result.document_flows.p2p_chains.len()
            );
        }
        if !result.document_flows.o2c_chains.is_empty() {
            info!(
                "  O2C chains: {} (data exported via individual document files)",
                result.document_flows.o2c_chains.len()
            );
        }
    }

    // ========================================================================
    // Subledger
    // ========================================================================
    let sl_dir = output_dir.join("subledger");
    if !result.subledger.ap_invoices.is_empty()
        || !result.subledger.ar_invoices.is_empty()
        || !result.subledger.fa_records.is_empty()
        || !result.subledger.inventory_positions.is_empty()
    {
        std::fs::create_dir_all(&sl_dir)?;
        info!("Writing subledger data...");

        write_json_safe(
            &result.subledger.ap_invoices,
            &sl_dir.join("ap_invoices.json"),
            "AP invoices",
        );
        write_json_safe(
            &result.subledger.ar_invoices,
            &sl_dir.join("ar_invoices.json"),
            "AR invoices",
        );
        write_json_safe(
            &result.subledger.fa_records,
            &sl_dir.join("fa_records.json"),
            "FA records",
        );
        write_json_safe(
            &result.subledger.inventory_positions,
            &sl_dir.join("inventory_positions.json"),
            "Inventory positions",
        );
        write_json_safe(
            &result.subledger.inventory_movements,
            &sl_dir.join("inventory_movements.json"),
            "Inventory movements",
        );
        write_json_safe(
            &result.subledger.ar_aging_reports,
            &sl_dir.join("ar_aging.json"),
            "AR aging reports",
        );
        write_json_safe(
            &result.subledger.ap_aging_reports,
            &sl_dir.join("ap_aging.json"),
            "AP aging reports",
        );
        write_json_safe(
            &result.subledger.depreciation_runs,
            &sl_dir.join("depreciation_runs.json"),
            "Depreciation runs",
        );
        write_json_safe(
            &result.subledger.inventory_valuations,
            &sl_dir.join("inventory_valuation.json"),
            "Inventory valuations",
        );
        // Dunning runs and letters (generated after AR aging)
        write_json_safe(
            &result.subledger.dunning_runs,
            &sl_dir.join("dunning_runs.json"),
            "Dunning runs",
        );
        write_json_safe(
            &result.subledger.dunning_letters,
            &sl_dir.join("dunning_letters.json"),
            "Dunning letters",
        );
    }

    // ========================================================================
    // Audit
    // ========================================================================
    let audit_dir = output_dir.join("audit");
    if !result.audit.engagements.is_empty() {
        std::fs::create_dir_all(&audit_dir)?;
        info!("Writing audit data...");

        write_json_safe(
            &result.audit.engagements,
            &audit_dir.join("audit_engagements.json"),
            "Audit engagements",
        );
        write_json_safe(
            &result.audit.audit_scopes,
            &audit_dir.join("audit_scopes.json"),
            "Audit scopes (ISA 220 / ISA 300)",
        );
        write_json_safe(
            &result.audit.workpapers,
            &audit_dir.join("audit_workpapers.json"),
            "Audit workpapers",
        );
        write_json_safe(
            &result.audit.evidence,
            &audit_dir.join("audit_evidence.json"),
            "Audit evidence",
        );
        write_json_safe(
            &result.audit.risk_assessments,
            &audit_dir.join("audit_risk_assessments.json"),
            "Audit risk assessments",
        );
        write_json_safe(
            &result.audit.findings,
            &audit_dir.join("audit_findings.json"),
            "Audit findings",
        );
        write_json_safe(
            &result.audit.judgments,
            &audit_dir.join("audit_judgments.json"),
            "Audit judgments",
        );
        write_json_safe(
            &result.audit.confirmations,
            &audit_dir.join("audit_confirmations.json"),
            "Audit confirmations",
        );
        write_json_safe(
            &result.audit.confirmation_responses,
            &audit_dir.join("audit_confirmation_responses.json"),
            "Audit confirmation responses",
        );
        write_json_safe(
            &result.audit.procedure_steps,
            &audit_dir.join("audit_procedure_steps.json"),
            "Audit procedure steps",
        );
        write_json_safe(
            &result.audit.samples,
            &audit_dir.join("audit_samples.json"),
            "Audit samples",
        );
        write_json_safe(
            &result.audit.analytical_results,
            &audit_dir.join("audit_analytical_results.json"),
            "Audit analytical results",
        );
        write_json_safe(
            &result.audit.ia_functions,
            &audit_dir.join("audit_ia_functions.json"),
            "Audit IA functions",
        );
        write_json_safe(
            &result.audit.ia_reports,
            &audit_dir.join("audit_ia_reports.json"),
            "Audit IA reports",
        );
        write_json_safe(
            &result.audit.related_parties,
            &audit_dir.join("audit_related_parties.json"),
            "Audit related parties",
        );
        write_json_safe(
            &result.audit.related_party_transactions,
            &audit_dir.join("audit_related_party_transactions.json"),
            "Audit related party transactions",
        );
        // ISA 600: Group audit artefacts
        if !result.audit.component_auditors.is_empty() {
            write_json_safe(
                &result.audit.component_auditors,
                &audit_dir.join("component_auditors.json"),
                "Component auditors (ISA 600)",
            );
            if let Some(plan) = &result.audit.group_audit_plan {
                write_json_single_safe(
                    plan,
                    &audit_dir.join("group_audit_plan.json"),
                    "Group audit plan (ISA 600)",
                );
            }
            write_json_safe(
                &result.audit.component_instructions,
                &audit_dir.join("component_instructions.json"),
                "Component instructions (ISA 600)",
            );
            write_json_safe(
                &result.audit.component_reports,
                &audit_dir.join("component_reports.json"),
                "Component auditor reports (ISA 600)",
            );
        }
        // ISA 210: Engagement letters
        write_json_safe(
            &result.audit.engagement_letters,
            &audit_dir.join("engagement_letters.json"),
            "Engagement letters (ISA 210)",
        );
        // ISA 560 / IAS 10: Subsequent events
        write_json_safe(
            &result.audit.subsequent_events,
            &audit_dir.join("subsequent_events.json"),
            "Subsequent events (ISA 560 / IAS 10)",
        );
        // ISA 402: Service organization controls
        write_json_safe(
            &result.audit.service_organizations,
            &audit_dir.join("service_organizations.json"),
            "Service organizations (ISA 402)",
        );
        write_json_safe(
            &result.audit.soc_reports,
            &audit_dir.join("soc_reports.json"),
            "SOC reports (ISA 402)",
        );
        write_json_safe(
            &result.audit.user_entity_controls,
            &audit_dir.join("user_entity_controls.json"),
            "User entity controls (ISA 402)",
        );

        // ISA 570: Going concern assessments
        write_json_safe(
            &result.audit.going_concern_assessments,
            &audit_dir.join("going_concern_assessments.json"),
            "Going concern assessments (ISA 570)",
        );

        // ISA 540: Accounting estimates
        write_json_safe(
            &result.audit.accounting_estimates,
            &audit_dir.join("accounting_estimates.json"),
            "Accounting estimates (ISA 540)",
        );

        // ISA 700/701/705/706: Audit opinions and Key Audit Matters.
        // Always write even if the vec is empty; see the always-emit block
        // below (outside the `engagements.is_empty()` guard) for the case
        // where audit is entirely disabled — the files still appear in the
        // archive with `[]` so SDK consumers don't get 404s on the manifest.
        write_json_always(
            &result.audit.audit_opinions,
            &audit_dir.join("audit_opinions.json"),
            "Audit opinions (ISA 700/705/706)",
        );
        write_json_always(
            &result.audit.key_audit_matters,
            &audit_dir.join("key_audit_matters.json"),
            "Key Audit Matters (ISA 701)",
        );

        // SOX 302 / 404
        if !result.audit.sox_302_certifications.is_empty() {
            write_json_safe(
                &result.audit.sox_302_certifications,
                &audit_dir.join("sox_302_certifications.json"),
                "SOX 302 certifications",
            );
            write_json_safe(
                &result.audit.sox_404_assessments,
                &audit_dir.join("sox_404_assessments.json"),
                "SOX 404 ICFR assessments",
            );
        }

        // ISA 320: Materiality calculations
        if !result.audit.materiality_calculations.is_empty() {
            write_json_safe(
                &result.audit.materiality_calculations,
                &audit_dir.join("materiality_calculations.json"),
                "Materiality calculations (ISA 320)",
            );
        }

        // ISA 315: Combined Risk Assessments
        if !result.audit.combined_risk_assessments.is_empty() {
            write_json_safe(
                &result.audit.combined_risk_assessments,
                &audit_dir.join("combined_risk_assessments.json"),
                "Combined Risk Assessments (ISA 315)",
            );
        }

        // ISA 530: Sampling Plans and Sampled Items
        if !result.audit.sampling_plans.is_empty() {
            write_json_safe(
                &result.audit.sampling_plans,
                &audit_dir.join("sampling_plans.json"),
                "Sampling plans (ISA 530)",
            );
            write_json_safe(
                &result.audit.sampled_items,
                &audit_dir.join("sampled_items.json"),
                "Sampled items (ISA 530)",
            );
        }

        // ISA 315: Significant Classes of Transactions (SCOTS)
        if !result.audit.significant_transaction_classes.is_empty() {
            write_json_safe(
                &result.audit.significant_transaction_classes,
                &audit_dir.join("significant_transaction_classes.json"),
                "Significant Classes of Transactions / SCOTS (ISA 315)",
            );
        }

        // ISA 520: Unusual Item Markers
        if !result.audit.unusual_items.is_empty() {
            write_json_safe(
                &result.audit.unusual_items,
                &audit_dir.join("unusual_items.json"),
                "Unusual item flags (ISA 520)",
            );
        }

        // ISA 520: Analytical Relationships
        if !result.audit.analytical_relationships.is_empty() {
            write_json_safe(
                &result.audit.analytical_relationships,
                &audit_dir.join("analytical_relationships.json"),
                "Analytical relationships (ISA 520)",
            );
        }

        // PCAOB-ISA cross-reference mappings
        if !result.audit.isa_pcaob_mappings.is_empty() {
            write_json_safe(
                &result.audit.isa_pcaob_mappings,
                &audit_dir.join("isa_pcaob_mappings.json"),
                "PCAOB-ISA standard mappings",
            );
        }

        // ISA standard reference (number, title, series for all 34 ISA standards)
        if !result.audit.isa_mappings.is_empty() {
            write_json_safe(
                &result.audit.isa_mappings,
                &audit_dir.join("isa_mappings.json"),
                "ISA standard reference mappings",
            );
        }

        // FSM event trail (when audit.fsm.enabled: true)
        if let Some(ref event_trail) = result.audit.fsm_event_trail {
            if !event_trail.is_empty() {
                write_json_safe(
                    event_trail,
                    &audit_dir.join("fsm_event_trail.json"),
                    "FSM audit event trail",
                );
            }
        }

        // v3.3.0: legal documents (when compliance_regulations.legal_documents.enabled)
        write_json_safe(
            &result.audit.legal_documents,
            &audit_dir.join("legal_documents.json"),
            "Legal documents (v3.3.0)",
        );

        // v3.3.0: IT general controls — access logs + change records
        write_json_safe(
            &result.audit.it_controls_access_logs,
            &audit_dir.join("it_controls_access_logs.json"),
            "IT general controls — access logs (v3.3.0)",
        );
        write_json_safe(
            &result.audit.it_controls_change_records,
            &audit_dir.join("it_controls_change_records.json"),
            "IT general controls — change management records (v3.3.0)",
        );
    } else {
        // Audit phase disabled or ran with no engagements — still emit
        // audit_opinions.json + key_audit_matters.json so the archive
        // structure is consistent and SDK consumers can rely on these
        // files always existing. v3.1 announced these as archive-shipping
        // files; v3.1.1 guarantees it regardless of audit.enabled.
        std::fs::create_dir_all(&audit_dir)?;
        write_json_always(
            &result.audit.audit_opinions,
            &audit_dir.join("audit_opinions.json"),
            "Audit opinions (ISA 700/705/706) — empty (audit phase disabled)",
        );
        write_json_always(
            &result.audit.key_audit_matters,
            &audit_dir.join("key_audit_matters.json"),
            "Key Audit Matters (ISA 701) — empty (audit phase disabled)",
        );
    }

    // ========================================================================
    // Banking (JSON - keep existing format for backward compat)
    // ========================================================================
    let banking_dir = output_dir.join("banking");
    if !result.banking.customers.is_empty() {
        std::fs::create_dir_all(&banking_dir)?;
        info!("Writing banking data...");

        // v4.4.2: dual-key risk tier. SDK consumers inspect `risk_level`;
        // the struct stores it as `risk_tier` for historical reasons.
        // Serialize through a `serde_json::Value` so we can inject the
        // `risk_level` alias key on every customer row without touching
        // the `BankingCustomer` Serialize impl (which has 40+ fields).
        match serde_json::to_value(&result.banking.customers) {
            Ok(mut v) => {
                if let Some(arr) = v.as_array_mut() {
                    for c in arr.iter_mut() {
                        if let Some(obj) = c.as_object_mut() {
                            if let Some(rt) = obj.get("risk_tier").cloned() {
                                obj.entry("risk_level").or_insert(rt);
                            }
                        }
                    }
                }
                match serde_json::to_string_pretty(&v) {
                    Ok(json) => {
                        let path = banking_dir.join("banking_customers.json");
                        if let Err(e) = std::fs::write(&path, json) {
                            warn!("Failed to write banking_customers.json: {}", e);
                        } else {
                            info!(
                                "  Banking customers written: {} records -> {}",
                                result.banking.customers.len(),
                                path.display()
                            );
                        }
                    }
                    Err(e) => warn!("Failed to serialize banking customers: {}", e),
                }
            }
            Err(e) => warn!("Failed to build banking customers Value: {}", e),
        }
        write_json_safe(
            &result.banking.accounts,
            &banking_dir.join("banking_accounts.json"),
            "Banking accounts",
        );
        write_json_safe(
            &result.banking.transactions,
            &banking_dir.join("banking_transactions.json"),
            "Banking transactions",
        );
        write_json_safe(
            &result.banking.transaction_labels,
            &banking_dir.join("aml_transaction_labels.json"),
            "AML transaction labels",
        );
        write_json_safe(
            &result.banking.customer_labels,
            &banking_dir.join("aml_customer_labels.json"),
            "AML customer labels",
        );
        write_json_safe(
            &result.banking.account_labels,
            &banking_dir.join("aml_account_labels.json"),
            "AML account labels",
        );
        write_json_safe(
            &result.banking.relationship_labels,
            &banking_dir.join("aml_relationship_labels.json"),
            "AML relationship labels",
        );
        write_json_safe(
            &result.banking.narratives,
            &banking_dir.join("aml_narratives.json"),
            "AML narratives",
        );
    }

    // ========================================================================
    // Sourcing (S2C)
    // ========================================================================
    let s2c_dir = output_dir.join("sourcing");
    if !result.sourcing.spend_analyses.is_empty() || !result.sourcing.sourcing_projects.is_empty() {
        std::fs::create_dir_all(&s2c_dir)?;
        info!("Writing sourcing (S2C) data...");

        write_json_safe(
            &result.sourcing.spend_analyses,
            &s2c_dir.join("spend_analyses.json"),
            "Spend analyses",
        );
        write_json_safe(
            &result.sourcing.sourcing_projects,
            &s2c_dir.join("sourcing_projects.json"),
            "Sourcing projects",
        );
        write_json_safe(
            &result.sourcing.qualifications,
            &s2c_dir.join("supplier_qualifications.json"),
            "Supplier qualifications",
        );
        write_json_safe(
            &result.sourcing.rfx_events,
            &s2c_dir.join("rfx_events.json"),
            "RFx events",
        );
        write_json_safe(
            &result.sourcing.bids,
            &s2c_dir.join("supplier_bids.json"),
            "Supplier bids",
        );
        write_json_safe(
            &result.sourcing.bid_evaluations,
            &s2c_dir.join("bid_evaluations.json"),
            "Bid evaluations",
        );
        write_json_safe(
            &result.sourcing.contracts,
            &s2c_dir.join("procurement_contracts.json"),
            "Procurement contracts",
        );
        write_json_safe(
            &result.sourcing.catalog_items,
            &s2c_dir.join("catalog_items.json"),
            "Catalog items",
        );
        write_json_safe(
            &result.sourcing.scorecards,
            &s2c_dir.join("supplier_scorecards.json"),
            "Supplier scorecards",
        );
    }

    // ========================================================================
    // Intercompany
    // ========================================================================
    let ic_dir = output_dir.join("intercompany");
    if result.intercompany.group_structure.is_some()
        || !result.intercompany.matched_pairs.is_empty()
    {
        std::fs::create_dir_all(&ic_dir)?;
        info!("Writing intercompany data...");

        // Always write group structure when present (independent of IC transactions).
        if let Some(gs) = &result.intercompany.group_structure {
            write_json_single_safe(gs, &ic_dir.join("group_structure.json"), "Group structure");
        }

        write_json_safe(
            &result.intercompany.matched_pairs,
            &ic_dir.join("ic_matched_pairs.json"),
            "IC matched pairs",
        );
        write_json_safe(
            &result.intercompany.seller_journal_entries,
            &ic_dir.join("ic_seller_journal_entries.json"),
            "IC seller journal entries",
        );
        write_json_safe(
            &result.intercompany.buyer_journal_entries,
            &ic_dir.join("ic_buyer_journal_entries.json"),
            "IC buyer journal entries",
        );
        write_json_safe(
            &result.intercompany.elimination_entries,
            &ic_dir.join("ic_elimination_entries.json"),
            "IC elimination entries",
        );

        // NCI measurements from group structure ownership percentages
        if !result.intercompany.nci_measurements.is_empty() {
            write_json_safe(
                &result.intercompany.nci_measurements,
                &ic_dir.join("nci_measurements.json"),
                "NCI measurements",
            );
        }
    }

    // ========================================================================
    // Financial Reporting
    // ========================================================================
    let fin_dir = output_dir.join("financial_reporting");
    if !result.financial_reporting.financial_statements.is_empty()
        || !result.financial_reporting.bank_reconciliations.is_empty()
        || !result
            .financial_reporting
            .consolidated_statements
            .is_empty()
    {
        std::fs::create_dir_all(&fin_dir)?;
        info!("Writing financial reporting data...");

        // Legacy flat file (all standalone statements combined)
        write_json_safe(
            &result.financial_reporting.financial_statements,
            &fin_dir.join("financial_statements.json"),
            "Financial statements",
        );

        // Per-entity standalone statements
        if !result.financial_reporting.standalone_statements.is_empty() {
            let standalone_dir = fin_dir.join("standalone");
            std::fs::create_dir_all(&standalone_dir)?;
            for (entity_code, stmts) in &result.financial_reporting.standalone_statements {
                let file_name = format!("{}_financial_statements.json", entity_code);
                write_json_safe(
                    stmts,
                    &standalone_dir.join(&file_name),
                    &format!("Standalone statements for {}", entity_code),
                );
            }
        }

        // Consolidated statements + schedule
        if !result
            .financial_reporting
            .consolidated_statements
            .is_empty()
            || !result
                .financial_reporting
                .consolidation_schedules
                .is_empty()
        {
            let consolidated_dir = fin_dir.join("consolidated");
            std::fs::create_dir_all(&consolidated_dir)?;
            write_json_safe(
                &result.financial_reporting.consolidated_statements,
                &consolidated_dir.join("consolidated_financial_statements.json"),
                "Consolidated financial statements",
            );
            write_json_safe(
                &result.financial_reporting.consolidation_schedules,
                &consolidated_dir.join("consolidation_schedule.json"),
                "Consolidation schedule",
            );
        }

        write_json_safe(
            &result.financial_reporting.bank_reconciliations,
            &fin_dir.join("bank_reconciliations.json"),
            "Bank reconciliations",
        );

        // IFRS 8 / ASC 280 Segment Reporting
        if !result.financial_reporting.segment_reports.is_empty()
            || !result
                .financial_reporting
                .segment_reconciliations
                .is_empty()
        {
            let seg_dir = fin_dir.join("segment_reporting");
            std::fs::create_dir_all(&seg_dir)?;
            write_json_safe(
                &result.financial_reporting.segment_reports,
                &seg_dir.join("segment_reports.json"),
                "Segment reports",
            );
            write_json_safe(
                &result.financial_reporting.segment_reconciliations,
                &seg_dir.join("segment_reconciliations.json"),
                "Segment reconciliations",
            );
        }

        // IAS 1 / ASC 235: Notes to financial statements
        write_json_safe(
            &result.financial_reporting.notes_to_financial_statements,
            &fin_dir.join("notes_to_financial_statements.json"),
            "Notes to financial statements",
        );
    }

    // ========================================================================
    // Period-Close Trial Balances
    // ========================================================================
    //
    // v5.1: convert each in-memory `PeriodTrialBalance` to the
    // canonical `datasynth_core::models::balance::TrialBalance` before
    // writing.  The on-disk shape is now identical to what the group
    // aggregate phase loads via `tb_loader::load_entity_trial_balance`,
    // so the loader's v5.0 dual-shape detection (`PeriodTrialBalanceOnDisk`
    // → `TrialBalance` synthesis) is no longer required.
    if !result.financial_reporting.trial_balances.is_empty() {
        let pc_dir = output_dir.join("period_close");
        std::fs::create_dir_all(&pc_dir)?;
        info!(
            "Writing {} period-close trial balances...",
            result.financial_reporting.trial_balances.len()
        );
        // Pick the first JE's company_code + currency as the
        // canonical identifiers; the orchestrator only emits one TB
        // per period (gated by `if company_idx == 0` at the push
        // site), so all trial-balance entries belong to that company.
        // Fallback to safe defaults when the JE list is empty
        // (effectively only test fixtures).
        let (company_code, currency) = result
            .journal_entries
            .first()
            .map(|je| (je.header.company_code.as_str(), je.header.currency.as_str()))
            .unwrap_or(("UNKNOWN", "USD"));
        let canonical: Vec<datasynth_core::models::balance::TrialBalance> = result
            .financial_reporting
            .trial_balances
            .iter()
            .cloned()
            .map(|tb| tb.into_canonical(company_code, currency))
            .collect();
        write_json_safe(
            &canonical,
            &pc_dir.join("trial_balances.json"),
            "Period-close trial balances (canonical)",
        );
    }

    // ========================================================================
    // Balance: Opening Balances + GL-Subledger Reconciliation
    // ========================================================================
    if !result.opening_balances.is_empty() || !result.subledger_reconciliation.is_empty() {
        let balance_dir = output_dir.join("balance");
        std::fs::create_dir_all(&balance_dir)?;
        info!("Writing balance data...");

        write_json_safe(
            &result.opening_balances,
            &balance_dir.join("opening_balances.json"),
            "Opening balances",
        );
        write_json_safe(
            &result.subledger_reconciliation,
            &balance_dir.join("subledger_reconciliation.json"),
            "Subledger reconciliation",
        );
    }

    // ========================================================================
    // HR (Payroll, Time Entries, Expense Reports, Benefit Enrollments, Pensions)
    // ========================================================================
    let hr_dir = output_dir.join("hr");
    if !result.hr.payroll_runs.is_empty()
        || !result.hr.time_entries.is_empty()
        || !result.hr.expense_reports.is_empty()
        || !result.hr.benefit_enrollments.is_empty()
        || !result.hr.pension_plans.is_empty()
        || !result.hr.stock_grants.is_empty()
        || !result.master_data.employee_change_history.is_empty()
    {
        std::fs::create_dir_all(&hr_dir)?;
        info!("Writing HR data...");

        write_json_safe(
            &result.hr.payroll_runs,
            &hr_dir.join("payroll_runs.json"),
            "Payroll runs",
        );
        write_json_safe(
            &result.hr.payroll_line_items,
            &hr_dir.join("payroll_line_items.json"),
            "Payroll line items",
        );
        write_json_safe(
            &result.hr.time_entries,
            &hr_dir.join("time_entries.json"),
            "Time entries",
        );
        write_json_safe(
            &result.hr.expense_reports,
            &hr_dir.join("expense_reports.json"),
            "Expense reports",
        );
        write_json_safe(
            &result.hr.benefit_enrollments,
            &hr_dir.join("benefit_enrollments.json"),
            "Benefit enrollments",
        );
        write_json_safe(
            &result.hr.pension_plans,
            &hr_dir.join("pension_plans.json"),
            "Pension plans",
        );
        write_json_safe(
            &result.hr.pension_obligations,
            &hr_dir.join("pension_obligations.json"),
            "Pension obligations",
        );
        write_json_safe(
            &result.hr.pension_plan_assets,
            &hr_dir.join("plan_assets.json"),
            "Plan assets",
        );
        write_json_safe(
            &result.hr.pension_disclosures,
            &hr_dir.join("pension_disclosures.json"),
            "Pension disclosures",
        );
        write_json_safe(
            &result.hr.stock_grants,
            &hr_dir.join("stock_grants.json"),
            "Stock grants",
        );
        write_json_safe(
            &result.hr.stock_comp_expenses,
            &hr_dir.join("stock_comp_expense.json"),
            "Stock comp expense",
        );
        write_json_safe(
            &result.master_data.employee_change_history,
            &hr_dir.join("employee_change_history.json"),
            "Employee change history",
        );
    }

    // ========================================================================
    // Manufacturing
    // ========================================================================
    let mfg_dir = output_dir.join("manufacturing");
    if !result.manufacturing.production_orders.is_empty()
        || !result.manufacturing.quality_inspections.is_empty()
        || !result.manufacturing.cycle_counts.is_empty()
        || !result.manufacturing.bom_components.is_empty()
        || !result.manufacturing.inventory_movements.is_empty()
    {
        std::fs::create_dir_all(&mfg_dir)?;
        info!("Writing manufacturing data...");

        write_json_safe(
            &result.manufacturing.production_orders,
            &mfg_dir.join("production_orders.json"),
            "Production orders",
        );
        write_json_safe(
            &result.manufacturing.quality_inspections,
            &mfg_dir.join("quality_inspections.json"),
            "Quality inspections",
        );
        write_json_safe(
            &result.manufacturing.cycle_counts,
            &mfg_dir.join("cycle_counts.json"),
            "Cycle counts",
        );
        write_json_safe(
            &result.manufacturing.bom_components,
            &mfg_dir.join("bom_components.json"),
            "BOM components",
        );
        write_json_safe(
            &result.manufacturing.inventory_movements,
            &mfg_dir.join("inventory_movements.json"),
            "Inventory movements",
        );
    }

    // ========================================================================
    // Sales, KPIs, Budgets
    // ========================================================================
    let sales_dir = output_dir.join("sales_kpi_budgets");
    if !result.sales_kpi_budgets.sales_quotes.is_empty()
        || !result.sales_kpi_budgets.kpis.is_empty()
        || !result.sales_kpi_budgets.budgets.is_empty()
    {
        std::fs::create_dir_all(&sales_dir)?;
        info!("Writing sales, KPI, and budget data...");

        write_json_safe(
            &result.sales_kpi_budgets.sales_quotes,
            &sales_dir.join("sales_quotes.json"),
            "Sales quotes",
        );
        write_json_safe(
            &result.sales_kpi_budgets.kpis,
            &sales_dir.join("management_kpis.json"),
            "Management KPIs",
        );
        write_json_safe(
            &result.sales_kpi_budgets.budgets,
            &sales_dir.join("budgets.json"),
            "Budgets",
        );
    }

    // ========================================================================
    // Tax
    // ========================================================================
    let tax_dir = output_dir.join("tax");
    if !result.tax.jurisdictions.is_empty()
        || !result.tax.codes.is_empty()
        || !result.tax.tax_provisions.is_empty()
    {
        std::fs::create_dir_all(&tax_dir)?;
        info!("Writing tax data...");

        write_json_safe(
            &result.tax.jurisdictions,
            &tax_dir.join("tax_jurisdictions.json"),
            "Tax jurisdictions",
        );
        write_json_safe(
            &result.tax.codes,
            &tax_dir.join("tax_codes.json"),
            "Tax codes",
        );
        write_json_safe(
            &result.tax.tax_provisions,
            &tax_dir.join("tax_provisions.json"),
            "Tax provisions",
        );
        write_json_safe(
            &result.tax.tax_lines,
            &tax_dir.join("tax_lines.json"),
            "Tax lines",
        );
        write_json_safe(
            &result.tax.tax_returns,
            &tax_dir.join("tax_returns.json"),
            "Tax returns",
        );
        write_json_safe(
            &result.tax.withholding_records,
            &tax_dir.join("withholding_records.json"),
            "Withholding tax records",
        );
        if !result.tax.tax_anomaly_labels.is_empty() {
            write_json_safe(
                &result.tax.tax_anomaly_labels,
                &tax_dir.join("tax_anomaly_labels.json"),
                "Tax anomaly labels",
            );
        }
        // Deferred tax engine output (IAS 12 / ASC 740)
        if !result.tax.deferred_tax.temporary_differences.is_empty() {
            write_json_safe(
                &result.tax.deferred_tax.temporary_differences,
                &tax_dir.join("temporary_differences.json"),
                "Temporary differences",
            );
            write_json_safe(
                &result.tax.deferred_tax.etr_reconciliations,
                &tax_dir.join("etr_reconciliation.json"),
                "ETR reconciliation",
            );
            write_json_safe(
                &result.tax.deferred_tax.rollforwards,
                &tax_dir.join("deferred_tax_rollforward.json"),
                "Deferred tax rollforward",
            );
            write_json_safe(
                &result.tax.deferred_tax.journal_entries,
                &tax_dir.join("deferred_tax_journal_entries.json"),
                "Deferred tax journal entries",
            );
        }
    }

    // ========================================================================
    // ESG
    // ========================================================================
    let esg_dir = output_dir.join("esg");
    if !result.esg.emissions.is_empty()
        || !result.esg.energy.is_empty()
        || !result.esg.diversity.is_empty()
        || !result.esg.governance.is_empty()
    {
        std::fs::create_dir_all(&esg_dir)?;
        info!("Writing ESG data...");

        write_json_safe(
            &result.esg.emissions,
            &esg_dir.join("emission_records.json"),
            "Emission records",
        );
        write_json_safe(
            &result.esg.energy,
            &esg_dir.join("energy_consumption.json"),
            "Energy consumption",
        );
        write_json_safe(
            &result.esg.water,
            &esg_dir.join("water_usage.json"),
            "Water usage",
        );
        write_json_safe(
            &result.esg.waste,
            &esg_dir.join("waste_records.json"),
            "Waste records",
        );
        write_json_safe(
            &result.esg.diversity,
            &esg_dir.join("workforce_diversity.json"),
            "Workforce diversity",
        );
        write_json_safe(
            &result.esg.pay_equity,
            &esg_dir.join("pay_equity.json"),
            "Pay equity",
        );
        write_json_safe(
            &result.esg.safety_incidents,
            &esg_dir.join("safety_incidents.json"),
            "Safety incidents",
        );
        write_json_safe(
            &result.esg.safety_metrics,
            &esg_dir.join("safety_metrics.json"),
            "Safety metrics",
        );
        write_json_safe(
            &result.esg.governance,
            &esg_dir.join("governance_metrics.json"),
            "Governance metrics",
        );
        write_json_safe(
            &result.esg.supplier_assessments,
            &esg_dir.join("supplier_esg_assessments.json"),
            "Supplier ESG assessments",
        );
        write_json_safe(
            &result.esg.materiality,
            &esg_dir.join("materiality_assessments.json"),
            "Materiality assessments",
        );
        write_json_safe(
            &result.esg.disclosures,
            &esg_dir.join("esg_disclosures.json"),
            "ESG disclosures",
        );
        write_json_safe(
            &result.esg.climate_scenarios,
            &esg_dir.join("climate_scenarios.json"),
            "Climate scenarios",
        );
        write_json_safe(
            &result.esg.anomaly_labels,
            &esg_dir.join("esg_anomaly_labels.json"),
            "ESG anomaly labels",
        );
    }

    // ========================================================================
    // Process Mining (OCPM)
    // ========================================================================
    if let Some(ref event_log) = result.ocpm.event_log {
        if !event_log.events.is_empty() || !event_log.objects.is_empty() {
            let pm_dir = output_dir.join("process_mining");
            std::fs::create_dir_all(&pm_dir)?;
            info!("Writing process mining (OCPM) data...");

            // Write the full OCEL 2.0 event log. v4.4.2+ patches every
            // `object_refs[*].object_type_id` with a companion
            // `object_type` key, matching the OCEL 2.0 spec and SDK
            // consumer expectations that previously saw `object_type`
            // arrive as null. See `add_ocel_object_type_alias` below.
            match serde_json::to_value(event_log) {
                Ok(mut v) => {
                    add_ocel_object_type_alias(&mut v);
                    match serde_json::to_string_pretty(&v) {
                        Ok(json) => {
                            if let Err(e) = std::fs::write(pm_dir.join("event_log.json"), json) {
                                warn!("Failed to write OCPM event log: {}", e);
                            } else {
                                info!(
                                    "  Event log written: {} events, {} objects",
                                    result.ocpm.event_count, result.ocpm.object_count
                                );
                            }
                        }
                        Err(e) => warn!("Failed to serialize OCPM event log: {}", e),
                    }
                }
                Err(e) => warn!("Failed to build OCPM event log Value: {}", e),
            }

            // Write events separately for easy consumption
            if !event_log.events.is_empty() {
                match serde_json::to_string_pretty(&event_log.events) {
                    Ok(json) => {
                        if let Err(e) = std::fs::write(pm_dir.join("events.json"), json) {
                            warn!("Failed to write OCPM events: {}", e);
                        } else {
                            info!("  Events written: {} records", event_log.events.len());
                        }
                    }
                    Err(e) => warn!("Failed to serialize OCPM events: {}", e),
                }
            }

            // Write objects separately for easy consumption
            if !event_log.objects.is_empty() {
                let objects: Vec<&_> = event_log.objects.iter().collect();
                match serde_json::to_string_pretty(&objects) {
                    Ok(json) => {
                        if let Err(e) = std::fs::write(pm_dir.join("objects.json"), json) {
                            warn!("Failed to write OCPM objects: {}", e);
                        } else {
                            info!("  Objects written: {} records", event_log.objects.len());
                        }
                    }
                    Err(e) => warn!("Failed to serialize OCPM objects: {}", e),
                }
            }

            // Write process variants if any were computed
            if !event_log.variants.is_empty() {
                let variants: Vec<&_> = event_log.variants.values().collect();
                match serde_json::to_string_pretty(&variants) {
                    Ok(json) => {
                        if let Err(e) = std::fs::write(pm_dir.join("process_variants.json"), json) {
                            warn!("Failed to write process variants: {}", e);
                        } else {
                            info!(
                                "  Process variants written: {} variants",
                                event_log.variants.len()
                            );
                        }
                    }
                    Err(e) => warn!("Failed to serialize process variants: {}", e),
                }
            }
        }
    }

    // ========================================================================
    // Chart of Accounts
    // ========================================================================
    // Primary file: flat array of accounts (shape stable since v3.x —
    // consumers iterate over it).
    match serde_json::to_string_pretty(&result.chart_of_accounts.accounts) {
        Ok(json) => {
            if let Err(e) = std::fs::write(output_dir.join("chart_of_accounts.json"), json) {
                warn!("Failed to write chart of accounts: {}", e);
            } else {
                info!("  Chart of accounts written");
            }
        }
        Err(e) => warn!("Failed to serialize chart of accounts: {}", e),
    }
    // v4.4.1 — companion metadata file so SDK consumers can read the
    // accounting framework + complexity + ID without having to infer
    // them from each account row. The SDK team flagged
    // `CoA.accounting_framework` arriving as null in v4.1.x; the field
    // didn't exist at all until v4.4.1.
    let coa_meta = serde_json::json!({
        "coa_id": result.chart_of_accounts.coa_id,
        "name": result.chart_of_accounts.name,
        "country": result.chart_of_accounts.country,
        "industry": result.chart_of_accounts.industry,
        "complexity": result.chart_of_accounts.complexity,
        "account_format": result.chart_of_accounts.account_format,
        "accounting_framework": result.chart_of_accounts.accounting_framework,
        "account_count": result.chart_of_accounts.accounts.len(),
    });
    match serde_json::to_string_pretty(&coa_meta) {
        Ok(json) => {
            if let Err(e) = std::fs::write(output_dir.join("chart_of_accounts_meta.json"), json) {
                warn!("Failed to write CoA metadata: {}", e);
            } else {
                info!(
                    "  Chart of accounts metadata written (accounting_framework: {:?})",
                    result.chart_of_accounts.accounting_framework
                );
            }
        }
        Err(e) => warn!("Failed to serialize CoA metadata: {}", e),
    }

    // ========================================================================
    // Balance Validation Summary
    // ========================================================================
    if result.balance_validation.validated {
        match serde_json::to_string_pretty(&BalanceValidationSummary::from(
            &result.balance_validation,
        )) {
            Ok(json) => {
                if let Err(e) = std::fs::write(output_dir.join("balance_validation.json"), json) {
                    warn!("Failed to write balance validation: {}", e);
                } else {
                    info!("  Balance validation summary written");
                }
            }
            Err(e) => warn!("Failed to serialize balance validation: {}", e),
        }
    }

    // ========================================================================
    // Data Quality Statistics (now serializable directly via Serialize derives)
    // ========================================================================
    {
        match serde_json::to_string_pretty(&result.data_quality_stats) {
            Ok(json) => {
                if let Err(e) = std::fs::write(output_dir.join("data_quality_stats.json"), json) {
                    warn!("Failed to write data quality stats: {}", e);
                } else {
                    info!("  Data quality stats written (full detail)");
                }
            }
            Err(e) => warn!("Failed to serialize data quality stats: {}", e),
        }
    }

    // ========================================================================
    // v3.3.0: Analytics-metadata phase outputs (prior year, industry
    // benchmarks, management reports, drift events).
    // ========================================================================
    {
        let am = &result.analytics_metadata;
        if !am.prior_year_comparatives.is_empty()
            || !am.industry_benchmarks.is_empty()
            || !am.management_reports.is_empty()
            || !am.drift_events.is_empty()
        {
            let analytics_dir = output_dir.join("analytics");
            std::fs::create_dir_all(&analytics_dir)?;
            write_json_safe(
                &am.prior_year_comparatives,
                &analytics_dir.join("prior_year_comparatives.json"),
                "Prior-year comparatives (v3.3.0)",
            );
            write_json_safe(
                &am.industry_benchmarks,
                &analytics_dir.join("industry_benchmarks.json"),
                "Industry benchmarks (v3.3.0)",
            );
            write_json_safe(
                &am.management_reports,
                &analytics_dir.join("management_reports.json"),
                "Management reports (v3.3.0)",
            );
            write_json_safe(
                &am.drift_events,
                &analytics_dir.join("drift_events.json"),
                "Drift event labels (v3.3.0)",
            );
        }
    }

    // ========================================================================
    // Pre-built Analytics (Benford, amount distribution, process variants)
    // ========================================================================
    {
        let analytics_dir = output_dir.join("analytics");

        // Collect non-zero amounts from journal entry lines
        let amounts: Vec<_> = result
            .journal_entries
            .iter()
            .flat_map(|je| je.lines.iter())
            .flat_map(|line| {
                let d = (!line.debit_amount.is_zero()).then_some(line.debit_amount);
                let c = (!line.credit_amount.is_zero()).then_some(line.credit_amount);
                d.into_iter().chain(c)
            })
            .collect();

        if amounts.len() >= 10 {
            std::fs::create_dir_all(&analytics_dir)?;
            info!("Writing pre-built analytics ({} amounts)...", amounts.len());

            // Benford's Law analysis
            let benford_analyzer = datasynth_eval::BenfordAnalyzer::default();
            match benford_analyzer.analyze(&amounts) {
                Ok(ref benford_result) => {
                    if let Ok(json) = serde_json::to_string_pretty(benford_result) {
                        if let Err(e) =
                            std::fs::write(analytics_dir.join("benford_analysis.json"), json)
                        {
                            warn!("Failed to write Benford analysis: {}", e);
                        } else {
                            info!(
                                "  Benford analysis written (conformity: {:?}, MAD: {:.4})",
                                benford_result.conformity, benford_result.mad
                            );
                        }
                    }
                }
                Err(e) => warn!("Benford analysis skipped: {}", e),
            }

            // Amount distribution analysis
            let amount_analyzer = datasynth_eval::AmountDistributionAnalyzer::new();
            match amount_analyzer.analyze(&amounts) {
                Ok(ref dist_result) => {
                    if let Ok(json) = serde_json::to_string_pretty(dist_result) {
                        if let Err(e) =
                            std::fs::write(analytics_dir.join("amount_distribution.json"), json)
                        {
                            warn!("Failed to write amount distribution: {}", e);
                        } else {
                            info!(
                                "  Amount distribution written (skewness: {:.2}, kurtosis: {:.2})",
                                dist_result.skewness, dist_result.kurtosis
                            );
                        }
                    }
                }
                Err(e) => warn!("Amount distribution analysis skipped: {}", e),
            }
        }

        // Process variant summary (from OCPM event log).
        //
        // v3.1.1 — always emit the file when an event_log exists. When the
        // event_log has no pre-computed `variants` map (older OCPM phases
        // didn't populate it), derive variants on the fly from the raw
        // events so SDK consumers see `analytics/process_variant_summary.json`
        // in every archive rather than `null`. Without this, the v3.1
        // claim that the file exists was only true when OCPM happened to
        // populate its variants map.
        if let Some(ref event_log) = result.ocpm.event_log {
            std::fs::create_dir_all(&analytics_dir)?;
            let variant_data: Vec<datasynth_eval::VariantData> = if !event_log.variants.is_empty() {
                event_log
                    .variants
                    .values()
                    .map(|v| datasynth_eval::VariantData {
                        variant_id: v.variant_id.clone(),
                        case_count: v.frequency as usize,
                        is_happy_path: v.is_happy_path,
                    })
                    .collect()
            } else {
                // Fallback: derive variants from raw events by case_id.
                // Each case's activity sequence (by activity_id) defines a
                // variant; cases with the same sequence collapse into one
                // variant. Events without a case_id are skipped since they
                // can't be grouped into a process instance.
                use std::collections::HashMap;
                // Key by case_id's string form to avoid pulling the uuid
                // crate into the output writer's dependency graph.
                let mut per_case: HashMap<String, Vec<String>> = HashMap::new();
                for ev in &event_log.events {
                    if let Some(case_id) = ev.case_id {
                        per_case
                            .entry(case_id.to_string())
                            .or_default()
                            .push(ev.activity_id.clone());
                    }
                }
                let mut variant_counts: HashMap<Vec<String>, usize> = HashMap::new();
                for activities in per_case.into_values() {
                    *variant_counts.entry(activities).or_insert(0) += 1;
                }
                // Happy path heuristic: the highest-frequency variant.
                let max_count = variant_counts.values().copied().max().unwrap_or(0);
                variant_counts
                    .into_iter()
                    .enumerate()
                    .map(|(i, (seq, count))| datasynth_eval::VariantData {
                        variant_id: format!("V{i:04}:{}", seq.join("->")),
                        case_count: count,
                        is_happy_path: count == max_count && max_count > 0,
                    })
                    .collect()
            };

            let variant_analyzer = datasynth_eval::VariantAnalyzer::new();
            match variant_analyzer.analyze(&variant_data) {
                Ok(ref variant_result) => {
                    if let Ok(json) = serde_json::to_string_pretty(variant_result) {
                        if let Err(e) =
                            std::fs::write(analytics_dir.join("process_variant_summary.json"), json)
                        {
                            warn!("Failed to write variant summary: {}", e);
                        } else {
                            info!(
                                "  Process variant summary written ({} variants, entropy: {:.2})",
                                variant_result.variant_count, variant_result.variant_entropy
                            );
                        }
                    }
                }
                Err(e) => {
                    // Even on analyzer error, emit a minimal JSON placeholder
                    // so the file always exists in the archive.
                    warn!("Variant analysis failed: {}; emitting empty summary", e);
                    let placeholder = serde_json::json!({
                        "variant_count": 0,
                        "variant_entropy": null,
                        "happy_path_concentration": null,
                        "top_variants": [],
                        "passes": false,
                        "issues": [format!("analyzer error: {e}")],
                    });
                    if let Ok(json) = serde_json::to_string_pretty(&placeholder) {
                        let _ = std::fs::write(
                            analytics_dir.join("process_variant_summary.json"),
                            json,
                        );
                    }
                }
            }
        }

        // Banking evaluation (KYC completeness + AML detectability).
        // Matches the payload served by /v1/jobs/{id}/analytics so
        // archive-mode consumers see the same four files the endpoint returns.
        if !result.banking.customers.is_empty() {
            use datasynth_core::models::banking::BankingCustomerType;
            use datasynth_eval::banking::{
                AmlDetectabilityAnalyzer, AmlTransactionData, BankingEvaluation,
                KycCompletenessAnalyzer, KycProfileData, TypologyData,
            };
            use std::collections::HashMap;
            std::fs::create_dir_all(&analytics_dir)?;

            let kyc_data: Vec<KycProfileData> = result
                .banking
                .customers
                .iter()
                .map(|c| KycProfileData {
                    profile_id: c.customer_id.to_string(),
                    has_name: true,
                    has_dob: c.date_of_birth.is_some(),
                    has_address: c.address_line1.is_some(),
                    has_id_document: c.national_id.is_some() || c.passport_number.is_some(),
                    has_risk_rating: true,
                    has_beneficial_owner: !c.beneficial_owners.is_empty(),
                    is_entity: c.customer_type == BankingCustomerType::Business,
                    is_verified: c.kyc_truthful,
                })
                .collect();

            let mut banking_eval = BankingEvaluation::new();
            if let Ok(kyc_res) = KycCompletenessAnalyzer::new().analyze(&kyc_data) {
                banking_eval.kyc = Some(kyc_res);
            }

            let suspicious: Vec<&_> = result
                .banking
                .transactions
                .iter()
                .filter(|t| t.is_suspicious)
                .collect();
            if !suspicious.is_empty() {
                // Use AmlTypology::canonical_name() so the evaluator's
                // exact-string match against EXPECTED_TYPOLOGIES succeeds.
                // Prior to v3.1.1 we used `format!("{:?}", r)` (Debug /
                // PascalCase) which never matched the lowercase expected
                // names and produced "typology_coverage = 0.000" in every
                // run regardless of actual typology injection.
                let aml_data: Vec<AmlTransactionData> = suspicious
                    .iter()
                    .map(|t| AmlTransactionData {
                        transaction_id: t.transaction_id.to_string(),
                        typology: t
                            .suspicion_reason
                            .as_ref()
                            .map(|r| r.canonical_name().to_string())
                            .unwrap_or_default(),
                        case_id: t.case_id.clone().unwrap_or_default(),
                        amount: t.amount.try_into().unwrap_or(0.0),
                        is_flagged: t.is_suspicious,
                    })
                    .collect();

                let mut typology_map: HashMap<String, (usize, HashMap<String, bool>)> =
                    HashMap::new();
                for txn in &aml_data {
                    if !txn.typology.is_empty() {
                        let entry = typology_map
                            .entry(txn.typology.clone())
                            .or_insert_with(|| (0, HashMap::new()));
                        entry.0 += 1;
                        entry.1.insert(txn.case_id.clone(), true);
                    }
                }
                let typology_data: Vec<TypologyData> = typology_map
                    .iter()
                    .map(|(name, (count, cases))| TypologyData {
                        name: name.clone(),
                        scenario_count: *count,
                        case_ids_consistent: cases.len() <= *count,
                    })
                    .collect();

                if let Ok(aml_res) =
                    AmlDetectabilityAnalyzer::new().analyze(&aml_data, &typology_data)
                {
                    banking_eval.aml = Some(aml_res);
                }
            }
            banking_eval.check_thresholds();

            match serde_json::to_string_pretty(&banking_eval) {
                Ok(json) => {
                    if let Err(e) =
                        std::fs::write(analytics_dir.join("banking_evaluation.json"), json)
                    {
                        warn!("Failed to write banking evaluation: {}", e);
                    } else {
                        info!(
                            "  Banking evaluation written ({} profiles, {} issues, passes={})",
                            result.banking.customers.len(),
                            banking_eval.issues.len(),
                            banking_eval.passes
                        );
                    }
                }
                Err(e) => warn!("Failed to serialize banking evaluation: {}", e),
            }
        }
    }

    // ========================================================================
    // Data Quality Issue Records + Quality Labels
    // ========================================================================
    if !result.quality_issues.is_empty() {
        let labels_dir = output_dir.join("labels");
        std::fs::create_dir_all(&labels_dir)?;
        info!("Writing data quality issue records...");
        write_json_safe(
            &result.quality_issues,
            &labels_dir.join("quality_issues.json"),
            "Data quality issues",
        );

        // Derive quality_labels.json from quality_issues: maps each QualityIssue
        // to a QualityIssueLabel with the corresponding LabeledIssueType and severity.
        use datasynth_generators::{
            LabeledIssueType, QualityIssueLabel, QualityIssueType, QualityLabels,
        };
        let mut quality_labels = QualityLabels::with_capacity(result.quality_issues.len());
        for issue in &result.quality_issues {
            let labeled_type = match issue.issue_type {
                QualityIssueType::MissingValue => LabeledIssueType::MissingValue,
                QualityIssueType::Typo => LabeledIssueType::Typo,
                QualityIssueType::DateFormatVariation
                | QualityIssueType::AmountFormatVariation
                | QualityIssueType::IdentifierFormatVariation
                | QualityIssueType::TextFormatVariation => LabeledIssueType::FormatVariation,
                QualityIssueType::ExactDuplicate
                | QualityIssueType::NearDuplicate
                | QualityIssueType::FuzzyDuplicate => LabeledIssueType::Duplicate,
                QualityIssueType::EncodingIssue => LabeledIssueType::EncodingIssue,
            };
            let mut label = QualityIssueLabel::new(
                labeled_type,
                issue.record_id.clone(),
                issue.field.clone().unwrap_or_else(|| "_record".to_string()),
                "data_quality_injector",
            );
            if let Some(ref orig) = issue.original_value {
                label = label.with_original(orig.clone());
            }
            if let Some(ref modified) = issue.modified_value {
                label = label.with_modified(modified.clone());
            }
            quality_labels.add(label);
        }
        if let Ok(json) = serde_json::to_string_pretty(&quality_labels) {
            if let Err(e) = std::fs::write(labels_dir.join("quality_labels.json"), json.as_bytes())
            {
                warn!("Failed to write quality labels: {}", e);
            } else {
                info!(
                    "  Quality labels written: {} labels -> labels/quality_labels.json",
                    quality_labels.len()
                );
            }
        }
    }

    // ========================================================================
    // Internal Controls
    // ========================================================================
    if !result.internal_controls.is_empty() || !result.sod_violations.is_empty() {
        let ctrl_dir = output_dir.join("internal_controls");
        std::fs::create_dir_all(&ctrl_dir)?;
        info!("Writing internal controls data...");

        write_json_safe(
            &result.internal_controls,
            &ctrl_dir.join("internal_controls.json"),
            "Internal controls",
        );
        // SoD violations extracted from control-annotated journal entries
        write_json_safe(
            &result.sod_violations,
            &ctrl_dir.join("sod_violations.json"),
            "SoD violations",
        );

        // SoD conflict pairs, SoD rules, control mappings, and COSO control mapping
        // are static reference data — export via ControlExporter regardless of whether
        // individual violations were generated so the master catalog is always present.
        let exporter = datasynth_output::ControlExporter::new(&ctrl_dir);
        match exporter.export_standard() {
            Ok(summary) => {
                info!(
                    "  Control master data written: {} controls, {} SoD conflicts, {} SoD rules, {} COSO mappings, {} account mappings",
                    summary.controls_count,
                    summary.sod_conflicts_count,
                    summary.sod_rules_count,
                    summary.coso_mappings_count,
                    summary.account_mappings_count,
                );
            }
            Err(e) => warn!("Failed to write control master data: {}", e),
        }
    }

    // ========================================================================
    // Accounting Standards
    // ========================================================================
    if !result.accounting_standards.contracts.is_empty()
        || !result.accounting_standards.impairment_tests.is_empty()
        || !result.accounting_standards.business_combinations.is_empty()
        || !result.accounting_standards.ecl_models.is_empty()
        || !result.accounting_standards.provisions.is_empty()
        || !result
            .accounting_standards
            .currency_translation_results
            .is_empty()
    {
        let acct_dir = output_dir.join("accounting_standards");
        std::fs::create_dir_all(&acct_dir)?;
        info!("Writing accounting standards data...");

        write_json_safe(
            &result.accounting_standards.contracts,
            &acct_dir.join("customer_contracts.json"),
            "Customer contracts",
        );
        write_json_safe(
            &result.accounting_standards.impairment_tests,
            &acct_dir.join("impairment_tests.json"),
            "Impairment tests",
        );
        write_json_safe(
            &result.accounting_standards.business_combinations,
            &acct_dir.join("business_combinations.json"),
            "Business combinations",
        );
        write_json_safe(
            &result
                .accounting_standards
                .business_combination_journal_entries,
            &acct_dir.join("business_combination_journal_entries.json"),
            "Business combination journal entries",
        );
        write_json_safe(
            &result.accounting_standards.ecl_models,
            &acct_dir.join("ecl_models.json"),
            "ECL models",
        );
        write_json_safe(
            &result.accounting_standards.ecl_provision_movements,
            &acct_dir.join("ecl_provision_movements.json"),
            "ECL provision movements",
        );
        write_json_safe(
            &result.accounting_standards.ecl_journal_entries,
            &acct_dir.join("ecl_journal_entries.json"),
            "ECL journal entries",
        );
        write_json_safe(
            &result.accounting_standards.provisions,
            &acct_dir.join("provisions.json"),
            "Provisions (IAS 37 / ASC 450)",
        );
        write_json_safe(
            &result.accounting_standards.provision_movements,
            &acct_dir.join("provision_movements.json"),
            "Provision movements",
        );
        write_json_safe(
            &result.accounting_standards.contingent_liabilities,
            &acct_dir.join("contingent_liabilities.json"),
            "Contingent liabilities",
        );
        write_json_safe(
            &result.accounting_standards.provision_journal_entries,
            &acct_dir.join("provision_journal_entries.json"),
            "Provision journal entries",
        );

        // IAS 21 — write under accounting_standards/fx/
        if !result
            .accounting_standards
            .currency_translation_results
            .is_empty()
        {
            let fx_dir = acct_dir.join("fx");
            std::fs::create_dir_all(&fx_dir)?;
            write_json_safe(
                &result.accounting_standards.currency_translation_results,
                &fx_dir.join("currency_translation_results.json"),
                "IAS 21 currency translation results",
            );
        }

        // v3.3.1: Leases (IFRS 16 / ASC 842)
        if !result.accounting_standards.leases.is_empty() {
            let leases_dir = acct_dir.join("leases");
            std::fs::create_dir_all(&leases_dir)?;
            write_json_safe(
                &result.accounting_standards.leases,
                &leases_dir.join("leases.json"),
                "Leases (IFRS 16 / ASC 842) — v3.3.1",
            );
        }

        // v3.3.1: Fair value measurements (IFRS 13 / ASC 820)
        if !result
            .accounting_standards
            .fair_value_measurements
            .is_empty()
        {
            let fv_dir = acct_dir.join("fair_value");
            std::fs::create_dir_all(&fv_dir)?;
            write_json_safe(
                &result.accounting_standards.fair_value_measurements,
                &fv_dir.join("fair_value_measurements.json"),
                "Fair value measurements (IFRS 13 / ASC 820) — v3.3.1",
            );
        }

        // v3.3.1: Framework reconciliation (dual reporting)
        if !result.accounting_standards.framework_differences.is_empty() {
            let diff_dir = acct_dir.join("framework_differences");
            std::fs::create_dir_all(&diff_dir)?;
            write_json_safe(
                &result.accounting_standards.framework_differences,
                &diff_dir.join("framework_differences.json"),
                "Framework differences (US GAAP vs IFRS) — v3.3.1",
            );
            write_json_safe(
                &result.accounting_standards.framework_reconciliations,
                &diff_dir.join("framework_reconciliations.json"),
                "Per-entity framework reconciliation — v3.3.1",
            );
        }
    }

    // ========================================================================
    // Quality Gate Results
    // ========================================================================
    if let Some(ref gate_result) = result.gate_result {
        match serde_json::to_string_pretty(gate_result) {
            Ok(json) => {
                if let Err(e) = std::fs::write(output_dir.join("quality_gate_result.json"), json) {
                    warn!("Failed to write quality gate result: {}", e);
                } else {
                    info!(
                        "  Quality gate result written (passed={})",
                        gate_result.passed
                    );
                }
            }
            Err(e) => warn!("Failed to serialize quality gate result: {}", e),
        }
    }

    // ========================================================================
    // Treasury
    // ========================================================================
    if !result.treasury.debt_instruments.is_empty()
        || !result.treasury.cash_positions.is_empty()
        || !result.treasury.hedging_instruments.is_empty()
    {
        let treasury_dir = output_dir.join("treasury");
        std::fs::create_dir_all(&treasury_dir)?;
        info!("Writing treasury data...");

        write_json_safe(
            &result.treasury.debt_instruments,
            &treasury_dir.join("debt_instruments.json"),
            "Debt instruments",
        );
        write_json_safe(
            &result.treasury.hedging_instruments,
            &treasury_dir.join("hedging_instruments.json"),
            "Hedging instruments",
        );
        write_json_safe(
            &result.treasury.hedge_relationships,
            &treasury_dir.join("hedge_relationships.json"),
            "Hedge relationships",
        );
        write_json_safe(
            &result.treasury.cash_positions,
            &treasury_dir.join("cash_positions.json"),
            "Cash positions",
        );
        write_json_safe(
            &result.treasury.cash_forecasts,
            &treasury_dir.join("cash_forecasts.json"),
            "Cash forecasts",
        );
        write_json_safe(
            &result.treasury.cash_pools,
            &treasury_dir.join("cash_pools.json"),
            "Cash pools",
        );
        write_json_safe(
            &result.treasury.cash_pool_sweeps,
            &treasury_dir.join("cash_pool_sweeps.json"),
            "Cash pool sweeps",
        );
        write_json_safe(
            &result.treasury.bank_guarantees,
            &treasury_dir.join("bank_guarantees.json"),
            "Bank guarantees",
        );
        write_json_safe(
            &result.treasury.netting_runs,
            &treasury_dir.join("netting_runs.json"),
            "Netting runs",
        );
        if !result.treasury.treasury_anomaly_labels.is_empty() {
            write_json_safe(
                &result.treasury.treasury_anomaly_labels,
                &treasury_dir.join("treasury_anomaly_labels.json"),
                "Treasury anomaly labels",
            );
        }
    }

    // ========================================================================
    // Project Accounting
    // ========================================================================
    if !result.project_accounting.projects.is_empty() {
        let pa_dir = output_dir.join("project_accounting");
        std::fs::create_dir_all(&pa_dir)?;
        info!("Writing project accounting data...");

        write_json_safe(
            &result.project_accounting.projects,
            &pa_dir.join("projects.json"),
            "Projects",
        );
        write_json_safe(
            &result.project_accounting.cost_lines,
            &pa_dir.join("cost_lines.json"),
            "Project cost lines",
        );
        write_json_safe(
            &result.project_accounting.revenue_records,
            &pa_dir.join("revenue_records.json"),
            "Project revenue records",
        );
        write_json_safe(
            &result.project_accounting.earned_value_metrics,
            &pa_dir.join("earned_value_metrics.json"),
            "Earned value metrics",
        );
        write_json_safe(
            &result.project_accounting.change_orders,
            &pa_dir.join("change_orders.json"),
            "Change orders",
        );
        write_json_safe(
            &result.project_accounting.milestones,
            &pa_dir.join("milestones.json"),
            "Project milestones",
        );
    }

    // ========================================================================
    // Evolution Events (Process Evolution + Organizational Events)
    // ========================================================================
    if !result.process_evolution.is_empty()
        || !result.organizational_events.is_empty()
        || !result.disruption_events.is_empty()
    {
        let events_dir = output_dir.join("events");
        std::fs::create_dir_all(&events_dir)?;
        info!("Writing evolution events...");

        write_json_safe(
            &result.process_evolution,
            &events_dir.join("process_evolution_events.json"),
            "Process evolution events",
        );
        write_json_safe(
            &result.organizational_events,
            &events_dir.join("organizational_events.json"),
            "Organizational events",
        );
        write_json_safe(
            &result.disruption_events,
            &events_dir.join("disruption_events.json"),
            "Disruption events",
        );
    }

    // ========================================================================
    // ML Training: Counterfactual Pairs
    // ========================================================================
    if !result.counterfactual_pairs.is_empty() {
        let ml_dir = output_dir.join("ml_training");
        std::fs::create_dir_all(&ml_dir)?;
        info!("Writing ML training data...");

        write_json_safe(
            &result.counterfactual_pairs,
            &ml_dir.join("counterfactual_pairs.json"),
            "Counterfactual pairs",
        );
    }

    // ========================================================================
    // Fraud Red-Flag Indicators
    // ========================================================================
    if !result.red_flags.is_empty() {
        let labels_dir = output_dir.join("labels");
        std::fs::create_dir_all(&labels_dir)?;
        info!("Writing fraud red-flag indicators...");

        write_json_safe(
            &result.red_flags,
            &labels_dir.join("fraud_red_flags.json"),
            "Fraud red flags",
        );
    }

    // ========================================================================
    // Collusion Rings
    // ========================================================================
    if !result.collusion_rings.is_empty() {
        let labels_dir = output_dir.join("labels");
        std::fs::create_dir_all(&labels_dir)?;
        info!("Writing collusion rings...");

        write_json_safe(
            &result.collusion_rings,
            &labels_dir.join("collusion_rings.json"),
            "Collusion rings",
        );
    }

    // ========================================================================
    // Temporal Vendor Version Chains
    // ========================================================================
    if !result.temporal_vendor_chains.is_empty() {
        let temporal_dir = output_dir.join("temporal");
        std::fs::create_dir_all(&temporal_dir)?;
        info!("Writing temporal vendor version chains...");

        write_json_safe(
            &result.temporal_vendor_chains,
            &temporal_dir.join("vendor_version_chains.json"),
            "Vendor version chains",
        );
    }

    // ========================================================================
    // Entity Relationship Graph + Cross-Process Links
    // ========================================================================
    if result.entity_relationship_graph.is_some() || !result.cross_process_links.is_empty() {
        let rel_dir = output_dir.join("relationships");
        std::fs::create_dir_all(&rel_dir)?;
        info!("Writing entity relationship data...");

        if let Some(ref graph) = result.entity_relationship_graph {
            match serde_json::to_string_pretty(graph) {
                Ok(json) => {
                    let path = rel_dir.join("entity_relationship_graph.json");
                    if let Err(e) = std::fs::write(&path, json) {
                        warn!("Failed to write entity relationship graph: {}", e);
                    } else {
                        info!(
                            "  Entity relationship graph written: {} nodes, {} edges -> {}",
                            graph.nodes.len(),
                            graph.edges.len(),
                            path.display()
                        );
                    }
                }
                Err(e) => warn!("Failed to serialize entity relationship graph: {}", e),
            }
        }

        write_json_safe(
            &result.cross_process_links,
            &rel_dir.join("cross_process_links.json"),
            "Cross-process links",
        );
    }

    // ========================================================================
    // Industry-Specific Data
    // ========================================================================
    if let Some(ref industry_output) = result.industry_output {
        if !industry_output.gl_accounts.is_empty() {
            let industry_dir = output_dir.join("industry");
            std::fs::create_dir_all(&industry_dir).ok();
            info!("Writing industry-specific data...");
            match serde_json::to_string_pretty(industry_output) {
                Ok(json) => {
                    if let Err(e) = std::fs::write(industry_dir.join("industry_data.json"), json) {
                        warn!("Failed to write industry data: {}", e);
                    } else {
                        info!(
                            "  Industry data written: {} GL accounts for {}",
                            industry_output.gl_accounts.len(),
                            industry_output.industry
                        );
                    }
                }
                Err(e) => warn!("Failed to serialize industry data: {}", e),
            }
        }
    }

    // ========================================================================
    // Graph Export Summary
    // ========================================================================
    if result.graph_export.exported {
        let graph_dir = output_dir.join("graph_export");
        std::fs::create_dir_all(&graph_dir).ok();
        match serde_json::to_string_pretty(&result.graph_export) {
            Ok(json) => {
                if let Err(e) = std::fs::write(graph_dir.join("graph_export_summary.json"), json) {
                    warn!("Failed to write graph export summary: {}", e);
                } else {
                    info!("  Graph export summary written");
                }
            }
            Err(e) => warn!("Failed to serialize graph export summary: {}", e),
        }
    }

    // ========================================================================
    // Compliance Regulations
    // ========================================================================
    let cr = &result.compliance_regulations;
    let has_compliance_data = !cr.standard_records.is_empty()
        || !cr.audit_procedures.is_empty()
        || !cr.findings.is_empty()
        || !cr.filings.is_empty();
    if has_compliance_data {
        let cr_dir = output_dir.join("compliance_regulations");
        std::fs::create_dir_all(&cr_dir)?;
        info!("Writing compliance regulations data...");

        write_json_safe(
            &cr.standard_records,
            &cr_dir.join("compliance_standards.json"),
            "Compliance standards",
        );
        write_json_safe(
            &cr.cross_reference_records,
            &cr_dir.join("cross_references.json"),
            "Cross-references",
        );
        write_json_safe(
            &cr.jurisdiction_records,
            &cr_dir.join("jurisdiction_profiles.json"),
            "Jurisdiction profiles",
        );
        write_json_safe(
            &cr.audit_procedures,
            &cr_dir.join("audit_procedures.json"),
            "Audit procedures",
        );
        write_json_safe(
            &cr.findings,
            &cr_dir.join("compliance_findings.json"),
            "Compliance findings",
        );
        write_json_safe(
            &cr.filings,
            &cr_dir.join("regulatory_filings.json"),
            "Regulatory filings",
        );

        if let Some(ref graph) = cr.compliance_graph {
            match serde_json::to_string_pretty(graph) {
                Ok(json) => {
                    if let Err(e) = std::fs::write(cr_dir.join("compliance_graph.json"), json) {
                        warn!("Failed to write compliance graph: {}", e);
                    } else {
                        info!(
                            "  Compliance graph written: {} nodes, {} edges",
                            graph.nodes.len(),
                            graph.edges.len()
                        );
                    }
                }
                Err(e) => warn!("Failed to serialize compliance graph: {}", e),
            }
        }
    }

    // ========================================================================
    // Generation Statistics
    // ========================================================================
    match serde_json::to_string_pretty(&result.statistics) {
        Ok(json) => {
            if let Err(e) = std::fs::write(output_dir.join("generation_statistics.json"), json) {
                warn!("Failed to write generation statistics: {}", e);
            } else {
                info!("  Generation statistics written");
            }
        }
        Err(e) => warn!("Failed to serialize generation statistics: {}", e),
    }

    info!("Output writing complete.");
    Ok(())
}

/// Write JSON with error handling - logs a warning on failure but does not abort.
///
/// When the `FLAT_LAYOUT_ACTIVE` thread-local is true (set by
/// `write_all_output_with_layout` when `export_layout: flat`), this routes
/// through `write_json_flat` so nested `{header, lines|items|allocations}`
/// shapes are automatically flattened. For structures without that shape,
/// `write_json_flat` passes through unchanged.
fn write_json_safe<T: serde::Serialize>(data: &[T], path: &Path, label: &str) {
    // Skip JSON entirely when not in requested output formats
    if SKIP_JSON.with(|c| c.get()) {
        return;
    }
    if FLAT_LAYOUT_ACTIVE.with(|c| c.get()) {
        write_json_flat(data, path, label);
    } else if let Err(e) = write_json(data, path, label) {
        warn!("Failed to write {}: {}", label, e);
    }
}

/// Write JSON, choosing flat or nested layout based on the flag.
fn write_json_auto<T: serde::Serialize>(data: &[T], path: &Path, label: &str, flat: bool) {
    if flat {
        write_json_flat(data, path, label);
    } else {
        write_json_safe(data, path, label);
    }
}

/// Write a JSON file ALWAYS, even when the slice is empty (writes `[]`).
///
/// Use for files that must exist in the archive for SDK consumers
/// (e.g., `audit_opinions.json`) regardless of whether the phase that
/// populates them ran. `write_json_safe` / `write_json` short-circuit
/// on empty slices, which would break manifest-driven clients that
/// expect the file to be present.
fn write_json_always<T: serde::Serialize>(data: &[T], path: &Path, label: &str) {
    if SKIP_JSON.with(|c| c.get()) {
        return;
    }
    match std::fs::File::create(path) {
        Ok(file) => {
            let mut writer = std::io::BufWriter::with_capacity(64 * 1024, file);
            if let Err(e) = (|| -> Result<(), Box<dyn std::error::Error>> {
                writer.write_all(b"[\n")?;
                for (i, item) in data.iter().enumerate() {
                    if i > 0 {
                        writer.write_all(b",\n")?;
                    }
                    serde_json::to_writer_pretty(&mut writer, item)?;
                }
                if !data.is_empty() {
                    writer.write_all(b"\n")?;
                }
                writer.write_all(b"]\n")?;
                writer.flush()?;
                Ok(())
            })() {
                warn!("Failed to write {}: {}", label, e);
            } else {
                info!(
                    "  {} written: {} records -> {}",
                    label,
                    data.len(),
                    path.display()
                );
            }
        }
        Err(e) => {
            warn!("Failed to create {}: {}", path.display(), e);
        }
    }
}

/// Write a flat JSON file by expanding a primary items array and merging the
/// surrounding context onto each line.
///
/// Flattens any record that contains a recognised items array
/// (`items`, `lines`, `line_items`, or `allocations`) into one row per line,
/// carrying over both the optional `header` sub-object and all other
/// top-level fields. Records without a recognised items array are emitted
/// as-is, except that an optional nested `header` sub-object is unwrapped
/// onto the top level so consumers see a uniformly flat shape.
///
/// Flow-style documents (`{header, items}`) and subledger-style documents
/// (`{..top-level scalars.., lines}`, e.g. AP/AR invoices, inventory
/// valuation runs) are both handled — fixing the SDK-team-reported gap
/// where subledger invoices were left with `lines` nested in flat mode.
///
/// Uses heap-allocated intermediates to avoid stack overflow with large
/// records in constrained environments (e.g., distroless containers with
/// glibc 2.36). Fixes #116.
fn write_json_flat<T: serde::Serialize>(data: &[T], path: &Path, label: &str) {
    if data.is_empty() {
        return;
    }

    // Pre-allocate on heap — avoid flat_map closure accumulating on the stack
    let mut flat: Vec<serde_json::Value> = Vec::with_capacity(data.len());

    for item in data {
        let val = match serde_json::to_value(item) {
            Ok(v) => v,
            Err(e) => {
                warn!("Failed to serialize record for flat export: {}", e);
                continue;
            }
        };

        let serde_json::Value::Object(map) = val else {
            flat.push(val);
            continue;
        };

        // Find the primary items array key (first match wins).
        let items_key = ["items", "lines", "allocations", "line_items"]
            .iter()
            .find(|k| map.contains_key(**k))
            .copied();

        // Optional nested header sub-object (used by document flows).
        let header_map = match map.get("header") {
            Some(serde_json::Value::Object(h)) => Some(h),
            _ => None,
        };

        let Some(items_key) = items_key else {
            // No items array. Emit one row, unwrapping the optional header
            // sub-object so consumers see a flat shape regardless of model
            // layout (e.g. Payments have `header` but no items/allocations
            // when allocations are empty).
            if let Some(header_map) = header_map {
                let mut merged = map.clone();
                merged.remove("header");
                for (k, v) in header_map {
                    merged.entry(k.clone()).or_insert_with(|| v.clone());
                }
                flat.push(serde_json::Value::Object(merged));
            } else {
                flat.push(serde_json::Value::Object(map));
            }
            continue;
        };

        let Some(serde_json::Value::Array(items)) = map.get(items_key) else {
            // `items_key` present but not an array — passthrough.
            flat.push(serde_json::Value::Object(map));
            continue;
        };

        // Empty items array: emit one row with the (unwrapped) header
        // context so downstream consumers can still find the parent
        // record — prevents silently dropping empty-lines invoices.
        if items.is_empty() {
            let mut merged = map.clone();
            merged.remove(items_key);
            if let Some(header_map) = header_map {
                merged.remove("header");
                for (k, v) in header_map {
                    merged.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
            flat.push(serde_json::Value::Object(merged));
            continue;
        }

        // Collect all other top-level fields (scalars, objects, arrays)
        // so they carry over onto every flattened line — matching pandas
        // `explode()` semantics. This is the behaviour SDK consumers
        // expect: header context is repeated per line, nested objects
        // like `net_amount: {amount, currency}` come along for the ride.
        let top_fields: Vec<(&String, &serde_json::Value)> = map
            .iter()
            .filter(|(k, _)| k.as_str() != "header" && k.as_str() != items_key)
            .collect();

        flat.reserve(items.len());
        for item_val in items {
            let mut merged = serde_json::Map::new();
            // Line/item fields first (take precedence on collisions).
            if let serde_json::Value::Object(m) = item_val {
                merged.extend(m.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
            // Header sub-object (when present) — don't overwrite line fields.
            if let Some(header_map) = header_map {
                for (k, v) in header_map {
                    merged.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
            // All other top-level fields.
            for &(k, v) in &top_fields {
                merged.entry(k.clone()).or_insert_with(|| v.clone());
            }
            flat.push(serde_json::Value::Object(merged));
        }
    }

    if flat.is_empty() {
        return;
    }

    // Stream-write each flattened record instead of serializing the whole Vec
    let count = flat.len();
    match std::fs::File::create(path) {
        Ok(file) => {
            use std::io::Write;
            let mut writer = std::io::BufWriter::with_capacity(512 * 1024, file);
            if let Err(e) = (|| -> Result<(), Box<dyn std::error::Error>> {
                writer.write_all(b"[\n")?;
                for (i, item) in flat.iter().enumerate() {
                    if i > 0 {
                        writer.write_all(b",\n")?;
                    }
                    serde_json::to_writer_pretty(&mut writer, item)?;
                }
                writer.write_all(b"\n]\n")?;
                writer.flush()?;
                Ok(())
            })() {
                warn!("Failed to write {}: {}", label, e);
            } else {
                info!(
                    "  {} written (flat): {} records -> {}",
                    label,
                    count,
                    path.display()
                );
            }
        }
        Err(e) => warn!("Failed to create {}: {}", label, e),
    }
}

/// Write a single serializable value as a JSON file.
fn write_json_single<T: serde::Serialize>(
    data: &T,
    path: &Path,
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let writer = std::io::BufWriter::with_capacity(256 * 1024, file);
    serde_json::to_writer_pretty(writer, data)?;
    info!("  {} written -> {}", label, path.display());
    Ok(())
}

/// Write a single serializable value as a JSON file, logging a warning on failure.
fn write_json_single_safe<T: serde::Serialize>(data: &T, path: &Path, label: &str) {
    if SKIP_JSON.with(|c| c.get()) {
        return;
    }
    if let Err(e) = write_json_single(data, path, label) {
        warn!("Failed to write {}: {}", label, e);
    }
}

/// Serializable summary of balance validation (avoids serializing the full
/// `BalanceValidationResult` which has non-Serialize validation error types).
#[derive(serde::Serialize)]
struct BalanceValidationSummary {
    validated: bool,
    is_balanced: bool,
    entries_processed: u64,
    total_debits: String,
    total_credits: String,
    accounts_tracked: usize,
    companies_tracked: usize,
    has_unbalanced_entries: bool,
    validation_error_count: usize,
}

impl BalanceValidationSummary {
    fn from(v: &crate::enhanced_orchestrator::BalanceValidationResult) -> Self {
        Self {
            validated: v.validated,
            is_balanced: v.is_balanced,
            entries_processed: v.entries_processed,
            total_debits: v.total_debits.to_string(),
            total_credits: v.total_credits.to_string(),
            accounts_tracked: v.accounts_tracked,
            companies_tracked: v.companies_tracked,
            has_unbalanced_entries: v.has_unbalanced_entries,
            validation_error_count: v.validation_errors.len(),
        }
    }
}

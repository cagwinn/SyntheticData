//! v5.8.0 — integration test for the `graphs/je_network.csv` flat
//! edge-list export and the line-level `predecessor_line_id` chain.
//!
//! Single test that runs the orchestrator once (≈90s, single-threaded)
//! and makes all assertions against the same output. Consolidated to
//! avoid OOM from parallel orchestrator runs in `cargo test`.
//!
//! Asserts:
//!   1. `graphs/je_network.csv` is produced with the expected schema.
//!   2. Edge count equals Σ (n_debit_lines × n_credit_lines) per JE.
//!   3. Each edge row joins back to `journal_entries.csv` rows via
//!      `from_line_id` / `to_line_id` → `transaction_id`.
//!   4. `predecessor_edge_id` is populated for at least some edges
//!      (document-flow chains produce predecessor pointers).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;

use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_test_utils::fixtures::minimal_config;
use tempfile::TempDir;

fn parse_csv_header(path: &std::path::Path) -> Vec<String> {
    let content = fs::read_to_string(path).unwrap();
    content
        .lines()
        .next()
        .unwrap()
        .split(',')
        .map(|s| s.to_string())
        .collect()
}

fn parse_csv_rows(path: &std::path::Path) -> Vec<HashMap<String, String>> {
    let content = fs::read_to_string(path).unwrap();
    let mut lines = content.lines();
    let header: Vec<String> = lines
        .next()
        .unwrap()
        .split(',')
        .map(|s| s.to_string())
        .collect();
    lines
        .map(|line| {
            // Naive split — the writer only quotes strings that contain
            // delimiters, and our schema fields are clean (UUIDs, dates,
            // numbers, snake_case enums). Sufficient for assertions.
            let fields: Vec<&str> = line.split(',').collect();
            header
                .iter()
                .cloned()
                .zip(fields.iter().map(|s| s.to_string()))
                .collect()
        })
        .collect()
}

#[test]
fn je_network_export_end_to_end() {
    let mut config = minimal_config();
    config.global.seed = Some(58000);
    config.global.period_months = 1;
    config.fraud.enabled = false;

    let mut phase_config = PhaseConfig::from_config(&config);
    phase_config.show_progress = false;
    phase_config.generate_journal_entries = true;
    phase_config.generate_document_flows = true; // need chain to test predecessor wiring
    phase_config.inject_anomalies = false;
    phase_config.inject_data_quality = false;
    phase_config.generate_intercompany = false;
    phase_config.generate_banking = false;
    phase_config.generate_graph_export = false;
    phase_config.generate_ocpm_events = false;
    phase_config.generate_audit = false;
    phase_config.generate_evolution_events = false;
    phase_config.generate_sourcing = false;
    phase_config.generate_period_close = false;
    phase_config.generate_accounting_standards = false;
    phase_config.generate_manufacturing = false;
    phase_config.generate_sales_kpi_budgets = false;
    phase_config.generate_tax = false;
    phase_config.generate_esg = false;
    phase_config.generate_hr = false;
    phase_config.generate_treasury = false;
    phase_config.generate_project_accounting = false;
    phase_config.generate_compliance_regulations = false;
    phase_config.generate_financial_statements = false;
    phase_config.generate_bank_reconciliation = false;
    phase_config.validate_balances = false;

    let mut orch = EnhancedOrchestrator::new(config, phase_config).expect("build orchestrator");
    let result = orch.generate().expect("generate");

    let tmp = TempDir::new().unwrap();
    let out = tmp.path().to_path_buf();
    fs::create_dir_all(&out).unwrap();

    // This test validates the Cartesian edge formula (Σ n_debit × n_credit
    // per JE), so it must request `JeNetworkMethod::Cartesian` explicitly —
    // the default flipped to Method A in v5.27 (one edge per 2-line JE).
    datasynth_runtime::output_writer::write_all_output_with_layout(
        &result,
        &out,
        datasynth_config::ExportLayout::Nested,
        &[
            datasynth_config::FileFormat::Csv,
            datasynth_config::FileFormat::Json,
        ],
        datasynth_config::JeNetworkMethod::Cartesian,
    )
    .expect("output writer");

    // ── 1. Schema ────────────────────────────────────────────────────
    let net_path = out.join("graphs/je_network.csv");
    assert!(net_path.exists(), "graphs/je_network.csv must exist");
    let header = parse_csv_header(&net_path);
    let expected_cols = [
        "edge_id",
        "document_id",
        "posting_date",
        "from_account",
        "to_account",
        "from_line_id",
        "to_line_id",
        "amount",
        "confidence",
        "predecessor_edge_id",
        "business_process",
        "is_fraud",
        "is_anomaly",
    ];
    let header_set: BTreeSet<&str> = header.iter().map(String::as_str).collect();
    let expected_set: BTreeSet<&str> = expected_cols.iter().copied().collect();
    assert_eq!(
        header_set, expected_set,
        "schema mismatch: got {:?}, expected {:?}",
        header, expected_cols
    );

    // ── 2. Edge count = Σ (n_debit × n_credit) per JE ────────────────
    let je_csv = out.join("journal_entries.csv");
    assert!(je_csv.exists(), "journal_entries.csv must exist");
    let je_rows = parse_csv_rows(&je_csv);
    let mut per_doc: HashMap<String, (usize, usize)> = HashMap::new();
    for r in &je_rows {
        let doc = r.get("document_id").cloned().unwrap_or_default();
        let debit: f64 = r
            .get("debit_amount")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let credit: f64 = r
            .get("credit_amount")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let entry = per_doc.entry(doc).or_insert((0, 0));
        if debit > 0.0 {
            entry.0 += 1;
        }
        if credit > 0.0 {
            entry.1 += 1;
        }
    }
    let expected_edges: usize = per_doc.values().map(|(d, c)| d * c).sum();
    let edges = parse_csv_rows(&net_path);
    assert_eq!(
        edges.len(),
        expected_edges,
        "edge count {} should equal Σ n_debit × n_credit per JE = {}",
        edges.len(),
        expected_edges
    );

    // ── 3. Edges join back to JE table by transaction_id ─────────────
    let line_ids: HashSet<String> = je_rows
        .iter()
        .filter_map(|r| r.get("transaction_id").cloned())
        .filter(|s| !s.is_empty())
        .collect();
    let sample = edges.iter().take(500); // bound for fast assertion
    let mut sampled = 0usize;
    for e in sample {
        let from = e.get("from_line_id").cloned().unwrap_or_default();
        let to = e.get("to_line_id").cloned().unwrap_or_default();
        assert!(
            line_ids.contains(&from),
            "from_line_id {from} not in JE table"
        );
        assert!(line_ids.contains(&to), "to_line_id {to} not in JE table");
        sampled += 1;
    }
    assert!(sampled > 0, "should have sampled at least one edge");

    // ── 4. predecessor_edge_id populated where document chains exist ─
    let with_predecessor = edges
        .iter()
        .filter(|e| {
            e.get("predecessor_edge_id")
                .map(|s| !s.is_empty())
                .unwrap_or(false)
        })
        .count();
    assert!(
        with_predecessor > 0,
        "expected ≥1 edge with predecessor_edge_id when document flows are on"
    );
    assert!(
        with_predecessor < edges.len(),
        "not every edge should have a predecessor — root JEs have none"
    );
}

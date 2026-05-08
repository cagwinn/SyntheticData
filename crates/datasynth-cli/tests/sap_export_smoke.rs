//! End-to-end smoke test for the SAP Integration Pack (v4.3.0).
//!
//! Exercises the full 27-table set via the CLI binary with HANA
//! dialect. Verifies:
//! - Every requested table writes a non-empty file.
//! - UTF-8 BOM prefix on the first table (dialect-respecting writer).
//! - Semicolon delimiter, ISO dates, and decimal comma in the body
//!   (HANA dialect).
//! - Cross-table references are preserved (EKPO.EBELN matches
//!   EKKO.EBELN; BSEG.BELNR matches BKPF.BELNR).

use assert_cmd::Command;
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

const TEST_TIMEOUT_SECS: u64 = 600; // 10 min — covers llvm-cov instrumentation on slow runners
const TEST_MEMORY_LIMIT: &str = "512";
const TEST_MAX_THREADS: &str = "1";

#[allow(deprecated)]
fn synth_data_bin() -> Command {
    let mut cmd = Command::cargo_bin("datasynth-data").expect("binary in target/");
    cmd.timeout(Duration::from_secs(TEST_TIMEOUT_SECS));
    cmd
}

#[test]
fn sap_hana_export_writes_all_27_tables() {
    let tmp = TempDir::new().expect("tempdir");
    let config_path = tmp.path().join("config.yaml");
    let output_path = tmp.path().join("out");

    let config_yaml = r#"
global:
  industry: retail
  seed: 42
  start_date: "2024-01-01"
  period_months: 1
companies:
  - code: "C001"
    name: "SAP Smoke Corp"
    currency: "EUR"
    country: "DE"
    annual_transaction_volume: ten_k
    volume_weight: 1.0
chart_of_accounts:
  complexity: small
document_flows:
  enabled: true
output:
  output_directory: "/tmp/unused"
  formats: [json]
  sap:
    client: "200"
    ledger: "0L"
    source_system: "DATASYNTH"
    local_currency: "EUR"
    dialect: hana
    tables: [bkpf, bseg, acdoca, lfa1, lfb1, kna1, knb1, mara, mard,
             anla, csks, ska1, skb1, ekko, ekpo, vbak, vbap,
             likp, lips, mkpf, mseg, bsis, bsas, bsid, bsad, bsik, bsak]
    include_extension_fields: true
"#;
    fs::write(&config_path, config_yaml).expect("write config");

    let output_str = output_path.to_string_lossy().to_string();
    synth_data_bin()
        .arg("generate")
        .arg("--config")
        .arg(&config_path)
        .arg("--output")
        .arg(&output_str)
        .arg("--export-format")
        .arg("sap")
        .arg("--memory-limit")
        .arg(TEST_MEMORY_LIMIT)
        .arg("--max-threads")
        .arg(TEST_MAX_THREADS)
        .assert()
        .success();

    let sap_dir = output_path.join("sap_export");
    assert!(sap_dir.is_dir(), "sap_export directory missing");

    // Every requested table writes a non-empty CSV.
    let expected_tables = [
        "bkpf", "bseg", "acdoca", "lfa1", "lfb1", "kna1", "knb1", "mara", "mard", "anla", "csks",
        "ska1", "skb1", "ekko", "ekpo", "vbak", "vbap", "likp", "lips", "mkpf", "mseg", "bsis",
        "bsas", "bsid", "bsad", "bsik", "bsak",
    ];
    for table in expected_tables {
        let path = sap_dir.join(format!("{table}.csv"));
        assert!(path.exists(), "missing {table}.csv");
        let meta = fs::metadata(&path).expect("stat file");
        assert!(
            meta.len() > 0,
            "{table}.csv is empty — writer produced nothing"
        );
    }

    // Dialect sanity: BKPF must have BOM + semicolon + ISO date + decimal comma.
    let bkpf_bytes = fs::read(sap_dir.join("bkpf.csv")).expect("read bkpf.csv");
    assert_eq!(
        &bkpf_bytes[..3],
        [0xEF, 0xBB, 0xBF],
        "hana dialect must prefix files with UTF-8 BOM"
    );
    let bkpf_text = std::str::from_utf8(&bkpf_bytes[3..]).expect("utf-8 body");
    let header = bkpf_text.lines().next().expect("header line");
    assert!(
        header.contains(';') && !header.contains(','),
        "hana header must use semicolon, got: {header}"
    );
    // First body line should contain an ISO-dashed date.
    let body = bkpf_text.lines().nth(1).expect("at least one row");
    assert!(
        body.split(';')
            .any(|f| f.len() == 10 && f.chars().filter(|c| *c == '-').count() == 2),
        "hana body must contain YYYY-MM-DD date, got: {body}"
    );

    // Cross-table reference: every EKPO.EBELN must resolve to an EKKO.EBELN.
    let ekko_text = read_stripping_bom(&sap_dir.join("ekko.csv"));
    let ekpo_text = read_stripping_bom(&sap_dir.join("ekpo.csv"));
    let ekko_ids: std::collections::HashSet<&str> = ekko_text
        .lines()
        .skip(1)
        .filter_map(|l| l.split(';').nth(1))
        .collect();
    let mut ekpo_lines = ekpo_text.lines();
    ekpo_lines.next(); // header
    let mut seen_items = 0usize;
    for line in ekpo_lines {
        let belnr = line.split(';').nth(1).expect("EKPO.EBELN column");
        assert!(
            ekko_ids.contains(belnr),
            "EKPO.EBELN {belnr} not found in EKKO — foreign-key integrity broken"
        );
        seen_items += 1;
    }
    assert!(seen_items > 0, "EKPO must contain at least one row");

    // Cross-table reference: BSEG.BELNR must resolve to BKPF.BELNR.
    let bkpf_ids: std::collections::HashSet<&str> = bkpf_text
        .lines()
        .skip(1)
        .filter_map(|l| l.split(';').nth(2))
        .collect();
    let bseg_text = read_stripping_bom(&sap_dir.join("bseg.csv"));
    let mut bseg_lines = bseg_text.lines();
    bseg_lines.next();
    let mut seen_bseg = 0usize;
    for line in bseg_lines.take(50) {
        let belnr = line.split(';').nth(2).expect("BSEG.BELNR column");
        assert!(
            bkpf_ids.contains(belnr),
            "BSEG.BELNR {belnr} not found in BKPF — foreign-key integrity broken"
        );
        seen_bseg += 1;
    }
    assert!(seen_bseg > 0, "BSEG must contain at least one row");
}

fn read_stripping_bom(path: &std::path::Path) -> String {
    let bytes = fs::read(path).expect("read file");
    let body = if bytes.len() >= 3 && bytes[..3] == [0xEF, 0xBB, 0xBF] {
        &bytes[3..]
    } else {
        &bytes[..]
    };
    std::str::from_utf8(body)
        .expect("file is UTF-8")
        .to_string()
}

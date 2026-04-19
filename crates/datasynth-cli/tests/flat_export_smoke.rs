//! End-to-end smoke test for `export_layout: flat` via the CLI binary.
//!
//! SDK teams report that `exportLayout: "flat"` hangs generation. The
//! v3.1.1 camelCase-alias fix addressed silent config rejection, but we
//! need a real end-to-end test that actually drives the full generate
//! command with flat layout and verifies the archive contents. A hang
//! here will exceed the test timeout and fail loudly with a clear
//! message instead of stalling silently.

use assert_cmd::Command;
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

/// Hard cap on elapsed time: if flat ever really hangs, the timeout
/// cuts the process and the test fails with a useful error.
const TEST_TIMEOUT_SECS: u64 = 300;
const TEST_MEMORY_LIMIT: &str = "512";
const TEST_MAX_THREADS: &str = "1";

#[allow(deprecated)]
fn synth_data_bin() -> Command {
    let mut cmd = Command::cargo_bin("datasynth-data").unwrap();
    cmd.timeout(Duration::from_secs(TEST_TIMEOUT_SECS));
    cmd
}

#[test]
fn flat_layout_generate_from_config_writes_archive_without_hanging() {
    let tmp = TempDir::new().expect("tempdir");
    let config_path = tmp.path().join("config.yaml");
    let output_path = tmp.path().join("out");

    // Minimal generator config that exercises the flat-layout JSON path.
    // Document flows enabled so the writer has nested {header, lines}
    // structures to flatten.
    let config_yaml = r#"
global:
  industry: retail
  seed: 123
  start_date: "2024-01-01"
  period_months: 1
companies:
  - code: "C001"
    name: "FlatTest Retail Corp"
    currency: "USD"
    country: "US"
    annual_transaction_volume: ten_k
    volume_weight: 1.0
chart_of_accounts:
  complexity: small
document_flows:
  enabled: true
output:
  output_directory: "/tmp/unused"
  formats: [json]
  export_layout: flat
"#;
    fs::write(&config_path, config_yaml).expect("write config");

    let output_str = output_path.to_string_lossy().to_string();
    synth_data_bin()
        .arg("generate")
        .arg("--config")
        .arg(&config_path)
        .arg("--output")
        .arg(&output_str)
        .arg("--memory-limit")
        .arg(TEST_MEMORY_LIMIT)
        .arg("--max-threads")
        .arg(TEST_MAX_THREADS)
        .assert()
        .success();

    // Headline file must exist and be non-empty.
    let je_path = output_path.join("journal_entries.json");
    assert!(
        je_path.exists(),
        "journal_entries.json missing after flat export at {:?}",
        je_path
    );
    let bytes = fs::read(&je_path).expect("read je file");
    assert!(
        bytes.len() > 2,
        "flat journal_entries.json is too small ({} bytes) — writer produced nothing",
        bytes.len()
    );

    // Verify the JSON is a valid array of flat records (header fields merged
    // onto line fields — no nested "header" key, no nested "lines" key).
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).expect("flat JE file is not valid JSON");
    let arr = json.as_array().expect("flat JE file is not a JSON array");
    assert!(
        !arr.is_empty(),
        "flat JE array is empty — no records written"
    );
    let first = &arr[0];
    // Flat format: both header and line fields on the same object.
    assert!(
        first.get("document_id").is_some(),
        "flat record missing header field `document_id` — flatten failed"
    );
    assert!(
        first.get("gl_account").is_some(),
        "flat record missing line field `gl_account` — flatten failed"
    );
    // No nested "header" / "lines" keys — that would mean nested layout
    // leaked through.
    assert!(
        first.get("lines").is_none(),
        "flat record has a nested `lines` array — flatten did not unwrap"
    );
}

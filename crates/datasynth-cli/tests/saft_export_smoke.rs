//! End-to-end smoke test for the SAF-T export (v4.3.1).
//!
//! Runs the CLI with `--export-format saft` for each supported
//! jurisdiction and asserts the resulting XML is well-formed and
//! contains the expected top-level structure.

use assert_cmd::Command;
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

const TEST_TIMEOUT_SECS: u64 = 300;
const TEST_MEMORY_LIMIT: &str = "512";
const TEST_MAX_THREADS: &str = "1";

#[allow(deprecated)]
fn synth_data_bin() -> Command {
    let mut cmd = Command::cargo_bin("datasynth-data").expect("binary in target/");
    cmd.timeout(Duration::from_secs(TEST_TIMEOUT_SECS));
    cmd
}

fn run_saft_with_jurisdiction(jurisdiction: &str, expected_filename: &str, expected_version: &str) {
    let tmp = TempDir::new().expect("tempdir");
    let config_path = tmp.path().join("config.yaml");
    let output_path = tmp.path().join("out");

    // Banking is explicitly disabled — this is a SAF-T export smoke
    // test, not a banking test, and the default banking flow generates
    // ~800 k banking_transactions which blows out the 300 s test
    // budget on Windows runners (saft_pl deterministically times out
    // at ~510 s without this — same pattern fixed for
    // flat_export_smoke + camelcase_sdk_config in earlier PRs).
    let config_yaml = format!(
        r#"
global:
  industry: retail
  seed: 42
  start_date: "2024-01-01"
  period_months: 1
companies:
  - code: "C001"
    name: "SAF-T Smoke Corp"
    currency: "EUR"
    country: "DE"
    annual_transaction_volume: ten_k
    volume_weight: 1.0
chart_of_accounts:
  complexity: small
banking:
  enabled: false
output:
  output_directory: "/tmp/unused"
  formats: [json]
  saft:
    jurisdiction: {jurisdiction}
    company_tax_id: "DE123456789"
    company_name: "SAF-T Smoke Corp"
"#
    );
    fs::write(&config_path, &config_yaml).expect("write config");

    let output_str = output_path.to_string_lossy().to_string();
    synth_data_bin()
        .arg("generate")
        .arg("--config")
        .arg(&config_path)
        .arg("--output")
        .arg(&output_str)
        .arg("--export-format")
        .arg("saft")
        .arg("--memory-limit")
        .arg(TEST_MEMORY_LIMIT)
        .arg("--max-threads")
        .arg(TEST_MAX_THREADS)
        .assert()
        .success();

    let path = output_path.join(expected_filename);
    assert!(
        path.exists(),
        "expected SAF-T output {} missing",
        path.display()
    );
    let text = fs::read_to_string(&path).expect("read SAF-T");
    assert!(
        text.starts_with(r#"<?xml version="1.0" encoding="UTF-8"?>"#),
        "SAF-T file must start with an XML declaration, got: {}",
        &text[..80.min(text.len())]
    );
    assert!(
        text.contains("<AuditFile"),
        "SAF-T file must have an <AuditFile> root"
    );
    assert!(
        text.contains("<Header>") && text.contains("</Header>"),
        "SAF-T file must have a Header section"
    );
    assert!(
        text.contains("<MasterFiles>") && text.contains("</MasterFiles>"),
        "SAF-T file must have a MasterFiles section"
    );
    assert!(
        text.contains("<GeneralLedgerEntries>") && text.contains("</GeneralLedgerEntries>"),
        "SAF-T file must have a GeneralLedgerEntries section"
    );
    assert!(
        text.contains(expected_version),
        "SAF-T file must declare AuditFileVersion {} (jurisdiction {}), got: {}",
        expected_version,
        jurisdiction,
        text.lines()
            .find(|l| l.contains("AuditFileVersion"))
            .unwrap_or("(version line not found)")
    );
    assert!(
        text.contains("<CompanyName>SAF-T Smoke Corp</CompanyName>"),
        "Company name should carry through"
    );
    assert!(text.contains("</AuditFile>"), "closing root element");
}

#[test]
fn saft_pt_export_writes_valid_xml() {
    run_saft_with_jurisdiction("pt", "saft_pt.xml", "1.04_01");
}

#[test]
fn saft_pl_export_writes_valid_xml() {
    run_saft_with_jurisdiction("pl", "jpk_kr.xml", "1.0");
}

#[test]
fn saft_no_export_writes_valid_xml() {
    run_saft_with_jurisdiction("no", "saft_no.xml", "1.10");
}

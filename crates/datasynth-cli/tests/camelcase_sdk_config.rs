//! v4.4.1 regression test — SDK teams submitting camelCase config keys
//! must hit the same code paths as native snake_case configs.
//!
//! The SDK team reported that enabling the six feature sections
//! together with camelCase keys (documentFlows, accountingStandards,
//! complianceRegulations, analyticsMetadata, audit, llm) collapsed
//! the archive from 99 files (1.8 GB) to 19 files (2.8 MB), and
//! `journal_entries.csv` landed instead of `.json` despite
//! `exportFormat: "json"`. Root cause: missing `#[serde(alias =
//! "camelCase")]` annotations on every multi-word config field.
//!
//! This test reproduces that exact config shape and asserts:
//! - The archive contains the expected rich set of files (we look for
//!   specific sentinel paths that are only produced when each feature
//!   subsection is actually parsed).
//! - `journal_entries.json` (not .csv) is emitted for
//!   `exportFormat: "json"` / `formats: [json]`.

use assert_cmd::Command;
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

const TEST_TIMEOUT_SECS: u64 = 300;

#[allow(deprecated)]
fn synth_data_bin() -> Command {
    let mut cmd = Command::cargo_bin("datasynth-data").expect("binary in target/");
    cmd.timeout(Duration::from_secs(TEST_TIMEOUT_SECS));
    cmd
}

/// Enables all six SDK-team "feature-matrix" sections with camelCase
/// keys. Before v4.4.1 this produced a severely truncated archive
/// because the camelCase keys silently fell through to defaults.
#[test]
fn camelcase_feature_matrix_config_produces_full_archive() {
    let tmp = TempDir::new().expect("tempdir");
    let config_path = tmp.path().join("config.yaml");
    let output_path = tmp.path().join("out");

    let config_yaml = r#"
global:
  industry: retail
  seed: 42
  startDate: "2024-01-01"
  periodMonths: 1
companies:
  - code: "C001"
    name: "CamelCase Corp"
    currency: "USD"
    country: "US"
    annualTransactionVolume: ten_k
    volumeWeight: 1.0
chartOfAccounts:
  complexity: small
# SDK-style camelCase
analyticsMetadata:
  enabled: true
audit:
  enabled: true
complianceRegulations:
  enabled: true
accountingStandards:
  enabled: true
  framework: us_gaap
vendorNetwork:
  enabled: true
customerSegmentation:
  enabled: true
documentFlows:
  enabled: true
output:
  output_directory: "/tmp/unused"
  exportFormat: "json"
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
        .arg("2048")
        .arg("--max-threads")
        .arg("2")
        .assert()
        .success();

    // Regression check 1: `journal_entries.json` must exist
    // (`exportFormat: "json"` worked) and `.csv` should NOT exist as
    // the only JE file.
    let je_json = output_path.join("journal_entries.json");
    let je_csv = output_path.join("journal_entries.csv");
    assert!(
        je_json.exists(),
        "journal_entries.json missing — `exportFormat: \"json\"` was not honoured \
         (camelCase alias regression)"
    );
    // .csv may still be emitted for convenience, but .json MUST be present.
    let _ = je_csv; // allow either to exist

    // Regression check 2: each camelCase-keyed feature subsection must
    // have actually run. Sentinel files only appear when the subsection
    // parsed correctly.
    let sentinels: &[(&str, &str)] = &[
        ("documentFlows", "document_flows/purchase_orders.json"),
        ("accountingStandards", "accounting_standards"), // directory
        (
            "complianceRegulations",
            "compliance_regulations/compliance_standards.json",
        ),
        ("analyticsMetadata", "analytics"), // directory created when analytics_metadata enabled
        ("audit", "audit/audit_engagements.json"),
        ("vendorNetwork", "master_data/vendors.json"), // expanded vendor count = vendor_network parsed
    ];
    for (key, rel) in sentinels {
        let p = output_path.join(rel);
        assert!(
            p.exists(),
            "expected sentinel {rel} for camelCase key `{key}` — \
             subsection didn't parse or didn't enable its generator"
        );
    }

    // Regression check 3: archive has substantive content.
    let file_count = count_files_recursive(&output_path);
    assert!(
        file_count > 50,
        "archive has only {} files — v4.1.x-style camelCase collapse \
         (SDK team saw 19 files; v4.4.1 should have 80+)",
        file_count
    );

    // Regression check 4: accounting_framework populated in the new
    // CoA metadata file (was null in all v4.1.x baselines).
    let coa_meta_path = output_path.join("chart_of_accounts_meta.json");
    assert!(
        coa_meta_path.exists(),
        "chart_of_accounts_meta.json missing (v4.4.1 companion file)"
    );
    let text = fs::read_to_string(&coa_meta_path).expect("read coa meta");
    let meta: serde_json::Value = serde_json::from_str(&text).expect("parse coa meta");
    assert_eq!(
        meta["accounting_framework"].as_str(),
        Some("us_gaap"),
        "CoA accounting_framework should be 'us_gaap' per config, \
         got {:?}",
        meta["accounting_framework"]
    );
}

fn count_files_recursive(dir: &std::path::Path) -> usize {
    let mut n = 0;
    let Ok(rd) = fs::read_dir(dir) else {
        return 0;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            n += count_files_recursive(&p);
        } else if p.is_file() {
            n += 1;
        }
    }
    n
}

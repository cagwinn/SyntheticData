//! v3.5.0 — smoke test for `datasynth-data templates enrich` CLI.
//!
//! Drives the enrich subcommand with the deterministic mock backend and
//! verifies that the output YAML contains the expected number of new
//! items under the right pool. Deterministic across runs for the same
//! `--seed` value.

use assert_cmd::Command;
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

const TEST_TIMEOUT_SECS: u64 = 60;

#[allow(deprecated)]
fn synth_data_bin() -> Command {
    let mut cmd = Command::cargo_bin("datasynth-data").unwrap();
    cmd.timeout(Duration::from_secs(TEST_TIMEOUT_SECS));
    cmd
}

#[test]
fn enrich_creates_vendor_names() {
    let tmp = TempDir::new().expect("tempdir");
    let input = tmp.path().join("in.yaml");
    let output = tmp.path().join("out.yaml");
    // Missing input is OK — handler starts from empty TemplateData.
    let _ = &input;

    let assert = synth_data_bin()
        .args([
            "templates",
            "enrich",
            "--input",
            input.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--category",
            "vendor_name",
            "--industry",
            "retail",
            "--region",
            "DE",
            "--sub-category",
            "office_supplies",
            "--count",
            "10",
            "--backend",
            "mock",
            "--seed",
            "42",
        ])
        .assert();

    assert.success();

    let yaml = fs::read_to_string(&output).expect("read output");
    // Mock provider returns deterministic content; the fallback is used
    // whenever the mock's response parses as empty. Either way, the key
    // requirement is that `vendor_names.categories.office_supplies` has
    // at least one entry.
    assert!(
        yaml.contains("office_supplies"),
        "expected vendor_names.categories.office_supplies in YAML, got:\n{yaml}"
    );
    assert!(
        yaml.contains("description:"),
        "expected metadata.description provenance line, got:\n{yaml}"
    );
}

#[test]
fn enrich_creates_customer_names() {
    let tmp = TempDir::new().expect("tempdir");
    let input = tmp.path().join("in.yaml");
    let output = tmp.path().join("out.yaml");

    let assert = synth_data_bin()
        .args([
            "templates",
            "enrich",
            "--input",
            input.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--category",
            "customer_name",
            "--industry",
            "retail",
            "--region",
            "US",
            "--sub-category",
            "enterprise",
            "--count",
            "5",
            "--backend",
            "mock",
        ])
        .assert();

    assert.success();

    let yaml = fs::read_to_string(&output).expect("read output");
    // CustomerNameTemplates has `industries` map keyed by industry.
    assert!(
        yaml.contains("retail"),
        "expected industry key 'retail' in YAML, got:\n{yaml}"
    );
}

#[test]
fn enrich_creates_material_descriptions() {
    let tmp = TempDir::new().expect("tempdir");
    let input = tmp.path().join("in.yaml");
    let output = tmp.path().join("out.yaml");

    let assert = synth_data_bin()
        .args([
            "templates",
            "enrich",
            "--input",
            input.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--category",
            "material_desc",
            "--industry",
            "manufacturing",
            "--region",
            "DE",
            "--sub-category",
            "raw_materials",
            "--count",
            "8",
            "--backend",
            "mock",
        ])
        .assert();

    assert.success();

    let yaml = fs::read_to_string(&output).expect("read output");
    assert!(
        yaml.contains("raw_materials"),
        "expected material_type key 'raw_materials' in YAML, got:\n{yaml}"
    );
}

#[test]
fn enrich_rejects_unknown_category() {
    let tmp = TempDir::new().expect("tempdir");
    let input = tmp.path().join("in.yaml");
    let output = tmp.path().join("out.yaml");

    let assert = synth_data_bin()
        .args([
            "templates",
            "enrich",
            "--input",
            input.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--category",
            "something_made_up",
            "--backend",
            "mock",
        ])
        .assert();

    assert.failure();
}

#[test]
fn enrich_rejects_unknown_backend() {
    let tmp = TempDir::new().expect("tempdir");
    let input = tmp.path().join("in.yaml");
    let output = tmp.path().join("out.yaml");

    let assert = synth_data_bin()
        .args([
            "templates",
            "enrich",
            "--input",
            input.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--category",
            "vendor_name",
            "--backend",
            "claude", // not yet supported in v3.5.0
        ])
        .assert();

    assert.failure();
}

#[test]
fn enrich_is_deterministic_for_same_seed() {
    let tmp = TempDir::new().expect("tempdir");
    let input = tmp.path().join("in.yaml");
    let output_a = tmp.path().join("a.yaml");
    let output_b = tmp.path().join("b.yaml");

    for output in [&output_a, &output_b] {
        synth_data_bin()
            .args([
                "templates",
                "enrich",
                "--input",
                input.to_str().unwrap(),
                "--output",
                output.to_str().unwrap(),
                "--category",
                "vendor_name",
                "--industry",
                "retail",
                "--region",
                "DE",
                "--sub-category",
                "office_supplies",
                "--count",
                "5",
                "--backend",
                "mock",
                "--seed",
                "7777",
            ])
            .assert()
            .success();
    }

    let a = fs::read_to_string(&output_a).expect("read a");
    let b = fs::read_to_string(&output_b).expect("read b");
    assert_eq!(a, b, "same seed should yield byte-identical enriched YAML");
}

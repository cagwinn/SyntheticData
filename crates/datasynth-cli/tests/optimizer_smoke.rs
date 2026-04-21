//! v4.1.2 — smoke test for the `optimizer` CLI subcommand family.
//!
//! Confirms that all 6 subcommands parse arguments correctly, emit a
//! structured JSON report to the declared `--output` path, and exit 0.
//! Deeper analytics per subcommand surface incrementally in v4.1.x.

use assert_cmd::Command;
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

const TEST_TIMEOUT_SECS: u64 = 30;

#[allow(deprecated)]
fn synth_data_bin() -> Command {
    let mut cmd = Command::cargo_bin("datasynth-data").unwrap();
    cmd.timeout(Duration::from_secs(TEST_TIMEOUT_SECS));
    cmd
}

fn run_optimizer(
    subcmd: &str,
    extra: &[&str],
    output_name: &str,
) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().expect("tempdir");
    let input = tmp.path().join("in.yaml");
    fs::write(&input, "engagement_id: test\n").expect("write input");
    let output = tmp.path().join(output_name);

    let mut args = vec![
        "optimizer",
        subcmd,
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ];
    args.extend(extra.iter().copied());

    let assert = synth_data_bin().args(args).assert();
    assert.success();
    (tmp, output)
}

#[test]
fn risk_scope_emits_report() {
    let (_tmp, output) = run_optimizer("risk-scope", &[], "risk-scope.json");
    let content = fs::read_to_string(&output).expect("read output");
    assert!(content.contains("\"command\""));
    assert!(content.contains("\"risk-scope\""));
}

#[test]
fn portfolio_emits_report() {
    let (_tmp, output) = run_optimizer("portfolio", &["--budget-hours", "1000"], "portfolio.json");
    let content = fs::read_to_string(&output).expect("read output");
    assert!(content.contains("\"command\""));
    assert!(content.contains("\"portfolio\""));
    assert!(content.contains("\"budget_hours\""));
}

#[test]
fn resources_emits_report() {
    let (_tmp, output) = run_optimizer("resources", &[], "resources.json");
    let content = fs::read_to_string(&output).expect("read output");
    assert!(content.contains("\"resources\""));
}

#[test]
fn conformance_emits_report() {
    let tmp = TempDir::new().expect("tempdir");
    let input = tmp.path().join("trace.json");
    let blueprint = tmp.path().join("bp.yaml");
    let output = tmp.path().join("conformance.json");
    fs::write(&input, "[]").expect("write input");
    fs::write(&blueprint, "name: bp\n").expect("write blueprint");
    synth_data_bin()
        .args([
            "optimizer",
            "conformance",
            "--input",
            input.to_str().unwrap(),
            "--blueprint",
            blueprint.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = fs::read_to_string(&output).expect("read output");
    assert!(content.contains("\"conformance\""));
}

#[test]
fn monte_carlo_emits_report() {
    let (_tmp, output) = run_optimizer(
        "monte-carlo",
        &["--runs", "100", "--seed", "123"],
        "mc.json",
    );
    let content = fs::read_to_string(&output).expect("read output");
    assert!(content.contains("\"monte-carlo\""));
    assert!(content.contains("\"runs\": 100"));
    assert!(content.contains("\"seed\": 123"));
}

#[test]
fn calibration_emits_report() {
    let (_tmp, output) = run_optimizer("calibration", &[], "cal.json");
    let content = fs::read_to_string(&output).expect("read output");
    assert!(content.contains("\"calibration\""));
}

//! v5.0+: integration tests for the `datasynth-data group …` subcommand
//! family — Task 10.6.
//!
//! Cheap tests (no orchestrator runs) cover the manifest action, the
//! shard-id validation path, and the auto-detection logic.  Heavy tests
//! that drive `EnhancedOrchestrator` end-to-end via `group shard`,
//! `group aggregate`, `group generate`, or auto-detected single-entity
//! generation are gated behind `#[ignore]` per the v5.0 memory-discipline
//! rules — each Mini-Nestlé entity peaks at ~17 GiB RSS so running the
//! aggregate suite from `cargo test -p datasynth-cli` would OOM a
//! workstation.
//!
//! Run cheap tests with:
//!
//!     cargo test -p datasynth-cli --test group_cli -- --test-threads=2
//!
//! Run a single ignored test with:
//!
//!     cargo test -p datasynth-cli --test group_cli \
//!         -- --test-threads=1 --ignored happy_path_name

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;

const TEST_TIMEOUT_SECS: u64 = 60;

#[allow(deprecated)]
fn synth_data_bin() -> Command {
    let mut cmd = Command::cargo_bin("datasynth-data").expect("datasynth-data binary must build");
    cmd.timeout(Duration::from_secs(TEST_TIMEOUT_SECS));
    cmd
}

/// Minimal-but-valid Mini-Nestlé-shaped GroupConfig — single CHF
/// presentation entity, no IC relationships, no generated blocks.  Keeps
/// the manifest cheap (no expanded entity blocks, no FX rates required
/// since presentation==functional, no ownership graph beyond the parent)
/// so the cheap tests stay sub-second.
fn write_minimal_group_config(path: &Path) {
    let yaml = r#"
id: "TEST_GROUP_CLI_2024_Q1"
name: "Test group for CLI tests"
presentation_currency: "CHF"
period:
  start_date: "2024-01-01"
  length: quarterly
seed: 42

scoping_profiles:
  significant:
    row_budget: 1000

ownership:
  parent_entity_code: TEST_PARENT
  entities:
    - code: TEST_PARENT
      country: CH
      functional_currency: CHF
      scoping_profile: significant
      consolidation_method: parent

fx:
  base_currency: CHF
  rate_source: inline
  rates: {}
  policy:
    balance_sheet: closing
    income_statement: average
    equity: historical

audit:
  group_materiality:
    basis: revenue
    percent: 0.01
"#;
    fs::write(path, yaml).expect("write minimal group config");
}

/// Single-entity GeneratorConfig YAML — used for the auto-detect
/// test that confirms the heuristic does NOT misclassify a non-group
/// config as a group.
fn write_minimal_generator_config(path: &Path) {
    let yaml = r#"
global:
  industry: manufacturing
  period_months: 1
  start_date: "2024-01-01"
  seed: 42
companies:
  - code: ACME
    name: Acme Inc
chart_of_accounts:
  complexity: small
output:
  output_directory: "./out"
"#;
    fs::write(path, yaml).expect("write minimal generator config");
}

// ── Cheap tests (no orchestrator) ────────────────────────────────────────────

/// Task 10.6 scenario 1 — `group manifest` happy path.
///
/// Round-trip: write the minimal fixture, invoke the CLI, parse the
/// emitted JSON back as a `GroupManifest`, and confirm the summary
/// fields match the fixture.
#[test]
fn group_manifest_happy_path() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("group.yaml");
    let out_path = tmp.path().join("manifest.json");
    write_minimal_group_config(&cfg_path);

    let assert = synth_data_bin()
        .args([
            "group",
            "manifest",
            "--config",
            cfg_path.to_str().unwrap(),
            "--out",
            out_path.to_str().unwrap(),
        ])
        .assert();
    assert.success();

    assert!(out_path.exists(), "manifest.json must be written");
    let bytes = fs::read(&out_path).expect("read manifest");
    let manifest: serde_json::Value =
        serde_json::from_slice(&bytes).expect("manifest must parse as JSON");

    // Sanity-check the round-trip — the manifest must carry the group
    // id, the presentation currency, exactly one entity (the parent),
    // and a non-empty shard plan.
    assert_eq!(manifest["group_id"], "TEST_GROUP_CLI_2024_Q1");
    assert_eq!(manifest["presentation_currency"], "CHF");
    let entities = manifest["ownership_graph"]["entities"]
        .as_array()
        .expect("ownership_graph.entities must be an array");
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0]["code"], "TEST_PARENT");
    let shards = manifest["shard_plan"]["shards"]
        .as_array()
        .expect("shard_plan.shards must be an array");
    assert!(!shards.is_empty(), "shard plan must contain >=1 shard");
}

/// Task 10.6 scenario 2 — `group manifest` rejects an invalid config.
///
/// Writes a YAML missing the required `presentation_currency` field;
/// confirms the CLI exits non-zero and the stderr mentions the parse
/// failure.
#[test]
fn group_manifest_invalid_config_fails() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("bad.yaml");
    let out_path = tmp.path().join("manifest.json");
    fs::write(
        &cfg_path,
        // Valid YAML but missing presentation_currency, ownership, fx,
        // period, seed — every required GroupConfig field bar `id`.
        "id: BAD\nname: missing-fields\n",
    )
    .expect("write bad config");

    let output = synth_data_bin()
        .args([
            "group",
            "manifest",
            "--config",
            cfg_path.to_str().unwrap(),
            "--out",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("run datasynth-data");

    assert!(
        !output.status.success(),
        "invalid config must produce a non-zero exit"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("parse") || stderr.contains("missing"),
        "stderr should mention parse / missing field; got: {stderr}"
    );
    assert!(
        !out_path.exists(),
        "manifest.json must not be written on failure"
    );
}

/// Task 10.6 scenario 3 — `group shard` with an unknown shard_id.
///
/// Builds a real manifest from the minimal fixture, then invokes
/// `group shard` with a bogus shard id and asserts the CLI exits with
/// code 2 and lists the valid ids.  This exercises the validation path
/// in `handle_group_shard` BEFORE any orchestrator construction kicks
/// in, so the test is cheap.
#[test]
fn group_shard_unknown_shard_id_fails_fast() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("group.yaml");
    let manifest_path = tmp.path().join("manifest.json");
    let out_path = tmp.path().join("shard_out");
    write_minimal_group_config(&cfg_path);

    // Build the manifest first.
    let assert = synth_data_bin()
        .args([
            "group",
            "manifest",
            "--config",
            cfg_path.to_str().unwrap(),
            "--out",
            manifest_path.to_str().unwrap(),
        ])
        .assert();
    assert.success();

    // Now run `group shard` with an obviously bogus id.
    let output = synth_data_bin()
        .args([
            "group",
            "shard",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--shard-id",
            "S_NOT_REAL",
            "--out",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("run datasynth-data");

    let exit_code = output.status.code().expect("exit code");
    assert_eq!(
        exit_code,
        2,
        "unknown shard_id must exit 2; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("S_NOT_REAL"),
        "stderr should echo the bogus shard id; got: {stderr}"
    );
    assert!(
        stderr.contains("valid ids"),
        "stderr should list the valid ids; got: {stderr}"
    );
}

/// Task 10.6 scenario 7 (cheap part) — auto-detect on
/// `Commands::Generate` correctly classifies a group config.
///
/// We do NOT actually run the orchestrator — the heavy path is gated
/// behind `#[ignore]` below.  Instead we use a sentinel: the binary's
/// help / error trace mentions "auto-detected group config" via tracing.
/// To exercise the heuristic without paying for orchestrator runs we
/// craft a config that is recognised as a group config but fails inside
/// `validate` (no parent entity), and assert the error path is the
/// group one (exit 2 with a config-error mention) rather than the
/// single-entity path (which would surface a different error).
#[test]
fn generate_auto_detect_group_config_dispatches_into_group() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("group.yaml");
    let out_path = tmp.path().join("out");

    // GroupConfig-shaped YAML that DOES carry the auto-detect markers
    // (presentation_currency + ownership) but fails validation: the
    // declared parent_entity_code is not present in the entities list,
    // which `datasynth_group::validate::validate` rejects with an
    // error message containing "parent_entity_code".
    let yaml = r#"
id: "AUTO_DETECT_TEST"
presentation_currency: "CHF"
period: { start_date: "2024-01-01", length: quarterly }
seed: 1
ownership:
  parent_entity_code: NOT_DECLARED
  entities: []
fx:
  base_currency: CHF
  rate_source: inline
  rates: {}
  policy: { balance_sheet: closing, income_statement: average, equity: historical }
"#;
    fs::write(&cfg_path, yaml).expect("write group cfg");

    let output = synth_data_bin()
        .args([
            "generate",
            "--config",
            cfg_path.to_str().unwrap(),
            "--output",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("run datasynth-data");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !output.status.success(),
        "must fail since validate rejects the config; stderr={stderr}",
    );
    // The group validator emits an error mentioning parent_entity_code;
    // the single-entity GeneratorConfig parser would emit something
    // entirely different (about missing `global` / `companies`).  The
    // presence of `parent_entity_code` in stderr is the proof we hit
    // the group path.
    assert!(
        stderr.contains("parent_entity_code") || stderr.contains("NOT_DECLARED"),
        "auto-detected group config should surface group-validator errors; \
         stdout={stdout}, stderr={stderr}",
    );
}

/// Task 10.6 scenario 7 (cheap part, complement) — auto-detect leaves
/// non-group configs on the single-entity path.
///
/// A YAML that lacks the GroupConfig markers (no `presentation_currency`,
/// no `ownership`) must NOT be classified as a group config; the
/// auto-detect heuristic should fall through and the single-entity
/// pipeline should pick up the parse.  We craft a config that fails
/// the single-entity parse so the test can complete sub-second; the
/// stderr message must mention single-entity-shaped failures rather
/// than anything group-related.
#[test]
fn generate_auto_detect_passthrough_for_non_group_config() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("single.yaml");
    let out_path = tmp.path().join("out");

    // Single-entity-shaped YAML with no `presentation_currency` and no
    // `ownership` — must NOT trip the group auto-detector.  We make it
    // intentionally invalid (missing required `chart_of_accounts`) so
    // the existing GeneratorConfig parser fails fast; this is the
    // sentinel that confirms we landed on the single-entity path.
    let yaml = r#"
global:
  industry: manufacturing
  period_months: 1
  start_date: "2024-01-01"
companies:
  - code: ACME
    name: Acme Inc
"#;
    fs::write(&cfg_path, yaml).expect("write single cfg");

    let output = synth_data_bin()
        .args([
            "generate",
            "--config",
            cfg_path.to_str().unwrap(),
            "--output",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("run datasynth-data");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "must fail since the GeneratorConfig parse is incomplete; stderr={stderr}"
    );
    // We must NOT have ended up on the group path — group errors
    // mention `parent_entity_code` / `presentation_currency` /
    // `ownership`.  Single-entity errors mention `chart_of_accounts`,
    // `output`, missing required field on GeneratorConfig, etc.
    assert!(
        !stderr.contains("parent_entity_code"),
        "non-group config must not be auto-detected as a group config; stderr={stderr}"
    );
}

/// v5.3+ — `group generate-chain` rejects an empty periods array
/// before any orchestrator run.  Cheap: hits the validation path only.
#[test]
fn group_generate_chain_rejects_empty_periods() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("group.yaml");
    let periods_path = tmp.path().join("periods.json");
    let out_dir = tmp.path().join("chain_out");
    write_minimal_group_config(&cfg_path);
    fs::write(&periods_path, "[]").expect("write empty periods");

    let output = synth_data_bin()
        .args([
            "group",
            "generate-chain",
            "--config",
            cfg_path.to_str().unwrap(),
            "--periods",
            periods_path.to_str().unwrap(),
            "--out",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("run datasynth-data");

    assert!(
        !output.status.success(),
        "empty periods array must produce a non-zero exit"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("at least one entry") || stderr.contains("must be non-empty"),
        "stderr should mention empty periods; got: {stderr}"
    );
}

/// v5.3+ — `group generate-chain` rejects a malformed periods JSON
/// payload (wrong shape).  Cheap: hits the JSON parse path only.
#[test]
fn group_generate_chain_rejects_malformed_periods() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("group.yaml");
    let periods_path = tmp.path().join("periods.json");
    let out_dir = tmp.path().join("chain_out");
    write_minimal_group_config(&cfg_path);
    // Wrong shape — array of strings instead of array of PeriodChainSpec.
    fs::write(&periods_path, r#"["not", "a", "spec"]"#).expect("write bad periods");

    let output = synth_data_bin()
        .args([
            "group",
            "generate-chain",
            "--config",
            cfg_path.to_str().unwrap(),
            "--periods",
            periods_path.to_str().unwrap(),
            "--out",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("run datasynth-data");

    assert!(
        !output.status.success(),
        "malformed periods JSON must produce a non-zero exit"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("PeriodChainSpec") || stderr.contains("parse"),
        "stderr should mention parse failure; got: {stderr}"
    );
}

// ── Heavy tests (orchestrator-driven, #[ignore]d) ───────────────────────────
//
// These exercise the full v5.0 pipeline and each peaks at ~17 GiB RSS
// per shard.  Run them deliberately with:
//
//   cargo test -p datasynth-cli --test group_cli \
//       -- --test-threads=1 --ignored
//
// per the v5.0 memory-discipline rules.

/// Task 10.6 scenario 4 — `group shard` happy path.
///
/// Drives `EnhancedOrchestrator` once per entity in the resolved shard.
/// `#[ignore]`d for memory safety.
#[test]
#[ignore = "v5.0: drives orchestrator end-to-end (~17 GiB RSS per entity); run manually with --ignored"]
fn group_shard_happy_path_ignored() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("group.yaml");
    let manifest_path = tmp.path().join("manifest.json");
    let shard_out = tmp.path().join("shard_out");
    write_minimal_group_config(&cfg_path);

    synth_data_bin()
        .args([
            "group",
            "manifest",
            "--config",
            cfg_path.to_str().unwrap(),
            "--out",
            manifest_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    // Read the manifest to discover the shard id we should run.
    let manifest_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("read manifest"))
            .expect("parse manifest");
    let shard_id = manifest_json["shard_plan"]["shards"][0]["shard_id"]
        .as_str()
        .expect("shard plan must have at least one shard")
        .to_string();

    synth_data_bin()
        .args([
            "group",
            "shard",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--shard-id",
            &shard_id,
            "--out",
            shard_out.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(
        shard_out.join("entities").join("TEST_PARENT").exists(),
        "shard runner must create entities/TEST_PARENT/"
    );
}

/// Task 10.6 scenario 5 — `group aggregate` happy path.
///
/// Requires shard outputs that you can't produce without the
/// orchestrator. `#[ignore]`d.
#[test]
#[ignore = "v5.0: requires shard-runner output (~17 GiB RSS); run manually with --ignored"]
fn group_aggregate_happy_path_ignored() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("group.yaml");
    let manifest_path = tmp.path().join("manifest.json");
    let shards_dir = tmp.path().join("shards");
    let agg_out = tmp.path().join("aggregate_out");
    write_minimal_group_config(&cfg_path);

    synth_data_bin()
        .args([
            "group",
            "manifest",
            "--config",
            cfg_path.to_str().unwrap(),
            "--out",
            manifest_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("read manifest"))
            .expect("parse manifest");
    let shard_id = manifest_json["shard_plan"]["shards"][0]["shard_id"]
        .as_str()
        .expect("shard plan must have at least one shard")
        .to_string();

    synth_data_bin()
        .args([
            "group",
            "shard",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--shard-id",
            &shard_id,
            "--out",
            shards_dir.to_str().unwrap(),
        ])
        .assert()
        .success();

    synth_data_bin()
        .args([
            "group",
            "aggregate",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--shards-dir",
            shards_dir.to_str().unwrap(),
            "--out",
            agg_out.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(
        agg_out
            .join("consolidated")
            .join("consolidated_financial_statements.json")
            .exists(),
        "aggregate must emit consolidated_financial_statements.json"
    );
}

/// Task 10.6 scenario 6 — `group generate` happy path.
///
/// Two orchestrator runs back-to-back (the standalone path runs
/// manifest + shards + aggregate in one call). `#[ignore]`d.
#[test]
#[ignore = "v5.0: drives orchestrator + aggregate end-to-end (~17 GiB RSS); run manually with --ignored"]
fn group_generate_happy_path_ignored() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("group.yaml");
    let out_dir = tmp.path().join("standalone_out");
    write_minimal_group_config(&cfg_path);

    synth_data_bin()
        .args([
            "group",
            "generate",
            "--config",
            cfg_path.to_str().unwrap(),
            "--out",
            out_dir.to_str().unwrap(),
            "--no-parallel-shards",
        ])
        .assert()
        .success();

    assert!(out_dir.join("manifest.json").exists());
    assert!(out_dir
        .join("consolidated")
        .join("consolidated_financial_statements.json")
        .exists());
}

/// Task 10.6 scenario 7 (heavy part) — auto-detect on `generate`
/// dispatches into `generate_standalone` and emits group-shaped output.
///
/// Heavy by definition; `#[ignore]`d.
#[test]
#[ignore = "v5.0: drives orchestrator + aggregate end-to-end (~17 GiB RSS); run manually with --ignored"]
fn generate_auto_detect_group_config_runs_standalone_ignored() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("group.yaml");
    let out_dir = tmp.path().join("auto_detect_out");
    write_minimal_group_config(&cfg_path);

    synth_data_bin()
        .args([
            "generate",
            "--config",
            cfg_path.to_str().unwrap(),
            "--output",
            out_dir.to_str().unwrap(),
        ])
        .assert()
        .success();

    // Group-shaped output: manifest + consolidated/ tree.
    assert!(out_dir.join("manifest.json").exists());
    assert!(out_dir.join("consolidated").exists());
}

// Compile-time assertion that we exercise the helper for non-group
// configs.  Keeps `write_minimal_generator_config` referenced so a
// future PR that drops the test cannot leave a dead helper behind.
#[test]
fn _helper_is_used() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("single.yaml");
    write_minimal_generator_config(&path);
    assert!(path.exists());
}

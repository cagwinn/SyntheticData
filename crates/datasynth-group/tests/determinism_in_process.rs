//! Task 11.4 — determinism harness: in-process vs subprocess byte-equality.
//!
//! The v5.0 contract is that `generate_standalone(&cfg, dir, opts)` is
//! deterministic over `(cfg, opts)` and that the multi-step CLI
//! pipeline (`group manifest` + `group shard` ×N + `group aggregate`)
//! produces a byte-identical archive.  This file pins both halves of
//! the contract:
//!
//! 1. **`generate_standalone_twice_byte_identical`** — calls
//!    [`generate_standalone`] twice with identical inputs and asserts
//!    every emitted file matches byte-for-byte across the two runs.
//!    This is the in-process determinism guarantee.
//!
//! 2. **`subprocess_pipeline_matches_standalone`** — drives the
//!    `datasynth-data` CLI binary via [`assert_cmd`] to run
//!    `group manifest` → `group shard` ×N → `group aggregate`, then
//!    byte-diffs the resulting archive against
//!    [`generate_standalone`]'s output.  This catches any divergence
//!    between the in-process and subprocess code paths (e.g. a stray
//!    timestamp in the subprocess output writer that the in-process
//!    path skips).
//!
//! # Why `#[ignore]`?
//!
//! Each test sequences the **full** Mini-Nestlé pipeline at least
//! once; `subprocess_pipeline_matches_standalone` runs it twice (in
//! both modes).  Each entity peaks at ~17 GiB RSS for ~15 minutes — a
//! full Mini-Nestlé run is ~85 GiB peak / 60+ min.  Running on the
//! workstation OOMs the host.  The XXL Azure VM verification harness
//! (`docs/superpowers/plans/2026-04-23-group-audit-v5.0-xxl-verification.md`
//! §3.2.b) is the canonical place to exercise these tests.
//!
//! # Determinism options
//!
//! Both tests pass [`StandaloneOptions { parallel_shards: false, .. }`]
//! so two runs over identical input produce byte-identical output.
//! Rayon's scheduler interleaving is NOT a determinism property the
//! tests aim to guarantee — see [`crate::generate_standalone`] module
//! docs for the trade-off.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use datasynth_group::{generate_standalone, GroupConfig, GroupManifest, StandaloneOptions};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Recursively walk `root`, returning every regular file's path
/// **relative to `root`** along with its raw bytes.  Output is sorted
/// lexicographically by relative path.
fn walk_dir_recursive(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    walk_one(root, root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn walk_one(root: &Path, current: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
    let read = match fs::read_dir(current) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_one(root, &path, out);
        } else if path.is_file() {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            let bytes = fs::read(&path).expect("read file");
            out.push((rel, bytes));
        }
    }
}

/// Compare two archive directories file-by-file.  Returns a list of
/// human-readable diff messages — empty when the archives are
/// byte-identical.
fn diff_archives(a: &Path, b: &Path) -> Vec<String> {
    let a_files: BTreeMap<PathBuf, Vec<u8>> = walk_dir_recursive(a).into_iter().collect();
    let b_files: BTreeMap<PathBuf, Vec<u8>> = walk_dir_recursive(b).into_iter().collect();

    let mut diffs: Vec<String> = Vec::new();
    for (path, bytes_a) in &a_files {
        match b_files.get(path) {
            None => diffs.push(format!(
                "MISSING in B: {} (A has {} bytes)",
                path.display(),
                bytes_a.len()
            )),
            Some(bytes_b) if bytes_a != bytes_b => diffs.push(format!(
                "BYTES DIFFER: {} (A={}b, B={}b)",
                path.display(),
                bytes_a.len(),
                bytes_b.len(),
            )),
            Some(_) => {}
        }
    }
    for path in b_files.keys() {
        if !a_files.contains_key(path) {
            diffs.push(format!(
                "MISSING in A: {} (B has {} bytes)",
                path.display(),
                b_files[path].len()
            ));
        }
    }
    diffs
}

/// Load and parse the canonical Mini-Nestlé fixture.
fn load_mini_nestle_config() -> GroupConfig {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse")
}

fn deterministic_opts() -> StandaloneOptions {
    StandaloneOptions {
        parallel_shards: false,
        ..StandaloneOptions::default()
    }
}

/// Find the `datasynth-data` binary.  Prefers
/// `assert_cmd::Command::cargo_bin` when it resolves, falls back to
/// `CARGO_TARGET_DIR / target/debug/datasynth-data` for cross-crate
/// invocation (the binary lives in `datasynth-cli`, so the env var
/// `CARGO_BIN_EXE_datasynth-data` is not set in this crate's test
/// process).
fn datasynth_data_bin() -> PathBuf {
    // Honour CARGO_BIN_EXE if Cargo populated it (only in the binary's
    // home crate, but harmless to check).
    if let Some(p) = std::env::var_os("CARGO_BIN_EXE_datasynth-data") {
        return PathBuf::from(p);
    }
    // Walk up from CARGO_MANIFEST_DIR to find the workspace target/debug.
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("target")
        });
    target_dir.join("debug").join("datasynth-data")
}

/// Run `datasynth-data group <subcommand>` with the given args, panic
/// with stderr on non-zero exit.
fn run_cli(args: &[&str]) {
    let bin = datasynth_data_bin();
    let output = Command::new(&bin)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn {}: {e}", bin.display()));
    if !output.status.success() {
        panic!(
            "datasynth-data {} failed (status {:?}): stderr=\n{}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Two consecutive [`generate_standalone`] runs over the same input
/// must produce byte-identical archives (sequential shards remove
/// rayon interleaving from the trace).
///
/// See module-level rustdoc for the `#[ignore]` rationale.
#[test]
#[ignore = "drives 2× full standalone runs (~170 GiB peak, 2+ hours combined) — XXL VM only"]
fn generate_standalone_twice_byte_identical() {
    let cfg = load_mini_nestle_config();

    let tmp1 = TempDir::new().expect("tempdir 1");
    let tmp2 = TempDir::new().expect("tempdir 2");

    let _ =
        generate_standalone(&cfg, tmp1.path(), &deterministic_opts()).expect("standalone run 1");
    let _ =
        generate_standalone(&cfg, tmp2.path(), &deterministic_opts()).expect("standalone run 2");

    let diffs = diff_archives(tmp1.path(), tmp2.path());
    assert!(
        diffs.is_empty(),
        "{} files differ between two `generate_standalone` runs over the same input:\n{}",
        diffs.len(),
        diffs.join("\n"),
    );
}

/// Drive the multi-step CLI pipeline (`group manifest` →
/// `group shard` ×N → `group aggregate`) and compare the resulting
/// archive against [`generate_standalone`]'s output for the same
/// input.  Mismatch means the in-process and subprocess paths have
/// diverged — typically a bug in one of the sub-command handlers.
///
/// See module-level rustdoc for the `#[ignore]` rationale.
#[test]
#[ignore = "drives subprocess + standalone pipelines (~170 GiB peak, 2+ hours combined) — XXL VM only"]
fn subprocess_pipeline_matches_standalone() {
    let cfg = load_mini_nestle_config();

    // ── In-process baseline ─────────────────────────────────────────────
    let standalone_dir = TempDir::new().expect("standalone tempdir");
    let _ = generate_standalone(&cfg, standalone_dir.path(), &deterministic_opts())
        .expect("standalone run");

    // ── Subprocess pipeline ─────────────────────────────────────────────
    let subprocess_dir = TempDir::new().expect("subprocess tempdir");
    // Persist the source YAML to a file so the CLI can `--config` it.
    let cfg_path = subprocess_dir.path().join("group.yaml");
    let yaml = serde_yaml::to_string(&cfg).expect("serialise cfg");
    fs::write(&cfg_path, yaml).expect("write cfg yaml");

    // 1. group manifest
    let manifest_path = subprocess_dir.path().join("manifest.json");
    run_cli(&[
        "group",
        "manifest",
        "--config",
        cfg_path.to_str().unwrap(),
        "--out",
        manifest_path.to_str().unwrap(),
    ]);

    // 2. group shard for every shard in the manifest
    let manifest_bytes = fs::read(&manifest_path).expect("read manifest");
    let manifest: GroupManifest = serde_json::from_slice(&manifest_bytes).expect("parse manifest");
    for shard in &manifest.shard_plan.shards {
        run_cli(&[
            "group",
            "shard",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--shard-id",
            &shard.shard_id,
            "--out",
            subprocess_dir.path().to_str().unwrap(),
        ]);
    }

    // 3. group aggregate
    run_cli(&[
        "group",
        "aggregate",
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--shards-dir",
        subprocess_dir.path().to_str().unwrap(),
        "--out",
        subprocess_dir.path().to_str().unwrap(),
    ]);

    // ── Compare ─────────────────────────────────────────────────────────
    //
    // The subprocess flow writes `group.yaml` into the subprocess
    // tempdir as well — strip that from the comparison since the
    // in-process flow doesn't (it consumes the cfg directly without
    // persisting it).
    let _ = fs::remove_file(&cfg_path);

    let diffs = diff_archives(standalone_dir.path(), subprocess_dir.path());
    assert!(
        diffs.is_empty(),
        "{} files differ between subprocess and in-process pipelines:\n{}",
        diffs.len(),
        diffs.join("\n"),
    );
}

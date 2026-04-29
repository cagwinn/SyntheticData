//! Task 11.1 — Mini-Nestlé golden archive harness.
//!
//! Two `#[ignore]`d tests that anchor the v5.0 standalone pipeline to a
//! committed Mini-Nestlé reference archive at
//! `tests/golden/mini_nestle/`:
//!
//! 1. **`check_against_golden`** — runs [`generate_standalone`] over the
//!    Mini-Nestlé fixture, then walks the freshly generated archive
//!    side-by-side with `tests/golden/mini_nestle/` and asserts every
//!    file matches byte-for-byte.  Mismatches are dumped to
//!    `target/golden_diff.txt` (a list of paths and the per-file
//!    expected-vs-actual length so the regen workflow has something to
//!    grep).  Until the first regen run lands the directory contains
//!    only `.gitkeep` and the test fails with a "golden archive is
//!    empty — run regenerate_golden first" message.
//!
//! 2. **`regenerate_golden`** — same setup, but copies the freshly
//!    generated archive into `tests/golden/mini_nestle/` (overwriting),
//!    so a future `check_against_golden` will pass.  Run with:
//!
//!    ```text
//!    cargo test -p datasynth-group --test golden_archive \
//!        -- --ignored regenerate_golden
//!    ```
//!
//!    after intentional v5.0 schema or determinism changes that update
//!    the canonical output.  The committed `.gitkeep` is preserved so
//!    the empty-bootstrap path keeps working before the first regen.
//!
//! # Determinism
//!
//! Both tests pass [`StandaloneOptions { parallel_shards: false, .. }`]
//! so two consecutive runs over the same input produce byte-identical
//! archives (rayon's scheduler interleaving is removed from the trace
//! — see [`crate::generate_standalone`] module docs).
//!
//! # `#[ignore]` rationale
//!
//! [`generate_standalone`] over Mini-Nestlé sequences five full
//! [`datasynth_runtime::EnhancedOrchestrator`] runs back-to-back inside
//! one process; each entity peaks at ~17 GiB RSS for ~15 minutes, so
//! the combined archive run is ~85 GiB peak / 60+ min total.  Running
//! it on the workstation OOMs the host.  The XXL Azure VM verification
//! harness
//! (`docs/superpowers/plans/2026-04-23-group-audit-v5.0-xxl-verification.md`
//! §3.2.b) is the canonical place to exercise these tests.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use datasynth_group::{generate_standalone, GroupConfig, StandaloneOptions};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Recursively walk `root`, returning every regular file's path
/// **relative to `root`** along with its raw bytes.  Output is sorted
/// lexicographically by relative path so two callers walking the same
/// tree produce identical traversal orders.
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
            // Skip the .gitkeep placeholder so a populated golden won't
            // also expect the bootstrap marker.  When regenerating, we
            // explicitly preserve .gitkeep separately.
            if path.file_name().and_then(|s| s.to_str()) == Some(".gitkeep") {
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            let bytes = fs::read(&path).expect("read regular file");
            out.push((rel, bytes));
        }
    }
}

/// Compare two archives file-by-file.  Returns a list of one
/// human-readable diff message per mismatch (empty Vec → archives are
/// byte-identical modulo the `.gitkeep` skip).
fn diff_archives(generated: &Path, golden: &Path) -> Vec<String> {
    let gen_files = walk_dir_recursive(generated);
    let golden_files = walk_dir_recursive(golden);

    let gen_map: std::collections::BTreeMap<PathBuf, Vec<u8>> = gen_files.into_iter().collect();
    let golden_map: std::collections::BTreeMap<PathBuf, Vec<u8>> =
        golden_files.into_iter().collect();

    let mut diffs: Vec<String> = Vec::new();

    // Files present in golden but missing from generated.
    for path in golden_map.keys() {
        if !gen_map.contains_key(path) {
            diffs.push(format!(
                "MISSING in generated: {} (golden has {} bytes)",
                path.display(),
                golden_map[path].len()
            ));
        }
    }
    // Files present in generated but missing from golden.
    for path in gen_map.keys() {
        if !golden_map.contains_key(path) {
            diffs.push(format!(
                "EXTRA in generated: {} ({} bytes)",
                path.display(),
                gen_map[path].len()
            ));
        }
    }
    // Byte-level mismatches on shared paths.
    for (path, gen_bytes) in &gen_map {
        if let Some(golden_bytes) = golden_map.get(path) {
            if gen_bytes != golden_bytes {
                diffs.push(format!(
                    "BYTES DIFFER: {} (generated={}b, golden={}b)",
                    path.display(),
                    gen_bytes.len(),
                    golden_bytes.len()
                ));
            }
        }
    }

    diffs
}

/// Load and parse the canonical Mini-Nestlé fixture.  Trim only when
/// the trim is itself part of the test (this harness uses the full
/// 5-entity fixture).
fn load_mini_nestle_config() -> GroupConfig {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse into GroupConfig")
}

/// Path to the committed golden archive under
/// `crates/datasynth-group/tests/golden/mini_nestle/`.
fn golden_archive_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("mini_nestle")
}

/// Standalone options for golden-archive runs: sequential shards so
/// the output trace is deterministic across two runs.
fn deterministic_opts() -> StandaloneOptions {
    StandaloneOptions {
        parallel_shards: false,
        ..StandaloneOptions::default()
    }
}

/// Recursively copy `src` into `dst`, overwriting existing files.
/// Skips the `.gitkeep` placeholder under `src` so we don't propagate
/// the bootstrap marker into the regenerated golden.
fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("create destination directory");
    for entry in fs::read_dir(src).expect("read source directory").flatten() {
        let path = entry.path();
        let name = match path.file_name() {
            Some(n) => n.to_owned(),
            None => continue,
        };
        if path.is_dir() {
            copy_dir_recursive(&path, &dst.join(&name));
        } else if path.is_file() && name != std::ffi::OsStr::new(".gitkeep") {
            fs::copy(&path, dst.join(&name)).expect("copy regular file");
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Runs `generate_standalone` over the Mini-Nestlé fixture and
/// byte-compares the resulting archive against the committed golden at
/// `tests/golden/mini_nestle/`.
///
/// On mismatch, dumps the per-file diff list to `target/golden_diff.txt`
/// (relative to `CARGO_TARGET_DIR`) and panics with the path so the
/// failure is greppable from CI logs.
///
/// **First-run bootstrap:** until `regenerate_golden` is run on the
/// XXL VM, `tests/golden/mini_nestle/` contains only a `.gitkeep`
/// placeholder.  This test fails with `"golden archive is empty — run
/// regenerate_golden first"` so the gap is explicit rather than a
/// false-positive `ok`.
///
/// See module-level rustdoc for the `#[ignore]` rationale.
#[test]
#[ignore = "runs full standalone pipeline (~85 GiB peak RSS, 60+ min) — XXL VM only"]
fn check_against_golden() {
    let golden = golden_archive_path();
    let golden_files = walk_dir_recursive(&golden);
    if golden_files.is_empty() {
        panic!(
            "golden archive is empty — run regenerate_golden first.\n\
             Path: {}\n\
             Run on the XXL VM: cargo test -p datasynth-group --test golden_archive \
             -- --ignored regenerate_golden",
            golden.display()
        );
    }

    let cfg = load_mini_nestle_config();
    let tmp = TempDir::new().expect("tempdir");
    let out_dir = tmp.path();

    let _summary = generate_standalone(&cfg, out_dir, &deterministic_opts())
        .expect("generate_standalone must succeed for Mini-Nestlé");

    let diffs = diff_archives(out_dir, &golden);
    if !diffs.is_empty() {
        // Dump the diff to a stable target-dir path so a CI log can
        // tail it without us streaming MB of bytes through stdout.
        let target_dir = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("..")
                    .join("target")
            });
        let _ = fs::create_dir_all(&target_dir);
        let diff_path = target_dir.join("golden_diff.txt");
        let body = diffs.join("\n");
        let _ = fs::write(&diff_path, &body);
        panic!(
            "{} files differ from golden archive — see {}",
            diffs.len(),
            diff_path.display()
        );
    }
}

/// Same as `check_against_golden`, but copies the freshly generated
/// archive INTO `tests/golden/mini_nestle/` (overwriting any existing
/// content).  Use this after intentional schema or determinism changes
/// that move the canonical output.
///
/// See module-level rustdoc for the regen workflow.
#[test]
#[ignore = "regenerate"]
fn regenerate_golden() {
    let cfg = load_mini_nestle_config();
    let tmp = TempDir::new().expect("tempdir");
    let out_dir = tmp.path();

    let _summary = generate_standalone(&cfg, out_dir, &deterministic_opts())
        .expect("generate_standalone must succeed for Mini-Nestlé");

    let golden = golden_archive_path();
    // Wipe everything except .gitkeep so a stale layout doesn't survive.
    if golden.exists() {
        for entry in fs::read_dir(&golden).expect("read golden dir").flatten() {
            let path = entry.path();
            if path.file_name() == Some(std::ffi::OsStr::new(".gitkeep")) {
                continue;
            }
            if path.is_dir() {
                fs::remove_dir_all(&path).expect("remove stale dir");
            } else {
                fs::remove_file(&path).expect("remove stale file");
            }
        }
    } else {
        fs::create_dir_all(&golden).expect("create golden dir");
    }
    copy_dir_recursive(out_dir, &golden);
    eprintln!("regenerated golden archive at {}", golden.display());
}

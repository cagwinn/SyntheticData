//! Integration test for per-entity output routing via `OutputRootConfig`.
//!
//! Exercises [`datasynth_cli::output_writer::write_all_output_with_root`]
//! directly with a minimal [`EnhancedGenerationResult::default()`] so the
//! test doesn't spin up a full `EnhancedOrchestrator` — the SAF-T smoke
//! test already covers the full pipeline end-to-end and is memory-heavy.
//!
//! The assertions here are purely about routing: flat mode writes at the
//! root, per-entity mode writes under `{root}/entities/{code}/`.

use datasynth_cli::output_writer::write_all_output_with_root;
use datasynth_output::OutputRootConfig;
use datasynth_runtime::enhanced_orchestrator::EnhancedGenerationResult;
use tempfile::TempDir;

fn minimal_result() -> EnhancedGenerationResult {
    // EnhancedGenerationResult derives Default — all snapshots are empty.
    // The output writer creates only the directories it needs and skips
    // empty slices, so the result is a no-op except for ensuring the
    // target directory exists.
    EnhancedGenerationResult::default()
}

#[test]
fn flat_layout_writes_at_root() {
    let tmp = TempDir::new().expect("tempdir");
    let root = OutputRootConfig::flat(tmp.path());
    let result = minimal_result();

    write_all_output_with_root(
        &result,
        &root,
        datasynth_config::ExportLayout::Nested,
        &[datasynth_config::FileFormat::Json],
    )
    .expect("flat layout should succeed");

    // The function creates the root dir even for an empty result.
    assert!(tmp.path().exists());
    assert!(tmp.path().is_dir());
    // No `entities/` subdir is created in flat mode.
    assert!(
        !tmp.path().join("entities").exists(),
        "flat mode must not create entities/ subdir"
    );
}

#[test]
fn per_entity_layout_routes_under_entities_code() {
    let tmp = TempDir::new().expect("tempdir");
    let root = OutputRootConfig::per_entity(tmp.path(), "NESTLE_SA");
    let result = minimal_result();

    write_all_output_with_root(
        &result,
        &root,
        datasynth_config::ExportLayout::Nested,
        &[datasynth_config::FileFormat::Json],
    )
    .expect("per-entity layout should succeed");

    let expected = tmp.path().join("entities").join("NESTLE_SA");
    assert!(expected.exists(), "{expected:?} must exist");
    assert!(expected.is_dir());

    // Root dir itself should contain only the entities subdir in
    // per-entity mode — nothing else, because all writes compose their
    // paths from the effective_dir().
    let root_entries: Vec<String> = std::fs::read_dir(tmp.path())
        .expect("read_dir tmp")
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        root_entries,
        vec!["entities".to_string()],
        "only entities/ should be at root in per-entity mode, found {root_entries:?}"
    );
}

#[test]
fn per_entity_without_code_falls_back_to_flat_layout() {
    // Defensive: a caller that sets per_entity_subtree: true but
    // forgets entity_code must still produce a usable archive at the
    // root, not a bogus `/tmp/.../entities/` without a code leaf.
    let tmp = TempDir::new().expect("tempdir");
    let root = OutputRootConfig {
        root_dir: tmp.path().to_path_buf(),
        per_entity_subtree: true,
        entity_code: None,
    };
    let result = minimal_result();

    write_all_output_with_root(
        &result,
        &root,
        datasynth_config::ExportLayout::Nested,
        &[datasynth_config::FileFormat::Json],
    )
    .expect("fallback routing should succeed");

    assert!(tmp.path().exists());
    assert!(
        !tmp.path().join("entities").exists(),
        "no entities/ subdir when entity_code is None"
    );
}

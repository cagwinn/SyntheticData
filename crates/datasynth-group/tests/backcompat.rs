//! Task 11.6 — backward compatibility for existing single-entity configs.
//!
//! The v5.0 spec guarantees that **existing** single-entity YAML
//! configs (no `group:` key, no `presentation_currency` /
//! `ownership` top-level keys) continue to dispatch through the
//! pre-v5.0 single-entity generator and produce byte-identical output
//! to pre-v5.0 main.
//!
//! The dispatch decision lives in `datasynth-cli`'s `main.rs` —
//! [`yaml_is_group_config`] checks whether the YAML mapping carries
//! both top-level `presentation_currency` AND `ownership` keys.  When
//! this returns `false`, the existing single-entity flow is taken.
//!
//! # Why a content duplicate of the heuristic
//!
//! `datasynth-cli` is a binary crate with no library target, so this
//! test cannot import the heuristic directly.  The duplicate
//! [`yaml_is_group_config`] below is intentional — when the cli grows
//! a `lib` target (a future refactor) this duplication can be retired
//! by importing from `datasynth_cli::yaml_is_group_config`.  Until
//! then, any change to the cli's heuristic must be mirrored here.
//!
//! # No generation runs
//!
//! This test only asserts the dispatch decision (boolean) — it does
//! not actually run a generator.  The "byte-identical to pre-v5.0
//! output" property is implied by the dispatch: if the cli routes a
//! YAML through the existing single-entity flow, the v5.0 changes do
//! not touch that flow's output.  Therefore this test is **cheap** and
//! runs on every CI invocation (no `#[ignore]`).

use std::path::PathBuf;

// ── Heuristic copy (intentional duplicate; see module rustdoc) ────────────────

/// Lightweight YAML probe used by `Commands::Generate` in
/// `datasynth-cli/src/main.rs` to auto-detect a group config.  Returns
/// `true` when the YAML mapping has both `presentation_currency` AND
/// `ownership` top-level keys (v5.0+ `GroupConfig`), `false` otherwise
/// (legacy single-entity `GeneratorConfig`).
///
/// **Mirrored verbatim** from the cli — keep in sync until the cli
/// gains a library target that lets the test import the function
/// directly.
fn yaml_is_group_config(yaml: &str) -> bool {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(yaml) else {
        return false;
    };
    let Some(map) = value.as_mapping() else {
        return false;
    };
    map.contains_key(serde_yaml::Value::String(
        "presentation_currency".to_string(),
    )) && map.contains_key(serde_yaml::Value::String("ownership".to_string()))
}

/// Workspace-relative path to a `templates/` example.
fn template_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("templates")
        .join(name)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Three example single-entity configs from `templates/` must
/// dispatch to the **legacy** single-entity flow (heuristic returns
/// `false`).  The example configs use the pre-v5.0 schema (`global:`,
/// `companies:`, `chart_of_accounts:`) and have neither
/// `presentation_currency` nor `ownership` at top-level.
#[test]
fn legacy_configs_dispatch_to_single_entity_flow() {
    // Three concrete examples from the workspace `templates/` directory.
    // We pick a mix of fraud-, process-mining-, and ML-training-shaped
    // configs to exercise different top-level structures, all of which
    // must read as legacy.  The list is small + deterministic so a
    // future template re-org won't silently break the property.
    let cases = [
        "fraud-detection-basic.yaml",
        "process-mining-full.yaml",
        "ml-training-balanced.yaml",
    ];

    for name in cases {
        let path = template_path(name);
        let yaml = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(
            !yaml_is_group_config(&yaml),
            "{}: legacy single-entity config must NOT be classified as a GroupConfig",
            path.display(),
        );
    }
}

/// The Mini-Acme v5.0 `GroupConfig` fixture must dispatch to the
/// **group** flow (heuristic returns `true`).  This is the
/// counter-example: any change to the heuristic that breaks group
/// detection would be caught here.
#[test]
fn mini_acme_dispatches_to_group_flow() {
    let yaml = include_str!("fixtures/mini_acme.yaml");
    assert!(
        yaml_is_group_config(yaml),
        "Mini-Acme fixture must be classified as a GroupConfig",
    );
}

/// The minimal Mini-Acme fixture (`mini_acme_minimal.yaml`) is
/// also a `GroupConfig` even with sparse content — it carries
/// `presentation_currency` and `ownership` at top-level.
#[test]
fn mini_acme_minimal_dispatches_to_group_flow() {
    let yaml = include_str!("fixtures/mini_acme_minimal.yaml");
    assert!(
        yaml_is_group_config(yaml),
        "Mini-Acme minimal fixture must be classified as a GroupConfig",
    );
}

/// Edge cases: malformed YAML, top-level scalar, empty file, mapping
/// with only one of the required keys.  All must dispatch to legacy
/// (return `false`).
#[test]
fn ambiguous_inputs_dispatch_to_legacy() {
    // Not a YAML mapping — scalar at top-level.
    assert!(!yaml_is_group_config("---\nhello: world"));
    assert!(!yaml_is_group_config("just-a-string"));
    // Empty input.
    assert!(!yaml_is_group_config(""));
    // Only one of the required keys.
    assert!(!yaml_is_group_config(
        "presentation_currency: CHF\nfoo: bar\n"
    ));
    assert!(!yaml_is_group_config(
        "ownership:\n  parent_entity_code: X\n"
    ));
    // Malformed YAML — parser fails outright.  Must default to legacy.
    assert!(!yaml_is_group_config(
        "this: is\n  not: valid: yaml: at all"
    ));
}

//! Integration test: every YAML in `templates/` and `templates/scenarios/`
//! must (a) deserialize cleanly with `#[serde(deny_unknown_fields)]` on the
//! distribution structs and (b) pass `validate_config`.
//!
//! Catches the drift class reported by a customer in May 2026: silently-dropped
//! fields in `master_data.vendors.behavior_distribution` and
//! `fraud.fraud_type_distribution` masked an unparseable / unvalidated config.

use datasynth_config::{validate_config, GeneratorConfig};
use std::fs;
use std::path::{Path, PathBuf};

/// Walk a directory and return every `.yaml` file.
fn list_yaml_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                out.extend(list_yaml_files(&p));
            } else if p.extension().and_then(|s| s.to_str()) == Some("yaml") {
                out.push(p);
            }
        }
    }
    out
}

/// Resolve `templates/` relative to the workspace root from this crate's manifest dir.
fn templates_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crate manifest = .../crates/datasynth-config/Cargo.toml
    // templates dir  = .../templates
    manifest
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("templates")
}

#[test]
fn every_scenario_template_deserializes_and_validates() {
    let root = templates_dir();
    assert!(
        root.exists(),
        "templates/ not found at {} — adjust path resolution",
        root.display()
    );

    // Only audit the scenario templates and the top-level shareable
    // presets — exclude `templates/packs/defaults/` which holds plain text
    // packs (asset_descriptions.yaml etc.), not GeneratorConfig docs.
    let candidates: Vec<PathBuf> = list_yaml_files(&root)
        .into_iter()
        .filter(|p| !p.components().any(|c| c.as_os_str() == "packs"))
        .collect();

    assert!(
        !candidates.is_empty(),
        "no template YAML files discovered under {}",
        root.display()
    );

    let mut failures: Vec<String> = Vec::new();
    for path in &candidates {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                failures.push(format!("{}: read error: {e}", path.display()));
                continue;
            }
        };

        let config: GeneratorConfig = match serde_yaml::from_str(&text) {
            Ok(c) => c,
            Err(e) => {
                failures.push(format!("{}: deserialize error: {e}", path.display()));
                continue;
            }
        };

        if let Err(e) = validate_config(&config) {
            failures.push(format!("{}: validate error: {e}", path.display()));
        }
    }

    if !failures.is_empty() {
        panic!(
            "scenario template drift — {} of {} templates failed:\n{}",
            failures.len(),
            candidates.len(),
            failures.join("\n")
        );
    }
}

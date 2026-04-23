//! Regression test — manifest generated from Mini-Nestlé fixture must match
//! the committed golden JSON. Run `cargo test -p datasynth-group --test
//! manifest_golden regenerate_golden -- --ignored` to update the golden
//! when a change is intentional.

use datasynth_group::{build_manifest, GroupConfig};

const FIXTURE_PATH: &str = "tests/fixtures/mini_nestle.yaml";
const GOLDEN_PATH: &str = "tests/golden/mini_nestle_manifest.json";

#[test]
fn test_mini_nestle_manifest_matches_golden() {
    let yaml = std::fs::read_to_string(FIXTURE_PATH).expect("fixture readable");
    let cfg: GroupConfig = serde_yaml::from_str(&yaml).expect("fixture parses");
    let manifest = build_manifest(&cfg).expect("manifest builds");
    let actual = serde_json::to_string_pretty(&manifest).expect("manifest serializes");

    let expected = std::fs::read_to_string(GOLDEN_PATH).unwrap_or_else(|e| {
        panic!(
            "golden file missing at {GOLDEN_PATH}: {e}. \
             Run `cargo test -p datasynth-group --test manifest_golden regenerate_golden -- --ignored` \
             to create it."
        )
    });

    if actual.trim_end() != expected.trim_end() {
        // Produce a useful diff for CI logs: write the actual output to a
        // sibling file so developers can `diff` locally.
        let actual_out = format!("{GOLDEN_PATH}.actual");
        let _ = std::fs::write(&actual_out, &actual);

        panic!(
            "golden mismatch — actual written to {actual_out}\n\
             If this change was intentional, run:\n\
             cargo test -p datasynth-group --test manifest_golden regenerate_golden -- --ignored"
        );
    }
}

#[test]
#[ignore = "run with --ignored to regenerate the manifest golden"]
fn regenerate_golden() {
    let yaml = std::fs::read_to_string(FIXTURE_PATH).expect("fixture readable");
    let cfg: GroupConfig = serde_yaml::from_str(&yaml).expect("fixture parses");
    let manifest = build_manifest(&cfg).expect("manifest builds");
    let json = serde_json::to_string_pretty(&manifest).expect("manifest serializes");

    // Ensure golden dir exists.
    let golden_dir = std::path::Path::new(GOLDEN_PATH).parent().unwrap();
    std::fs::create_dir_all(golden_dir).expect("golden dir");

    std::fs::write(GOLDEN_PATH, &json).expect("golden writable");

    // Also verify it reads back identically.
    let read_back = std::fs::read_to_string(GOLDEN_PATH).unwrap();
    assert_eq!(
        read_back, json,
        "round-trip mismatch when re-reading just-written golden"
    );
    println!("Regenerated golden at {GOLDEN_PATH} ({} bytes)", json.len());
}

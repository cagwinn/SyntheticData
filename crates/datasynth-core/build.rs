//! Build-time validator for `templates/defaults.yaml`.
//!
//! The bundled defaults file is `include_str!`-ed into the crate at
//! compile time and drives `DefaultTemplateProvider::new()` (via
//! `bundled()`). A typo or missing required key would manifest as a
//! runtime panic in downstream user code with a confusing message, so
//! validate structure at build time instead.
//!
//! Validates:
//! - The file is readable and valid YAML.
//! - The three universal-fallback entries exist and are non-empty:
//!   * `person_names.cultures.us` with `male_first_names`,
//!     `female_first_names`, `last_names` sequences.
//!   * `vendor_names.categories.manufacturing` sequence.
//!   * `customer_names.industries.retail` sequence.
//!
//! These three entries are the fallbacks that `DefaultTemplateProvider`
//! consults when a requested culture/category/industry isn't mirrored in
//! the YAML — removing them would regress any run that asks for an
//! unlisted value.

use std::path::PathBuf;

fn main() {
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    let path = PathBuf::from(manifest_dir).join("templates/defaults.yaml");
    println!("cargo:rerun-if-changed={}", path.display());

    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read bundled defaults at {}: {}",
            path.display(),
            e
        );
    });

    let value: serde_yaml::Value = serde_yaml::from_str(&text)
        .unwrap_or_else(|e| panic!("templates/defaults.yaml is not valid YAML: {e}"));

    require_nonempty_sequence(
        &value,
        &["person_names", "cultures", "us", "male_first_names"],
    );
    require_nonempty_sequence(
        &value,
        &["person_names", "cultures", "us", "female_first_names"],
    );
    require_nonempty_sequence(&value, &["person_names", "cultures", "us", "last_names"]);
    require_nonempty_sequence(&value, &["vendor_names", "categories", "manufacturing"]);
    require_nonempty_sequence(&value, &["customer_names", "industries", "retail"]);
}

fn require_nonempty_sequence(root: &serde_yaml::Value, path: &[&str]) {
    let mut cur = root;
    for segment in path {
        cur = cur.get(*segment).unwrap_or_else(|| {
            panic!(
                "templates/defaults.yaml missing required path: {}",
                path.join(".")
            );
        });
    }
    let seq = cur.as_sequence().unwrap_or_else(|| {
        panic!(
            "templates/defaults.yaml path {} must be a YAML sequence",
            path.join(".")
        );
    });
    if seq.is_empty() {
        panic!(
            "templates/defaults.yaml path {} must have at least one entry",
            path.join(".")
        );
    }
}

# YAML-as-Source-of-Truth migration (v4.0.0 → v4.1.0+)

## Current state (v4.0.0)

Default template pools — person names per culture, vendor categories,
customer industries, material descriptions, asset descriptions, line
texts, header templates, bank names, audit findings, department names —
are hardcoded as `const &[&str]` arrays in
`crates/datasynth-core/src/templates/*.rs` and in several generator
crates.  These arrays power `DefaultTemplateProvider::new()`.

Users who want to override them today use the v3.2.0 template-pack
feature: export via `datasynth-data templates export`, edit YAML, pass
`--templates <path>` on `generate` (or `templates.path` in config).

## Why the migration is gated (not done in v4.0)

Moving the canonical source from Rust arrays to YAML while preserving
byte-identity for the same seed requires:

1. Exporting every embedded array to YAML in stable order.
2. A `build.rs` that validates the YAML round-trips and embeds it via
   `include_bytes!`.
3. Rewriting `DefaultTemplateProvider::new()` to parse the bundled YAML
   at startup.
4. A regression test that iterates every culture/industry/category and
   confirms the array-vs-YAML output streams are byte-identical.

That's ~1-2 weeks of careful work and carries non-trivial regression
risk. v4.0.0 deliberately ships the foundation without forcing the
migration.

## v4.0.0 foundation

- `TemplateLoader::load_from_yaml_str(yaml: &str) -> Result<
  TemplateData, TemplateError>` — parses a compile-time-bundled YAML
  string. Pair with `include_str!("…")`.
- `TemplateLoader::save_to_yaml(&data, path)` (already v3.5.0) lets
  maintainers export the current embedded state once the Rust arrays
  have been copied into a `TemplateData` literal.

## How to migrate once the YAML is exported

```rust
// In `crates/datasynth-core/src/templates/provider.rs`
impl DefaultTemplateProvider {
    pub fn new() -> Self {
        const BUNDLED: &str = include_str!("../../templates/defaults.yaml");
        let data = TemplateLoader::load_from_yaml_str(BUNDLED)
            .expect("bundled defaults YAML is valid");
        Self::from_template_data(data)
    }
}
```

Paired with a `build.rs` that asserts the YAML parses, this gives:

- Single source of truth (YAML committed in-repo).
- Byte-identical output for seeded runs across v4.0 → v4.1 (because
  the YAML content equals the former array content).
- User overrides still work via `templates.path` merging.

## Owner + timeline

Targeted for v4.1.0. A provisional YAML export (produced by
`templates export --output crates/datasynth-core/templates/` on a v4.0
build) is already sufficient source material; the missing steps are
the build.rs wiring and the regression test.

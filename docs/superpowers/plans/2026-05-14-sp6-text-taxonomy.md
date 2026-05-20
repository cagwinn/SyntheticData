# SP6 Text Taxonomy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace SP4.4's verbatim source-keyed text extraction with a PII-safe placeholder grammar and `(source × account-class)`-keyed conditional template pools.

**Architecture:** A new isolated `text_taxonomy` module in `datasynth-core` holds the placeholder grammar (tokenize / fill / residual-PII scan) and the `TextTaxonomyPrior` data types. The extractor (`datasynth-fingerprint`) two-phase-tokenizes corpus text (automated structural + curated denylist) and groups line text by `(source, ISO-21378-class)`. The generator (`datasynth-generators`) resolves each line's account class and samples from the matching pool via a lookup cascade, filling PII placeholders from the run's master data. Old code is removed only after every consumer is rewired, so the build stays green at every task boundary.

**Tech Stack:** Rust, `serde`/`serde_yaml`, `regex` (no lookaround support — important), `rand` 0.10 (`RngExt`, `random_range`), `zip` for `.dsf` archives.

**Spec:** `docs/superpowers/specs/2026-05-14-sp6-text-taxonomy-design.md`

**Branch:** `sp6-text-taxonomy` (already created)

---

## File Structure

| File | Responsibility | Tasks |
|------|----------------|-------|
| `crates/datasynth-core/src/distributions/text_taxonomy.rs` (new) | Placeholder grammar (tokenize/fill/scan), `TextTaxonomyPrior` data types, `PlaceholderResolver` trait, `SyntheticExampleResolver` | T1–T4 |
| `crates/datasynth-core/src/distributions/mod.rs` (modify) | Register + re-export the new module | T1 |
| `crates/datasynth-core/src/distributions/behavioral_priors.rs` (modify) | Add `text_taxonomy` field to `BehavioralPriors`; later remove `TextTemplate*` | T1, T12 |
| `crates/datasynth-fingerprint/src/extraction/pii_denylist.rs` (new) | `PiiDenylist` — load from private TSV, apply Phase B | T5 |
| `crates/datasynth-fingerprint/src/extraction/text_extractor.rs` (rewrite) | `extract_text_taxonomy` — two-phase tokenize, `(source,class)` grouping, inline scan, `synthetic_example` | T6, T12 |
| `crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs` (modify) | `aggregate_text_taxonomy` across clients | T7, T8, T12 |
| `crates/datasynth-fingerprint/src/extraction/mod.rs` + `models/behavioral.rs` (modify) | Re-export `pii_denylist`; drop `TextTemplate*` re-exports | T8, T12 |
| `crates/datasynth-generators/src/priors_loader.rs` (modify) | Load `text_taxonomy`; lookup cascade; `sample_line_template(src,class)`, `sample_coa_description(acct)` | T9, T12 |
| `crates/datasynth-generators/src/je_generator.rs` (modify) | Resolve account class per line; `MasterDataResolver`; wire line/header sampling | T10, T12 |
| `crates/datasynth-generators/src/coa_generator.rs` (modify) | Fill CoA description templates once per account | T11 |
| `crates/datasynth-runtime/tests/bundle_pii_audit.rs` (new) | CI gate: residual-PII scan over every committed bundle | T13 |
| `crates/datasynth-runtime/tests/sp6_text_taxonomy_smoke.rs` (new) | Integration smoke: no `{…}` literals, no residual PII, line text populated | T14 |
| `scripts/regenerate-industry-priors.sh` (modify) | `--pii-denylist` arg + build-time audit gate | T15 |

**Green-build discipline:** T1 adds `text_taxonomy` *alongside* `text_templates`. Tasks T6–T11 add new functions/methods alongside the old ones. T12 removes the old code only after every consumer uses the new path. The workspace compiles after every task.

---

## Task 1: Core data types, traits, and the `BehavioralPriors` field

**Files:**
- Create: `crates/datasynth-core/src/distributions/text_taxonomy.rs`
- Modify: `crates/datasynth-core/src/distributions/mod.rs`
- Modify: `crates/datasynth-core/src/distributions/behavioral_priors.rs` (struct `BehavioralPriors`, ~line 151)

- [ ] **Step 1: Write the failing test**

Create `crates/datasynth-core/src/distributions/text_taxonomy.rs` with only the test module to start:

```rust
//! SP6 — corpus text taxonomy: PII-safe placeholder grammar + conditional
//! template pools keyed on (source, account-class).
//!
//! Replaces SP4.4's verbatim source-keyed `TextTemplatePrior`. Generated text
//! is synthetic-by-construction: tokenized real templates whose PII spans are
//! fillable placeholders. Line text is conditioned on (source, account-class);
//! header text is source-keyed; CoA descriptions are per-account templates
//! filled once per run.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pii_placeholder_kind_token_roundtrip() {
        for kind in [
            PiiPlaceholderKind::Patient,
            PiiPlaceholderKind::Person,
            PiiPlaceholderKind::Company,
            PiiPlaceholderKind::Street,
        ] {
            assert_eq!(PiiPlaceholderKind::from_token(kind.token()), Some(kind));
        }
        assert_eq!(PiiPlaceholderKind::from_token("{year}"), None);
        assert_eq!(PiiPlaceholderKind::from_token("{unknown}"), None);
    }

    #[test]
    fn line_key_format() {
        assert_eq!(TextTaxonomyPrior::line_key("KR", "A.B"), "KR|A.B");
        assert_eq!(
            TextTaxonomyPrior::line_key("RE", TextTaxonomyPrior::UNKNOWN_CLASS),
            "RE|_unknown_"
        );
    }

    #[test]
    fn synthetic_example_resolver_emits_obvious_fakes() {
        let mut r = SyntheticExampleResolver;
        let mut rng = rand::rng();
        for kind in [
            PiiPlaceholderKind::Patient,
            PiiPlaceholderKind::Person,
            PiiPlaceholderKind::Company,
            PiiPlaceholderKind::Street,
        ] {
            let v = r.resolve(kind, &mut rng);
            assert!(v.starts_with("Example"), "expected obvious fake, got {v}");
        }
    }

    #[test]
    fn prior_serde_roundtrip() {
        let mut prior = TextTaxonomyPrior::default();
        prior.line_pools.insert(
            TextTaxonomyPrior::line_key("KR", "A.B"),
            TemplatePool {
                templates: vec![TemplateEntry {
                    template: "Rechnung {company}".to_string(),
                    probability: 1.0,
                    synthetic_example: "Rechnung Example GmbH".to_string(),
                }],
                n: 42,
            },
        );
        prior.meta.class_tier = "iso21378_l2".to_string();
        let yaml = serde_yaml::to_string(&prior).expect("serialize");
        let back: TextTaxonomyPrior = serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(prior, back);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p datasynth-core --lib text_taxonomy 2>&1 | tail -15`
Expected: FAIL — `cannot find type PiiPlaceholderKind` etc. (module not yet registered / types not defined).

- [ ] **Step 3: Implement the types**

Insert this above the `#[cfg(test)]` block in `text_taxonomy.rs`:

```rust
/// A PII-placeholder kind the generator must resolve to a concrete value.
/// Structural placeholders (`{year}`, `{quarter}`, `{month}`, `{date}`,
/// `{digits}`) are NOT in this enum — `PlaceholderGrammar::fill` handles those.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PiiPlaceholderKind {
    Patient,
    Person,
    Company,
    Street,
}

impl PiiPlaceholderKind {
    /// The placeholder token as it appears in a template string.
    pub fn token(self) -> &'static str {
        match self {
            PiiPlaceholderKind::Patient => "{patient}",
            PiiPlaceholderKind::Person => "{person}",
            PiiPlaceholderKind::Company => "{company}",
            PiiPlaceholderKind::Street => "{street}",
        }
    }

    /// Parse a placeholder token to its kind. `None` for structural or unknown.
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "{patient}" => Some(PiiPlaceholderKind::Patient),
            "{person}" => Some(PiiPlaceholderKind::Person),
            "{company}" => Some(PiiPlaceholderKind::Company),
            "{street}" => Some(PiiPlaceholderKind::Street),
            _ => None,
        }
    }
}

/// Resolves a PII-placeholder kind to a concrete value. Implemented by the
/// generator (wired to master data) and by `SyntheticExampleResolver` (used at
/// extraction time, where master data does not exist).
pub trait PlaceholderResolver {
    fn resolve(&mut self, kind: PiiPlaceholderKind, rng: &mut dyn rand::Rng) -> String;
}

/// A built-in resolver emitting obviously-synthetic tokens. Used to produce
/// `TemplateEntry::synthetic_example` at extraction time and in tests.
pub struct SyntheticExampleResolver;

impl PlaceholderResolver for SyntheticExampleResolver {
    fn resolve(&mut self, kind: PiiPlaceholderKind, _rng: &mut dyn rand::Rng) -> String {
        match kind {
            PiiPlaceholderKind::Patient => "Example Patient".to_string(),
            PiiPlaceholderKind::Person => "Example Person".to_string(),
            PiiPlaceholderKind::Company => "Example GmbH".to_string(),
            PiiPlaceholderKind::Street => "Example Street 1".to_string(),
        }
    }
}

/// One residual-PII scan hit.
#[derive(Debug, Clone, PartialEq)]
pub struct PiiHit {
    /// Static label of the pattern that matched (e.g. `"patient_record"`).
    pub pattern: &'static str,
    /// The substring that matched.
    pub matched: String,
}

/// A single PII-safe text template.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TemplateEntry {
    /// Tokenized, PII-safe template string.
    pub template: String,
    /// Probability mass within the owning pool (renormalised after filtering).
    pub probability: f64,
    /// The template run through `fill` once at extraction time with a
    /// fixed-seed RNG and `SyntheticExampleResolver` — a debug/audit example
    /// carrying ZERO corpus content. Replaces SP4.4's verbatim `example` field.
    pub synthetic_example: String,
}

/// A weighted pool of templates for one `(source, class)` or `source` key.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TemplatePool {
    pub templates: Vec<TemplateEntry>,
    /// Total observations underpinning the pool (pre-truncation).
    pub n: usize,
}

/// Extraction metadata for a `TextTaxonomyPrior`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TaxonomyMeta {
    pub min_occurrences: usize,
    pub max_templates_per_pool: usize,
    /// Class-granularity tier used for `line_pools` keys (e.g. `"iso21378_l2"`).
    pub class_tier: String,
    pub n_client_inputs: usize,
}

/// SP6 — corpus text taxonomy prior. Replaces `TextTemplatePrior`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TextTaxonomyPrior {
    /// Line text keyed on the flattened string `"SOURCE|CLASS"`. `CLASS` is the
    /// ISO 21378 Level-2 account class; lines whose account has no resolvable
    /// class are grouped under `"SOURCE|_unknown_"`.
    pub line_pools: BTreeMap<String, TemplatePool>,
    /// Header text keyed on source only (a JE header has no single account).
    pub header_pools: BTreeMap<String, TemplatePool>,
    /// CoA description templates keyed on account number — one per account.
    pub coa_pools: BTreeMap<String, TemplateEntry>,
    /// Extraction metadata.
    pub meta: TaxonomyMeta,
}

impl TextTaxonomyPrior {
    /// Sentinel class component used when a line's account has no resolvable
    /// ISO 21378 class.
    pub const UNKNOWN_CLASS: &'static str = "_unknown_";

    /// Build the flattened `"SOURCE|CLASS"` key used by `line_pools`.
    pub fn line_key(source: &str, account_class: &str) -> String {
        format!("{source}|{account_class}")
    }
}
```

- [ ] **Step 4: Register the module**

In `crates/datasynth-core/src/distributions/mod.rs`, find the block of `pub mod …;` declarations and the block of `pub use …;` re-exports (match the existing style — the file already has e.g. `pub mod behavioral_priors;`). Add, in alphabetical position:

```rust
pub mod text_taxonomy;
```

And add a re-export line next to the other `pub use` lines:

```rust
pub use text_taxonomy::{
    PiiHit, PiiPlaceholderKind, PlaceholderResolver, SyntheticExampleResolver, TaxonomyMeta,
    TemplateEntry, TemplatePool, TextTaxonomyPrior,
};
```

- [ ] **Step 5: Add the `text_taxonomy` field to `BehavioralPriors`**

In `crates/datasynth-core/src/distributions/behavioral_priors.rs`, find the `pub struct BehavioralPriors` definition (near line 151). Find the existing `pub text_templates: Option<TextTemplatePrior>,` field. Immediately **after** it, add — copying the exact `#[serde(default, …)]` attribute pattern used on the neighbouring `Option` fields (e.g. `tb_anchor`):

```rust
    /// SP6 — corpus text taxonomy. Replaces `text_templates`. When `Some`,
    /// the generator samples line text keyed on `(source, account-class)`,
    /// header text on `source`, and CoA descriptions per account — all PII-safe
    /// templates filled at generation time. `None` for pre-SP6 bundles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_taxonomy: Option<TextTaxonomyPrior>,
```

Add `TextTaxonomyPrior` to the `use` list at the top of `behavioral_priors.rs` if the file `use`s its own module types explicitly — otherwise it is in scope already (same module). Then update **every** `BehavioralPriors { … }` struct literal **across the entire workspace** to add `text_taxonomy: None,`. `BehavioralPriors` has exhaustive fields (no `..Default::default()` tail), so a literal in *any* crate that omits the new field will not compile. Run `grep -rn "BehavioralPriors {" crates/ --include=*.rs` to find them all — known sites beyond `behavioral_priors.rs`'s own test blocks: `industry_aggregator.rs` (`aggregate_industry`, production), `behavioral_extractor.rs` (`extract_behavioral_priors`, production), `priors_loader.rs` (test blocks), `sp3_priors_smoke.rs` (8 test literals). Each gets a sibling `text_taxonomy: None,` immediately after its `text_templates` field/block.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p datasynth-core --lib text_taxonomy 2>&1 | tail -15`
Expected: PASS — 4 tests.

Run: `cargo build --workspace --tests 2>&1 | tail -5`
Expected: `Finished` — no errors. The new `BehavioralPriors` field is additive, but every `BehavioralPriors` literal across the workspace must carry `text_taxonomy: None,` (Step 5) for the whole workspace to compile.

- [ ] **Step 7: Commit**

```bash
git add crates/datasynth-core/src/distributions/text_taxonomy.rs \
        crates/datasynth-core/src/distributions/mod.rs \
        crates/datasynth-core/src/distributions/behavioral_priors.rs
git commit -m "feat(sp6): text-taxonomy core types + BehavioralPriors.text_taxonomy field

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: `PlaceholderGrammar::residual_pii_scan`

**Files:**
- Modify: `crates/datasynth-core/src/distributions/text_taxonomy.rs`
- Modify: `crates/datasynth-core/Cargo.toml` (ensure `regex` dependency)

**Note:** Rust's `regex` crate has **no lookaround support** (`(?=…)`, `(?<=…)`). Every pattern below is written without lookarounds.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `text_taxonomy.rs`:

```rust
    #[test]
    fn residual_scan_flags_patient_record() {
        let hits = PlaceholderGrammar::residual_pii_scan("*Gambon,Laurin G:01.02.03 E:04.05.06");
        assert!(
            hits.iter().any(|h| h.pattern == "patient_record"),
            "expected patient_record hit, got {hits:?}"
        );
    }

    #[test]
    fn residual_scan_flags_person_shapes() {
        // star record
        assert!(PlaceholderGrammar::residual_pii_scan("*Mueller,Hans")
            .iter()
            .any(|h| h.pattern == "person_star"));
        // initial + surname
        assert!(PlaceholderGrammar::residual_pii_scan("Forschung U. Frey")
            .iter()
            .any(|h| h.pattern == "initial_surname"));
        // title
        assert!(PlaceholderGrammar::residual_pii_scan("Kontokorrent Prof. Dr. M. Buess")
            .iter()
            .any(|h| h.pattern == "title"));
    }

    #[test]
    fn residual_scan_passes_clean_templates() {
        for clean in [
            "Rechnung {company}",
            "Mieten {month}.{year}",
            "ARIBA_ASN",
            "Darlehen {person}",
            "*{patient} G:{date} E:{date} A:{date}",
            "Umbuchung Anlage",
        ] {
            assert!(
                PlaceholderGrammar::residual_pii_scan(clean).is_empty(),
                "false positive on clean template: {clean:?}"
            );
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p datasynth-core --lib text_taxonomy::tests::residual 2>&1 | tail -15`
Expected: FAIL — `cannot find … PlaceholderGrammar`.

- [ ] **Step 3: Ensure `regex` is a dependency**

Check `crates/datasynth-core/Cargo.toml` for a `regex` line under `[dependencies]`. If absent, add:

```toml
regex = "1"
```

Run `cargo build -p datasynth-core 2>&1 | tail -3` to confirm it resolves.

- [ ] **Step 4: Implement `PlaceholderGrammar` + `residual_pii_scan`**

Add to `text_taxonomy.rs`, above the `#[cfg(test)]` block. Add `use std::sync::LazyLock;` and `use regex::Regex;` to the file's imports.

```rust
/// Stateless tokenize / fill / scan engine. No dependency on the generator or
/// fingerprint crates — locale and master-data wiring arrive via a
/// `PlaceholderResolver` at fill time.
pub struct PlaceholderGrammar;

// --- residual-PII scan patterns (no lookaround — `regex` crate limitation) ---

/// Patient record marker: `G:dd.dd.dd`. Presence implies an un-stripped name.
static RE_PATIENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"G:\s*\d{2}\.\d{2}\.\d{2}").unwrap());
/// `*Lastname,Firstname` star record, anchored at start.
static RE_PERSON_STAR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\*[A-ZÄÖÜ][\w\u{00C0}-\u{017F}.'\-]+\s*,\s*[A-ZÄÖÜ]").unwrap()
});
/// Honorific / title followed by a name.
static RE_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(Prof|Dr|Dipl|Pfr|Pfarrer|Herr|Frau|Hr|Fr|med|iur|lic)\.\s").unwrap()
});
/// `Initial. Surname` (e.g. `U. Frey`).
static RE_INITIAL_SURNAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-ZÄÖÜ]\.\s*[A-ZÄÖÜ][a-zäöüß]{2,}\b").unwrap());
/// `Surname Initial.` (e.g. `Frey U.`). No trailing lookahead — a scanner may
/// over-flag; that is the safe direction.
static RE_SURNAME_INITIAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-ZÄÖÜ][a-zäöüß]{2,}\s+[A-ZÄÖÜ]\.").unwrap());

impl PlaceholderGrammar {
    /// Scan a string for residual PII patterns. Returns one hit per pattern
    /// that matches; an empty result means the string is clean. Patterns that
    /// detect PII-bearing *shapes* — used as a hard gate at extraction and in
    /// CI. Templates whose PII spans are already placeholders (`{person}`,
    /// `{patient}`, …) do not match these patterns.
    pub fn residual_pii_scan(s: &str) -> Vec<PiiHit> {
        let mut hits = Vec::new();
        let checks: &[(&'static str, &Regex)] = &[
            ("patient_record", &RE_PATIENT),
            ("person_star", &RE_PERSON_STAR),
            ("title", &RE_TITLE),
            ("initial_surname", &RE_INITIAL_SURNAME),
            ("surname_initial", &RE_SURNAME_INITIAL),
        ];
        for (label, re) in checks {
            if let Some(m) = re.find(s) {
                hits.push(PiiHit {
                    pattern: label,
                    matched: m.as_str().to_string(),
                });
            }
        }
        hits
    }
}
```

**Verify by hand:** `"*{patient} G:{date} E:{date} A:{date}"` must NOT match `RE_PATIENT` — it matches `G:\s*\d{2}…` only on literal digits, and `{date}` has none. `"Darlehen {person}"` must NOT match `RE_INITIAL_SURNAME` — `{person}` has no `Initial. Surname` shape. The clean-templates test (Step 1) pins this.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p datasynth-core --lib text_taxonomy 2>&1 | tail -15`
Expected: PASS — all 7 tests (4 from T1 + 3 new).

- [ ] **Step 6: Commit**

```bash
git add crates/datasynth-core/src/distributions/text_taxonomy.rs crates/datasynth-core/Cargo.toml
git commit -m "feat(sp6): PlaceholderGrammar::residual_pii_scan

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: `PlaceholderGrammar::tokenize` (Phase A — automated structural)

**Files:**
- Modify: `crates/datasynth-core/src/distributions/text_taxonomy.rs`

`tokenize` performs **Phase A only** — automated structural placeholder-ization. Phase B (curated denylist) is applied separately by the extractor (Task 6), because the denylist type lives in `datasynth-fingerprint` and `datasynth-core` must not depend on it. Signature is `tokenize(s: &str) -> String`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:

```rust
    #[test]
    fn tokenize_patient_record_strips_name_even_with_g_in_it() {
        // The name "Gambon" contains a G — the strip must consume it. A naive
        // [^G]*? class cannot, and would leak the name. This is the bug the
        // first-pass cleaning sweep found.
        assert_eq!(
            PlaceholderGrammar::tokenize("*Gambon,Laurin G:01.02.03 E:04.05.06 A:07.08.09"),
            "*{patient} G:{date} E:{date} A:{date}"
        );
        assert_eq!(
            PlaceholderGrammar::tokenize("*Rykart,Frank G G:11.12.13"),
            "*{patient} G:{date}"
        );
    }

    #[test]
    fn tokenize_person_star_record() {
        assert_eq!(
            PlaceholderGrammar::tokenize("*Mueller,Hans"),
            "*{person}"
        );
    }

    #[test]
    fn tokenize_street_address() {
        assert_eq!(
            PlaceholderGrammar::tokenize("LUKB Mietzinskaution Roentgenpraxis, Spitalstrasse 5"),
            "LUKB Mietzinskaution Roentgenpraxis, {street}"
        );
    }

    #[test]
    fn tokenize_structural_temporal() {
        assert_eq!(PlaceholderGrammar::tokenize("Mieten 04.2021"), "Mieten 04.{year}");
        assert_eq!(PlaceholderGrammar::tokenize("Sales Accrual Q1"), "Sales Accrual {quarter}");
        assert_eq!(PlaceholderGrammar::tokenize("January accrual"), "{month} accrual");
        assert_eq!(PlaceholderGrammar::tokenize("INV 1234567"), "INV {digits}");
        assert_eq!(PlaceholderGrammar::tokenize("GL 470"), "GL 470"); // short run kept
    }

    #[test]
    fn tokenize_fixed_vocab_unchanged() {
        assert_eq!(PlaceholderGrammar::tokenize("ARIBA_ASN"), "ARIBA_ASN");
        assert_eq!(
            PlaceholderGrammar::tokenize("CH Post: KUREPO Intercomp"),
            "CH Post: KUREPO Intercomp"
        );
    }

    #[test]
    fn tokenize_then_scan_is_clean() {
        // Every Phase-A-tokenized string with structural PII must scan clean.
        for raw in [
            "*Gambon,Laurin G:01.02.03 E:04.05.06 A:07.08.09",
            "*Mueller,Hans",
            "LUKB Spitalstrasse 5",
        ] {
            let tok = PlaceholderGrammar::tokenize(raw);
            assert!(
                PlaceholderGrammar::residual_pii_scan(&tok).is_empty(),
                "tokenize left residual PII: {raw:?} -> {tok:?}"
            );
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p datasynth-core --lib text_taxonomy::tests::tokenize 2>&1 | tail -15`
Expected: FAIL — `no function … tokenize`.

- [ ] **Step 3: Implement `tokenize`**

Add these statics near the other `LazyLock<Regex>` statics, and the `tokenize` method inside `impl PlaceholderGrammar`:

```rust
/// `dd.mm.yy` date triplet inside a patient `G:`/`E:`/`A:` record.
static RE_GEA_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([GEA]):\s*\d{2}\.\d{2}\.\d{2}").unwrap());
/// Street address: a capitalised word ending in a street-type suffix, then a
/// number. Case-insensitive on the suffix.
static RE_STREET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[A-ZÄÖÜ][\w\u{00C0}-\u{017F}.\-]*(?:str\.|strasse|gasse|weg|platz)\s*\d+[A-Za-z]?\b").unwrap()
});
/// 4-digit year 19xx / 20xx not embedded in a longer digit run.
static RE_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:19|20)\d{2}\b").unwrap());
/// Quarter marker Q1–Q4 (case-insensitive), not followed by another digit.
static RE_QUARTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bQ[1-4]\b").unwrap());
/// Run of >=4 digits.
static RE_DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{4,}").unwrap());

/// Month names (English + German, full + common abbreviations), longest-first.
const MONTH_NAMES: &[&str] = &[
    "September", "Februar", "Dezember", "November", "February", "December",
    "January", "October", "Januar", "Oktober", "August", "März", "Maerz",
    "April", "March", "Juni", "Juli", "June", "July", "Mai", "May",
    "Jan", "Feb", "Mar", "Apr", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

impl PlaceholderGrammar {
    /// Phase A — automated structural placeholder-ization. Raw corpus string to
    /// a PII-safe template using only deterministic structural rules. Phase B
    /// (curated denylist for fuzzy proper nouns) is applied by the extractor.
    ///
    /// Rules, in order:
    /// 1. Patient `G:`-record: strip everything up to the first `G:dd.dd.dd`
    ///    marker (the name region, regardless of any `G` in the name), prepend
    ///    `*{patient} `, then replace each `G:/E:/A:dd.dd.dd` with `X:{date}`.
    /// 2. `*Lastname,Firstname` star record: replace the matched span with
    ///    `*{person}`.
    /// 3. Street address: replace with `{street}`.
    /// 4. 4-digit years -> `{year}`; `Q1`-`Q4` -> `{quarter}`; month names ->
    ///    `{month}`; runs of >=4 digits -> `{digits}`.
    pub fn tokenize(s: &str) -> String {
        let t = s.trim();
        if t.is_empty() {
            return String::new();
        }

        // Rule 1 — patient record. find() the date marker, slice off the name.
        if let Some(m) = RE_PATIENT.find(t) {
            let from_marker = &t[m.start()..];
            let dated = RE_GEA_DATE.replace_all(from_marker, "$1:{date}");
            return format!("*{{patient}} {dated}").trim().to_string();
        }

        // Rule 2 — star person record.
        if let Some(m) = RE_PERSON_STAR.find(t) {
            // The star record covers "*Lastname,F" — replace the whole string
            // when the match anchors at start and the remainder is name-shaped.
            // Conservative: replace only the matched span, keep any suffix.
            let mut out = String::with_capacity(t.len());
            out.push_str("*{person}");
            out.push_str(&t[m.end()..]);
            // A trailing fragment of the firstname may remain (e.g. "ans" from
            // "*Mueller,Hans" if the class stopped early) — RE_PERSON_STAR's
            // last class is a single `[A-ZÄÖÜ]`, so the remainder begins with
            // lowercase name chars. Drop a leading run of name chars.
            let cleaned = trim_leading_name_fragment(&out);
            return cleaned;
        }

        // Rule 3 — street address.
        let t = RE_STREET.replace_all(t, "{street}").into_owned();

        // Rule 4 — structural / temporal.
        let t = RE_YEAR.replace_all(&t, "{year}").into_owned();
        let t = RE_QUARTER.replace_all(&t, "{quarter}").into_owned();
        let t = replace_months(&t);
        let t = RE_DIGITS.replace_all(&t, "{digits}").into_owned();
        t
    }
}

/// Replace month names with `{month}` at word boundaries (longest-first so
/// "September" wins over a hypothetical "Sep" prefix).
fn replace_months(s: &str) -> String {
    let mut result = s.to_string();
    for name in MONTH_NAMES {
        // Word-boundary replace, case-sensitive (month names are capitalised
        // in this corpus). Build a fresh string to avoid re-matching `{month}`.
        let mut out = String::with_capacity(result.len());
        let bytes = result.as_bytes();
        let nlen = name.len();
        let mut i = 0;
        while i < result.len() {
            if result[i..].starts_with(name) {
                let prev_alpha = i > 0
                    && result[..i]
                        .chars()
                        .next_back()
                        .map(|c| c.is_alphabetic())
                        .unwrap_or(false);
                let next_alpha = result[i + nlen..]
                    .chars()
                    .next()
                    .map(|c| c.is_alphabetic())
                    .unwrap_or(false);
                if !prev_alpha && !next_alpha {
                    out.push_str("{month}");
                    i += nlen;
                    continue;
                }
            }
            // push one char
            let ch_len = result[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            out.push_str(&result[i..i + ch_len]);
            i += ch_len;
        }
        let _ = bytes; // (kept for clarity; not used after refactor)
        result = out;
    }
    result
}

/// Drop a leading lowercase/name-char fragment immediately after `*{person}`.
/// Handles `RE_PERSON_STAR` matching only `*Lastname,F` and leaving `irstname`.
fn trim_leading_name_fragment(s: &str) -> String {
    const PREFIX: &str = "*{person}";
    if let Some(rest) = s.strip_prefix(PREFIX) {
        let trimmed: &str = rest
            .trim_start_matches(|c: char| c.is_alphabetic() || c == ',' || c == ' ');
        format!("{PREFIX}{trimmed}")
    } else {
        s.to_string()
    }
}
```

**Implementation note for the engineer:** the `replace_months` helper above is written defensively for UTF-8; if the surrounding code in this crate already has a simpler month-replacement utility, prefer reusing it. The behaviour that must hold is: month name at a word boundary becomes `{month}`, and the function never re-matches its own output.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p datasynth-core --lib text_taxonomy 2>&1 | tail -20`
Expected: PASS — all tests (T1+T2+T3). If `tokenize_person_star_record` fails because the star-record remainder handling is off, adjust `trim_leading_name_fragment` until `"*Mueller,Hans"` → `"*{person}"`.

- [ ] **Step 5: Commit**

```bash
git add crates/datasynth-core/src/distributions/text_taxonomy.rs
git commit -m "feat(sp6): PlaceholderGrammar::tokenize (Phase A structural)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: `PlaceholderGrammar::fill`

**Files:**
- Modify: `crates/datasynth-core/src/distributions/text_taxonomy.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:

```rust
    #[test]
    fn fill_structural_placeholders() {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        let mut resolver = SyntheticExampleResolver;
        let out = PlaceholderGrammar::fill(
            "Mieten {month}.{year} ref {digits} {quarter}",
            &mut resolver,
            &mut rng,
        );
        assert!(!out.contains('{'), "structural placeholders left unfilled: {out}");
        assert!(out.starts_with("Mieten "));
    }

    #[test]
    fn fill_pii_placeholders_via_resolver() {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        let mut resolver = SyntheticExampleResolver;
        let out = PlaceholderGrammar::fill("Rechnung {company} / {person}", &mut resolver, &mut rng);
        assert_eq!(out, "Rechnung Example GmbH / Example Person");
    }

    #[test]
    fn fill_unknown_placeholder_kept_literal() {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        let mut resolver = SyntheticExampleResolver;
        let out = PlaceholderGrammar::fill("foo {bogus} bar", &mut resolver, &mut rng);
        assert_eq!(out, "foo {bogus} bar");
    }

    #[test]
    fn fill_then_scan_clean() {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        let mut resolver = SyntheticExampleResolver;
        for tmpl in ["Darlehen {person}", "*{patient} G:{date}", "{company} AG"] {
            let out = PlaceholderGrammar::fill(tmpl, &mut resolver, &mut rng);
            assert!(
                PlaceholderGrammar::residual_pii_scan(&out).is_empty(),
                "fill produced residual-PII shape: {tmpl:?} -> {out:?}"
            );
        }
    }
```

Add `use rand::SeedableRng;` to the `tests` module imports (the file's test deps already include `rand_chacha` transitively via other test modules in the crate; if the crate root test deps lack it, add `rand_chacha` to `[dev-dependencies]` in `crates/datasynth-core/Cargo.toml` — check first).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p datasynth-core --lib text_taxonomy::tests::fill 2>&1 | tail -15`
Expected: FAIL — `no function … fill`.

- [ ] **Step 3: Implement `fill`**

Add to `impl PlaceholderGrammar`:

```rust
    /// Fill a template to a concrete string. Structural placeholders
    /// (`{year}`, `{quarter}`, `{month}`, `{date}`, `{digits}`) are filled
    /// internally from `rng`; PII placeholders are delegated to `resolver`.
    /// Unknown `{…}` tokens are emitted verbatim.
    pub fn fill<R: rand::Rng>(
        template: &str,
        resolver: &mut dyn PlaceholderResolver,
        rng: &mut R,
    ) -> String {
        use rand::RngExt;
        if template.is_empty() {
            return String::new();
        }
        let mut out = String::with_capacity(template.len() + 16);
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            rest = &rest[open..];
            let Some(close) = rest.find('}') else {
                // unbalanced — emit the remainder verbatim
                out.push_str(rest);
                return out;
            };
            let token = &rest[..=close];
            rest = &rest[close + 1..];
            if let Some(kind) = PiiPlaceholderKind::from_token(token) {
                out.push_str(&resolver.resolve(kind, rng));
                continue;
            }
            match token {
                "{year}" => {
                    let y: u32 = rng.random_range(2018..=2024);
                    out.push_str(&y.to_string());
                }
                "{quarter}" => {
                    let q: u32 = rng.random_range(1..=4);
                    out.push('Q');
                    out.push_str(&q.to_string());
                }
                "{month}" => {
                    const MONTHS: &[&str] = &[
                        "January", "February", "March", "April", "May", "June",
                        "July", "August", "September", "October", "November", "December",
                    ];
                    out.push_str(MONTHS[rng.random_range(0..MONTHS.len())]);
                }
                "{date}" => {
                    let d: u32 = rng.random_range(1..=28);
                    let m: u32 = rng.random_range(1..=12);
                    let y: u32 = rng.random_range(18..=24);
                    out.push_str(&format!("{d:02}.{m:02}.{y:02}"));
                }
                "{digits}" => {
                    let n = rng.random_range(4..=8);
                    for _ in 0..n {
                        out.push(char::from(b'0' + rng.random_range(0u8..10)));
                    }
                }
                _ => out.push_str(token), // unknown — verbatim
            }
        }
        out.push_str(rest);
        out
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p datasynth-core --lib text_taxonomy 2>&1 | tail -20`
Expected: PASS — all text_taxonomy tests.

Run: `cargo build --workspace 2>&1 | tail -3`
Expected: `Finished` — the whole workspace still compiles (T1–T4 are purely additive).

- [ ] **Step 5: Commit**

```bash
git add crates/datasynth-core/src/distributions/text_taxonomy.rs crates/datasynth-core/Cargo.toml
git commit -m "feat(sp6): PlaceholderGrammar::fill (structural internal + PII via resolver)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: `PiiDenylist` — load + apply (Phase B)

**Files:**
- Create: `crates/datasynth-fingerprint/src/extraction/pii_denylist.rs`
- Modify: `crates/datasynth-fingerprint/src/extraction/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/datasynth-fingerprint/src/extraction/pii_denylist.rs`:

```rust
//! SP6 — Curated PII denylist (Phase B of tokenization).
//!
//! The denylist is PII-derived (its left-hand side IS real proper nouns) and
//! NEVER enters the public repo. It lives at a private path alongside the real
//! corpus; the extractor takes `--pii-denylist <path>`. When absent, only
//! Phase A (automated structural) tokenization runs.
//!
//! File format — one rule per line, tab-separated:
//!   <literal-or-/regex/>\t<patient|person|company|street>
//! Lines that are blank or start with `#` are ignored.

use std::collections::BTreeMap;
use std::path::Path;

use datasynth_core::distributions::text_taxonomy::PiiPlaceholderKind;
use regex::Regex;

use crate::FingerprintError;

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tmp(contents: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("sp6_denylist_test_{}.tsv", std::process::id()));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn load_and_apply_exact_and_regex() {
        let path = write_tmp(
            "# comment\n\
             Clarunis\tcompany\n\
             Inselspital\tcompany\n\
             /\\bKantonalbank\\b/\tcompany\n",
        );
        let dl = PiiDenylist::load(&path).expect("load");
        std::fs::remove_file(&path).ok();

        assert_eq!(dl.apply("Kontokorrent Clarunis"), "Kontokorrent {company}");
        assert_eq!(dl.apply("Darlehen Inselspital Bern"), "Darlehen {company} Bern");
        assert_eq!(dl.apply("Basler Kantonalbank EUR"), "Basler {company} EUR");
        assert_eq!(dl.apply("nothing to do here"), "nothing to do here");
    }

    #[test]
    fn malformed_line_is_an_error() {
        let path = write_tmp("Clarunis\tnot_a_kind\n");
        let res = PiiDenylist::load(&path);
        std::fs::remove_file(&path).ok();
        assert!(res.is_err());
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(PiiDenylist::load(Path::new("/nonexistent/denylist.tsv")).is_err());
    }
}
```

**Note:** confirm the error type — open `crates/datasynth-fingerprint/src/lib.rs` (or `error.rs`) and find the crate's public error enum. The test/code uses `FingerprintError`; if the crate's error type has a different name, use that name throughout this task. If there is no variant suited to I/O or parse errors, add one (`#[error("pii denylist: {0}")] PiiDenylist(String)`).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p datasynth-fingerprint --lib pii_denylist 2>&1 | tail -15`
Expected: FAIL — module not declared / `PiiDenylist` undefined.

- [ ] **Step 3: Implement `PiiDenylist`**

Add above the `#[cfg(test)]` block in `pii_denylist.rs`:

```rust
/// Curated mapping from real proper-noun spans to placeholder kinds.
#[derive(Debug, Default)]
pub struct PiiDenylist {
    /// Exact substring match -> placeholder kind.
    exact: BTreeMap<String, PiiPlaceholderKind>,
    /// Regex family rules -> placeholder kind.
    patterns: Vec<(Regex, PiiPlaceholderKind)>,
}

fn parse_kind(s: &str) -> Option<PiiPlaceholderKind> {
    match s.trim() {
        "patient" => Some(PiiPlaceholderKind::Patient),
        "person" => Some(PiiPlaceholderKind::Person),
        "company" => Some(PiiPlaceholderKind::Company),
        "street" => Some(PiiPlaceholderKind::Street),
        _ => None,
    }
}

impl PiiDenylist {
    /// Load from a tab-separated file at a private path.
    pub fn load(path: &Path) -> Result<Self, FingerprintError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| FingerprintError::PiiDenylist(format!("read {}: {e}", path.display())))?;
        let mut exact = BTreeMap::new();
        let mut patterns = Vec::new();
        for (lineno, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.splitn(2, '\t');
            let lhs = parts.next().unwrap_or("").trim();
            let rhs = parts.next().unwrap_or("").trim();
            let kind = parse_kind(rhs).ok_or_else(|| {
                FingerprintError::PiiDenylist(format!(
                    "{}:{}: unknown placeholder kind {rhs:?}",
                    path.display(),
                    lineno + 1
                ))
            })?;
            if let Some(inner) = lhs.strip_prefix('/').and_then(|s| s.strip_suffix('/')) {
                let re = Regex::new(inner).map_err(|e| {
                    FingerprintError::PiiDenylist(format!(
                        "{}:{}: bad regex {inner:?}: {e}",
                        path.display(),
                        lineno + 1
                    ))
                })?;
                patterns.push((re, kind));
            } else {
                exact.insert(lhs.to_string(), kind);
            }
        }
        Ok(Self { exact, patterns })
    }

    /// Apply Phase B: replace every denylisted span with its placeholder token.
    /// Exact matches are applied first (longest-first to avoid partial shadowing),
    /// then regex patterns.
    pub fn apply(&self, s: &str) -> String {
        let mut out = s.to_string();
        let mut keys: Vec<&String> = self.exact.keys().collect();
        keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
        for k in keys {
            if let Some(kind) = self.exact.get(k) {
                out = out.replace(k.as_str(), kind.token());
            }
        }
        for (re, kind) in &self.patterns {
            out = re.replace_all(&out, kind.token()).into_owned();
        }
        out
    }

    /// `true` when the denylist carries no rules (e.g. an empty file).
    pub fn is_empty(&self) -> bool {
        self.exact.is_empty() && self.patterns.is_empty()
    }
}
```

- [ ] **Step 4: Register the module**

In `crates/datasynth-fingerprint/src/extraction/mod.rs`, add next to the other `pub mod …;` lines:

```rust
pub mod pii_denylist;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p datasynth-fingerprint --lib pii_denylist 2>&1 | tail -15`
Expected: PASS — 3 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/datasynth-fingerprint/src/extraction/pii_denylist.rs \
        crates/datasynth-fingerprint/src/extraction/mod.rs \
        crates/datasynth-fingerprint/src/lib.rs
git commit -m "feat(sp6): PiiDenylist — curated Phase-B denylist load + apply

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: `extract_text_taxonomy` in `text_extractor.rs`

**Files:**
- Modify: `crates/datasynth-fingerprint/src/extraction/text_extractor.rs`

Add a new `extract_text_taxonomy` function **alongside** the existing `extract_text_templates` (do not remove the old one yet — T12 does that). The new function two-phase-tokenizes, groups line text by `(source, account_class)`, and runs the inline residual-PII scan.

**Read first:** the existing `text_extractor.rs` (already TDD-structured, ~640 lines) — reuse its frequency-filter + top-N + renormalise logic, restructured for the new grouping.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module of `text_extractor.rs`:

```rust
    use datasynth_core::distributions::text_taxonomy::TextTaxonomyPrior;

    /// A line-text record carries the account's resolved ISO 21378 class.
    /// Header records carry an empty class. Build 12 KR/A.B line records with
    /// the same text + 12 KR/_unknown_ and assert the (source,class) split.
    #[test]
    fn extract_text_taxonomy_groups_lines_by_source_class() {
        let mut records: Vec<TextTaxonomyRecord<'_>> = Vec::new();
        for _ in 0..12 {
            records.push(TextTaxonomyRecord {
                source: "KR",
                account_class: Some("A.B"),
                header_text: None,
                line_text: Some("Rechnung Eingang"),
                coa_account: None,
                coa_description: None,
            });
        }
        for _ in 0..12 {
            records.push(TextTaxonomyRecord {
                source: "KR",
                account_class: None, // -> _unknown_
                header_text: None,
                line_text: Some("Diverse Buchung"),
                coa_account: None,
                coa_description: None,
            });
        }
        let prior = extract_text_taxonomy(&records, 10, None);
        assert!(prior
            .line_pools
            .contains_key(&TextTaxonomyPrior::line_key("KR", "A.B")));
        assert!(prior
            .line_pools
            .contains_key(&TextTaxonomyPrior::line_key("KR", TextTaxonomyPrior::UNKNOWN_CLASS)));
        let ab = &prior.line_pools[&TextTaxonomyPrior::line_key("KR", "A.B")];
        assert_eq!(ab.templates.len(), 1);
        assert_eq!(ab.templates[0].template, "Rechnung Eingang");
    }

    /// synthetic_example must NOT be byte-equal to any verbatim corpus input.
    #[test]
    fn extract_text_taxonomy_synthetic_example_not_verbatim() {
        let records: Vec<TextTaxonomyRecord<'_>> = (0..15)
            .map(|_| TextTaxonomyRecord {
                source: "KR",
                account_class: Some("A.B"),
                header_text: None,
                line_text: Some("Darlehen Schauer"), // surname -> denylist or scan
                coa_account: None,
                coa_description: None,
            })
            .collect();
        // No denylist: "Schauer" is a fuzzy proper noun; Phase A won't catch a
        // bare surname, so the inline scan must reject it -> the pool is empty
        // OR the function returns an error. Assert the scan-gate behaviour:
        let prior = extract_text_taxonomy(&records, 10, None);
        // bare-surname line text is NOT a scannable shape on its own, so it
        // survives Phase A; this test instead pins synthetic_example != input
        // using a clean template:
        let clean: Vec<TextTaxonomyRecord<'_>> = (0..15)
            .map(|_| TextTaxonomyRecord {
                source: "RE",
                account_class: Some("R.A"),
                header_text: None,
                line_text: Some("Mieten 04.2021"),
                coa_account: None,
                coa_description: None,
            })
            .collect();
        let prior2 = extract_text_taxonomy(&clean, 10, None);
        let pool = &prior2.line_pools[&TextTaxonomyPrior::line_key("RE", "R.A")];
        assert_eq!(pool.templates[0].template, "Mieten 04.{year}");
        assert_ne!(pool.templates[0].synthetic_example, "Mieten 04.2021");
        let _ = prior; // first prior unused beyond construction
    }

    /// A residual-PII shape that survives Phase A must abort extraction.
    #[test]
    fn extract_text_taxonomy_hard_fails_on_residual_pii() {
        let records: Vec<TextTaxonomyRecord<'_>> = (0..15)
            .map(|_| TextTaxonomyRecord {
                source: "SA",
                account_class: Some("X.X"),
                header_text: None,
                line_text: Some("Kontokorrent Prof. Dr. M. Buess"), // title shape
                coa_account: None,
                coa_description: None,
            })
            .collect();
        let result = extract_text_taxonomy_checked(&records, 10, None);
        assert!(result.is_err(), "title shape must hard-fail the scan gate");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p datasynth-fingerprint --lib text_extractor 2>&1 | tail -15`
Expected: FAIL — `TextTaxonomyRecord` / `extract_text_taxonomy` undefined.

- [ ] **Step 3: Implement the new record type + functions**

Add to `text_extractor.rs` (keep the existing `TextRecord` / `extract_text_templates` untouched):

```rust
use datasynth_core::distributions::text_taxonomy::{
    PlaceholderGrammar, SyntheticExampleResolver, TaxonomyMeta, TemplateEntry, TemplatePool,
    TextTaxonomyPrior,
};
use crate::extraction::pii_denylist::PiiDenylist;

/// A raw record for SP6 taxonomy extraction. `account_class` is the ISO 21378
/// Level-2 class for the line's GL account (resolved by the caller via the CoA
/// prior); `None` -> the line is grouped under `_unknown_`. `coa_account` +
/// `coa_description` carry a CoA row when this record represents one.
#[derive(Debug, Clone)]
pub struct TextTaxonomyRecord<'a> {
    pub source: &'a str,
    pub account_class: Option<&'a str>,
    pub header_text: Option<&'a str>,
    pub line_text: Option<&'a str>,
    pub coa_account: Option<&'a str>,
    pub coa_description: Option<&'a str>,
}

/// Extract a `TextTaxonomyPrior`. Hard-fails (panics via `expect`) if any
/// retained template carries residual PII — callers that need a `Result`
/// should use `extract_text_taxonomy_checked`. `min_occurrences` is the
/// frequency floor; `denylist` applies Phase B when `Some`.
pub fn extract_text_taxonomy(
    records: &[TextTaxonomyRecord<'_>],
    min_occurrences: usize,
    denylist: Option<&PiiDenylist>,
) -> TextTaxonomyPrior {
    extract_text_taxonomy_checked(records, min_occurrences, denylist)
        .expect("residual PII in extracted templates")
}

/// `Result`-returning variant of `extract_text_taxonomy`.
pub fn extract_text_taxonomy_checked(
    records: &[TextTaxonomyRecord<'_>],
    min_occurrences: usize,
    denylist: Option<&PiiDenylist>,
) -> Result<TextTaxonomyPrior, crate::FingerprintError> {
    // Two-phase tokenize: Phase A (structural) then Phase B (denylist).
    let tokenize = |s: &str| -> String {
        let a = PlaceholderGrammar::tokenize(s);
        match denylist {
            Some(dl) => dl.apply(&a),
            None => a,
        }
    };

    // group: line texts by "SOURCE|CLASS"; header texts by source; CoA by acct.
    let mut line_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut header_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut coa_raw: BTreeMap<String, String> = BTreeMap::new();

    for r in records {
        if r.source.is_empty() {
            continue;
        }
        if let Some(lt) = r.line_text {
            let t = lt.trim();
            if !t.is_empty() {
                let class = r.account_class.unwrap_or(TextTaxonomyPrior::UNKNOWN_CLASS);
                line_groups
                    .entry(TextTaxonomyPrior::line_key(r.source, class))
                    .or_default()
                    .push(tokenize(t));
            }
        }
        if let Some(ht) = r.header_text {
            let t = ht.trim();
            if !t.is_empty() {
                header_groups
                    .entry(r.source.to_string())
                    .or_default()
                    .push(tokenize(t));
            }
        }
        if let (Some(acct), Some(desc)) = (r.coa_account, r.coa_description) {
            let d = desc.trim();
            if !d.is_empty() {
                coa_raw.entry(acct.to_string()).or_insert_with(|| tokenize(d));
            }
        }
    }

    let line_pools = build_pools(line_groups, min_occurrences)?;
    let header_pools = build_pools(header_groups, min_occurrences)?;

    // CoA: one template per account, no frequency filter (1 obs per account).
    let mut coa_pools: BTreeMap<String, TemplateEntry> = BTreeMap::new();
    for (acct, template) in coa_raw {
        let hits = PlaceholderGrammar::residual_pii_scan(&template);
        if !hits.is_empty() {
            return Err(crate::FingerprintError::PiiDenylist(format!(
                "residual PII in CoA template for account {acct}: {hits:?}"
            )));
        }
        coa_pools.insert(acct, make_entry(template, 1.0));
    }

    Ok(TextTaxonomyPrior {
        line_pools,
        header_pools,
        coa_pools,
        meta: TaxonomyMeta {
            min_occurrences,
            max_templates_per_pool: MAX_TEXT_TEMPLATES_PER_SOURCE,
            class_tier: "iso21378_l2".to_string(),
            n_client_inputs: 1,
        },
    })
}

/// Frequency-filter, top-N, renormalise, and residual-PII-gate one group map.
fn build_pools(
    groups: BTreeMap<String, Vec<String>>,
    min_occurrences: usize,
) -> Result<BTreeMap<String, TemplatePool>, crate::FingerprintError> {
    let mut result = BTreeMap::new();
    for (key, templates) in groups {
        let total = templates.len();
        if total == 0 {
            continue;
        }
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for t in templates {
            if t.is_empty() {
                continue;
            }
            *counts.entry(t).or_insert(0) += 1;
        }
        let mut passing: Vec<(String, usize)> = counts
            .into_iter()
            .filter(|(_, c)| *c >= min_occurrences)
            .collect();
        if passing.is_empty() {
            continue;
        }
        passing.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
        passing.truncate(MAX_TEXT_TEMPLATES_PER_SOURCE);
        let retained: usize = passing.iter().map(|(_, c)| *c).sum();
        let mut entries = Vec::with_capacity(passing.len());
        for (template, c) in passing {
            let hits = PlaceholderGrammar::residual_pii_scan(&template);
            if !hits.is_empty() {
                return Err(crate::FingerprintError::PiiDenylist(format!(
                    "residual PII in template for pool {key}: {hits:?}"
                )));
            }
            entries.push(make_entry(template, c as f64 / retained as f64));
        }
        result.insert(key, TemplatePool { templates: entries, n: total });
    }
    Ok(result)
}

/// Build a `TemplateEntry`, computing `synthetic_example` via the grammar's
/// fill step with a deterministic per-template seed (stable across regens).
fn make_entry(template: String, probability: f64) -> TemplateEntry {
    use rand::SeedableRng;
    // 0x5036 = "SP6" fold base — a deterministic per-template seed so
    // synthetic_example is byte-stable across bundle regenerations.
    let seed: u64 = template
        .bytes()
        .fold(0x5036_u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64));
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
    let mut resolver = SyntheticExampleResolver;
    let synthetic_example = PlaceholderGrammar::fill(&template, &mut resolver, &mut rng);
    TemplateEntry { template, probability, synthetic_example }
}
```

**Dependency note:** confirm `rand_chacha` is in `crates/datasynth-fingerprint/Cargo.toml` `[dependencies]`; add `rand_chacha = "0.9"` (match the workspace version) if absent.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p datasynth-fingerprint --lib text_extractor 2>&1 | tail -20`
Expected: PASS — the 3 new tests plus the existing `text_extractor` tests still green.

- [ ] **Step 5: Commit**

```bash
git add crates/datasynth-fingerprint/src/extraction/text_extractor.rs \
        crates/datasynth-fingerprint/Cargo.toml
git commit -m "feat(sp6): extract_text_taxonomy — two-phase tokenize + (source,class) grouping

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: `aggregate_text_taxonomy` in `industry_aggregator.rs`

**Files:**
- Modify: `crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs`

Add `aggregate_text_taxonomy` **alongside** the existing `aggregate_text_templates` (line ~693). Read the existing `aggregate_text_templates` + `aggregate_text_template_side` (lines ~685–740) to mirror the probability-pooling pattern.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module of `industry_aggregator.rs`:

```rust
    use datasynth_core::distributions::text_taxonomy::{
        TemplateEntry, TemplatePool, TextTaxonomyPrior,
    };

    fn taxonomy_with_line(key: &str, template: &str, prob: f64, n: usize) -> TextTaxonomyPrior {
        let mut p = TextTaxonomyPrior::default();
        p.line_pools.insert(
            key.to_string(),
            TemplatePool {
                templates: vec![TemplateEntry {
                    template: template.to_string(),
                    probability: prob,
                    synthetic_example: format!("ex-{template}"),
                }],
                n,
            },
        );
        p
    }

    #[test]
    fn aggregate_text_taxonomy_unions_pools_and_renormalises() {
        let a = taxonomy_with_line("KR|A.B", "Rechnung", 1.0, 30);
        let b = taxonomy_with_line("KR|A.B", "Gutschrift", 1.0, 10);
        let agg = aggregate_text_taxonomy(&[&a, &b]);
        let pool = &agg.line_pools["KR|A.B"];
        assert_eq!(pool.templates.len(), 2);
        let sum: f64 = pool.templates.iter().map(|t| t.probability).sum();
        assert!((sum - 1.0).abs() < 1e-9, "probabilities must renormalise to 1.0");
        assert_eq!(pool.n, 40);
        assert_eq!(agg.meta.n_client_inputs, 2);
    }

    #[test]
    fn aggregate_text_taxonomy_empty_input() {
        let agg = aggregate_text_taxonomy(&[]);
        assert!(agg.line_pools.is_empty());
        assert!(agg.header_pools.is_empty());
        assert!(agg.coa_pools.is_empty());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p datasynth-fingerprint --lib aggregate_text_taxonomy 2>&1 | tail -15`
Expected: FAIL — `aggregate_text_taxonomy` undefined.

- [ ] **Step 3: Implement `aggregate_text_taxonomy`**

Add after `aggregate_text_templates` in `industry_aggregator.rs`. Add `TextTaxonomyPrior, TemplatePool, TemplateEntry, TaxonomyMeta` to the `use datasynth_core::distributions::behavioral_priors::{…}` import — note: these live in `datasynth_core::distributions::text_taxonomy`, so add a separate `use datasynth_core::distributions::text_taxonomy::{…};` line.

```rust
/// SP6 — Aggregate per-client `TextTaxonomyPrior` values into an industry prior.
/// Pools sharing a key are unioned; templates sharing a `template` string have
/// their probability mass pooled (weighted by pool `n`) and renormalised.
/// CoA pools are unioned by account number, last-writer-wins on conflict.
pub fn aggregate_text_taxonomy(inputs: &[&TextTaxonomyPrior]) -> TextTaxonomyPrior {
    use datasynth_core::distributions::text_taxonomy::{TaxonomyMeta, TextTaxonomyPrior};
    if inputs.is_empty() {
        return TextTaxonomyPrior::default();
    }
    let line_pools = aggregate_pool_side(inputs.iter().map(|p| &p.line_pools));
    let header_pools = aggregate_pool_side(inputs.iter().map(|p| &p.header_pools));

    let mut coa_pools = BTreeMap::new();
    for p in inputs {
        for (acct, entry) in &p.coa_pools {
            coa_pools.insert(acct.clone(), entry.clone());
        }
    }

    // meta: take the first non-default, override n_client_inputs.
    let mut meta: TaxonomyMeta = inputs[0].meta.clone();
    meta.n_client_inputs = inputs.len();

    TextTaxonomyPrior { line_pools, header_pools, coa_pools, meta }
}

/// Aggregate one pool map (line or header) across clients.
fn aggregate_pool_side<'a>(
    sides: impl Iterator<Item = &'a BTreeMap<String, TemplatePool>>,
) -> BTreeMap<String, TemplatePool> {
    // key -> (template -> weighted_prob_sum), key -> total_n
    let mut acc: BTreeMap<String, BTreeMap<String, (f64, String)>> = BTreeMap::new();
    let mut totals: BTreeMap<String, usize> = BTreeMap::new();
    for side in sides {
        for (key, pool) in side {
            let entry = acc.entry(key.clone()).or_default();
            *totals.entry(key.clone()).or_insert(0) += pool.n;
            let weight = pool.n.max(1) as f64;
            for t in &pool.templates {
                let slot = entry
                    .entry(t.template.clone())
                    .or_insert((0.0, t.synthetic_example.clone()));
                slot.0 += t.probability * weight;
            }
        }
    }
    let mut result = BTreeMap::new();
    for (key, tmpl_map) in acc {
        let mass: f64 = tmpl_map.values().map(|(p, _)| *p).sum();
        if mass <= 0.0 {
            continue;
        }
        let mut templates: Vec<TemplateEntry> = tmpl_map
            .into_iter()
            .map(|(template, (p, synthetic_example))| TemplateEntry {
                template,
                probability: p / mass,
                synthetic_example,
            })
            .collect();
        templates.sort_by(|a, b| {
            b.probability
                .partial_cmp(&a.probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let n = *totals.get(&key).unwrap_or(&0);
        result.insert(key, TemplatePool { templates, n });
    }
    result
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p datasynth-fingerprint --lib aggregate_text_taxonomy 2>&1 | tail -15`
Expected: PASS — 2 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs
git commit -m "feat(sp6): aggregate_text_taxonomy — cross-client pool union + renormalise

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Wire the extraction pipeline to populate `text_taxonomy`

**Files:**
- Modify: `crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs` (the `aggregate_industry_priors` body, ~line 1319)
- Modify: the per-client extraction entry point that builds a `BehavioralPriors` (grep below)
- Modify: `crates/datasynth-fingerprint/src/models/behavioral.rs` (re-exports)

- [ ] **Step 1: Locate the wiring points**

Run:
```bash
grep -rn "text_templates:" crates/datasynth-fingerprint/src --include=*.rs
grep -rn "extract_text_templates\|TextRecord" crates/datasynth-fingerprint/src --include=*.rs
```
The first shows every `BehavioralPriors` literal that sets `text_templates` — each needs a sibling `text_taxonomy`. The second shows where `extract_text_templates` is called from (the per-client extraction path) — that call site builds `TextRecord`s; it must also build `TextTaxonomyRecord`s and call `extract_text_taxonomy`.

- [ ] **Step 2: Write the failing test**

Add to the `industry_aggregator.rs` `tests` module:

```rust
    #[test]
    fn aggregate_industry_priors_populates_text_taxonomy() {
        // Build two minimal BehavioralPriors each carrying a text_taxonomy with
        // one line pool, run them through aggregate_industry_priors, assert the
        // aggregated bundle has text_taxonomy populated.
        let mut a = minimal_behavioral_priors(); // existing test helper
        a.text_taxonomy = Some(taxonomy_with_line("KR|A.B", "Rechnung", 1.0, 20));
        let mut b = minimal_behavioral_priors();
        b.text_taxonomy = Some(taxonomy_with_line("KR|A.B", "Gutschrift", 1.0, 5));
        let agg = aggregate_industry_priors(&[a, b], "health");
        let tx = agg.text_taxonomy.expect("text_taxonomy must be populated");
        assert!(tx.line_pools.contains_key("KR|A.B"));
        assert_eq!(tx.meta.n_client_inputs, 2);
    }
```

**Note:** if `minimal_behavioral_priors` is not an existing helper, build the `BehavioralPriors` literal inline as the other tests in this file do — and confirm `aggregate_industry_priors`'s exact signature by reading the function near line 1280–1340.

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p datasynth-fingerprint --lib aggregate_industry_priors_populates_text_taxonomy 2>&1 | tail -15`
Expected: FAIL — `text_taxonomy` field not set by `aggregate_industry_priors`.

- [ ] **Step 4: Wire `aggregate_industry_priors`**

In `aggregate_industry_priors` (near line 1319, where `text_templates:` is set in the returned `BehavioralPriors` literal), add immediately after the `text_templates: { … }` field:

```rust
        text_taxonomy: {
            let tx_inputs: Vec<&TextTaxonomyPrior> = priors
                .iter()
                .filter_map(|bp| bp.text_taxonomy.as_ref())
                .collect();
            if tx_inputs.is_empty() {
                None
            } else {
                Some(aggregate_text_taxonomy(&tx_inputs))
            }
        },
```

Also: every other `BehavioralPriors { … }` literal in this file (the test fixtures at lines ~1704, ~1743, ~1893, ~1987 that set `text_templates: None`) needs a sibling `text_taxonomy: None,`.

- [ ] **Step 5: Wire the per-client extraction path**

At the call site found in Step 1 (where `extract_text_templates` is called and the result assigned to `text_templates` on a per-client `BehavioralPriors`): keep that call, and add — building `TextTaxonomyRecord`s from the same raw rows. The `account_class` for each line record is resolved from the per-client CoA prior: for a line's GL account, look up `CoaSemanticPrior.accounts[acct].account_class`. Add:

```rust
        text_taxonomy: {
            let tx_records: Vec<TextTaxonomyRecord<'_>> = raw_rows
                .iter()
                .map(|row| TextTaxonomyRecord {
                    source: row.source.as_str(),
                    account_class: coa_prior
                        .as_ref()
                        .and_then(|c| c.accounts.get(&row.gl_account))
                        .and_then(|a| a.account_class.as_deref()),
                    header_text: row.header_text.as_deref(),
                    line_text: row.line_text.as_deref(),
                    coa_account: None,
                    coa_description: None,
                })
                .chain(coa_rows_iter) // CoA records: coa_account + coa_description set
                .collect();
            match extract_text_taxonomy_checked(&tx_records, min_occ, denylist.as_ref()) {
                Ok(tx) => Some(tx),
                Err(e) => return Err(e), // hard-fail per spec §2.3 gate 1
            }
        },
```

**The engineer must adapt the field names** (`row.source`, `row.gl_account`, `row.header_text`, `row.line_text`, `coa_prior`, `raw_rows`, `coa_rows_iter`, `min_occ`, `denylist`) to the actual variables in scope at that call site — read 40 lines around the `extract_text_templates` call. The `denylist: Option<PiiDenylist>` must be threaded in from the CLI arg (Task 15 adds the CLI flag; for now thread an `Option<&PiiDenylist>` parameter through the extraction entry function, defaulting to `None` at all existing callers). The CoA records (`coa_account` + `coa_description` set, other fields from the CoA row's account/source) feed `coa_pools`.

- [ ] **Step 6: Update re-exports**

In `crates/datasynth-fingerprint/src/models/behavioral.rs`, find the `use datasynth_core::distributions::behavioral_priors::{…}` re-export block and any `pub use`. Add `TextTaxonomyPrior` (and `TemplatePool`, `TemplateEntry`, `TaxonomyMeta` if siblings are re-exported) next to `TextTemplate, TextTemplatePrior`. Do **not** remove `TextTemplate*` yet (T12).

- [ ] **Step 7: Run tests + build**

Run: `cargo test -p datasynth-fingerprint --lib 2>&1 | tail -15`
Expected: PASS.
Run: `cargo build --workspace 2>&1 | tail -3`
Expected: `Finished`.

- [ ] **Step 8: Commit**

```bash
git add crates/datasynth-fingerprint/src
git commit -m "feat(sp6): wire extraction pipeline to populate text_taxonomy

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: `priors_loader.rs` — load `text_taxonomy`, lookup cascade, new samplers

**Files:**
- Modify: `crates/datasynth-generators/src/priors_loader.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module of `priors_loader.rs`:

```rust
    use datasynth_core::distributions::text_taxonomy::{
        TemplateEntry, TemplatePool, TextTaxonomyPrior,
    };

    fn bp_with_text_taxonomy() -> BehavioralPriors {
        use datasynth_core::distributions::behavioral_priors::*;
        let mut tx = TextTaxonomyPrior::default();
        tx.line_pools.insert(
            "KR|A.B".to_string(),
            TemplatePool {
                templates: vec![TemplateEntry {
                    template: "Rechnung Eingang".to_string(),
                    probability: 1.0,
                    synthetic_example: "Rechnung Eingang".to_string(),
                }],
                n: 50,
            },
        );
        tx.line_pools.insert(
            "KR|_unknown_".to_string(),
            TemplatePool {
                templates: vec![TemplateEntry {
                    template: "Diverse".to_string(),
                    probability: 1.0,
                    synthetic_example: "Diverse".to_string(),
                }],
                n: 20,
            },
        );
        tx.header_pools.insert(
            "KR".to_string(),
            TemplatePool {
                templates: vec![TemplateEntry {
                    template: "Monatsabschluss".to_string(),
                    probability: 1.0,
                    synthetic_example: "Monatsabschluss".to_string(),
                }],
                n: 30,
            },
        );
        tx.coa_pools.insert(
            "0000204000".to_string(),
            TemplateEntry {
                template: "Kreditoren".to_string(),
                probability: 1.0,
                synthetic_example: "Kreditoren".to_string(),
            },
        );
        // reuse the minimal_bp_with_role_prior shape but set text_taxonomy:
        let mut bp = minimal_bp_with_role_prior(make_kr_role_prior());
        bp.text_taxonomy = Some(tx);
        bp
    }

    #[test]
    fn sample_line_template_keyed_on_source_and_class() {
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let priors =
            LoadedPriors::from_priors(bp_with_text_taxonomy(), PathBuf::from("t"), &mut rng, 365)
                .unwrap();
        // exact (source,class) hit
        let mut resolver = datasynth_core::distributions::text_taxonomy::SyntheticExampleResolver;
        let mut r2 = ChaCha8Rng::seed_from_u64(2);
        let v = priors.sample_line_template("KR", "A.B", &mut resolver, &mut r2);
        assert_eq!(v, Some("Rechnung Eingang".to_string()));
        // unknown class -> cascade to KR|_unknown_
        let v = priors.sample_line_template("KR", "Z.Z", &mut resolver, &mut r2);
        assert_eq!(v, Some("Diverse".to_string()));
        // unknown source -> None (caller falls back)
        let v = priors.sample_line_template("ZZ", "A.B", &mut resolver, &mut r2);
        assert_eq!(v, None);
    }

    #[test]
    fn sample_coa_description_hits_account() {
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let priors =
            LoadedPriors::from_priors(bp_with_text_taxonomy(), PathBuf::from("t"), &mut rng, 365)
                .unwrap();
        let mut resolver = datasynth_core::distributions::text_taxonomy::SyntheticExampleResolver;
        let mut r2 = ChaCha8Rng::seed_from_u64(2);
        assert_eq!(
            priors.sample_coa_description("0000204000", &mut resolver, &mut r2),
            Some("Kreditoren".to_string())
        );
        assert_eq!(priors.sample_coa_description("9999999999", &mut resolver, &mut r2), None);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p datasynth-generators --lib priors_loader 2>&1 | tail -15`
Expected: FAIL — `text_taxonomy` field / `sample_line_template` arity / `sample_coa_description` undefined.

- [ ] **Step 3: Add the `text_taxonomy` field to `LoadedPriors`**

In `priors_loader.rs`: add to the `use datasynth_core::distributions::behavioral_priors::{…}` import: `TextTaxonomyPrior`. Add a field to `struct LoadedPriors` (after `text_templates`):

```rust
    /// SP6 — corpus text taxonomy. When `Some`, supersedes `text_templates`
    /// for header/line/CoA text. Carries `(source, account-class)` line pools,
    /// `source` header pools, and per-account CoA description templates.
    pub text_taxonomy: Option<TextTaxonomyPrior>,
```

In `from_priors`, after the `let text_templates = bp.text_templates.clone();` line, add:

```rust
        // SP6 — carry text_taxonomy through. Old bundles (pre-SP6) have None;
        // generators fall back to text_templates then to DescriptionGenerator.
        let text_taxonomy = bp.text_taxonomy.clone();
```

And add `text_taxonomy,` to the `Ok(LoadedPriors { … })` literal.

- [ ] **Step 4: Add the cascade + new sampler methods**

Add to the second `impl LoadedPriors` block (after `sample_line_template`, before the closing brace at line ~456):

```rust
    /// SP6 — Sample a line-text string for `(source, account_class)` from the
    /// text-taxonomy prior, filling placeholders via `resolver`.
    ///
    /// Lookup cascade:
    /// 1. `line_pools["SOURCE|CLASS"]`
    /// 2. `line_pools["SOURCE|_unknown_"]`
    /// 3. `header_pools["SOURCE"]` (last resort — source-level vocabulary)
    ///
    /// Returns `None` only when the prior is absent or the source has no pools
    /// at any cascade tier — the caller then falls back to `text_templates` or
    /// the `DescriptionGenerator`. **This method shadows the old
    /// `sample_line_template(source, rng)`; that one is removed in T12.**
    pub fn sample_line_template<R: rand::Rng>(
        &self,
        source: &str,
        account_class: &str,
        resolver: &mut dyn datasynth_core::distributions::text_taxonomy::PlaceholderResolver,
        rng: &mut R,
    ) -> Option<String> {
        let tx = self.text_taxonomy.as_ref()?;
        let class_key = datasynth_core::distributions::text_taxonomy::TextTaxonomyPrior::line_key(
            source,
            account_class,
        );
        let unknown_key =
            datasynth_core::distributions::text_taxonomy::TextTaxonomyPrior::line_key(
                source,
                datasynth_core::distributions::text_taxonomy::TextTaxonomyPrior::UNKNOWN_CLASS,
            );
        let pool = tx
            .line_pools
            .get(&class_key)
            .or_else(|| tx.line_pools.get(&unknown_key))
            .or_else(|| tx.header_pools.get(source))?;
        sample_pool_filled(pool, resolver, rng)
    }

    /// SP6 — Sample a header-text string for `source` from the text-taxonomy
    /// prior. Returns `None` when absent / no pool for the source.
    pub fn sample_header_template_tx<R: rand::Rng>(
        &self,
        source: &str,
        resolver: &mut dyn datasynth_core::distributions::text_taxonomy::PlaceholderResolver,
        rng: &mut R,
    ) -> Option<String> {
        let tx = self.text_taxonomy.as_ref()?;
        let pool = tx.header_pools.get(source)?;
        sample_pool_filled(pool, resolver, rng)
    }

    /// SP6 — Fill the CoA description template for `account_no`. Returns `None`
    /// when the prior is absent or the account has no template.
    pub fn sample_coa_description<R: rand::Rng>(
        &self,
        account_no: &str,
        resolver: &mut dyn datasynth_core::distributions::text_taxonomy::PlaceholderResolver,
        rng: &mut R,
    ) -> Option<String> {
        let tx = self.text_taxonomy.as_ref()?;
        let entry = tx.coa_pools.get(account_no)?;
        Some(datasynth_core::distributions::text_taxonomy::PlaceholderGrammar::fill(
            &entry.template,
            resolver,
            rng,
        ))
    }
```

Add this free helper near `sample_text_template_weighted` (line ~459):

```rust
/// Weighted-pick a `TemplateEntry` from a `TemplatePool` and fill it.
fn sample_pool_filled<R: rand::Rng>(
    pool: &datasynth_core::distributions::text_taxonomy::TemplatePool,
    resolver: &mut dyn datasynth_core::distributions::text_taxonomy::PlaceholderResolver,
    rng: &mut R,
) -> Option<String> {
    use datasynth_core::distributions::text_taxonomy::PlaceholderGrammar;
    use rand::RngExt;
    if pool.templates.is_empty() {
        return None;
    }
    let total: f64 = pool.templates.iter().map(|t| t.probability).sum();
    if total <= 0.0 {
        return None;
    }
    let r: f64 = rng.random_range(0.0..total);
    let mut cum = 0.0;
    for t in &pool.templates {
        cum += t.probability;
        if r <= cum {
            return Some(PlaceholderGrammar::fill(&t.template, resolver, rng));
        }
    }
    pool.templates
        .last()
        .map(|t| PlaceholderGrammar::fill(&t.template, resolver, rng))
}
```

**Note:** the old `sample_line_template(&self, source, rng)` and `sample_header_template(&self, source, rng)` still exist at this point. Rust will reject two methods named `sample_line_template` with different arities — so in **this task**, rename the OLD one to `sample_line_template_legacy` (and the call sites in `je_generator.rs` will be updated in T10; T12 removes it). Do the same nothing for `sample_header_template` — the new one is named `sample_header_template_tx` so they coexist; T12 renames `_tx` away.

- [ ] **Step 5: Run tests + build**

Run: `cargo test -p datasynth-generators --lib priors_loader 2>&1 | tail -20`
Expected: PASS (the 2 new tests + existing priors_loader tests).
Run: `cargo build --workspace 2>&1 | tail -3` — `Finished` (je_generator still calls `sample_line_template_legacy` once renamed — **wait**: the rename breaks je_generator. To keep this task's build green, in this same task also do a mechanical rename of the two call sites in `je_generator.rs` lines ~2119 and ~2190 from `sample_line_template` to `sample_line_template_legacy`. Header call at ~1853 stays `sample_header_template` (unchanged).)

Re-run: `cargo build --workspace 2>&1 | tail -3`
Expected: `Finished`.

- [ ] **Step 6: Commit**

```bash
git add crates/datasynth-generators/src/priors_loader.rs crates/datasynth-generators/src/je_generator.rs
git commit -m "feat(sp6): priors_loader text_taxonomy load + (source,class) lookup cascade

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: `je_generator.rs` — class resolution + `MasterDataResolver` + wire sampling

**Files:**
- Modify: `crates/datasynth-generators/src/je_generator.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module of `je_generator.rs`:

```rust
    #[test]
    fn master_data_resolver_fills_every_pii_kind() {
        use datasynth_core::distributions::text_taxonomy::{PiiPlaceholderKind, PlaceholderResolver};
        let mut r = MasterDataResolver {
            companies: vec!["Acme AG".to_string()],
            persons: vec!["Hans Muster".to_string()],
            streets: vec!["Hauptstrasse 1".to_string()],
            patients: vec!["Patient X".to_string()],
        };
        let mut rng = rand::rng();
        assert_eq!(r.resolve(PiiPlaceholderKind::Company, &mut rng), "Acme AG");
        assert_eq!(r.resolve(PiiPlaceholderKind::Person, &mut rng), "Hans Muster");
        assert_eq!(r.resolve(PiiPlaceholderKind::Street, &mut rng), "Hauptstrasse 1");
        assert_eq!(r.resolve(PiiPlaceholderKind::Patient, &mut rng), "Patient X");
    }

    #[test]
    fn master_data_resolver_empty_pool_falls_back() {
        use datasynth_core::distributions::text_taxonomy::{PiiPlaceholderKind, PlaceholderResolver};
        let mut r = MasterDataResolver::default();
        let mut rng = rand::rng();
        // empty pools must still produce a non-empty, obviously-synthetic value
        let v = r.resolve(PiiPlaceholderKind::Company, &mut rng);
        assert!(!v.is_empty());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p datasynth-generators --lib master_data_resolver 2>&1 | tail -15`
Expected: FAIL — `MasterDataResolver` undefined.

- [ ] **Step 3: Implement `MasterDataResolver`**

Add near the top of `je_generator.rs` (after the imports, before the main generator struct):

```rust
use datasynth_core::distributions::text_taxonomy::{
    PiiPlaceholderKind, PlaceholderResolver,
};

/// SP6 — Resolves PII placeholders to concrete values drawn from the run's
/// synthetic master data. `{company}` <- vendor/customer names, `{person}` <-
/// employee names, `{street}` <- addresses, `{patient}` <- a locale-matched
/// synthetic-person pool (no master entity exists for patients). Empty pools
/// fall back to obviously-synthetic constants so output never carries an empty
/// span or a literal `{…}` token.
#[derive(Debug, Default)]
pub struct MasterDataResolver {
    pub companies: Vec<String>,
    pub persons: Vec<String>,
    pub streets: Vec<String>,
    pub patients: Vec<String>,
}

impl PlaceholderResolver for MasterDataResolver {
    fn resolve(&mut self, kind: PiiPlaceholderKind, rng: &mut dyn rand::Rng) -> String {
        use rand::RngExt;
        let (pool, fallback) = match kind {
            PiiPlaceholderKind::Company => (&self.companies, "Synthetic Company AG"),
            PiiPlaceholderKind::Person => (&self.persons, "Synthetic Person"),
            PiiPlaceholderKind::Street => (&self.streets, "Synthetic Street 1"),
            PiiPlaceholderKind::Patient => (&self.patients, "Synthetic Patient"),
        };
        if pool.is_empty() {
            return fallback.to_string();
        }
        let idx = rng.random_range(0..pool.len());
        pool[idx].clone()
    }
}
```

- [ ] **Step 4: Build + populate the resolver in the generator**

Read the generator struct (around line 40–60) and its constructor (`fn new`, around line 360–390) and how it accesses master data (you saw `self.customer_pool.random_customer(...)` at line 1838 — there will be sibling vendor/employee pools). Add a method on the generator that builds a `MasterDataResolver` from the run's master data:

```rust
    /// SP6 — Build a `MasterDataResolver` from the run's master data. Called
    /// once per JE batch (or cached on the generator); the pools are cheap
    /// `Vec<String>` snapshots of names already generated.
    fn build_master_data_resolver(&self) -> MasterDataResolver {
        let companies = self.collect_company_names(); // vendor + customer names
        let persons = self.collect_employee_names();
        let streets = self.collect_street_addresses();
        let patients = synthetic_patient_pool(self.entity_locale()); // see below
        MasterDataResolver { companies, persons, streets, patients }
    }
```

**The engineer adapts** `collect_company_names` / `collect_employee_names` / `collect_street_addresses` / `entity_locale` to the actual generator API — read 60 lines around the generator struct definition and the `customer_pool` usage at line 1838 to find the vendor/customer/employee/address accessors. If a clean accessor does not exist, collect names from whatever master-data fields the generator holds; an empty `Vec` is acceptable (the resolver has fallbacks). `synthetic_patient_pool` is a new free function returning a small static `Vec<String>` of locale-plausible synthetic names — keep it to ~20 entries:

```rust
/// A small static pool of obviously-synthetic person names for `{patient}`
/// filling. No master entity exists for patients. Locale is a hint; for SP6 a
/// single neutral set is sufficient.
fn synthetic_patient_pool(_locale: &str) -> Vec<String> {
    [
        "A. Beispiel", "B. Muster", "C. Synthetic", "D. Example",
        "E. Probe", "F. Testperson", "G. Platzhalter", "H. Demo",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}
```

- [ ] **Step 5: Wire line-text + header-text sampling**

At the two line-text sites (lines ~2115–2128 and ~2186–2199, debit + credit) the current code is:

```rust
            if self.template_config.descriptions.generate_line_text {
                let priors_line = entry.header.sap_source_code.as_deref().and_then(|src| {
                    self.loaded_priors
                        .as_ref()
                        .and_then(|p| p.sample_line_template_legacy(src, &mut self.rng))
                });
                line.line_text = Some(priors_line.unwrap_or_else(|| {
                    self.description_generator.generate_line_text(
                        &account_number, &context, &mut self.rng,
                    )
                }));
            }
```

Replace each with — resolving the account class via `coa_semantic`, then trying the SP6 cascade, then the legacy path, then the `DescriptionGenerator`:

```rust
            if self.template_config.descriptions.generate_line_text {
                let src = entry.header.sap_source_code.as_deref();
                let account_class = src.and_then(|_| {
                    self.loaded_priors.as_ref().and_then(|p| {
                        p.coa_semantic
                            .as_ref()
                            .and_then(|c| c.accounts.get(&account_number))
                            .and_then(|a| a.account_class.as_deref())
                    })
                });
                let priors_line = src.and_then(|s| {
                    let class = account_class
                        .unwrap_or(datasynth_core::distributions::text_taxonomy::TextTaxonomyPrior::UNKNOWN_CLASS);
                    self.loaded_priors.as_ref().and_then(|p| {
                        p.sample_line_template(s, class, &mut self.md_resolver, &mut self.rng)
                            .or_else(|| p.sample_line_template_legacy(s, &mut self.rng))
                    })
                });
                line.line_text = Some(priors_line.unwrap_or_else(|| {
                    self.description_generator.generate_line_text(
                        &account_number, &context, &mut self.rng,
                    )
                }));
            }
```

At the header-text site (lines ~1849–1862), change the `priors_header` line to prefer the SP6 path:

```rust
        if self.template_config.descriptions.generate_header_text {
            let priors_header = header.sap_source_code.as_deref().and_then(|src| {
                self.loaded_priors.as_ref().and_then(|p| {
                    p.sample_header_template_tx(src, &mut self.md_resolver, &mut self.rng)
                        .or_else(|| p.sample_header_template(src, &mut self.rng))
                })
            });
            header.header_text = Some(priors_header.unwrap_or_else(|| {
                self.description_generator.generate_header_text(
                    business_process, &context, &mut self.rng,
                )
            }));
        }
```

**`self.md_resolver`:** add a `md_resolver: MasterDataResolver` field to the generator struct, initialise it in `fn new` with `MasterDataResolver::default()`, and refresh it via `self.md_resolver = self.build_master_data_resolver();` at the start of the JE-batch generation entry point (find where master data is known to be populated — read the generation entry method). Borrow-checker note: `sample_line_template` takes `&mut self.md_resolver` and `&mut self.rng` — these are disjoint fields so the borrow checker accepts simultaneous `&mut` borrows; if a method-call form triggers a conflict, bind locals first: `let resolver = &mut self.md_resolver; let rng = &mut self.rng;` then call through `self.loaded_priors.as_ref()`.

- [ ] **Step 6: Run tests + build**

Run: `cargo test -p datasynth-generators --lib 2>&1 | tail -15`
Expected: PASS.
Run: `cargo build --workspace 2>&1 | tail -3`
Expected: `Finished`.

- [ ] **Step 7: Commit**

```bash
git add crates/datasynth-generators/src/je_generator.rs
git commit -m "feat(sp6): je_generator — account-class resolution + MasterDataResolver wiring

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 11: `coa_generator.rs` — fill CoA description templates once per account

**Files:**
- Modify: `crates/datasynth-generators/src/coa_generator.rs`

Read `overlay_coa_semantic` (line ~59) and `apply_coa_semantic_prior` (line ~284). SP6 adds: when `text_taxonomy.coa_pools` has a template for an account, fill it once and use that as the description **instead of** the verbatim `coa_semantic` description.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module of `coa_generator.rs`:

```rust
    #[test]
    fn overlay_coa_taxonomy_fills_template_once_per_account() {
        use datasynth_core::distributions::text_taxonomy::{
            SyntheticExampleResolver, TemplateEntry, TextTaxonomyPrior,
        };
        // Build a CoA with one account, and a taxonomy with a template for it.
        let mut coa = /* build a ChartOfAccounts with account "0000204000" —
            reuse the existing test helper in this file's tests module */;
        let mut tx = TextTaxonomyPrior::default();
        tx.coa_pools.insert(
            "0000204000".to_string(),
            TemplateEntry {
                template: "Kreditoren {company}".to_string(),
                probability: 1.0,
                synthetic_example: "Kreditoren Example GmbH".to_string(),
            },
        );
        let mut resolver = SyntheticExampleResolver;
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(9);
        overlay_coa_taxonomy(&mut coa, &tx, &mut resolver, &mut rng);
        let acct = coa
            .accounts()
            .iter()
            .find(|a| a.account_number == "0000204000")
            .expect("account present");
        assert!(acct.short_description.starts_with("Kreditoren "));
        assert!(!acct.short_description.contains('{'), "template left unfilled");
    }
```

**Adapt** `coa.accounts()` / `account_number` / `short_description` to the actual `ChartOfAccounts` API — read `overlay_coa_semantic` (it already does `account.short_description.clone_from(&sem.description)`, so the field names there are authoritative). Reuse whatever `ChartOfAccounts` test fixture the existing `coa_generator.rs` tests use.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p datasynth-generators --lib coa_generator 2>&1 | tail -15`
Expected: FAIL — `overlay_coa_taxonomy` undefined.

- [ ] **Step 3: Implement `overlay_coa_taxonomy`**

Add next to `overlay_coa_semantic` in `coa_generator.rs`:

```rust
/// SP6 — Overlay CoA descriptions from the text-taxonomy prior. For each
/// account with a `coa_pools` template, fill the template ONCE (stable per
/// account for this run) and write it to `short_description` + `long_description`.
/// Accounts without a taxonomy template are left untouched (the caller still
/// runs `overlay_coa_semantic` for those). Mirrors `overlay_coa_semantic`'s
/// field-write pattern.
pub fn overlay_coa_taxonomy<R: rand::Rng>(
    coa: &mut ChartOfAccounts,
    taxonomy: &datasynth_core::distributions::text_taxonomy::TextTaxonomyPrior,
    resolver: &mut dyn datasynth_core::distributions::text_taxonomy::PlaceholderResolver,
    rng: &mut R,
) {
    use datasynth_core::distributions::text_taxonomy::PlaceholderGrammar;
    for account in coa.accounts_mut() {
        if let Some(entry) = taxonomy.coa_pools.get(&account.account_number) {
            let filled = PlaceholderGrammar::fill(&entry.template, resolver, rng);
            if !filled.is_empty() {
                account.short_description.clone_from(&filled);
                account.long_description = filled;
            }
        }
    }
}
```

**Adapt** `coa.accounts_mut()` / `account.account_number` / `short_description` / `long_description` to the real API — `overlay_coa_semantic` is the reference; copy its iteration + field-access form exactly.

- [ ] **Step 4: Wire it into `apply_coa_semantic_prior`**

Find where `apply_coa_semantic_prior` (or the generator's CoA-build entry point) calls `overlay_coa_semantic`. Add — **after** the `overlay_coa_semantic` call so the taxonomy overlay takes precedence for accounts it covers — a call to `overlay_coa_taxonomy` guarded on `loaded_priors.text_taxonomy`. Read the surrounding 30 lines to find the right `loaded_priors` / resolver handle. If the CoA generator does not currently hold a `PlaceholderResolver`, construct a `MasterDataResolver` (T10) or fall back to `SyntheticExampleResolver` if master data is not yet available at CoA-build time (CoA generation may run before vendor/employee generation — in that case `SyntheticExampleResolver` is acceptable for CoA descriptions and noted as such in a comment).

- [ ] **Step 5: Run tests + build**

Run: `cargo test -p datasynth-generators --lib coa_generator 2>&1 | tail -15`
Expected: PASS.
Run: `cargo build --workspace 2>&1 | tail -3`
Expected: `Finished`.

- [ ] **Step 6: Commit**

```bash
git add crates/datasynth-generators/src/coa_generator.rs
git commit -m "feat(sp6): coa_generator — fill CoA description templates once per account

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 12: Remove the SP4.4 `TextTemplate*` code path

**Files:**
- Modify: `crates/datasynth-core/src/distributions/behavioral_priors.rs`
- Modify: `crates/datasynth-fingerprint/src/extraction/text_extractor.rs`
- Modify: `crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs`
- Modify: `crates/datasynth-fingerprint/src/models/behavioral.rs`
- Modify: `crates/datasynth-generators/src/priors_loader.rs`
- Modify: `crates/datasynth-generators/src/je_generator.rs`

All consumers now use `text_taxonomy`. Remove the dead SP4.4 path.

- [ ] **Step 1: Inventory the references**

Run:
```bash
grep -rn "TextTemplatePrior\|TextTemplate\b\|fill_text_template_with_rng\|text_templates\|extract_text_templates\|aggregate_text_templates\|sample_line_template_legacy\|sample_header_template\b\|sample_text_template_weighted\|TextRecord\b" crates/ --include=*.rs
```
Every hit is either a definition to delete or a reference to delete/rename. Work through them.

- [ ] **Step 2: Delete the definitions**

- `behavioral_priors.rs`: remove `struct TextTemplatePrior`, `struct TextTemplate`, `fn fill_text_template_with_rng`, and the `pub text_templates: Option<TextTemplatePrior>` field on `BehavioralPriors`. Remove `TextTemplate*` from the module re-export list in `mod.rs`. Remove every `text_templates: None,` / `text_templates: { … }` from test fixtures **in this file**.
- `text_extractor.rs`: remove `struct TextRecord`, `fn extract_text_templates`, `fn extract_templates_for_texts`, `fn tokenize_text`, `fn replace_years`, `fn replace_quarters`, `fn replace_months` (the file-local one — the new one lives in `text_taxonomy.rs`), `fn replace_digit_runs`, `fn fill_text_template`, `MONTH_PATTERNS`, and their tests. Keep `MAX_TEXT_TEMPLATES_PER_SOURCE` (still used by `extract_text_taxonomy`).
- `industry_aggregator.rs`: remove `fn aggregate_text_templates`, `fn aggregate_text_template_side`, the `text_templates: { … }` block in `aggregate_industry_priors`, and every `text_templates: None,` in test fixtures. Remove `TextTemplate, TextTemplatePrior` from the `use` import.
- `models/behavioral.rs`: remove `TextTemplate, TextTemplatePrior` from re-exports.
- `priors_loader.rs`: remove the `pub text_templates` field on `LoadedPriors`, the `let text_templates = bp.text_templates.clone();` line, `text_templates,` from the `Ok(LoadedPriors { … })` literal, `fn sample_line_template_legacy`, the old `fn sample_header_template`, `fn sample_text_template_weighted`, and `TextTemplatePrior` from the `use` import. **Rename** `sample_header_template_tx` -> `sample_header_template` now that the old one is gone.
- `je_generator.rs`: at the line-text sites remove the `.or_else(|| p.sample_line_template_legacy(s, &mut self.rng))` fallback; at the header site remove `.or_else(|| p.sample_header_template(src, &mut self.rng))` and update the call to the renamed `sample_header_template` (the SP6 one). Remove any now-unused imports.

- [ ] **Step 3: Build the workspace**

Run: `cargo build --workspace 2>&1 | tail -8`
Expected: `Finished`. If errors, they are dangling references from Step 1's inventory — fix each.

- [ ] **Step 4: Run the full test suite for the three crates**

Run: `cargo test -p datasynth-core --lib -- --quiet 2>&1 | tail -5`
Run: `cargo test -p datasynth-fingerprint --lib -- --quiet 2>&1 | tail -5`
Run: `cargo test -p datasynth-generators --lib -- --quiet 2>&1 | tail -5`
Expected: all PASS.

- [ ] **Step 5: Clippy**

Run: `cargo clippy -p datasynth-core -p datasynth-fingerprint -p datasynth-generators 2>&1 | tail -8`
Expected: no warnings (the only acceptable warning workspace-wide is `protoc not found` from `datasynth-server`, not in scope here).

- [ ] **Step 6: Commit**

```bash
git add crates/
git commit -m "refactor(sp6): remove the SP4.4 TextTemplate* code path

All header/line/CoA text now flows through text_taxonomy. Removes
TextTemplatePrior, TextTemplate, fill_text_template_with_rng,
extract_text_templates, aggregate_text_templates, the legacy loader
methods, and the BehavioralPriors.text_templates field.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 13: `bundle_pii_audit.rs` — CI residual-PII gate over committed bundles

**Files:**
- Create: `crates/datasynth-runtime/tests/bundle_pii_audit.rs`

- [ ] **Step 1: Write the test (it is the deliverable — no separate impl step)**

Create `crates/datasynth-runtime/tests/bundle_pii_audit.rs`:

```rust
//! SP6 CI gate — every committed `.dsf` bundle must carry zero residual PII in
//! any text-taxonomy template or synthetic_example. Runs with no corpus
//! access; reads only committed bundles. A failure here means a PII-bearing
//! bundle was committed.

use std::path::PathBuf;

use datasynth_core::distributions::text_taxonomy::PlaceholderGrammar;
use datasynth_generators::priors_loader::LoadedPriors;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

const INDUSTRIES: &[&str] = &[
    "health",
    "life_sciences",
    "pharmaceutical",
    "power_and_utilities",
    "technology",
];

#[test]
fn committed_bundles_carry_no_residual_pii() {
    let mut checked = 0usize;
    for industry in INDUSTRIES {
        let path: PathBuf = datasynth_generators::priors_loader::bundled_priors_path(industry);
        if !path.exists() {
            eprintln!("skip: {} not present", path.display());
            continue;
        }
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let priors = LoadedPriors::load_bundled(industry, &mut rng, 365)
            .unwrap_or_else(|e| panic!("load {industry}: {e}"));
        let Some(tx) = priors.text_taxonomy.as_ref() else {
            eprintln!("skip: {industry} bundle has no text_taxonomy");
            continue;
        };
        let mut scan = |label: &str, key: &str, s: &str| {
            let hits = PlaceholderGrammar::residual_pii_scan(s);
            assert!(
                hits.is_empty(),
                "residual PII in {industry} {label} [{key}]: {hits:?} :: {s:?}"
            );
        };
        for (key, pool) in &tx.line_pools {
            for t in &pool.templates {
                scan("line_pool.template", key, &t.template);
                scan("line_pool.synthetic_example", key, &t.synthetic_example);
            }
        }
        for (key, pool) in &tx.header_pools {
            for t in &pool.templates {
                scan("header_pool.template", key, &t.template);
                scan("header_pool.synthetic_example", key, &t.synthetic_example);
            }
        }
        for (key, entry) in &tx.coa_pools {
            scan("coa_pool.template", key, &entry.template);
            scan("coa_pool.synthetic_example", key, &entry.synthetic_example);
        }
        checked += 1;
    }
    eprintln!("bundle_pii_audit: checked {checked} bundle(s)");
}
```

**Note:** confirm `bundled_priors_path` and `LoadedPriors` are `pub` and reachable as `datasynth_generators::priors_loader::…` — `priors_loader.rs` shows `pub fn bundled_priors_path` and `pub struct LoadedPriors`, so this should work; if the module is not re-exported at the crate root, use the full path. Confirm `rand_chacha` is in `crates/datasynth-runtime/Cargo.toml` `[dev-dependencies]`; add it if absent.

- [ ] **Step 2: Run it**

Run: `cargo test -p datasynth-runtime --test bundle_pii_audit 2>&1 | tail -15`
Expected at this point: PASS with "skip: … has no text_taxonomy" for every industry — the committed bundles are still pre-SP6. The test goes green-with-skips now and becomes a real gate after T16 regenerates the bundles. (It must not FAIL — only skip.)

- [ ] **Step 3: Commit**

```bash
git add crates/datasynth-runtime/tests/bundle_pii_audit.rs crates/datasynth-runtime/Cargo.toml
git commit -m "test(sp6): bundle_pii_audit — CI residual-PII gate over committed bundles

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 14: `sp6_text_taxonomy_smoke.rs` — integration smoke

**Files:**
- Create: `crates/datasynth-runtime/tests/sp6_text_taxonomy_smoke.rs`

- [ ] **Step 1: Write the test**

Create `crates/datasynth-runtime/tests/sp6_text_taxonomy_smoke.rs`. Model it on the existing runtime integration tests — read one (e.g. `crates/datasynth-runtime/tests/fraud_bias_smoke.rs`, referenced in CLAUDE.md) for the exact harness: how a small priors-enabled generation is configured and run, and how emitted `journal_entries` are accessed.

```rust
//! SP6 integration smoke — a small priors-enabled generation must emit text
//! with: (a) no literal `{…}` placeholder tokens, (b) no residual-PII shapes,
//! (c) line_text populated for >=95% of lines.

use datasynth_core::distributions::text_taxonomy::PlaceholderGrammar;

#[test]
fn sp6_generation_emits_clean_filled_text() {
    // ARRANGE: configure a ~1000-JE generation with industry_profile.priors
    // enabled for "health" (priors.enabled = true). Reuse the config-building
    // pattern from fraud_bias_smoke.rs / the runtime test harness.
    let entries = run_small_priors_enabled_generation(); // helper — see note

    assert!(!entries.is_empty(), "generation produced no entries");

    let mut lines_total = 0usize;
    let mut lines_with_text = 0usize;
    for je in &entries {
        if let Some(ht) = &je.header.header_text {
            assert!(!ht.contains('{'), "header_text has literal placeholder: {ht:?}");
            assert!(
                PlaceholderGrammar::residual_pii_scan(ht).is_empty(),
                "header_text residual PII: {ht:?}"
            );
        }
        for line in &je.lines {
            lines_total += 1;
            if let Some(lt) = &line.line_text {
                lines_with_text += 1;
                assert!(!lt.contains('{'), "line_text has literal placeholder: {lt:?}");
                assert!(
                    PlaceholderGrammar::residual_pii_scan(lt).is_empty(),
                    "line_text residual PII: {lt:?}"
                );
            }
        }
    }
    let coverage = lines_with_text as f64 / lines_total.max(1) as f64;
    assert!(
        coverage >= 0.95,
        "line_text coverage {coverage:.3} < 0.95 ({lines_with_text}/{lines_total})"
    );
}
```

**Helper note:** `run_small_priors_enabled_generation` is not a real function — replace it with the actual generation invocation. Read `fraud_bias_smoke.rs` and copy its config + `EnhancedOrchestrator` (or `JeGenerator`) setup, setting `industry_profile.priors.enabled = true` and `industry_profile.industry = "health"`, ~1000 entries, seed 42. Access the emitted `JournalEntry` vector the same way that test does. If the committed health bundle has no `text_taxonomy` yet (pre-T16), the generation falls back to the `DescriptionGenerator` — the `{…}` and residual-PII assertions still hold (the fallback emits clean text), and the coverage assertion holds (the fallback always populates `line_text`). After T16 the test additionally exercises the real taxonomy path.

- [ ] **Step 2: Run it**

Run: `cargo test -p datasynth-runtime --test sp6_text_taxonomy_smoke 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/datasynth-runtime/tests/sp6_text_taxonomy_smoke.rs
git commit -m "test(sp6): integration smoke — no placeholder leaks, no residual PII

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 15: `regenerate-industry-priors.sh` — `--pii-denylist` + build-time audit gate

**Files:**
- Modify: `scripts/regenerate-industry-priors.sh`
- Modify: the fingerprint CLI extraction entry point (to accept `--pii-denylist`)

- [ ] **Step 1: Add the CLI flag**

Find the fingerprint extraction CLI command (grep for the subcommand that calls into the per-client extraction path / `aggregate-industry`):
```bash
grep -rn "aggregate-industry\|extract.*behavioral\|fingerprint" crates/datasynth-cli/src --include=*.rs | head
```
Add an optional `--pii-denylist <PATH>` argument to that subcommand. When present, `PiiDenylist::load(path)?` and thread `Some(&denylist)` into the extraction entry function (the `denylist` parameter added in T8 Step 5). When absent, pass `None` and print a warning to stderr: `warning: --pii-denylist not supplied; Phase B (fuzzy proper-noun generalization) skipped`.

- [ ] **Step 2: Add the build-time audit gate to the script**

Read `scripts/regenerate-industry-priors.sh`. After the bundle-build step, add a gate that loads each freshly-built bundle and runs the residual scan. The simplest robust form: a `cargo test` invocation of the T13 audit against the freshly-written bundles (they are written to `crates/datasynth-generators/resources/priors/`, exactly where `bundled_priors_path` looks):

```bash
# SP6 — build-time residual-PII gate. The freshly regenerated bundles must
# carry zero residual PII before they are considered valid.
echo "==> SP6 residual-PII audit on regenerated bundles"
if ! cargo test -p datasynth-runtime --test bundle_pii_audit -- --nocapture; then
    echo "ERROR: regenerated bundles failed the residual-PII audit — aborting." >&2
    exit 1
fi
```

Also: if the script accepts arguments, add a `--pii-denylist <path>` pass-through that is forwarded to the CLI extraction command. Document it in the script's usage/header comment.

- [ ] **Step 3: Verify the script parses + the CLI flag exists**

Run: `bash -n scripts/regenerate-industry-priors.sh && echo "script syntax OK"`
Run: `cargo run -p datasynth-cli -- <extract-subcommand> --help 2>&1 | grep -i denylist`
Expected: the `--pii-denylist` flag appears in the help output.

Run: `cargo build --workspace 2>&1 | tail -3`
Expected: `Finished`.

- [ ] **Step 4: Commit**

```bash
git add scripts/regenerate-industry-priors.sh crates/datasynth-cli/src
git commit -m "feat(sp6): regen script --pii-denylist passthrough + build-time audit gate

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 16: Regenerate bundles, baseline, CHANGELOG *(main session — not a subagent)*

**This task runs in the main session, not a dispatched subagent** — it requires the corpus and the curated PII denylist, both at private paths the subagents cannot access.

**Files:**
- Modify: `crates/datasynth-generators/resources/priors/industry_priors_*.dsf` (all 5, regenerated)
- Modify: `CHANGELOG.md`
- Create: `docs/baselines/2026-05-14-v5.27.0/SUMMARY.md`

- [ ] **Step 1: Regenerate the bundles with the denylist**

Run `scripts/regenerate-industry-priors.sh --pii-denylist <private-denylist-path>` (the curated denylist produced from the first-pass cleaning review). The build-time audit gate (T15) must pass — if it fails, it names the offending template; add the missed shape to the denylist or to `PlaceholderGrammar` Phase A, and re-run.

- [ ] **Step 2: Verify the CI audit passes on the regenerated bundles**

Run: `cargo test -p datasynth-runtime --test bundle_pii_audit 2>&1 | tail -10`
Expected: PASS — `checked 5 bundle(s)`, no skips, no failures.

- [ ] **Step 3: Run the integration smoke against real taxonomy bundles**

Run: `cargo test -p datasynth-runtime --test sp6_text_taxonomy_smoke 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 4: Behavioral-fidelity baseline**

Run the behavioral-fidelity scorer for the `gl-source-tp` profile (the v5.26 baseline procedure). Record mean / median / volume-corrected. Acceptance: each within ±1× of v5.26 (42.2 / 18.3 / 38.8). Investigate any movement >1× before accepting — text is not a P1–P4 signal, so the composite should not move.

- [ ] **Step 5: Write the baseline SUMMARY + CHANGELOG entry**

Create `docs/baselines/2026-05-14-v5.27.0/SUMMARY.md` (follow the v5.26 SUMMARY structure; reference the corpus only vaguely per the legal constraint). Add a `## [5.27.0]` entry to `CHANGELOG.md` describing SP6: PII-safe placeholder grammar, `(source × account-class)` line-text coherence, two-gate audit, `TextTemplate*` removed.

- [ ] **Step 6: Run the full workspace test + clippy gate**

Run: `cargo build --workspace --tests 2>&1 | tail -3`
Run: `cargo clippy --workspace 2>&1 | tail -5`
Expected: `Finished`; no warnings except the known `protoc not found` from `datasynth-server`.

- [ ] **Step 7: Commit**

```bash
git add crates/datasynth-generators/resources/priors/ CHANGELOG.md docs/baselines/
git commit -m "baseline(v5.27): SP6 — text taxonomy regen + PII-safe bundles

5 industry bundles regenerated with the curated PII denylist; CI
bundle_pii_audit green on all 5; BF composite within +-1x of v5.26.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 8: Open the PR**

```bash
git push -u origin sp6-text-taxonomy
gh pr create --title "SP6 — corpus text taxonomy + PII-safe placeholder grammar" \
  --body "$(cat <<'EOF'
Implements `docs/superpowers/specs/2026-05-14-sp6-text-taxonomy-design.md` (stages 1+2).

- PII-safe placeholder grammar (two-phase tokenize, fill via resolver, residual-PII scan)
- `(source × ISO-21378-account-class)` line-text coherence keying
- CoA descriptions templated + filled once per account
- Two-gate privacy model: build-time audit + CI `bundle_pii_audit`
- SP4.4 `TextTemplate*` path removed; all 5 bundles regenerated
- BF composite within +-1x of v5.26 (text is not a P1-P4 signal)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review

**1. Spec coverage:**

| Spec section | Plan task(s) |
|--------------|--------------|
| §2.1.1 `PlaceholderGrammar` (tokenize/fill/scan) | T2, T3, T4 |
| §2.1.1 `PlaceholderResolver` + `SyntheticExampleResolver` | T1 |
| §2.1.2 `TextTaxonomyPrior` / `TemplatePool` / `TemplateEntry` / `TaxonomyMeta` | T1 |
| §2.1.2 `synthetic_example` (not verbatim) | T6 (`make_entry`), T13 (audit) |
| §2.1.3 `PiiDenylist` | T5 |
| §2.2 extraction data flow (two-phase, `(source,class)` grouping, inline scan) | T6, T8 |
| §2.2 generation data flow (cascade, resolver, CoA fill-once) | T9, T10, T11 |
| §2.3 gate 1 — build-time audit | T15 |
| §2.3 gate 2 — CI `bundle_pii_audit` | T13 |
| §3 `behavioral_priors.rs` field swap (add then remove) | T1 (add), T12 (remove) |
| §3 re-export updates | T8, T12 |
| §4 error handling (denylist absent/malformed, cascade, old bundle) | T5, T6, T9, T10 |
| §5.1 unit tests | T1–T7 (each task's test step) |
| §5.2 generator consumption tests | T9, T10, T11 |
| §5.3 integration smoke | T14 |
| §5.4 CI bundle audit | T13 |
| §5.5 baseline | T16 |
| §6 acceptance criteria | T16 (full gate) |
| §7 W1–W5 | T1–T4 / T5–T8 / T9–T11 / T12–T15 / T16 |

All spec sections map to a task. No gaps.

**2. Placeholder scan:** No "TBD"/"TODO". Three places hand off detail to the engineer with explicit "read these N lines, adapt these names" instructions rather than vague directions — T8 Step 5 (extraction call-site variable names), T10 Step 4 (master-data accessors), T11 Steps 1+4 (`ChartOfAccounts` API). These are unavoidable: the exact local variable names at those integration points are not knowable from the spec, and the instruction names the exact anchor function to read. All *new* code (types, grammar, denylist, extractor functions, samplers, resolver, tests, audit) is complete and literal.

**3. Type consistency:** `TextTaxonomyPrior` / `TemplatePool` / `TemplateEntry` / `TaxonomyMeta` / `PiiPlaceholderKind` / `PiiHit` / `PlaceholderResolver` / `SyntheticExampleResolver` / `PlaceholderGrammar` — names identical across T1→T16. `line_key` / `UNKNOWN_CLASS` consistent. `sample_line_template(source, account_class, resolver, rng)` — 4-arg form defined in T9, called with 4 args in T10, old 2-arg form renamed `sample_line_template_legacy` in T9 and removed in T12. `sample_header_template_tx` in T9/T10, renamed to `sample_header_template` in T12. `extract_text_taxonomy` / `extract_text_taxonomy_checked` / `TextTaxonomyRecord` consistent T6→T8. `aggregate_text_taxonomy` consistent T7→T8. `overlay_coa_taxonomy` consistent T11. `MasterDataResolver` fields (`companies`/`persons`/`streets`/`patients`) consistent T10. `FingerprintError::PiiDenylist` used in T5/T6 — T5 Step 1 note instructs adding the variant if absent.

**Fix applied during review:** T6 Step 3's `make_entry` originally used an invalid Rust literal (`0x5P6_u64`). Fixed inline — the seed is now a clean per-template fold over `0x5036_u64`, deterministic and byte-stable across regens. No engineer adaptation needed there.

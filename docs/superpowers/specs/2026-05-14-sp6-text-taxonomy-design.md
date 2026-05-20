# SP6 — corpus Text Taxonomy & PII-Safe Placeholder Grammar — Design Spec

**Date:** 2026-05-14
**Status:** Draft (post-brainstorming, autonomous-execution-approved, subagent-driven)
**Scope:** Stages 1+2 of the text-taxonomy initiative — a PII-safe placeholder grammar (stage 1) and `(source × account-class)` coherence keying for line text (stage 2). Stage 3 (corpus-mined pattern-type taxonomy with generative template synthesis) is explicitly deferred to a follow-up spec.
**Predecessor:** [v5.26 baseline](../../baselines/2026-05-13-v5.26.0/SUMMARY.md). SP4 (`2026-05-13-sp4-corpus-grounding-design.md`) shipped `TextTemplatePrior` (SP4.4) as verbatim source-keyed string extraction. SP6 replaces it.
**Branch:** `sp6-text-taxonomy`

## 1. Why

SP4.4 extracts header/line text verbatim from the corpus into per-source string pools, gated only by a frequency floor (`min_occurrences = 10`). This has two structural defects:

1. **PII leaks.** The frequency floor does not stop *recurring* identifiable content. A first-pass cleaning sweep of the shipped bundles found, in the health bundle alone, 197 patient records (`*Name,First G:dd.dd.dd E:… A:…`), person names, street addresses, and company/institution proper nouns — all of which clear 10 occurrences. The `TextTemplate.example` field independently ships a verbatim corpus string per template. Verbatim extraction cannot simultaneously be corpus-grounded, PII-safe, and legally vague — the three requirements are in structural tension, and the cleaning pass is manual labour patching a design that leaks by construction.

2. **No account coherence.** Text templates key on the SAP source code only. The generator samples a line's GL account (via SP3.7 `per_source_attribute` / SP4.6 `per_source_role`) and its line text (via `sample_line_template`) as *independent* draws conditioned on the same source — `P(account, line_text | source) = P(account|source) × P(line_text|source)`. A "medicine purchase" line text under source `RE` can land on any account `RE`'s conditional draws, not a medical-supplies account. This is the SP3.6→SP3.7 problem one layer up: vocabulary correct, joint structure not modelled.

SP6 resolves both at the root. Generated text is **synthetic by construction** — tokenized real templates with PII spans as fillable placeholders — and line text is **conditioned on `(source, account-class)`** so it lands in a coherent account context.

### 1.1 What SP6 is not

- **Not a behavioral-fidelity change.** The P1–P4 metrics measure timing / fanout / clustering, not text content. The composite BF score (v5.26: 42.2 mean / 18.3 median / 38.8 vol-corrected) should not move. SP6 is a *semantic + legal + downstream-data-quality* deliverable, judged on different criteria. A no-regression gate confirms the composite holds.
- **Not stage 3.** SP6 *replays tokenized real templates*. It does not *synthesize* new template structures. Clustering templates into pattern-types ("counterparty-name pattern", "rent-payment pattern", …) and a generative grammar that produces novel templates is stage 3, deferred to its own spec — that part carries genuine "templated/AI-slop" risk and benefits from being designed after stages 1+2 land.
- **Not the HF dataset refresh.** The public 1M-JE dataset regeneration *consumes* SP6's output and is sequenced after it.
- **Not OCEL / process text.** Extending corpus grounding to OCEL event logs is a separate roadmap item.
- **Not formal differential privacy.** The privacy model here is: automated structural placeholder-ization + curated denylist + frequency floor + a two-gate residual-PII audit. A formal DP proof over the vocabulary is a separate concern.

### 1.2 Decisions locked in brainstorming

| # | Decision | Choice |
|---|----------|--------|
| 1 | Spec scope | Stages 1+2 (placeholder grammar + `(source, account-class)` keying). Stage 3 deferred. |
| 2 | PII generalization model | Hybrid — automated structural placeholder-ization + a human-curated denylist for fuzzy proper nouns. |
| 3 | Header-text keying | Line text → `(source, account-class)`; header text → `source` only (a JE header has no single account). |
| 4 | Placeholder fill source | Reuse the run's synthetic master data (`{company}`←vendors/customers, `{person}`←employees, `{street}`←address generator); a locale-matched synthetic pool for placeholder types with no master entity (`{patient}`). |
| 5 | CoA descriptions | Templated + filled at CoA-generation time, **once per account** (stable within a run, locale-matched, uniform with header/line text). |
| 6 | Denylist location | A private path outside the repo (alongside the corpus, which CI cannot access). Extractor takes `--pii-denylist <path>`; absent → automated-structural-only. |

## 2. Architecture

### 2.1 New module: `crates/datasynth-core/src/distributions/text_taxonomy.rs`

Two cleanly-separated concerns in one module.

#### 2.1.1 `PlaceholderGrammar` — tokenize + fill + scan

The placeholder vocabulary:

| Placeholder | Kind | Filled with |
|-------------|------|-------------|
| `{year}` | structural | year in a configured range |
| `{quarter}` | structural | `Q1`–`Q4` |
| `{month}` | structural | month name (locale-aware) |
| `{date}` | structural | `dd.mm.yy` within the run's fiscal window |
| `{digits}` | structural | 4–8 random digits |
| `{patient}` | PII | synthetic locale-matched person name (no master entity) |
| `{person}` | PII | a name from the run's employee master |
| `{company}` | PII | a name from the run's vendor/customer master |
| `{street}` | PII | an address from the run's address generator |

API:

```rust
/// Stateless tokenize/fill/scan engine. No dependency on generator or
/// fingerprint crates — locale + master-data wiring arrives via the resolver.
pub struct PlaceholderGrammar;

impl PlaceholderGrammar {
    /// Raw corpus string → PII-safe template. Two phases:
    ///   Phase A — automated structural placeholder-ization (regex-driven):
    ///     patient `G:`-records, `*Name,First` star-records, street addresses,
    ///     `dd.mm.yy` dates, ≥4-digit runs, years, quarters, months.
    ///   Phase B — curated denylist substitution: any denylisted proper-noun
    ///     span → its mapped placeholder. Skipped when `denylist` is `None`.
    pub fn tokenize(s: &str, denylist: Option<&PiiDenylist>) -> String;

    /// Template → concrete string. Structural placeholders are filled
    /// internally from `rng`; PII placeholders are delegated to `resolver`.
    pub fn fill<R: rand::Rng>(
        template: &str,
        resolver: &mut dyn PlaceholderResolver,
        rng: &mut R,
    ) -> String;

    /// Scan a string for residual PII patterns (the productionized
    /// first-pass self-check). Returns one hit per match; empty = clean.
    pub fn residual_pii_scan(s: &str) -> Vec<PiiHit>;
}

/// PII-placeholder kinds the generator must resolve. Structural placeholders
/// are NOT in this enum — the grammar fills those itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiiPlaceholderKind { Patient, Person, Company, Street }

/// Implemented by the generator. Maps a PII-placeholder kind to a concrete
/// value drawn from the run's master data or a locale pool. The grammar
/// module stays free of vendor/customer/employee types and of locale logic.
pub trait PlaceholderResolver {
    fn resolve(&mut self, kind: PiiPlaceholderKind, rng: &mut dyn rand::Rng) -> String;
}

#[derive(Debug, Clone, PartialEq)]
pub struct PiiHit { pub pattern: &'static str, pub matched: String }
```

Phase-A regex set is ported from the validated first-pass cleaning script (the v3 sweep that reached 0 residual PII), with the patient-name fix already proven (`^.*?(?=G:\d…)`, **not** `[^G]*?` — a negated-G class cannot consume names containing a G).

#### 2.1.2 `TextTaxonomyPrior` — the conditional pools

```rust
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TextTaxonomyPrior {
    /// Line text keyed on (source, account_class). Key is the flattened
    /// string "SOURCE|CLASS" (YAML-friendly). account_class is the ISO 21378
    /// Level-2 class resolved via CoaSemanticPrior at extraction time;
    /// lines whose account has no class are grouped under "SOURCE|_unknown_".
    pub line_pools: BTreeMap<String, TemplatePool>,
    /// Header text keyed on source only (a JE header has no single account).
    pub header_pools: BTreeMap<String, TemplatePool>,
    /// CoA description templates, keyed on account number. One entry per account.
    pub coa_pools: BTreeMap<String, TemplateEntry>,
    /// Extraction metadata: thresholds, tier, contributing-client count.
    pub meta: TaxonomyMeta,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TemplatePool {
    pub templates: Vec<TemplateEntry>,
    /// Total observations underpinning the pool (pre-truncation).
    pub n: usize,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TemplateEntry {
    /// Tokenized, PII-safe template string.
    pub template: String,
    /// Probability mass within the pool (frequency-filtered + renormalised).
    pub probability: f64,
    /// The template run through `fill` ONCE at extraction time, using a
    /// fixed-seed RNG and a built-in `SyntheticExampleResolver` that emits
    /// obviously-synthetic tokens (e.g. "Example Person", "Example GmbH") —
    /// master data does not exist at extraction time. A useful debug/audit
    /// example that contains ZERO corpus content. Replaces SP4.4's verbatim
    /// `example` field.
    pub synthetic_example: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TaxonomyMeta {
    pub min_occurrences: usize,
    pub max_templates_per_pool: usize,
    /// "iso21378_l2" — recorded for forward-compat if stage 3 changes the tier.
    pub class_tier: String,
    pub n_client_inputs: usize,
}
```

Account-class granularity: **ISO 21378 Level-2** (`AccountSemantic.account_class`). ~10–15 classes × ~50 sources ≈ 500–750 line-pool cells over ~1.4M corpus lines — comfortably dense. Level-2 is fixed for SP6; the `class_tier` meta field records it for forward compatibility. The generation-time lookup *cascade* (below) handles sparsity, not the extraction tier.

#### 2.1.3 `PiiDenylist`

```rust
// crates/datasynth-fingerprint/src/extraction/pii_denylist.rs
#[derive(Debug, Clone, Default)]
pub struct PiiDenylist {
    /// Exact-match: real proper noun → placeholder kind.
    pub exact: BTreeMap<String, PiiPlaceholderKind>,
    /// Regex family rules (e.g. r"\bKantonalbank\b" → Company).
    pub patterns: Vec<(regex::Regex, PiiPlaceholderKind)>,
}

impl PiiDenylist {
    /// Load from a TSV at a private path. Format per line:
    ///   <literal-or-/regex/>\t<patient|person|company|street>
    pub fn load(path: &std::path::Path) -> Result<Self, SynthError>;
    /// Apply to a (partially Phase-A-tokenized) string — Phase B.
    pub fn apply(&self, s: &str) -> String;
}
```

The denylist file is PII-derived (its left-hand side *is* real proper nouns) and **never enters the public repo**. It lives at a private path alongside the corpus. The extractor CLI gains `--pii-denylist <path>`; when omitted, only Phase A runs.

### 2.2 Data flow

**Extraction** (`text_extractor.rs`, rewritten):

```
corpus JE parquet + CoA parquet + [private] PiiDenylist
  │
  ├─ for each line: resolve GL account → ISO 21378 Level-2 class (via CoaSemanticPrior)
  ├─ tokenize every header_text / line_text / CoA description:
  │     Phase A  PlaceholderGrammar::tokenize(s, None)         — structural
  │     Phase B  denylist.apply(...)                           — fuzzy proper nouns
  ├─ group:  line texts  → (source, class)
  │          header texts → source
  │          CoA descrs   → account_no
  ├─ frequency filter (min_occurrences) + top-N per pool + renormalise
  ├─ residual_pii_scan EVERY retained template  ──► HARD FAIL on any hit
  └─ synthetic_example = PlaceholderGrammar::fill(template, synthetic_resolver, fixed_seed_rng)
  ↓
TextTaxonomyPrior  → aggregate across clients → bundled in .dsf
```

**Generation:**

```
LoadedPriors.text_taxonomy : Option<TextTaxonomyPrior>
  │
  ├─ je_generator, per line:
  │     account_number (already sampled, SP3.7/SP4.6)
  │       → resolve account_class via coa_semantic
  │       → lookup cascade:
  │            line_pools["SRC|CLASS"]
  │            → line_pools["SRC|_unknown_"]
  │            → header_pools["SRC"]
  │            → legacy description_generator
  │       → weighted-pick TemplateEntry
  │       → PlaceholderGrammar::fill(template, &mut master_data_resolver, rng)
  │
  ├─ je_generator, per header:
  │       header_pools["SRC"] → weighted-pick → fill
  │
  └─ coa_generator, per account:
        coa_pools[account_no] → fill ONCE (cached for the run — stable per account)
```

The `master_data_resolver` is a `PlaceholderResolver` impl owned by the generator, wired to the run's vendor/customer master (`{company}`), employee master (`{person}`), address generator (`{street}`), and a locale-selected synthetic-person pool (`{patient}`). Locale follows the synthetic entity automatically because master data is already locale-correct and the resolver picks the `{patient}` pool by entity locale — the grammar module stays locale-agnostic.

### 2.3 Privacy model — two gates

1. **Build-time audit** — `scripts/regenerate-industry-priors.sh` runs `residual_pii_scan` over every template in the freshly-built `TextTaxonomyPrior` (extraction already does this inline; the script makes it an explicit gate). Any hit aborts the regen and names the offending template.
2. **CI test** — `crates/datasynth-runtime/tests/bundle_pii_audit.rs` loads every *committed* `.dsf`, scans every `template` and every `synthetic_example` in `line_pools` / `header_pools` / `coa_pools`. Any hit fails CI. This runs with **no corpus access** — it only reads committed bundles — so it is the permanent safety net against a PII-bearing bundle ever being committed.

Bundle regeneration only happens where the corpus + denylist live (a developer machine, not CI). CI compiles, tests, and audits the *committed* bundles.

## 3. Files

**New:**
- `crates/datasynth-core/src/distributions/text_taxonomy.rs` — `PlaceholderGrammar`, `TextTaxonomyPrior`, `TemplatePool`, `TemplateEntry`, `TaxonomyMeta`, `PiiPlaceholderKind`, `PlaceholderResolver`, `SyntheticExampleResolver` (built-in resolver emitting obviously-fake tokens, used for `synthetic_example` at extraction + in tests), `PiiHit`.
- `crates/datasynth-fingerprint/src/extraction/pii_denylist.rs` — `PiiDenylist` load/apply.
- `crates/datasynth-runtime/tests/bundle_pii_audit.rs` — CI residual-PII gate over committed bundles.

**Rewritten:**
- `crates/datasynth-fingerprint/src/extraction/text_extractor.rs` — two-phase tokenize, `(source, class)` grouping via `CoaSemanticPrior`, inline residual scan, `synthetic_example` generation.
- `crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs` — aggregate `TextTaxonomyPrior` across clients (replaces the `TextTemplatePrior` aggregation path).

**Modified:**
- `crates/datasynth-core/src/distributions/behavioral_priors.rs` — remove `TextTemplatePrior`, `TextTemplate`, `fill_text_template_with_rng`; `BehavioralPriors.text_templates: Option<TextTemplatePrior>` → `text_taxonomy: Option<TextTaxonomyPrior>`.
- `crates/datasynth-generators/src/priors_loader.rs` — load `text_taxonomy`; `sample_line_template(src)` → `sample_line_template(src, account_class)`; `sample_header_template(src)` retained; new `sample_coa_description(account_no)`; the lookup cascade lives here.
- `crates/datasynth-generators/src/je_generator.rs` — resolve account_class per line, pass it into line-text sampling, construct and pass the `master_data_resolver`.
- `crates/datasynth-generators/src/coa_generator.rs` — fill CoA description templates once per account (cached for the run).
- `crates/datasynth-fingerprint/src/extraction/behavioral.rs` and `extraction/mod.rs` — update re-exports.
- `scripts/regenerate-industry-priors.sh` — accept + forward `--pii-denylist`; run the build-time audit gate.

## 4. Error handling

| Condition | Behaviour |
|-----------|-----------|
| `--pii-denylist` omitted | Phase A only; log a WARNING that fuzzy proper nouns are not generalized. Build-time audit still runs and will catch residual fuzzy PII. |
| `residual_pii_scan` hit at extraction | Hard error; abort the bundle build; error names the pool key and the offending template. |
| Denylist file malformed | Hard error at load; do not silently fall back to Phase-A-only. |
| Generation: account_class unresolvable | Lookup cascade → `(source, _unknown_)` → `header_pools[source]` → legacy `description_generator`. Never panic. |
| Old bundle (`text_templates`, no `text_taxonomy`) | `text_taxonomy` deserializes as `None`; generator uses the legacy `description_generator` path. (All 5 bundles are regenerated in W5 regardless.) |
| `PlaceholderResolver` returns empty | Grammar emits the literal placeholder token rather than an empty span; the integration smoke test asserts this does not happen for the standard placeholder set. |

## 5. Testing

### 5.1 Unit
- **`text_taxonomy.rs`** — `tokenize` produces the expected template for every PII shape (patient `G:`-record incl. G-containing names, `*Name,First`, street, date, digit run, year/quarter/month); `fill` fills every placeholder kind and leaves no `{…}` literal for the standard set; `residual_pii_scan` flags the known leak shapes (the `[^G]`-bug patient case, an `Initial. Surname`, a `*Name,First`) and passes clean templates; **round-trip** `tokenize → fill → residual_pii_scan` returns empty.
- **`pii_denylist.rs`** — load a TSV (exact + `/regex/` rows); `apply` substitutes exact and pattern matches; malformed file errors.
- **`text_extractor.rs`** — `(source, class)` grouping uses the CoA class; accounts with no class land under `_unknown_`; frequency filter + top-N; `synthetic_example` is not byte-equal to any input string.
- **`industry_aggregator.rs`** — N per-client `TextTaxonomyPrior` → aggregated prior; pool union + probability re-normalisation; `meta.n_client_inputs` correct.

### 5.2 Generator consumption
- Priors loaded → a line whose account resolves to class X draws from `line_pools["SRC|X"]`; an unknown-class account cascades; a CoA account's description is filled once and is byte-stable across repeated reads within a run.

### 5.3 Integration smoke — `crates/datasynth-runtime/tests/sp6_text_taxonomy_smoke.rs`
Small priors-enabled generation (~1000 JEs). Asserts: `line_text` populated for ≥95% of lines; **zero** `{…}` literal placeholder tokens in any emitted `line_text` / `header_text` / `account_description`; **zero** residual-PII-scan hits across all emitted text; CoA `account_description` filled and stable.

### 5.4 CI bundle audit — `bundle_pii_audit.rs`
Loads each committed `.dsf`; `residual_pii_scan` over every `template` and `synthetic_example` in all three pool maps; asserts zero hits. No corpus access required.

### 5.5 Baseline
A v5.27 baseline run after W5. Acceptance: composite BF mean/median/vol-corrected within ±1× of v5.26 (42.2 / 18.3 / 38.8) — text is not a P1–P4 signal, so the composite must not move materially. Any movement >1× is investigated before the baseline is accepted.

## 6. Acceptance criteria

| Criterion | Pass condition |
|-----------|----------------|
| PII-safe by construction | CI `bundle_pii_audit` = 0 hits on all 5 committed bundles. |
| No verbatim corpus content | `TemplateEntry` has no verbatim field; `synthetic_example` ≠ any input string (extractor unit test). |
| Line-text coherence | A line's text is drawn from a pool keyed on its own account's ISO 21378 class (generator consumption test). |
| CoA descriptions PII-safe + stable | CoA `account_description` filled from templates, byte-stable per account per run, 0 scan hits. |
| Behavioral fidelity preserved | v5.27 composite within ±1× of v5.26 on all three composites. |
| Graceful fallback | Generation with `text_taxonomy = None` falls back to `description_generator` with no panic. |
| Build-time gate | `regenerate-industry-priors.sh` aborts on any residual-PII hit. |

## 7. Execution — subagent-driven waves

Each wave is one subagent task = one focused commit + its tests, dispatched via `superpowers:subagent-driven-development`. Waves are serial — each builds on the previous one's types/APIs.

- **W1 — core module.** `text_taxonomy.rs`: `PlaceholderGrammar` (tokenize + fill + scan), the prior structs, `PlaceholderResolver` trait, `PiiPlaceholderKind`, `PiiHit`. Pure `datasynth-core`; no dependency on other crates. Fully unit-tested (§5.1 first bullet). **Gate:** `cargo test -p datasynth-core --lib text_taxonomy` green.
- **W2 — extraction side.** `pii_denylist.rs`; rewrite `text_extractor.rs` (two-phase tokenize, `(source,class)` grouping, inline scan, `synthetic_example`); aggregation in `industry_aggregator.rs`; remove `TextTemplatePrior`/`TextTemplate`/`fill_text_template_with_rng` from `behavioral_priors.rs` and swap `BehavioralPriors` field. Update `behavioral.rs` / `mod.rs` re-exports. **Gate:** `cargo test -p datasynth-fingerprint --lib` green; `cargo build --workspace` green.
- **W3 — generator wiring.** `priors_loader.rs` (load `text_taxonomy`, lookup cascade, new sampler signatures); `je_generator.rs` (resolve class per line, pass it, construct the `master_data_resolver`); `coa_generator.rs` (fill once per account). **Gate:** `cargo test -p datasynth-generators --lib` green; generator consumption test (§5.2) green.
- **W4 — gates + smoke.** `bundle_pii_audit.rs`; `sp6_text_taxonomy_smoke.rs`; `regenerate-industry-priors.sh` `--pii-denylist` + build-time audit. **Gate:** `cargo build --workspace --tests` green; new integration tests green; `cargo clippy --workspace` clean.
- **W5 — bundles + baseline** *(main session — needs the corpus + the curated denylist, not a subagent)*. Regenerate all 5 bundles with `--pii-denylist`; build-time audit passes; CI `bundle_pii_audit` passes on the regenerated bundles; v5.27 BF baseline; CHANGELOG; commit. **Gate:** all of §6.

Parallelism: none across waves (strict dependency chain). Within W2, the `behavioral_priors.rs` field swap should land first inside the wave so the rest of the crate compiles against the new type.

## 8. Risks and mitigations

| Risk | Mitigation |
|------|------------|
| Phase-A regexes miss a PII shape not seen in the first-pass sweep | Two-gate audit catches it before commit; the build-time gate is a hard fail. Add the missed shape to the Phase-A set + a unit test when found. |
| Denylist drifts out of date as the corpus changes | `--pii-denylist` is explicit per regen; `meta.n_client_inputs` + the audit gate surface coverage gaps. Denylist maintenance is a documented step in the regen runbook. |
| `(source, class)` cells too sparse for some pairs | Extraction frequency filter drops thin cells; generation cascade falls back to `(source, _unknown_)` then source-marginal then legacy. No cell is required to exist. |
| Removing `TextTemplatePrior` breaks an unknown consumer | W2 gate is a full `cargo build --workspace`; `grep` for `TextTemplate` / `text_templates` across the workspace before the field swap. |
| Composite BF moves unexpectedly | §5.5 no-regression gate; >1× movement is investigated before the v5.27 baseline is accepted. |
| Master-data resolver not available at a fill site (e.g. CoA gen runs before master data) | CoA fill uses the same resolver; W3 ensures master data is constructed before CoA + JE generation, or the resolver lazily initialises its pools. Verified by the generator consumption test. |
| Bundle size growth from `(source,class)` fan-out | Top-N per pool caps each cell; `synthetic_example` is one short string. Expected net change small; `.dsf` is compressed-on-write. |

## 9. Self-review map

| Spec section | Wave |
|--------------|------|
| §2.1 `text_taxonomy.rs` core module | W1 |
| §2.1.3 `PiiDenylist` + §2.2 extraction + behavioral_priors swap | W2 |
| §2.2 generation wiring | W3 |
| §2.3 two-gate privacy model + §5.3 smoke | W4 |
| §5.5 baseline + bundle regen | W5 |
| §5 testing | each wave's gate |
| §6 acceptance | W5 gate (full) |

All six locked decisions (§1.2) are reflected: stages 1+2 only (§1.1 defers stage 3); hybrid PII model (§2.1.1 Phase A/B, §2.1.3); header→source / line→(source,class) (§2.1.2); master-data fill (§2.2 resolver); CoA templated+filled-once (§2.2); private denylist path (§2.1.3, §3 `--pii-denylist`). The legal posture is strengthened, not just preserved: output text is synthetic-by-construction, and the CI audit gate makes a PII-bearing bundle un-committable.

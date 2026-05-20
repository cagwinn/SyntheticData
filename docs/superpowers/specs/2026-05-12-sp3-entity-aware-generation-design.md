# SP3 — Entity-Aware Generation — Design Spec

**Date:** 2026-05-12
**Status:** Draft (post-brainstorming, user-approved scope B)
**Sub-project:** SP3 of the broader Behavioral-Fidelity initiative ([SP1 ✅][sp1] · [SP2 ✅][sp2] · SP3 here · SP4 last)
**Target release:** v5.12 (additive; new behavior strictly opt-in via config)
**Predecessors:**
- [SP1 baseline][baseline] — composite BF 59.0× on real-JE_3, with five concrete generator-side fix targets
- [SP2 bundles][priors] — five industry-priors `.dsf` files committed at `crates/datasynth-generators/resources/priors/`

[sp1]: 2026-05-11-sp1-behavioral-fidelity-design.md
[sp2]: 2026-05-12-sp2-real-world-prior-extraction-design.md
[baseline]: ../../baselines/2026-05-12-sp1-v5.10.0/SUMMARY.md
[priors]: ../../real-world-priors.md

## 1. Overview

SP1 measured the gap. SP2 mined the priors that close five of the six biggest baseline gaps. SP3 wires those priors into the generators so the *runtime* output matches the *real* corpus on the behavioral signals the Sajja paper proves row-independent tabular generators cannot reproduce.

The rewire is **strictly opt-in** via a new config sub-section. When unset, every generator behaves identically to v5.11 — no behavioral or numerical regression on the ~2000 existing tests, no surprise behavior in existing user configs. When the user opts in by naming an industry and enabling priors, the generator loads the matching `industry_priors_{industry}.dsf` once at init and routes its RNG through five new entity-aware samplers.

The user-flagged Trading Partner schema gap also lands here: `journal_entries.csv` gets a new `trading_partner` column that's always emitted (additive — empty string for non-doc-linked rows). This is the only non-opt-in change.

### 1.1 Primary use case

A `config.yaml` containing:

```yaml
industry_profile:
  name: health
  priors:
    enabled: true
    source: bundled        # default: load from crates/datasynth-generators/resources/priors/
```

… run via `datasynth-data generate --config config.yaml --output ./out`, produces `out/journal_entries.csv` where (compared to v5.11 with the same seed and other config) the lines-per-JE distribution matches `industry_priors_health.dsf`, per-Source IETs match the prior's empirical CDFs with realistic burst regularity, attribute fan-out distributions track the corpus motif structure, each Source's active window is sampled from the real distribution, and the new `trading_partner` column carries `vendor_id` / `customer_id` for P2P/O2C-derived lines.

### 1.2 Goals

- Implement three new samplers in `datasynth-core::distributions`: `ConditionalIETSampler`, `BipartiteFanoutSampler`, `SourceActiveWindow`.
- Implement `LoadedPriors` in `datasynth-generators::priors_loader` — one-time deserialisation of `BehavioralPriors` plus per-Source lookup tables for the hot path.
- Rewire `je_generator.rs` to use the new samplers when priors are loaded; preserve existing code paths when priors are disabled.
- Rewire `p2p_generator.rs` and `o2c_generator.rs` to propagate `vendor_id` / `customer_id` to the new `trading_partner` field on `JournalEntry`.
- Add a `trading_partner: Option<String>` field to the `JournalEntry` core model.
- Add `industry_profile.priors` sub-section to `datasynth-config::schema`.
- Append a `trading_partner` column to `journal_entries.csv` output (always emitted).
- Ship an integration smoke test confirming priors-driven generation produces distributions that match the prior, and a backward-compat test confirming `priors.enabled: false` produces output identical to a pre-SP3 baseline.
- Zero changes to ACDOCA output (no `trading_partner` column added there — keep ACDOCA schema stable).
- Document the runtime behavior in `docs/entity-aware-generation.md` and cross-link from `docs/behavioral-fidelity.md` and `docs/real-world-priors.md`.

### 1.3 Non-goals (deferred)

- **Velocity-rule calibration loss term** (P4 4.5× gap). The training-time objective that drives generator parameters to match corpus rule-trigger rates is more research than engineering — deferred to **SP3.1** with its own spec. The SP1 baseline already shows P4 is the least-bad metric (4.5×), so deferring it doesn't hurt the showcase narrative.
- **Generation-time TP enforcement on graph motifs.** SP3 propagates TP from existing doc linkage; we don't (yet) drive synthetic vendors/customers to share GL accounts the way real fraud rings share devices. That would be a SP3.2.
- **SP4 — Showcase release.** HF dataset + Gradio Space + paper writeup. Once SP3 lands, we re-run the SP1 scorer with priors enabled and capture the improvement as the v5.10 → v5.12 delta.
- **Per-domain priors for banking / OCEL / payroll.** SP3 consumes GL-domain priors only. Other domains can mirror the pattern when their behavioral-fidelity work begins.

## 2. Architecture

### 2.1 Crate placement

```
crates/datasynth-core/src/distributions/
├── conditional_iet.rs         [new]   ConditionalIETSampler
├── fanout_sampler.rs          [new]   BipartiteFanoutSampler
└── source_active_window.rs    [new]   SourceActiveWindow

crates/datasynth-core/src/models/
└── journal_entry.rs           [modify] add `trading_partner: Option<String>`

crates/datasynth-config/src/
└── schema.rs                  [modify] industry_profile.priors sub-section

crates/datasynth-generators/src/
├── priors_loader.rs           [new]   LoadedPriors + load_from_config helper
├── je_generator.rs            [modify] use new samplers when priors loaded
└── document_flow/
    ├── p2p_generator.rs       [modify] propagate vendor_id → JE trading_partner
    └── o2c_generator.rs       [modify] propagate customer_id → JE trading_partner

crates/datasynth-output/src/
└── csv_writer.rs              [modify] append trading_partner column

crates/datasynth-generators/tests/                 [new tests]
├── sp3_priors_smoke.rs
└── sp3_backward_compat.rs

docs/
└── entity-aware-generation.md [new]

CHANGELOG.md, README.md, CLAUDE.md, .github/workflows/ci.yml  [small touch-ups]
```

The new samplers live in `datasynth-core::distributions` next to `amount.rs`, `benford.rs`, `temporal.rs` etc. — that's the established home for RNG-driven samplers. The bundle-loading sits in `datasynth-generators` because that's where the existing `industry_profile` runtime configuration is consumed.

### 2.2 Sampler interfaces

```rust
// crates/datasynth-core/src/distributions/conditional_iet.rs
pub struct ConditionalIETSampler {
    per_source: HashMap<String, SourceIetState>,
    fallback: SourceIetState,
}
pub struct SourceIetState {
    cdf_values: Vec<f64>,        // 256 quantile knots from the prior
    cdf_probabilities: Vec<f64>, // monotone increasing
    lag1_autocorr: f64,
    last_iet_days: Option<f64>,  // mutable: last sample for autocorr coupling
}

impl ConditionalIETSampler {
    pub fn from_prior(prior: &PerSourceIetPrior) -> Self;
    /// Sample the next IET in days for `source`. Uses lag-1 autocorr coupling
    /// when a previous sample exists.
    pub fn sample_next<R: Rng>(&mut self, source: &str, rng: &mut R) -> f64;
}
```

```rust
// crates/datasynth-core/src/distributions/fanout_sampler.rs
pub struct BipartiteFanoutSampler {
    /// For one attribute (e.g. "GLAccount"), a pool of attribute values
    /// pre-sized to match the prior's fan-out histogram.
    pub buckets: Vec<AttributeBucket>,
}
pub struct AttributeBucket {
    pub target_fanout: u32,           // how many distinct entities should touch this value
    pub current_users: HashSet<String>, // running set of entities that have used this value
    pub attribute_value: String,        // an opaque ID (e.g. synthesized account #)
}

impl BipartiteFanoutSampler {
    pub fn from_prior(prior: &FanoutHistogram, n_values: usize, value_gen: impl Fn(usize) -> String) -> Self;
    /// Pick an attribute value to assign to `entity_id`. Prefers buckets whose
    /// `current_users.len() < target_fanout` to honour the prior.
    pub fn pick_for<R: Rng>(&mut self, entity_id: &str, rng: &mut R) -> &str;
}
```

```rust
// crates/datasynth-core/src/distributions/source_active_window.rs
pub struct SourceActiveWindow {
    /// For each Source code, a sampled active window (start_day, end_day) in
    /// days-since-period-start.
    pub by_source: HashMap<String, ActiveWindow>,
}
pub struct ActiveWindow { pub start_day: i64, pub end_day: i64 }

impl SourceActiveWindow {
    pub fn from_prior(prior: &ActiveLifetimePrior, period_days: i64, rng: &mut impl Rng) -> Self;
    /// `true` if `source` is allowed to emit on `day` within the period.
    pub fn is_active(&self, source: &str, day: i64) -> bool;
}
```

All three samplers are deterministic given a seeded RNG and the same prior — round-trip-reproducible.

**`LinesPerJeSampler` is intentionally minimal** and lives inside `priors_loader.rs` rather than `distributions/` — it's a thin function around the existing `LineCountHistogram::sample_bucket(rng)` helper (which we add to `datasynth-fingerprint::models::behavioral` in Phase A). Signature:

```rust
fn sample_lines_for_je<R: Rng>(prior: &LinesPerJePrior, source: &str, rng: &mut R) -> u32 {
    // Prefer per-source histogram if available, else overall.
    let hist = prior.by_source.get(source).unwrap_or(&prior.overall);
    hist.sample_bucket(rng)
}
```

`LineCountHistogram::sample_bucket` picks a bucket by probability mass then samples uniformly inside the bucket — added in Phase A's first task.

### 2.3 LoadedPriors

```rust
// crates/datasynth-generators/src/priors_loader.rs
pub struct LoadedPriors {
    pub industry: String,
    pub bundle_path: PathBuf,
    pub source_mix: SourceMixPrior,
    pub iet_sampler: ConditionalIETSampler,
    pub lines_per_je: LinesPerJePrior,
    pub active_window: SourceActiveWindow,
    pub fanout_samplers: HashMap<String, BipartiteFanoutSampler>, // by attribute name
    pub posting_lag: Option<PostingLagPrior>,
}

impl LoadedPriors {
    /// Load the bundled prior for `industry` from
    /// `crates/datasynth-generators/resources/priors/industry_priors_{industry}.dsf`.
    pub fn load_bundled(industry: &str, rng: &mut impl Rng, period_days: i64)
        -> Result<Self, PriorsLoadError>;

    /// Load from an explicit path.
    pub fn load_from_path(path: &Path, rng: &mut impl Rng, period_days: i64)
        -> Result<Self, PriorsLoadError>;
}

#[derive(Debug, thiserror::Error)]
pub enum PriorsLoadError {
    #[error("priors bundle not found at {0}")]
    NotFound(PathBuf),
    #[error("bundle has no behavioral section")]
    MissingBehavioral,
    #[error("bundle industry mismatch: bundle={bundle}, requested={requested}")]
    IndustryMismatch { bundle: String, requested: String },
    #[error("read error: {0}")]
    Io(#[from] datasynth_fingerprint::FingerprintError),
}
```

`LoadedPriors` is constructed once per generator run (during `EnhancedOrchestrator` init) and threaded through to `je_generator::generate_journal_entry` via the existing context object.

### 2.4 Generator rewires

`je_generator.rs` is large (3502 LOC); we minimise the surgery:

- **IET timing.** Find the existing posting-time loop (Poisson-distributed per-day count). When `LoadedPriors` is present in context, replace the inner loop's day-gap draw with `iet_sampler.sample_next(source, rng)`. The outer loop structure is preserved.
- **Lines-per-JE.** Find the existing `lines_per_entry` config-driven draw. Replace with `lines_per_je_sampler.sample(rng)` (a small wrapper around the histogram) when priors are present.
- **Fan-out for GLAccount / CostCenter / ProfitCenter.** Find the existing random-from-config-list draws. Replace with `fanout_samplers["GLAccount"].pick_for(source, rng)` (and similarly for CC / PC).
- **Source-active gating.** Wrap the per-day emission decision with `active_window.is_active(source, day) || continue;`.
- **No structural refactor of je_generator.** All existing helper functions stay; we add an `if let Some(priors) = &ctx.loaded_priors` branch at four call sites.

`p2p_generator.rs` and `o2c_generator.rs` already know the `vendor_id` / `customer_id` when they emit JE lines — they just need to pass it through to the new `JournalEntry::trading_partner` field.

### 2.5 Config schema

```yaml
industry_profile:
  name: health                # existing field
  priors:                     # NEW sub-section
    enabled: false            # default: false (opt-in)
    source: bundled           # enum: bundled | file
    path: ~                   # required when source: file
```

`source: bundled` resolves to `crates/datasynth-generators/resources/priors/industry_priors_{industry_profile.name}.dsf`. `source: file` takes the explicit `path`. Unknown industries with `source: bundled` fail config validation with a clear error listing the five available bundles.

### 2.6 Output schema

The CSV writer for `journal_entries.csv` appends one column after the existing columns:

```
… cost_center, profit_center, line_text, …, predecessor_line_id, trading_partner
```

Position: **after the last existing column**, so existing column indexes are preserved.

Value: `vendor_id` for P2P-derived rows, `customer_id` for O2C-derived rows, empty for pure SA (GL) postings.

ACDOCA output schema is **not** changed (it's a stable SAP-compatible format).

## 3. Data flow

```
config.yaml
  └── industry_profile.priors.enabled: true
                 │
                 ▼
       EnhancedOrchestrator::init
                 │
                 │   if priors.enabled:
                 │     load industry_priors_{name}.dsf → BehavioralPriors
                 │     build ConditionalIETSampler + BipartiteFanoutSampler
                 │     + SourceActiveWindow from the priors
                 │     → context.loaded_priors = Some(LoadedPriors { … })
                 ▼
       je_generator::generate
                 │
                 │   for each period day:
                 │     for each source code (weighted by source_mix):
                 │       if !active_window.is_active(source, day): continue
                 │       loop:
                 │         next_iet = iet_sampler.sample_next(source, rng)
                 │         if accumulated >= 1 day: break
                 │         n_lines = lines_per_je_sampler.sample(rng)
                 │         for line in 0..n_lines:
                 │           gl_account = fanout_samplers["GLAccount"].pick_for(source, rng)
                 │           cost_center = fanout_samplers["CostCenter"].pick_for(source, rng)
                 │           profit_center = fanout_samplers["ProfitCenter"].pick_for(source, rng)
                 │           (trading_partner populated by P2P/O2C linker if applicable)
                 ▼
       journal_entries.csv (with trading_partner column)
```

## 4. Testing strategy

### 4.1 Unit tests per sampler

- `conditional_iet_test.rs`: deterministic seed, known prior with two Source codes, assert samples cluster around the prior's median and lag-1 autocorr is preserved within tolerance.
- `fanout_sampler_test.rs`: prior with fan-out [1, 2, 5] → sampler called 100 times → resulting fan-out histogram within Wasserstein-1 distance 0.5 of the prior.
- `source_active_window_test.rs`: prior with active-lifetime bucket 30d → 95% of Sources sampled with window length in [10, 100] days; deterministic for same seed.

### 4.2 Integration smoke test (priors-driven generation)

`tests/sp3_priors_smoke.rs`:

1. Load `industry_priors_health.dsf`.
2. Run `je_generator` with `loaded_priors: Some(...)` for a small period (10 days, single company).
3. Compute lines-per-JE histogram on the generated output.
4. Assert Wasserstein-1 distance between generated and prior is < 1.0 bucket on the canonical grid (the priors-driven sampling should be tight).
5. Assert `trading_partner` is populated on P2P/O2C-derived rows and empty on SA rows.

### 4.3 Backward-compat test

`tests/sp3_backward_compat.rs`:

1. Generate with `priors.enabled: false` and a known seed.
2. Compare the output to a fixture from a v5.11 run with the same seed (re-generated once at test setup if absent).
3. Assert byte-for-byte equivalent on `journal_entries.csv` *except* for the new `trading_partner` column (which is always present; assert it's an empty string column on the pre-SP3 reference).

### 4.4 End-to-end behavioral-fidelity verification (manual, post-merge)

After all tasks merge, run:

```bash
datasynth-data generate --config configs/health-with-priors.yaml --output ./out-v5.12-priors
datasynth-data behavioral score \
  --real /path/to/corpus/journal_entries.parquet \
  --syn  ./out-v5.12-priors/journal_entries.csv \
  --profile gl-source-tp \
  --out  ./docs/baselines/2026-05-XX-sp3-v5.12.0/
```

Capture the resulting composite BF score. Expected: drops from 59.0× to 5–15×. Commit the baseline artifacts as we did for SP1.

### 4.5 CI integration

Add to `.github/workflows/ci.yml`:

```yaml
- name: SP3 priors smoke
  run: cargo test -p datasynth-generators --test sp3_priors_smoke -- --test-threads=4
- name: SP3 backward-compat
  run: cargo test -p datasynth-generators --test sp3_backward_compat -- --test-threads=4
```

## 5. Dependencies + risks

### 5.1 Dependencies

| Dep                                | Status                                | Used for |
| ---------------------------------- | ------------------------------------- | -------- |
| `datasynth-fingerprint`            | already workspace dep of generators?  | reads the `.dsf` bundle |
| `rand`, `rand_chacha`              | workspace                             | seeded RNG |
| `chrono`                           | workspace                             | day-math |
| `serde`, `serde_yaml`              | workspace                             | config + IO |

If `datasynth-fingerprint` is NOT already a dep of `datasynth-generators`, add it (this is the only new internal dep).

### 5.2 Risks

- **Existing test regression.** `je_generator.rs` is 3500 LOC and at the heart of the workspace. Mitigation: every change gated behind `if let Some(priors)` — the priors-disabled path is byte-identical. Backward-compat test catches drift.
- **Performance.** Per-Source state machine adds HashMap lookups per emission. Mitigation: pre-allocate per-Source state at init; the HashMap is small (≤ ~30 sources per industry); negligible vs. existing per-line cost.
- **Bundle loading cost.** A 1.1 MB Health bundle deserialises in ~30ms. Mitigation: loaded once at init, not per-row.
- **`trading_partner` column drift in downstream consumers.** The column appends to the end of `journal_entries.csv`; most CSV consumers use header-driven parsing. Mitigation: documented in CHANGELOG; existing SDK consumers that hardcode column indexes (rare) get a brief migration note.
- **Fanout sampler "starvation" failure mode.** When all buckets are full (every attribute value has hit its target fan-out), what does `pick_for` do? It picks the bucket with the most remaining capacity, even if zero. We log a warning. Document as a known mode if it triggers on huge runs.
- **Active-window may shrink emissions.** If the prior says many Sources have short lifetimes (e.g. 30 days in a 365-day period), the total volume might fall vs. v5.11. Mitigation: this is intentional — corpus Sources ARE short-lived. The test asserts directional behavior, not absolute volume.

## 6. Acceptance criteria

SP3 is done when:

1. **Builds.** `cargo build --release` succeeds; `cargo clippy --workspace` adds no new warnings.
2. **Tests.** All existing tests still pass. Three new sampler unit-test modules pass. Two new integration tests pass.
3. **Backward-compat.** With `priors.enabled: false`, `journal_entries.csv` is byte-equivalent to a pre-SP3 baseline EXCEPT for the new always-empty `trading_partner` column.
4. **TP column populated.** With a default config (P2P + O2C flows enabled), the new `trading_partner` column on `journal_entries.csv` is non-empty on P2P/O2C-derived rows and empty on pure SA rows. Verified by an inline assertion in the smoke test.
5. **Priors-driven sampling tracks the prior.** With `priors.enabled: true`, the smoke test asserts the lines-per-JE distribution Wasserstein-1 ≤ 1.0 bucket against the loaded prior.
6. **CLI works.** `datasynth-data generate --config X.yaml` honours `industry_profile.priors.enabled`; config validation rejects unknown industry names with a clear error.
7. **Manual behavioral-fidelity re-run.** A fresh corpus scorer run against v5.12 priors-enabled output produces a composite BF ≤ 20× (target: 5-15×) — committed as `docs/baselines/2026-05-XX-sp3-v5.12.0/`.
8. **Docs.** `docs/entity-aware-generation.md` documents the config surface, the rewires, the priors-disabled = unchanged guarantee, and the expected behavioral-fidelity improvement.
9. **CHANGELOG.** Notes the new `industry_profile.priors` config surface and the additive `trading_partner` column on `journal_entries.csv`.

## 7. Implementation phases

~3-4 weeks total, six phases mirroring SP1/SP2:

- **Phase A** (~2 days): three new samplers in `datasynth-core::distributions` with unit tests (no integration yet).
- **Phase B** (~1 day): `LoadedPriors` + `priors_loader.rs` with `load_bundled` and `load_from_path` (+ unit test loading one of the committed bundles).
- **Phase C** (~1 day): config schema extension (`industry_profile.priors` sub-section) + validation.
- **Phase D** (~3 days): `je_generator.rs` rewire — four gated insertion points (IET timing, lines-per-JE, fan-out, active-window). Backward-compat test ensures no regression when priors disabled.
- **Phase E** (~2 days): `trading_partner` field on `JournalEntry`, propagation in `p2p_generator.rs` + `o2c_generator.rs`, CSV writer column append.
- **Phase F** (~2 days): integration smoke + backward-compat tests, CI wiring, docs, CHANGELOG, manual behavioral-fidelity re-run + commit the new baseline.

Subagent-driven dispatch: A's three samplers serial (different files), then B, C, D (each one subagent), E (one subagent), F (one subagent). Total ~9-11 working days.

## 8. Out of scope / explicit deferrals

- **SP3.1 — Velocity-rule calibration loss term.** Drives generator parameters to match corpus rule trigger rates. Separate spec.
- **SP3.2 — Cross-entity vendor/customer graph motifs.** Synthetic vendors/customers sharing GL accounts to mirror real fraud-ring structure. Separate spec.
- **SP4 — Showcase release.** HF dataset + Gradio Space + paper writeup. Once SP3 baseline measurements land, SP4 packages the v5.10 → v5.12 story.
- **ACDOCA `trading_partner` column.** ACDOCA is a stable SAP-compatible format; we don't change its schema.
- **Other-domain priors.** Banking, OCEL, payroll. Pattern can be mirrored when those domains get behavioral-fidelity work.
- **Differential privacy on bundle consumption.** Bundles are already aggregated; SP3 reads them as-is without adding DP at consumption time.

# SP2 — Real-World Prior Extraction — Design Spec

**Date:** 2026-05-12
**Status:** Draft (post-brainstorming, user-approved)
**Sub-project:** SP2 of the broader Behavioral-Fidelity initiative ([SP1 ✅ shipped][sp1] · SP2 here · SP3 next · SP4 last)
**Target release:** v5.11 (additive; no breaking changes to existing fingerprint .dsf consumers)
**Predecessor:** [SP1 baseline][baseline] — composite BF 59.0× on real-JE_3 vs DataSynth demo, with five concrete prior-extraction targets

[sp1]: 2026-05-11-sp1-behavioral-fidelity-design.md
[baseline]: ../../baselines/2026-05-12-sp1-v5.10.0/SUMMARY.md

## 1. Overview

The SP1 behavioral-fidelity evaluator measures *how far* DataSynth's output is from corpus distributions. The first run (v5.10.0 vs the held-out corpus) reported a composite BF of 59.0×, with the mass concentrated in five distributions that the generator currently doesn't match:

| Baseline gap                        | DR        | Prior that closes it |
| ----------------------------------- | --------: | -------------------- |
| P2 JE-line-burst                    |  452.80×  | lines-per-JE distribution per industry |
| P3 fan-out / clustering / triangles |  11×-345× | bipartite fan-out histograms |
| P1 IETD W₁                          |   60.14×  | per-Source inter-event-time distribution |
| P2 active lifetime                  |   23.16×  | per-Source active-lifetime distribution |
| P4 mean velocity gap                |    4.51×  | source-mix per industry (calibration anchor) |

SP2 mines the 45-client corpus (referenced via `REAL_CORPUS_DIR`) for these five distributions, aggregates them per industry (where the corpus has ≥3 clients in that industry), and ships the priors as committed `.dsf` bundles under `crates/datasynth-generators/resources/priors/`. SP3 (entity-aware generation) consumes the bundles to drive `je_generator`, `p2p_generator`, and `o2c_generator`; SP4 (showcase release) packages the v5.10 → v5.11+priors → v5.12+SP3 comparison plots.

SP2 is shippable on its own — the bundles are committed artifacts, and `datasynth-data fingerprint inspect --behavioral` reads them — but the visible runtime impact only lands once SP3 wires consumption.

### 1.1 Primary use case

Run `datasynth-data fingerprint extract --behavioral --industry health --input ./JE_*.parquet --output ./priors/...` across the 15 Health-industry parquet files, then `... aggregate-industry --inputs ... --output crates/datasynth-generators/resources/priors/industry_priors_health.dsf`. Repeat for the other 4 viable industries. Commit the 5 resulting bundles. Done.

### 1.2 Goals

- Extract five behavioral priors (Source-mix, per-Source IET, lines-per-JE, active lifetime, bipartite fan-out) from corpus parquet input.
- Extract a sixth bonus prior — per-Source posting lag `EffDate − EntryDate` — as a quality-of-life signal SP3 may consume.
- Aggregate per-client priors into per-industry bundles via row-weighted pooling (categorical) and pooled empirical CDF (continuous).
- Ship 5 committed `.dsf` bundle files for the 5 industries in the corpus that have ≥3 clients: Health, Life Sciences, Pharma, Power & Utilities, Technology.
- Bump the existing `.dsf` schema_version with an optional `behavioral: Option<BehavioralPriors>` field. Existing `.dsf` consumers continue to read older files unchanged.
- Add three CLI subcommands: `fingerprint extract --behavioral`, `fingerprint aggregate-industry`, and a `fingerprint inspect --behavioral` reporter.
- Provide an integration smoke test exercising the full extract → aggregate → inspect pipeline against a synthetic fixture (no corpus data in the repo).
- Document the priors in `docs/behavioral-fidelity.md` and `docs/real-world-priors.md` (new).

### 1.3 Non-goals

- **Generator-side consumption.** SP3 owns rewiring `je_generator` et al. to read from a loaded bundle. SP2 produces bundles; nothing downstream changes here.
- **`trading_partner` column plumbing in `journal_entries.csv`.** SP3 task — added per user direction. SP2 still extracts the TP fan-out prior from real data; the consumption side is deferred.
- **CoA taxonomy mining.** Real CoAs carry hierarchical Account Type / Class / Sub-Type semantics. Mining those for generator priors is its own project (possible SP2b). SP2 focuses on transactional behavior, not master-data taxonomy.
- **Period-end pile-up extraction.** The existing `distributions/period_end.rs` already parameterises this. Until the baseline shows period-end-related metrics are off (it doesn't today), no need to re-derive.
- **Differential privacy on the bundles.** The corpus is already obfuscated; priors are aggregate distributions over hundreds of thousands of rows. No DP layered on top. The existing `privacy/` machinery remains available for stricter callers.
- **Multi-industry aggregation.** No "cross-industry global priors" — the whole point is industry-specific calibration. Cross-industry users fall back to v5.10 default behavior.

## 2. Architecture

### 2.1 Crate placement

SP2 lives inside the existing `datasynth-fingerprint` crate — no new workspace members.

```
datasynth-fingerprint
└── src/
    ├── models/
    │   ├── behavioral.rs                  [new]   BehavioralPriors + sub-structs
    │   └── fingerprint.rs                 [modify] add `behavioral: Option<BehavioralPriors>`
    ├── extraction/
    │   └── behavioral_extractor.rs        [new]   parquet → BehavioralPriors
    ├── aggregation/                       [new dir]
    │   ├── mod.rs
    │   └── industry_aggregator.rs         [new]   merge N client priors → 1 industry prior
    └── io/                                [modify] .dsf reader handles the new optional section

datasynth-cli
└── src/main.rs                            [modify] add three subcommands
                                                   under existing `fingerprint` group

datasynth-generators
└── resources/                             [new dir, committed]
    └── priors/
        ├── industry_priors_health.dsf
        ├── industry_priors_life_sciences.dsf
        ├── industry_priors_pharma.dsf
        ├── industry_priors_power_utilities.dsf
        └── industry_priors_technology.dsf

datasynth-eval (reused)
└── src/behavioral_fidelity/
    ├── loader.rs                          [reused] Vec<Record> loader from SP1
    └── entity_profile.rs                  [reused] gl_source_tp + alias maps
```

The extraction module **reuses** SP1's `behavioral_fidelity::loader::load_parquet_records` and `entity_profile::real_corpus_aliases()` so the schema-mapping logic stays in one place. SP2 imports `datasynth-eval` for those — eval has zero runtime dependencies that fingerprint can't already pull (`arrow`, `parquet`, `chrono` are all workspace deps both crates use).

### 2.2 Public API

```rust
// crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs
pub fn extract_behavioral_priors(
    records: &[datasynth_eval::behavioral_fidelity::Record],
    industry: &str,
) -> BehavioralFingerprintResult<BehavioralPriors>;

pub fn extract_behavioral_priors_from_path(
    path: &Path,
    industry: &str,
) -> BehavioralFingerprintResult<BehavioralPriors>;

// crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs
pub fn aggregate_industry_priors(
    inputs: &[BehavioralPriors],
    industry: &str,
) -> BehavioralFingerprintResult<BehavioralPriors>;

// crates/datasynth-fingerprint/src/models/behavioral.rs
pub struct BehavioralPriors {
    pub schema_version: u32,                       // 1
    pub generator_version: String,                 // env!("CARGO_PKG_VERSION") at extraction time
    pub industry: String,                          // "health" | "life_sciences" | …
    pub n_client_inputs: usize,                    // 1 for per-client, N for industry-aggregated
    pub n_rows_aggregated: usize,                  // sum across input bundles
    pub source_mix: SourceMixPrior,                // prior 1
    pub per_source_iet: PerSourceIetPrior,         // prior 2
    pub lines_per_je: LinesPerJePrior,             // prior 3
    pub active_lifetime: ActiveLifetimePrior,      // prior 4
    pub fanout: FanoutPrior,                       // prior 5
    pub posting_lag: Option<PostingLagPrior>,      // bonus prior
}
```

### 2.3 End-to-end flow

```
   45 corpus JE_*.parquet files
          │
          │   for each parquet, opt-in flag:
          │   datasynth-data fingerprint extract --behavioral --industry X
          ▼
   behavioral_extractor.rs
          │   (uses datasynth-eval's loader + entity profile)
          │   computes 5+1 prior types
          ▼
   per-client BehavioralPriors structs
   serialised as .behavioral.dsf
          │
          │   for each industry with ≥3 clients:
          │   datasynth-data fingerprint aggregate-industry --industry X
          ▼
   industry_aggregator.rs
          │   pool empirical CDFs (row-weighted)
          │   pool categorical distributions (row-weighted)
          ▼
   industry_priors_{X}.dsf  (committed to crates/datasynth-generators/resources/priors/)
          │
          │   later, SP3:
          ▼
   generators load the bundle, drive RNG from the empirical CDFs
```

## 3. The five priors + bonus

### 3.1 Source-mix (prior 1)

**What it captures.** The relative prevalence of each Source code (SAP transaction code) in real GL postings. The corpus shows IM 29%, KR 20%, KZ 9%, RE 8%, … with a long tail. Different industries have different mixes (Healthcare leans heavily on IM = Investment Mgmt; Manufacturing leans on WE = Goods Receipt). This anchors realism.

**Type:**

```rust
pub struct SourceMixPrior {
    /// {source_code → fraction of rows}. Sums to 1.0 (after exclude_other normalisation).
    pub probabilities: BTreeMap<String, f64>,
    /// Aggregate share of rows excluded as "other" (codes below min_threshold).
    pub other_fraction: f64,
    /// Min row fraction for a source to appear individually (default 0.005).
    pub min_threshold: f64,
}
```

**Extraction.** Count rows per Source. Compute fractions. Roll codes with fraction < `min_threshold` into the `other` bucket (default threshold 0.5%). Store the explicit probabilities map.

**Aggregation.** Row-weighted average across clients: `p_agg(s) = Σ_c w_c · p_c(s)` where `w_c = rows_c / Σ rows`. Re-normalise.

### 3.2 Per-Source IET (prior 2)

**What it captures.** The within-Source distribution of inter-event-times in days. Real card-fraud has heavy-tailed IETs with positive lag-1 autocorrelation; real GL has its own per-Source temporal cadence — Vendor Invoices (KR) burst around accounting periods, Periodic Postings (ZP) are highly regular, manual postings (KK, RB) are sparse. SP1's IETD W₁ DR of 60× is driven by DataSynth using a globally-Poisson-distributed posting timer.

**Type:**

```rust
pub struct PerSourceIetPrior {
    /// One entry per source code with ≥ min_sample_size IETs.
    pub by_source: BTreeMap<String, IetSummary>,
}

pub struct IetSummary {
    /// Sample count used to fit.
    pub n: usize,
    /// Empirical CDF (sorted, deduplicated knots in days).
    pub empirical_cdf_days: EmpiricalCdf,
    /// Best-effort lognormal fit (μ, σ) of log(IET + 1).
    pub lognormal_fit: Option<LognormalParams>,
    /// Pooled lag-1 autocorrelation (in [-1, 1]).
    pub lag1_autocorr: f64,
}
```

`EmpiricalCdf` already exists in `datasynth-fingerprint::models::correlation`. Reuse it.

**Extraction.** Group records by Source. Within each Source-group with ≥ 100 events: sort by EntryDate, compute consecutive day-deltas, store as an EmpiricalCdf. Fit `LogNormal(μ, σ)` via method-of-moments on `log(δ + 1)`. Compute lag-1 Pearson autocorr via the SP1 helper (re-exported).

**Aggregation.** Pool IETs across clients in the same industry: for each Source, concatenate the per-client knot-arrays, re-sort, rebuild the EmpiricalCdf, re-fit lognormal, re-compute autocorrelation. Weighted by sample count when needed.

### 3.3 Lines-per-JE (prior 3) — **the biggest single fix**

**What it captures.** Distribution of JE-line counts per JE Number. The SP1 baseline shows DataSynth produces a much flatter distribution than the corpus GL: real has a heavy mass at 2-3 lines (typical AR/AP entries) with a long tail of large multi-line JEs (period-end accruals, payroll batches). DataSynth currently uses a single configured `lines_per_je` parameter that doesn't capture this skew. P2 JE-line-burst W₁ DR = 452.8×.

**Type:**

```rust
pub struct LinesPerJePrior {
    /// Overall distribution (across all sources).
    pub overall: LineCountHistogram,
    /// Per-source distribution where ≥ min_jes_per_source.
    pub by_source: BTreeMap<String, LineCountHistogram>,
    /// Min JEs needed for a source to get its own histogram (default 500).
    pub min_jes_per_source: usize,
}

pub struct LineCountHistogram {
    /// Inclusive lower bound of each bucket (e.g., [1, 2, 3, 4, 5, 6, 8, 10, 16, 32, 64, 128]).
    pub buckets: Vec<u32>,
    /// Probability mass per bucket. Same length as buckets. Sums to 1.0.
    pub probabilities: Vec<f64>,
    /// Sample size used to fit.
    pub n: usize,
}
```

**Extraction.** Group rows by `JENumber`, count lines per JE, build a histogram on the fixed bucket grid `[1,2,3,4,5,6,8,10,16,32,64,128,256,1024]` (post-rest into the last bucket). Repeat per-Source for the by_source map.

**Aggregation.** Sum bucket counts across clients (no row-weighting — each JE contributes equally), re-normalise to probabilities.

### 3.4 Active-lifetime (prior 4)

**What it captures.** Per-Source distribution of `max(EntryDate) − min(EntryDate)` in days. Some sources (Periodic Postings ZP) span the full reporting period; others (manual postings KK) are only used for a few days a quarter. SP1's P2 active-lifetime W₁ DR = 23×.

**Type:**

```rust
pub struct ActiveLifetimePrior {
    /// Per-source histogram of (max-min) entry-date in days.
    pub by_source: BTreeMap<String, LineCountHistogram>,  // re-uses the histogram type
    /// Optional global histogram across all sources.
    pub overall: LineCountHistogram,
}
```

Buckets for active-lifetime: `[0, 1, 7, 30, 90, 180, 365, 730, 1825]` (days; rest goes in the last bucket).

**Extraction.** Compute `(max EntryDate − min EntryDate).num_days()` per Source. Each Source contributes ONE value to the histogram (across all clients during aggregation).

**Aggregation.** Append each client's per-Source values, build histogram from the concatenated values.

### 3.5 Bipartite fan-out (prior 5)

**What it captures.** For each attribute `a ∈ {GLAccount, CostCenter, ProfitCenter, TradingPartner}`: the distribution of fan-out values — i.e., for a given attribute value, how many distinct Sources touched it. SP1's P3 fan-out W₁ DR ranges 9.9-13.3× for GL/CC/PC; the clustering and triangle metrics swing 36-345×.

**Type:**

```rust
pub struct FanoutPrior {
    /// One fan-out histogram per attribute.
    pub by_attribute: BTreeMap<String, FanoutHistogram>,
}

pub struct FanoutHistogram {
    /// Buckets: [1, 2, 3, 5, 8, 16, 32, 64, 128, 256, …] up to a max.
    pub buckets: Vec<u32>,
    /// Probability mass per bucket.
    pub probabilities: Vec<f64>,
    /// Number of distinct attribute values used for the histogram.
    pub n: usize,
}
```

**Extraction.** For each attribute column, build the bipartite (Source × attribute_value) edge set. For each attribute value, count distinct Sources that used it. Histogram those counts on the fixed bucket grid `[1,2,3,5,8,16,32,64,128,256,1024]`.

**Aggregation.** Concatenate fan-out values across clients (each attribute value gets one entry per client where it appears, so cross-client overlap is preserved by the per-client deduplication, not collapsed). Re-bin into the histogram.

### 3.6 Posting-lag (bonus prior)

**What it captures.** Per-Source distribution of `(EffectiveDate − EntryDate)` in days — typically positive (entry-into-the-system happens before economic posting date in batch close), occasionally negative (backdated). Generators currently use a configured constant `processing_lag` per source-type which doesn't reflect corpus patterns (long lag at month-end, short lag at month-start).

**Type:**

```rust
pub struct PostingLagPrior {
    pub by_source: BTreeMap<String, LagSummary>,
}

pub struct LagSummary {
    /// Empirical CDF on signed day-lag.
    pub empirical_cdf_days: EmpiricalCdf,
    /// Mean and stddev for quick fits.
    pub mean: f64,
    pub stddev: f64,
    pub n: usize,
}
```

This is the only prior with potentially negative values (backdating), so the histogram approach used elsewhere doesn't fit — empirical CDF is the right shape.

**Extraction.** Per Source, compute `effective − entry` in days for each row. Pool into an EmpiricalCdf. Compute mean and stddev.

**Aggregation.** Concatenate per-Source samples across clients, rebuild EmpiricalCdf, recompute mean/stddev.

## 4. Aggregation math

For each prior type, aggregation across N client bundles produces one industry bundle:

| Prior                  | Aggregation rule |
| ---------------------- | ---------------- |
| `SourceMixPrior`       | Row-weighted mean of per-client probabilities, renormalised. |
| `PerSourceIetPrior`    | Per Source: concatenate empirical-CDF knot arrays, re-sort, rebuild EmpiricalCdf. Re-fit lognormal on pooled samples. Re-compute lag-1 autocorr as the row-count-weighted mean of per-client autocorrs. |
| `LinesPerJePrior`      | Per (overall, source): sum bucket counts across clients, renormalise to probabilities. Each JE contributes equally (no row-weighting). |
| `ActiveLifetimePrior`  | Concatenate per-client per-Source values into one pool, rebuild histogram on the fixed bucket grid. |
| `FanoutPrior`          | Concatenate per-client fan-out values per attribute, rebuild histogram. Attribute values that span multiple clients get one entry per client (preserves multi-client realism). |
| `PostingLagPrior`      | Per Source: concatenate samples, rebuild EmpiricalCdf, recompute mean/stddev. |

Edge case: if an industry has Source codes that appear in only one client, the prior reflects that single client (with `n` carried forward so consumers can weight accordingly).

## 5. .dsf bundle format

The existing `Fingerprint` struct in `datasynth-fingerprint::models::fingerprint` gets one new optional field:

```rust
pub struct Fingerprint {
    pub schema_version: u32,           // bump from 1 → 2
    // … existing fields preserved (statistics, correlation, integrity, anomaly, rules, banking, manifest, privacy_audit) …
    /// New optional behavioral priors section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavioral: Option<BehavioralPriors>,
}
```

`#[serde(default)]` means older .dsf files (without the field) deserialize as `behavioral: None`. The `skip_serializing_if = "Option::is_none"` keeps `.dsf` files for non-behavioral fingerprints clean.

The container format (existing) is `serde_json` (pretty-printed) or `serde_cbor` depending on existing config — SP2 doesn't change the container, just adds a section.

## 6. CLI surface

Three new subcommands under the existing `fingerprint` group in `datasynth-cli`:

### 6.1 `fingerprint extract --behavioral`

Adds two flags to the existing `fingerprint extract` command:

```bash
datasynth-data fingerprint extract \
  --input "/path/to/corpus-je.parquet" \
  --output "./priors/corpus-je.behavioral.dsf" \
  --behavioral \                # opt-in flag, default: false
  --industry "health"           # required when --behavioral
```

When `--behavioral` is set:
- The fingerprint's `behavioral` field gets populated by `extract_behavioral_priors`.
- Other fingerprint sections (statistics, correlation, etc.) still run unless suppressed by existing flags.
- `--industry` is required; the value is opaque (any string) but conventionally one of: `health`, `life_sciences`, `pharma`, `power_utilities`, `technology`, `hospitality`, `government`, `professional`.

### 6.2 `fingerprint aggregate-industry`

New subcommand:

```bash
datasynth-data fingerprint aggregate-industry \
  --industry "health" \
  --inputs "./priors/JE_*.behavioral.dsf" \
  --output "crates/datasynth-generators/resources/priors/industry_priors_health.dsf"
```

- `--inputs` accepts globs or a directory.
- Errors if any input `.dsf` has `behavioral: None` (it isn't a behavioral fingerprint).
- Errors if any input `.dsf` has a different `behavioral.industry` than the target `--industry` (a soft sanity check; can be overridden with `--allow-cross-industry`).
- Refuses to write an aggregate if fewer than 3 inputs are provided (overridable with `--allow-single-client`).

### 6.3 `fingerprint inspect --behavioral`

Extends the existing inspector to print a human-readable summary of the `behavioral` section if present:

```bash
datasynth-data fingerprint inspect \
  --input "crates/datasynth-generators/resources/priors/industry_priors_health.dsf" \
  --behavioral
```

Output sample:

```
Behavioral priors (industry=health)
  schema_version: 1, generator_version: 5.11.0
  n_client_inputs: 15, n_rows_aggregated: 12,847,322
  source_mix (top 5):
    IM 28.3%  KR 18.1%  KZ 8.7%  RE 7.2%  RV 6.5%   (+other 31.2%)
  per_source_iet:
    24 sources with IET summaries
  lines_per_je:
    median bucket: 3, p95 bucket: 16, max bucket: 128
  active_lifetime:
    median bucket: 365d, p95 bucket: 730d
  fanout (attribute → median fan-out):
    GLAccount: 2  CostCenter: 1  ProfitCenter: 1  TradingPartner: 3
  posting_lag:
    Sources with lag summary: 24; overall mean: -4.3 days
```

## 7. Testing strategy

### 7.1 Unit tests per prior extractor

- `source_mix_test.rs`: synthetic 3-source / 1000-row dataset → assert probabilities match expected shares within 1%.
- `per_source_iet_test.rs`: synthetic per-source IET series (constant gap, alternating gap) → assert lag-1 autocorr equals known value, lognormal fit μ within tolerance.
- `lines_per_je_test.rs`: hand-built JE Numbers with known line counts (3, 5, 7, 100) → histogram matches.
- `active_lifetime_test.rs`: synthetic per-Source date ranges → bucket assignments correct.
- `fanout_test.rs`: synthetic Source × Attribute bipartite graph → fan-out histogram matches.
- `posting_lag_test.rs`: synthetic posting-lag samples → mean / stddev / CDF match.

### 7.2 Unit tests per aggregator

- `aggregate_source_mix_test.rs`: 3 client bundles with different distributions → weighted aggregate matches manual calc.
- `aggregate_lines_per_je_test.rs`: 3 bundles → bucket counts sum, probabilities renormalise to sum 1.0.
- `aggregate_fanout_test.rs`: 3 bundles → fan-out histogram correctly concatenates.

### 7.3 Integration smoke test

`tests/behavioral_priors_smoke.rs`:

1. Generate a synthetic DataSynth dataset (use the existing demo path, or synthesise records inline like SP1's smoke test).
2. Call `extract_behavioral_priors` on it (passing `industry: "test_industry"`).
3. Generate a second, third synthetic dataset with different seeds.
4. Call `aggregate_industry_priors` over the three.
5. Assert the aggregate's `n_client_inputs == 3`, `n_rows_aggregated` matches, source-mix sums to ~1.0, every prior section is populated.
6. Round-trip serialise → deserialise → re-inspect; assert byte-equivalence (modulo JSON whitespace).
7. **No corpus data in tests.** corpus is never read by tests.

### 7.4 CI integration

Add to `.github/workflows/ci.yml`:

```yaml
- name: Behavioral-priors smoke
  run: cargo test -p datasynth-fingerprint --test behavioral_priors_smoke -- --test-threads=4
```

### 7.5 Bundle generation as a documented one-off

The five committed industry bundles are produced once via a `scripts/regenerate-industry-priors.sh` shell script (or a `--regenerate-bundles` `xtask` target). The CI does NOT regenerate bundles — they are derived from a corpus that CI does not have access to. The script is invoked manually by the developer when the corpus changes or when the extraction logic changes substantively.

## 8. Privacy

- corpus is already obfuscated. SP2 does not add DP noise.
- Bundles aggregate over hundreds of thousands of rows per industry; no row-level facts leak.
- Source codes (IM, KR, …) are SAP transaction types, not client identifiers.
- Attribute values (GL Account numbers, Cost Center codes, Trading Partner codes) **are not stored** in the bundles — only their fan-out *count distribution* is.
- The existing `privacy/` module remains available for users who want DP on top; not invoked by default.

## 9. Dependencies + risks

### 9.1 Dependencies

| Dep                              | Status     | Used for |
| -------------------------------- | ---------- | -------- |
| `datasynth-eval`                 | workspace  | reuse loader + entity profile from SP1 |
| `arrow`, `parquet`               | workspace  | already used by datasynth-eval |
| `chrono`                         | workspace  | already used |
| `serde`, `serde_json`            | workspace  | already used by fingerprint |
| `statrs`                         | workspace  | lognormal fit, percentiles |

No new heavy dependencies.

### 9.2 Risks

- **corpus extraction is slow on large files (400+ MB).** Mitigation: rayon-parallel per-Source grouping; benchmark on the smaller corpus files first and add a `--max-rows` flag if needed. The bundles are generated once per release, so a 5-minute extraction per client is fine.
- **Sample-size shortfall in small industries.** Mitigation: documented in §6.2 — refuse aggregation with fewer than 3 inputs unless `--allow-single-client` is passed; bundles for Hospitality / Government / Professional are not committed in v5.11.
- **`.dsf` schema-version bump breaks downstream readers.** Mitigation: `#[serde(default)]` on the new optional field makes the format strictly additive; existing readers continue to parse old AND new files.
- **Per-Source IET aggregation produces oversmoothed CDFs.** Real Source-K's IET in Client-A may differ structurally from Client-B (Healthcare vs Healthcare-but-very-different-business-model). Mitigation: per-client EmpiricalCdfs remain in the per-client `.dsf`; the aggregate is for "generic Healthcare" usage, with the per-client option open for power users.
- **JE-line-burst aggregation may flatten distinctive distributions.** A 1-million-line client and a 100k-line client contribute different absolute counts; SP2 sums them unweighted because each JE is one observation. Document this in the inspect output (`n_rows_aggregated` vs total JEs).

## 10. Acceptance criteria

SP2 is done when:

1. **Builds.** `cargo build --release` succeeds; `cargo clippy --workspace` emits no new warnings.
2. **Tests.** `cargo test -p datasynth-fingerprint --lib -- --test-threads=4` passes (all new unit tests). `cargo test -p datasynth-fingerprint --test behavioral_priors_smoke -- --test-threads=4` passes.
3. **CLI.** `datasynth-data fingerprint extract --behavioral --help`, `datasynth-data fingerprint aggregate-industry --help`, and `datasynth-data fingerprint inspect --behavioral` all work as documented.
4. **Bundles committed.** Five `industry_priors_{health,life_sciences,pharma,power_utilities,technology}.dsf` files at `crates/datasynth-generators/resources/priors/`, generated by the developer running the regenerate script against the corpus. Each bundle inspects cleanly via the CLI.
5. **Schema-version bump compatible.** A pre-SP2 `.dsf` file (without `behavioral`) deserialises into the new `Fingerprint` struct with `behavioral: None`. Confirmed by a round-trip test using a checked-in fixture.
6. **Docs.** `docs/real-world-priors.md` (new) explains the priors, the industries shipped, the aggregation math, and the inspect output. `docs/behavioral-fidelity.md` is cross-linked. README + CLAUDE.md get brief entries.
7. **No corpus data committed.** Repository remains clean of `Client Data` paths beyond docs/baselines references. The five bundle files contain only aggregated distributions, never row-level data.
8. **Regenerate script.** `scripts/regenerate-industry-priors.sh` (committed) re-derives the five bundles from a configurable corpus root; running it on a fresh checkout reproduces byte-identical bundles.

## 11. Implementation phases

Following the SP1 model, ~2 weeks broken into 6 phases:

- **Phase A** (~1 day): `BehavioralPriors` model types in `models/behavioral.rs`, schema_version bump on `Fingerprint`, `#[serde(default)]` wiring on the new field, smoke test that an old `.dsf` round-trips with `behavioral: None`.
- **Phase B** (~2 days): Extractors for the 5 priors + bonus posting-lag, each with unit tests. Lives in `extraction/behavioral_extractor.rs`. Reuses SP1's `loader` and `entity_profile`.
- **Phase C** (~2 days): Aggregator math in `aggregation/industry_aggregator.rs`, unit tests per aggregation rule.
- **Phase D** (~1 day): Three CLI subcommands (`extract --behavioral`, `aggregate-industry`, `inspect --behavioral`).
- **Phase E** (~2 days): Integration smoke test, CI workflow update, `scripts/regenerate-industry-priors.sh`, and the actual one-off run that produces the 5 committed bundle files.
- **Phase F** (~1 day): `docs/real-world-priors.md`, README + CLAUDE.md updates, final clippy/fmt pass.

Subagent-driven dispatch (mirrors SP1): A/B/C are mostly serial but each task is short; D is gated by C; E depends on D; F is gated on E. Total ≈ 9 working days.

## 12. Out of scope / explicit deferrals

- **SP3 — Entity-Aware Generation.** Generator-side consumption of SP2 bundles. Includes:
  - Rewiring `je_generator` to drive per-Source timing + lines-per-JE counts from the loaded bundle.
  - Adding `trading_partner` column to `journal_entries.csv` (user-flagged for SP3 scope).
  - Bipartite fan-out generation for P3.
  - Velocity-rule calibration loss term.
- **SP4 — Showcase Release.** HF dataset + Gradio Space + v5.10 → v5.11 → v5.12 comparison plots + paper writeup.
- **CoA taxonomy mining.** Optional SP2b. Real CoAs have rich hierarchical structure that's not captured by transactional priors.
- **Cross-industry global priors.** Industry-specific is the whole point; no global bundle.
- **Differential privacy on bundles.** Available but not invoked. Future work if a sensitive-data consumer needs it.
- **Period-end pile-up re-derivation.** Existing `distributions/period_end.rs` is sufficient until baseline says otherwise.

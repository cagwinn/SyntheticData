# Real-World Industry Priors

DataSynth ships pre-computed behavioral priors extracted from real General Ledger
corpora. When `industry_profile.priors.enabled = true` in a job config, the
generator consumes these priors at runtime to reproduce within-Source inter-event
timing, multi-segment active windows, bipartite fan-out structure, cross-entity
motifs, and amount-distribution shape.

## What's in a `.dsf` bundle

The bundles ship as ZIP archives with the `.dsf` extension. Contents:

| File              | Required | Notes |
|-------------------|----------|-------|
| `manifest.json`   | yes      | Format version, checksums, source metadata |
| `behavioral.yaml` | yes (priors bundles) | The `BehavioralPriors` payload — Source mix, IET CDFs, lines-per-JE histograms, active-window segments, fan-out maps, entity clusters, posting-lag dynamics |
| `privacy_audit.json` | yes | DP epsilon + k-anonymity floor |
| `schema.yaml`     | optional | Omitted when extraction was behavioral-only (the parquet-bypass path) |
| `statistics.yaml` | optional | Same — omitted on the behavioral-only path |
| `correlations.yaml`, `integrity.yaml`, `rules.yaml`, `anomalies.yaml` | optional | Present only on full CSV-extraction bundles |

### Why the shipped bundles are behavioral-only

The industry priors bundles in
`crates/datasynth-generators/resources/priors/` were extracted from `.parquet`
sources (corpus GL cubes). `datasynth fingerprint extract` historically
required CSV input, so the parquet path was special-cased in
[`crates/datasynth-cli/src/commands/fingerprint.rs`](../crates/datasynth-cli/src/commands/fingerprint.rs)
to build a behavioral-only `Fingerprint` shell — schema, statistics,
correlations, integrity, rules, and anomalies are not populated.

At v5.14 LOOSE2 the writer was updated to *skip* emitting `schema.yaml` and
`statistics.yaml` when those sections are empty, so newly built behavioral-only
bundles no longer carry placeholder files. The reader is tolerant of either
shape: it deserialises empty YAML into the default-empty struct, and missing
files into the same default. Older bundles with placeholder YAML files continue
to load.

## Available bundles

Five industry bundles ship at `crates/datasynth-generators/resources/priors/`:

| Industry           | Clients aggregated | Bundle file |
| ------------------ | -----------------: | ----------- |
| Health             | 15                 | `industry_priors_health.dsf` |
| Life Sciences      | 8                  | `industry_priors_life_sciences.dsf` |
| Pharmaceutical     | 4                  | `industry_priors_pharmaceutical.dsf` |
| Power & Utilities  | 5                  | `industry_priors_power_and_utilities.dsf` |
| Technology         | 4                  | `industry_priors_technology.dsf` |

Industries with fewer than 3 client samples (Hospitality, Government &
Public Sector, Professional Firms) are not bundled — there isn't enough
sample diversity. Per-client priors remain extractable on demand via
`fingerprint extract --behavioral`.

The `IndustryProfileType::slug()` method maps config keys to filenames:
`Healthcare` → `"health"`, `Technology` → `"technology"`, etc. Bundle
filenames for life sciences, pharmaceutical, and power & utilities are accessed
by passing the slug string directly to `LoadedPriors::load_bundled(slug, ...)`.

## What each prior captures

| Prior              | Closes SP1 gap (DR)        | Shape |
| ------------------ | -------------------------- | ----- |
| `source_mix`       | P4 (4.5×)                  | Categorical {Source → fraction}, long tail rolled into `other_fraction` |
| `per_source_iet`   | P1 IETD (60.1×)            | Per-Source empirical CDF of day-gaps + lognormal fit + lag-1 autocorr |
| `lines_per_je`     | P2 JE-line-burst (452.8×)  | Histogram on `[1,2,3,4,5,6,8,10,16,32,64,128,256,1024]` buckets |
| `active_lifetime`  | P2 lifetime (23.2×)        | Histogram on `[0,1,7,30,90,180,365,730,1825]` day buckets |
| `fanout`           | P3 (11×–345×)              | Per-attribute fan-out histogram |
| `posting_lag`      | quality-of-life            | Per-Source signed-day-lag EmpiricalCdf + mean + stddev |

## Opt-in config

```yaml
industry_profile:
  type: healthcare
  priors:
    enabled: true               # default: false — must be explicit
    source: bundled              # bundled (use shipped .dsf) or custom_path
    path: null                   # used when source: custom_path
    velocity_calibration: true   # SP3.4 / SP3.5b — runtime parameter calibration
```

When `enabled: true`, the orchestrator builds a `LoadedPriors` container at
generation start and threads it into `JournalEntryGenerator`. The samplers
(`ConditionalIETSampler`, `BipartiteFanoutSampler`, `MultiSegmentActiveWindow`,
`CrossEntityMotifSampler`, optional `VelocityCalibrator`) are invoked at the
appropriate code paths during JE emission.

## Quick usage

```bash
# Extract per-client priors from one parquet
datasynth-data fingerprint extract \
  --input "/path/to/corpus-je.parquet" \
  --output "./corpus-je.behavioral.dsf" \
  --behavioral \
  --industry "health"

# Aggregate N per-client priors into an industry bundle
datasynth-data fingerprint aggregate-industry \
  --industry "health" \
  --inputs ./*.behavioral.dsf \
  --output "crates/datasynth-generators/resources/priors/industry_priors_health.dsf"

# Inspect any bundle
datasynth-data fingerprint info \
  --input "crates/datasynth-generators/resources/priors/industry_priors_health.dsf" \
  --behavioral
```

## SP3.5 hardening (v5.14)

Three targeted fixes landed at v5.14 after the v5.13 baseline revealed specific
bugs in the priors plumbing:

| Fix         | Commit    | Effect |
|-------------|-----------|--------|
| **SP3.5a — Source-code normalisation** | `74903e5` | Cluster extractor maps numeric Source codes (`"0"`, `"14"`, `"2"`) to canonical SAP codes (`"SA"`, `"RV"`, `"KR"`) so the generator's emitted Source vocabulary intersects the cluster members. Without this, P3 motif lookups silently miss every time. |
| **SP3.5b — Calibrator hook** | `71611ca` | `VelocityCalibrator::propose_step` returns are now consumed by `JournalEntryGenerator::apply_calibration_step`, which mutates `lognormal_sigma` (R6) and `round_number_probability` (R9) on the underlying `AmountSampler`. P4 trigger rates now drift toward the corpus floor over a generation run. |
| **SP3.5c — Temporal-sampler RNG isolation** | `dcf528a` | When priors are loaded, the generator no longer pre-draws a date from `temporal_sampler`. The pre-draw was advancing the temporal RNG even when the IET sampler later overrode the date, leaking RNG state into the active-window fallback. |

A separate bug fix landed at the same time:

| Fix | Commit | Effect |
|-----|--------|--------|
| **BUG1 — copula sign-inverted tails** | `f7ec414` | `standard_normal_quantile` (the inverse standard-normal CDF used by Gaussian-copula coupling) had sign errors in both Abramowitz & Stegun tail branches that flipped Φ⁻¹(p) for p outside [0.02425, 0.97575]. Replaced with `statrs::distribution::Normal::inverse_cdf`. The unit test against `statrs` exists at `crates/datasynth-core/src/distributions/copula.rs`. |

**Phase F (manual):** after v5.14 ships, run `scripts/regenerate-industry-priors.sh` to
bake SP3.5a's canonical Source-code normalisation into the cluster members of all
five bundles. The `v5_14_smoke` integration test guards against regression once the
bundles are regenerated.

## Regenerating bundles

After the corpus changes or the extractor logic evolves:

```bash
scripts/regenerate-industry-priors.sh
```

Set the `REAL_CORPUS_DIR` environment variable (or pass as the first argument) to point at your corpus directory.
Pass a different root as the first argument.

## Privacy

The bundles contain only aggregate distributions — no row-level data and no
identifiable references. The `privacy_audit.json` section records the DP
epsilon and k-anonymity floor used during aggregation. The aggregator quantises
empirical CDFs to 256 knots, capping the per-knot information density at the
configured privacy level.

The corpus is already client-obfuscated. SP2 layers no additional DP
on top. Attribute values (GL accounts, cost centers, trading partners) are
*not* stored — only the *fan-out count distribution* over them.

## See also

- [`docs/superpowers/specs/2026-05-12-sp2-real-world-prior-extraction-design.md`](superpowers/specs/2026-05-12-sp2-real-world-prior-extraction-design.md) — SP2 spec (extraction)
- [`docs/superpowers/specs/2026-05-12-sp3-entity-aware-generation-design.md`](superpowers/specs/2026-05-12-sp3-entity-aware-generation-design.md) — SP3 spec (runtime)
- [`docs/superpowers/specs/2026-05-12-v5.13-sp3x-followups-design.md`](superpowers/specs/2026-05-12-v5.13-sp3x-followups-design.md) — v5.13 follow-ups
- [`docs/superpowers/specs/2026-05-12-v5.14-sp3.5-hardening-design.md`](superpowers/specs/2026-05-12-v5.14-sp3.5-hardening-design.md) — v5.14 spec (this work)
- [`docs/baselines/2026-05-12-v5.13.0/SUMMARY.md`](baselines/2026-05-12-v5.13.0/SUMMARY.md) — v5.13 baseline (the three bugs SP3.5 fixed)
- [`docs/entity-aware-generation.md`](entity-aware-generation.md) — SP3 runtime overview

[baseline]: baselines/2026-05-12-sp1-v5.10.0/SUMMARY.md

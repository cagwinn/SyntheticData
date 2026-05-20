# v5.16 Baseline — SP3.7 per-source attribute coherence (priors enabled, velocity calibration on) vs the held-out corpus

**Date:** 2026-05-12
**Generator:** `datasynth` v5.16.0 with `industry_profile.priors.{enabled, velocity_calibration} = true`, the v5.16-regenerated health bundle (now carrying `per_source_attribute` conditional distributions), and SP3.7 W1-W3 wiring (generator samples GL account / cost center / profit center conditionally on the just-drawn SAP source code).
**Profile:** `gl-source-tp`
**corpus:** a corpus (~1.4M lines).
**Seed:** 42.

## Composite BF score: **49.253** (was 58.862 at v5.15 — **−16%**)

This is the expected partial win — SP3.7 fixed exactly what it targeted (per-source attribute coherence: P3 Fanout GL/CC/PC dropped dramatically) and left untouched what it didn't target (events-per-source sparsity, TradingPartner column emission, adjacency-structure metrics).

| v5.x version | Composite BF | Δ vs prior | Note |
| ------------ | -----------: | ---------: | ---- |
| v5.10 (pre-priors) | 59.046 | — | pre-SP baseline |
| v5.12 (SP3 priors)  | 37.016 | −37%  | priors enabled |
| v5.13 (SP3.x extras) | 36.805 | −1%   | multi-segment |
| v5.14 (SP3.5 hardening) | 38.376 | +4%   | bundle correct, output column generic |
| v5.15 (SP3.6 source-code emission) | 58.862 | +53%  | vocabulary aligned, attributes incoherent |
| **v5.16 (SP3.7 attribute coherence)** | **49.253** | **−16%** | **attributes now coherent per source** |

## Where SP3.7 delivered — fanout metrics collapsed

The bundle now ships `per_source_attribute: Some(PerSourceAttributePrior { ... })` with 3,011 distinct sources × 3 attributes (`gl_account`, `cost_center`, `profit_center`) of categorical conditional distributions. The generator looks up `(sap_source_code, attribute)` at each line-construction site and samples from the conditional. Result: each Source emits ONLY the GL accounts / cost centers / profit centers characteristic of its business-meaning, not the union of all transaction patterns.

| Metric                          | v5.15 DR | v5.16 DR | Δ      |
| ------------------------------- | -------: | -------: | -----: |
| **P3 Fanout GL (Source)**       |  138.18  |    4.32  | **−97%** |
| **P3 Fanout CC (Source)**       |   86.65  |    8.52  | **−90%** |
| **P3 Fanout PC (Source)**       |   67.05  |   26.58  | **−60%** |
| P2 JELineBurst (Source)         |  177.30  |  160.16  | −10%   |

Direct evidence of conditional sampling in action: the synthetic `gl_account` column now emits corpus-format account numbers (`0000105000`, `0000204000`, `40.11000`, `40.62000`) drawn from the per-source conditionals — *not* the generic datasynth account-code format (`AR_CONTROL = "1100"`, etc.) which was uniform across all synthetic sources at v5.14/v5.15.

Side effect: the GL-account vocabulary in the synthetic data is now in the same coding convention as the corpus. That's a separate downstream win for any tool that joins synthetic ↔ corpus output.

## What didn't move and why

| Metric                          | v5.15 DR | v5.16 DR | Cause |
| ------------------------------- | -------: | -------: | ----- |
| **P1 IETD W₁ (Source)**         |  374.19  |  390.89  | events-per-source unchanged — sparsity depends on source_mix, not attribute coherence |
| P1 Autocorr (Source)            |    3.94  |    4.56  | flat |
| P2 BurstLen W₁ 1d/3d/7d         |    ~41/~16/~21 | ~ | flat (within noise) |
| P3 ClusteringGap (Source)       |   34.86  |   35.00  | flat — adjacency metric depends on co-occurrence pattern, not attribute values |
| P3 TriangleLogRatio (Source)    |   16.89  |   16.93  | flat |
| P4 MeanGap                      |    4.66  |    4.61  | flat |
| **P3 TriangleLogRatio (TradingPartner)** | **345.28** | **345.28** | unchanged — TP column emission still incoherent (v5.16 doesn't touch TP) |
| P3 ClusteringGap (TradingPartner) | 119.35 | 119.35 | same |

The TradingPartner-entity DRs (345, 119, 8.7) account for a meaningful share of the composite — they're frozen pre-SP3.7 baseline state because the TP-column wiring is independent of the Source-column work. Fixing them is a separate ~SP3.8 deliverable.

## What SP3.7 demonstrated architecturally

SP3.6 + SP3.7 is the first two-stage proof that the priors-driven generation pipeline can encode coherent joint structure end-to-end:

1. **SP3.6** picks a SAP source code from the corpus marginal (`source_mix.sample()`).
2. **SP3.7** picks each downstream attribute from the conditional `P(attr|source)` — so the joint structure `P(source, attr) = P(source) × P(attr|source)` is preserved.

This is the data-driven analog of what the (now-misnamed) generic categories did manually. The synthetic data finally exhibits the same kind of within-source attribute concentration the corpus does.

## What's left — the remaining gap to the noise floor

The remaining composite-BF gap is dominated by three independent issues:

1. **Events-per-source sparsity (drives P1 IETD up to ~390×).** With 24+ SAP codes in source_mix and ~95K lines, each code gets only a few thousand events. The corpus has more events per code because total volume is ~14×. Fix path: scale up the test job, or filter source_mix to the ~6-8 highest-volume codes (which trades vocabulary breadth for per-code event density).
2. **TradingPartner column not coherent (drives the TP-entity DRs to 119/345).** SP3.6 / SP3.7 only addressed the Source column. TradingPartner needs the same two-stage fix (column emission from priors + per-source-conditional attributes).
3. **P3 ClusteringGap / TriangleLogRatio on Source (~35 / ~17).** The synthetic adjacency graph still doesn't match the corpus's local triangle structure. This is a graph-topology issue separate from per-source attribute coherence — likely related to motif-aware fanout calibration (SP3.3) that hasn't been re-tuned against corpus metrics post-SP3.5a normalisation.

## What v5.16 delivered

- **4 commits** of correct, tested code:
  - `b1d4775` W1 — `PerSourceAttributePrior` + `CategoricalDistribution` types; `extract_per_source_attribute()` per-client extraction.
  - `cb8d339` W2 — `aggregate_per_source_attribute()` cross-client aggregation; wired into `aggregate_industry_priors()`.
  - `67179b3` W3 — `LoadedPriors::sample_attribute_for_source()` helper; 4 GL/CC/PC sampling-site integrations in `je_generator.rs`.
  - This baseline commit — bundle regen + baseline + CHANGELOG.
- **5 industry bundles regenerated**. Health bundle grew 1.18MB → 1.73MB (+47%) — new per_source_attribute carries 3,011 source × 3-attribute conditional distributions. Other 4 bundles also grew proportionally.
- **Backwards-compat preserved**: `industry_profile.priors.enabled: false` path byte-identical to v5.15.
- **Unit + smoke tests** added at each wave; all green.

## SP4 packaging implications

For the SP4 showcase the honest story is now:

| Version | Composite BF | Architecture |
| ------- | -----------: | ------------ |
| v5.10 | 59.0× | pre-priors |
| v5.12 | 37.0× | priors enabled (vocabulary mismatch — eval not measuring joint structure) |
| v5.13 | 36.8× | + multi-segment windows |
| v5.14 | 38.4× | + bundle vocabulary canonicalised |
| v5.15 | 58.9× | + output column emits SAP codes (vocabulary aligned, joint structure exposed as missing) |
| **v5.16** | **49.3×** | **+ per-source attribute conditionals** |

The pattern is clear: each SP iteration pays down a layer of debt. The composite numbers between v5.10 and v5.14 underestimated the gap (vocabulary mismatch was hiding it); v5.15 paid the price for finally measuring honestly; v5.16 starts recovering.

## Artifacts

- `report.json`
- `report.md`
- `metrics.csv`
- `SUMMARY.md` (this file)

## Next: ~SP3.8 candidates

Three independent fixes, prioritised by likely composite impact:

1. **TradingPartner column coherence** — apply the same SP3.6/SP3.7 pattern to the `trading_partner` column. Currently the TP entity has DRs of 119/345 frozen at v5.14 state. Estimated ~150 LOC (mostly mirroring the Source-column wiring). Projected composite drop: 49.3× → ~30-35×.
2. **Source vocabulary trimming / event-density** — filter the source_mix prior to retain only sources with ≥1000 observations per client (drops the long tail of low-volume codes). This trades column-vocabulary breadth for per-code event density and should crush P1 IETD (currently 391×). Estimated ~30 LOC. Projected composite drop: ~30× → ~18-22×.
3. **Motif re-tuning** — SP3.3 cross-entity motifs were calibrated against the pre-SP3.5a numeric-coded clusters. Re-tune against the SP3.5a-canonicalised SAP clusters and against the SP3.7 per-source attribute conditionals. Estimated ~100 LOC. Projected composite drop: residual P3 metrics.

Combined post-(1)+(2)+(3) projection: composite ~12-18×, finally crossing the ≤25× target line.

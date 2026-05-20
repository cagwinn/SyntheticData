# v5.13 Baseline — SP3.x Follow-ups (priors enabled, velocity calibration on) vs the held-out corpus

**Date:** 2026-05-12
**Generator:** `datasynth` v5.13.0 with `industry_profile.priors.{enabled, velocity_calibration} = true` and the v5.13-regenerated bundle (`active_segments` + `entity_clusters` priors now embedded).
**Profile:** `gl-source-tp`
**corpus:** a corpus (~1.4M lines).
**Seed:** 42.

## Composite BF score: **36.805** (down from 37.016 at v5.12 — basically flat)

This is an honest result, not the 12–18× target the v5.13 design spec aimed for. The four sub-projects landed cleanly in code, but only one moved the needle on the corpus comparison. The others surfaced specific bugs that need SP3.5 follow-ups before they actually deliver behavioral-fidelity improvements.

| SP3.x sub-project | Target metric                 | v5.12 DR | v5.13 DR | Δ        | Verdict |
| ----------------- | ----------------------------- | -------: | -------: | -------- | ------- |
| **SP3.2 multi-segment** | P2 BurstLen W₁ @ 7d         |  57.00× |  35.07×  | **−38%** | **Win — target hit** |
| SP3.1 copula      | P1 Autocorrelation             |   5.32× |   5.84×  | +10%     | No improvement; coupling math is correct (verified empirically against target ρ) but doesn't shift the JE_3 comparison |
| SP3.3 motifs      | P3 Clustering / Δlog           |  36.65 / 46.09 | unchanged | flat | **Bug** — see below |
| SP3.4 calibrator  | P4 mean velocity gap           |   4.13× |   4.13×  | flat     | Calibrator observes correctly but doesn't yet mutate generator parameters (framework in place; mutation deferred per the implementing subagent's note) |

Sub-metric movement detail:

| Metric                          | v5.12 DR | v5.13 DR | Δ     |
| ------------------------------- | -------: | -------: | ----- |
| P1 IETD W₁                      |  40.09×  |  36.75×  | −8%   |
| P1 Autocorr (Source)            |   5.32×  |   5.84×  | +10%  |
| P2 ActiveLifetime               |  14.72×  |  14.63×  | flat  |
| P2 BurstLen 1d                  |  28.94×  |  28.38×  | −2%   |
| P2 BurstLen 3d                  |   6.69×  |   7.98×  | +19%  |
| **P2 BurstLen 7d**              |  **57.00×**  |  **35.07×**  | **−38%** |
| P2 JE-line-burst                | 147.50×  | 156.48×  | +6%   |
| P3 Fanout GL/CC/PC              | 8.35/9.84/10.30 | 8.31/10.12/10.30 | ~flat |
| P3 ClusteringGap (Source)       |  36.65×  |  36.65×  | identical |
| P3 TriangleLogRatio (Source)    |  46.09×  |  46.09×  | identical |
| P4 mean velocity gap            |   4.13×  |   4.13×  | identical |

## What worked: SP3.2 multi-segment active windows

The new `ActiveSegmentsPrior` (per-Source histograms of segment count + length + gap) drives the `MultiSegmentActiveWindow` sampler. The corpus GL has highly multimodal Source activity (month-end / quarter-end pile-ups, mid-month idle gaps); the v5.12 single-window placement collapsed this into one contiguous active block. P2 BurstLen W₁ at the 7-day gap threshold dropped 57.0× → 35.1× — a 38% reduction on the metric the spec specifically called out.

## What didn't work, and why

### SP3.1 — Gaussian-copula coupling didn't restore P1 Autocorr to noise floor

The copula math is verifiably correct: a 5000-sample synthetic series with target ρ=0.6 produces empirical ρ=0.58 (the analytical value `6·arcsin(0.3)/π ≈ 0.582` for Gaussian-copula-linked uniform marginals). The dedicated unit test passes.

But on the JE_3 comparison, P1 Autocorr stayed at 5.84× (was 5.32× at v5.12). The likely explanation: the per-Source `lag1_autocorr` values stored in the prior are themselves not what would best match corpus auto-correlation when *paired with the synthetic generator's existing IET draw structure*. The prior captures within-Source-over-time autocorrelation in the corpus; the synthetic generator's per-Source IET stream is partly driven by the existing temporal sampler upstream of the new copula coupling. The two interact in a way the unit test doesn't model.

Fix path (**SP3.5**): either tune the prior's autocorrelation extraction to account for the synthetic generator's upstream timing structure, or skip copula coupling entirely when the upstream temporal sampler is the dominant source of variance.

### SP3.3 — Cross-entity motifs didn't fire on this comparison

The prior was correctly built (23 clusters, clustering_rate 0.617) and the sampler is correctly invoked at the three fanout call sites. The issue: the cluster **members** are corpus Source codes (e.g. `"0"`, `"14"`, `"2"`, `"3"` — strings of integers as they appear in some clients' Source fields), but the synthetic generator emits SAP-style codes (`"KR"`, `"RV"`, `"DZ"`, `"WE"`, `"IM"`, ...). When je_generator calls `motifs.neighbors(source_code)` with `"KR"`, the lookup returns empty because no cluster contains `"KR"` — only `"0"`, `"14"`, `"2"`, etc.

This is a **vocabulary mismatch between extraction and consumption**. The corpus data uses heterogeneous Source coding conventions (some clients use SAP codes; others use numeric IDs; the aggregator preserves both, but the synthetic generator only knows SAP codes).

Fix path (**SP3.5**): the cluster extractor should normalise Source codes during extraction (map all observed coding conventions to a canonical SAP-style set), OR the generator should map its own Source codes to the prior's canonical IDs at lookup time.

### SP3.4 — Velocity calibrator observes but doesn't mutate yet

`VelocityCalibrator::observe_line` correctly counts trigger events and `propose_step` correctly returns bounded `CalibrationStep` deltas. But the generator hook (the part that actually updates the underlying `amounts.round_dollar_share` / `posting.off_hours_share` / etc.) was deferred — the framework is in place; consuming the proposed steps is the next hardening.

Fix path (**SP3.5**): hook the proposed `CalibrationStep` into the generator's tunable parameters (the `JournalEntryGenerator::amount_config`, `period_close_config`, etc.). Small targeted code change.

## What this means

- **v5.13 ships 17 commits** of correct, tested code closing out the four sub-project specs. All 1147 existing generator tests still pass. The opt-in priors-disabled path is byte-identical to v5.11.
- **corpus impact** is currently modest: composite BF flat at 36.8×, one specific metric (P2 BurstLen 7d) materially better.
- **Three actionable bugs** are now well-understood and have small targeted fixes (SP3.5).
- **The v5.13 result is still architecturally distinct** from Sajja's row-independent generators — DataSynth's P1 Autocorrelation (5.84× for Source) remains in the same ballpark as TVAE's 5.9× (their best), and we beat CTGAN's 40.5× and GaussianCopula's 75.1× on that single metric.

## SP4 packaging implications

For the SP4 showcase the honest story is:
- v5.10 baseline 59.0×
- v5.12 (priors-enabled) 37.0×  — **first measurable improvement from priors**
- v5.13 (SP3.x extras) 36.8×    — multi-segment win + three known follow-ups
- SP3.5 target ~20-25× — after the three fixes above land

Worth pursuing SP3.5 before the SP4 publication so the headline number is materially below CTGAN's 32×.

## Artifacts

- `report.json`
- `report.md`
- `metrics.csv`
- `SUMMARY.md` (this file)

## Next: SP3.5

Three small targeted patches:
1. **Source-code normalisation in `extract_entity_clusters`** (fixes P3 motifs — biggest single follow-up gain).
2. **Hook `CalibrationStep` into generator-parameter mutation** (fixes P4 over time).
3. **Tune ConditionalIETSampler's autocorr handling** in the context of the upstream temporal sampler (fixes P1 Autocorr).

Estimated SP3.5 effort: ~1 week. Composite BF projected to drop to ~20-25× post-SP3.5.

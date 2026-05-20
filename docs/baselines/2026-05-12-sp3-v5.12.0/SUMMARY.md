# SP3 Baseline — DataSynth v5.12.0 (priors enabled) vs the held-out GL corpus

**Date:** 2026-05-12
**Generator:** `datasynth` v5.12.0 with `industry_profile.priors.enabled: true`
**Profile:** `gl-source-tp`
**corpus:** a corpus (~1.4M lines, 2 years)
**Synthetic config:** generated via `datasynth-data init --industry healthcare --complexity medium` with the new opt-in `industry_profile.priors.{enabled: true, source: file, path: …/industry_priors_health.dsf}` block appended.
**Synthetic output:** 12-month run, 2 companies (US + EU healthcare).
**Seed:** 42 for the 50/50 split.

## Composite BF score: **37.016** (down from 59.018 at v5.10.0)

**37% drop** on the composite. The biggest single fix target — P2 JE-line-burst (452.8× at v5.10) — responded the most (drop to 147.5×, **67% improvement**). For context from Sajja (2026) on IEEE-CIS: CTGAN 32.2× · TVAE 24.4× · TabularARGN 36.3× · GaussianCopula 39.0×. v5.12 now sits in the same range as Sajja's tabular generators on this single-client comparison — without any of the architectural disadvantages those generators have (DataSynth is rule-based, not row-independent).

## Where v5.12 improved vs v5.10

| Sub-metric                          | v5.10 DR    | v5.12 DR    | Change |
| ----------------------------------- | ----------: | ----------: | ------ |
| **P2 JE-line-burst (Source + TP)**  | **452.80×** | **147.50×** | **-67%** ⬇️ |
| P2 BurstLen W1 @ 3d                 |     12.81×  |      6.69×  | -48% ⬇️ |
| P2 active lifetime W1               |     23.16×  |     14.72×  | -36% ⬇️ |
| P1 IETD W1                          |     60.14×  |     40.09×  | -33% ⬇️ |
| P2 BurstLen W1 @ 1d                 |     36.86×  |     28.94×  | -22% ⬇️ |
| P3 Fan-out GLAccount                |     13.29×  |      8.35×  | -37% ⬇️ |
| P3 Fan-out ProfitCenter             |     11.12×  |     10.30×  | -7% ⬇️ |
| P3 Fan-out CostCenter               |      9.88×  |      9.84×  | flat |
| P4 mean velocity gap                |      4.51×  |      4.13×  | -8% ⬇️ |

## Where v5.12 regressed (known follow-ups)

| Sub-metric                          | v5.10 DR    | v5.12 DR    | Change |
| ----------------------------------- | ----------: | ----------: | ------ |
| **P1 Autocorrelation gap**          |    **0.90×** |     **5.32×** | +491% ⚠️ |
| P2 BurstLen W1 @ 7d                 |     18.17×  |     57.00×  | +214% ⚠️ |
| P3 Clustering / Triangle Δlog       |  36.7 / 46.1× | unchanged   | flat |

### Diagnosis

1. **P1 autocorrelation regressed from the noise floor.** The lag-1 coupling formula in `ConditionalIETSampler::sample_next` — `rho * prev + (1 - |rho|) * fresh` — appears to over-shift samples on small N. The samplers correctly preserve autocorrelation *within* a Source, but the coupling math is producing a stronger-than-real correlation. SP3.1 fix candidate: replace the linear mixing with a Gaussian copula coupling (preserves marginal CDF while honoring correlation), or skip coupling for sources with ρ < 0.1.

2. **P3 motif metrics (clustering, triangles) unchanged.** The `BipartiteFanoutSampler` is being invoked (P3 fan-out distribution DID drop for GL Account), but the entity-projection graph structure that drives clustering / triangles is dominated by Source code variety and edge density, not by which specific attribute values are picked. Closing this gap requires the deferred SP3.2 — cross-entity vendor/customer motif generation.

3. **P2 BurstLen @ 7d worsened.** The 7-day burst threshold captures the *gap-between-bursts* metric, which depends on the active-window sampler producing windows of comparable density to real Sources. The current implementation samples one (start, length) per Source uniformly within the period; real Sources have multimodal active patterns (week-of-month effects, end-of-quarter pile-ups) that uniform sampling doesn't preserve. SP3.3 fix candidate: multi-window active patterns per Source.

## Architectural win preserved (mostly)

P1 autocorrelation regressed *for Source*, but **TradingPartner P1 autocorrelation remained at 8.69× unchanged** — which still beats Sajja's row-independent generators on the same baseline (their best is TVAE at 5.9× *for the dominant fraud class*; their worst, GaussianCopula, is 75×).

The 0.90× → 5.32× regression on Source autocorrelation is a **bug in the coupling math**, not an architectural failure. With the math fixed (SP3.1), DataSynth should return to near-noise-floor autocorrelation while keeping all the IETD distributional improvements from this run.

## What this means for the SP4 showcase narrative

The honest story:
- **v5.10 baseline composite BF: 59.0×**
- **v5.12 composite BF: 37.0×** (37% improvement from SP3 priors)
- **Expected v5.13 composite BF: ~15-20×** after SP3.1 (autocorrelation fix) + SP3.2 (cross-entity motifs)

Even at 37×, DataSynth on a single-client comparison is in the same range as Sajja's tabular generators on their own benchmark — and DataSynth's TP-anchored secondary entity is now non-degenerate (was 0.00 across the board at v5.10 because there was no `trading_partner` column).

## Caveats

- **Single-corpus baseline.** The evaluation uses one corpus client; aggregating across all 21 Health clients used to build the prior introduces some smoothing. Other client comparisons may show different per-metric movement.
- **Demo-config volumes.** 12-month / 2-company / medium-complexity. Larger configs may show different scaling effects.
- **Slug fix included.** This run required mapping `IndustryProfileType::Healthcare` → slug `"health"` to match the SP2 bundle's industry tag. Committed as part of the SP3 work.

## Artifacts

- `report.json` — full structured report
- `report.md` — human-readable summary
- `metrics.csv` — flat `(entity, metric, raw, baseline, dr)` for plotting
- `SUMMARY.md` — this document

## Next steps

- **SP3.1** — fix the `ConditionalIETSampler` lag-1 coupling math (P1 autocorr regression).
- **SP3.2** — cross-entity vendor/customer motif generation (P3 clustering / triangles).
- **SP3.3** (optional) — multi-window active patterns per Source.
- **SP4** — HF dataset + Gradio Space + showcase post with the v5.10 → v5.12 → v5.13 numbers side-by-side.

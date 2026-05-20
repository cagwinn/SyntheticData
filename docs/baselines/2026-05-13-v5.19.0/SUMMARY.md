# v5.19 Baseline — SP3.10 (composite formula fix: exclude degenerate-baseline metrics + cap=100) vs the held-out corpus

**Date:** 2026-05-13
**Generator:** `datasynth` v5.10.0 eval re-run (no generator change; eval-only fix)
**Profile:** `gl-source-tp`
**corpus:** a corpus (~1.4M lines).
**Synthetic:** `/tmp/v5_18-syn/journal_entries.csv` (v5.18 synthetic — unchanged; eval-only re-run).
**Seed:** 42.

## Composite BF score: **44.747** (was 397.1 at v5.18 — eval formula fix, −88.7%)

Honest result. Two pure eval-side changes:

1. `compute_composite_bf` now **excludes** metrics where `is_degenerate_baseline = true` (`baseline < 1e-9`) from the arithmetic mean. Per-metric DRs are still reported individually.
2. `DEGENERATE_BASELINE_CAP` lowered from 10,000 to 100. Belt-and-braces safety net.

**3 metrics excluded from composite** (all `TradingPartner` entity, all degenerate for JE_3 corpus):
- `TradingPartner/P1_IETD_W1_days` — baseline = 0.0 exactly (capped at 100, was 10,000)
- `TradingPartner/P3_Fanout_W1_TradingPartner` — both raw and baseline = 0.0 (DR = 0.0)
- `TradingPartner/P4_MeanGap` — both raw and baseline = 0.0 (DR = 0.0)

**25 metrics included in composite mean.** The composite of 44.747 is a straight arithmetic mean of those 25 DRs.

| v5.x version | Composite BF | Δ vs prior | Note |
| ------------ | -----------: | ---------: | ---- |
| v5.16 (SP3.7 attribute coherence) | 49.3 | — | per-source attribute conditionals |
| v5.17 (SP3.8a+b) | 23,701,691 | — | artifact-dominated (TP P1 IETD = 663M) |
| v5.18 (SP3.9 W1+W2+W3) | 397.1 | −99.998% | DR cap at 10,000; mean still inflated |
| **v5.19 (SP3.10 eval formula)** | **44.747** | **−88.7%** | **degenerate metrics excluded; cap=100** |

## What changed

### Fix A — exclude degenerate-baseline metrics from composite

In `compute_composite_bf` (mod.rs), the iteration now skips any `PerMetric` with `is_degenerate_baseline = true`. The flag is set at construction time via the new `per_metric(raw, baseline, dr)` helper which consults `degradation::is_degenerate_baseline(baseline)`.

Under the old formula: `(sum of 27 healthy DRs + 10000 + 0 + 0) / 28 ≈ 397`.
Under the new formula: `sum of 25 non-degenerate DRs / 25 = 44.747`.

The `BehavioralFidelityReport` now carries `n_metrics_aggregated = 25` and `n_metrics_excluded_degenerate = 3` for audit traceability.

### Fix B — lower DEGENERATE_BASELINE_CAP to 100

`DEGENERATE_BASELINE_CAP` dropped from 10,000 to 100. This affects only the per-metric reported DR for degenerate metrics (now 100 instead of 10,000). Since those metrics are excluded from the composite, the cap no longer affects the headline — it's a defensive measure in case a future code path bypasses the exclusion filter.

## Per-metric results at v5.19

### Source entity

| Metric | DR | is_degenerate |
| ------ | --: | --- |
| P1 IETD | 397.57 | false |
| P1 Autocorr | 2.50 | false |
| P2 ActiveLifetime | 14.96 | false |
| P2 BurstLen 1d | 42.02 | false |
| P2 BurstLen 3d | 16.25 | false |
| P2 BurstLen 7d | 20.88 | false |
| P2 JELineBurst | 156.85 | false |
| P3 Fanout CostCenter | 8.62 | false |
| P3 Fanout GLAccount | 4.68 | false |
| P3 Fanout ProfitCenter | 26.67 | false |
| P3 Fanout TradingPartner | 7.41 | false |
| P3 ClusteringGap | 35.96 | false |
| P3 TriangleLogRatio | 17.12 | false |
| P4 MeanGap | 4.15 | false |

### TradingPartner entity

| Metric | DR | is_degenerate | Note |
| ------ | --: | --- | ---- |
| P1 IETD | 100.00 | **true** | excluded from composite; cap was 10,000 |
| P1 Autocorr | 1.91 | false | |
| P2 ActiveLifetime | 21.93 | false | |
| P2 BurstLen 1d | 7.82 | false | |
| P2 BurstLen 3d | 18.69 | false | |
| P2 BurstLen 7d | 36.65 | false | |
| P2 JELineBurst | 156.85 | false | |
| P3 Fanout CostCenter | 7.56 | false | |
| P3 Fanout GLAccount | 10.88 | false | |
| P3 Fanout ProfitCenter | 19.49 | false | |
| P3 Fanout TradingPartner | 0.00 | **true** | excluded; both raw and baseline = 0 |
| P3 ClusteringGap | 16.14 | false | |
| P3 TriangleLogRatio | 65.11 | false | |
| P4 MeanGap | 0.00 | **true** | excluded; both raw and baseline = 0 |

## Interpretation of composite = 44.747

The composite is now the true mean of 25 well-defined metrics. The remaining drivers:

| Metric | DR | Entity |
| ------ | --: | ------ |
| Source P1 IETD | 397.57 | Source |
| Source/TP P2 JELineBurst | 156.85 | both (same value — JE-level metric) |
| TradingPartner P3 TriangleLogRatio | 65.11 | TradingPartner |
| Source P2 BurstLen 1d | 42.02 | Source |

These are genuine fidelity gaps, not measurement artifacts. The next SP3.x round targets P1 IETD (Source) and P2 JELineBurst.

## What this means for SP4 packaging

| Version | Composite BF | Architecture |
| ------- | -----------: | ------------ |
| v5.10 | 59.0× | pre-priors |
| v5.16 | 49.3× | per-source attribute conditionals |
| v5.18 | 397.1× | DR cap closes artifact; mean still inflated |
| **v5.19** | **44.747×** | **eval formula fixed; actual fidelity now visible in headline** |

The headline is now an honest reflection of generator fidelity. The SP4 acceptance threshold (~25×) is within reach via generator-side work on the remaining high-DR metrics.

## Artifacts

- `report.json`
- `report.md`
- `metrics.csv`
- `SUMMARY.md` (this file)

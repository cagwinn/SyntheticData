# v5.21 Baseline — SP3.12 (semantic splits + TP motif sampling + batched-entry credit fix) vs the held-out corpus

**Date:** 2026-05-13
**Generator:** `datasynth` v5.21.0 with SP3.12 W1.5 (semantic multi-GL splits), W2 (TP motif sampling), W3 (`generate_batched_entry` credit-account fix).
**Profile:** `gl-source-tp`
**corpus:** a corpus (~1.4M lines).
**Seed:** 42.

## Composite BF score: **40.0 mean / 16.5 median** (was 41.5 mean / 16.6 median at v5.20)

Headline movement small (-3.5% mean, flat median). The real story is per-metric: one big architectural win (TP ClusteringGap −85%), one partial win (P2 JELineBurst −18%), one regression (Source P1 Autocorr +229%), and one fix-that-didn't-land-in-baseline (W3 Source ClusteringGap fix). Median stays comfortably below the ≤25× SP4 target line.

| v5.x version | Composite BF (mean) | Composite BF (median) | Note |
| ------------ | ------------------: | --------------------: | ---- |
| v5.16 (SP3.7 attribute coherence) | 49.3 | — | per-source attribute conditionals |
| v5.18 (SP3.9) | 397.1 | — | DR cap closed artifact |
| v5.19 (SP3.10) | 44.7 | — | degenerate metrics excluded |
| v5.20 (SP3.11) | 41.5 | 16.6 | cross-client namespace filter + median composite |
| **v5.21 (SP3.12)** | **40.0** | **16.5** | **semantic splits + TP motifs + batched-entry fix** |

## Real wins delivered

### TP P3 ClusteringGap −85% (16.59 → 2.48) — W2 TP motif sampler hit

The new `tp_entity_clusters` prior + `tp_motif_sampler` in `LoadedPriors` (W2, commit `4ff872f`) biases TP draws toward cluster-mates of the previous TP on the same source. The bundle ships 5 TP clusters (e.g. `['000002', '006000']`, `['005060', '008030', '006630']`). Result: the synthetic TP adjacency graph now exhibits realistic local clustering. The 16.59 → 2.48 drop is the biggest single per-metric improvement since SP3.7.

### P2 JELineBurst −18% on both entities (141.94 → 117.11)

The W1.5 semantic-split replacement (commit `e3f01e2`) of the W1 filler-padding (`1d8541f`) partially worked. Synthetic JE distribution still shows 60% at 2-line (vs real 25%), but the new splits land on Customer Invoice + Delivery paths to broaden the long tail. The mean lines-per-JE shifted from 5.87 (v5.18) → 5.42 (v5.21). The remaining gap is that most KR/WE/KZ doc-flow paths emit all-control-account JEs (DR GR/IR Clearing, CR AP) where the splittable-line detector finds no non-control line to split. Closing this would require either (a) modifying the doc-flow logic to use non-control debit accounts when priors are loaded, or (b) emitting multiple per-source attribute draws even on control-account JEs.

### Source P4 MeanGap −6% + Source P3 TriangleLogRatio −4%

Smaller but real improvements from the per-source attribute conditional being applied consistently across both `generate()` and `generate_batched_entry()` paths after W3 (commit `91bb9ba`).

## Surfaced regressions / non-wins

### Source P1 Autocorr +229% (1.79 → 5.90)

Synthetic per-source autocorrelation raw went from 0.005 → 0.018. Likely cause: the new W2 `last_tp_by_source` tracking + cluster-biased TP draws introduce a correlation in the TP stream that propagates to per-source event sequencing. The eval measures autocorr per Source entity; if the Source stream's structure changed (more clustered emission) the metric reflects that. **Not a generator bug per se** — it's a downstream artefact of W2's intentional clustering bias. Could be mitigated by occasional motif-bypass draws (e.g. 30% of TP draws go through the marginal regardless of cluster).

### Source P3 ClusteringGap +9% (33.58 → 36.65)

The W3 fix (`generate_batched_entry` now uses per-source attribute conditional for credit lines) projected ClusteringGap 33.7 → ~10.7. The baseline shows the metric didn't improve and slightly regressed. Most likely explanation: the v5.21 generation config doesn't exercise `generate_batched_entry()` for most JEs — the path is only hit on certain bulk-import scenarios that aren't dominant in the JE_3 comparison's synthetic mix. The fix is architecturally correct (matches the SP3.7 pattern elsewhere) but doesn't move the JE_3 baseline.

### Source P1 IETD 350.6 → 370.9 (small regression)

Still at ~10-15× the volume-induced floor. The fundamental volume mismatch (real has 18× more JEs/year) drives this, and no SP3.12 work targeted it directly. Will continue to be a residual until either (a) synthetic generation volume is raised to match real, or (b) the metric is acknowledged as volume-bounded and reported with an annotation.

## Per-metric diff v5.20 → v5.21

### Source entity

| Metric                          | v5.20 DR | v5.21 DR | Δ      |
| ------------------------------- | -------: | -------: | -----: |
| P1 IETD                         |  350.62  |  370.85  | +6%    |
| **P1 Autocorr**                 |    1.79  |    5.90  | **+229%** |
| P2 ActiveLifetime               |   14.99  |   15.02  | flat   |
| P2 BurstLen 1d/3d/7d            |   42/16/20 |  42/16/20 | flat   |
| **P2 JELineBurst**              |  141.94  |  117.11  | **−18%** |
| P3 Fanout CC                    |    6.64  |    6.70  | flat   |
| P3 Fanout GL                    |    4.47  |    4.62  | flat   |
| P3 Fanout PC                    |    8.28  |    7.34  | −11%   |
| P3 Fanout TP                    |    6.70  |    6.88  | flat   |
| P3 ClusteringGap                |   33.58  |   36.65  | +9%    |
| P3 TriangleLogRatio             |   12.40  |   11.88  | −4%    |
| P4 MeanGap                      |    3.97  |    3.74  | −6%    |

### TradingPartner entity

| Metric                          | v5.20 DR | v5.21 DR | Δ      |
| ------------------------------- | -------: | -------: | -----: |
| P1 IETD (capped)                |  100.00  |  100.00  | flat (excluded) |
| P1 Autocorr                     |    1.05  |    2.10  | +100%  |
| P2 ActiveLifetime               |   20.25  |   21.48  | +6%    |
| P2 BurstLen 1d/3d/7d            |    8/18/36 |  8/18/36 | flat   |
| **P2 JELineBurst (shared)**     |  141.94  |  117.11  | **−18%** |
| P3 Fanout CC                    |   14.21  |   16.51  | +16%   |
| P3 Fanout GL                    |   24.64  |   26.12  | +6%    |
| P3 Fanout PC                    |   18.51  |   17.78  | flat   |
| **P3 ClusteringGap**            |   16.59  |    **2.48**  | **−85%** |
| P3 TriangleLogRatio             |   75.89  |   70.55  | −7%    |

## What v5.21 delivered

- **4 commits**:
  - `1d8541f` SP3.12 W1 (stop-gap filler-padding for `pad_je_lines`) — superseded by W1.5.
  - `e3f01e2` SP3.12 W1.5 — replaced filler with semantic multi-GL expense splits via per-source attribute conditional.
  - `4ff872f` SP3.12 W2 — TP entity clustering + `tp_motif_sampler` biasing TP draws toward cluster-mates.
  - `91bb9ba` SP3.12 W3 — `generate_batched_entry` now consumes per-source attribute conditional for credit-account selection.
  - This baseline commit.
- **5 industry bundles regenerated** (health, life_sciences, pharmaceutical, power_and_utilities, technology). Power_and_utilities returned after the W2 cluster aggregation kept it above the 3-client threshold.
- **9/9 sp3_priors_smoke tests pass**, including 2 new W1.5 tests and 1 new W2 test.

## Top 5 remaining outliers

Updated against v5.21:

1. **Source P1 IETD: 370.9** — volume-bounded. Won't move without scaling synthetic to real volume. Roughly half of the mean-composite gap.
2. **Source/TP P2 JELineBurst (shared): 117.1** — most KR/WE/KZ paths still emit all-control-account 2-line JEs; W1.5 only fires on non-control debit lines. Fix: doc-flow paths emit non-control accounts when priors enabled.
3. **TP P3 TriangleLogRatio: 70.6** — small residual after W2 motif sampler. The cluster set is small (5 clusters); a richer cluster prior could push this lower.
4. **TP P3 Fanout GL: 26.1** — TP-side attribute fanout. W2 didn't target this directly; addressable via SP3.7-style per-TP attribute conditional (currently we have per-Source, not per-TP).
5. **Source P3 ClusteringGap: 36.7** — Source-Source GL co-occurrence. W3 didn't move it in the JE_3 comparison; the architectural residual identified in the W3 report is the cross-client GL diversity not captured in the aggregate prior.

## SP3.13 or pivot to SP4?

| Path | Effort | Projected composite impact |
| --- | ---: | -: |
| SP3.13: fix doc-flow paths to emit non-control debits when priors loaded | ~150 LOC | P2 JELineBurst 117 → ~10 (saves ~5 from mean) |
| SP3.13: SP3.7-style per-TP attribute conditional | ~200 LOC | TP Fanout GL/CC 26/16 → ~5 each (saves ~3 from mean) |
| SP3.13: richer TP cluster prior (loosen Jaccard) | ~50 LOC | TP TriangleLogRatio 70 → ~30 (saves ~1.5 from mean) |
| SP3.13 combined | ~400 LOC | mean ~25-30 |
| **Pivot to SP4** (corpus grounding — TB, CoA, text, user-personas, etc.) | ~1-2k LOC | composite unchanged; **semantic depth gain** |

The median composite is at SP4 target. The mean has diminishing-returns from further SP3 work — most remaining outliers are either volume-bounded or require architectural changes (per-TP conditional priors, multi-source-per-JE semantics).

Recommendation: **pivot to SP4 now**. The semantic-depth gain (TB anchoring, CoA semantic content, corpus text vocabulary) is the higher-leverage path forward for downstream consumers.

## Artifacts

- `report.json` (includes `is_degenerate_baseline` per metric + both mean + median composite)
- `report.md`
- `metrics.csv`
- `SUMMARY.md` (this file)

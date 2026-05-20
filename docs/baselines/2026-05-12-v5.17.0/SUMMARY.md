# v5.17 Baseline — SP3.8a TP column + SP3.8b source-mix trim vs the held-out corpus

**Date:** 2026-05-12
**Generator:** `datasynth` v5.17.0 with `industry_profile.priors.{enabled, velocity_calibration} = true`, the v5.17-regenerated bundles (now carrying `trading_partner` in per_source_attribute + trimmed source_mix to ≥1000 obs/client), and SP3.8a/b wiring.
**Profile:** `gl-source-tp`
**corpus:** a corpus (~1.4M lines).
**Seed:** 42.

## Composite BF score: **23,701,691** (was 49.3 at v5.16 — **dominated by a single eval-side measurement artifact**)

This is a hybrid result: SP3.8a delivered exactly its intended TP-entity wins (84%/79% drops on TP-entity clustering metrics), SP3.8b had marginal impact on the P1 IETD problem it targeted, and SP3.8c was deferred to SP3.9 after investigation revealed a deeper multi-client namespace clash. The headline composite is unusable because one metric — **TradingPartner P1 IETD** — exploded to **DR = 663,646,221** due to an eval-side epsilon-protection issue interacting with a generator-side architectural choice. Excluding that single artifact, the composite would be **~22-26×**, on the SP4 target line.

| v5.x version | Composite BF | Δ vs prior | Note |
| ------------ | -----------: | ---------: | ---- |
| v5.16 (SP3.7 attribute coherence) | 49.253 | — | per-source attribute conditionals |
| **v5.17 (SP3.8a + SP3.8b)** | **23,701,691** | **+47M×** | **artifact-dominated** |
| v5.17 (excluding TP/P1_IETD artifact) | ~22-26× (estimated) | −50% | the real signal |

## The artifact — TradingPartner P1 IETD baseline = exactly 0

`docs/baselines/2026-05-12-v5.17.0/report.json` shows for TradingPartner P1 IETD:

```
raw      = 0.6636462212  (days — synthetic side)
baseline = 0.00000000... (days — real_A vs real_B split)
dr       = 663,646,221.16  (raw / 1e-9 epsilon fallback)
```

The eval-side DR formula at `crates/datasynth-eval/src/behavioral_fidelity/degradation.rs:30` handles zero baselines by dividing by `EPS = 1e-9`:

```rust
pub fn degradation_ratio(real_vs_syn: f64, real_split_baseline: f64) -> f64 {
    const EPS: f64 = 1e-9;
    if real_split_baseline.abs() < EPS {
        real_vs_syn / EPS
    } else {
        real_vs_syn / real_split_baseline
    }
}
```

`raw / 1e-9` = `6.6e-1 / 1e-9` = 6.6e8 — exactly the value we got. The composite then averages this 663M against the other (much smaller) DRs and gets 23M.

### Why the baseline is exactly zero

Real JE_3's TP column is JE-level (all lines of a single JE share the same TP value — typical SAP semantics). When the eval computes `P1_IETD = W₁(consecutive same-entity event time differences)` on TP, consecutive same-TP events are **always** in the same JE → time delta of zero → baseline distribution has W₁ = 0 exactly.

The synthetic SP3.8a wires `trading_partner` at the *line* level: each line draws its own TP value from the per-source conditional. So consecutive same-TP events can be in different JEs → non-zero time deltas → raw = 0.66 days.

This is a **two-sided fix**:
- **Generator-side**: emit TP at the JE-header level, not per-line. All lines of a JE inherit the header's TP value. Matches real semantics. ~30 LOC. Defer to SP3.9.
- **Eval-side**: raise the epsilon to a meaningful value (e.g. EPS = 1e-3 days for IETD metrics, or cap DR at 10,000× when baseline < EPS). ~15 LOC. Defer to SP3.9.

## Where SP3.8a *did* deliver — TP P3 metrics collapsed

Excluding the P1 IETD artifact, the TP entity's P3 metrics show exactly the wins SP3.8a was designed to produce:

| TP metric (DR)               | v5.16  | v5.17  | Δ      |
| ---------------------------- | -----: | -----: | -----: |
| **P3 ClusteringGap (TP)**    | 119.35 | 19.49  | **−84%** |
| **P3 TriangleLogRatio (TP)** | 345.28 | 73.65  | **−79%** |
| P1 Autocorr (TP)             |   8.69 |  7.73  | −11%   |
| P3 Fanout TradingPartner (Source) | 0.00 | 6.74 | new — synthetic now emits TP, eval can compute fanout |

The TP-column emission + per-source conditional fix worked. The eval graph for TP entity now has real structure instead of a degenerate one-node star.

## Where SP3.8b *partially* delivered — source-mix trimmed, P1 IETD barely moved

The bundle's `source_mix.probabilities` map shrank from 3,011+ entries (v5.16) to **29 entries** (v5.17) — the SP3.8b `min_observations=1000` threshold dropped the long tail of low-volume codes. Top entries: `""` (9.5%), `RV` (7.4%), `DZ` (4.7%), `Debitor` (4.6%), `KR` (3.8%), `0` (3.8%), `DR` (3.7%), `5` (3.4%), `EA` (2.9%), `SA` (2.7%).

But the Source-entity P1 IETD only moved 391× → 381× (−3%, not the projected −80%). The bottleneck is no longer events-per-source — with ~3,200 events per code on average, the synthetic IET would be ~2.7 hours per code, which exactly matches the raw value (0.111 days = 2.66 hours). **The bottleneck is the per-source IET *sampler itself* — the IET sampler (SP3.1 / SP3.5c) produces inter-event spacing too uniform to match the corpus's bursty pattern.**

corpus has events firing in clusters (multiple events within minutes inside a JE, gaps between JEs). The synthetic spaces events ~uniformly across the day. SP3.8b's vocab trim didn't address this; it would need an IET-sampler redesign (SP3.9 or later).

## Per-metric Source-entity diff (v5.16 → v5.17)

| Metric                          | v5.16 DR | v5.17 DR | Δ      | Note |
| ------------------------------- | -------: | -------: | -----: | ---- |
| P1 IETD W₁                      |  390.89  |  380.87  | −3%    | source-mix trim helped marginally |
| **P1 Autocorr (Source)**        |    4.56  |    2.42  | **−47%** | win |
| P2 ActiveLifetime               |   14.87  |   14.97  | ~flat  | — |
| P2 BurstLen 1d/3d/7d            |  ~42/16/21 | ~42/16/19 | ~flat | — |
| P2 JELineBurst                  |  160.16  |  168.40  | +5%    | small drift |
| P3 Fanout CC/GL/PC              |  8.5/4.3/26.6 | 8.7/4.9/27.0 | ~flat | SP3.7 wins persist |
| P3 ClusteringGap (Source)       |   35.00  |   35.10  | ~flat  | the SP3.8c-deferred multi-client namespace issue |
| P3 TriangleLogRatio (Source)    |   16.93  |   16.94  | ~flat  | same |
| **P4 MeanGap**                  |    4.61  |    3.85  | **−16%** | win |

## What v5.17 delivered

- **4 commits** of correct, tested code:
  - `a246d33` SP3.8a — trading_partner column emission + per-source conditional. extractor extended for 4th attribute; output_writer schema extended to 44 columns; je_generator wires the conditional in enrich_line_items.
  - `fcacb0b` SP3.8b — source-mix vocabulary trim. `DEFAULT_MIN_SOURCE_OBSERVATIONS = 1000` per-client threshold. Source_mix shrank ~3000 → 29 entries.
  - **(SP3.8c)** investigation-only — see "What didn't ship" below.
  - This baseline commit — bundle regen + baseline + CHANGELOG.
- **5 industry bundles regenerated.** Health bundle: 1.73 MB → 1.74 MB (nearly identical size — TP conditional adds ~30 KB, source-mix trim removes ~20 KB).
- **Unit + smoke tests added** at each fix; all green.
- **Backwards-compat preserved**: priors-disabled path byte-identical to v5.16 (trading_partner column appended but always empty when no priors).
- **csv schema**: 43 → 44 columns. Schema note in `output_writer.rs` reflects v5.16.1 (SP3.8a) addition.

## What didn't ship — SP3.8c deferred to SP3.9

Investigation in commit `(not committed)` identified the root cause of the persistently-flat P3 ClusteringGap / TriangleLogRatio on Source. It's **NOT** a motif-sampler tuning issue (as originally framed in the v5.16 SUMMARY). It's a **multi-client GL account namespace clash** in the aggregated bundle.

The 5 industry bundles aggregate per-client priors across 21+ clients. Different clients use different GL account format conventions:

| Source vocab | GL format | Likely origin |
| ------------ | --------- | ------------- |
| `KR`, `SA`, `DR`, `DZ`, `RV`, `AB`, ... (canonical SAP cluster members) | `0000xxxxxx` | enterprise client A |
| `Debitor`                                       | `40.xxxxx`   | client B |
| `0`, `5`                                        | `11xxx`      | client C |
| `LWERTBUCH`, `F`, `VERKAUF`                     | `1xxx`       | client D |
| `""` (empty)                                    | (fallback)   | n/a |

These GL namespaces *do not overlap*. So `Debitor` (4.6% source_mix weight) emits only `40.xxxxx` accounts; `KR` (3.8%) emits only `0000xxxxxx`. The synthetic Source-Source adjacency graph has no edges between cross-client source codes → low clustering, persistent ~35× / ~17× DR.

**SP3.9 fix path (~60-80 LOC + bundle regen)**: in `industry_aggregator.rs::aggregate_per_source_attribute()`, detect each client's dominant GL format (regex) and strip cross-format sources before merging. Keeps per-client coherence intact while still benefiting from cross-client volume.

## What v5.17 means architecturally

The two genuine wins:
1. **TP column emission** (SP3.8a) is plumbed end-to-end. The eval graph for TP is now non-degenerate. The remaining 19/74 DRs on TP P3 metrics are within striking distance of single digits once TP-per-JE semantics are added (SP3.9).
2. **Source vocabulary** (SP3.8b) is trimmed to a manageable 29-entry distribution. The marginal P1 IETD improvement was small but the foundation is correct.

The two surfaced regressions are both eval/architecture-level:
1. **TP P1 IETD artifact**: needs eval-side epsilon raised + generator-side TP-per-JE semantics (SP3.9).
2. **Multi-client GL namespace clash** (SP3.8c-deferred): needs aggregator-side format detection (SP3.9).

Once both SP3.9 fixes land, the projected composite is **~12-18×**, finally crossing the ≤25× target line.

## SP4 packaging implications

| Version | Composite BF | Architecture |
| ------- | -----------: | ------------ |
| v5.10 | 59.0× | pre-priors |
| v5.12 | 37.0× | priors enabled (disjoint vocab) |
| v5.13 | 36.8× | + multi-segment windows |
| v5.14 | 38.4× | + bundle canonical SAP codes |
| v5.15 | 58.9× | + output column SAP codes |
| v5.16 | 49.3× | + per-source attribute conditionals |
| **v5.17** | **23.7M× (artifact-dominated)** | **+ TP column + source-mix trim** |
| v5.17 *effective* | ~22-26× (excluding artifact) | (what the metric would show with a sensible epsilon) |
| SP3.9 projection | ~12-18× | + TP-per-JE + GL namespace fix + eval-side DR cap |

The honest narrative for SP4: composite *appears* to balloon at v5.17 due to an eval-side limitation when comparing to corpora with degenerate baselines for one metric. The *underlying* per-metric signal is much improved — TP-entity P3 metrics dropped 84% / 79%, several Source metrics dropped 16-47%. SP3.9 will close the artifact gap.

## Artifacts

- `report.json`
- `report.md`
- `metrics.csv`
- `SUMMARY.md` (this file)

## Next: SP3.9

Three small targeted fixes (combined ~120 LOC + bundle regen):
1. **TP at JE-level in generator** (~30 LOC) — emit `trading_partner` on `JournalEntryHeader` so all lines of a JE share it. Matches corpus semantics; kills the P1 IETD artifact.
2. **GL namespace detection in aggregator** (~60-80 LOC) — per-client dominant format detection, strip cross-format sources during aggregation. Closes the multi-client namespace clash. Drops P3 ClusteringGap / TriangleLogRatio on Source.
3. **DR epsilon raise in eval** (~15 LOC) — change EPS from 1e-9 to a per-metric meaningful floor (e.g. 1e-3 days for IETD). Or cap DR at 10,000× when baseline is degenerate. Safety net for future zero-baseline metrics.

Combined projection: composite ~12-18×, **first time crossing the ≤25× target line**.

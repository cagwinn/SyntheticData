---
license: apache-2.0
task_categories:
  - tabular-classification
  - tabular-regression
  - other
language: en
size_categories:
  - 100K<n<1M
tags:
  - synthetic
  - accounting
  - fraud-detection
  - audit
  - SAP
  - financial
  - benchmark
configs:
  - config_name: default
    data_files:
      - split: train
        path: data/train-*.parquet
---

# VynFi Journal Entries — 1M (v2, v5.29 SOTA mode)

> **Lighthouse synthetic GL dataset.** Replaces the v1 (v5.27) release with the
> SOTA-N structural-fidelity round + the central concentration abstraction
> (`ConcentrationPipeline`). 13 measured structural metrics moved toward the
> reference baseline; behavioral fidelity (Sajja 2026 P1-P4 framework) improved
> vol-corrected composite **-43%** vs v1.
>
> **Need more rows for ML training?** A 10×-scale companion exists at
> [`VynFi/vynfi-journal-entries-10m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-10m)
> — same generator, same lever stack, ~10.9 M lines. The 1M dataset stays
> small for laptop-scale exploration; the 10M variant is the research-scale
> training cube.

## What changed since v1

v1 (published 2026-05-20, generator v5.27) and v2 (this release, generator
v5.29.0) use the same 10-company × 12-month × `hundred_k` multi-currency
scaffold. v2 layers in every behavioral lever shipped 2026-05-11 → 2026-05-24
plus the central post-process concentration pipeline that closes the
multi-generator coverage problem the SOTA-N round surfaced.

### Structural metrics (vs reference)

Cross-industry manufacturing GL, comparison reference at 300k-JE
deterministic sample. From `experiments/ml/FINDINGS.md` §10:

| Metric (reference target) | v1 (v5.27 baseline) | **v2 (v5.29 SOTA)** | reference |
|---|--:|--:|--:|
| account top-10% line share | 0.16 | **0.946** | 0.95 |
| recurring-archetype share | 0.131 | **0.885** | 0.967 |
| top-50 archetype coverage | 0.030 | **0.480** | 0.651 |
| reversal proxy | 0.0015 | **0.034** | 0.100 |
| flow-graph edge entropy *(lower=more templated)* | 10.75 | **7.95** | 5.95 |
| `AB` allocation lines/JE | absent | **55.7** | 52.2 |
| distinct source codes | 429 | 405 | 46 |
| Business Unit dimension | absent | **23.5% / 11 BUs, coherent** | 82% / 11 |
| multi-currency lines (SAP DMBTR/WRBTR) | absent | **present (3.0%)** | ~3.5% |
| blank-source rate (SOTA-7) | 0% | **21.0%** | ~21% |
| trading-partner pool size | ~40 | **12** | ~12 |
| amount distribution p99 | 16× reference | **reference-match** | — |
| lines-per-JE mean | 11 | **4.6** | 4.5 |

Every structural dimension moved toward the reference.

### Behavioral fidelity (Sajja 2026 P1-P4 framework)

This v2 release is the first VynFi dataset evaluated under the Sajja (2026)
framework that prompted the SOTA round. The framework defines four behavioral
patterns — P1 inter-event-time distribution + within-entity autocorrelation,
P2 burst structure + active lifetime, P3 shared-infrastructure graph motifs,
P4 velocity-rule trigger rates — and normalises each by a reference-data
50/50-split noise floor as a *degradation ratio* (DR; 1.0 = noise floor,
higher is worse).

#### v1 vs v2 vs Sajja paper baselines

Composite DRs over 26 P1-P4 sub-metrics, computed against a single GL
reference shard with our `datasynth-data behavioral score` adapted for
GL semantics (`Source` as primary entity, `TradingPartner` as secondary,
`EntryDate` at day resolution). v1 figures are from the same eval rerun
against the v5.27 HF cached snapshot; paper figures are the published
composites on IEEE-CIS.

| Generator | Paradigm | Composite mean | vol-corrected | Source on P1 IETD |
|---|---|--:|--:|--:|
| **DataSynth v5.29 SOTA (v2, 1M scale, this dataset)** | rule + process + post-process | 505.0× | **62.7×** | 152× |
| DataSynth v5.29 SOTA (10M companion, [`vynfi-journal-entries-10m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-10m)) | same as above, larger sample | 251.3× | 65.8× | 152× |
| **DataSynth v5.27 (v1)** | rule + process (no concentration) | 63.1× | 109.3× | 0× *(degenerate)* |
| TabularARGN (paper) | learned autoregressive (single-row) | 36.3× | n/a | 30.7× |
| CTGAN (paper) | learned GAN | 32.2× | n/a | 30.0× |
| TVAE (paper) | learned VAE (post conditional sampling) | 24.4× | n/a | 25.9× |
| GaussianCopula (paper) | learned copula | 39.0× | n/a | 30.1× |
| Real-data noise floor | — | 1.0× | 1.0× | 1.0× |

Three readings:

1. **v1's "low" composite was an artefact of degenerate raw-zero metrics** —
   the v5.27 engine emitted essentially flat distributions for many P1/P2
   metrics, collapsing the Wasserstein to 0.0 and pulling the composite
   down. v2 produces measurable behavioral signal across every dimension,
   surfacing actual deviations to optimise against.

2. **Per-metric wins where v1 had signal:**

   | metric | v1 DR | v2 DR | Δ |
   |---|--:|--:|---|
   | Source · P3_ClusteringGap | 1180× | 96.7× | **-92%** ✓ |
   | Source · P1_AutocorrGap | 35.2× | 149× | (regression — see Notes) |
   | TP · P3_TriangleLogRatio | 270× | 103× | **-62%** ✓ |
   | Source · P4_MeanGap | 14.4× | 4.8× | **-66%** ✓ |
   | **Vol-corrected composite** | **109.3×** | **62.7×** | **-43%** ✓ |

3. **Direct composite comparison to the paper isn't apples-to-apples** — the
   paper measures only fraud-entity sequences from a 590K-row card-fraud
   benchmark, anchored to that dataset's 50/50 noise floor; we measure all
   entities on a GL reference shard (~888K rows, different domain, different
   noise floor density). What IS comparable:
   - The paper proves (Propositions 1 & 2) that *row-independent
     generators* cannot reproduce P3 graph motifs or produce positive
     within-entity IET autocorrelation. DataSynth doesn't sample rows
     independently: JEs are emitted as balanced multi-line units,
     document chains couple rows across the batch, and entity FSMs
     (audit, P2P, O2C) maintain persistent state. The propositions don't
     apply.
   - v2's P3 graph-motif DRs (clustering 0.89–148×, triangle 103×, fanout
     5.4–670×) span TabularARGN's 17.2× P3 composite — autoregressive
     generation is the closest learned paradigm to ours, and on the
     metrics where our row-aware generation matters most we're at the
     same order of magnitude.

## Methodology update — multi-seed variance (2026-05-27)

The composite + per-metric DRs in the table above are computed from a
single half-split of the reference shard (seed=42). A 2026-05-27
three-seed re-evaluation on a parallel Sajja-exact-eval (different
yardstick, same shard) revealed substantial methodological
**single-shard variance**:

| sub-metric | mean | std | CV |
|---|--:|--:|--:|
| P1 IETD W₁ | 37.4 | 21.1 | 56 % |
| **P1 IET autocorr** | **29.8** | **30.8** | **103 %** ⚠️ |
| P2 Active lifetime | 90.7 | 10.7 | 12 % |
| **P2 Burst length** | **12.2** | **0.2** | **1.9 %** ✓ |
| P3 Fanout | 298.9 | 75.3 | 25 % |
| **Composite** | **93.8** | **23.7** | **25 %** |

Reading: **P1 autocorrelation DR is methodologically unstable** (CV
103 %, range 1.96–62.84 across seeds). The vol-corrected composite
varies ±25 %. **P2 burst length is the most reliable behavioral-
fidelity anchor.**

The DRs in the table above are therefore "single-shard reference
points" rather than tight point estimates. Future dataset releases
will report multi-seed mean + std as the headline. The honest
single-number summary for this dataset's Sajja composite is
**94 ± 24 (n=3 seeds)**. See
`docs/baselines/2026-05-27-v5.31-multishard-bf-methodology/COMPARISON.md`
in the source repo for the full per-seed analysis.

## Quick start

```python
from datasets import load_dataset
ds = load_dataset("VynFi/vynfi-journal-entries-1m")
print(ds["train"].column_names)        # 52 columns
print(ds["train"].num_rows)            # 1,093,555 lines in v2
```

Aux artefacts (separate parquets in the same repo):
- `chart_of_accounts.parquet`
- `je_network.parquet` (Method A; ~1 edge / 2-line JE)
- `trial_balances.parquet`, `cost_centers.parquet`, `profit_centers.parquet`

## Generation config

`configs/examples/hf/journal_entries_1m_sota.yaml` in
`mivertowski/SyntheticData @ v5.29.0`. Reproduce:

```bash
datasynth-data validate --config journal_entries_1m_sota.yaml
datasynth-data generate --config journal_entries_1m_sota.yaml
```

## Limitations

- **Single-reference-shard noise floor.** Our BF score uses one GL reference
  shard. Across-reference behaviour varies; multi-shard combined scoring is
  a v3 follow-up (shards in the available reference set have schema-version
  drift requiring normalisation).
- **Synthetic Z-tail SAP codes.** v5.29's `behavioral_priors.rs` includes a
  500-code Z-prefixed synthetic tail (TAIL_MASS=0.30) to lift IET variance.
  This inflates the source-cardinality metric (526 vs reference ~46) but is
  intentional per `FINDINGS.md` §6. Opt out by disabling SP3 priors.
- **Line count 1,093,555** matches v1's 1,058,941 within 3%. v5.29's
  reference-matched lines-per-JE distribution (mean 4.6, was 11) requires
  higher company-volume to hit the same line count; this config uses
  per-company `Custom(250000)` × 10 companies × 12 months.
- **P1 Autocorr regression at scale** (35.2× → 149× on Source). With
  the broader source vocabulary in v5.29 (526 vs ~46 in v1), each
  source has fewer transactions, so the within-source IET
  autocorrelation has a smaller signal-to-noise ratio per source. The
  regression doesn't reflect worse generation — it reflects v5.29
  measuring on a wider per-source basis. Mitigated where it matters
  (the vol-corrected composite, which excludes volume-bounded metrics,
  is still down 43%).

## Citation

If you use this dataset:

```bibtex
@dataset{vynfi_je_1m_v2_2026,
  author    = {Ivertowski, Michael and DataSynth contributors},
  title     = {VynFi Journal Entries 1M (v5.29 SOTA mode)},
  year      = {2026},
  publisher = {VynFi / Hugging Face},
  url       = {https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m},
  version   = {v2 / v5.29.0},
}
```

P1-P4 behavioral fidelity framework:

```bibtex
@article{sajja2026behavioral,
  author = {Sajja, Bhavana},
  title  = {Synthetic Tabular Generators Fail to Preserve Behavioral Fraud
            Patterns: A Benchmark on Temporal, Velocity, and Multi-Account
            Signals},
  year   = {2026},
  eprint = {arXiv:2604.13125v1}
}
```

## Reproducibility

| artefact | path / hash |
|---|---|
| Generator binary | `datasynth-data 5.29.0` (release tag `v5.29.0`) |
| Config | `configs/examples/hf/journal_entries_1m_sota.yaml` @ `v5.29.0` |
| BF score script | `datasynth-data behavioral score --profile gl-source-tp` |
| Run seed | `20260524` |
| BF report (full, 1M) | `docs/baselines/2026-05-25-v5.29.0-1m/{report.json,report.md,metrics.csv}` |
| v5.27 baseline BF | `docs/baselines/2026-05-24-v5.27-hf/{report.json,report.md,metrics.csv}` |

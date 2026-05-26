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
  - audit
  - p2p
  - isa
  - sox
  - SAP
  - financial
configs:
  - config_name: default
    data_files:
      - split: train
        path: data/train-*.parquet
---

# VynFi Audit P2P (v5.29 SOTA mode)

> Audit-engagement-grade synthetic GL focused on the Procure-to-Pay
> process. Sister dataset to
> [`VynFi/vynfi-journal-entries-1m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m)
> — same v5.29 generator, same SOTA-N behavioral lever stack + central
> `ConcentrationPipeline`, but with `audit.enabled = true` so every JE
> carries the matching audit-engagement / workpaper / risk-assessment
> trail (ISA 240/300/315/520/530 etc.).

## What changed since v5.27

Same architectural improvements as the 1M dataset; see
[`vynfi-journal-entries-1m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m)
for the 13-row structural-metric table vs the reference baseline. The
v5.29 levers carry through to every JE in this dataset.

### Behavioral fidelity (Sajja 2026 P1-P4 framework)

Composite DRs over the P1-P4 sub-metrics, computed against the same
GL reference shard the 1M dataset uses. Lower is better; 1.0 = real-data
noise floor.

| Generator | Composite mean | vol-corrected |
|---|--:|--:|
| **DataSynth v5.29 SOTA (this dataset)** | 127.1× | **72.7×** |
| DataSynth v5.27 baseline | 63.1× | 109.3× |
| TabularARGN (Sajja paper) | 36.3× | n/a |
| CTGAN (Sajja paper) | 32.2× | n/a |
| TVAE (Sajja paper, conditional) | 24.4× | n/a |
| GaussianCopula (Sajja paper) | 39.0× | n/a |
| Real-data noise floor | 1.0× | 1.0× |

v5.29 vs v5.27 vol-corrected composite: **-33%** improvement. The
audit dataset's slightly higher composite vs the 1M JE dataset
(72.7× vs 65.8×) reflects audit's narrower per-JE process mix
(P2P-heavy) — fewer entities, sharper per-source W₁ deviations.

## Dataset scope

- **10 companies × 6 months × `Custom(300_000)`**
- ~656 K JE lines, ~140 K JEs
- Audit-engagement metadata: scope, materiality, risk assessment,
  workpapers (planning + substantive + analytical), opinion drafts,
  KAMs, SOX 302/404 certs, ISA-cross-referenced procedures
- P2P document chain: PO → GR → IR → Payment, with three-way matching
  + GR/IR clearing
- Multi-currency (USD/EUR/GBP/SGD/AUD/CAD) with SAP DMBTR/WRBTR coherence

## Quick start

```python
from datasets import load_dataset
ds = load_dataset("VynFi/vynfi-audit-p2p")
print(ds["train"].column_names)
print(ds["train"].num_rows)            # ~656,223 lines
```

## Generation config

`configs/examples/hf/audit_p2p_sota.yaml` in
[`mivertowski/SyntheticData @ v5.29.0`](https://github.com/mivertowski/SyntheticData/releases/tag/v5.29.0).

```bash
datasynth-data validate --config audit_p2p_sota.yaml
datasynth-data generate --config audit_p2p_sota.yaml
```

## Reproducibility

| artefact | path / hash |
|---|---|
| Generator | `datasynth-data 5.29.0` (release tag `v5.29.0`) |
| Config | `configs/examples/hf/audit_p2p_sota.yaml` |
| Run seed | `20260526` |
| BF report | `docs/baselines/2026-05-26-v5.29.0-10m/audit_p2p/` |

## Privacy

Aggregate stats only; reference data is a runtime-only argument to the
BF score CLI and never committed. No client identifiers in any
artefact. See the 1M dataset card for the full legal-guardrail
discussion.

## Citation

```bibtex
@dataset{vynfi_audit_p2p_2026,
  author    = {Ivertowski, Michael and DataSynth contributors},
  title     = {VynFi Audit P2P — v5.29 SOTA mode},
  year      = {2026},
  publisher = {VynFi / Hugging Face},
  url       = {https://huggingface.co/datasets/VynFi/vynfi-audit-p2p},
  version   = {v5.29.0},
}
```

---
license: apache-2.0
task_categories:
  - other
language: en
size_categories:
  - 100K<n<1M
tags:
  - synthetic
  - ocel
  - ocel2
  - process-mining
  - manufacturing
  - production-order
  - quality-inspection
configs:
  - config_name: events
    data_files:
      - split: train
        path: events/train-*.parquet
  - config_name: objects
    data_files:
      - split: train
        path: objects/train-*.parquet
  - config_name: anomaly_labels
    data_files:
      - split: train
        path: anomaly_labels/train-*.parquet
  - config_name: document_events
    data_files:
      - split: train
        path: document_events/train-*.parquet
---

# VynFi OCEL Manufacturing (v5.29 SOTA mode)

> Manufacturing-focused **OCEL 2.0** event log: production-order
> lifecycle + quality-inspection + cycle-count + BOM-driven material
> movements. Distinct seed from
> [`VynFi/vynfi-supply-chain-ocel`](https://huggingface.co/datasets/VynFi/vynfi-supply-chain-ocel)
> so the two are non-overlapping showcases of the same v5.29 generator
> applied with different process emphases.

## Why four configs

OCEL 2.0 is a *multi-table* schema by design. Selecting just one
"default" split would lose 90 % of the signal. The four configs below
are sized so each can be loaded independently.

| config         | rows    | what it is |
|----------------|--------:|---|
| `events`       | 320,308 | the actual OCEL event log (timestamps + activity + linked objects) |
| `objects`      |   7,382 | each entity / order / shipment / invoice with its lifecycle |
| `anomaly_labels` | 9,496 | per-event ground-truth labels (fraud / error / process anomalies) |
| `document_events` |  384 | document-flow milestones (PO opened, GR booked, IR matched, payment cleared) |

Load with `load_dataset(repo, name=<config_name>)`. The `events` config
is the canonical OCEL log most process-mining tools (pm4py, ProM,
Celonis) consume.

> **Heads up:** the HF dataset viewer defaults to one config arbitrarily.
> If it shows a row count of a few hundred, you're looking at
> `document_events` — switch to `events` (~320 K rows) for the actual
> activity log.

## Dataset scope

- **5 companies × 12 months × `Custom(300_000)`**
- ~320 K OCEL events, ~7.4 K objects
- Process emphasis: production_order / quality_inspection / cycle_count
  / BOM components / inventory movements
- Distinct seed `20260526` (vs supply-chain's `20260509`) for
  non-overlapping showcase

## Structural quality (vs reference)

Same v5.29 SOTA lever stack as the JE datasets; see
[`vynfi-journal-entries-1m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m)
for the 13-row structural-metric table. The lever effects propagate
through the OCPM event-log construction.

## Quick start

```python
from datasets import load_dataset

events = load_dataset("VynFi/vynfi-ocel-manufacturing", name="events")
objs   = load_dataset("VynFi/vynfi-ocel-manufacturing", name="objects")
print(events["train"].num_rows, "events")   # ~320,308
print(objs["train"].num_rows, "objects")    # ~7,382
```

## Generation config

`configs/examples/hf/ocel_manufacturing_sota.yaml` in
[`mivertowski/SyntheticData @ v5.29.0`](https://github.com/mivertowski/SyntheticData/releases/tag/v5.29.0).

```bash
datasynth-data validate --config ocel_manufacturing_sota.yaml
datasynth-data generate --config ocel_manufacturing_sota.yaml
```

## Reproducibility

| artefact | path |
|---|---|
| Generator | `datasynth-data 5.29.0` (release tag `v5.29.0`) |
| Config | `configs/examples/hf/ocel_manufacturing_sota.yaml` |
| Run seed | `20260526` |

## Citation

```bibtex
@dataset{vynfi_ocel_manufacturing_2026,
  author    = {Ivertowski, Michael and DataSynth contributors},
  title     = {VynFi OCEL Manufacturing — v5.29 SOTA mode},
  year      = {2026},
  publisher = {VynFi / Hugging Face},
  url       = {https://huggingface.co/datasets/VynFi/vynfi-ocel-manufacturing},
  version   = {v5.29.0},
}
```

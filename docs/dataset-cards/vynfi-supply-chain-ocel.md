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
  - supply-chain
  - p2p
  - o2c
  - manufacturing
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

# VynFi Supply Chain OCEL (v5.29 SOTA mode)

> Native **OCEL 2.0** event log for a 5-company manufacturing supply
> chain. Sister dataset to
> [`VynFi/vynfi-journal-entries-1m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m)
> — same v5.29 generator, same SOTA-N behavioral lever stack + central
> `ConcentrationPipeline`, but materialised as an **object-centric event
> log** (events + objects + relationships) instead of a flat GL.

## Why four configs

OCEL 2.0 is a *multi-table* schema by design. Selecting just one
"default" split would lose 90 % of the signal. The four configs below
are sized so each can be loaded independently for the analyses they
support.

| config         | rows    | what it is |
|----------------|--------:|---|
| `events`       | 320,459 | the actual OCEL event log (timestamps + activity + linked objects) |
| `objects`      |   7,439 | each entity / order / shipment / invoice with its lifecycle |
| `anomaly_labels` | 9,804 | per-event ground-truth labels (fraud / error / process anomalies) |
| `document_events` |  358 | document-flow milestones (PO opened, GR booked, IR matched, payment cleared) |

Load with `load_dataset(repo, name=<config_name>)`. The `events` config
is the canonical OCEL log most process-mining tools (pm4py, ProM,
Celonis) consume.

## Structural quality (vs reference)

Same v5.29 SOTA lever stack as the JE datasets; see
[`vynfi-journal-entries-1m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m)
for the 13-row structural-metric table. The lever effects propagate
through the OCPM event-log construction (P2P/O2C process chains
inherit the same SOTA-N behavioral patterns).

## Dataset scope

- **5 companies × 12 months × `Custom(300_000)`** (heap cap raised to
  64 GB for VM-class regen)
- ~320 K OCEL events, ~7.4 K objects across P2P/O2C/manufacturing
  process types
- Method-A flat edge list (`je_network.parquet`) joinable to events via
  shared `document_id` + entry date
- Multi-currency (USD/EUR/SGD)

## Quick start

```python
from datasets import load_dataset

events = load_dataset("VynFi/vynfi-supply-chain-ocel", name="events")
objs   = load_dataset("VynFi/vynfi-supply-chain-ocel", name="objects")
print(events["train"].num_rows, "events")   # ~320,459
print(objs["train"].num_rows, "objects")    # ~7,439
```

For an end-to-end OCEL 2.0 file, the events + objects + relationships
roll up into the `ocel_json` output the engine writes during
generation; the parquet split here is for tabular consumers.

## Generation config

`configs/examples/hf/supply_chain_ocel_sota.yaml` in
[`mivertowski/SyntheticData @ v5.29.0`](https://github.com/mivertowski/SyntheticData/releases/tag/v5.29.0).

```bash
datasynth-data validate --config supply_chain_ocel_sota.yaml
datasynth-data generate --config supply_chain_ocel_sota.yaml
```

## Reproducibility

| artefact | path |
|---|---|
| Generator | `datasynth-data 5.29.0` (release tag `v5.29.0`) |
| Config | `configs/examples/hf/supply_chain_ocel_sota.yaml` |
| Run seed | `20260509` (preserved from v5.10 lineage) |

## Citation

```bibtex
@dataset{vynfi_sc_ocel_2026,
  author    = {Ivertowski, Michael and DataSynth contributors},
  title     = {VynFi Supply Chain OCEL — v5.29 SOTA mode},
  year      = {2026},
  publisher = {VynFi / Hugging Face},
  url       = {https://huggingface.co/datasets/VynFi/vynfi-supply-chain-ocel},
  version   = {v5.29.0},
}
```

---
title: VynFi Fraud-GNN Demo
emoji: 🛡️
colorFrom: red
colorTo: indigo
sdk: gradio
sdk_version: 5.5.0
python_version: '3.11'
app_file: app.py
pinned: true
license: apache-2.0
short_description: GraphSAGE fraud + GAE anomaly on synthetic JE network
tags:
  - vynfi
  - graph-neural-network
  - fraud-detection
  - anomaly-detection
  - synthetic-data
---

# 🛡️ VynFi Fraud-GNN Demo

Interactive inference Space for the
[`VynFi/je-fraud-gnn`](https://huggingface.co/VynFi/je-fraud-gnn)
model bundle.

## Three tabs

* **Edge fraud predictor** — pick a curated sample (clear fraud / clear
  normal / borderline) or build your own edge from any of the 499 GL
  accounts in the published COA.  Returns fraud probability + anomaly MSE.
* **Node anomaly explorer** — top-K accounts ranked by GAE
  reconstruction error on a 5,000-edge sample; surfaces accounts whose
  attribute patterns don't fit the structural prior.
* **Live evaluation** — sample N edges from
  [`VynFi/vynfi-journal-entries-1m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m),
  run the classifier, render confusion matrix + ROC against ground truth.

## Tech

* Gradio + torch-geometric + pandas + matplotlib
* Loads model bundle from `VynFi/je-fraud-gnn` at cold-start (cached after).
* Loads dataset slices from `VynFi/vynfi-journal-entries-1m` on demand.

## Source

* [Engine repo (`spaces/fraud-gnn-demo/`)](https://github.com/mivertowski/SyntheticData/tree/main/spaces/fraud-gnn-demo)
* [Model card](https://huggingface.co/VynFi/je-fraud-gnn) — full training details, metrics, and honest discussion of where GNN helps vs LR baseline.
* [Companion paper (SSRN)](https://ssrn.com/abstract=6538639)

## License

Apache-2.0.

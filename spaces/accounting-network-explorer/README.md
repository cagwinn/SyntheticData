---
title: VynFi Accounting Network Explorer
emoji: 🔗
colorFrom: blue
colorTo: indigo
sdk: streamlit
sdk_version: 1.39.0
python_version: '3.11'
app_file: app.py
pinned: true
license: apache-2.0
short_description: Interactive ISO 21378 account-class flow graph (v5.9.0)
tags:
  - vynfi
  - accounting
  - graph
  - iso-21378
  - synthetic-data
  - financial-network
---

# 🔗 VynFi Accounting Network Explorer

Interactive view of the v5.9.0 Method-A accounting network published in
[`VynFi/vynfi-journal-entries-1m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m),
aggregated to **ISO 21378 Level-2** account classes (~30 nodes).

## What you can do

* **Filter** the underlying 61 656 line-level edges by business process
  (P2P / O2C / R2R / H2R / A2R), `is_fraud`, `is_anomaly`,
  minimum edge amount, and top-N.
* **Inspect** any class node to see total flow, fraud %, and the top
  in/out class pairs.
* **Drill in** to the Level-3 sub-class breakdown
  (`A.A.A — Operating Cash`, `A.A.B — Petty Cash`, …).
* **Toggle** force-directed vs hierarchical layout.

## Method A vs Cartesian

In v5.9.0 the JE-network defaults to *Method A* from Ivertowski 2024:
exactly **one edge per 2-line journal entry**, confidence = 1.0.
This avoids the Cartesian explosion (225 M edges on 1 M JEs) that the
legacy `cartesian` method produces, and gives a clean topology for
graph-ML training.

## Tech

Streamlit + `streamlit-agraph` (vis-network) · pandas/pyarrow ·
loads parquet directly from the HF dataset on cold-start, then
caches in-memory.

## Source

* App code: [github.com/mivertowski/SyntheticData/tree/main/spaces/accounting-network-explorer](https://github.com/mivertowski/SyntheticData/tree/main/spaces/accounting-network-explorer)
* Generation engine: [github.com/mivertowski/SyntheticData](https://github.com/mivertowski/SyntheticData)
* Companion paper: [SSRN abstract 6538639](https://ssrn.com/abstract=6538639)

## License

Apache-2.0.

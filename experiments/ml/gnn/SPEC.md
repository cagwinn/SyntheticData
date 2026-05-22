# Track 1 — GNN relational sampler

## Objective

Learn the **interconnectivity structure** of the corpus's entity graphs
(trading-partner co-occurrence, vendor/counterparty network, IC bilateral
edges) and sample new graphs with the same motif statistics — closing the
**P3 ClusteringGap** and **TriangleLogRatio** gaps the hand-tuned
`CrossEntityMotifSampler` / TP motif sampler only partially close.

The symbolic generator keeps ownership of *what posts on each edge* (JEs,
amounts, balance). This model only decides *which entities connect and how
densely* — the relational scaffold.

## Why a GNN (vs the current motif sampler)

The current samplers bias draws toward recent cluster-mates — a local
heuristic. They can't represent global structure (community sizes, degree
distribution tails, triangle density) jointly. A graph autoencoder learns a
latent node embedding whose inner-product geometry reproduces the corpus's
joint edge structure, so sampled graphs match clustering + triangle counts by
construction rather than by tuning.

## Data (`common.data_export --track gnn`)

Per client (namespaces kept disjoint — see SP3.11):

* **Nodes** = entities `(client, trading_partner)`; anonymized integer ids.
* **Edges** = co-occurrence: two TPs sharing a JE or a (source, period) bucket;
  weight = count.
* **Node features** `x`: `[log-degree, source-mix histogram (k sources),
  active-window length (days), mean lines-per-JE]`. All aggregated — no names,
  no row-level text.

Artifacts (gitignored): `edge_index.pt`, `edge_weight.pt`, `node_feat.pt`,
`node_ids.parquet` (anonymized id ↔ opaque hash, stays private).

## Architecture

GAE-style:

```
x, edge_index ─▶ GraphSAGE(2 layers, hidden=128) ─▶ z  (node embeddings, d=64)
sample edges:   p(i~j) = σ(zᵢ · zⱼ)             (inner-product decoder)
```

Loss: negative-sampling reconstruction (BCE on observed vs sampled non-edges)
+ a **degree-distribution KL** regularizer and a **triangle-count** penalty so
the embedding geometry matches the corpus's P3 statistics, not just edges.

Sampling: draw a degree sequence from the fitted tail, then realize edges by
thresholding `σ(zᵢ·zⱼ)` with calibrated sparsity → new anonymized graph.

## Success criteria

* P3 ClusteringGap DR and TriangleLogRatio DR (Source + TP) **down ≥ 40%** vs
  the v5.26 baseline, measured by `common.bf_bridge.score_canonical` on a full
  generate run that consumes the sampled graph.
* No regression > 10% on P1/P2/P4 (relational change shouldn't perturb timing).
* Coherence unaffected — IC matching coverage + balance checks still pass
  (they run downstream of edge selection).

## Handoff to the Rust generator

The model emits a **graph artifact** (anonymized edge list + per-node
source-mix), not weights the generator must run. The Rust generator gains a
"load relational scaffold" path that consumes this artifact at build time and
routes entity selection through it. → keep the NN Python-side; ship the
sampled scaffold. (Re-evaluate if we want online sampling later.)

## Privacy

Highest memorization risk of the four tracks — the embedding can encode genuine
counterparty adjacency. Before sharing any weights or artifact off the private
box:
* node ids are opaque hashes (done at export);
* apply **k-anonymity** on the degree/feature join (drop nodes with degree <
  k) and/or **DP-SGD** (ε budget recorded in the run config);
* memorization probe: nearest-neighbour attack on embeddings must not recover
  held-out edges above chance. Gate in `train.py --privacy-check`.

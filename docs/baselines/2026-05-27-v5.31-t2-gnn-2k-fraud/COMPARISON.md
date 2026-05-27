# v5.31 T2 — GNN edge-fraud retrain on 2k group-audit je_network

Training a GraphSAGE edge-fraud classifier on the v5.31 C1 Phase 2
2k group-audit consolidated je_network, with the fraud-on-shard
fix (commit `e7428d8b`) propagating v5.30 SOTA fraud rates through
the per-entity orchestrator config.

**Test AUC: 0.6353** on the 2M-edge subsample. Per-business-process
AUC spread is 0.62-0.66 — modest, well below the 1M HF dataset's
0.919 baseline from the v5.27 retrain.

## Setup

| | value |
|---|---|
| Dataset | `consolidated/je_network.csv` from
2k group-audit aggregate (Phase 2 streaming JE-network writer) |
| Total edges in CSV | 42 272 976 |
| Subsample size | 2 000 000 (stratified by `is_fraud`) |
| Positive rate | 5.65 % (113K positives in subsample) |
| Unique GL-account nodes | 592 |
| Edge features | log-amount, is_anomaly, 8-dim business-process one-hot |
| Node features | log in-degree, log out-degree, log total degree |
| Model | 2-layer SAGEConv + edge-head MLP (64 hidden) |
| Training | 50 epochs, Adam lr=1e-3, BCE + pos-weight 16.6 |
| Hardware | A10 GPU (CUDA 13.0, torch 2.7, PyG 2.7) |
| Wall time | 96 s |

## Test results

| metric | value |
|---|--:|
| Test AUC-ROC | **0.6353** |
| Test AP (AUC-PR) | 0.0846 |
| Test F1 @ best threshold | 0.1518 |
| Best val AUC (during training) | 0.6355 |

### Per-business-process AUC

| process | AUC | n (test) | positives |
|---|--:|--:|--:|
| R2R | **0.6555** | 71 817 | 3 573 |
| A2R | 0.6527 | 13 941 | 820 |
| O2C | 0.6329 | 99 050 | 5 721 |
| H2R | 0.6197 | 27 951 | 1 652 |
| P2P | 0.6185 | 87 233 | 5 179 |

**Spread: 0.037 (R2R 0.6555 vs P2P 0.6185).**

## Why AUC is 0.635 vs v5.27's 0.919

The 1M HF dataset's GNN showcase (v5.27 retrain, AUC 0.919) trained on
a fundamentally different graph:

| | 1M HF (v5.27 GNN showcase) | 2k group (this) |
|---|---|---|
| Graph type | flat single-company JE network | consolidated multi-entity je_network |
| Nodes | ~ thousands (full chart-of-accounts join) | **592** (GL accounts shared across 2000 entities) |
| Node features | account_class, account_sub_class, type embedding | log-degree only (no CoA join) |
| Edge density | ~moderate | ~67K edges/node avg (very dense) |
| Fraud signal | per-line `is_fraud` within ~3M lines | aggregated through consolidation, diluted |

The 2k consolidated graph collapses every entity's GL postings onto
shared account nodes (Cash, AP, Sales, etc.) — so the graph encodes
**which account-pairs see fraud** rather than **which JEs are
fraud**. The 1M flat case has richer per-line context.

Concretely: with only 592 nodes, GraphSAGE's neighborhood
aggregation can't distinguish fraud patterns at the granularity the
1M HF case allowed.

## What this means

This isn't a fair "did the 2k regen match the 1M GNN performance"
comparison — it's two different graph-construction approaches:

- **1M HF case**: line-level graph with rich node features → ML-ready
  for fraud detection at the line level. AUC 0.919 is achievable.
- **2k group case**: consolidated graph compressing all entities
  onto shared accounts → measures *which account-pairs see fraud
  aggregated across entities*. AUC 0.635 reflects the harder task.

Both are useful artefacts. For the GNN fraud-detection roadmap, the
1M HF case is the right benchmark. For graph-structure research on
multi-entity consolidation patterns, the 2k case opens new questions.

## B3 per-process variance retrospective

The B3 task (#153) targeted "GNN AUC 0.919 → 0.93-0.94, per-process
variance widening to a more realistic 0.88-0.95 spread". On the 2k
group graph the spread is 0.037 (P2P 0.6185 → R2R 0.6555). On the
1M HF graph the B3 effect (whether the per-process fraud rate
config actually widens spread) needs a fresh 1M training with
`per_process_rates` activated — that's queued for a future session.

## Stratification-bug note

The first training pass had a subsample-stratification bug producing
0.64 % positive rate (instead of 5.65 %). After the fix the test AUC
moved from 0.6469 → 0.6353 — small absolute change, but the proper
stratification is now in the script.

## Artefacts

```
docs/baselines/2026-05-27-v5.31-t2-gnn-2k-fraud/
├── COMPARISON.md   (this file)
├── results.json    (full per-epoch + per-process metrics)
└── train.log       (training log: 50 epochs, ~100s wallclock)
```

Trained model artefact (`model.pt`, 109 KB) stays VM-local at
`/home/ubuntu/regen/gnn_fraud_2k/model.pt` — user can push to HF
when convenient.

Closes T2 from the overnight runbook.

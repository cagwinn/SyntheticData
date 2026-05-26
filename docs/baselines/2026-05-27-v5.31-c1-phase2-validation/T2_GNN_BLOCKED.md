# T2 GNN retrain — blocked by 2k regen config gap

The C1 Phase 2 streaming aggregate produced
`consolidated/je_network.csv` at 42 272 976 edges (8.5 GB), which would
have been the input for T2 in the overnight runbook. **Training failed
because all 42 M edges have `is_fraud=false`** — the 2k regen had no
fraud injection.

## Root cause

The 2k group config (`enterprise_2000_sota.yaml`) is a `GroupConfig`
schema, **not** the generator config that `journal_entries_1m_sota.yaml`
uses. The per-entity orchestrator config that drives each entity's
journal-entry generation is built by
`crates/datasynth-group/src/shard/per_entity_config.rs::build_entity_generator_config()`
which calls `create_preset(industry, 1, period_months, complexity,
volume)` — and the preset's `FraudConfig::default()` has
`enabled: false`.

The `defaults:` block in the 2k group config has no `fraud:` key, so
fraud stays off for every entity. The standalone 1M dataset's HF
config (`journal_entries_1m_sota.yaml`) sets fraud explicitly:
```yaml
fraud:
  enabled: true
  fraud_rate: 0.05
  document_fraud_rate: 0.07
```

The group-audit pipeline never threaded this through.

## Confirmed by inspection

```
$ awk -F, 'NR>1 {c[$13]++} END {for (k in c) print k, c[k]}' \
    consolidated/je_network.csv
false 42272975
(blank) 1
```

42 M edges all `is_fraud=false`. No positive labels → ROC AUC undefined →
training script crashes at the first val epoch.

## What's needed to unblock T2

Either:
1. **Fix the config gap.** Wire `fraud:` through from `GroupConfig`
   defaults to `per_entity_config.rs::build_entity_generator_config()`.
   The fraud SOTA-mode block from `journal_entries_1m_sota.yaml` would
   produce ~5 % line-level + ~7 % document-fraud-propagated edges in
   the je_network. ~50 LOC change + lib test + 2k regen + ~30 min for
   T2 training to actually run.
2. **Re-run with explicit per-entity fraud config.** Inject a
   `fraud:` block into the group config's `defaults:` after
   confirming the propagation path is wired.

Both options require another 2k regen (~1h entity phase) + the C1
Phase 2 aggregate-only re-run (~40 min through OOM point) — net 1.5-2h
to unblock T2 alone. Not viable in tonight's queue.

## Alternative ML tasks on the existing 2k network

The 42M-edge consolidated graph IS rich, just without fraud labels.
Three meaningful tasks that work:

- **`business_process` multi-class edge classification** — 5 process
  families (P2P, O2C, R2R, H2R, A2R), well-balanced in the data.
  Could train a 2-layer GraphSAGE + softmax head, ~30 min on A10.
- **Node embedding + downstream entity-similarity clustering** —
  unsupervised; produces useful structural embeddings for the 585
  unique GL accounts in the 2k consolidated graph.
- **`is_anomaly` binary classification** — only useful if anomaly
  injection ran. Need to confirm `is_anomaly` rate before committing.

All deferred for now; T3 takes priority because it doesn't depend on
the 2k regen at all.

## Decision

Skip T2 in tonight's queue. Advance to **T3 (multi-shard BF eval
methodology)**. Capture this finding here so morning resume has full
context. When the user prioritises the 2k fraud-config fix, T2 can
re-launch in a single hour.

## Dataset is still useful as-is

`consolidated/je_network.csv` (8.5 GB / 42M edges) is a **structural
artefact** even without fraud labels. It's the largest synthetic
group-audit edge graph the engine has ever produced. The
per-entity files (1 221 of 2 000) provide entity-grouped subgraphs.
Both are usable for graph-structure analysis, embedding learning,
or as a benchmark substrate once labels exist.

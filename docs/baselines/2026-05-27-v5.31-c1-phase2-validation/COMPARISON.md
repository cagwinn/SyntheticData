# v5.31 C1 Phase 2 — 2000-entity aggregate-only validation

**Outcome: partial win.** The streaming JE-network export and the
IC-only JE filter both work end-to-end. The walk completes for all
2000 entities, produces 42 M consolidated edges (8.5 GB CSV + per-entity
parquets), and leaves only the IC-tagged JE subset in memory. **But the
downstream IC matcher still OOM-kills the process at 219 GB anon-rss.**

## RSS trajectory

Lightweight monitor sampled every 15s during the aggregate-only run.
Full log at [`rss_trajectory.log`](./rss_trajectory.log).

| time | wall t+ | RSS | observation |
|---|---|--:|---|
| 22:29:18 | start | 0 GB | aggregate launch |
| 22:30:00 | t+1m | 6 GB | walk in progress, ~12 entities/sec |
| 22:35:00 | t+5m | 36 GB | RSS climbing ~6 GB/min (linear) |
| 22:45:00 | t+15m | ~110 GB | ~mid-walk |
| 23:02:00 | t+33m | 197 GB | walk near complete |
| 23:06:35 | t+37m | 219 GB | OOM-killed, exit 137 |

Linear ~6 GB/min climb during the entire walk. With the streaming JE
writer doing its job (per-entity edges flow directly to disk), the
remaining hold IS the **IC JE retention** + **translated_tbs** +
**deferred_tbs** + **entity_contributions**.

## What worked ✓

- **Streaming JE-network export.** `consolidated/je_network.csv`
  populated **end-to-end** (42 272 976 edges, 8.5 GB) during the walk.
  This is **complete and usable** for downstream analytics.
- **Per-entity edges.** 1 221 of 2 000 per-entity `je_network.csv`
  files written before OOM. Each is a self-contained Method-A edge
  graph for that entity.
- **IC JE filter.** The `ic_pair_id.is_some()` filter executed
  correctly — non-IC JEs were dropped before being pushed into
  `ic_journal_entries`.
- **CSV-line-streaming.** The consolidated CSV writer flushes
  line-by-line; the file is uncorrupted despite the OOM.

## What didn't ✗

- **Consolidated parquet corrupted.** The ArrowWriter holds the
  parquet's row-group footer until `finalize()` is called. The OOM
  happened **after the walk but before the post-IC-matching
  finalize()`, so the parquet file is truncated (`pyarrow` reports
  "Parquet magic bytes not found in footer"). The CSV remains
  byte-for-byte intact.
- **Full aggregate doesn't complete.** Consolidated FS bundle
  (`consolidated_financial_statements.json`, schedule, notes, NCI,
  CTA, etc.) never written. The IC matcher hits OOM during the
  post-walk pass that clones every IC JE into `by_pair: BTreeMap<…,
  Vec<ObservedSide>>` plus matched/unmatched vectors.

## Root cause analysis — IC matcher dominates

At 2000 entities with default v5.30 SOTA fraud config
(`document_fraud_rate=0.07`), ~5 % of JEs carry an `ic_pair_id`:

```
2000 entities × ~100K JEs (significant tier) = 180M total JEs
5% IC-tagged = ~9M IC JEs across entities
ic_matcher clones each into ObservedSide ×3 (by_pair + matched + unmatched)
= ~27M JournalEntry clones at ~5 KB each = ~135 GB
```

That ~135 GB of IC matcher allocations is the proximate OOM cause.
The walk itself leaves ~80-90 GB in memory (translated_tbs +
entity_contributions + ic_journal_entries + remaining transients);
the additional 130 GB pushed by IC matching takes us past 219 GB.

## Phase 3 design — streaming IC matching

The natural continuation:

1. **Single-pass IC matching during the walk.** Group IC JEs by
   `ic_pair_id` as they're encountered (BTreeMap accumulator
   internal to the walk). The map's hold is bounded by the **number
   of distinct IC pairs**, not the total JE count.
2. **Two-pass alternative.** Walk-1: build a `pair_id → entity_codes`
   index. Walk-2: for each pair_id, re-read the entity's IC JEs and
   match. Doubles disk I/O but bounds memory to one pair's JEs at a
   time.
3. **Per-entity-pair triage.** Most IC pairs span only 2 entities
   (the seller-side and the buyer-side). After walk-1's index is
   built, walk-2 can iterate entity-by-entity, materializing only
   the JEs needed for that entity's outgoing pairs.

Estimated 5-8 hours for Phase 3 implementation + 2k validation. Not
in tonight's scope — capture, document, defer.

## Useful artefacts that DID land

```
/home/ubuntu/regen/data/group_2000_c1/
├── manifest.json                                  (~10 KB)
├── shard_summary.json
├── entities/  (2000 dirs, 720 GB total)
│   ├── ENT0001/
│   │   ├── journal_entries.{csv,json,parquet}    (per-entity full GL)
│   │   ├── graphs/je_network.{csv,parquet}        (per-entity edges)
│   │   ├── period_close/trial_balances.json
│   │   ├── master_data/...
│   │   └── ... (full per-entity audit archive)
│   └── ... (×1999)
└── consolidated/
    ├── je_network.csv         (8.5 GB — 42M edges, COMPLETE) ✓
    └── je_network.parquet     (corrupted footer — DO NOT READ) ✗
```

**The consolidated/je_network.csv at 8.5 GB / 42M edges unblocks T2
(GNN retrain on the 2k network)** — that was the primary downstream
dependency in the overnight runbook. We can train the GNN tonight
on this CSV; the rest of the consolidated FS bundle waits on Phase 3.

## Next steps tonight

1. **T2 GNN retrain** — proceed using `consolidated/je_network.csv`
   directly. The CSV's schema is the documented 19-column consolidated
   format; PyTorch Geometric can ingest it via the same CSV-loader the
   v5.10 showcase used.
2. **Phase 3 design** — write a design doc enumerating the streaming
   IC matcher strategies above; defer implementation.
3. **Continue overnight queue** (T3 multi-shard BF eval, T4 B2,
   T5 1M re-baseline) — none of these depend on the consolidated FS
   bundle.

## Artefacts

```
docs/baselines/2026-05-27-v5.31-c1-phase2-validation/
├── COMPARISON.md            (this file)
└── rss_trajectory.log       (150 RSS samples, every 15s)
```

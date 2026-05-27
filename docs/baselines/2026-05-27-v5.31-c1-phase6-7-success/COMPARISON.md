# v5.31 C1 — closed. Full 2k consolidated FS bundle ships at peak < 1 GB RSS.

**Phase 6 (streaming JSON parse) + Phase 7 (defensive IC elimination)
together turn the 2k aggregate from a 218 GB OOM-kill into a 0.38 GB
walk that ships every consolidated artefact in 65 minutes wallclock.**

## Headline

| metric | Phase 5 (pre-C1) | Phase 7 (this run) | Δ |
|---|--:|--:|--:|
| Peak RSS | **218 GB** (OOM-killed) | **0.382 GB** | **−570×** |
| Walk completion | ✗ (idx ~1230 / 2000) | ✓ (idx 2000) | full |
| IC eliminations | ✗ (never reached) | ✓ (6 814 pairs matched) | full |
| Consolidated FS bundle | ✗ (never written) | ✓ (8 artefacts, 38 GB) | full |
| Walltime | 38 min before OOM | 65 min end-to-end | +71 % |
| Outcome | aggregate error | `coverage 1.0000` | ✓ |

## Three-phase RSS trajectory

Tracked accumulators (`translated_tbs` lines, `entity_contributions`
pairs) are byte-identical across all three phases — the streaming
refactor changes only the per-iteration allocation pattern, not the
output.

| entity_idx | Phase 5 RSS | Phase 6 RSS | Phase 7 RSS | translated_tbs lines |
|---:|--:|--:|--:|--:|
| 100 | (n/a) | 0.25 GB | 0.25 GB | 20 486 |
| 300 | **56 GB** | 0.25 GB | 0.25 GB | 60 657 |
| 600 | 107 GB | 0.26 GB | 0.27 GB | 117 596 |
| 900 | 167 GB | 0.27 GB | 0.28 GB | 182 097 |
| 1 200 | **216 GB** | 0.30 GB | 0.30 GB | 238 093 |
| 1 500 | (OOM) | 0.32 GB | 0.32 GB | 296 844 |
| 1 800 | (OOM) | 0.34 GB | 0.34 GB | 355 411 |
| 2 000 | (OOM) | 0.37 GB | 0.38 GB | 394 508 |

Phase 5 climbed linearly at **178 MB per entity** even though the
tracked accumulators grew at only **75 KB per entity** (a 2 400× gap
— see the [Phase 5 diagnostic](../2026-05-27-v5.31-c1-phase5-diagnostic/COMPARISON.md)).

Phase 6 + 7 grow at exactly **75 KB per entity** — the gap is gone.

## Root causes & fixes

### Phase 6 — JSON-parse fragmentation (`a1596d7e`)

The walk loaded each entity's `journal_entries.json` (3-30 MB on disk)
via `serde_json::from_slice::<Vec<JournalEntry>>`. The in-memory
deserialisation expanded that to a 25-300 MB tree of `String`s +
nested `Vec`s + nested struct fields, all dropped at end of iteration.

After 1 200 such allocate-and-drop cycles, mimalloc had returned the
memory to the heap but could not reclaim pages to the OS because the
heap was fragmented by the small-allocation pattern (each JE's lines
contain ~10-20 separately-allocated `String` fields). RSS climbed to
218 GB.

**Fix:** new `JeNetworkEdgeBuilder` in `datasynth-runtime` accepts JEs
one at a time while preserving the `line_id → edge_id` HashMap that
cross-JE predecessor chains depend on. `walk_aggregate_streaming`
now uses `serde_json::Deserializer::from_reader` + a custom `Visitor`
that yields `JournalEntry` values one at a time through a `FnMut`
callback. Peak per-iteration allocation drops from ~300 MB to ~25 KB.
Allocator pattern becomes uniform → no fragmentation → no RSS climb.

`build_je_network_edges(&[JE])` (the legacy slice variant) is refactored
to delegate to the same per-JE emit helper — byte-identical output is
preserved across all eight production tests
(`golden_archive`, `ic_je_injection`, `elimination_to_je`,
`consolidated_bs/is/cf`, `consolidation_schedule`, `cta`,
`equity_method`, `equity_changes` — 109 group lib tests + 45 group
integration tests, all green).

### Phase 7 — `ReversedAmount` corrupting IC JEs (`4ebb2206`)

The walk completed in Phase 6 but the elimination factory then aborted:

> `generate_eliminations: seller-side JE for pair 742ef1a7... has no
> debit line — expected exactly one debit per IC injector contract`

The IC injector emits each IC JE as exactly one debit + one credit
line. But `anomaly_injection.strategies::ReversedAmountStrategy::apply`
picks a random line and swaps its `debit_amount` ↔ `credit_amount` to
simulate a clerical error. On a 2-line IC JE, that produces:

  - Original debit line → (DR=0, CR=X)
  - Original credit line (untouched) → (DR=0, CR=X)
  - JE now has TWO credit lines and ZERO debit lines

Phase 5 never reached this code path (OOM'd in the walk). Phase 6
exposed it the moment the walk completed and elimination tried to
read each IC pair's notional from `seller_je`.

**Fix (defense-in-depth):**

1. **Anomaly gate** (future shards):
   `ReversedAmountStrategy::can_apply` now requires
   `entry.header.ic_pair_id.is_none()`. IC postings are deterministic
   by manifest contract — picking them as anomaly targets adds no
   realism but breaks downstream consolidation.

2. **Defensive elimination** (existing shards):
   `elimination_amount` now uses a three-tier resolution:
     - **(a)** seller JE's debit line amount (legacy behaviour, fast path)
     - **(b)** total credit (covers `ReversedAmount` swap)
     - **(c)** `plan.amount` from the manifest (authoritative notional)
   Each fallback emits a `tracing::warn` so coverage diagnostics can
   flag the divergence.

On this 2k run the fallback fired **4 times** (out of 6 814 matched
pairs — 0.06 %) and the elimination still netted correctly using the
total-credit path. The consolidated FS bundle is structurally correct
even though those four IC JEs remain visibly corrupted in
`je_network.csv`.

## Run details

```
Started:      2026-05-27T12:42:23Z
Walk done:    2026-05-27T13:47:15Z   (entity_idx 2000 / 2000)
Aggregate:    2026-05-27T13:47:15Z   (same second — IC matcher + elim
                                      finish in <1 s once IC subset
                                      is in memory)
Final stats:  6 814 IC pairs matched, coverage 1.0000
              2 000 entities aggregated
              68 463 796 consolidated je_network edges
              13 628 elim edges
              4 IC pair fallback warns (ReversedAmount-corrupted)
```

### Consolidated bundle on disk

```
consolidated/
├── je_network.csv                              14.8 GB   (68.5M edges)
├── je_network.parquet                           5.8 GB
├── translation_worksheet.json                  91.6 MB
├── consolidation_schedule.json                 15.1 MB
├── cta_rollforward.json                         303 KB
├── notes_to_consolidated_fs.json                155 KB
├── consolidated_financial_statements.json        33 KB
├── equity_method_investments.json                 3 B   (manifest has none)
├── equity_method_suppressed_losses.json           3 B
└── nci_rollforward.json                           3 B   (manifest has none)

Total bundle: 38 GB (746 GB of input shards distilled to a 38 GB
consolidation surface, 51× compression).
```

### Per-entity output

2 000 entity directories under `entities/{ENT_NNNN}/graphs/` each
with their own `je_network.csv` + `je_network.parquet`. Used by the
T2 GNN training surface.

## What this means for the engine

- **C1 #156 closes.** 2k group regen → full consolidated FS bundle is
  now a production workflow at peak < 1 GB RSS. Fits in a developer
  laptop, not just the 222 GB VM.
- **Streaming JSON parse is the load-bearing change.** The
  `JeNetworkEdgeBuilder` is a self-contained primitive other crates
  can adopt (e.g. for streaming je_network export from the
  orchestrator's runtime-side output_writer).
- **Anomaly post-process pass needs a contract audit.** Phase 7 only
  fixed `ReversedAmountStrategy`; the same JE-shape-violating issue
  may exist in `SplitTransaction`, `WrongAccount`, etc. for IC JEs.
  Tracked as follow-up — current shards survive via the defensive
  elimination fallback.
- **C1 unlocks C2 (multi-period carry-forward) + C3 (adversarial
  calibration).** Both need a working aggregate at scale.

## Phases recap

| phase | commit | role |
|---|---|---|
| Phase 1 | `b5dc9408` | TB streaming walk |
| Phase 2 | `50b64654` | JE-network streaming writer + IC-only retention |
| Phase 3 | `fc7e298f` | consume-by-value IC matcher |
| Phase 4 | `28385e61` | mimalloc global allocator |
| Phase 5 | `06f8f9ca` | RSS diagnostic instrumentation (no fix) |
| **Phase 6** | **`a1596d7e`** | **streaming JSON parse + JeNetworkEdgeBuilder** |
| **Phase 7** | **`4ebb2206`** | **defensive IC elimination + anomaly gate** |

Phases 1-4 are *necessary* (each removed a different hold) and
collectively get to 2k je_network export. Phase 6 is *sufficient*
for the walk; Phase 7 is *sufficient* for the elimination step.
Together they ship the full bundle.

## RSS trajectory log

See [`rss_trajectory.csv`](./rss_trajectory.csv) — 240+ samples at
15-second intervals across the 65-minute run. Peak: 0.382 GB at
13:44:50 UTC (entity_idx ~1900, near end of walk where parquet row
groups are being flushed).

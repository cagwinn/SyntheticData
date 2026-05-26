# v5.31 C1 Phase 1 — 2000-entity validation result

**Outcome: OOM at 228 GB anon-rss. C1 Phase 1 is insufficient at 2k
scale.** The streaming-walk refactor successfully eliminated the
`contributing_tbs` memory hold (entity phase peaks at 4 GB instead
of the prior 100-200 GB), but a second, larger memory hotspot was
exposed: `contributing_jes: Vec<(String, Vec<JournalEntry>)>` at
~200-400 GB in-memory for the 2k case.

## RSS trajectory

Lightweight monitor sampled every 30s during the C1 2k regen
(commit `b5dc9408`, generate PID 82422). Full log at
[`rss_trajectory.log`](./rss_trajectory.log).

| phase | wall time | RSS | observation |
|---|---|--:|---|
| entity emission | 20:32 → 21:34 | 3-4 GB stable | streaming walk works at TB scale |
| entity emission done | 21:34 | 4 GB | all 2000 dirs written |
| aggregate phase begins | 21:35 | climbing | walk_aggregate_streaming runs |
| aggregate t+1m | 21:36 | 44 GB | JE data loading into memory |
| aggregate t+2m | 21:37 | 53 GB | continued JE accumulation |
| aggregate t+5m | 21:40 | 99 GB | half the way to OOM ceiling |
| aggregate t+10m | 21:45 | 182 GB | linear climb, ~10 GB/min |
| aggregate t+12m | 21:47 | 217 GB | last sample before kill |
| **OOM kill** | **21:48** | **228 GB anon-rss** | kernel OOM-killer |

Linear climb of ~10-15 GB/min during the aggregate phase — entirely
during `walk_aggregate_streaming`'s loop over the 2000 entity
directories. The streaming walk drops each source TB after
accumulation, but `load_entity_journal_entries(&entity_dir, ...)`
loads each entity's JEs into a `Vec<JournalEntry>` that gets pushed
into `contributing_jes` and **persists** for IC matching + JE network
export downstream.

## Root cause — JE data dominates

Per-entity journal_entries.json file sizes (significant tier):

| entity | JE count | JSON size |
|---|--:|--:|
| ENT0001 | 103,847 | 710 MB |
| ENT0500 | 103,849 | 726 MB |
| ENT_PARENT | ~100K | 732 MB |
| ENT1500 (limited) | 804 | 2 MB |

**Total JE data across all 2000 entities: 700 GB on disk.** Loading
into `Vec<JournalEntry>` typically inflates ~1.5-3× due to String
allocations + Vec overhead → 1-2 TB peak in memory if held all at
once. The OOM at 228 GB is reached only because the JEs are pushed
into `contributing_jes` *as they're read* (no eager-load of all
2000 at once), so the hold grows linearly during the walk.

## Why C1 Phase 1 missed this

The Phase 1 design analysed `contributing_tbs` as the dominant
hotspot. That was correct for the TBs themselves (which would have
been ~200 GB), but the JEs sit on top:

```
Estimated peak hold during aggregate phase (Phase 1):
  contributing_tbs:        eliminated by streaming walk ✓
  pre_elim:                ~10 MB
  translated_tbs:          ~40 GB (2000 × ~20 MB each)
  entity_contributions:    ~200 MB
  deferred_tbs:            ~4 GB
  contributing_jes:        ~200-400 GB ← NEW DOMINANT HOTSPOT
  IC matcher (after walk): ~100-200 GB additional (JE clones)
```

The 228 GB kill point is right inside the `contributing_jes` growth
window, before IC matcher even begins.

## Phase 2 design — stream JEs through

Same architectural pattern as Phase 1, applied to JEs:

1. **During the walk**: read each entity's JEs, do all per-entity
   work that needs them inline:
   - **JE network export**: write per-entity je_network edges
     directly to disk (no consolidated edge vec in memory)
   - **Filter to IC JEs only**: keep only JEs with
     `header.ic_pair_id.is_some()` for downstream IC matching
   - Drop the rest immediately

2. **After the walk**: `contributing_jes` contains only IC JEs (a
   tiny subset — typically <5 % of total JEs). IC matcher operates
   on this reduced set.

3. **JE network consolidation**: stream-concatenate per-entity edge
   CSV files into the consolidated CSV (the existing per-entity +
   consolidated split already exists in `je_network.rs`).

Expected memory profile (Phase 1+2):

```
Phase 2 peak hold during aggregate:
  contributing_tbs:        eliminated ✓
  pre_elim:                ~10 MB
  translated_tbs:          ~40 GB
  entity_contributions:    ~200 MB
  deferred_tbs:            ~4 GB
  contributing_jes (IC):   ~5-10 GB (was ~200-400 GB)
  IC matcher clones:       ~5-10 GB (proportional reduction)
  JE network edges:        streamed to disk (was held in vec)
```

Total: ~55-65 GB peak. Fits in 222 GB box with substantial headroom.

## What Phase 1 IS good for

Phase 1 isn't wasted work:

- **It eliminates the contributing_tbs hold** at 2k+ scale. The TBs
  weren't the proximate OOM cause (because the JEs got loaded first
  and OOM-killed before TBs would have been instantiated), but at
  4k or 10k scale they'd add another 400-1000 GB on top.
- **The infrastructure** (accumulate_entity_into_aggregate,
  finalise_streaming_aggregate, empty_aggregate,
  build_consolidation_schedule_with_contributions, translate_one_entity)
  is correctness-preserving and reusable for Phase 2.
- **Phase 1 alone fits in box at ~500-entity scale.** Up to ~500
  entities, JE data is ~100-150 GB (~half of significant tier × 700
  MB), which is below the OOM threshold. Mini-Acme + small enterprise
  configurations are unaffected.

## Next step

Phase 2 implementation queued. Estimate ~2 hours including local
tests + VM rebuild + re-run 2k validation. Phase 2 makes the changes
that actually unblock 2k.

## Artefacts

```
docs/baselines/2026-05-26-v5.31-c1-2k-validation/
├── COMPARISON.md            (this file)
└── rss_trajectory.log       (50 RSS samples from the monitor)
```

The 720 GB of partial entity output on the VM is **kept** until
Phase 2 is validated — saves rerunning the 1-hour entity-emission
phase if Phase 2 holds and only the aggregate phase needs the
fresh code.

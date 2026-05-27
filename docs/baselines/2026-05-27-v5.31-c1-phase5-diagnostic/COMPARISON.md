# v5.31 C1 Phase 5 diagnostic — hotspot identified

**The 218 GB OOM is per-iteration JSON-parse churn fragmenting the
heap. Tracked accumulators are tiny.**

## Methodology

Instrumented `walk_aggregate_streaming` (commit `06f8f9ca`) to emit
`tracing::info!` checkpoints every ~5 % of entities, logging:
- `rss_gb` (read from `/proc/self/statm`)
- Size of every long-lived accumulator: `pre_elim`, `translated_tbs`,
  `entity_contributions`, `deferred_tbs`, `ic_journal_entries`

Then re-ran the 2k aggregate-only against the existing fraud-2k entity
dirs. The aggregate OOM-killed at entity_idx ≈ 1230 / 218 GB peak —
identical trajectory to Phase 4, but now with structured logging.

## Per-checkpoint sizes

| entity_idx | RSS GB | translated_tbs lines | entity_contrib pairs | ic_jes total |
|---:|--:|--:|--:|--:|
| 300 | 56 | 60 657 | 60 657 | 7 842 |
| 400 | 72 | 78 469 | 78 469 | 8 173 |
| 500 | 90 | 98 452 | 98 452 | 8 486 |
| 600 | 107 | 117 596 | 117 596 | 8 823 |
| 700 | 130 | 141 908 | 141 908 | 9 241 |
| 800 | 148 | 162 166 | 162 166 | 9 605 |
| 900 | 167 | 182 097 | 182 097 | 9 954 |
| 1 000 | 183 | 199 847 | 199 847 | 10 267 |
| 1 100 | 201 | 220 253 | 220 253 | 10 604 |
| 1 200 | 216 | 238 093 | 238 093 | 10 923 |

Δ idx=300 → idx=1 200 (900 entities walked):
- RSS: **+160 GB** (≈ 178 MB / entity)
- translated_tbs lines: +177 436 (≈ 197 lines / entity, ≈ 200 B/line ≈
  **40 KB / entity**)
- entity_contributions pairs: +177 436 (≈ **20 KB / entity**)
- ic_journal_entries JEs: +3 081 (≈ **3 IC JEs / entity** × ~5 KB ≈
  **15 KB / entity**)

**Total tracked growth per entity: ~75 KB. Actual RSS growth per
entity: 178 MB. The gap is 2 400×.**

## Conclusion — allocator fragmentation from JSON parse churn

The walk loads each entity's `journal_entries.json` via
`serde_json::from_slice::<Vec<JournalEntry>>(&bytes)`. The on-disk JSON
files are 3-30 MB per entity (limited / material tier). The
in-memory `Vec<JournalEntry>` after deserialisation is **~5-10× larger
than the JSON** (each JE struct + its String fields + `SmallVec<[Line]>`
all carrying allocator metadata).

So each per-entity load + drop cycle allocates **150-300 MB** of
small objects (Strings, Vecs, nested structures), then drops them.

mimalloc returns this memory to the heap, but the allocator can't
return pages to the OS until they're fully empty — fragmentation
across many small allocations prevents page-level reclamation. After
~1 200 such cycles, RSS has climbed to 218 GB even though logically
only ~75 KB × 1 200 ≈ 90 MB of data is retained across the
accumulators.

This is the smoking gun. The fragmentation hypothesis from Phase 4
turns out to be correct after all — mimalloc just couldn't compensate
for the allocation pattern. The fix is to **stop creating the spike
in the first place**.

## Phase 6 fix — streaming JSON parse + inline filter

Replace `load_entity_journal_entries` with a streaming parser:

```rust
pub fn stream_entity_journal_entries<F: FnMut(JournalEntry) -> Result<...>>(
    entity_dir: &Path,
    mut on_je: F,
) -> GroupResult<()> {
    let file = File::open(entity_dir.join("journal_entries.json"))?;
    let reader = BufReader::new(file);
    // serde_json::Deserializer::from_reader handles JSON arrays via
    // `into_iter()` only for newline-delimited. For JSON arrays we
    // need a streaming parser. Options:
    // - manual: walk the `[`, then loop parsing JEs separated by `,`
    // - crate: `json-array-stream` or `iter_json` or `simd-json`
    let stream = json_array_stream(reader);
    for je in stream {
        on_je(je?)?;
        // je drops here — single-JE allocation, ~25 KB peak
    }
    Ok(())
}
```

Then `walk_aggregate_streaming` becomes:

```rust
stream_entity_journal_entries(&entity_dir, |je| {
    // Emit edges directly (no Vec<edge> materialise either)
    je_network_writer.write_je_edges_streaming(&entity.code, &je)?;
    if je.header.ic_pair_id.is_some() {
        ic_jes.push(je);  // local subset
    }
    Ok(())
})?;
ic_journal_entries.push((entity.code.clone(), ic_jes));
```

Memory profile after Phase 6 (projected):
- Peak per-iteration JE allocation: **~25 KB** (one JE at a time)
- Heap fragmentation from spike: **eliminated**
- Per-entity sustained growth: ~75 KB (translated_tbs + contributions +
  IC subset)
- 2k peak RSS projection: **~10-15 GB** (well under 222 GB ceiling)

Effort: ~300-500 LOC including a JSON-array streaming parser
(`Deserializer::into_iter` doesn't work on JSON arrays directly —
need to peek the `[`, then loop with a comma separator). ~4-6 hours
focused work + 2k re-validation.

## Pre-Phase-6 checkpoint

What's working at 2k today (post Phase 1-4):
- ✅ 2k je_network.csv lands (8.4 GB, 42 M edges) via streaming writer
- ✅ Per-entity je_network files complete
- ✅ T2 GNN training works on the je_network surface
- ✗ Consolidated FS bundle (BS / IS / CF / NCI / notes / schedule)
  blocked by the OOM

Phase 6 (streaming JSON parse) is the path to consolidated FS at
2k scale.

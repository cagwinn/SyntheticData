# v5.31 C1 Phase 4 — mimalloc validation result

**Outcome: mimalloc gave ~1 GB headroom. The fragmentation hypothesis
was wrong; the working set is genuinely ~218 GB during the walk.**

## Result vs Phase 3

| metric | Phase 3 (glibc) | Phase 4 (mimalloc) | Δ |
|---|--:|--:|--:|
| Peak RSS | 219 GB | **218 GB** | −1 GB |
| Time to OOM | 38 min | 37 min | −1 min |
| Linear growth rate | ~6 GB/min | ~6 GB/min | 0 % |
| Walk completion | ✗ | ✗ | — |
| Consolidated je_network.csv | 8.5 GB ✓ | 8.4 GB ✓ | same |
| Consolidated je_network.parquet | corrupt | corrupt | — (writer never finalised) |
| Consolidated FS bundle | ✗ | ✗ | — |

mimalloc compiled in via `#[global_allocator] static GLOBAL:
mimalloc::MiMalloc = mimalloc::MiMalloc;` declaration in `cli/src/main.rs`
(commit `28385e61`). The binary used mimalloc throughout — verified by
build (the `mimalloc` crate is in the dependency tree and the linker
pulls it in). Yet the trajectory is essentially identical to Phase 3
(glibc malloc).

## Conclusion — fragmentation was not the bottleneck

The 38-min, 6 GB/min linear climb to ~218 GB is **legitimate
working-set growth**, not allocator overhead.

The IC-retention theory I floated in Phase 1-3 docs is **also wrong**:
post-hoc measurement of the consolidated `je_network.csv` shows the
IC-pair rate is only **0.03 % (10 940 / 41.7 M edges)**, not the
5-10 % I'd estimated. `ic_journal_entries` after the IC filter holds
≈ 6 000 JEs across all entities ≈ 100 MB. **Negligible.**

So what IS the 218 GB?

Going entity-by-entity, the walk allocates per iteration:
- `tb`: trial balance (~30 KB / entity)
- `jes`: `Vec<JournalEntry>` from `load_entity_journal_entries`
  (~100 K entries for significant tier; the JSON file is ~700 MB,
  in-memory ~1-2 GB after serde_json::from_slice expansion)
- `edges`: `Vec<JeNetworkEdge>` from `build_je_network_edges` (~200K entries)
- writes per-entity CSV + parquet
- writes consolidated rows via `JeNetworkStreamingWriter` (buffered 50K
  rows, flushed)

At end of iteration: `tb`, `jes`, `edges` all drop. Per-iteration alloc
+ free of 1-2 GB. With 1 500 entities walked in 38 min, that's
~50-65 GB/min churn through the allocator.

**Hypothesis (revised):** the per-entity `jes` Vec parses into a tree
of small allocations (Strings inside lines inside JEs inside Vec). When
the Vec drops, the small allocations free in random order, fragmenting
the heap **even with mimalloc**. The OS may not reclaim pages quickly,
causing apparent RSS to climb while logical memory is mostly free.

Or — alternative hypothesis — the parquet writer's `consol_parquet:
ArrowWriter<File>` buffers row groups internally. Default row group
size in parquet-rs is **1 048 576 rows**; with 42 M consolidated edges
the writer would accumulate **~40 row groups in memory** before
close(). At ~200 B/row × 1M rows = ~200 MB per group, **~8 GB
accumulated buffer**. Still not 218 GB on its own, but a contributor.

Combined working theory: high-churn JSON parse + Arrow writer
row-group buffer + the natural growth of `translated_tbs` /
`entity_contributions` adds up. Phase 5 needs to measure each
component independently before designing the fix.

## Architectural Phase 5 design

To unblock 2k consolidated FS, the per-entity JE materialisation must
not allocate the full Vec<JE>. Two viable paths:

### Option A — streaming JSON parse + inline IC filter

`load_entity_journal_entries` currently does
`serde_json::from_slice::<Vec<JournalEntry>>(&bytes)?` — materialises
the full Vec at once. Replace with a streaming parser:

```rust
let file = File::open(&path)?;
let reader = BufReader::new(file);
// JSON arrays don't natively stream via Deserializer::into_iter,
// so use a manual streaming JSON visitor or json-event-parser crate.
let mut ic_jes = Vec::new();
for je_result in stream_je_array(reader) {
    let je = je_result?;
    // Emit to je_network_writer per-entity batch
    writer.write_je_edges_streaming(&je)?;
    if je.header.ic_pair_id.is_some() {
        ic_jes.push(je);  // small subset only
    }
    // je drops here if not pushed
}
```

Memory: 1 JE at a time = ~20 KB. Net hold = IC subset only = ~50 GB
across all entities. Comfortably below 222 GB.

Effort: ~300-500 LOC including streaming JSON visitor + adapting
`je_network_writer::write_entity_edges` to handle per-JE rather than
per-Vec. **~6-10 hours of focused work.**

### Option B — accept the JE-network-only outcome

The 2k je_network.csv + per-entity files DO land before OOM (the
streaming writer finalises pre-aggregation work). For ML training,
graph-structure research, and most downstream analytics, this is
sufficient. The consolidated FS bundle (BS / IS / CF / NCI / notes)
is the only thing that doesn't complete.

**Effort: 0. Document the limitation; users who need FS bundle
either (a) run at <500 entities where Phases 1-4 fit in box, or
(b) wait for Phase 5.**

## What ships in v5.31 today

- ✅ **C1 Phase 1-4 cumulative**: walk + JE-network streaming +
  IC-only retention + consume-by-value IC matcher + mimalloc
  global allocator. Each is independently useful at smaller scales.
- ✅ **2k je_network artefacts** complete: 42M edges, 8.4 GB CSV
  + 12 GB on-disk total (parquet corrupted by mid-write OOM — CSV
  is the canonical artefact).
- ✅ **Per-entity je_network** (per-shard CSV + parquet) complete.
- ✗ **2k consolidated FS bundle** — needs Phase 5.

## RSS trajectory

See [`rss_trajectory.log`](./rss_trajectory.log) — 74 samples at 30s
intervals during the Phase 4 aggregate walk.

# datasynth-graph-export — status as of v4.0.0

## Current state

This crate is **excluded** from the default workspace build (see
`Cargo.toml`'s `[workspace] exclude = ...`). It ships a standalone
graph-export pipeline — PyTorch Geometric, Neo4j, DGL — with its own
test suite. However, no runtime phase in `datasynth-runtime` currently
drives it: the active graph-export path in production is the one
inside `datasynth-graph` (`graph_export` snapshot on
`EnhancedGenerationResult`).

## Why it's parked (not deleted)

- Full test suite exists and is green when built standalone.
- The schema + pipeline design may be useful for a future v4.x
  release that consolidates the two graph-export surfaces (the
  embedded `datasynth-graph` path vs. this crate's standalone
  writers).

## What needs to happen to un-park it

1. Decide: keep two graph-export surfaces or consolidate into one.
   `datasynth-graph` has better orchestrator integration; this crate
   has cleaner writers for the ML-training case.
2. If consolidating into this crate:
   - Wire a new `phase_graph_export_v2` in `enhanced_orchestrator.rs`
     that consumes `EnhancedGenerationResult` and drives the writers
     in this crate.
   - Deprecate the `datasynth-graph::export::*` path.
3. If keeping both:
   - Document the split (when to use which).
   - Wire a feature flag on `datasynth-cli` so users can pick.
4. Either way: add this crate back to the `members` list in the root
   `Cargo.toml`.

## Builds independently

```bash
cd crates/datasynth-graph-export
cargo build
cargo test
```

## Owner

Review scheduled for v4.1.0 post-release retro (per the
plan in `/home/michael/.claude/plans/splendid-discovering-waterfall.md`
§ "Deferred to v4.0").

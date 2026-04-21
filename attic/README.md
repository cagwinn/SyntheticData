# attic/

Parked crates that are excluded from the default workspace build.

Parking here (vs. deletion) means:

- The code + tests are preserved in git history and on disk.
- `Cargo.toml`'s `[workspace] exclude` list keeps them out of
  day-to-day `cargo build --workspace`.
- Re-introduction is a move back to `crates/` + an edit of
  `Cargo.toml`'s `members` list.

## Current residents (v4.1.5+)

### `datasynth-graph-export`

Standalone graph-export pipeline — PyTorch Geometric, Neo4j, DGL
writers. Parked in favour of the in-workspace `datasynth-graph`
crate, which is wired into `EnhancedOrchestrator` and produces the
`graph_export` snapshot that all production callers consume.

See `attic/datasynth-graph-export/STATUS.md` for the un-park
checklist — consolidation or re-wire decision documented there.

## Unparking workflow

1. `git mv attic/<crate> crates/<crate>`
2. Add `"crates/<crate>"` to the `members` list in the root
   `Cargo.toml` and remove the `exclude` entry.
3. `cargo check -p <crate>` to confirm it still builds.
4. Ship the un-park in a release that clearly calls out the
   re-introduction (CHANGELOG + README architecture diagram).

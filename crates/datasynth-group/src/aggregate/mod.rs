//! Aggregate phase — Chunk 5.
//!
//! After the shard phase has driven [`crate::shard::run_shard`] for every
//! shard in the manifest, the aggregate phase walks the resulting
//! `{out_dir}/entities/{code}/` subtrees, loads each entity's per-entity
//! artefacts back into memory, and combines them into group-level
//! consolidation outputs (group trial balance, eliminations,
//! NCI measurement, segment reporting, FX translation results).
//!
//! Task 5.1 lays the foundation: a per-entity trial balance loader that
//! later modules (group TB combiner, IC elimination engine, NCI roll-up,
//! segment aggregator) will all build on. The loader's contract is
//! deliberately narrow — open one file, deserialise one [`TrialBalance`],
//! re-verify the balance invariant, return it — so the higher-level
//! combiners can assume a clean per-entity TB or a typed
//! [`crate::errors::GroupError::Aggregate`] failure they can attribute to
//! a specific entity directory.
//!
//! See `docs/superpowers/specs/2026-04-23-group-audit-simulation-design.md`
//! §"Aggregate phase" for the full Chunk-5 module layout.

pub mod ic_matcher;
pub mod pre_elim;
pub mod tb_loader;

pub use ic_matcher::{
    match_ic_pairs, IcMatchResult, IcMatchedPair, UnmatchedReason, UnmatchedSide,
};
pub use pre_elim::{aggregate_pre_elimination, AggregatedAccount, AggregatedTb, DeferredEntity};
pub use tb_loader::load_entity_trial_balance;

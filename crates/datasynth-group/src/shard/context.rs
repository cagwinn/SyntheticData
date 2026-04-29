//! Build a per-entity [`ShardContext`] from a group manifest (Task 4.1).
//!
//! A shard runner calls [`build_shard_context`] once with the shard's own
//! `entity_code` to obtain the opaque context the orchestrator in
//! `datasynth-runtime` consumes.  The context carries:
//!
//! - `entity_code` — the entity this shard is responsible for;
//! - `entity_seed` — the 32-byte per-entity seed decoded from the manifest
//!   entry's hex-encoded `entity_seed` string;
//! - `extra_journal_entries` — the IC journal entries this entity must post
//!   as either seller or buyer of each relationship it participates in.
//!
//! # v5.0 scope
//!
//! Only IC journal-entry injection is wired through the context.  FX,
//! chart-of-accounts, and shared-master pool wiring lands in v5.0 Task 4.2
//! via `GeneratorConfig` — those concerns deliberately don't leak into the
//! opaque runtime [`ShardContext`] surface.
//!
//! # Determinism
//!
//! Two calls with the same manifest and entity code return contexts with
//! byte-identical `entity_code`, `entity_seed`, and
//! `extra_journal_entries.len()`.  Full-struct equality does **not** hold
//! because `JournalEntryHeader::new` assigns a wall-clock-based
//! `document_id` via `Uuid::now_v7()` — the deterministic surface is the
//! set of IC-specific header fields, accounts, and amounts (see
//! `tests/shard_context.rs::test_deterministic_across_calls`).

use datasynth_runtime::ShardContext;

use crate::errors::{GroupError, GroupResult};
use crate::manifest::builder::GroupManifest;
use crate::shard::ic_je_injector::{inject_ic_journal_entries, InjectionCtx};
use crate::shard::ic_plan::derive_ic_pair_plans;

/// Build the [`ShardContext`] for `entity_code`, using the group
/// [`GroupManifest`] as the single source of truth.
///
/// # Errors
///
/// Returns [`GroupError::Config`] if `entity_code` does not match any
/// entity in `manifest.ownership_graph.entities`.
///
/// # Panics
///
/// Panics if the manifest entity's `entity_seed` field is not lowercase
/// 32-byte hex.  This is an internal invariant violation — the manifest
/// builder (see `manifest/builder.rs`) always produces a 32-byte
/// blake3 digest hex-encoded via `hex::encode`, so hitting this panic
/// means the manifest was corrupted after construction.
pub fn build_shard_context(
    manifest: &GroupManifest,
    entity_code: &str,
) -> GroupResult<ShardContext> {
    // 1. Locate the entity in the manifest.  A typo here is a caller bug,
    //    not a data problem — surface it clearly through `GroupError::Config`.
    let entity = manifest
        .ownership_graph
        .entities
        .iter()
        .find(|e| e.code == entity_code)
        .ok_or_else(|| {
            GroupError::Config(format!(
                "build_shard_context: unknown entity_code `{entity_code}` — not in manifest.ownership_graph.entities"
            ))
        })?;

    // 2. Decode the per-entity seed back to [u8; 32].  The manifest builder
    //    guarantees this format (see `ManifestEntity::entity_seed` docs), so
    //    any failure here signals manifest corruption and should panic.
    let seed_bytes =
        hex::decode(&entity.entity_seed).expect("manifest entity_seed must be lowercase hex");
    let entity_seed: [u8; 32] = seed_bytes
        .try_into()
        .expect("manifest entity_seed must be 32 bytes");

    // 3. Derive the IC pair plans this shard is responsible for.  Returns
    //    an empty Vec when the entity is not a participant in any IC
    //    relationship — no error, no panic.
    let plans = derive_ic_pair_plans(manifest, entity_code);

    // 4. Turn each plan into a balanced JE for this entity.
    let ctx = InjectionCtx {
        entity_code: entity_code.to_string(),
    };
    let extra_journal_entries = inject_ic_journal_entries(&plans, &ctx);

    Ok(ShardContext {
        entity_code: entity_code.to_string(),
        entity_seed,
        extra_journal_entries,
    })
}

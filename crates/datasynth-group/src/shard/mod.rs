//! Shard-phase derivations — plans derived per-entity from the manifest.
//!
//! A shard is the slice of work a single entity's generator executes.  Every
//! shard-level plan is deterministically derived from the [`GroupManifest`]
//! and the shard's own `entity_code`; no cross-shard communication is
//! required.  See spec §5 ("Shard phase").
//!
//! [`GroupManifest`]: crate::manifest::GroupManifest

pub mod ic_je_injector;
pub mod ic_plan;

pub use ic_je_injector::{
    buyer_accounts, inject_ic_journal_entries, seller_accounts, InjectionCtx,
};
pub use ic_plan::{avg_amount, derive_ic_pair_plans, IcPairPlan, IcRole};

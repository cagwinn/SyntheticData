//! Manifest builder — produces the JSON artifact that drives shard and
//! aggregate phases. See spec §4.

pub mod expansion;
pub mod seeds;

pub use expansion::{expand_ownership, EntitySource, ExpandedEntity};
pub use seeds::{
    chacha_rng_from_seed, derive_aggregate_seed, derive_entity_seed, derive_ic_pair_id,
    derive_manifest_seed,
};

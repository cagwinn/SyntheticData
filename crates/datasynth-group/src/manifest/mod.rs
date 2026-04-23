//! Manifest builder — produces the JSON artifact that drives shard and
//! aggregate phases. See spec §4.

pub mod coa_master;
pub mod expansion;
pub mod fx_master;
pub mod ic_expansion;
pub mod seeds;

pub use coa_master::{build_coa_master, ChartOfAccountsMaster};
pub use expansion::{expand_ownership, EntitySource, ExpandedEntity};
pub use fx_master::{build_fx_master, FxRateMaster};
pub use ic_expansion::{expand_ic_relationships, IcSource, ResolvedIcRelationship};
pub use seeds::{
    chacha_rng_from_seed, derive_aggregate_seed, derive_entity_seed, derive_ic_pair_id,
    derive_manifest_seed,
};

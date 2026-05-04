//! Manifest builder — produces the JSON artifact that drives shard and
//! aggregate phases. See spec §4.

pub mod audit_plan;
pub mod builder;
pub mod cgu_plan;
pub mod coa_master;
pub mod expansion;
pub mod fx_master;
pub mod ic_expansion;
pub mod seeds;
pub mod shard_plan;
pub mod tax_plan;

pub use audit_plan::{
    build_audit_engagement_plan, AuditEngagementPlan, ComponentAuditor,
    ComponentMaterialityAllocation, ComponentScope,
};
pub use builder::{
    build_manifest, GroupManifest, ManifestEntity, ManifestPeriod, OwnershipGraphSection,
    MANIFEST_SCHEMA_VERSION,
};
pub use cgu_plan::{build_cgu_plan, CguPlan};
pub use coa_master::{build_coa_master, ChartOfAccountsMaster};
pub use expansion::{expand_ownership, EntitySource, ExpandedEntity};
pub use fx_master::{build_fx_master, FxRateMaster};
pub use ic_expansion::{expand_ic_relationships, IcSource, ResolvedIcRelationship};
pub use seeds::{
    chacha_rng_from_seed, derive_aggregate_seed, derive_entity_seed, derive_ic_pair_id,
    derive_manifest_seed,
};
pub use shard_plan::{build_shard_plan, ShardAssignment, ShardPlan};
pub use tax_plan::{
    build_tax_group_plan, CbcReportPlan, PillarTwoPlan, TaxGroupPlan, TransferPricingPlan,
};

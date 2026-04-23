//! DataSynth group audit simulation engine.
//!
//! Manifest / shard / aggregate three-phase model layered above
//! [`datasynth_runtime::EnhancedOrchestrator`]. See
//! `docs/superpowers/specs/2026-04-23-group-audit-simulation-design.md`.

pub mod config;
pub mod errors;
pub mod manifest;
pub mod resolve;
pub mod validate;

pub use config::{
    AuditEngagementConfig, CbcReportConfig, ComponentScopeThresholds, ConsolidationMethod,
    EntityConfig, FleetConfig, FxConfig, FxPolicyConfig, FxRateBasis, FxRateSource,
    GeneratedEntityBlock, GroupConfig, GroupMaterialityConfig, IcMatchingConfig,
    IcMatchingStrategy, IcPattern, IcRelationshipConfig, IcRelationshipExplicit,
    IcRelationshipPattern, IcTransactionType, IntercompanyConfig, MaterialityBasis,
    OutputCompression, OutputLayout, OutputLayoutConfig, OwnershipConfig, PeriodConfig,
    PeriodLength, PillarTwoConfig, TaxGroupConfig, TpConfig, TransferPricingMethod,
};
pub use errors::{GroupError, GroupResult};
pub use manifest::{
    chacha_rng_from_seed, derive_aggregate_seed, derive_entity_seed, derive_ic_pair_id,
    derive_manifest_seed, expand_ownership, EntitySource, ExpandedEntity,
};
pub use resolve::{resolve_entity, ResolvedEntity};

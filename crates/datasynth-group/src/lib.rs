//! DataSynth group audit simulation engine.
//!
//! Manifest / shard / aggregate three-phase model layered above
//! [`datasynth_runtime::EnhancedOrchestrator`]. See
//! `docs/superpowers/specs/2026-04-23-group-audit-simulation-design.md`.

pub mod aggregate;
pub mod config;
pub mod errors;
pub mod manifest;
pub mod resolve;
pub mod shard;
pub mod validate;

pub use aggregate::{
    aggregate_pre_elimination, apply_eliminations_to_tb, build_coverage_report, classify_account,
    compute_cta, cta_rollforward, eliminations_to_journal_entries, generate_eliminations,
    load_entity_trial_balance, match_ic_pairs, translate_entity_tb, write_coverage_report,
    write_cta_rollforward, write_translation_worksheet, AggregatedAccount, AggregatedTb,
    CoverageReport, CtaRollforward, DeferredEntity, DrCr, EliminationResult, IcMatchResult,
    IcMatchedPair, RateBasis, TranslatedLine, TranslatedTb, TranslationAccountType,
    UnmatchedReason, UnmatchedSide, WorksheetEntity, WorksheetLine, CONSOLIDATED_SUBDIR,
    COVERAGE_REPORT_FILENAME, COVERAGE_REPORT_SUBDIR, CTA_ROLLFORWARD_FILENAME,
    TRANSLATION_WORKSHEET_FILENAME, UNMATCHED_SAMPLE_CAP,
};
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
    build_audit_engagement_plan, build_coa_master, build_manifest, build_shard_plan,
    build_tax_group_plan, chacha_rng_from_seed, derive_aggregate_seed, derive_entity_seed,
    derive_ic_pair_id, derive_manifest_seed, expand_ic_relationships, expand_ownership,
    AuditEngagementPlan, CbcReportPlan, ChartOfAccountsMaster, ComponentAuditor,
    ComponentMaterialityAllocation, ComponentScope, EntitySource, ExpandedEntity, FxRateMaster,
    GroupManifest, IcSource, ManifestEntity, ManifestPeriod, OwnershipGraphSection, PillarTwoPlan,
    ResolvedIcRelationship, ShardAssignment, ShardPlan, TaxGroupPlan, TransferPricingPlan,
    MANIFEST_SCHEMA_VERSION,
};
pub use resolve::{resolve_entity, ResolvedEntity};

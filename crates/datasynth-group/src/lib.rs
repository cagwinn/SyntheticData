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
pub mod standalone;
pub mod validate;

pub use aggregate::{
    aggregate_pre_elimination, apply_eliminations_to_tb, apply_nci_and_equity_method,
    build_consolidated_balance_sheet, build_consolidated_balance_sheet_with_names,
    build_consolidated_cash_flow, build_consolidated_income_statement,
    build_consolidated_income_statement_with_names, build_consolidation_schedule,
    build_coverage_report, build_notes_to_consolidated_fs, build_statement_of_changes_in_equity,
    classify_account, compute_cta, compute_equity_method_investment, compute_nci_rollforward,
    cta_rollforward, eliminations_to_journal_entries, generate_eliminations,
    ingest_opening_equity_method_carrying_values, ingest_opening_nci_balances,
    ingest_opening_suppressed_losses, load_entity_trial_balance, match_ic_pairs, run_aggregate,
    run_aggregate_chain, translate_entity_tb, translate_entity_tb_with_hyperinflation,
    write_consolidated_fs, write_coverage_report, write_cta_rollforward,
    write_equity_method_investments, write_nci_rollforward, write_suppressed_losses,
    write_translation_worksheet, AccountNameDictionary, AggregateOptions, AggregateSummary,
    AggregatedAccount, AggregatedTb, BsLine, CashFlowInputs, CfLine, CfSection,
    ConsolidatedBalanceSheet, ConsolidatedCashFlow, ConsolidatedFinancialStatements,
    ConsolidatedIncomeStatement, ConsolidationSchedule, CoverageReport, CtaRollforward,
    DeferredEntity, DrCr, EliminationResult, EquityChangesInputs, EquityMethodInputs,
    EquityMethodInvestment, EquityRollforward, IcMatchResult, IcMatchedPair, IsLine, NciInputs,
    NciRollforward, Note, NotesInputs, NotesToConsolidatedFs, PeriodSpec, RateBasis, ScheduleLine,
    StatementOfChangesInEquity, TranslatedLine, TranslatedTb, TranslationAccountType,
    UnmatchedReason, UnmatchedSide, WorksheetEntity, WorksheetLine, CONSOLIDATED_FS_FILENAME,
    CONSOLIDATED_SUBDIR, CONSOLIDATION_SCHEDULE_FILENAME, COVERAGE_REPORT_FILENAME,
    COVERAGE_REPORT_SUBDIR, CTA_ROLLFORWARD_FILENAME, EQUITY_METHOD_INVESTMENTS_FILENAME,
    NCI_ROLLFORWARD_FILENAME, NOTES_FILENAME, TRANSLATION_WORKSHEET_FILENAME, UNMATCHED_SAMPLE_CAP,
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
pub use standalone::{generate_standalone, StandaloneOptions, StandaloneSummary};

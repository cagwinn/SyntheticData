//! `group:` YAML configuration types (spec §3).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level group engagement config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupConfig {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub presentation_currency: String,
    pub period: PeriodConfig,
    pub seed: u64,

    #[serde(default)]
    pub defaults: serde_yaml::Value, // inherited into each entity; opaque at this layer

    #[serde(default)]
    pub scoping_profiles: BTreeMap<String, serde_yaml::Value>,

    pub ownership: OwnershipConfig,

    #[serde(default)]
    pub intercompany: IntercompanyConfig,

    pub fx: FxConfig,

    #[serde(default)]
    pub audit: AuditEngagementConfig,

    #[serde(default)]
    pub tax: TaxGroupConfig,

    #[serde(default)]
    pub output: OutputLayoutConfig,

    #[serde(default)]
    pub fleet: Option<FleetConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeriodConfig {
    pub start_date: NaiveDate,
    pub length: PeriodLength,
    #[serde(default)]
    pub fiscal_year_end: Option<NaiveDate>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PeriodLength {
    Monthly,
    Quarterly,
    SemiAnnual,
    Annual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnershipConfig {
    pub parent_entity_code: String,
    #[serde(default)]
    pub entities: Vec<EntityConfig>,
    #[serde(default)]
    pub generated: Vec<GeneratedEntityBlock>,
    #[serde(default)]
    pub entities_from: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityConfig {
    pub code: String,
    #[serde(default)]
    pub name: Option<String>,
    pub country: String,
    pub functional_currency: String,
    pub scoping_profile: String,
    pub consolidation_method: ConsolidationMethod,
    #[serde(default)]
    pub ownership_percent: Option<Decimal>,
    #[serde(default)]
    pub parent_code: Option<String>,
    #[serde(default)]
    pub acquisition_date: Option<NaiveDate>,
    #[serde(default)]
    pub accounting_framework: Option<String>,
    #[serde(default)]
    pub industry: Option<String>,
    #[serde(default)]
    pub rows: Option<u64>,
    #[serde(default, flatten)]
    pub overrides: BTreeMap<String, serde_yaml::Value>, // generic per-entity overrides
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationMethod {
    Parent,
    Full,
    EquityMethod,
    Proportional,
    FairValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedEntityBlock {
    pub count: u32,
    pub code_prefix: String,
    #[serde(default)]
    pub country: Vec<String>,
    #[serde(default)]
    pub functional_currency: Option<String>,
    pub scoping_profile: String,
    pub consolidation_method: ConsolidationMethod,
    #[serde(default)]
    pub ownership_percent_range: Option<[Decimal; 2]>,
    #[serde(default)]
    pub parent_code: Option<String>,
    #[serde(default)]
    pub accounting_framework: Option<String>,
    #[serde(default)]
    pub industry: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntercompanyConfig {
    #[serde(default)]
    pub relationships: Vec<IcRelationshipConfig>,
    #[serde(default)]
    pub matching: IcMatchingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IcRelationshipConfig {
    Explicit(IcRelationshipExplicit),
    Pattern(IcRelationshipPattern),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcRelationshipExplicit {
    pub seller: String,
    pub buyer: String,
    pub types: Vec<IcTransactionType>,
    pub annual_volume: Decimal,
    #[serde(default)]
    pub transfer_pricing: Option<TransferPricingMethod>,
    #[serde(default)]
    pub markup_percent: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcRelationshipPattern {
    pub pattern: IcPattern,
    pub types: Vec<IcTransactionType>,
    pub per_pair_volume: Decimal,
    #[serde(default)]
    pub transfer_pricing: Option<TransferPricingMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcPattern {
    #[serde(default)]
    pub seller_scoping_profile: Option<String>,
    #[serde(default)]
    pub buyer_scoping_profile: Option<String>,
    #[serde(default)]
    pub seller: Option<String>,
    #[serde(default)]
    pub buyer: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IcTransactionType {
    GoodsSale,
    ServiceProvided,
    ManagementFee,
    Royalty,
    CostSharing,
    LoanInterest,
    Dividend,
    ExpenseRecharge,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferPricingMethod {
    CostPlus,
    ComparableUncontrolled,
    ResalePrice,
    TransactionalNetMargin,
    ProfitSplit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcMatchingConfig {
    #[serde(default = "default_strategy")]
    pub strategy: IcMatchingStrategy,
    #[serde(default = "default_coverage_target")]
    pub coverage_target: f64,
}

fn default_strategy() -> IcMatchingStrategy {
    IcMatchingStrategy::ManifestDriven
}
fn default_coverage_target() -> f64 {
    0.98
}

impl Default for IcMatchingConfig {
    fn default() -> Self {
        Self {
            strategy: default_strategy(),
            coverage_target: default_coverage_target(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IcMatchingStrategy {
    ManifestDriven,
    EmergentFuzzy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxConfig {
    pub base_currency: String,
    #[serde(default)]
    pub rate_source: FxRateSource,
    #[serde(default)]
    pub rates: BTreeMap<String, BTreeMap<NaiveDate, Decimal>>,
    pub policy: FxPolicyConfig,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FxRateSource {
    #[default]
    Inline,
    UserSupplied,
    HistoricalSeries,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxPolicyConfig {
    pub balance_sheet: FxRateBasis,
    pub income_statement: FxRateBasis,
    pub equity: FxRateBasis,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FxRateBasis {
    #[default]
    Closing,
    Average,
    Historical,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditEngagementConfig {
    #[serde(default)]
    pub engagement_id: Option<String>,
    #[serde(default)]
    pub lead_auditor: Option<String>,
    #[serde(default)]
    pub framework: Option<String>, // isa | pcaob | dual
    #[serde(default)]
    pub fsm_blueprint: Option<String>,
    #[serde(default)]
    pub group_materiality: Option<GroupMaterialityConfig>,
    #[serde(default)]
    pub component_scope_thresholds: Option<ComponentScopeThresholds>,
    #[serde(default)]
    pub generate_kams: bool,
    #[serde(default)]
    pub generate_group_opinion: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMaterialityConfig {
    pub basis: MaterialityBasis,
    pub percent: Decimal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MaterialityBasis {
    Revenue,
    Assets,
    PretaxIncome,
    Equity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentScopeThresholds {
    pub full_scope: Decimal,
    pub specific_scope: Decimal,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaxGroupConfig {
    #[serde(default)]
    pub pillar_two: Option<PillarTwoConfig>,
    #[serde(default)]
    pub cbc_report: Option<CbcReportConfig>,
    #[serde(default)]
    pub transfer_pricing: Option<TpConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PillarTwoConfig {
    pub enabled: bool,
    #[serde(default)]
    pub jurisdictions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CbcReportConfig {
    pub enabled: bool,
    #[serde(default)]
    pub reporting_jurisdiction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TpConfig {
    #[serde(default)]
    pub master_file: bool,
    #[serde(default)]
    pub local_files_for: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutputLayoutConfig {
    #[serde(default = "default_layout")]
    pub layout: OutputLayout,
    #[serde(default = "default_true")]
    pub shared_masters_at_root: bool,
    #[serde(default)]
    pub compression: Option<OutputCompression>,
}

fn default_layout() -> OutputLayout {
    OutputLayout::PerEntitySubtree
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputLayout {
    #[default]
    PerEntitySubtree,
    Flat,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputCompression {
    Json,
    Csv,
    Parquet,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FleetConfig {
    #[serde(default)]
    pub dispatcher: Option<String>, // v5.3: in_process | subprocess | remote
    #[serde(default)]
    pub max_concurrent_shards: Option<u32>,
    #[serde(default)]
    pub per_shard_timeout_seconds: Option<u64>,
}

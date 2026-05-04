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

    /// **v5.2** — IAS 36 § 10 cash-generating-unit (CGU) plan: defines
    /// the CGUs the engagement tests for goodwill impairment + the
    /// goodwill amounts allocated to each CGU at acquisition date.
    /// Empty by default; engagements without CGU allocations skip the
    /// annual impairment test entirely (no `consolidated/cgu_impairment_tests.json`
    /// is emitted).
    #[serde(default)]
    pub cgu: CguConfig,

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
    /// **v5.2** — IFRS 3 § 41-42 / IFRS 10 § 23 / IFRS 10.B97
    /// ownership-change events that affected this entity during the
    /// reporting period.  Each entry describes one mid-period
    /// transition: control gained (new acquisition or increase from
    /// associate), control increased / decreased within consolidation
    /// (equity-transaction treatment per IFRS 10.23), or control lost
    /// (deconsolidation per IFRS 10.B97).  Empty by default; the
    /// shard runner emits `intercompany/ownership_change_events.json`
    /// per entity only when this list is non-empty so v5.0–v5.1
    /// archives stay byte-identical.  The aggregate-phase rollforward
    /// wiring (consuming these events to drive proper IFRS 3 / IFRS
    /// 10 NCI treatment) is a follow-up PR.
    #[serde(default)]
    pub ownership_changes: Vec<OwnershipChangeEntry>,
    /// **v5.2** — IAS 29 hyperinflationary status of this entity's
    /// functional currency.  Defaults to `NotHyperinflationary`,
    /// preserving v5.0–v5.1 behaviour byte-for-byte.  When set to
    /// `Hyperinflationary`, the aggregate phase will (in a follow-up
    /// PR) apply IAS 29 § 12 restatement to non-monetary items
    /// before IAS 21 closing-rate translation per IAS 21 § 42(b).
    #[serde(default)]
    pub hyperinflation_status: datasynth_core::models::HyperinflationStatus,
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IcMatchingConfig {
    #[serde(default = "default_strategy")]
    pub strategy: IcMatchingStrategy,
    #[serde(default = "default_coverage_target")]
    pub coverage_target: f64,
    /// **v5.3** — fuzzy-matching amount-drift tolerance, expressed as
    /// a percentage of the larger of the seller / buyer side amounts.
    /// Used **only** when `strategy == EmergentFuzzy`.  When the
    /// observed drift exceeds this tolerance, the matcher rejects the
    /// pair with [`crate::aggregate::ic_matcher::UnmatchedReason::AmountDriftAboveTolerance`]
    /// instead of silently treating it as matched.
    ///
    /// Default `0.0` means "exact match required" — the v5.0–v5.2
    /// `ManifestDriven` contract.  A typical fuzzy-mode value is
    /// `0.005` (50 bps) which absorbs FX-rounding drift but flags
    /// genuine reconciliation breaks.
    #[serde(default = "default_tolerance_percent")]
    pub tolerance_percent: rust_decimal::Decimal,
}

fn default_strategy() -> IcMatchingStrategy {
    IcMatchingStrategy::ManifestDriven
}
fn default_coverage_target() -> f64 {
    0.98
}
fn default_tolerance_percent() -> rust_decimal::Decimal {
    rust_decimal::Decimal::ZERO
}

impl Default for IcMatchingConfig {
    fn default() -> Self {
        Self {
            strategy: default_strategy(),
            coverage_target: default_coverage_target(),
            tolerance_percent: default_tolerance_percent(),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
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

/// **v5.2** — IAS 36 § 10 cash-generating-unit (CGU) plan.
///
/// Engagement-static CGU configuration: defines the CGUs the engagement
/// will test for annual goodwill impairment + the goodwill amounts
/// allocated to each CGU at the original acquisition date (IAS 36 § 80).
/// Per-period test inputs (fair value less costs of disposal, value in
/// use) live elsewhere — outside the manifest layer this PR ships.
///
/// Empty by default: an engagement without configured CGUs simply
/// skips the impairment test phase and emits no
/// `consolidated/cgu_impairment_tests.json` artefact.  This preserves
/// backwards compatibility byte-for-byte for v5.0–v5.1 archives.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CguConfig {
    /// CGU definitions.  IDs must be unique within the engagement;
    /// each CGU's `member_entity_codes` must reference entities that
    /// exist in `ownership.entities`.  Validation happens in
    /// [`crate::manifest::cgu_plan::build_cgu_plan`].
    #[serde(default)]
    pub cgus: Vec<CguDefinitionEntry>,

    /// Goodwill allocations (one per (CGU, business combination) pair).
    /// Amounts are non-negative; bargain purchases produce no
    /// goodwill and therefore no allocation row.  Each entry's
    /// `cgu_id` must reference an entry in `cgus`; the
    /// `business_combination_id` is loosely validated (BC files live
    /// per-shard so cross-validation is deferred to the aggregate
    /// phase).
    #[serde(default)]
    pub goodwill_allocations: Vec<CguGoodwillAllocationEntry>,
}

/// One CGU definition entry.  Mirrors
/// [`datasynth_core::models::cgu::CashGeneratingUnit`] so the manifest
/// builder can lift it directly into the manifest plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CguDefinitionEntry {
    /// Stable CGU identifier — used by goodwill allocations and per-
    /// period impairment-test inputs to refer to this CGU across
    /// periods.
    pub cgu_id: String,
    /// Human-readable name (e.g. `"EMEA Consumer Products"`).
    pub name: String,
    /// Entity codes whose cash flows aggregate to form this CGU.  May
    /// span multiple legal entities (cross-entity CGU) or be a sub-
    /// division of a single entity (in which case it has one member).
    /// Must be non-empty: the manifest builder rejects empty member
    /// lists.
    #[serde(default)]
    pub member_entity_codes: Vec<String>,
    /// Optional reportable-segment attribution for IFRS 8 / ASC 280
    /// disclosure linkage.  Multiple CGUs can map to the same segment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_code: Option<String>,
}

/// One goodwill-allocation entry — links a business combination's
/// goodwill amount to one CGU at the acquisition date.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CguGoodwillAllocationEntry {
    /// CGU receiving the allocation (must reference a `cgu_id` in
    /// [`CguConfig::cgus`]).
    pub cgu_id: String,
    /// Source business-combination identifier.  Loosely validated at
    /// the manifest layer (BC files live per-shard); cross-validation
    /// of the BC's existence happens during aggregate-phase
    /// impairment-test wiring.
    pub business_combination_id: String,
    /// Allocated goodwill amount in the group presentation currency.
    /// Always non-negative; manifest builder rejects negatives.
    pub goodwill_amount: Decimal,
    /// Acquisition date the allocation took effect.
    pub allocation_date: NaiveDate,
}

/// **v5.2** — IFRS 3 § 41-42 / IFRS 10 § 23 / IFRS 10.B97 mid-period
/// ownership-change event declared on an [`EntityConfig`].
///
/// `entity_code` and `parent_entity_code` are NOT carried here — the
/// manifest builder fills them from the host [`EntityConfig::code`]
/// and [`EntityConfig::parent_code`] respectively when lifting this
/// entry into a [`datasynth_core::models::OwnershipChangeEvent`].
/// The host entity must therefore have `parent_code` set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OwnershipChangeEntry {
    /// What kind of ownership change occurred — drives IFRS 3 / IFRS 10
    /// accounting treatment.
    pub event_type: datasynth_core::models::intercompany::OwnershipChangeType,
    /// Date the change took effect (must lie within the manifest
    /// period — validated at manifest build).
    pub effective_date: NaiveDate,
    /// Parent's ownership percent immediately before the event,
    /// in `[0, 1]`.
    pub ownership_percent_before: Decimal,
    /// Parent's ownership percent immediately after the event,
    /// in `[0, 1]`.
    pub ownership_percent_after: Decimal,
    /// Carrying amount of the previously-held interest in the
    /// investor's books (IFRS 3.42 input for `ControlGained`).
    /// Only meaningful for `ControlGained` / `ControlLost`; ignored
    /// for the equity-transaction variants.
    #[serde(default)]
    pub previously_held_interest_carrying: Option<Decimal>,
    /// Acquisition-date fair value of the previously-held interest
    /// (IFRS 3.42 / IFRS 10.B97 re-measurement input).
    #[serde(default)]
    pub previously_held_interest_fair_value: Option<Decimal>,
    /// Cash / share consideration paid (positive on `ControlGained` /
    /// `ControlIncreased`) or received (negative on `ControlDecreased` /
    /// `ControlLost`).  Sign convention: positive = outflow from
    /// parent.
    pub consideration_paid_or_received: Decimal,
    /// IFRS 3 § 19 acquisition-date NCI fair value when this event
    /// triggers a new consolidation (`ControlGained` only — must be
    /// `Some(fv)` when method is `FullGoodwill`).
    #[serde(default)]
    pub acquisition_date_nci_fair_value: Option<Decimal>,
    /// Method used to measure the new NCI (only relevant for
    /// `ControlGained`).
    #[serde(default)]
    pub nci_measurement_method: datasynth_core::models::intercompany::NciMeasurementMethod,
    /// Group presentation currency the amounts are denominated in.
    pub currency: String,
}

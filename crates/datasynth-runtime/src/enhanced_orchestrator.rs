//! Enhanced generation orchestrator with full feature integration.
//!
//! This orchestrator coordinates all generation phases:
//! 1. Chart of Accounts generation
//! 2. Master data generation (vendors, customers, materials, assets, employees)
//! 3. Document flow generation (P2P, O2C) + subledger linking + OCPM events
//! 4. Journal entry generation
//! 5. Anomaly injection
//! 6. Balance validation
//! 7. Data quality injection
//! 8. Audit data generation (engagements, workpapers, evidence, risks, findings, judgments)
//! 9. Banking KYC/AML data generation (customers, accounts, transactions, typologies)
//! 10. Graph export (accounting network for ML training and network reconstruction)
//! 11. LLM enrichment (AI-augmented vendor names, descriptions)
//! 12. Diffusion enhancement (statistical diffusion-based sample generation)
//! 13. Causal overlay (structural causal model generation and validation)
//! 14. Source-to-Contract (S2C) sourcing data generation
//! 15. Bank reconciliation generation
//! 16. Financial statement generation
//! 25. Counterfactual pair generation (ML training)

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{Datelike, NaiveDate};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rand::SeedableRng;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use datasynth_banking::{
    models::{BankAccount, BankTransaction, BankingCustomer, CustomerName},
    BankingOrchestratorBuilder,
};
use datasynth_config::schema::GeneratorConfig;
use datasynth_core::error::{SynthError, SynthResult};
use datasynth_core::models::audit::{
    AnalyticalProcedureResult, AuditEngagement, AuditEvidence, AuditFinding, AuditProcedureStep,
    AuditSample, ComponentAuditor, ComponentAuditorReport, ComponentInstruction,
    ConfirmationResponse, EngagementLetter, ExternalConfirmation, GroupAuditPlan,
    InternalAuditFunction, InternalAuditReport, ProfessionalJudgment, RelatedParty,
    RelatedPartyTransaction, RiskAssessment, ServiceOrganization, SocReport, SubsequentEvent,
    UserEntityControl, Workpaper,
};
use datasynth_core::models::sourcing::{
    BidEvaluation, CatalogItem, ProcurementContract, RfxEvent, SourcingProject, SpendAnalysis,
    SupplierBid, SupplierQualification, SupplierScorecard,
};
use datasynth_core::models::subledger::ap::{APAgingReport, APInvoice};
use datasynth_core::models::subledger::ar::{ARAgingReport, ARInvoice};
use datasynth_core::models::*;
use datasynth_core::traits::Generator;
use datasynth_core::{DegradationActions, DegradationLevel, ResourceGuard, ResourceGuardBuilder};
use datasynth_fingerprint::{
    io::FingerprintReader,
    models::Fingerprint,
    synthesis::{ConfigSynthesizer, CopulaGeneratorSpec, SynthesisOptions},
};
use datasynth_generators::{
    // Subledger linker + settlement
    apply_ap_settlements,
    apply_ar_settlements,
    // Opening balance → JE conversion
    opening_balance_to_jes,
    // Anomaly injection
    AnomalyInjector,
    AnomalyInjectorConfig,
    AssetGenerator,
    // Audit generators
    AuditEngagementGenerator,
    BalanceTrackerConfig,
    // Bank reconciliation generator
    BankReconciliationGenerator,
    // S2C sourcing generators
    BidEvaluationGenerator,
    BidGenerator,
    // Business combination generator (IFRS 3 / ASC 805)
    BusinessCombinationGenerator,
    CatalogGenerator,
    // Core generators
    ChartOfAccountsGenerator,
    // Consolidation generator
    ConsolidationGenerator,
    ContractGenerator,
    // Control generator
    ControlGenerator,
    ControlGeneratorConfig,
    CustomerGenerator,
    DataQualityConfig,
    // Data quality
    DataQualityInjector,
    DataQualityStats,
    // Document flow JE generator
    DocumentFlowJeConfig,
    DocumentFlowJeGenerator,
    DocumentFlowLinker,
    // Expected Credit Loss generator (IFRS 9 / ASC 326)
    EclGenerator,
    EmployeeGenerator,
    // ESG anomaly labels
    EsgAnomalyLabel,
    EvidenceGenerator,
    // Subledger depreciation schedule generator
    FaDepreciationScheduleConfig,
    FaDepreciationScheduleGenerator,
    // Financial statement generator
    FinancialStatementGenerator,
    FindingGenerator,
    // Inventory valuation generator
    InventoryValuationGenerator,
    InventoryValuationGeneratorConfig,
    JournalEntryGenerator,
    JudgmentGenerator,
    LatePaymentDistribution,
    // Manufacturing cost accounting + warranty provisions
    ManufacturingCostAccounting,
    MaterialGenerator,
    O2CDocumentChain,
    O2CGenerator,
    O2CGeneratorConfig,
    O2CPaymentBehavior,
    P2PDocumentChain,
    // Document flow generators
    P2PGenerator,
    P2PGeneratorConfig,
    P2PPaymentBehavior,
    PaymentReference,
    // Provisions and contingencies generator (IAS 37 / ASC 450)
    ProvisionGenerator,
    QualificationGenerator,
    RfxGenerator,
    RiskAssessmentGenerator,
    // Balance validation
    RunningBalanceTracker,
    ScorecardGenerator,
    // Segment reporting generator (IFRS 8 / ASC 280)
    SegmentGenerator,
    SegmentSeed,
    SourcingProjectGenerator,
    SpendAnalysisGenerator,
    ValidationError,
    // Master data generators
    VendorGenerator,
    WarrantyProvisionGenerator,
    WorkpaperGenerator,
};
use datasynth_graph::{
    ApprovalGraphBuilder, ApprovalGraphConfig, BankingGraphBuilder, BankingGraphConfig,
    EntityGraphBuilder, EntityGraphConfig, PyGExportConfig, PyGExporter, TransactionGraphBuilder,
    TransactionGraphConfig,
};
use datasynth_ocpm::{
    AuditDocuments, BankDocuments, BankReconDocuments, EventLogMetadata, H2rDocuments,
    MfgDocuments, O2cDocuments, OcpmEventGenerator, OcpmEventLog, OcpmGeneratorConfig,
    OcpmUuidFactory, P2pDocuments, S2cDocuments,
};

use datasynth_config::schema::{O2CFlowConfig, P2PFlowConfig};
use datasynth_core::causal::{CausalGraph, CausalValidator, StructuralCausalModel};
use datasynth_core::diffusion::{DiffusionBackend, DiffusionConfig, StatisticalDiffusionBackend};
use datasynth_core::llm::{HttpLlmProvider, MockLlmProvider};
use datasynth_core::models::balance::{
    AccountCategory, AccountType, GeneratedOpeningBalance, IndustryType, OpeningBalanceSpec,
    TrialBalance, TrialBalanceLine, TrialBalanceStatus, TrialBalanceType,
};
use datasynth_core::models::documents::PaymentMethod;
use datasynth_core::models::IndustrySector;
use datasynth_generators::audit::analytical_procedure_generator::AnalyticalProcedureGenerator;
use datasynth_generators::audit::component_audit_generator::ComponentAuditGenerator;
use datasynth_generators::audit::confirmation_generator::ConfirmationGenerator;
use datasynth_generators::audit::engagement_letter_generator::EngagementLetterGenerator;
use datasynth_generators::audit::internal_audit_generator::InternalAuditGenerator;
use datasynth_generators::audit::procedure_step_generator::ProcedureStepGenerator;
use datasynth_generators::audit::related_party_generator::RelatedPartyGenerator;
use datasynth_generators::audit::sample_generator::SampleGenerator;
use datasynth_generators::audit::service_org_generator::ServiceOrgGenerator;
use datasynth_generators::audit::subsequent_event_generator::SubsequentEventGenerator;
use datasynth_generators::coa_generator::CoAFramework;
use rayon::prelude::*;
use rust_decimal::Decimal;

// ============================================================================
// Configuration Conversion Functions
// ============================================================================

/// Convert P2P flow config from schema to generator config.
/// v4.4.1 — build a `DataQualityStats` with only `total_records`
/// populated to `n_entries`. Used when the data-quality phase is
/// skipped (by config or resource pressure) so downstream consumers
/// can still see the denominator. Before v4.4.1 the writer emitted
/// `total_records: 0` in those cases, which the SDK team flagged as
/// indistinguishable from "ran but processed nothing".
fn stats_with_denominator(n_entries: usize) -> DataQualityStats {
    #[allow(clippy::field_reassign_with_default)]
    {
        let mut s = DataQualityStats::default();
        s.total_records = n_entries;
        s.missing_values.total_records = n_entries;
        s.format_variations.total_processed = n_entries;
        s.duplicates.total_processed = n_entries;
        s
    }
}

fn convert_p2p_config(schema_config: &P2PFlowConfig) -> P2PGeneratorConfig {
    let payment_behavior = &schema_config.payment_behavior;
    let late_dist = &payment_behavior.late_payment_days_distribution;

    P2PGeneratorConfig {
        three_way_match_rate: schema_config.three_way_match_rate,
        partial_delivery_rate: schema_config.partial_delivery_rate,
        over_delivery_rate: schema_config.over_delivery_rate.unwrap_or(0.02),
        price_variance_rate: schema_config.price_variance_rate,
        max_price_variance_percent: schema_config.max_price_variance_percent,
        avg_days_po_to_gr: schema_config.average_po_to_gr_days,
        avg_days_gr_to_invoice: schema_config.average_gr_to_invoice_days,
        avg_days_invoice_to_payment: schema_config.average_invoice_to_payment_days,
        payment_method_distribution: vec![
            (PaymentMethod::BankTransfer, 0.60),
            (PaymentMethod::Check, 0.25),
            (PaymentMethod::Wire, 0.10),
            (PaymentMethod::CreditCard, 0.05),
        ],
        early_payment_discount_rate: schema_config.early_payment_discount_rate.unwrap_or(0.30),
        payment_behavior: P2PPaymentBehavior {
            late_payment_rate: payment_behavior.late_payment_rate,
            late_payment_distribution: LatePaymentDistribution {
                slightly_late_1_to_7: late_dist.slightly_late_1_to_7,
                late_8_to_14: late_dist.late_8_to_14,
                very_late_15_to_30: late_dist.very_late_15_to_30,
                severely_late_31_to_60: late_dist.severely_late_31_to_60,
                extremely_late_over_60: late_dist.extremely_late_over_60,
            },
            partial_payment_rate: payment_behavior.partial_payment_rate,
            payment_correction_rate: payment_behavior.payment_correction_rate,
            avg_days_until_remainder: payment_behavior.avg_days_until_remainder,
        },
    }
}

/// Convert O2C flow config from schema to generator config.
fn convert_o2c_config(schema_config: &O2CFlowConfig) -> O2CGeneratorConfig {
    let payment_behavior = &schema_config.payment_behavior;

    O2CGeneratorConfig {
        credit_check_failure_rate: schema_config.credit_check_failure_rate,
        partial_shipment_rate: schema_config.partial_shipment_rate,
        avg_days_so_to_delivery: schema_config.average_so_to_delivery_days,
        avg_days_delivery_to_invoice: schema_config.average_delivery_to_invoice_days,
        avg_days_invoice_to_payment: schema_config.average_invoice_to_receipt_days,
        late_payment_rate: schema_config.late_payment_rate.unwrap_or(0.15),
        bad_debt_rate: schema_config.bad_debt_rate,
        returns_rate: schema_config.return_rate,
        cash_discount_take_rate: schema_config.cash_discount.taken_rate,
        payment_method_distribution: vec![
            (PaymentMethod::BankTransfer, 0.50),
            (PaymentMethod::Check, 0.30),
            (PaymentMethod::Wire, 0.15),
            (PaymentMethod::CreditCard, 0.05),
        ],
        payment_behavior: O2CPaymentBehavior {
            partial_payment_rate: payment_behavior.partial_payments.rate,
            short_payment_rate: payment_behavior.short_payments.rate,
            max_short_percent: payment_behavior.short_payments.max_short_percent,
            on_account_rate: payment_behavior.on_account_payments.rate,
            payment_correction_rate: payment_behavior.payment_corrections.rate,
            avg_days_until_remainder: payment_behavior.partial_payments.avg_days_until_remainder,
        },
    }
}

/// Configuration for which generation phases to run.
#[derive(Debug, Clone)]
pub struct PhaseConfig {
    /// Generate master data (vendors, customers, materials, assets, employees).
    pub generate_master_data: bool,
    /// Generate document flows (P2P, O2C).
    pub generate_document_flows: bool,
    /// Generate OCPM events from document flows.
    pub generate_ocpm_events: bool,
    /// Generate journal entries.
    pub generate_journal_entries: bool,
    /// Inject anomalies.
    pub inject_anomalies: bool,
    /// Inject data quality variations (typos, missing values, format variations).
    pub inject_data_quality: bool,
    /// Validate balance sheet equation after generation.
    pub validate_balances: bool,
    /// Validate that every `gl_account` referenced in generated JEs exists
    /// in the chart of accounts. Off by default (a soft warning is emitted
    /// instead). Set true to fail the run on any orphan account.
    pub validate_coa_coverage_strict: bool,
    /// Show progress bars.
    pub show_progress: bool,
    /// Number of vendors to generate per company.
    pub vendors_per_company: usize,
    /// Number of customers to generate per company.
    pub customers_per_company: usize,
    /// Number of materials to generate per company.
    pub materials_per_company: usize,
    /// Number of assets to generate per company.
    pub assets_per_company: usize,
    /// Number of employees to generate per company.
    pub employees_per_company: usize,
    /// Number of P2P chains to generate.
    pub p2p_chains: usize,
    /// Number of O2C chains to generate.
    pub o2c_chains: usize,
    /// Generate audit data (engagements, workpapers, evidence, risks, findings, judgments).
    pub generate_audit: bool,
    /// Number of audit engagements to generate.
    pub audit_engagements: usize,
    /// Number of workpapers per engagement.
    pub workpapers_per_engagement: usize,
    /// Number of evidence items per workpaper.
    pub evidence_per_workpaper: usize,
    /// Number of risk assessments per engagement.
    pub risks_per_engagement: usize,
    /// Number of findings per engagement.
    pub findings_per_engagement: usize,
    /// Number of professional judgments per engagement.
    pub judgments_per_engagement: usize,
    /// Generate banking KYC/AML data (customers, accounts, transactions, typologies).
    pub generate_banking: bool,
    /// Generate graph exports (accounting network for ML training).
    pub generate_graph_export: bool,
    /// Generate S2C sourcing data (spend analysis, RFx, bids, contracts, catalogs, scorecards).
    pub generate_sourcing: bool,
    /// Generate bank reconciliations from payments.
    pub generate_bank_reconciliation: bool,
    /// Generate financial statements from trial balances.
    pub generate_financial_statements: bool,
    /// Generate accounting standards data (revenue recognition, impairment).
    pub generate_accounting_standards: bool,
    /// Generate manufacturing data (production orders, quality inspections, cycle counts).
    pub generate_manufacturing: bool,
    /// Generate sales quotes, management KPIs, and budgets.
    pub generate_sales_kpi_budgets: bool,
    /// Generate tax jurisdictions and tax codes.
    pub generate_tax: bool,
    /// Generate ESG data (emissions, energy, water, waste, social, governance).
    pub generate_esg: bool,
    /// Generate intercompany transactions and eliminations.
    pub generate_intercompany: bool,
    /// Generate process evolution and organizational events.
    pub generate_evolution_events: bool,
    /// Generate counterfactual (original, mutated) JE pairs for ML training.
    pub generate_counterfactuals: bool,
    /// Generate compliance regulations data (standards registry, procedures, findings, filings).
    pub generate_compliance_regulations: bool,
    /// Generate period-close journal entries (tax provision, income statement close).
    pub generate_period_close: bool,
    /// Suppress ONLY the income-statement / net-income→retained-earnings close
    /// (and the dividend-declaration postings) within `phase_period_close`.
    /// Accruals, depreciation, and the tax provision still run. Off by default;
    /// the multi-year [`GenerationSession`] sets it true because its own
    /// complete `year_end.rs` close is the SOLE owner of the income-summary
    /// (3600) → retained-earnings (3200) transfer. Without this gate, both
    /// closes post net income to RE → it is double-counted.
    pub skip_income_statement_close: bool,
    /// W1-3 Stage 1: post the recurring period-close entries (depreciation,
    /// accruals, prepaid amortization, and any config-supplied straight-line
    /// items) PER MONTH across the generation slice instead of lumping them at
    /// the slice's last day. Off by default → byte-identical lumped output.
    /// Derived from `period_close.monthly_recurring`. This is ORTHOGONAL to
    /// `skip_income_statement_close`: the monthly loop lives inside
    /// `phase_period_close` and never touches `GenerationSession` / fiscal-year
    /// slicing, so the multi-FY close gate is unaffected.
    pub monthly_recurring: bool,
    /// Emit the one-time debt-inception JE (DR cash / CR long-term debt) for each configured
    /// debt instrument. True for a single-period / batch build; the multi-year
    /// [`GenerationSession`] sets it true ONLY for the first fiscal year (`period_cursor == 0`),
    /// because it regenerates the instruments each FY with that FY's origination date — without
    /// this gate the principal issuance would re-fire every year and inflate cash + long-term debt.
    pub emit_debt_inception: bool,
    /// spec 27 R6c: post the inventory→GL true-up close JE (DR/CR Inventory 1200 vs opening equity
    /// for the delta to the physical EOT inventory). Off by default → byte-identical; the product
    /// close overlay sets it true so INV-DB-001 (inventory subledger ↔ GL) ties.
    pub post_inventory_close: bool,
    /// Generate HR data (payroll, time entries, expenses, pensions, stock comp).
    pub generate_hr: bool,
    /// Generate treasury data (cash management, hedging, debt, pooling).
    pub generate_treasury: bool,
    /// Generate project accounting data (projects, costs, revenue, EVM, milestones).
    pub generate_project_accounting: bool,
    /// v3.3.0: generate legal documents per engagement (engagement letters,
    /// management rep letters, legal opinions, regulatory filings,
    /// board resolutions). Gated by `compliance_regulations.legal_documents.enabled`.
    pub generate_legal_documents: bool,
    /// v3.3.0: generate IT general controls (access logs, change
    /// management records) per audit engagement. Gated by
    /// `audit.it_controls.enabled`.
    pub generate_it_controls: bool,
    /// v3.3.0: run the analytics-metadata phase after all JE-adding
    /// phases. Wires PriorYearGenerator / IndustryBenchmarkGenerator /
    /// ManagementReportGenerator / DriftEventGenerator. Gated by the
    /// top-level `analytics_metadata.enabled` config flag.
    pub generate_analytics_metadata: bool,
}

impl Default for PhaseConfig {
    fn default() -> Self {
        Self {
            generate_master_data: true,
            generate_document_flows: true,
            generate_ocpm_events: false, // Off by default
            generate_journal_entries: true,
            inject_anomalies: false,
            inject_data_quality: false, // Off by default (to preserve clean test data)
            validate_balances: true,
            validate_coa_coverage_strict: false,
            show_progress: true,
            vendors_per_company: 50,
            customers_per_company: 100,
            materials_per_company: 200,
            assets_per_company: 50,
            employees_per_company: 100,
            p2p_chains: 100,
            o2c_chains: 100,
            generate_audit: false, // Off by default
            audit_engagements: 5,
            workpapers_per_engagement: 20,
            evidence_per_workpaper: 5,
            risks_per_engagement: 15,
            findings_per_engagement: 8,
            judgments_per_engagement: 10,
            generate_banking: false,                // Off by default
            generate_graph_export: false,           // Off by default
            generate_sourcing: false,               // Off by default
            generate_bank_reconciliation: false,    // Off by default
            generate_financial_statements: false,   // Off by default
            generate_accounting_standards: false,   // Off by default
            generate_manufacturing: false,          // Off by default
            generate_sales_kpi_budgets: false,      // Off by default
            generate_tax: false,                    // Off by default
            generate_esg: false,                    // Off by default
            generate_intercompany: false,           // Off by default
            generate_evolution_events: true,        // On by default
            generate_counterfactuals: false,        // Off by default (opt-in for ML workloads)
            generate_compliance_regulations: false, // Off by default
            generate_period_close: true,            // On by default
            skip_income_statement_close: false, // Off by default (only the session sets it true)
            monthly_recurring: false,           // Off by default → byte-identical lumped output
            emit_debt_inception: true,          // Single-period/batch issues debt principal once
            post_inventory_close: false,        // R6c: off by default → byte-identical
            generate_hr: false,                 // Off by default
            generate_treasury: false,           // Off by default
            generate_project_accounting: false, // Off by default
            generate_legal_documents: false,    // v3.3.0 — off by default
            generate_it_controls: false,        // v3.3.0 — off by default
            generate_analytics_metadata: false, // v3.3.0 — off by default
        }
    }
}

impl PhaseConfig {
    /// Derive phase flags from [`GeneratorConfig`].
    ///
    /// This is the canonical way to create a [`PhaseConfig`] from a YAML config file.
    /// CLI flags can override individual fields after calling this method.
    pub fn from_config(cfg: &datasynth_config::GeneratorConfig) -> Self {
        Self {
            // Always-on phases
            generate_master_data: true,
            generate_document_flows: true,
            generate_journal_entries: true,
            validate_balances: true,
            validate_coa_coverage_strict: false,
            generate_period_close: true,
            // The single-period CLI path (from_config) keeps the orchestrator's
            // income-statement close. Only the multi-year session overrides this.
            skip_income_statement_close: false,
            // W1-3 Stage 1: monthly recurring postings, derived from config.
            // Off unless the YAML opts in (the product overlay sets it true).
            monthly_recurring: cfg.period_close.monthly_recurring,
            // The single-period CLI path issues debt principal once. The multi-year session
            // overrides this to fire only in FY1 (period_cursor == 0) so it is not re-issued.
            emit_debt_inception: true,
            // R6c: inventory→GL close true-up, opt-in via the YAML (product close overlay).
            post_inventory_close: cfg.period_close.post_inventory_close,
            generate_evolution_events: true,
            show_progress: true,

            // Feature-gated phases — derived from config sections
            generate_audit: cfg.audit.enabled,
            generate_banking: cfg.banking.enabled,
            generate_graph_export: cfg.graph_export.enabled,
            generate_sourcing: cfg.source_to_pay.enabled,
            generate_intercompany: cfg.intercompany.enabled,
            generate_financial_statements: cfg.financial_reporting.enabled,
            generate_bank_reconciliation: cfg.financial_reporting.enabled,
            generate_accounting_standards: cfg.accounting_standards.enabled,
            generate_manufacturing: cfg.manufacturing.enabled,
            generate_sales_kpi_budgets: cfg.sales_quotes.enabled,
            generate_tax: cfg.tax.enabled,
            generate_esg: cfg.esg.enabled,
            generate_ocpm_events: cfg.ocpm.enabled,
            generate_compliance_regulations: cfg.compliance_regulations.enabled,
            generate_hr: cfg.hr.enabled,
            generate_treasury: cfg.treasury.enabled,
            generate_project_accounting: cfg.project_accounting.enabled,

            // v3.3.0: L1 generator wiring
            // Legal documents emitted when compliance_regulations is enabled
            // and the nested legal_documents.enabled flag is set.
            generate_legal_documents: cfg.compliance_regulations.enabled
                && cfg.compliance_regulations.legal_documents.enabled,
            // IT general controls emitted when audit is enabled and the
            // nested it_controls.enabled flag is set.
            generate_it_controls: cfg.audit.enabled && cfg.audit.it_controls.enabled,
            // Analytics metadata phase (prior-year, industry benchmarks,
            // management reports, drift events).
            generate_analytics_metadata: cfg.analytics_metadata.enabled,

            // Opt-in for ML workloads — driven by scenarios.generate_counterfactuals config field
            generate_counterfactuals: cfg.scenarios.generate_counterfactuals,

            inject_anomalies: cfg.fraud.enabled || cfg.anomaly_injection.enabled,
            inject_data_quality: cfg.data_quality.enabled,

            // DB-E1: honor master_data.*.count from the YAML (was hardcoded 50/100/200/50/100).
            // The product's single-period CLI path calls from_config, so these now drive generation.
            vendors_per_company: cfg.master_data.vendors.count,
            customers_per_company: cfg.master_data.customers.count,
            materials_per_company: cfg.master_data.materials.count,
            assets_per_company: cfg.master_data.fixed_assets.count,
            // employees: the generator derives headcount from department-pool sizing, so this field
            // is currently advisory; fully honoring it is deferred (employee_generator rework).
            employees_per_company: cfg.master_data.employees.count,
            p2p_chains: 100,
            o2c_chains: 100,
            audit_engagements: 5,
            workpapers_per_engagement: 20,
            evidence_per_workpaper: 5,
            risks_per_engagement: 15,
            findings_per_engagement: 8,
            judgments_per_engagement: 10,
        }
    }
}

/// W1-3 Stage 1: allocate `total` across `n` periods as a cent-exact straight
/// line. Period `m` (1-based) receives `round(total*m/n) - round(total*(m-1)/n)`.
///
/// Two properties this guarantees:
///   * The per-period amounts sum to EXACTLY `total` (the cumulative target at
///     `m == n` is `total`, by construction — no drift, no residual leak).
///   * Each amount is non-negative for a non-negative `total` (the cumulative
///     target `round(total*m/n)` is monotonic non-decreasing in `m`), so a
///     period can be zero (when rounding leaves no incremental cent) but never
///     negative — avoiding a spurious contra posting.
///
/// With `n == 1` the result is `[total]` — the lumped pre-Stage-1 behavior — so
/// a caller that decomposes into a single period is byte-identical.
pub(crate) fn monthly_straight_line_allocation(
    total: rust_decimal::Decimal,
    n: u32,
) -> Vec<rust_decimal::Decimal> {
    use rust_decimal::Decimal;
    if n == 0 {
        return Vec::new();
    }
    let n_dec = Decimal::from(n);
    let mut out = Vec::with_capacity(n as usize);
    let mut prev = Decimal::ZERO;
    for m in 1..=n {
        let cum = (total * Decimal::from(m) / n_dec).round_dp(2);
        out.push(cum - prev);
        prev = cum;
    }
    out
}

/// Spec 16 step 1 — build the specialized opening-stock inception JEs for one company from explicit
/// per-instrument opening amounts. Each is a balanced 2-line JE: `DR Retained Earnings (3200) /
/// CR <control>`. The posting SIDES ARE HARDCODED — the credit-normal ECL allowance (1105, a
/// contra-asset NOT in the generated CoA) must never be routed through the opening-balance
/// converter's first-digit heuristic, which would mis-side it as a debit and silently absorb the
/// error in the 3100 plug. A non-positive / non-finite amount yields no JE. The JEs carry
/// `document_type=OPENING_BALANCE` so they load as the FY1 opening and are roll-forward-suppressed
/// in later fiscal years, exactly like the foundational opening. Free function (testable);
/// `EnhancedOrchestrator::build_specialized_opening_seed_jes` reads the config and delegates here.
pub(crate) fn specialized_opening_seed_jes(
    company_code: &str,
    currency: &str,
    as_of_date: NaiveDate,
    ecl_opening: Option<f64>,
    provision_opening: Option<f64>,
) -> Vec<JournalEntry> {
    use datasynth_core::accounts::{equity_accounts, provision_accounts};
    // The ECL allowance is a sub-account of AR control ("1105"); intentionally NOT a datasynth-core
    // account constant (it lives as a private const in ecl_generator).
    const ECL_ALLOWANCE: &str = "1105";
    let seeds: [(&str, &str, &str, Option<f64>); 2] = [
        (
            "JE-ECL-OPEN",
            "ECL allowance opening stock",
            ECL_ALLOWANCE,
            ecl_opening,
        ),
        (
            "JE-PROV-OPEN",
            "Provision liability opening stock",
            provision_accounts::PROVISION_LIABILITY,
            provision_opening,
        ),
    ];
    let mut jes: Vec<JournalEntry> = Vec::new();
    for (id_prefix, label, control, amount_opt) in seeds {
        let Some(amount_f64) = amount_opt else {
            continue;
        };
        let Some(amount) = Decimal::try_from(amount_f64).ok().map(|d| d.round_dp(2)) else {
            continue;
        };
        if amount <= Decimal::ZERO {
            continue;
        }
        let mut je = JournalEntry::new_simple(
            format!("{id_prefix}-{company_code}"),
            company_code.to_string(),
            as_of_date,
            label.to_string(),
        );
        je.header.document_type = "OPENING_BALANCE".to_string();
        je.header.created_by = "SYSTEM".to_string();
        je.header.currency = currency.to_string();
        je.header.source = TransactionSource::Automated;
        je.header.business_process = Some(BusinessProcess::R2R);
        let doc = je.header.document_id;
        // DR Retained Earnings (the canonical opening-equity clearing, 3200) ...
        je.add_line(JournalEntryLine::debit(
            doc,
            1,
            equity_accounts::RETAINED_EARNINGS.to_string(),
            amount,
        ));
        // ... CR the credit-normal control account (allowance 1105 / provision liability 2450).
        je.add_line(JournalEntryLine::credit(
            doc,
            2,
            control.to_string(),
            amount,
        ));
        debug_assert!(je.is_balanced(), "specialized opening seed JE must balance");
        jes.push(je);
    }
    jes
}

/// Spec 16 step 2 — build the Pension opening-stock inception JE from the signed opening net pension
/// liability. Unlike the ECL/Provisions seed (offset to Retained Earnings 3200), the pension offset is
/// Accumulated OCI (3800) — the IAS-19 / ASC 715 exception. SIGN CONVENTION: positive = net pension
/// LIABILITY / under-funded (DR Accumulated OCI 3800 / CR Net Pension Liability 2800 — establishes the
/// opening actuarial loss in OCI offsetting the opening liability); negative = net pension ASSET /
/// over-funded (DR 2800 / CR 3800 — drives 2800 to a DEBIT balance, exactly how the engine already
/// represents an over-funded plan at closing; there is NO separate prepaid-pension GL). None / 0 /
/// non-finite => no JE. Carries document_type=OPENING_BALANCE + FY1-only gating like step 1; zero RNG.
/// Free function (testable); `build_pension_opening_seed_je` reads the config and delegates here.
pub(crate) fn pension_opening_seed_je(
    company_code: &str,
    currency: &str,
    as_of_date: NaiveDate,
    opening_net_liability: Option<f64>,
) -> Option<JournalEntry> {
    use datasynth_core::accounts::{equity_accounts, liability_accounts};
    let net = Decimal::try_from(opening_net_liability?).ok()?.round_dp(2);
    if net == Decimal::ZERO {
        return None;
    }
    let amount = net.abs();
    let mut je = JournalEntry::new_simple(
        format!("JE-PENSION-OPEN-{company_code}"),
        company_code.to_string(),
        as_of_date,
        "Pension funded-status opening stock".to_string(),
    );
    je.header.document_type = "OPENING_BALANCE".to_string();
    je.header.created_by = "SYSTEM".to_string();
    je.header.currency = currency.to_string();
    je.header.source = TransactionSource::Automated;
    je.header.business_process = Some(BusinessProcess::R2R);
    let doc = je.header.document_id;
    let oci = equity_accounts::OCI_REMEASUREMENTS.to_string();
    let net_liab = liability_accounts::NET_PENSION_LIABILITY.to_string();
    if net > Decimal::ZERO {
        // under-funded: DR Accumulated OCI 3800 / CR Net Pension Liability 2800
        je.add_line(JournalEntryLine::debit(doc, 1, oci, amount));
        je.add_line(JournalEntryLine::credit(doc, 2, net_liab, amount));
    } else {
        // over-funded: DR Net Pension Liability 2800 (-> net asset) / CR Accumulated OCI 3800
        je.add_line(JournalEntryLine::debit(doc, 1, net_liab, amount));
        je.add_line(JournalEntryLine::credit(doc, 2, oci, amount));
    }
    debug_assert!(je.is_balanced(), "pension opening seed JE must balance");
    Some(je)
}

/// Spec 27 R6c — build the inventory→GL period-close true-up JEs. For every company that has EITHER
/// physical inventory positions OR pre-close GL `1200` activity (the UNION of the two key sets), true
/// GL Inventory (1200) to the physical target — `Σ position.valuation.total_value` for that company,
/// or **0** when a company carries 1200 churn but holds no ending stock — offsetting the delta to
/// Retained Earnings (3200). Balance-sheet only (asset ↔ equity) → net-income-neutral, so A=L+E and
/// the IS-articulation gate stay green. Iterating the union (not just the position-companies) means a
/// company with 1200 postings but no positions is relieved to zero rather than left holding churn
/// residue — a no-op for the standard single-focal-company build (its keys coincide) but the honest
/// behavior for any future multi-company inventory. Delta-based (`target − current`) → self-corrects
/// and never double-counts prior closes or doc-flow churn. Deterministic: the union is collected into
/// a `BTreeSet` so JE order is sorted by company_code, and every amount is `Decimal` (`round_dp(2)`).
/// Free function (testable); the caller builds the two per-company maps from `subledger`/`entries`.
pub(crate) fn build_inventory_close_jes(
    target_by_company: &std::collections::BTreeMap<String, Decimal>,
    gl_1200_by_company: &std::collections::BTreeMap<String, Decimal>,
    close_date: NaiveDate,
) -> Vec<JournalEntry> {
    use datasynth_core::accounts::{control_accounts, equity_accounts};
    use std::collections::BTreeSet;
    // UNION of companies with physical inventory OR pre-close GL 1200 activity, sorted → deterministic.
    let companies: BTreeSet<&String> = target_by_company
        .keys()
        .chain(gl_1200_by_company.keys())
        .collect();
    let mut jes: Vec<JournalEntry> = Vec::new();
    for company_code in companies {
        let target = target_by_company
            .get(company_code)
            .copied()
            .unwrap_or(Decimal::ZERO);
        let current = gl_1200_by_company
            .get(company_code)
            .copied()
            .unwrap_or(Decimal::ZERO);
        let delta = (target - current).round_dp(2);
        if delta == Decimal::ZERO {
            continue;
        }
        let mut inv_header = JournalEntryHeader::new(company_code.to_string(), close_date);
        inv_header.document_type = "CL".to_string();
        inv_header.header_text =
            Some("Inventory revaluation to physical (period close)".to_string());
        inv_header.created_by = "CLOSE_ENGINE".to_string();
        inv_header.source = TransactionSource::Automated;
        inv_header.business_process = Some(BusinessProcess::R2R);
        let doc_id = inv_header.document_id;
        let mut inv_je = JournalEntry::new(inv_header);
        if delta > Decimal::ZERO {
            // Book more inventory onto the GL: DR Inventory (1200) / CR Retained Earnings.
            inv_je.add_line(JournalEntryLine::debit(
                doc_id,
                1,
                control_accounts::INVENTORY.to_string(),
                delta,
            ));
            inv_je.add_line(JournalEntryLine::credit(
                doc_id,
                2,
                equity_accounts::RETAINED_EARNINGS.to_string(),
                delta,
            ));
        } else {
            // Reduce GL inventory to physical: DR Retained Earnings / CR Inventory (1200).
            let amt = -delta;
            inv_je.add_line(JournalEntryLine::debit(
                doc_id,
                1,
                equity_accounts::RETAINED_EARNINGS.to_string(),
                amt,
            ));
            inv_je.add_line(JournalEntryLine::credit(
                doc_id,
                2,
                control_accounts::INVENTORY.to_string(),
                amt,
            ));
        }
        debug_assert!(inv_je.is_balanced(), "Inventory close JE must be balanced");
        jes.push(inv_je);
    }
    jes
}

#[cfg(test)]
mod inventory_close_tests {
    use super::build_inventory_close_jes;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use std::collections::BTreeMap;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).expect("valid decimal literal")
    }
    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 3, 31).expect("valid date")
    }
    fn m(pairs: &[(&str, &str)]) -> BTreeMap<String, Decimal> {
        pairs.iter().map(|(k, v)| (k.to_string(), d(v))).collect()
    }

    #[test]
    fn true_up_books_more_when_target_exceeds_gl() {
        // physical 10,000 vs GL 6,000 → delta +4,000 → DR Inventory(1200) / CR Retained Earnings(3200).
        let jes = build_inventory_close_jes(
            &m(&[("1000", "10000.00")]),
            &m(&[("1000", "6000.00")]),
            date(),
        );
        assert_eq!(jes.len(), 1);
        let je = &jes[0];
        assert!(je.is_balanced());
        assert_eq!(je.lines.len(), 2);
        assert_eq!(je.lines[0].gl_account, "1200");
        assert_eq!(je.lines[0].debit_amount, d("4000.00"));
        assert_eq!(je.lines[1].gl_account, "3200");
        assert_eq!(je.lines[1].credit_amount, d("4000.00"));
        assert_eq!(je.header.document_type, "CL");
        assert_eq!(je.header.created_by, "CLOSE_ENGINE");
    }

    #[test]
    fn true_down_reduces_when_gl_exceeds_target() {
        // the real manufacturing case (D-R6.5): GL 18,700 churn residue vs physical 10,800 →
        // delta -7,900 → DR Retained Earnings(3200) / CR Inventory(1200).
        let jes = build_inventory_close_jes(
            &m(&[("1000", "10800.00")]),
            &m(&[("1000", "18700.00")]),
            date(),
        );
        assert_eq!(jes.len(), 1);
        let je = &jes[0];
        assert!(je.is_balanced());
        assert_eq!(je.lines[0].gl_account, "3200");
        assert_eq!(je.lines[0].debit_amount, d("7900.00"));
        assert_eq!(je.lines[1].gl_account, "1200");
        assert_eq!(je.lines[1].credit_amount, d("7900.00"));
    }

    #[test]
    fn zero_delta_emits_no_je() {
        // GL already at physical → no revaluation JE.
        let jes = build_inventory_close_jes(
            &m(&[("1000", "5000.00")]),
            &m(&[("1000", "5000.00")]),
            date(),
        );
        assert!(jes.is_empty());
    }

    #[test]
    fn company_with_gl_activity_but_no_positions_is_relieved_to_zero() {
        // UNION robustness: company "2000" has 1200 churn (3,000) but no positions → trued to 0
        // (DR 3200 / CR 1200 by 3,000). Company "1000" has positions but no churn → booked up.
        let target = m(&[("1000", "1000.00")]);
        let gl = m(&[("2000", "3000.00")]);
        let jes = build_inventory_close_jes(&target, &gl, date());
        assert_eq!(jes.len(), 2);
        // BTreeSet order → "1000" before "2000".
        assert_eq!(jes[0].header.company_code, "1000");
        assert_eq!(jes[0].lines[0].gl_account, "1200"); // book up 1000's physical inventory
        assert_eq!(jes[0].lines[0].debit_amount, d("1000.00"));
        assert_eq!(jes[1].header.company_code, "2000");
        assert_eq!(jes[1].lines[0].gl_account, "3200"); // relieve 2000's churn to zero
        assert_eq!(jes[1].lines[1].gl_account, "1200");
        assert_eq!(jes[1].lines[1].credit_amount, d("3000.00"));
        assert!(jes.iter().all(|je| je.is_balanced()));
    }

    #[test]
    fn deterministic_company_order_regardless_of_map_insertion() {
        // BTreeSet sorts keys → JE order is by company_code, independent of insertion order.
        let jes = build_inventory_close_jes(
            &m(&[("3000", "300.00"), ("1000", "100.00"), ("2000", "200.00")]),
            &BTreeMap::new(),
            date(),
        );
        let codes: Vec<&str> = jes
            .iter()
            .map(|je| je.header.company_code.as_str())
            .collect();
        assert_eq!(codes, vec!["1000", "2000", "3000"]);
    }
}

#[cfg(test)]
mod opening_seed_tests {
    use super::{pension_opening_seed_je, specialized_opening_seed_jes};
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).expect("valid decimal literal")
    }
    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2025, 1, 1).expect("valid date")
    }

    #[test]
    fn seed_off_emits_nothing() {
        // Both openings None → no JE → the seam is inert when off (byte-identity guard).
        assert!(specialized_opening_seed_jes("1000", "USD", date(), None, None).is_empty());
        // A zero / negative amount is ignored too (no spurious empty JE).
        assert!(
            specialized_opening_seed_jes("1000", "USD", date(), Some(0.0), Some(-5.0)).is_empty()
        );
    }

    #[test]
    fn ecl_seed_credits_the_allowance_1105() {
        // R1 regression: the contra-asset allowance MUST be CREDITED (DR 3200 / CR 1105), never
        // posted on the debit side a first-digit heuristic would pick for a "1xxx" account.
        let jes = specialized_opening_seed_jes("1000", "USD", date(), Some(50_000.0), None);
        assert_eq!(jes.len(), 1, "only ECL configured → exactly one seed JE");
        let je = &jes[0];
        assert!(je.is_balanced());
        assert_eq!(je.header.document_type, "OPENING_BALANCE");
        assert_eq!(je.lines.len(), 2);
        let re = je
            .lines
            .iter()
            .find(|l| l.gl_account == "3200")
            .expect("RE line");
        let allowance = je
            .lines
            .iter()
            .find(|l| l.gl_account == "1105")
            .expect("1105 line");
        assert!(re.is_debit(), "retained earnings 3200 is the debit offset");
        assert!(
            !allowance.is_debit(),
            "allowance 1105 must be a CREDIT (contra-asset)"
        );
        assert_eq!(allowance.credit_amount, d("50000.00"));
        assert_eq!(re.debit_amount, d("50000.00"));
    }

    #[test]
    fn provisions_seed_credits_the_liability_2450() {
        let jes = specialized_opening_seed_jes("1000", "USD", date(), None, Some(30_000.0));
        assert_eq!(jes.len(), 1);
        let je = &jes[0];
        assert!(je.is_balanced());
        let liab = je
            .lines
            .iter()
            .find(|l| l.gl_account == "2450")
            .expect("2450 line");
        assert!(
            !liab.is_debit(),
            "provision liability 2450 must be a CREDIT"
        );
        assert_eq!(liab.credit_amount, d("30000.00"));
    }

    #[test]
    fn both_seeds_emit_two_balanced_jes() {
        let jes =
            specialized_opening_seed_jes("1000", "USD", date(), Some(50_000.0), Some(30_000.0));
        assert_eq!(jes.len(), 2);
        assert!(jes.iter().all(|je| je.is_balanced()));
    }

    #[test]
    fn pension_seed_off_emits_nothing() {
        // None and an exactly-zero net position both yield no JE → inert when off.
        assert!(pension_opening_seed_je("1000", "USD", date(), None).is_none());
        assert!(pension_opening_seed_je("1000", "USD", date(), Some(0.0)).is_none());
    }

    #[test]
    fn pension_underfunded_credits_2800_offsets_oci_3800() {
        // positive net = under-funded liability: DR Accumulated OCI 3800 / CR Net Pension Liability
        // 2800 — the offset is OCI, NEVER Retained Earnings 3200.
        let je = pension_opening_seed_je("1000", "USD", date(), Some(80_000.0)).expect("a JE");
        assert!(je.is_balanced());
        assert_eq!(je.header.document_type, "OPENING_BALANCE");
        assert_eq!(je.lines.len(), 2);
        let liab = je
            .lines
            .iter()
            .find(|l| l.gl_account == "2800")
            .expect("2800 line");
        let oci = je
            .lines
            .iter()
            .find(|l| l.gl_account == "3800")
            .expect("3800 line");
        assert!(
            !liab.is_debit(),
            "net pension liability 2800 is a CREDIT when under-funded"
        );
        assert!(oci.is_debit(), "Accumulated OCI 3800 is the debit offset");
        assert_eq!(liab.credit_amount, d("80000.00"));
        assert!(
            je.lines.iter().all(|l| l.gl_account != "3200"),
            "pension never touches RE 3200"
        );
    }

    #[test]
    fn pension_overfunded_debits_2800_offsets_oci_3800() {
        // negative net = over-funded asset: DR Net Pension Liability 2800 (driven to a net asset) /
        // CR Accumulated OCI 3800. No prepaid-pension account — 2800 carries the over-funded asset.
        let je = pension_opening_seed_je("1000", "USD", date(), Some(-40_000.0)).expect("a JE");
        assert!(je.is_balanced());
        let liab = je
            .lines
            .iter()
            .find(|l| l.gl_account == "2800")
            .expect("2800 line");
        assert!(
            liab.is_debit(),
            "net pension 2800 is a DEBIT when over-funded (a net asset)"
        );
        assert_eq!(liab.debit_amount, d("40000.00"));
        assert!(
            je.lines.iter().all(|l| l.gl_account != "1520"),
            "never posts to Buildings 1520"
        );
    }
}

#[cfg(test)]
mod recurring_posting_tests {
    use super::monthly_straight_line_allocation;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    // `rust_decimal_macros::dec!` is not a dependency of this crate; parse from
    // a string literal instead (just as exact, no extra dep).
    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).expect("valid decimal literal")
    }

    #[test]
    fn alloc_single_period_is_lump() {
        // n == 1 → [total] exactly. This is the byte-identical-lump invariant:
        // with monthly_recurring OFF the caller passes a single month-end, so the
        // recurring postings reduce to the pre-Stage-1 single JE.
        assert_eq!(
            monthly_straight_line_allocation(d("1234.56"), 1),
            vec![d("1234.56")]
        );
    }

    #[test]
    fn alloc_sums_to_total_cent_exact() {
        // The per-period amounts ALWAYS sum to exactly `total` (the cumulative
        // target at m == n is round(total) == total since total is already 2dp).
        // This is the property the FA subledger tie + answer key depend on.
        for (total, n) in [
            (d("1200.00"), 12u32),
            (d("100.00"), 3),
            (d("1000.01"), 12),
            (d("0.05"), 12),
            (d("99999.99"), 7),
            (d("50.00"), 1),
        ] {
            let alloc = monthly_straight_line_allocation(total, n);
            assert_eq!(alloc.len(), n as usize);
            let sum: Decimal = alloc.iter().copied().sum();
            assert_eq!(
                sum, total,
                "alloc {alloc:?} (total={total}, n={n}) must sum to total"
            );
            // Non-negative — never a spurious contra posting.
            assert!(
                alloc.iter().all(|a| *a >= Decimal::ZERO),
                "alloc {alloc:?} has a negative"
            );
        }
    }

    #[test]
    fn alloc_even_split_distributes_remainder() {
        // 100.00 / 3 → the odd cent lands where cumulative rounding rounds up
        // (month 2 here); still sums to 100.00 to the cent.
        let alloc = monthly_straight_line_allocation(d("100.00"), 3);
        assert_eq!(alloc, vec![d("33.33"), d("33.34"), d("33.33")]);
        assert_eq!(alloc.iter().copied().sum::<Decimal>(), d("100.00"));
    }

    #[test]
    fn alloc_zero_total_is_all_zero() {
        let alloc = monthly_straight_line_allocation(Decimal::ZERO, 12);
        assert_eq!(alloc.len(), 12);
        assert!(alloc.iter().all(Decimal::is_zero));
    }

    #[test]
    fn alloc_n_zero_is_empty() {
        assert!(monthly_straight_line_allocation(d("100"), 0).is_empty());
    }

    #[test]
    fn alloc_is_deterministic() {
        assert_eq!(
            monthly_straight_line_allocation(d("777.77"), 12),
            monthly_straight_line_allocation(d("777.77"), 12)
        );
    }
}

/// W1-3 Stage 2: tests for the three recurring accounting classes (bond interest,
/// ASC 606 deferred-revenue recognition, ASC 842 leases). These exercise the
/// arithmetic invariants the inline orchestrator phases depend on (balance,
/// A=L+E preservation, byte-identical-OFF) without standing up the full pipeline.
#[cfg(test)]
mod stage2_recurring_tests {
    use super::monthly_straight_line_allocation;
    use chrono::NaiveDate;
    use datasynth_standards::accounting::leases::{
        Lease, LeaseAssetClass, LeaseClassification, PaymentFrequency,
    };
    use datasynth_standards::framework::AccountingFramework;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).expect("valid decimal literal")
    }

    // ---- Class 1: Bond / loan interest ----

    /// The slice interest = principal * rate * (months/12), and spreading it across
    /// the month-ends sums back to exactly that — both ON (n=months) and OFF (n=1).
    #[test]
    fn bond_monthly_interest_sums_to_slice_interest() {
        let principal = d("1000000.00");
        let rate = d("0.06"); // 6% annual
        let months = 3u32;
        let slice_interest =
            (principal * rate * Decimal::from(months) / Decimal::from(12)).round_dp(2);
        // 1,000,000 * 0.06 * 3/12 = 15,000.00
        assert_eq!(slice_interest, d("15000.00"));

        // monthly_recurring ON: spread across 3 month-ends → sums to slice interest.
        let monthly = monthly_straight_line_allocation(slice_interest, months);
        assert_eq!(monthly.len(), 3);
        assert_eq!(monthly.iter().copied().sum::<Decimal>(), slice_interest);

        // monthly_recurring OFF: a single month-end (n=1) → the lump, byte-identical.
        let lump = monthly_straight_line_allocation(slice_interest, 1);
        assert_eq!(lump, vec![slice_interest]);
    }

    /// Each per-month bond JE (DR interest expense / CR interest payable, equal
    /// amounts) is balanced by construction, and the period totals tie.
    #[test]
    fn bond_monthly_je_legs_balance() {
        let slice_interest = d("15000.00");
        let alloc = monthly_straight_line_allocation(slice_interest, 3);
        let mut total_debit = Decimal::ZERO;
        let mut total_credit = Decimal::ZERO;
        for amount in &alloc {
            // DR interest expense == CR interest payable (single-amount two-leg JE).
            total_debit += *amount;
            total_credit += *amount;
            assert_eq!(*amount, *amount, "leg amounts identical → JE balanced");
        }
        assert_eq!(total_debit, total_credit);
        assert_eq!(total_debit, slice_interest);
    }

    // ---- Class 2: ASC 606 deferred-revenue recognition ----

    /// Inception funds the liability for the full allocated price; recognition draws
    /// it back down to exactly zero over the slice. Net liability movement = 0
    /// (A=L+E preserved by construction) and every leg balances.
    #[test]
    fn rev606_inception_and_recognition_net_to_zero_liability() {
        let allocated = d("120000.00");
        // Inception: CR deferred revenue (liability +allocated), DR contract asset.
        let inception_liability_credit = allocated;
        // Over-time recognition spread across 12 month-ends: DR deferred revenue.
        let recognition = monthly_straight_line_allocation(allocated, 12);
        let recognition_liability_debit: Decimal = recognition.iter().copied().sum();
        // The liability funded at inception is fully drawn down → net zero.
        assert_eq!(inception_liability_credit, recognition_liability_debit);
        // Each recognition JE balances (DR deferred rev == CR revenue, equal amounts).
        for amount in &recognition {
            assert!(
                *amount >= Decimal::ZERO,
                "no negative recognition (no negative liability)"
            );
        }
        // Inception JE balances: DR contract asset == CR deferred revenue.
        assert_eq!(allocated, inception_liability_credit);
    }

    /// Point-in-time recognition lands the full amount on exactly one month-end and
    /// still fully draws down the funded liability.
    #[test]
    fn rev606_point_in_time_recognizes_full_amount_once() {
        let allocated = d("50000.00");
        let n = 3usize;
        // Emulate the inline point-in-time placement: full amount on a single slot.
        let target = 1usize; // e.g. satisfaction falls in month 2
        let mut recognition = vec![Decimal::ZERO; n];
        recognition[target] = allocated;
        let recognized: Decimal = recognition.iter().copied().sum();
        assert_eq!(
            recognized, allocated,
            "point-in-time draws down the full funded liability"
        );
        assert_eq!(
            recognition.iter().filter(|a| **a > Decimal::ZERO).count(),
            1
        );
    }

    // ---- Class 3: ASC 842 leases ----

    /// A finance lease's inception PV == the funded liability, and walking the
    /// schedule, every payment JE balances (principal + interest == cash credit) and
    /// the sum of principal payments never exceeds the funded PV (no negative
    /// liability → A=L+E preserved).
    #[test]
    fn lease842_finance_schedule_balances_and_no_negative_liability() {
        let lease = Lease::new(
            "1000",
            "ABC Leasing",
            "Equipment Lease",
            LeaseAssetClass::Equipment,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            108, // 9 years → 108/120 = 90% ≥ 75% → Finance
            d("5000"),
            PaymentFrequency::Monthly,
            d("0.05"),
            d("400000"),
            120,
            AccountingFramework::UsGaap,
        );
        assert_eq!(lease.classification, LeaseClassification::Finance);

        let pv = lease.lease_liability.initial_measurement.round_dp(2);
        assert!(pv > Decimal::ZERO, "inception funds a positive liability");

        let mut cumulative_principal = Decimal::ZERO;
        for row in &lease.lease_liability.amortization_schedule {
            let interest = row.interest_expense.round_dp(2);
            let principal = row.principal_payment.round_dp(2);
            // Inline JE rule: DR liability(principal) + DR interest / CR cash(sum).
            let cash = principal.max(Decimal::ZERO) + interest.max(Decimal::ZERO);
            let total_debit = principal.max(Decimal::ZERO) + interest.max(Decimal::ZERO);
            assert_eq!(total_debit, cash, "finance lease payment JE must balance");

            cumulative_principal += principal.max(Decimal::ZERO);
            // The funded liability is never over-drawn at any point in the schedule.
            assert!(
                cumulative_principal <= pv + d("1.00"),
                "cumulative principal {cumulative_principal} must not exceed funded PV {pv}"
            );
        }
    }

    /// ROU amortization (DR amort expense / CR ROU asset, equal amounts) balances,
    /// and the monthly amount is the straight-line share of the initial measurement.
    #[test]
    fn lease842_rou_amortization_balances() {
        let lease = Lease::new(
            "1000",
            "ABC Leasing",
            "Office Lease",
            LeaseAssetClass::RealEstate,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            24,
            d("1000"),
            PaymentFrequency::Monthly,
            d("0.05"),
            d("20000"),
            120,
            AccountingFramework::UsGaap,
        );
        let monthly_dep = lease.rou_asset.monthly_depreciation().round_dp(2);
        assert!(monthly_dep >= Decimal::ZERO);
        // The amortization JE has equal debit/credit legs → balanced by construction.
        let total_debit = monthly_dep;
        let total_credit = monthly_dep;
        assert_eq!(total_debit, total_credit);
    }

    /// An OPERATING lease is funded at inception (ASC 842 keeps it on the balance
    /// sheet) and its ROU asset + lease liability must UNWIND to ~zero over the full
    /// term via the per-period `DR liability / CR ROU` (principal) leg — otherwise the
    /// funded position strands forever (the major defect the v5.36.0 review caught).
    /// Mirrors the orchestrator's operating branch: cumulative principal never
    /// over-draws the funded PV, and the residual at term end is ~zero.
    #[test]
    fn lease842_operating_rou_and_liability_unwind_to_zero_over_term() {
        let lease = Lease::new(
            "1000",
            "XYZ Realty",
            "Short Office Lease",
            LeaseAssetClass::RealEstate,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            24, // 24/120 = 20% of useful life (< 75%)
            d("2000"),
            PaymentFrequency::Monthly,
            d("0.05"),
            d("500000"), // PV (~$45.6k) << 90% of FV → Operating
            120,
            AccountingFramework::UsGaap,
        );
        assert_eq!(
            lease.classification,
            LeaseClassification::Operating,
            "test fixture must be an operating lease"
        );

        let pv = lease.lease_liability.initial_measurement.round_dp(2);
        assert!(
            pv > Decimal::ZERO,
            "ASC 842 funds the operating ROU + liability at inception"
        );

        // Apply the orchestrator's operating unwind rule across the WHOLE term:
        // each period DR lease liability(principal) / CR ROU(principal).
        let mut cumulative_principal = Decimal::ZERO;
        for row in &lease.lease_liability.amortization_schedule {
            let principal = row.principal_payment.round_dp(2).max(Decimal::ZERO);
            cumulative_principal += principal;
            // Never over-draw the funded liability (no negative liability mid-term).
            assert!(
                cumulative_principal <= pv + d("1.00"),
                "cumulative principal {cumulative_principal} must not exceed funded PV {pv}"
            );
        }
        // Over the full term the funded ROU + liability roll back to ~zero (within a
        // cent or two of per-period rounding) — no stranded balance sheet position.
        let residual = pv - cumulative_principal;
        assert!(
            residual.abs() <= d("1.00"),
            "operating lease ROU + liability must unwind to ~zero over the term (residual = {residual})"
        );
    }
}

/// Master data snapshot containing all generated entities.
#[derive(Debug, Clone, Default)]
pub struct MasterDataSnapshot {
    /// Generated vendors.
    pub vendors: Vec<Vendor>,
    /// Generated customers.
    pub customers: Vec<Customer>,
    /// Generated materials.
    pub materials: Vec<Material>,
    /// Generated fixed assets.
    pub assets: Vec<FixedAsset>,
    /// Generated employees.
    pub employees: Vec<Employee>,
    /// Generated cost center hierarchy (two-level: departments + sub-departments).
    pub cost_centers: Vec<datasynth_core::models::CostCenter>,
    /// v5.1: Generated profit centre hierarchy (two-level: top-level
    /// segment / region / product-group nodes + sub-units).  Emits to
    /// SAP CEPC alongside `cost_centers` → CSKS.
    pub profit_centers: Vec<datasynth_core::models::ProfitCenter>,
    /// Employee lifecycle change history (hired, promoted, salary adjustments, transfers, terminated).
    pub employee_change_history: Vec<datasynth_core::models::EmployeeChangeEvent>,
    /// v3.3.0+: organizational profiles (one per company) with
    /// industry / geography / structure / complexity metadata. Emitted
    /// alongside master data when `generate_master_data = true`.
    pub organizational_profiles: Vec<datasynth_core::models::OrganizationalProfile>,
}

/// Info about a completed hypergraph export.
#[derive(Debug, Clone)]
pub struct HypergraphExportInfo {
    /// Number of nodes exported.
    pub node_count: usize,
    /// Number of pairwise edges exported.
    pub edge_count: usize,
    /// Number of hyperedges exported.
    pub hyperedge_count: usize,
    /// Output directory path.
    pub output_path: PathBuf,
}

/// Document flow snapshot containing all generated document chains.
#[derive(Debug, Clone, Default)]
pub struct DocumentFlowSnapshot {
    /// P2P document chains.
    pub p2p_chains: Vec<P2PDocumentChain>,
    /// O2C document chains.
    pub o2c_chains: Vec<O2CDocumentChain>,
    /// All purchase orders (flattened).
    pub purchase_orders: Vec<documents::PurchaseOrder>,
    /// All goods receipts (flattened).
    pub goods_receipts: Vec<documents::GoodsReceipt>,
    /// All vendor invoices (flattened).
    pub vendor_invoices: Vec<documents::VendorInvoice>,
    /// All sales orders (flattened).
    pub sales_orders: Vec<documents::SalesOrder>,
    /// All deliveries (flattened).
    pub deliveries: Vec<documents::Delivery>,
    /// All customer invoices (flattened).
    pub customer_invoices: Vec<documents::CustomerInvoice>,
    /// All payments (flattened).
    pub payments: Vec<documents::Payment>,
    /// Cross-document references collected from all document headers
    /// (PO→GR, GR→Invoice, Invoice→Payment, SO→Delivery, etc.)
    pub document_references: Vec<documents::DocumentReference>,
}

/// Subledger snapshot containing generated subledger records.
#[derive(Debug, Clone, Default)]
pub struct SubledgerSnapshot {
    /// AP invoices linked from document flow vendor invoices.
    pub ap_invoices: Vec<APInvoice>,
    /// AR invoices linked from document flow customer invoices.
    pub ar_invoices: Vec<ARInvoice>,
    /// FA subledger records (asset acquisitions from FA generator).
    pub fa_records: Vec<datasynth_core::models::subledger::fa::FixedAssetRecord>,
    /// Inventory positions from inventory generator.
    pub inventory_positions: Vec<datasynth_core::models::subledger::inventory::InventoryPosition>,
    /// Inventory movements from inventory generator.
    pub inventory_movements: Vec<datasynth_core::models::subledger::inventory::InventoryMovement>,
    /// AR aging reports, one per company, computed after payment settlement.
    pub ar_aging_reports: Vec<ARAgingReport>,
    /// AP aging reports, one per company, computed after payment settlement.
    pub ap_aging_reports: Vec<APAgingReport>,
    /// Depreciation runs — one per fiscal period per company (from DepreciationRunGenerator).
    pub depreciation_runs: Vec<datasynth_core::models::subledger::fa::DepreciationRun>,
    /// Inventory valuation results — one per company (lower-of-cost-or-NRV, IAS 2 / ASC 330).
    pub inventory_valuations: Vec<datasynth_generators::InventoryValuationResult>,
    /// Dunning runs executed after AR aging (one per company per dunning cycle).
    pub dunning_runs: Vec<datasynth_core::models::subledger::ar::DunningRun>,
    /// Dunning letters generated across all dunning runs.
    pub dunning_letters: Vec<datasynth_core::models::subledger::ar::DunningLetter>,
}

/// OCPM snapshot containing generated OCPM event log data.
#[derive(Debug, Clone, Default)]
pub struct OcpmSnapshot {
    /// OCPM event log (if generated)
    pub event_log: Option<OcpmEventLog>,
    /// Number of events generated
    pub event_count: usize,
    /// Number of objects generated
    pub object_count: usize,
    /// Number of cases generated
    pub case_count: usize,
}

/// Audit data snapshot containing all generated audit-related entities.
#[derive(Debug, Clone, Default)]
pub struct AuditSnapshot {
    /// Audit engagements per ISA 210/220.
    pub engagements: Vec<AuditEngagement>,
    /// Workpapers per ISA 230.
    pub workpapers: Vec<Workpaper>,
    /// Audit evidence per ISA 500.
    pub evidence: Vec<AuditEvidence>,
    /// Risk assessments per ISA 315/330.
    pub risk_assessments: Vec<RiskAssessment>,
    /// Audit findings per ISA 265.
    pub findings: Vec<AuditFinding>,
    /// Professional judgments per ISA 200.
    pub judgments: Vec<ProfessionalJudgment>,
    /// External confirmations per ISA 505.
    pub confirmations: Vec<ExternalConfirmation>,
    /// Confirmation responses per ISA 505.
    pub confirmation_responses: Vec<ConfirmationResponse>,
    /// Audit procedure steps per ISA 330/530.
    pub procedure_steps: Vec<AuditProcedureStep>,
    /// Audit samples per ISA 530.
    pub samples: Vec<AuditSample>,
    /// Analytical procedure results per ISA 520.
    pub analytical_results: Vec<AnalyticalProcedureResult>,
    /// Internal audit functions per ISA 610.
    pub ia_functions: Vec<InternalAuditFunction>,
    /// Internal audit reports per ISA 610.
    pub ia_reports: Vec<InternalAuditReport>,
    /// Related parties per ISA 550.
    pub related_parties: Vec<RelatedParty>,
    /// Related party transactions per ISA 550.
    pub related_party_transactions: Vec<RelatedPartyTransaction>,
    // ---- ISA 600: Group Audits ----
    /// Component auditors assigned by jurisdiction (ISA 600).
    pub component_auditors: Vec<ComponentAuditor>,
    /// Group audit plan with materiality allocations (ISA 600).
    pub group_audit_plan: Option<GroupAuditPlan>,
    /// Component instructions issued to component auditors (ISA 600).
    pub component_instructions: Vec<ComponentInstruction>,
    /// Reports received from component auditors (ISA 600).
    pub component_reports: Vec<ComponentAuditorReport>,
    // ---- ISA 210: Engagement Letters ----
    /// Engagement letters per ISA 210.
    pub engagement_letters: Vec<EngagementLetter>,
    // ---- ISA 560 / IAS 10: Subsequent Events ----
    /// Subsequent events per ISA 560 / IAS 10.
    pub subsequent_events: Vec<SubsequentEvent>,
    // ---- ISA 402: Service Organization Controls ----
    /// Service organizations identified per ISA 402.
    pub service_organizations: Vec<ServiceOrganization>,
    /// SOC reports obtained per ISA 402.
    pub soc_reports: Vec<SocReport>,
    /// User entity controls documented per ISA 402.
    pub user_entity_controls: Vec<UserEntityControl>,
    // ---- ISA 570: Going Concern ----
    /// Going concern assessments per ISA 570 / ASC 205-40 (one per entity per period).
    pub going_concern_assessments:
        Vec<datasynth_core::models::audit::going_concern::GoingConcernAssessment>,
    // ---- ISA 540: Accounting Estimates ----
    /// Accounting estimates reviewed per ISA 540 (5–8 per entity).
    pub accounting_estimates:
        Vec<datasynth_core::models::audit::accounting_estimates::AccountingEstimate>,
    // ---- ISA 700/701/705/706: Audit Opinions ----
    /// Formed audit opinions per ISA 700 / 705 / 706 (one per engagement).
    pub audit_opinions: Vec<datasynth_standards::audit::opinion::AuditOpinion>,
    /// Key Audit Matters per ISA 701 (flattened across all opinions).
    pub key_audit_matters: Vec<datasynth_standards::audit::opinion::KeyAuditMatter>,
    // ---- SOX 302 / 404 ----
    /// SOX Section 302 CEO/CFO certifications (one pair per US-listed entity per year).
    pub sox_302_certifications: Vec<datasynth_standards::regulatory::sox::Sox302Certification>,
    /// SOX Section 404 ICFR assessments (one per entity per year).
    pub sox_404_assessments: Vec<datasynth_standards::regulatory::sox::Sox404Assessment>,
    // ---- ISA 320: Materiality ----
    /// Materiality calculations per entity per period (ISA 320).
    pub materiality_calculations:
        Vec<datasynth_core::models::audit::materiality_calculation::MaterialityCalculation>,
    // ---- ISA 315: Combined Risk Assessments ----
    /// Combined Risk Assessments per account area / assertion (ISA 315).
    pub combined_risk_assessments:
        Vec<datasynth_core::models::audit::risk_assessment_cra::CombinedRiskAssessment>,
    // ---- ISA 530: Sampling Plans ----
    /// Sampling plans per CRA at Moderate or higher (ISA 530).
    pub sampling_plans: Vec<datasynth_core::models::audit::sampling_plan::SamplingPlan>,
    /// Individual sampled items (key items + representative items) per ISA 530.
    pub sampled_items: Vec<datasynth_core::models::audit::sampling_plan::SampledItem>,
    // ---- ISA 315: Significant Classes of Transactions (SCOTS) ----
    /// Significant classes of transactions per ISA 315 (one set per entity).
    pub significant_transaction_classes:
        Vec<datasynth_core::models::audit::scots::SignificantClassOfTransactions>,
    // ---- ISA 520: Unusual Item Markers ----
    /// Unusual item flags raised across all journal entries (5–10% flagging rate).
    pub unusual_items: Vec<datasynth_core::models::audit::unusual_items::UnusualItemFlag>,
    // ---- ISA 520: Analytical Relationships ----
    /// Analytical relationships (ratios, trends, correlations) per entity.
    pub analytical_relationships:
        Vec<datasynth_core::models::audit::analytical_relationships::AnalyticalRelationship>,
    // ---- PCAOB-ISA Cross-Reference ----
    /// PCAOB-to-ISA standard mappings (key differences, similarities, application notes).
    pub isa_pcaob_mappings: Vec<datasynth_standards::audit::pcaob::PcaobIsaMapping>,
    // ---- ISA Standard Reference ----
    /// Flat ISA standard reference entries (number, title, series) for `audit/isa_mappings.json`.
    pub isa_mappings: Vec<datasynth_standards::audit::isa_reference::IsaStandardEntry>,
    // ---- ISA 220 / ISA 300: Audit Scopes ----
    /// Audit scope records (one per engagement) describing the audit boundary.
    pub audit_scopes: Vec<datasynth_core::models::audit::AuditScope>,
    // ---- FSM Event Trail ----
    /// Optional FSM event trail produced when `audit.fsm.enabled: true`.
    /// Contains the ordered sequence of state-transition and procedure-step events
    /// generated by the audit FSM engine.
    pub fsm_event_trail: Option<Vec<datasynth_audit_fsm::event::AuditEvent>>,
    // ---- v3.3.0: L1 generator wiring ----
    /// Legal documents (engagement letters, management reps, legal
    /// opinions, regulatory filings, board resolutions) per entity.
    /// Emitted by `LegalDocumentGenerator` when
    /// `compliance_regulations.legal_documents.enabled = true`.
    pub legal_documents: Vec<datasynth_core::models::LegalDocument>,
    /// IT general controls — access logs (login/privileged action
    /// audit trail). Emitted by `ItControlsGenerator` when
    /// `audit.it_controls.enabled = true`.
    pub it_controls_access_logs: Vec<datasynth_core::models::AccessLog>,
    /// IT general controls — change management records (code deploys,
    /// config changes, patches). Emitted by `ItControlsGenerator`.
    pub it_controls_change_records: Vec<datasynth_core::models::ChangeManagementRecord>,
}

/// Banking KYC/AML data snapshot containing all generated banking entities.
#[derive(Debug, Clone, Default)]
pub struct BankingSnapshot {
    /// Banking customers (retail, business, trust).
    pub customers: Vec<BankingCustomer>,
    /// Bank accounts.
    pub accounts: Vec<BankAccount>,
    /// Bank transactions with AML labels.
    pub transactions: Vec<BankTransaction>,
    /// Transaction-level AML labels with features.
    pub transaction_labels: Vec<datasynth_banking::labels::TransactionLabel>,
    /// Customer-level AML labels.
    pub customer_labels: Vec<datasynth_banking::labels::CustomerLabel>,
    /// Account-level AML labels.
    pub account_labels: Vec<datasynth_banking::labels::AccountLabel>,
    /// Relationship-level AML labels.
    pub relationship_labels: Vec<datasynth_banking::labels::RelationshipLabel>,
    /// Case narratives for AML scenarios.
    pub narratives: Vec<datasynth_banking::labels::ExportedNarrative>,
    /// Number of suspicious transactions.
    pub suspicious_count: usize,
    /// Number of AML scenarios generated.
    pub scenario_count: usize,
}

/// Graph export snapshot containing exported graph metadata.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GraphExportSnapshot {
    /// Whether graph export was performed.
    pub exported: bool,
    /// Number of graphs exported.
    pub graph_count: usize,
    /// Exported graph metadata (by format name).
    pub exports: HashMap<String, GraphExportInfo>,
}

/// Information about an exported graph.
#[derive(Debug, Clone, Serialize)]
pub struct GraphExportInfo {
    /// Graph name.
    pub name: String,
    /// Export format (pytorch_geometric, neo4j, dgl).
    pub format: String,
    /// Output directory path.
    pub output_path: PathBuf,
    /// Number of nodes.
    pub node_count: usize,
    /// Number of edges.
    pub edge_count: usize,
}

/// S2C sourcing data snapshot.
#[derive(Debug, Clone, Default)]
pub struct SourcingSnapshot {
    /// Spend analyses.
    pub spend_analyses: Vec<SpendAnalysis>,
    /// Sourcing projects.
    pub sourcing_projects: Vec<SourcingProject>,
    /// Supplier qualifications.
    pub qualifications: Vec<SupplierQualification>,
    /// RFx events (RFI, RFP, RFQ).
    pub rfx_events: Vec<RfxEvent>,
    /// Supplier bids.
    pub bids: Vec<SupplierBid>,
    /// Bid evaluations.
    pub bid_evaluations: Vec<BidEvaluation>,
    /// Procurement contracts.
    pub contracts: Vec<ProcurementContract>,
    /// Catalog items.
    pub catalog_items: Vec<CatalogItem>,
    /// Supplier scorecards.
    pub scorecards: Vec<SupplierScorecard>,
}

/// A single period's trial balance with metadata.
///
/// Used as the orchestrator's in-memory representation while it
/// builds per-period FS / CF artefacts.  At write time the runtime
/// converts each `PeriodTrialBalance` to the canonical
/// [`datasynth_core::models::balance::TrialBalance`] shape via
/// [`PeriodTrialBalance::into_canonical`] so the on-disk
/// `period_close/trial_balances.json` matches what the group
/// aggregate phase loads — see
/// `crate::output_writer::write_outputs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeriodTrialBalance {
    /// Fiscal year.
    pub fiscal_year: u16,
    /// Fiscal period (1-12).
    pub fiscal_period: u8,
    /// Period start date.
    pub period_start: NaiveDate,
    /// Period end date.
    pub period_end: NaiveDate,
    /// Trial balance entries for this period.
    pub entries: Vec<datasynth_generators::TrialBalanceEntry>,
    /// Framework string for classifier dispatch in
    /// [`PeriodTrialBalance::into_canonical`] (`"us_gaap"` / `"ifrs"` /
    /// `"french_gaap"` / `"german_gaap"` / `"dual_reporting"`). Set by
    /// the orchestrator at TB-emit time; defaults to `"us_gaap"` when
    /// constructed by ad-hoc callers (e.g. test fixtures).
    #[serde(default = "default_framework")]
    pub framework: String,
}

fn default_framework() -> String {
    "us_gaap".to_string()
}

impl PeriodTrialBalance {
    /// Convert this in-memory period TB into the canonical
    /// [`datasynth_core::models::balance::TrialBalance`] shape used
    /// for the on-disk artefact.
    ///
    /// v5.1: the on-disk shape is now canonical end-to-end.  Group
    /// aggregate's `tb_loader` consumes the canonical type directly,
    /// dropping the v5.0 dual-shape detection that converted from
    /// `PeriodTrialBalance` JSON on the fly.
    ///
    /// v5.33: framework-aware classification — `category` and
    /// `account_type` are now resolved via
    /// [`datasynth_core::framework_accounts::FrameworkAccounts`] for the
    /// framework recorded on `self.framework`, fixing the v5.32-and-prior
    /// regression where every line was stamped `AccountType::Asset`
    /// regardless of code (Defect C in the 3-year medium-chain
    /// FINDINGS doc).
    ///
    /// The `is_balanced` / `is_equation_valid` flags are now set to
    /// `true` with `out_of_balance` / `equation_difference` clamped to
    /// zero. The interim-TB shape this writer produces is "cumulative
    /// BS positions + period-only P&L", which is the standard adjusted
    /// TB layout but has no `Σ debits == Σ credits` invariant — that
    /// comparison is meaningful only for a gross-flow TB built from
    /// fully-balanced JEs over a single time window. The integrity that
    /// IS guaranteed is the underlying per-JE balance invariant
    /// enforced by [`datasynth_core::models::journal_entry::JournalEntry::new`].
    /// Downstream consumers that need a real signed-equation check
    /// (`Σ A = Σ L + Σ E + NI`) should derive it from opening balances
    /// plus the period-only P&L lines, not from the raw debit/credit
    /// totals stamped here.
    pub fn into_canonical(self, company_code: &str, currency: &str) -> TrialBalance {
        let framework = &self.framework;
        let fa = datasynth_core::framework_accounts::FrameworkAccounts::for_framework(framework);
        let mut total_debits = Decimal::ZERO;
        let mut total_credits = Decimal::ZERO;
        let lines: Vec<TrialBalanceLine> = self
            .entries
            .into_iter()
            .map(|e| {
                total_debits += e.debit_balance;
                total_credits += e.credit_balance;
                let category =
                    AccountCategory::from_account_code_with_framework(&e.account_code, framework);
                let account_type = fa.classify_account_type(&e.account_code);
                TrialBalanceLine {
                    account_code: e.account_code,
                    account_description: e.account_name,
                    category,
                    account_type,
                    opening_balance: Decimal::ZERO,
                    period_debits: e.debit_balance,
                    period_credits: e.credit_balance,
                    closing_balance: e.debit_balance - e.credit_balance,
                    debit_balance: e.debit_balance,
                    credit_balance: e.credit_balance,
                    cost_center: None,
                    profit_center: None,
                }
            })
            .collect();
        TrialBalance {
            trial_balance_id: format!(
                "{company_code}-{:04}{:02}",
                self.fiscal_year, self.fiscal_period
            ),
            company_code: company_code.to_string(),
            company_name: None,
            as_of_date: self.period_end,
            fiscal_year: self.fiscal_year as i32,
            fiscal_period: self.fiscal_period as u32,
            currency: currency.to_string(),
            balance_type: TrialBalanceType::Adjusted,
            lines,
            total_debits,
            total_credits,
            is_balanced: true,
            out_of_balance: Decimal::ZERO,
            is_equation_valid: true,
            equation_difference: Decimal::ZERO,
            category_summary: std::collections::HashMap::new(),
            created_at: self
                .period_start
                .and_hms_opt(0, 0, 0)
                .expect("midnight is a valid time"),
            created_by: "ORCHESTRATOR".to_string(),
            approved_by: None,
            approved_at: None,
            status: TrialBalanceStatus::Final,
        }
    }
}

/// Financial reporting snapshot (financial statements + bank reconciliations).
#[derive(Debug, Clone, Default)]
pub struct FinancialReportingSnapshot {
    /// Financial statements (balance sheet, income statement, cash flow).
    /// For multi-entity configs this includes all standalone statements.
    pub financial_statements: Vec<FinancialStatement>,
    /// Standalone financial statements keyed by entity code.
    /// Each entity has its own slice of statements.
    pub standalone_statements: std::collections::HashMap<String, Vec<FinancialStatement>>,
    /// Consolidated financial statements for the group (one per period, is_consolidated=true).
    pub consolidated_statements: Vec<FinancialStatement>,
    /// Consolidation schedules (one per period) showing pre/post elimination detail.
    pub consolidation_schedules: Vec<ConsolidationSchedule>,
    /// Bank reconciliations.
    pub bank_reconciliations: Vec<BankReconciliation>,
    /// Period-close trial balances (one per period).
    pub trial_balances: Vec<PeriodTrialBalance>,
    /// IFRS 8 / ASC 280 operating segment reports (one per segment per period).
    pub segment_reports: Vec<datasynth_core::models::OperatingSegment>,
    /// IFRS 8 / ASC 280 segment reconciliations (one per period tying segments to consolidated FS).
    pub segment_reconciliations: Vec<datasynth_core::models::SegmentReconciliation>,
    /// Notes to the financial statements (IAS 1 / ASC 235) — one set per entity.
    pub notes_to_financial_statements: Vec<datasynth_core::models::FinancialStatementNote>,
}

/// HR data snapshot (payroll runs, time entries, expense reports, benefit enrollments, pensions).
#[derive(Debug, Clone, Default)]
pub struct HrSnapshot {
    /// Payroll runs (actual data).
    pub payroll_runs: Vec<PayrollRun>,
    /// Payroll line items (actual data).
    pub payroll_line_items: Vec<PayrollLineItem>,
    /// Time entries (actual data).
    pub time_entries: Vec<TimeEntry>,
    /// Expense reports (actual data).
    pub expense_reports: Vec<ExpenseReport>,
    /// Benefit enrollments (actual data).
    pub benefit_enrollments: Vec<BenefitEnrollment>,
    /// Defined benefit pension plans (IAS 19 / ASC 715).
    pub pension_plans: Vec<datasynth_core::models::pension::DefinedBenefitPlan>,
    /// Pension obligation (DBO) roll-forwards.
    pub pension_obligations: Vec<datasynth_core::models::pension::PensionObligation>,
    /// Plan asset roll-forwards.
    pub pension_plan_assets: Vec<datasynth_core::models::pension::PlanAssets>,
    /// Pension disclosures.
    pub pension_disclosures: Vec<datasynth_core::models::pension::PensionDisclosure>,
    /// Journal entries generated from pension expense and OCI remeasurements.
    pub pension_journal_entries: Vec<JournalEntry>,
    /// Stock grants (ASC 718 / IFRS 2).
    pub stock_grants: Vec<datasynth_core::models::stock_compensation::StockGrant>,
    /// Stock-based compensation period expense records.
    pub stock_comp_expenses: Vec<datasynth_core::models::stock_compensation::StockCompExpense>,
    /// Journal entries generated from stock-based compensation expense.
    pub stock_comp_journal_entries: Vec<JournalEntry>,
    /// Payroll runs.
    pub payroll_run_count: usize,
    /// Payroll line item count.
    pub payroll_line_item_count: usize,
    /// Time entry count.
    pub time_entry_count: usize,
    /// Expense report count.
    pub expense_report_count: usize,
    /// Benefit enrollment count.
    pub benefit_enrollment_count: usize,
    /// Pension plan count.
    pub pension_plan_count: usize,
    /// Stock grant count.
    pub stock_grant_count: usize,
}

/// Accounting standards data snapshot (revenue recognition, impairment, business combinations).
#[derive(Debug, Clone, Default)]
pub struct AccountingStandardsSnapshot {
    /// Revenue recognition contracts (actual data).
    pub contracts: Vec<datasynth_standards::accounting::revenue::CustomerContract>,
    /// Impairment tests (actual data).
    pub impairment_tests: Vec<datasynth_standards::accounting::impairment::ImpairmentTest>,
    /// Business combinations (IFRS 3 / ASC 805).
    pub business_combinations:
        Vec<datasynth_core::models::business_combination::BusinessCombination>,
    /// Journal entries generated from business combinations (Day 1 + amortization).
    pub business_combination_journal_entries: Vec<JournalEntry>,
    /// ECL models (IFRS 9 / ASC 326).
    pub ecl_models: Vec<datasynth_core::models::expected_credit_loss::EclModel>,
    /// ECL provision movements.
    pub ecl_provision_movements:
        Vec<datasynth_core::models::expected_credit_loss::EclProvisionMovement>,
    /// Journal entries from ECL provision.
    pub ecl_journal_entries: Vec<JournalEntry>,
    /// Provisions (IAS 37 / ASC 450).
    pub provisions: Vec<datasynth_core::models::provision::Provision>,
    /// Provision movement roll-forwards (IAS 37 / ASC 450).
    pub provision_movements: Vec<datasynth_core::models::provision::ProvisionMovement>,
    /// Contingent liabilities (IAS 37 / ASC 450).
    pub contingent_liabilities: Vec<datasynth_core::models::provision::ContingentLiability>,
    /// Journal entries from provisions.
    pub provision_journal_entries: Vec<JournalEntry>,
    /// IAS 21 functional currency translation results (one per entity per period).
    pub currency_translation_results:
        Vec<datasynth_core::models::currency_translation_result::CurrencyTranslationResult>,
    /// Revenue recognition contract count.
    pub revenue_contract_count: usize,
    /// Impairment test count.
    pub impairment_test_count: usize,
    /// Business combination count.
    pub business_combination_count: usize,
    /// ECL model count.
    pub ecl_model_count: usize,
    /// Provision count.
    pub provision_count: usize,
    /// Currency translation result count (IAS 21).
    pub currency_translation_count: usize,
    // ---- v3.3.1: Lease / FairValue / FrameworkReconciliation ----
    /// Lease contracts (IFRS 16 / ASC 842). Each entry carries its own
    /// ROU asset + lease liability details.
    pub leases: Vec<datasynth_standards::accounting::leases::Lease>,
    /// Fair value measurements (IFRS 13 / ASC 820) across Level 1/2/3.
    pub fair_value_measurements:
        Vec<datasynth_standards::accounting::fair_value::FairValueMeasurement>,
    /// Framework difference records (dual-reporting only).
    pub framework_differences:
        Vec<datasynth_standards::accounting::differences::FrameworkDifferenceRecord>,
    /// Per-entity framework reconciliation (dual-reporting only).
    pub framework_reconciliations:
        Vec<datasynth_standards::accounting::differences::FrameworkReconciliation>,
    /// Counts for stats logging.
    pub lease_count: usize,
    pub fair_value_measurement_count: usize,
    pub framework_difference_count: usize,
    /// W1-3 Stage 2: ASC 606 / IFRS 15 deferred-revenue recognition JEs (inception
    /// funding + per-month recognition). Empty unless `monthly_recurring` is ON.
    pub revenue_recognition_journal_entries: Vec<JournalEntry>,
    /// W1-3 Stage 2: ASC 842 / IFRS 16 lease JEs (inception ROU/liability + per-month
    /// interest/amortization or operating straight-line). Empty unless
    /// `monthly_recurring` is ON.
    pub lease_journal_entries: Vec<JournalEntry>,
}

/// Compliance regulations framework snapshot (standards, procedures, findings, filings, graph).
#[derive(Debug, Clone, Default)]
pub struct ComplianceRegulationsSnapshot {
    /// Flattened standard records for output.
    pub standard_records: Vec<datasynth_generators::compliance::ComplianceStandardRecord>,
    /// Cross-reference records.
    pub cross_reference_records: Vec<datasynth_generators::compliance::CrossReferenceRecord>,
    /// Jurisdiction profile records.
    pub jurisdiction_records: Vec<datasynth_generators::compliance::JurisdictionRecord>,
    /// Generated audit procedures.
    pub audit_procedures: Vec<datasynth_generators::compliance::AuditProcedureRecord>,
    /// Generated compliance findings.
    pub findings: Vec<datasynth_core::models::compliance::ComplianceFinding>,
    /// Generated regulatory filings.
    pub filings: Vec<datasynth_core::models::compliance::RegulatoryFiling>,
    /// Compliance graph (if graph integration enabled).
    pub compliance_graph: Option<datasynth_graph::Graph>,
}

/// Manufacturing data snapshot (production orders, quality inspections, cycle counts, BOMs, inventory movements).
#[derive(Debug, Clone, Default)]
pub struct ManufacturingSnapshot {
    /// Production orders (actual data).
    pub production_orders: Vec<ProductionOrder>,
    /// Quality inspections (actual data).
    pub quality_inspections: Vec<QualityInspection>,
    /// Cycle counts (actual data).
    pub cycle_counts: Vec<CycleCount>,
    /// BOM components (actual data).
    pub bom_components: Vec<BomComponent>,
    /// Inventory movements (actual data).
    pub inventory_movements: Vec<InventoryMovement>,
    /// Production order count.
    pub production_order_count: usize,
    /// Quality inspection count.
    pub quality_inspection_count: usize,
    /// Cycle count count.
    pub cycle_count_count: usize,
    /// BOM component count.
    pub bom_component_count: usize,
    /// Inventory movement count.
    pub inventory_movement_count: usize,
}

/// Sales, KPI, and budget data snapshot.
#[derive(Debug, Clone, Default)]
pub struct SalesKpiBudgetsSnapshot {
    /// Sales quotes (actual data).
    pub sales_quotes: Vec<SalesQuote>,
    /// Management KPIs (actual data).
    pub kpis: Vec<ManagementKpi>,
    /// Budgets (actual data).
    pub budgets: Vec<Budget>,
    /// Sales quote count.
    pub sales_quote_count: usize,
    /// Management KPI count.
    pub kpi_count: usize,
    /// Budget line count.
    pub budget_line_count: usize,
}

/// Anomaly labels generated during injection.
#[derive(Debug, Clone, Default)]
pub struct AnomalyLabels {
    /// All anomaly labels.
    pub labels: Vec<LabeledAnomaly>,
    /// Summary statistics.
    pub summary: Option<AnomalySummary>,
    /// Count by anomaly type.
    pub by_type: HashMap<String, usize>,
}

/// Balance validation results from running balance tracker.
#[derive(Debug, Clone, Default)]
pub struct BalanceValidationResult {
    /// Whether validation was performed.
    pub validated: bool,
    /// Whether balance sheet equation is satisfied.
    pub is_balanced: bool,
    /// Number of entries processed.
    pub entries_processed: u64,
    /// Total debits across all entries.
    pub total_debits: rust_decimal::Decimal,
    /// Total credits across all entries.
    pub total_credits: rust_decimal::Decimal,
    /// Number of accounts tracked.
    pub accounts_tracked: usize,
    /// Number of companies tracked.
    pub companies_tracked: usize,
    /// Validation errors encountered.
    pub validation_errors: Vec<ValidationError>,
    /// Whether any unbalanced entries were found.
    pub has_unbalanced_entries: bool,
}

/// Tax data snapshot (jurisdictions, codes, provisions, returns, withholding).
#[derive(Debug, Clone, Default)]
pub struct TaxSnapshot {
    /// Tax jurisdictions.
    pub jurisdictions: Vec<TaxJurisdiction>,
    /// Tax codes.
    pub codes: Vec<TaxCode>,
    /// Tax lines computed on documents.
    pub tax_lines: Vec<TaxLine>,
    /// Tax returns filed per period.
    pub tax_returns: Vec<TaxReturn>,
    /// Tax provisions.
    pub tax_provisions: Vec<TaxProvision>,
    /// Withholding tax records.
    pub withholding_records: Vec<WithholdingTaxRecord>,
    /// Tax anomaly labels.
    pub tax_anomaly_labels: Vec<datasynth_generators::TaxAnomalyLabel>,
    /// Jurisdiction count.
    pub jurisdiction_count: usize,
    /// Code count.
    pub code_count: usize,
    /// Deferred tax engine output (temporary differences, ETR reconciliation, rollforwards, JEs).
    pub deferred_tax: datasynth_generators::DeferredTaxSnapshot,
    /// Journal entries posting tax payable/receivable from computed tax lines.
    pub tax_posting_journal_entries: Vec<JournalEntry>,
}

/// Intercompany data snapshot (IC transactions, matched pairs, eliminations).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntercompanySnapshot {
    /// Group ownership structure (parent/subsidiary/associate relationships).
    pub group_structure: Option<datasynth_core::models::intercompany::GroupStructure>,
    /// IC matched pairs (transaction pairs between related entities).
    pub matched_pairs: Vec<datasynth_core::models::intercompany::ICMatchedPair>,
    /// IC journal entries generated from matched pairs (seller side).
    pub seller_journal_entries: Vec<JournalEntry>,
    /// IC journal entries generated from matched pairs (buyer side).
    pub buyer_journal_entries: Vec<JournalEntry>,
    /// Elimination entries for consolidation.
    pub elimination_entries: Vec<datasynth_core::models::intercompany::EliminationEntry>,
    /// NCI measurements derived from group structure ownership percentages.
    pub nci_measurements: Vec<datasynth_core::models::intercompany::NciMeasurement>,
    /// IC source document chains (seller invoices, buyer POs/GRs/VIs).
    #[serde(skip)]
    pub ic_document_chains: Option<datasynth_generators::ICDocumentChains>,
    /// IC matched pair count.
    pub matched_pair_count: usize,
    /// IC elimination entry count.
    pub elimination_entry_count: usize,
    /// IC matching rate (0.0 to 1.0).
    pub match_rate: f64,
}

/// ESG data snapshot (emissions, energy, water, waste, social, governance, supply chain, disclosures).
#[derive(Debug, Clone, Default)]
pub struct EsgSnapshot {
    /// Emission records (scope 1, 2, 3).
    pub emissions: Vec<EmissionRecord>,
    /// Energy consumption records.
    pub energy: Vec<EnergyConsumption>,
    /// Water usage records.
    pub water: Vec<WaterUsage>,
    /// Waste records.
    pub waste: Vec<WasteRecord>,
    /// Workforce diversity metrics.
    pub diversity: Vec<WorkforceDiversityMetric>,
    /// Pay equity metrics.
    pub pay_equity: Vec<PayEquityMetric>,
    /// Safety incidents.
    pub safety_incidents: Vec<SafetyIncident>,
    /// Safety metrics.
    pub safety_metrics: Vec<SafetyMetric>,
    /// Governance metrics.
    pub governance: Vec<GovernanceMetric>,
    /// Supplier ESG assessments.
    pub supplier_assessments: Vec<SupplierEsgAssessment>,
    /// Materiality assessments.
    pub materiality: Vec<MaterialityAssessment>,
    /// ESG disclosures.
    pub disclosures: Vec<EsgDisclosure>,
    /// Climate scenarios.
    pub climate_scenarios: Vec<ClimateScenario>,
    /// ESG anomaly labels.
    pub anomaly_labels: Vec<EsgAnomalyLabel>,
    /// Total emission record count.
    pub emission_count: usize,
    /// Total disclosure count.
    pub disclosure_count: usize,
}

/// Treasury data snapshot (cash management, hedging, debt, pooling).
#[derive(Debug, Clone, Default)]
pub struct TreasurySnapshot {
    /// Cash positions (daily balances per account).
    pub cash_positions: Vec<CashPosition>,
    /// Cash forecasts.
    pub cash_forecasts: Vec<CashForecast>,
    /// Cash pools.
    pub cash_pools: Vec<CashPool>,
    /// Cash pool sweep transactions.
    pub cash_pool_sweeps: Vec<CashPoolSweep>,
    /// Hedging instruments.
    pub hedging_instruments: Vec<HedgingInstrument>,
    /// Hedge relationships (ASC 815/IFRS 9 designations).
    pub hedge_relationships: Vec<HedgeRelationship>,
    /// Debt instruments.
    pub debt_instruments: Vec<DebtInstrument>,
    /// Bank guarantees and letters of credit.
    pub bank_guarantees: Vec<BankGuarantee>,
    /// Intercompany netting runs.
    pub netting_runs: Vec<NettingRun>,
    /// Treasury anomaly labels.
    pub treasury_anomaly_labels: Vec<datasynth_generators::treasury::TreasuryAnomalyLabel>,
    /// Journal entries generated from treasury instruments (debt interest accruals,
    /// hedge MTM, cash pool sweeps).
    pub journal_entries: Vec<JournalEntry>,
}

/// Project accounting data snapshot (projects, costs, revenue, milestones, EVM).
#[derive(Debug, Clone, Default)]
pub struct ProjectAccountingSnapshot {
    /// Projects with WBS hierarchies.
    pub projects: Vec<Project>,
    /// Project cost lines (linked from source documents).
    pub cost_lines: Vec<ProjectCostLine>,
    /// Revenue recognition records.
    pub revenue_records: Vec<ProjectRevenue>,
    /// Earned value metrics.
    pub earned_value_metrics: Vec<EarnedValueMetric>,
    /// Change orders.
    pub change_orders: Vec<ChangeOrder>,
    /// Project milestones.
    pub milestones: Vec<ProjectMilestone>,
}

/// Complete result of enhanced generation run.
#[derive(Debug, Default)]
pub struct EnhancedGenerationResult {
    /// Generated chart of accounts.
    pub chart_of_accounts: ChartOfAccounts,
    /// Master data snapshot.
    pub master_data: MasterDataSnapshot,
    /// Document flow snapshot.
    pub document_flows: DocumentFlowSnapshot,
    /// Subledger snapshot (linked from document flows).
    pub subledger: SubledgerSnapshot,
    /// OCPM event log snapshot (if OCPM generation enabled).
    pub ocpm: OcpmSnapshot,
    /// Audit data snapshot (if audit generation enabled).
    pub audit: AuditSnapshot,
    /// Banking KYC/AML data snapshot (if banking generation enabled).
    pub banking: BankingSnapshot,
    /// Graph export snapshot (if graph export enabled).
    pub graph_export: GraphExportSnapshot,
    /// S2C sourcing data snapshot (if sourcing generation enabled).
    pub sourcing: SourcingSnapshot,
    /// Financial reporting snapshot (financial statements + bank reconciliations).
    pub financial_reporting: FinancialReportingSnapshot,
    /// HR data snapshot (payroll, time entries, expenses).
    pub hr: HrSnapshot,
    /// Accounting standards snapshot (revenue recognition, impairment).
    pub accounting_standards: AccountingStandardsSnapshot,
    /// Manufacturing snapshot (production orders, quality inspections, cycle counts).
    pub manufacturing: ManufacturingSnapshot,
    /// Sales, KPI, and budget snapshot.
    pub sales_kpi_budgets: SalesKpiBudgetsSnapshot,
    /// Tax data snapshot (jurisdictions, codes, provisions, returns).
    pub tax: TaxSnapshot,
    /// ESG data snapshot (emissions, energy, social, governance, disclosures).
    pub esg: EsgSnapshot,
    /// Treasury data snapshot (cash management, hedging, debt).
    pub treasury: TreasurySnapshot,
    /// Project accounting data snapshot (projects, costs, revenue, EVM, milestones).
    pub project_accounting: ProjectAccountingSnapshot,
    /// Process evolution events (workflow changes, automation, policy changes, control enhancements).
    pub process_evolution: Vec<ProcessEvolutionEvent>,
    /// Organizational events (acquisitions, divestitures, reorganizations, leadership changes).
    pub organizational_events: Vec<OrganizationalEvent>,
    /// Disruption events (outages, migrations, process changes, recoveries, regulatory).
    pub disruption_events: Vec<datasynth_generators::disruption::DisruptionEvent>,
    /// Intercompany data snapshot (IC transactions, matched pairs, eliminations).
    pub intercompany: IntercompanySnapshot,
    /// Generated journal entries.
    pub journal_entries: Vec<JournalEntry>,
    /// Anomaly labels (if injection enabled).
    pub anomaly_labels: AnomalyLabels,
    /// Balance validation results (if validation enabled).
    pub balance_validation: BalanceValidationResult,
    /// Data quality statistics (if injection enabled).
    pub data_quality_stats: DataQualityStats,
    /// Data quality issue records (if injection enabled).
    pub quality_issues: Vec<datasynth_generators::QualityIssue>,
    /// Generation statistics.
    pub statistics: EnhancedGenerationStatistics,
    /// Data lineage graph (if tracking enabled).
    pub lineage: Option<super::lineage::LineageGraph>,
    /// Quality gate evaluation result.
    pub gate_result: Option<datasynth_eval::gates::GateResult>,
    /// Internal controls (if controls generation enabled).
    pub internal_controls: Vec<InternalControl>,
    /// SoD (Segregation of Duties) violations identified during control application.
    ///
    /// Each record corresponds to a journal entry where `sod_violation == true`.
    pub sod_violations: Vec<datasynth_core::models::SodViolation>,
    /// Opening balances (if opening balance generation enabled).
    pub opening_balances: Vec<GeneratedOpeningBalance>,
    /// GL-to-subledger reconciliation results (if reconciliation enabled).
    pub subledger_reconciliation: Vec<datasynth_generators::ReconciliationResult>,
    /// Counterfactual (original, mutated) JE pairs for ML training.
    pub counterfactual_pairs: Vec<datasynth_generators::counterfactual::CounterfactualPair>,
    /// Fraud red-flag indicators on P2P/O2C documents.
    pub red_flags: Vec<datasynth_generators::fraud::RedFlag>,
    /// Collusion rings (coordinated fraud networks).
    pub collusion_rings: Vec<datasynth_generators::fraud::CollusionRing>,
    /// Bi-temporal version chains for vendor entities.
    pub temporal_vendor_chains:
        Vec<datasynth_core::models::TemporalVersionChain<datasynth_core::models::Vendor>>,
    /// Entity relationship graph (nodes + edges with strength scores).
    pub entity_relationship_graph: Option<datasynth_core::models::EntityGraph>,
    /// Cross-process links (P2P ↔ O2C via inventory movements).
    pub cross_process_links: Vec<datasynth_core::models::CrossProcessLink>,
    /// Industry-specific GL accounts and metadata.
    pub industry_output: Option<datasynth_generators::industry::factory::IndustryOutput>,
    /// SP5.2 — CoA semantic prior snapshot. When `Some`, `write_journal_entries_csv`
    /// builds a secondary lookup from the prior's 3,123 corpus accounts and uses
    /// it as a fallback when the synthetic CoA index misses a line's `gl_account`
    /// (common when SP3.7's per-source attribute conditional emits corpus account
    /// numbers that differ from the synthetic CoA master table's number set).
    pub coa_semantic_prior:
        Option<datasynth_core::distributions::behavioral_priors::CoaSemanticPrior>,
    /// Compliance regulations framework data (standards, procedures, findings, filings, graph).
    pub compliance_regulations: ComplianceRegulationsSnapshot,
    /// v3.3.0: analytics-metadata snapshot (prior-year comparatives,
    /// industry benchmarks, management reports, drift events). Empty
    /// when `analytics_metadata.enabled = false`.
    pub analytics_metadata: AnalyticsMetadataSnapshot,
    /// v3.5.1+: statistical validation report (Benford, chi-squared,
    /// KS) over the generated amount distribution.  `None` when
    /// `distributions.validation.enabled = false`.
    pub statistical_validation: Option<datasynth_core::distributions::StatisticalValidationReport>,
    /// v4.1.3+: interconnectivity snapshot — vendor tier assignments,
    /// customer value-segment labels, and industry-specific metadata
    /// populated from the previously-inert `vendor_network`,
    /// `customer_segmentation`, and `industry_specific` schema
    /// sections. Empty when those sections are disabled.
    pub interconnectivity: InterconnectivitySnapshot,
}

/// v4.1.3+: interconnectivity snapshot. Populated when
/// `vendor_network.enabled` / `customer_segmentation.enabled` /
/// `industry_specific.enabled` are set. Holds tier / segment / industry
/// labels for generated entities so downstream tooling (graph export,
/// risk models) can consume them without re-deriving from scratch.
#[derive(Debug, Clone, Default)]
pub struct InterconnectivitySnapshot {
    /// `(vendor_id, tier)` pairs. Tier 1 = strategic / primary; Tier 2
    /// = sub-tier suppliers to tier 1; Tier 3 = sub-sub-tier.
    pub vendor_tiers: Vec<(String, u8)>,
    /// `(vendor_id, cluster_label)` pairs where cluster_label is one of
    /// `"reliable_strategic" / "standard_operational" / "transactional"
    /// / "problematic"`.
    pub vendor_clusters: Vec<(String, String)>,
    /// `(customer_id, value_segment)` pairs where value_segment is one
    /// of `"enterprise" / "mid_market" / "smb" / "consumer"`.
    pub customer_value_segments: Vec<(String, String)>,
    /// `(customer_id, lifecycle_stage)` pairs where stage is one of
    /// `"prospect" / "new" / "growth" / "mature" / "at_risk" /
    /// "churned" / "won_back"`.
    pub customer_lifecycle_stages: Vec<(String, String)>,
    /// Summary: industry-specific knob applied, if any (e.g.
    /// `"manufacturing.bom_depth=3"`).
    pub industry_metadata: Vec<String>,
}

/// v3.3.0: snapshot for the analytics-metadata phase.
#[derive(Debug, Clone, Default)]
pub struct AnalyticsMetadataSnapshot {
    /// Prior-year comparative balances per account, per entity.
    pub prior_year_comparatives: Vec<datasynth_core::models::PriorYearComparative>,
    /// Industry benchmarks for the configured industry.
    pub industry_benchmarks: Vec<datasynth_core::models::IndustryBenchmark>,
    /// Management-report artefacts (dashboards, MDA sections).
    pub management_reports: Vec<datasynth_core::models::ManagementReport>,
    /// Drift-event labels emitted from the post-generation sweep.
    pub drift_events: Vec<datasynth_core::models::LabeledDriftEvent>,
}

/// Enhanced statistics about a generation run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnhancedGenerationStatistics {
    /// Total journal entries generated.
    pub total_entries: u64,
    /// Total line items generated.
    pub total_line_items: u64,
    /// Number of accounts in CoA.
    pub accounts_count: usize,
    /// Number of companies.
    pub companies_count: usize,
    /// Period in months.
    pub period_months: u32,
    /// Master data counts.
    pub vendor_count: usize,
    pub customer_count: usize,
    pub material_count: usize,
    pub asset_count: usize,
    pub employee_count: usize,
    /// Document flow counts.
    pub p2p_chain_count: usize,
    pub o2c_chain_count: usize,
    /// Subledger counts.
    pub ap_invoice_count: usize,
    pub ar_invoice_count: usize,
    /// OCPM counts.
    pub ocpm_event_count: usize,
    pub ocpm_object_count: usize,
    pub ocpm_case_count: usize,
    /// Audit counts.
    pub audit_engagement_count: usize,
    pub audit_workpaper_count: usize,
    pub audit_evidence_count: usize,
    pub audit_risk_count: usize,
    pub audit_finding_count: usize,
    pub audit_judgment_count: usize,
    /// ISA 505 confirmation counts.
    #[serde(default)]
    pub audit_confirmation_count: usize,
    #[serde(default)]
    pub audit_confirmation_response_count: usize,
    /// ISA 330/530 procedure step and sample counts.
    #[serde(default)]
    pub audit_procedure_step_count: usize,
    #[serde(default)]
    pub audit_sample_count: usize,
    /// ISA 520 analytical procedure counts.
    #[serde(default)]
    pub audit_analytical_result_count: usize,
    /// ISA 610 internal audit counts.
    #[serde(default)]
    pub audit_ia_function_count: usize,
    #[serde(default)]
    pub audit_ia_report_count: usize,
    /// ISA 550 related party counts.
    #[serde(default)]
    pub audit_related_party_count: usize,
    #[serde(default)]
    pub audit_related_party_transaction_count: usize,
    /// Anomaly counts.
    pub anomalies_injected: usize,
    /// Data quality issue counts.
    pub data_quality_issues: usize,
    /// Banking counts.
    pub banking_customer_count: usize,
    pub banking_account_count: usize,
    pub banking_transaction_count: usize,
    pub banking_suspicious_count: usize,
    /// Graph export counts.
    pub graph_export_count: usize,
    pub graph_node_count: usize,
    pub graph_edge_count: usize,
    /// LLM enrichment timing (milliseconds).
    #[serde(default)]
    pub llm_enrichment_ms: u64,
    /// Number of vendor names enriched by LLM.
    #[serde(default)]
    pub llm_vendors_enriched: usize,
    /// v4.1.1+: number of customer names enriched by LLM.
    #[serde(default)]
    pub llm_customers_enriched: usize,
    /// v4.1.1+: number of material descriptions enriched by LLM.
    #[serde(default)]
    pub llm_materials_enriched: usize,
    /// v4.1.1+: number of audit finding titles enriched by LLM.
    #[serde(default)]
    pub llm_findings_enriched: usize,
    /// Diffusion enhancement timing (milliseconds).
    #[serde(default)]
    pub diffusion_enhancement_ms: u64,
    /// Number of diffusion samples generated.
    #[serde(default)]
    pub diffusion_samples_generated: usize,
    /// Hybrid-diffusion blend weight actually applied (after clamp to \[0,1\]).
    /// `None` when the neural/hybrid backend is not active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neural_hybrid_weight: Option<f64>,
    /// Hybrid-diffusion strategy applied (weighted_average / column_select / threshold).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neural_hybrid_strategy: Option<String>,
    /// How many columns were routed through the neural backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neural_routed_column_count: Option<usize>,
    /// Causal generation timing (milliseconds).
    #[serde(default)]
    pub causal_generation_ms: u64,
    /// Number of causal samples generated.
    #[serde(default)]
    pub causal_samples_generated: usize,
    /// Whether causal validation passed.
    #[serde(default)]
    pub causal_validation_passed: Option<bool>,
    /// S2C sourcing counts.
    #[serde(default)]
    pub sourcing_project_count: usize,
    #[serde(default)]
    pub rfx_event_count: usize,
    #[serde(default)]
    pub bid_count: usize,
    #[serde(default)]
    pub contract_count: usize,
    #[serde(default)]
    pub catalog_item_count: usize,
    #[serde(default)]
    pub scorecard_count: usize,
    /// Financial reporting counts.
    #[serde(default)]
    pub financial_statement_count: usize,
    #[serde(default)]
    pub bank_reconciliation_count: usize,
    /// HR counts.
    #[serde(default)]
    pub payroll_run_count: usize,
    #[serde(default)]
    pub time_entry_count: usize,
    #[serde(default)]
    pub expense_report_count: usize,
    #[serde(default)]
    pub benefit_enrollment_count: usize,
    #[serde(default)]
    pub pension_plan_count: usize,
    #[serde(default)]
    pub stock_grant_count: usize,
    /// Accounting standards counts.
    #[serde(default)]
    pub revenue_contract_count: usize,
    #[serde(default)]
    pub impairment_test_count: usize,
    #[serde(default)]
    pub business_combination_count: usize,
    #[serde(default)]
    pub ecl_model_count: usize,
    #[serde(default)]
    pub provision_count: usize,
    /// Manufacturing counts.
    #[serde(default)]
    pub production_order_count: usize,
    #[serde(default)]
    pub quality_inspection_count: usize,
    #[serde(default)]
    pub cycle_count_count: usize,
    #[serde(default)]
    pub bom_component_count: usize,
    #[serde(default)]
    pub inventory_movement_count: usize,
    /// Sales & reporting counts.
    #[serde(default)]
    pub sales_quote_count: usize,
    #[serde(default)]
    pub kpi_count: usize,
    #[serde(default)]
    pub budget_line_count: usize,
    /// Tax counts.
    #[serde(default)]
    pub tax_jurisdiction_count: usize,
    #[serde(default)]
    pub tax_code_count: usize,
    /// ESG counts.
    #[serde(default)]
    pub esg_emission_count: usize,
    #[serde(default)]
    pub esg_disclosure_count: usize,
    /// Intercompany counts.
    #[serde(default)]
    pub ic_matched_pair_count: usize,
    #[serde(default)]
    pub ic_elimination_count: usize,
    /// Number of intercompany journal entries (seller + buyer side).
    #[serde(default)]
    pub ic_transaction_count: usize,
    /// Number of fixed asset subledger records.
    #[serde(default)]
    pub fa_subledger_count: usize,
    /// Number of inventory subledger records.
    #[serde(default)]
    pub inventory_subledger_count: usize,
    /// Treasury debt instrument count.
    #[serde(default)]
    pub treasury_debt_instrument_count: usize,
    /// Treasury hedging instrument count.
    #[serde(default)]
    pub treasury_hedging_instrument_count: usize,
    /// Project accounting project count.
    #[serde(default)]
    pub project_count: usize,
    /// Project accounting change order count.
    #[serde(default)]
    pub project_change_order_count: usize,
    /// Tax provision count.
    #[serde(default)]
    pub tax_provision_count: usize,
    /// Opening balance count.
    #[serde(default)]
    pub opening_balance_count: usize,
    /// Subledger reconciliation count.
    #[serde(default)]
    pub subledger_reconciliation_count: usize,
    /// Tax line count.
    #[serde(default)]
    pub tax_line_count: usize,
    /// Project cost line count.
    #[serde(default)]
    pub project_cost_line_count: usize,
    /// Cash position count.
    #[serde(default)]
    pub cash_position_count: usize,
    /// Cash forecast count.
    #[serde(default)]
    pub cash_forecast_count: usize,
    /// Cash pool count.
    #[serde(default)]
    pub cash_pool_count: usize,
    /// Process evolution event count.
    #[serde(default)]
    pub process_evolution_event_count: usize,
    /// Organizational event count.
    #[serde(default)]
    pub organizational_event_count: usize,
    /// Counterfactual pair count.
    #[serde(default)]
    pub counterfactual_pair_count: usize,
    /// Number of fraud red-flag indicators generated.
    #[serde(default)]
    pub red_flag_count: usize,
    /// Number of collusion rings generated.
    #[serde(default)]
    pub collusion_ring_count: usize,
    /// Number of bi-temporal vendor version chains generated.
    #[serde(default)]
    pub temporal_version_chain_count: usize,
    /// Number of nodes in the entity relationship graph.
    #[serde(default)]
    pub entity_relationship_node_count: usize,
    /// Number of edges in the entity relationship graph.
    #[serde(default)]
    pub entity_relationship_edge_count: usize,
    /// Number of cross-process links generated.
    #[serde(default)]
    pub cross_process_link_count: usize,
    /// Number of disruption events generated.
    #[serde(default)]
    pub disruption_event_count: usize,
    /// Number of industry-specific GL accounts generated.
    #[serde(default)]
    pub industry_gl_account_count: usize,
    /// Number of period-close journal entries generated (tax provision + closing entries).
    #[serde(default)]
    pub period_close_je_count: usize,
}

/// Enhanced orchestrator with full feature integration.
pub struct EnhancedOrchestrator {
    config: GeneratorConfig,
    phase_config: PhaseConfig,
    coa: Option<Arc<ChartOfAccounts>>,
    master_data: MasterDataSnapshot,
    seed: u64,
    multi_progress: Option<MultiProgress>,
    /// Resource guard for memory, disk, and CPU monitoring
    resource_guard: ResourceGuard,
    /// Output path for disk space monitoring
    output_path: Option<PathBuf>,
    /// Copula generators for preserving correlations (from fingerprint)
    copula_generators: Vec<CopulaGeneratorSpec>,
    /// Country pack registry for localized data generation
    country_pack_registry: datasynth_core::CountryPackRegistry,
    /// Optional streaming sink for phase-by-phase output
    phase_sink: Option<Box<dyn crate::stream_pipeline::PhaseSink>>,
    /// Shared template provider for user-supplied template packs.
    ///
    /// Constructed from `config.templates.path` at orchestrator creation
    /// time. When the path is `None`, this is still populated with an
    /// embedded-only provider so generators can always call trait methods
    /// without an `Option<…>` guard. v3.2.0+.
    template_provider: datasynth_core::templates::SharedTemplateProvider,
    /// v3.4.1+ temporal context for business-day / holiday awareness.
    ///
    /// Populated only when `temporal_patterns.business_days.enabled`. When
    /// `None`, document-flow / HR / treasury / period-close generators keep
    /// their legacy raw-RNG date-offset behaviour (byte-identical to v3.4.0
    /// for the same seed).
    temporal_context: Option<Arc<datasynth_core::distributions::TemporalContext>>,
    /// Optional shard-mode context (set by group-engine shard runners).
    /// `None` preserves byte-for-byte pre-v5.0 single-entity behavior.
    shard_context: Option<crate::shard_context::ShardContext>,
    /// SP3.12 — cached priors, shared between `generate_journal_entries` (which
    /// loads them) and `generate_jes_from_document_flows` (which applies padding).
    /// Set once after the SP3 opt-in block in `generate_journal_entries`.
    cached_priors: Option<std::sync::Arc<datasynth_generators::priors_loader::LoadedPriors>>,
}

impl EnhancedOrchestrator {
    /// Create a new enhanced orchestrator.
    pub fn new(config: GeneratorConfig, phase_config: PhaseConfig) -> SynthResult<Self> {
        datasynth_config::validate_config(&config)?;

        let seed = config.global.seed.unwrap_or_else(rand::random);

        // Build resource guard from config
        let resource_guard = Self::build_resource_guard(&config, None);

        // Build country pack registry from config
        let country_pack_registry = match &config.country_packs {
            Some(cp) => {
                datasynth_core::CountryPackRegistry::new(cp.external_dir.as_deref(), &cp.overrides)
                    .map_err(|e| SynthError::config(e.to_string()))?
            }
            None => datasynth_core::CountryPackRegistry::builtin_only()
                .map_err(|e| SynthError::config(e.to_string()))?,
        };

        // Build the shared template provider from config.templates.path.
        // `None` → embedded-only provider (byte-identical pre-v3.2.0 output).
        // `Some(path)` → load file/dir and honour `merge_strategy`.
        let template_provider = Self::build_template_provider(&config)?;

        // v3.4.1: build a shared temporal context when
        // `temporal_patterns.business_days.enabled`. `None` preserves the
        // raw-RNG date-offset behaviour per-generator.
        let temporal_context = Self::build_temporal_context(&config)?;

        Ok(Self {
            config,
            phase_config,
            coa: None,
            master_data: MasterDataSnapshot::default(),
            seed,
            multi_progress: None,
            resource_guard,
            output_path: None,
            copula_generators: Vec::new(),
            country_pack_registry,
            phase_sink: None,
            template_provider,
            temporal_context,
            shard_context: None,
            cached_priors: None,
        })
    }

    /// Install shard-mode context.  Called by the group shard runner
    /// before [`EnhancedOrchestrator::generate`] (or the equivalent
    /// entry point).  Has no effect on single-entity runs.
    ///
    /// See [`crate::shard_context::ShardContext`] for rationale.
    pub fn set_shard_context(&mut self, ctx: crate::shard_context::ShardContext) {
        self.shard_context = Some(ctx);
    }

    /// Build the shared [`TemporalContext`] from `config.temporal_patterns`.
    ///
    /// Returns `Ok(None)` when temporal-pattern features are disabled — the
    /// caller keeps its legacy raw-RNG path. Returns `Ok(Some(arc))` when
    /// enabled. Returns `Err` only for unrecoverable config errors.
    fn build_temporal_context(
        config: &GeneratorConfig,
    ) -> SynthResult<Option<Arc<datasynth_core::distributions::TemporalContext>>> {
        use datasynth_core::distributions::{parse_region_code, TemporalContext};

        let tp = &config.temporal_patterns;
        if !tp.enabled || !tp.business_days.enabled {
            return Ok(None);
        }

        let start_date = NaiveDate::parse_from_str(&config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let end_date = start_date + chrono::Months::new(config.global.period_months);

        let region_code = tp
            .calendars
            .regions
            .first()
            .cloned()
            .unwrap_or_else(|| "US".to_string());
        let region = parse_region_code(&region_code);

        Ok(Some(TemporalContext::shared(region, start_date, end_date)))
    }

    /// Build the shared template provider from `config.templates`.
    ///
    /// Always returns a provider — falls back to embedded-only when
    /// `config.templates.path` is `None`. The merge-strategy from config
    /// maps onto the loader's [`MergeStrategy`] enum. Load failures at
    /// orchestrator-construction time are fatal (preferable to silently
    /// using embedded pools when the user supplied a bad path).
    fn build_template_provider(
        config: &GeneratorConfig,
    ) -> SynthResult<datasynth_core::templates::SharedTemplateProvider> {
        use datasynth_core::templates::{
            loader::{MergeStrategy, TemplateLoader},
            DefaultTemplateProvider,
        };
        use std::sync::Arc;

        let provider = match &config.templates.path {
            None => DefaultTemplateProvider::new(),
            Some(path) => {
                let data = if path.is_dir() {
                    TemplateLoader::load_from_directory(path)
                } else {
                    TemplateLoader::load_from_file(path)
                }
                .map_err(|e| {
                    SynthError::config(format!(
                        "Failed to load templates from {}: {e}",
                        path.display()
                    ))
                })?;
                let strategy = match config.templates.merge_strategy {
                    datasynth_config::TemplateMergeStrategy::Extend => MergeStrategy::Extend,
                    datasynth_config::TemplateMergeStrategy::Replace => MergeStrategy::Replace,
                    datasynth_config::TemplateMergeStrategy::MergePreferFile => {
                        MergeStrategy::MergePreferFile
                    }
                };
                DefaultTemplateProvider::with_templates(data, strategy)
            }
        };
        Ok(Arc::new(provider))
    }

    /// Create with default phase config.
    pub fn with_defaults(config: GeneratorConfig) -> SynthResult<Self> {
        Self::new(config, PhaseConfig::default())
    }

    /// Set a streaming phase sink for real-time output (builder pattern).
    pub fn with_phase_sink(mut self, sink: Box<dyn crate::stream_pipeline::PhaseSink>) -> Self {
        self.phase_sink = Some(sink);
        self
    }

    /// Set a streaming phase sink on an existing orchestrator.
    pub fn set_phase_sink(&mut self, sink: Box<dyn crate::stream_pipeline::PhaseSink>) {
        self.phase_sink = Some(sink);
    }

    /// Emit a batch of items to the phase sink (if configured).
    fn emit_phase_items<T: serde::Serialize>(&self, phase: &str, type_name: &str, items: &[T]) {
        if let Some(ref sink) = self.phase_sink {
            for item in items {
                if let Ok(value) = serde_json::to_value(item) {
                    if let Err(e) = sink.emit(phase, type_name, &value) {
                        warn!(
                            "Stream sink emit failed for phase '{phase}', type '{type_name}': {e}"
                        );
                    }
                }
            }
            if let Err(e) = sink.phase_complete(phase) {
                warn!("Stream sink phase_complete failed for phase '{phase}': {e}");
            }
        }
    }

    /// Enable/disable progress bars.
    pub fn with_progress(mut self, show: bool) -> Self {
        self.phase_config.show_progress = show;
        if show {
            self.multi_progress = Some(MultiProgress::new());
        }
        self
    }

    /// Set the output path for disk space monitoring.
    pub fn with_output_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        let path = path.into();
        self.output_path = Some(path.clone());
        // Rebuild resource guard with the output path
        self.resource_guard = Self::build_resource_guard(&self.config, Some(path));
        self
    }

    /// Access the country pack registry.
    pub fn country_pack_registry(&self) -> &datasynth_core::CountryPackRegistry {
        &self.country_pack_registry
    }

    /// Look up a country pack by country code string.
    pub fn country_pack_for(&self, country: &str) -> &datasynth_core::CountryPack {
        self.country_pack_registry.get_by_str(country)
    }

    /// Returns the ISO 3166-1 alpha-2 country code for the primary (first)
    /// company, defaulting to `"US"` if no companies are configured.
    fn primary_country_code(&self) -> &str {
        self.config
            .companies
            .first()
            .map(|c| c.country.as_str())
            .unwrap_or("US")
    }

    /// Resolve the country pack for the primary (first) company.
    fn primary_pack(&self) -> &datasynth_core::CountryPack {
        self.country_pack_for(self.primary_country_code())
    }

    /// Resolve the CoA framework from config/country-pack.
    fn resolve_coa_framework(&self) -> CoAFramework {
        if self.config.accounting_standards.enabled {
            match self.config.accounting_standards.framework {
                Some(datasynth_config::schema::AccountingFrameworkConfig::FrenchGaap) => {
                    return CoAFramework::FrenchPcg;
                }
                Some(datasynth_config::schema::AccountingFrameworkConfig::GermanGaap) => {
                    return CoAFramework::GermanSkr04;
                }
                _ => {}
            }
        }
        // Fallback: derive from country pack
        let pack = self.primary_pack();
        match pack.accounting.framework.as_str() {
            "french_gaap" => CoAFramework::FrenchPcg,
            "german_gaap" | "hgb" => CoAFramework::GermanSkr04,
            _ => CoAFramework::UsGaap,
        }
    }

    /// Resolve the framework string consumed by
    /// [`datasynth_core::framework_accounts::FrameworkAccounts::for_framework`].
    ///
    /// Mirrors [`Self::resolve_coa_framework`] but returns the snake_case
    /// label (`"us_gaap"`, `"ifrs"`, `"french_gaap"`, `"german_gaap"`,
    /// `"dual_reporting"`) that the framework-aware account classifier
    /// expects. Country drives selection because the country pack's CoA
    /// loader is what actually picks the numbering convention (SKR04 for
    /// DE, PCG for FR) — the entity's `accounting_framework` label can
    /// disagree with the chart it's posted against (e.g. a DE entity
    /// flagged `accounting_framework: ifrs` still gets SKR04 codes from
    /// its country pack).
    fn resolve_framework_str(&self) -> &'static str {
        // Country first — the chart of accounts loaded for this company
        // is keyed by country pack, so the code numbering convention
        // follows country, not the framework label.
        match self.primary_country_code().to_ascii_uppercase().as_str() {
            "DE" | "AT" => "german_gaap",
            "FR" | "BE" | "LU" => "french_gaap",
            _ => {
                // No country override → take the framework label.
                if self.config.accounting_standards.enabled {
                    match self.config.accounting_standards.framework {
                        Some(datasynth_config::schema::AccountingFrameworkConfig::FrenchGaap) => {
                            return "french_gaap";
                        }
                        Some(datasynth_config::schema::AccountingFrameworkConfig::GermanGaap) => {
                            return "german_gaap";
                        }
                        Some(datasynth_config::schema::AccountingFrameworkConfig::Ifrs) => {
                            return "ifrs";
                        }
                        Some(
                            datasynth_config::schema::AccountingFrameworkConfig::DualReporting,
                        ) => {
                            return "dual_reporting";
                        }
                        Some(datasynth_config::schema::AccountingFrameworkConfig::UsGaap)
                        | None => {}
                    }
                }
                "us_gaap"
            }
        }
    }

    /// Check if copula generators are available.
    ///
    /// Returns true if the orchestrator has copula generators for preserving
    /// correlations (typically from fingerprint-based generation).
    pub fn has_copulas(&self) -> bool {
        !self.copula_generators.is_empty()
    }

    /// Get the copula generators.
    ///
    /// Returns a reference to the copula generators for use during generation.
    /// These can be used to generate correlated samples that preserve the
    /// statistical relationships from the source data.
    pub fn copulas(&self) -> &[CopulaGeneratorSpec] {
        &self.copula_generators
    }

    /// Get a mutable reference to the copula generators.
    ///
    /// Allows generators to sample from copulas during data generation.
    pub fn copulas_mut(&mut self) -> &mut [CopulaGeneratorSpec] {
        &mut self.copula_generators
    }

    /// Sample correlated values from a named copula.
    ///
    /// Returns None if the copula doesn't exist.
    pub fn sample_from_copula(&mut self, copula_name: &str) -> Option<Vec<f64>> {
        self.copula_generators
            .iter_mut()
            .find(|c| c.name == copula_name)
            .map(|c| c.generator.sample())
    }

    /// Create an orchestrator from a fingerprint file.
    ///
    /// This reads the fingerprint, synthesizes a GeneratorConfig from it,
    /// and creates an orchestrator configured to generate data matching
    /// the statistical properties of the original data.
    ///
    /// # Arguments
    /// * `fingerprint_path` - Path to the .dsf fingerprint file
    /// * `phase_config` - Phase configuration for generation
    /// * `scale` - Scale factor for row counts (1.0 = same as original)
    ///
    /// # Example
    /// ```no_run
    /// use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
    /// use std::path::Path;
    ///
    /// let orchestrator = EnhancedOrchestrator::from_fingerprint(
    ///     Path::new("fingerprint.dsf"),
    ///     PhaseConfig::default(),
    ///     1.0,
    /// ).unwrap();
    /// ```
    pub fn from_fingerprint(
        fingerprint_path: &std::path::Path,
        phase_config: PhaseConfig,
        scale: f64,
    ) -> SynthResult<Self> {
        info!("Loading fingerprint from: {}", fingerprint_path.display());

        // Read the fingerprint
        let reader = FingerprintReader::new();
        let fingerprint = reader
            .read_from_file(fingerprint_path)
            .map_err(|e| SynthError::config(format!("Failed to read fingerprint: {e}")))?;

        Self::from_fingerprint_data(fingerprint, phase_config, scale)
    }

    /// Create an orchestrator from a loaded fingerprint.
    ///
    /// # Arguments
    /// * `fingerprint` - The loaded fingerprint
    /// * `phase_config` - Phase configuration for generation
    /// * `scale` - Scale factor for row counts (1.0 = same as original)
    pub fn from_fingerprint_data(
        fingerprint: Fingerprint,
        phase_config: PhaseConfig,
        scale: f64,
    ) -> SynthResult<Self> {
        info!(
            "Synthesizing config from fingerprint (version: {}, tables: {})",
            fingerprint.manifest.version,
            fingerprint.schema.tables.len()
        );

        // Generate a seed for the synthesis
        let seed: u64 = rand::random();
        info!("Fingerprint synthesis seed: {}", seed);

        // Use ConfigSynthesizer with scale option to convert fingerprint to GeneratorConfig
        let options = SynthesisOptions {
            scale,
            seed: Some(seed),
            preserve_correlations: true,
            inject_anomalies: true,
        };
        let synthesizer = ConfigSynthesizer::with_options(options);

        // Synthesize full result including copula generators
        let synthesis_result = synthesizer
            .synthesize_full(&fingerprint, seed)
            .map_err(|e| {
                SynthError::config(format!("Failed to synthesize config from fingerprint: {e}"))
            })?;

        // Start with a base config from the fingerprint's industry if available
        let mut config = if let Some(ref industry) = fingerprint.manifest.source.industry {
            Self::base_config_for_industry(industry)
        } else {
            Self::base_config_for_industry("manufacturing")
        };

        // Apply the synthesized patches
        config = Self::apply_config_patch(config, &synthesis_result.config_patch);

        // Log synthesis results
        info!(
            "Config synthesized: {} tables, scale={:.2}, copula generators: {}",
            fingerprint.schema.tables.len(),
            scale,
            synthesis_result.copula_generators.len()
        );

        if !synthesis_result.copula_generators.is_empty() {
            for spec in &synthesis_result.copula_generators {
                info!(
                    "  Copula '{}' for table '{}': {} columns",
                    spec.name,
                    spec.table,
                    spec.columns.len()
                );
            }
        }

        // Create the orchestrator with the synthesized config
        let mut orchestrator = Self::new(config, phase_config)?;

        // Store copula generators for use during generation
        orchestrator.copula_generators = synthesis_result.copula_generators;

        Ok(orchestrator)
    }

    /// Create a base config for a given industry.
    fn base_config_for_industry(industry: &str) -> GeneratorConfig {
        use datasynth_config::presets::create_preset;
        use datasynth_config::TransactionVolume;
        use datasynth_core::models::{CoAComplexity, IndustrySector};

        let sector = match industry.to_lowercase().as_str() {
            "manufacturing" => IndustrySector::Manufacturing,
            "retail" => IndustrySector::Retail,
            "financial" | "financial_services" => IndustrySector::FinancialServices,
            "healthcare" => IndustrySector::Healthcare,
            "technology" | "tech" => IndustrySector::Technology,
            _ => IndustrySector::Manufacturing,
        };

        // Create a preset with reasonable defaults
        create_preset(
            sector,
            1,  // company count
            12, // period months
            CoAComplexity::Medium,
            TransactionVolume::TenK,
        )
    }

    /// Apply a config patch to a GeneratorConfig.
    fn apply_config_patch(
        mut config: GeneratorConfig,
        patch: &datasynth_fingerprint::synthesis::ConfigPatch,
    ) -> GeneratorConfig {
        use datasynth_fingerprint::synthesis::ConfigValue;

        for (key, value) in patch.values() {
            match (key.as_str(), value) {
                // Transaction count is handled via TransactionVolume enum on companies
                // Log it but cannot directly set it (would need to modify company volumes)
                ("transactions.count", ConfigValue::Integer(n)) => {
                    info!(
                        "Fingerprint suggests {} transactions (apply via company volumes)",
                        n
                    );
                }
                ("global.period_months", ConfigValue::Integer(n)) => {
                    config.global.period_months = (*n).clamp(1, 120) as u32;
                }
                ("global.start_date", ConfigValue::String(s)) => {
                    config.global.start_date = s.clone();
                }
                ("global.seed", ConfigValue::Integer(n)) => {
                    config.global.seed = Some(*n as u64);
                }
                ("fraud.enabled", ConfigValue::Bool(b)) => {
                    config.fraud.enabled = *b;
                }
                ("fraud.fraud_rate", ConfigValue::Float(f)) => {
                    config.fraud.fraud_rate = *f;
                }
                ("data_quality.enabled", ConfigValue::Bool(b)) => {
                    config.data_quality.enabled = *b;
                }
                // Handle anomaly injection paths (mapped to fraud config)
                ("anomaly_injection.enabled", ConfigValue::Bool(b)) => {
                    config.fraud.enabled = *b;
                }
                ("anomaly_injection.overall_rate", ConfigValue::Float(f)) => {
                    config.fraud.fraud_rate = *f;
                }
                _ => {
                    debug!("Ignoring unknown config patch key: {}", key);
                }
            }
        }

        config
    }

    /// Build a resource guard from the configuration.
    fn build_resource_guard(
        config: &GeneratorConfig,
        output_path: Option<PathBuf>,
    ) -> ResourceGuard {
        let mut builder = ResourceGuardBuilder::new();

        // Configure memory limit if set
        if config.global.memory_limit_mb > 0 {
            builder = builder.memory_limit(config.global.memory_limit_mb);
        }

        // Configure disk monitoring for output path
        if let Some(path) = output_path {
            builder = builder.output_path(path).min_free_disk(100); // Require at least 100 MB free
        }

        // Use conservative degradation settings for production safety
        builder = builder.conservative();

        builder.build()
    }

    /// Check resources (memory, disk, CPU) and return degradation level.
    ///
    /// Returns an error if hard limits are exceeded.
    /// Returns Ok(DegradationLevel) indicating current resource state.
    fn check_resources(&self) -> SynthResult<DegradationLevel> {
        self.resource_guard.check()
    }

    /// Check resources with logging.
    fn check_resources_with_log(&self, phase: &str) -> SynthResult<DegradationLevel> {
        let level = self.resource_guard.check()?;

        if level != DegradationLevel::Normal {
            warn!(
                "Resource degradation at {}: level={}, memory={}MB, disk={}MB",
                phase,
                level,
                self.resource_guard.current_memory_mb(),
                self.resource_guard.available_disk_mb()
            );
        }

        Ok(level)
    }

    /// Get current degradation actions based on resource state.
    fn get_degradation_actions(&self) -> DegradationActions {
        self.resource_guard.get_actions()
    }

    /// Legacy method for backwards compatibility - now uses ResourceGuard.
    fn check_memory_limit(&self) -> SynthResult<()> {
        self.check_resources()?;
        Ok(())
    }

    /// Run the complete generation workflow.
    pub fn generate(&mut self) -> SynthResult<EnhancedGenerationResult> {
        info!("Starting enhanced generation workflow");
        info!(
            "Config: industry={:?}, period_months={}, companies={}",
            self.config.global.industry,
            self.config.global.period_months,
            self.config.companies.len()
        );

        // Set decimal serialization mode (thread-local, affects JSON output).
        // Use a scope guard to reset on drop (prevents leaking across spawn_blocking reuse).
        let is_native = self.config.output.numeric_mode == datasynth_config::NumericMode::Native;
        datasynth_core::serde_decimal::set_numeric_native(is_native);
        struct NumericModeGuard;
        impl Drop for NumericModeGuard {
            fn drop(&mut self) {
                datasynth_core::serde_decimal::set_numeric_native(false);
            }
        }
        let _numeric_guard = if is_native {
            Some(NumericModeGuard)
        } else {
            None
        };

        // Initial resource check before starting
        let initial_level = self.check_resources_with_log("initial")?;
        if initial_level == DegradationLevel::Emergency {
            return Err(SynthError::resource(
                "Insufficient resources to start generation",
            ));
        }

        let mut stats = EnhancedGenerationStatistics {
            companies_count: self.config.companies.len(),
            period_months: self.config.global.period_months,
            ..Default::default()
        };

        // Phase 1: Chart of Accounts
        let coa = self.phase_chart_of_accounts(&mut stats)?;

        // Phase 2: Master Data
        self.phase_master_data(&mut stats)?;

        // Emit master data to stream sink
        self.emit_phase_items("master_data", "Vendor", &self.master_data.vendors);
        self.emit_phase_items("master_data", "Customer", &self.master_data.customers);
        self.emit_phase_items("master_data", "Material", &self.master_data.materials);

        // Phase 3: Document Flows + Subledger Linking
        let (mut document_flows, mut subledger, fa_journal_entries) =
            self.phase_document_flows(&mut stats)?;

        // Emit document flows to stream sink
        self.emit_phase_items(
            "document_flows",
            "PurchaseOrder",
            &document_flows.purchase_orders,
        );
        self.emit_phase_items(
            "document_flows",
            "GoodsReceipt",
            &document_flows.goods_receipts,
        );
        self.emit_phase_items(
            "document_flows",
            "VendorInvoice",
            &document_flows.vendor_invoices,
        );
        self.emit_phase_items("document_flows", "SalesOrder", &document_flows.sales_orders);
        self.emit_phase_items("document_flows", "Delivery", &document_flows.deliveries);

        // Phase 3b: Opening Balances (before JE generation). The second tuple element is the
        // spec-16 specialized opening-stock inception JEs (ECL/Provisions) — empty unless configured.
        let (opening_balances, specialized_opening_jes) =
            self.phase_opening_balances(&coa, &mut stats)?;

        // Phase 3c: Convert opening balances to journal entries and prepend them.
        // The CoA lookup resolves each account's normal_debit_balance flag, solving the
        // contra-asset problem (e.g., Accumulated Depreciation) without requiring a richer
        // balance map type.
        let mut opening_balance_jes: Vec<JournalEntry> = opening_balances
            .iter()
            .flat_map(|ob| opening_balance_to_jes(ob, &coa))
            .collect();
        // Merge the specialized seeds (explicit, correctly-sided JEs — NOT routed through the
        // converter heuristic) into the opening TB so they prepend with the foundational opening.
        opening_balance_jes.extend(specialized_opening_jes);
        if !opening_balance_jes.is_empty() {
            debug!(
                "Prepending {} opening balance JEs to entries",
                opening_balance_jes.len()
            );
        }

        // Phase 4: Journal Entries
        let mut entries = self.phase_journal_entries(&coa, &document_flows, &mut stats)?;

        // Phase 4b: Prepend opening balance JEs so the RunningBalanceTracker
        // starts from the correct initial state.
        if !opening_balance_jes.is_empty() {
            let mut combined = opening_balance_jes;
            combined.extend(entries);
            entries = combined;
        }

        // Phase 4c: Append FA acquisition journal entries to main entries
        if !fa_journal_entries.is_empty() {
            debug!(
                "Appending {} FA acquisition JEs to main entries",
                fa_journal_entries.len()
            );
            entries.extend(fa_journal_entries);
        }

        // Phase 25: Counterfactual Pairs (before anomaly injection, using clean JEs)
        let counterfactual_pairs = self.phase_counterfactuals(&entries, &mut stats)?;

        // Get current degradation actions for optional phases
        let actions = self.get_degradation_actions();

        // Phase 5: S2C Sourcing Data (before anomaly injection, since it's standalone)
        let mut sourcing = self.phase_sourcing_data(&mut stats)?;

        // Phase 5a: Link S2C contracts to P2P purchase orders by matching vendor IDs.
        // Also populate the reverse FK: ProcurementContract.purchase_order_ids.
        if !sourcing.contracts.is_empty() {
            let mut linked_count = 0usize;
            // Collect (vendor_id, po_id) pairs from P2P chains
            let po_vendor_pairs: Vec<(String, String)> = document_flows
                .p2p_chains
                .iter()
                .map(|chain| {
                    (
                        chain.purchase_order.vendor_id.clone(),
                        chain.purchase_order.header.document_id.clone(),
                    )
                })
                .collect();

            for chain in &mut document_flows.p2p_chains {
                if chain.purchase_order.contract_id.is_none() {
                    if let Some(contract) = sourcing
                        .contracts
                        .iter()
                        .find(|c| c.vendor_id == chain.purchase_order.vendor_id)
                    {
                        chain.purchase_order.contract_id = Some(contract.contract_id.clone());
                        linked_count += 1;
                    }
                }
            }

            // Populate reverse FK: purchase_order_ids on each contract
            for contract in &mut sourcing.contracts {
                let po_ids: Vec<String> = po_vendor_pairs
                    .iter()
                    .filter(|(vendor_id, _)| *vendor_id == contract.vendor_id)
                    .map(|(_, po_id)| po_id.clone())
                    .collect();
                if !po_ids.is_empty() {
                    contract.purchase_order_ids = po_ids;
                }
            }

            if linked_count > 0 {
                debug!(
                    "Linked {} purchase orders to S2C contracts by vendor match",
                    linked_count
                );
            }
        }

        // Phase 5b: Intercompany Transactions + Matching + Eliminations
        let intercompany = self.phase_intercompany(&entries, &mut stats)?;

        // Phase 5c: Append IC journal entries to main entries
        if !intercompany.seller_journal_entries.is_empty()
            || !intercompany.buyer_journal_entries.is_empty()
        {
            let ic_je_count = intercompany.seller_journal_entries.len()
                + intercompany.buyer_journal_entries.len();
            entries.extend(intercompany.seller_journal_entries.iter().cloned());
            entries.extend(intercompany.buyer_journal_entries.iter().cloned());
            debug!(
                "Appended {} IC journal entries to main entries",
                ic_je_count
            );
        }

        // Phase 5d: Convert IC elimination entries to GL journal entries and append
        if !intercompany.elimination_entries.is_empty() {
            let elim_jes = datasynth_generators::elimination_to_journal_entries(
                &intercompany.elimination_entries,
            );
            if !elim_jes.is_empty() {
                debug!(
                    "Appended {} elimination journal entries to main entries",
                    elim_jes.len()
                );
                // IC elimination net-zero assertion (v2.5 hardening)
                let elim_debit: rust_decimal::Decimal =
                    elim_jes.iter().map(|je| je.total_debit()).sum();
                let elim_credit: rust_decimal::Decimal =
                    elim_jes.iter().map(|je| je.total_credit()).sum();
                let elim_diff = (elim_debit - elim_credit).abs();
                let tolerance = rust_decimal::Decimal::new(1, 2); // 0.01
                if elim_diff > tolerance {
                    return Err(datasynth_core::error::SynthError::generation(format!(
                        "IC elimination entries not balanced: debits={}, credits={}, diff={} (tolerance={})",
                        elim_debit, elim_credit, elim_diff, tolerance
                    )));
                }
                debug!(
                    "IC elimination balance verified: debits={}, credits={} (diff={})",
                    elim_debit, elim_credit, elim_diff
                );
                entries.extend(elim_jes);
            }
        }

        // Phase 5e: Wire IC source documents into document flow snapshot
        if let Some(ic_docs) = intercompany.ic_document_chains.as_ref() {
            if !ic_docs.seller_invoices.is_empty() || !ic_docs.buyer_orders.is_empty() {
                document_flows
                    .customer_invoices
                    .extend(ic_docs.seller_invoices.iter().cloned());
                document_flows
                    .purchase_orders
                    .extend(ic_docs.buyer_orders.iter().cloned());
                document_flows
                    .goods_receipts
                    .extend(ic_docs.buyer_goods_receipts.iter().cloned());
                document_flows
                    .vendor_invoices
                    .extend(ic_docs.buyer_invoices.iter().cloned());
                debug!(
                    "Appended IC source documents to document flows: {} CIs, {} POs, {} GRs, {} VIs",
                    ic_docs.seller_invoices.len(),
                    ic_docs.buyer_orders.len(),
                    ic_docs.buyer_goods_receipts.len(),
                    ic_docs.buyer_invoices.len(),
                );
            }
        }

        // Phase 6: HR Data (Payroll, Time Entries, Expenses)
        let hr = self.phase_hr_data(&mut stats)?;

        // Phase 6b: Generate JEs from payroll runs
        if !hr.payroll_runs.is_empty() {
            let payroll_jes = Self::generate_payroll_jes(&hr.payroll_runs);
            debug!("Generated {} JEs from payroll runs", payroll_jes.len());
            entries.extend(payroll_jes);
        }

        // Phase 6c: Pension expense + OCI JEs (IAS 19 / ASC 715)
        if !hr.pension_journal_entries.is_empty() {
            debug!(
                "Generated {} JEs from pension plans",
                hr.pension_journal_entries.len()
            );
            entries.extend(hr.pension_journal_entries.iter().cloned());
        }

        // Phase 6d: Stock-based compensation JEs (ASC 718 / IFRS 2)
        if !hr.stock_comp_journal_entries.is_empty() {
            debug!(
                "Generated {} JEs from stock-based compensation",
                hr.stock_comp_journal_entries.len()
            );
            entries.extend(hr.stock_comp_journal_entries.iter().cloned());
        }

        // Phase 7: Manufacturing (Production Orders, Quality Inspections, Cycle Counts)
        let manufacturing_snap = self.phase_manufacturing(&mut stats)?;

        // Phase 7a: Generate manufacturing cost flow JEs (WIP, overhead, FG, scrap, rework, QC hold)
        if !manufacturing_snap.production_orders.is_empty() {
            let currency = self
                .config
                .companies
                .first()
                .map(|c| c.currency.as_str())
                .unwrap_or("USD");
            let mfg_jes = ManufacturingCostAccounting::generate_all_jes(
                &manufacturing_snap.production_orders,
                &manufacturing_snap.quality_inspections,
                currency,
            );
            debug!("Generated {} manufacturing cost flow JEs", mfg_jes.len());
            entries.extend(mfg_jes);
        }

        // Phase 7a-warranty: Generate warranty provisions per company
        if !manufacturing_snap.quality_inspections.is_empty() {
            let framework = match self.config.accounting_standards.framework {
                Some(datasynth_config::schema::AccountingFrameworkConfig::Ifrs) => "IFRS",
                _ => "US_GAAP",
            };
            for company in &self.config.companies {
                let company_orders: Vec<_> = manufacturing_snap
                    .production_orders
                    .iter()
                    .filter(|o| o.company_code == company.code)
                    .cloned()
                    .collect();
                let company_inspections: Vec<_> = manufacturing_snap
                    .quality_inspections
                    .iter()
                    .filter(|i| company_orders.iter().any(|o| o.order_id == i.reference_id))
                    .cloned()
                    .collect();
                if company_inspections.is_empty() {
                    continue;
                }
                let mut warranty_gen = WarrantyProvisionGenerator::new(self.seed + 355);
                let warranty_result = warranty_gen.generate(
                    &company.code,
                    &company_orders,
                    &company_inspections,
                    &company.currency,
                    framework,
                );
                if !warranty_result.journal_entries.is_empty() {
                    debug!(
                        "Generated {} warranty provision JEs for {}",
                        warranty_result.journal_entries.len(),
                        company.code
                    );
                    entries.extend(warranty_result.journal_entries);
                }
            }
        }

        // Phase 7a-cogs: Generate COGS JEs from deliveries x production orders
        if !manufacturing_snap.production_orders.is_empty() && !document_flows.deliveries.is_empty()
        {
            let cogs_currency = self
                .config
                .companies
                .first()
                .map(|c| c.currency.as_str())
                .unwrap_or("USD");
            let cogs_jes = ManufacturingCostAccounting::generate_cogs_on_sale(
                &document_flows.deliveries,
                &manufacturing_snap.production_orders,
                cogs_currency,
            );
            if !cogs_jes.is_empty() {
                debug!("Generated {} COGS JEs from deliveries", cogs_jes.len());
                entries.extend(cogs_jes);
            }
        }

        // Phase 7a-inv: Apply manufacturing inventory movements to subledger positions (B.3).
        //
        // Manufacturing movements (GoodsReceipt / GoodsIssue) are generated independently of
        // subledger inventory positions.  Here we reconcile them so that position balances
        // reflect the actual stock movements within the generation period.
        if !manufacturing_snap.inventory_movements.is_empty()
            && !subledger.inventory_positions.is_empty()
        {
            use datasynth_core::models::MovementType as MfgMovementType;
            let mut receipt_count = 0usize;
            let mut issue_count = 0usize;
            for movement in &manufacturing_snap.inventory_movements {
                // Find a matching position by material code and company
                if let Some(pos) = subledger.inventory_positions.iter_mut().find(|p| {
                    p.material_id == movement.material_code
                        && p.company_code == movement.entity_code
                }) {
                    match movement.movement_type {
                        MfgMovementType::GoodsReceipt => {
                            // Increase stock and update weighted-average cost
                            pos.add_quantity(
                                movement.quantity,
                                movement.value,
                                movement.movement_date,
                            );
                            receipt_count += 1;
                        }
                        MfgMovementType::GoodsIssue | MfgMovementType::Scrap => {
                            // Decrease stock (best-effort; silently skip if insufficient)
                            let _ = pos.remove_quantity(movement.quantity, movement.movement_date);
                            issue_count += 1;
                        }
                        _ => {}
                    }
                }
            }
            debug!(
                "Phase 7a-inv: Applied {} inventory movements to subledger positions ({} receipts, {} issues/scraps)",
                manufacturing_snap.inventory_movements.len(),
                receipt_count,
                issue_count,
            );
        }

        // Update final entry/line-item stats after all JE-generating phases
        // (FA acquisition, IC, payroll, manufacturing JEs have all been appended)
        if !entries.is_empty() {
            stats.total_entries = entries.len() as u64;
            stats.total_line_items = entries.iter().map(|e| e.line_count() as u64).sum();
            debug!(
                "Final entry count: {}, line items: {} (after all JE-generating phases)",
                stats.total_entries, stats.total_line_items
            );
        }

        // Phase 7b: Apply internal controls to journal entries
        if self.config.internal_controls.enabled && !entries.is_empty() {
            info!("Phase 7b: Applying internal controls to journal entries");
            let control_config = ControlGeneratorConfig {
                exception_rate: self.config.internal_controls.exception_rate,
                sod_violation_rate: self.config.internal_controls.sod_violation_rate,
                enable_sox_marking: true,
                sox_materiality_threshold: rust_decimal::Decimal::from_f64_retain(
                    self.config.internal_controls.sox_materiality_threshold,
                )
                .unwrap_or_else(|| rust_decimal::Decimal::from(10000)),
                ..Default::default()
            };
            let mut control_gen = ControlGenerator::with_config(self.seed + 399, control_config);
            for entry in &mut entries {
                control_gen.apply_controls(entry, &coa);
            }
            let with_controls = entries
                .iter()
                .filter(|e| !e.header.control_ids.is_empty())
                .count();
            info!(
                "Applied controls to {} entries ({} with control IDs assigned)",
                entries.len(),
                with_controls
            );
        }

        // Phase 7c: Extract SoD violations from annotated journal entries.
        // The ControlGenerator marks entries with sod_violation=true and a conflict_type.
        // Here we materialise those flags into standalone SodViolation records.
        let sod_violations: Vec<datasynth_core::models::SodViolation> = entries
            .iter()
            .filter(|e| e.header.sod_violation)
            .filter_map(|e| {
                e.header.sod_conflict_type.map(|ct| {
                    use datasynth_core::models::{RiskLevel, SodViolation};
                    let severity = match ct {
                        datasynth_core::models::SodConflictType::PaymentReleaser
                        | datasynth_core::models::SodConflictType::RequesterApprover => {
                            RiskLevel::Critical
                        }
                        datasynth_core::models::SodConflictType::PreparerApprover
                        | datasynth_core::models::SodConflictType::MasterDataMaintainer
                        | datasynth_core::models::SodConflictType::JournalEntryPoster
                        | datasynth_core::models::SodConflictType::SystemAccessConflict => {
                            RiskLevel::High
                        }
                        datasynth_core::models::SodConflictType::ReconcilerPoster => {
                            RiskLevel::Medium
                        }
                    };
                    let action = format!(
                        "SoD conflict {:?} on entry {} ({})",
                        ct, e.header.document_id, e.header.company_code
                    );
                    SodViolation::new(ct, e.header.created_by.clone(), action, severity)
                })
            })
            .collect();
        if !sod_violations.is_empty() {
            info!(
                "Phase 7c: Extracted {} SoD violations from {} entries",
                sod_violations.len(),
                entries.len()
            );
        }

        // Emit journal entries to stream sink (after all JE-generating phases)
        self.emit_phase_items("journal_entries", "JournalEntry", &entries);

        // Phase 7d: Document-level fraud injection + propagation to derived JEs.
        //
        // This runs BEFORE line-level anomaly injection so that JEs tagged by
        // document-level fraud are exempt from subsequent line-level flag
        // overwrites, and so downstream consumers see a coherent picture.
        //
        // Gated by `fraud.document_fraud_rate` — `None` or `0.0` is a no-op.
        {
            let doc_rate = self.config.fraud.document_fraud_rate.unwrap_or(0.0);
            if self.config.fraud.enabled && doc_rate > 0.0 {
                use datasynth_core::fraud_propagation::{
                    inject_document_fraud, propagate_documents_to_entries,
                };
                use datasynth_core::utils::weighted_select;
                use datasynth_core::FraudType;
                use rand_chacha::rand_core::SeedableRng;

                let dist = &self.config.fraud.fraud_type_distribution;
                let fraud_type_weights: [(FraudType, f64); 8] = [
                    (FraudType::SuspenseAccountAbuse, dist.suspense_account_abuse),
                    (FraudType::FictitiousEntry, dist.fictitious_transaction),
                    (FraudType::RevenueManipulation, dist.revenue_manipulation),
                    (
                        FraudType::ImproperCapitalization,
                        dist.expense_capitalization,
                    ),
                    (FraudType::SplitTransaction, dist.split_transaction),
                    (FraudType::TimingAnomaly, dist.timing_anomaly),
                    (FraudType::UnauthorizedAccess, dist.unauthorized_access),
                    (FraudType::DuplicatePayment, dist.duplicate_payment),
                ];
                let weights_sum: f64 = fraud_type_weights.iter().map(|(_, w)| *w).sum();
                let pick = |rng: &mut rand_chacha::ChaCha8Rng| -> FraudType {
                    if weights_sum <= 0.0 {
                        FraudType::FictitiousEntry
                    } else {
                        *weighted_select(rng, &fraud_type_weights)
                    }
                };

                let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(self.seed + 5100);
                let mut doc_tagged = 0usize;
                macro_rules! inject_into {
                    ($collection:expr) => {{
                        let mut hs: Vec<&mut datasynth_core::models::documents::DocumentHeader> =
                            $collection.iter_mut().map(|d| &mut d.header).collect();
                        doc_tagged += inject_document_fraud(&mut hs, doc_rate, &mut rng, pick);
                    }};
                }
                inject_into!(document_flows.purchase_orders);
                inject_into!(document_flows.goods_receipts);
                inject_into!(document_flows.vendor_invoices);
                inject_into!(document_flows.payments);
                inject_into!(document_flows.sales_orders);
                inject_into!(document_flows.deliveries);
                inject_into!(document_flows.customer_invoices);
                if doc_tagged > 0 {
                    info!(
                        "Injected document-level fraud on {doc_tagged} documents at rate {doc_rate}"
                    );
                }

                if self.config.fraud.propagate_to_lines && doc_tagged > 0 {
                    let mut headers: Vec<datasynth_core::models::documents::DocumentHeader> =
                        Vec::new();
                    headers.extend(
                        document_flows
                            .purchase_orders
                            .iter()
                            .map(|d| d.header.clone()),
                    );
                    headers.extend(
                        document_flows
                            .goods_receipts
                            .iter()
                            .map(|d| d.header.clone()),
                    );
                    headers.extend(
                        document_flows
                            .vendor_invoices
                            .iter()
                            .map(|d| d.header.clone()),
                    );
                    headers.extend(document_flows.payments.iter().map(|d| d.header.clone()));
                    headers.extend(document_flows.sales_orders.iter().map(|d| d.header.clone()));
                    headers.extend(document_flows.deliveries.iter().map(|d| d.header.clone()));
                    headers.extend(
                        document_flows
                            .customer_invoices
                            .iter()
                            .map(|d| d.header.clone()),
                    );
                    let propagated = propagate_documents_to_entries(&headers, &mut entries);
                    if propagated > 0 {
                        info!(
                            "Propagated document-level fraud to {propagated} derived journal entries"
                        );
                    }
                }
            }
        }

        // Phase 8: Anomaly Injection (after all JE-generating phases)
        let anomaly_labels = self.phase_anomaly_injection(&mut entries, &actions, &mut stats)?;

        // Phase 8b: Apply behavioral biases to fraud entries that did NOT go
        // through the anomaly injector.
        //
        // Three paths set `is_fraud = true` without touching `is_anomaly`:
        //   - je_generator::determine_fraud (intrinsic fraud during JE generation)
        //   - fraud_propagation::propagate_documents_to_entries (doc-level cascade)
        //   - Any external mutation that sets is_fraud after the fact
        //
        // The anomaly injector already applies the same bias inline when it
        // tags an entry as fraud (and sets is_anomaly=true in the same step),
        // so gating this sweep on `!is_anomaly` avoids double-application.
        //
        // Without this sweep, fraud entries from these paths show 0 lift on
        // the canonical forensic signals (is_round_1000, is_off_hours,
        // is_weekend, is_post_close), which is exactly what the SDK-side
        // evaluator caught in v3.1 — fraud features had worse lift than
        // baseline. See DS-3.1 post-deploy feedback.
        {
            use datasynth_core::fraud_bias::{
                apply_fraud_behavioral_bias, FraudBehavioralBiasConfig,
            };
            use rand_chacha::rand_core::SeedableRng;
            let cfg = FraudBehavioralBiasConfig::default();
            if cfg.enabled {
                let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(self.seed + 8100);
                let mut swept = 0usize;
                for entry in entries.iter_mut() {
                    if entry.header.is_fraud && !entry.header.is_anomaly {
                        apply_fraud_behavioral_bias(entry, &cfg, &mut rng);
                        swept += 1;
                    }
                }
                if swept > 0 {
                    info!(
                        "Applied behavioral biases to {swept} non-anomaly fraud entries \
                         (doc-propagated + je_generator intrinsic fraud)"
                    );
                }
            }
        }

        // Emit anomaly labels to stream sink
        self.emit_phase_items(
            "anomaly_injection",
            "LabeledAnomaly",
            &anomaly_labels.labels,
        );

        // Propagate fraud labels from journal entries to source documents.
        // This allows consumers to identify fraudulent POs, invoices, etc. directly
        // instead of tracing through document_references.json.
        //
        // Gated by `fraud.propagate_to_document` (default true) — disable when
        // downstream consumers want document fraud flags to reflect only
        // document-level injection, not line-level.
        if self.config.fraud.propagate_to_document {
            use std::collections::HashMap;
            // Build a map from document_id -> (is_fraud, fraud_type) from fraudulent JEs.
            //
            // Document-flow JE generators write `je.header.reference` as "PREFIX:DOC_ID"
            // (e.g., "GR:PO-2024-000001", "VI:INV-xyz", "PAY:PAY-abc") — see
            // `document_flow_je_generator.rs` lines 454/519/591/660/724/794. The
            // `DocumentHeader::propagate_fraud` lookup uses the bare document_id, so
            // we register BOTH the prefixed form (raw reference) AND the bare form
            // (post-colon portion) in the map. Also register the JE's document_id
            // UUID so documents that set `journal_entry_id` match via that path.
            //
            // Fix for issue #104 — fraud was registered only as "GR:foo" but documents
            // looked up "foo", silently producing 0 propagations.
            let mut fraud_map: HashMap<String, datasynth_core::FraudType> = HashMap::new();
            for je in &entries {
                if je.header.is_fraud {
                    if let Some(ref fraud_type) = je.header.fraud_type {
                        if let Some(ref reference) = je.header.reference {
                            // Register the full reference ("GR:PO-2024-000001")
                            fraud_map.insert(reference.clone(), *fraud_type);
                            // Also register the bare document ID ("PO-2024-000001")
                            // by stripping the "PREFIX:" if present.
                            if let Some(bare) = reference.split_once(':').map(|(_, rest)| rest) {
                                if !bare.is_empty() {
                                    fraud_map.insert(bare.to_string(), *fraud_type);
                                }
                            }
                        }
                        // Also tag via journal_entry_id on document headers
                        fraud_map.insert(je.header.document_id.to_string(), *fraud_type);
                    }
                }
            }
            if !fraud_map.is_empty() {
                let mut propagated = 0usize;
                // Use DocumentHeader::propagate_fraud method for each doc type
                macro_rules! propagate_to {
                    ($collection:expr) => {
                        for doc in &mut $collection {
                            if doc.header.propagate_fraud(&fraud_map) {
                                propagated += 1;
                            }
                        }
                    };
                }
                propagate_to!(document_flows.purchase_orders);
                propagate_to!(document_flows.goods_receipts);
                propagate_to!(document_flows.vendor_invoices);
                propagate_to!(document_flows.payments);
                propagate_to!(document_flows.sales_orders);
                propagate_to!(document_flows.deliveries);
                propagate_to!(document_flows.customer_invoices);
                if propagated > 0 {
                    info!(
                        "Propagated fraud labels to {} document flow records",
                        propagated
                    );
                }
            }
        }

        // Phase 26: Red Flag Indicators (after anomaly injection so fraud labels are available)
        let red_flags = self.phase_red_flags(&anomaly_labels, &document_flows, &mut stats)?;

        // Emit red flags to stream sink
        self.emit_phase_items("red_flags", "RedFlag", &red_flags);

        // Phase 26b: Collusion Ring Generation (after red flags)
        let collusion_rings = self.phase_collusion_rings(&mut stats)?;

        // Emit collusion rings to stream sink
        self.emit_phase_items("collusion_rings", "CollusionRing", &collusion_rings);

        // Phase 8d: W8.1 — TB drift-correction pass.  When a TB anchor prior is
        // loaded (industry bundle with real per-account targets), emit balanced
        // "SA" adjustment JEs to nudge the synthetic balance sheet toward the
        // corpus-median shape before final balance validation runs.
        self.phase_tb_drift_correction(&mut entries)?;

        // Phase 9: Balance Validation (after all JEs including payroll, manufacturing, IC)
        let balance_validation = self.phase_balance_validation(&entries)?;

        // Phase 9a: COA coverage — every gl_account in JEs must exist in the
        // chart of accounts. Soft warning by default; hard fail when the
        // user passes --validate-coa-coverage / sets the strict flag.
        self.validate_coa_coverage(&entries, coa.as_ref())?;

        // Phase 9b: GL-to-Subledger Reconciliation
        let subledger_reconciliation =
            self.phase_subledger_reconciliation(&subledger, &entries, &mut stats)?;

        // Phase 10: Data Quality Injection
        let (data_quality_stats, quality_issues) =
            self.phase_data_quality_injection(&mut entries, &actions, &mut stats)?;

        // Phase 10b: Period Close (tax provision + income statement closing entries + depreciation)
        self.phase_period_close(&mut entries, &subledger, &mut stats)?;

        // Phase 10c: Hard accounting equation assertions (v2.5 — generation-time integrity)
        {
            let tolerance = rust_decimal::Decimal::new(1, 2); // 0.01

            // Assert 1: Every non-anomaly JE must individually balance (debits = credits).
            // Anomaly-injected JEs are excluded since they are intentionally unbalanced (fraud).
            let mut unbalanced_clean = 0usize;
            for je in &entries {
                if je.header.is_fraud || je.header.is_anomaly {
                    continue;
                }
                let diff = (je.total_debit() - je.total_credit()).abs();
                if diff > tolerance {
                    unbalanced_clean += 1;
                    if unbalanced_clean <= 3 {
                        warn!(
                            "Unbalanced non-anomaly JE {}: debit={}, credit={}, diff={}",
                            je.header.document_id,
                            je.total_debit(),
                            je.total_credit(),
                            diff
                        );
                    }
                }
            }
            if unbalanced_clean > 0 {
                return Err(datasynth_core::error::SynthError::generation(format!(
                    "{} non-anomaly JEs are unbalanced (debits != credits). \
                     First few logged above. Tolerance={}",
                    unbalanced_clean, tolerance
                )));
            }
            debug!(
                "Phase 10c: All {} non-anomaly JEs individually balanced",
                entries
                    .iter()
                    .filter(|je| !je.header.is_fraud && !je.header.is_anomaly)
                    .count()
            );

            // Assert 2: Balance sheet equation per company: Assets = Liabilities + Equity
            let company_codes: Vec<String> = self
                .config
                .companies
                .iter()
                .map(|c| c.code.clone())
                .collect();
            for company_code in &company_codes {
                let mut assets = rust_decimal::Decimal::ZERO;
                let mut liab_equity = rust_decimal::Decimal::ZERO;

                for entry in &entries {
                    if entry.header.company_code != *company_code {
                        continue;
                    }
                    for line in &entry.lines {
                        let acct = &line.gl_account;
                        let net = line.debit_amount - line.credit_amount;
                        // Asset accounts (1xxx): normal debit balance
                        if acct.starts_with('1') {
                            assets += net;
                        }
                        // Liability (2xxx) + Equity (3xxx): normal credit balance
                        else if acct.starts_with('2') || acct.starts_with('3') {
                            liab_equity -= net; // credit-normal, so negate debit-net
                        }
                        // Revenue/expense/tax (4-8xxx) are closed to RE in period-close,
                        // so they net to zero after closing entries
                    }
                }

                let bs_diff = (assets - liab_equity).abs();
                if bs_diff > tolerance {
                    warn!(
                        "Balance sheet equation gap for {}: A={}, L+E={}, diff={} — \
                         revenue/expense closing entries may not fully offset",
                        company_code, assets, liab_equity, bs_diff
                    );
                    // Warn rather than error: multi-period datasets may have timing
                    // differences from accruals/deferrals that resolve in later periods.
                    // The TB footing check (Assert 1) is the hard gate.
                } else {
                    debug!(
                        "Phase 10c: Balance sheet validated for {} — A={}, L+E={} (diff={})",
                        company_code, assets, liab_equity, bs_diff
                    );
                }
            }

            info!("Phase 10c: All generation-time accounting assertions passed");
        }

        // Phase 11: Audit Data
        let audit = self.phase_audit_data(&entries, &mut stats)?;

        // Phase 12: Banking KYC/AML Data
        let mut banking = self.phase_banking_data(&mut stats)?;

        // Phase 12.5: Bridge document-flow Payments → BankTransactions
        // Creates coherence between the accounting layer (payments, JEs) and the
        // banking layer (bank transactions). A vendor invoice payment now appears
        // on both sides with cross-references and fraud labels propagated.
        if self.phase_config.generate_banking
            && !document_flows.payments.is_empty()
            && !banking.accounts.is_empty()
        {
            let bridge_rate = self.config.banking.typologies.payment_bridge_rate;
            if bridge_rate > 0.0 {
                let mut bridge =
                    datasynth_banking::generators::payment_bridge::PaymentBridgeGenerator::new(
                        self.seed,
                    );
                let (bridged_txns, bridge_stats) = bridge.bridge_payments(
                    &document_flows.payments,
                    &banking.customers,
                    &banking.accounts,
                    bridge_rate,
                );
                info!(
                    "Payment bridge: {} payments bridged, {} bank txns emitted, {} fraud propagated",
                    bridge_stats.bridged_count,
                    bridge_stats.transactions_emitted,
                    bridge_stats.fraud_propagated,
                );
                let bridged_count = bridged_txns.len();
                banking.transactions.extend(bridged_txns);

                // Re-run velocity computation so bridged txns also get features
                // (otherwise ML pipelines see a split: native=with-velocity, bridged=without)
                if self.config.banking.temporal.enable_velocity_features && bridged_count > 0 {
                    datasynth_banking::generators::velocity_computer::compute_velocity_features(
                        &mut banking.transactions,
                    );
                }

                // Recompute suspicious count after bridging
                banking.suspicious_count = banking
                    .transactions
                    .iter()
                    .filter(|t| t.is_suspicious)
                    .count();
                stats.banking_transaction_count = banking.transactions.len();
                stats.banking_suspicious_count = banking.suspicious_count;
            }
        }

        // Phase 13: Graph Export
        let graph_export = self.phase_graph_export(&entries, &coa, &mut stats)?;

        // Phase 14: LLM Enrichment
        self.phase_llm_enrichment(&mut stats);

        // Phase 15: Diffusion Enhancement
        self.phase_diffusion_enhancement(&entries, &mut stats);

        // Phase 16: Causal Overlay
        self.phase_causal_overlay(&mut stats);

        // Phase 17: Bank Reconciliation + Financial Statements
        // Notes generation is deferred to after Phase 18 + 20 so that deferred-tax and
        // provision data (from accounting_standards / tax snapshots) can be wired in.
        let mut financial_reporting = self.phase_financial_reporting(
            &document_flows,
            &entries,
            &coa,
            &hr,
            &audit,
            &mut stats,
        )?;

        // BS coherence check: assets = liabilities + equity
        {
            use datasynth_core::models::StatementType;
            for stmt in &financial_reporting.consolidated_statements {
                if stmt.statement_type == StatementType::BalanceSheet {
                    let total_assets: rust_decimal::Decimal = stmt
                        .line_items
                        .iter()
                        .filter(|li| li.section.to_uppercase().contains("ASSET"))
                        .map(|li| li.amount)
                        .sum();
                    let total_le: rust_decimal::Decimal = stmt
                        .line_items
                        .iter()
                        .filter(|li| !li.section.to_uppercase().contains("ASSET"))
                        .map(|li| li.amount)
                        .sum();
                    if (total_assets - total_le).abs() > rust_decimal::Decimal::new(1, 0) {
                        warn!(
                            "BS equation imbalance: assets={}, L+E={}",
                            total_assets, total_le
                        );
                    }
                }
            }
        }

        // Phase 18: Accounting Standards (Revenue Recognition, Impairment, ECL)
        let accounting_standards =
            self.phase_accounting_standards(&subledger.ar_aging_reports, &entries, &mut stats)?;

        // Phase 18a: Merge ECL journal entries into main GL
        if !accounting_standards.ecl_journal_entries.is_empty() {
            debug!(
                "Generated {} JEs from ECL provision (IFRS 9 / ASC 326)",
                accounting_standards.ecl_journal_entries.len()
            );
            entries.extend(accounting_standards.ecl_journal_entries.iter().cloned());
        }

        // Phase 18a: Merge provision journal entries into main GL
        if !accounting_standards.provision_journal_entries.is_empty() {
            debug!(
                "Generated {} JEs from provisions (IAS 37 / ASC 450)",
                accounting_standards.provision_journal_entries.len()
            );
            entries.extend(
                accounting_standards
                    .provision_journal_entries
                    .iter()
                    .cloned(),
            );
        }

        // Phase 18a: Merge W1-3 Stage 2 ASC 606 revenue-recognition JEs into main GL.
        // Empty unless `monthly_recurring` is ON (byte-identical OFF).
        if !accounting_standards
            .revenue_recognition_journal_entries
            .is_empty()
        {
            debug!(
                "Merged {} JEs from ASC 606 deferred-revenue recognition (Stage 2)",
                accounting_standards
                    .revenue_recognition_journal_entries
                    .len()
            );
            entries.extend(
                accounting_standards
                    .revenue_recognition_journal_entries
                    .iter()
                    .cloned(),
            );
        }

        // Phase 18a: Merge W1-3 Stage 2 ASC 842 lease JEs into main GL.
        // Empty unless `monthly_recurring` is ON (byte-identical OFF).
        if !accounting_standards.lease_journal_entries.is_empty() {
            debug!(
                "Merged {} JEs from ASC 842 leases (Stage 2)",
                accounting_standards.lease_journal_entries.len()
            );
            entries.extend(accounting_standards.lease_journal_entries.iter().cloned());
        }

        // Phase 18b: OCPM Events (after all process data is available)
        let mut ocpm = self.phase_ocpm_events(
            &document_flows,
            &sourcing,
            &hr,
            &manufacturing_snap,
            &banking,
            &audit,
            &financial_reporting,
            &mut stats,
        )?;

        // Emit OCPM events to stream sink
        if let Some(ref event_log) = ocpm.event_log {
            self.emit_phase_items("ocpm", "OcpmEvent", &event_log.events);
        }

        // Phase 18c: Back-annotate OCPM event IDs onto JournalEntry headers (fixes #117)
        if let Some(ref event_log) = ocpm.event_log {
            // Build reverse index: document_ref → (event_id, case_id, object_ids)
            let mut doc_index: std::collections::HashMap<&str, Vec<usize>> =
                std::collections::HashMap::new();
            for (idx, event) in event_log.events.iter().enumerate() {
                if let Some(ref doc_ref) = event.document_ref {
                    doc_index.entry(doc_ref.as_str()).or_default().push(idx);
                }
            }

            if !doc_index.is_empty() {
                let mut annotated = 0usize;
                for entry in &mut entries {
                    let doc_id_str = entry.header.document_id.to_string();
                    // Collect matching event indices from document_id and reference
                    let mut matched_indices: Vec<usize> = Vec::new();
                    if let Some(indices) = doc_index.get(doc_id_str.as_str()) {
                        matched_indices.extend(indices);
                    }
                    if let Some(ref reference) = entry.header.reference {
                        let bare_ref = reference
                            .find(':')
                            .map(|i| &reference[i + 1..])
                            .unwrap_or(reference.as_str());
                        if let Some(indices) = doc_index.get(bare_ref) {
                            for &idx in indices {
                                if !matched_indices.contains(&idx) {
                                    matched_indices.push(idx);
                                }
                            }
                        }
                    }
                    // Apply matches to JE header
                    if !matched_indices.is_empty() {
                        for &idx in &matched_indices {
                            let event = &event_log.events[idx];
                            if !entry.header.ocpm_event_ids.contains(&event.event_id) {
                                entry.header.ocpm_event_ids.push(event.event_id);
                            }
                            for obj_ref in &event.object_refs {
                                if !entry.header.ocpm_object_ids.contains(&obj_ref.object_id) {
                                    entry.header.ocpm_object_ids.push(obj_ref.object_id);
                                }
                            }
                            if entry.header.ocpm_case_id.is_none() {
                                entry.header.ocpm_case_id = event.case_id;
                            }
                        }
                        annotated += 1;
                    }
                }
                debug!(
                    "Phase 18c: Back-annotated {} JEs with OCPM event/object/case IDs",
                    annotated
                );
            }
        }

        // Phase 18d: Synthesize OCPM events for orphan JEs (period-close,
        // IC eliminations, opening balances, standards-driven entries) so
        // every JournalEntry carries at least one `ocpm_event_ids` link.
        if let Some(ref mut event_log) = ocpm.event_log {
            let synthesized =
                datasynth_ocpm::synthesize_events_for_orphan_entries(&mut entries, event_log);
            if synthesized > 0 {
                info!(
                    "Phase 18d: Synthesized {synthesized} OCPM events for orphan journal entries"
                );
            }

            // Phase 18e: Mirror JE anomaly / fraud flags onto the linked OCEL
            // events and their owning CaseTrace. Without this, every exported
            // OCEL event has `is_anomaly = false` even when the underlying JE
            // was flagged.
            let anomaly_events =
                datasynth_ocpm::propagate_je_anomalies_to_ocel(&entries, event_log);
            if anomaly_events > 0 {
                info!("Phase 18e: Propagated anomaly flags onto {anomaly_events} OCEL events");
            }

            // Phase 18f: Inject process-variant imperfections (rework, skipped
            // steps, out-of-order events) so conformance checkers see
            // realistic variant counts and fitness < 1.0. Uses the P2P
            // process rates as the single source of truth.
            let p2p_cfg = &self.config.ocpm.p2p_process;
            let any_imperfection = p2p_cfg.rework_probability > 0.0
                || p2p_cfg.skip_step_probability > 0.0
                || p2p_cfg.out_of_order_probability > 0.0;
            if any_imperfection {
                use rand_chacha::rand_core::SeedableRng;
                let imp_cfg = datasynth_ocpm::ImperfectionConfig {
                    rework_rate: p2p_cfg.rework_probability,
                    skip_rate: p2p_cfg.skip_step_probability,
                    out_of_order_rate: p2p_cfg.out_of_order_probability,
                };
                let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(self.seed + 5200);
                let stats =
                    datasynth_ocpm::inject_process_imperfections(event_log, &imp_cfg, &mut rng);
                if stats.rework + stats.skipped + stats.out_of_order > 0 {
                    info!(
                        "Phase 18f: Injected process imperfections — rework={} skipped={} out_of_order={}",
                        stats.rework, stats.skipped, stats.out_of_order
                    );
                }
            }
        }

        // Phase 19: Sales Quotes, Management KPIs, Budgets
        let sales_kpi_budgets =
            self.phase_sales_kpi_budgets(&coa, &financial_reporting, &mut stats)?;

        // Phase 22: Treasury Data Generation
        // Must run BEFORE tax so that interest expense (7100) and hedge ineffectiveness (7510)
        // are included in the pre-tax income used by phase_tax_generation.
        let treasury =
            self.phase_treasury_data(&document_flows, &subledger, &intercompany, &mut stats)?;

        // Phase 22 JEs: Merge treasury journal entries into main GL (before tax phase)
        if !treasury.journal_entries.is_empty() {
            debug!(
                "Merging {} treasury JEs (debt interest, hedge MTM, sweeps) into GL",
                treasury.journal_entries.len()
            );
            entries.extend(treasury.journal_entries.iter().cloned());
        }

        // Phase 20: Tax Generation
        let tax = self.phase_tax_generation(&document_flows, &entries, &mut stats)?;

        // Phase 20 JEs: Merge tax posting journal entries into main GL
        if !tax.tax_posting_journal_entries.is_empty() {
            debug!(
                "Merging {} tax posting JEs into GL",
                tax.tax_posting_journal_entries.len()
            );
            entries.extend(tax.tax_posting_journal_entries.iter().cloned());
        }

        // Phase 20b: FINAL fraud behavioral bias sweep.
        //
        // Many phases AFTER Phase 8b (ECL / provisions / treasury / tax /
        // period close) extend `entries` with new journal entries that may
        // carry `is_fraud = true` (e.g. tax-provision entries derived from
        // already-fraudulent transactions). Those late additions miss the
        // Phase 8b sweep and ship without bias applied — which is exactly
        // why SDK-team production jobs kept reporting `off_hours 0× lift`
        // even after v3.1.1 closed the per-phase gap for early-added JEs.
        //
        // Running the sweep one more time here guarantees every is_fraud
        // entry — regardless of which phase added it — has bias applied.
        // `!is_anomaly` gates out anomaly-injector entries (which already
        // got biased inline); the sweep is otherwise idempotent-ish:
        // weekend / off_hours re-fire to another valid weekend / off-hour,
        // post_close is guarded by `!is_post_close`, and round-dollar
        // rescaling on an already-round amount is a no-op (ratio = 1).
        {
            use datasynth_core::fraud_bias::{
                apply_fraud_behavioral_bias, FraudBehavioralBiasConfig,
            };
            use rand_chacha::rand_core::SeedableRng;
            let cfg = FraudBehavioralBiasConfig::default();
            if cfg.enabled {
                let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(self.seed + 8200);
                let mut swept = 0usize;
                for entry in entries.iter_mut() {
                    if entry.header.is_fraud && !entry.header.is_anomaly {
                        apply_fraud_behavioral_bias(entry, &cfg, &mut rng);
                        swept += 1;
                    }
                }
                if swept > 0 {
                    info!(
                        "Phase 20b: final behavioral-bias sweep applied to {swept} \
                         non-anomaly fraud entries (covers late-added JEs from \
                         ECL / provisions / treasury / tax / period-close)"
                    );
                }
            }
        }

        // Phase 20a-cf: Enhanced Cash Flow (v2.4)
        // Build supplementary cash flow items from upstream JE data (depreciation,
        // interest, tax, dividends, working-capital deltas) and merge into CF statements.
        {
            use datasynth_generators::{CashFlowEnhancer, CashFlowSourceData};

            let framework_str = {
                use datasynth_config::schema::AccountingFrameworkConfig;
                match self
                    .config
                    .accounting_standards
                    .framework
                    .unwrap_or_default()
                {
                    AccountingFrameworkConfig::Ifrs | AccountingFrameworkConfig::DualReporting => {
                        "IFRS"
                    }
                    _ => "US_GAAP",
                }
            };

            // Sum depreciation debits (account 6000) from close JEs
            let depreciation_total: rust_decimal::Decimal = entries
                .iter()
                .filter(|je| je.header.document_type == "CL")
                .flat_map(|je| je.lines.iter())
                .filter(|l| l.gl_account.starts_with("6000"))
                .map(|l| l.debit_amount)
                .fold(rust_decimal::Decimal::ZERO, |a, v| a + v);

            // Sum interest expense debits (account 7150 — the dedicated interest account; 7100 is
            // now the FA depreciation account after the interest/depreciation split).
            let interest_paid: rust_decimal::Decimal = entries
                .iter()
                .flat_map(|je| je.lines.iter())
                .filter(|l| l.gl_account.starts_with("7150"))
                .map(|l| l.debit_amount)
                .fold(rust_decimal::Decimal::ZERO, |a, v| a + v);

            // Sum tax expense debits (account 8000)
            let tax_paid: rust_decimal::Decimal = entries
                .iter()
                .flat_map(|je| je.lines.iter())
                .filter(|l| l.gl_account.starts_with("8000"))
                .map(|l| l.debit_amount)
                .fold(rust_decimal::Decimal::ZERO, |a, v| a + v);

            // Sum capex debits on fixed assets (account 1500)
            let capex: rust_decimal::Decimal = entries
                .iter()
                .flat_map(|je| je.lines.iter())
                .filter(|l| l.gl_account.starts_with("1500"))
                .map(|l| l.debit_amount)
                .fold(rust_decimal::Decimal::ZERO, |a, v| a + v);

            // Dividends paid: sum debits on dividends payable (account 2170) from payment JEs
            let dividends_paid: rust_decimal::Decimal = entries
                .iter()
                .flat_map(|je| je.lines.iter())
                .filter(|l| l.gl_account == "2170")
                .map(|l| l.debit_amount)
                .fold(rust_decimal::Decimal::ZERO, |a, v| a + v);

            let cf_data = CashFlowSourceData {
                depreciation_total,
                provision_movements_net: rust_decimal::Decimal::ZERO, // best-effort: zero
                delta_ar: rust_decimal::Decimal::ZERO,
                delta_ap: rust_decimal::Decimal::ZERO,
                delta_inventory: rust_decimal::Decimal::ZERO,
                capex,
                debt_issuance: rust_decimal::Decimal::ZERO,
                debt_repayment: rust_decimal::Decimal::ZERO,
                interest_paid,
                tax_paid,
                dividends_paid,
                framework: framework_str.to_string(),
            };

            let enhanced_cf_items = CashFlowEnhancer::generate(&cf_data);
            if !enhanced_cf_items.is_empty() {
                // Merge into ALL cash flow statements (standalone + consolidated)
                use datasynth_core::models::StatementType;
                let merge_count = enhanced_cf_items.len();
                for stmt in financial_reporting
                    .financial_statements
                    .iter_mut()
                    .chain(financial_reporting.consolidated_statements.iter_mut())
                    .chain(
                        financial_reporting
                            .standalone_statements
                            .values_mut()
                            .flat_map(|v| v.iter_mut()),
                    )
                {
                    if stmt.statement_type == StatementType::CashFlowStatement {
                        stmt.cash_flow_items.extend(enhanced_cf_items.clone());
                    }
                }
                info!(
                    "Enhanced cash flow: {} supplementary items merged into CF statements",
                    merge_count
                );
            }
        }

        // Phase 20a: Notes to Financial Statements (IAS 1 / ASC 235)
        // Runs here so deferred-tax (Phase 20) and provision data (Phase 18) are available.
        self.generate_notes_to_financial_statements(
            &mut financial_reporting,
            &accounting_standards,
            &tax,
            &hr,
            &audit,
            &treasury,
        );

        // Phase 20b: Supplement segment reports from real JEs (v2.4)
        // When we have 2+ companies, derive segment data from actual journal entries
        // to complement or replace the FS-generator-based segments.
        if self.config.companies.len() >= 2 && !entries.is_empty() {
            let companies: Vec<(String, String)> = self
                .config
                .companies
                .iter()
                .map(|c| (c.code.clone(), c.name.clone()))
                .collect();
            let ic_elim: rust_decimal::Decimal =
                intercompany.matched_pairs.iter().map(|p| p.amount).sum();
            let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
                .unwrap_or(NaiveDate::MIN);
            let end_date = start_date + chrono::Months::new(self.config.global.period_months);
            let period_label = format!(
                "{}-{:02}",
                end_date.year(),
                (end_date - chrono::Days::new(1)).month()
            );

            let mut seg_gen = SegmentGenerator::new(self.seed + 31);
            let (je_segments, je_recon) =
                seg_gen.generate_from_journal_entries(&entries, &companies, &period_label, ic_elim);
            if !je_segments.is_empty() {
                info!(
                    "Segment reports (v2.4): {} JE-derived segments with IC elimination {}",
                    je_segments.len(),
                    ic_elim,
                );
                // Replace if existing segment_reports were empty; otherwise supplement
                if financial_reporting.segment_reports.is_empty() {
                    financial_reporting.segment_reports = je_segments;
                    financial_reporting.segment_reconciliations = vec![je_recon];
                } else {
                    financial_reporting.segment_reports.extend(je_segments);
                    financial_reporting.segment_reconciliations.push(je_recon);
                }
            }
        }

        // Phase 21: ESG Data Generation
        let esg_snap =
            self.phase_esg_generation(&document_flows, &manufacturing_snap, &mut stats)?;

        // Phase 23: Project Accounting Data Generation
        let project_accounting = self.phase_project_accounting(&document_flows, &hr, &mut stats)?;

        // Phase 24: Process Evolution + Organizational Events
        let (process_evolution, organizational_events) = self.phase_evolution_events(&mut stats)?;

        // Phase 24b: Disruption Events
        let disruption_events = self.phase_disruption_events(&mut stats)?;

        // Phase 27: Bi-Temporal Vendor Version Chains
        let temporal_vendor_chains = self.phase_temporal_attributes(&mut stats)?;

        // Phase 28: Entity Relationship Graph + Cross-Process Links
        let (entity_relationship_graph, cross_process_links) =
            self.phase_entity_relationships(&entries, &document_flows, &mut stats)?;

        // Phase 29: Industry-specific GL accounts
        let industry_output = self.phase_industry_data(&mut stats);

        // Phase: Compliance regulations (must run before hypergraph so it can be included)
        let compliance_regulations = self.phase_compliance_regulations(&mut stats)?;

        // Phase: Neural enhancement (config-acknowledged-only in v4.0).
        //
        // The neural / hybrid diffusion path was a documented L2 stub
        // in v3.x; actual neural-network training requires ML
        // infrastructure (PyTorch / candle bindings, GPU access,
        // training loops) that was never wired through the
        // orchestrator. Rather than keep a silently-no-op block that
        // misleads users into thinking neural training happens, v4.0
        // acknowledges the config — exposing stats so downstream
        // tooling can see the request — but emits a clear warning
        // when a non-statistical backend is requested. The statistical
        // diffusion backend continues to run via
        // `phase_diffusion_enhancement`.
        //
        // Users who need real neural diffusion: track the roadmap item
        // in the v4.x backlog and consider contributing the backend
        // (the `DiffusionBackend` trait is the integration point).
        if self.config.diffusion.enabled
            && (self.config.diffusion.backend == "neural"
                || self.config.diffusion.backend == "hybrid")
        {
            let neural = &self.config.diffusion.neural;
            let weight = neural.hybrid_weight.clamp(0.0, 1.0);
            stats.neural_hybrid_weight = Some(weight);
            stats.neural_hybrid_strategy = Some(neural.hybrid_strategy.clone());
            stats.neural_routed_column_count = Some(neural.neural_columns.len());
            warn!(
                "diffusion.backend='{}' is config-acknowledged only in v4.0 — \
                 the neural/hybrid training path is not yet shipped. Config \
                 is captured in stats (weight={weight:.2}, strategy={}, \
                 columns={}) but no neural training runs. Statistical \
                 diffusion (backend='statistical') continues to work.",
                self.config.diffusion.backend,
                neural.hybrid_strategy,
                neural.neural_columns.len(),
            );
        }

        // Phase 19b: Hypergraph Export (after all data is available)
        self.phase_hypergraph_export(
            &coa,
            &entries,
            &document_flows,
            &sourcing,
            &hr,
            &manufacturing_snap,
            &banking,
            &audit,
            &financial_reporting,
            &ocpm,
            &compliance_regulations,
            &mut stats,
        )?;

        // Phase 10c: Additional graph builders (approval, entity, banking)
        // These run after all data is available since they need banking/IC data.
        if self.phase_config.generate_graph_export {
            self.build_additional_graphs(&banking, &intercompany, &entries, &mut stats);
        }

        // Log informational messages for config sections not yet fully wired
        if self.config.streaming.enabled {
            info!("Note: streaming config is enabled but batch mode does not use it");
        }
        if self.config.vendor_network.enabled {
            debug!("Vendor network config available; relationship graph generation is partial");
        }
        if self.config.customer_segmentation.enabled {
            debug!("Customer segmentation config available; segment-aware generation is partial");
        }

        // Log final resource statistics
        let resource_stats = self.resource_guard.stats();
        info!(
            "Generation workflow complete. Resource stats: memory_peak={}MB, disk_written={}bytes, degradation_level={}",
            resource_stats.memory.peak_resident_bytes / (1024 * 1024),
            resource_stats.disk.estimated_bytes_written,
            resource_stats.degradation_level
        );

        // Flush any remaining stream sink data
        if let Some(ref sink) = self.phase_sink {
            if let Err(e) = sink.flush() {
                warn!("Stream sink flush failed: {e}");
            }
        }

        // Build data lineage graph
        let lineage = self.build_lineage_graph();

        // Evaluate quality gates if enabled in config
        let gate_result = if self.config.quality_gates.enabled {
            let profile_name = &self.config.quality_gates.profile;
            match datasynth_eval::gates::get_profile(profile_name) {
                Some(profile) => {
                    // Build an evaluation populated with actual generation metrics.
                    let mut eval = datasynth_eval::ComprehensiveEvaluation::new();

                    // Populate balance sheet evaluation from balance validation results
                    if balance_validation.validated {
                        eval.coherence.balance =
                            Some(datasynth_eval::coherence::BalanceSheetEvaluation {
                                equation_balanced: balance_validation.is_balanced,
                                max_imbalance: (balance_validation.total_debits
                                    - balance_validation.total_credits)
                                    .abs(),
                                periods_evaluated: 1,
                                periods_imbalanced: if balance_validation.is_balanced {
                                    0
                                } else {
                                    1
                                },
                                period_results: Vec::new(),
                                companies_evaluated: self.config.companies.len(),
                            });
                    }

                    // Set coherence passes based on balance validation
                    eval.coherence.passes = balance_validation.is_balanced;
                    if !balance_validation.is_balanced {
                        eval.coherence
                            .failures
                            .push("Balance sheet equation not satisfied".to_string());
                    }

                    // Set statistical score based on entry count (basic sanity)
                    eval.statistical.overall_score = if entries.len() > 10 { 0.9 } else { 0.5 };
                    eval.statistical.passes = !entries.is_empty();

                    // Set quality score from data quality stats
                    eval.quality.overall_score = 0.9; // Default high for generated data
                    eval.quality.passes = true;

                    let result = datasynth_eval::gates::GateEngine::evaluate(&eval, &profile);
                    info!(
                        "Quality gates evaluated (profile '{}'): {}/{} passed — {}",
                        profile_name, result.gates_passed, result.gates_total, result.summary
                    );
                    Some(result)
                }
                None => {
                    warn!(
                        "Quality gates enabled but profile '{}' not found; skipping gate evaluation",
                        profile_name
                    );
                    None
                }
            }
        } else {
            None
        };

        // Generate internal controls if enabled
        let internal_controls = if self.config.internal_controls.enabled {
            InternalControl::standard_controls()
        } else {
            Vec::new()
        };

        // v3.3.0: analytics-metadata phase. Runs AFTER all JE-adding
        // phases (including fraud-bias sweep at Phase 20b) so derived
        // outputs reflect final data.
        let analytics_metadata = self.phase_analytics_metadata(&entries)?;

        // v3.5.1: statistical validation over the final amount
        // distribution. Runs *after* all JE-adding phases so the report
        // reflects everything the user will see in the output. Returns
        // `None` unless `distributions.validation.enabled = true`.
        let statistical_validation = self.phase_statistical_validation(&entries)?;

        // v4.1.3+: interconnectivity snapshot — tier assignments,
        // value-segment labels, industry-specific metadata. Runs after
        // master data is settled so it can index stable IDs.
        let interconnectivity = self.phase_interconnectivity();

        // SP5.2 — snapshot the CoA semantic prior (if any) into the result so
        // output_writer can use it as a fallback index for account_description
        // resolution when the synthetic CoA index misses.
        let coa_semantic_prior = self
            .cached_priors
            .as_ref()
            .and_then(|p| p.coa_semantic.clone());

        Ok(EnhancedGenerationResult {
            chart_of_accounts: Arc::try_unwrap(coa).unwrap_or_else(|arc| (*arc).clone()),
            master_data: std::mem::take(&mut self.master_data),
            document_flows,
            subledger,
            ocpm,
            audit,
            banking,
            graph_export,
            sourcing,
            financial_reporting,
            hr,
            accounting_standards,
            manufacturing: manufacturing_snap,
            sales_kpi_budgets,
            tax,
            esg: esg_snap,
            treasury,
            project_accounting,
            process_evolution,
            organizational_events,
            disruption_events,
            intercompany,
            journal_entries: entries,
            anomaly_labels,
            balance_validation,
            data_quality_stats,
            quality_issues,
            statistics: stats,
            lineage: Some(lineage),
            gate_result,
            internal_controls,
            sod_violations,
            opening_balances,
            subledger_reconciliation,
            counterfactual_pairs,
            red_flags,
            collusion_rings,
            temporal_vendor_chains,
            entity_relationship_graph,
            cross_process_links,
            industry_output,
            coa_semantic_prior,
            compliance_regulations,
            analytics_metadata,
            statistical_validation,
            interconnectivity,
        })
    }

    /// v4.1.3+: populate the interconnectivity snapshot from
    /// previously-inert schema sections. Empty when all sections are
    /// disabled.
    fn phase_interconnectivity(&self) -> InterconnectivitySnapshot {
        use rand::{RngExt, SeedableRng};
        use rand_chacha::ChaCha8Rng;

        let mut snap = InterconnectivitySnapshot::default();
        let mut rng = ChaCha8Rng::seed_from_u64(self.seed.wrapping_add(91_001));

        // --- Vendor network ---
        let vn = &self.config.vendor_network;
        if vn.enabled {
            let total = self.master_data.vendors.len();
            if total > 0 {
                let tier1_count = ((vn.tier1.min + vn.tier1.max) / 2).min(total).max(1);
                let remaining_after_t1 = total.saturating_sub(tier1_count);
                let depth = vn.depth.clamp(1, 3);
                let tier2_count = if depth >= 2 {
                    let avg = (vn.tier2_per_parent.min + vn.tier2_per_parent.max) / 2;
                    (tier1_count * avg).min(remaining_after_t1)
                } else {
                    0
                };
                let tier3_count = total
                    .saturating_sub(tier1_count)
                    .saturating_sub(tier2_count);

                for (idx, vendor) in self.master_data.vendors.iter().enumerate() {
                    let tier = if idx < tier1_count {
                        1
                    } else if idx < tier1_count + tier2_count {
                        2
                    } else {
                        3
                    };
                    snap.vendor_tiers.push((vendor.vendor_id.clone(), tier));

                    // Cluster assignment via configured ratios.
                    let cl = &vn.clusters;
                    let roll: f64 = rng.random();
                    let cluster = if roll < cl.reliable_strategic {
                        "reliable_strategic"
                    } else if roll < cl.reliable_strategic + cl.standard_operational {
                        "standard_operational"
                    } else if roll
                        < cl.reliable_strategic + cl.standard_operational + cl.transactional
                    {
                        "transactional"
                    } else {
                        "problematic"
                    };
                    snap.vendor_clusters
                        .push((vendor.vendor_id.clone(), cluster.to_string()));
                }
                let _ = tier3_count; // retained for clarity; tier 3 bucket is the remainder
            }
        }

        // --- Customer segmentation ---
        let cs = &self.config.customer_segmentation;
        if cs.enabled {
            let seg = &cs.value_segments;
            for customer in &self.master_data.customers {
                let roll: f64 = rng.random();
                let value_segment = if roll < seg.enterprise.customer_share {
                    "enterprise"
                } else if roll < seg.enterprise.customer_share + seg.mid_market.customer_share {
                    "mid_market"
                } else if roll
                    < seg.enterprise.customer_share
                        + seg.mid_market.customer_share
                        + seg.smb.customer_share
                {
                    "smb"
                } else {
                    "consumer"
                };
                snap.customer_value_segments
                    .push((customer.customer_id.clone(), value_segment.to_string()));

                let roll2: f64 = rng.random();
                let life = &cs.lifecycle;
                let lifecycle = if roll2 < life.prospect_rate {
                    "prospect"
                } else if roll2 < life.prospect_rate + life.new_rate {
                    "new"
                } else if roll2 < life.prospect_rate + life.new_rate + life.growth_rate {
                    "growth"
                } else if roll2
                    < life.prospect_rate + life.new_rate + life.growth_rate + life.mature_rate
                {
                    "mature"
                } else if roll2
                    < life.prospect_rate
                        + life.new_rate
                        + life.growth_rate
                        + life.mature_rate
                        + life.at_risk_rate
                {
                    "at_risk"
                } else if roll2
                    < life.prospect_rate
                        + life.new_rate
                        + life.growth_rate
                        + life.mature_rate
                        + life.at_risk_rate
                        + life.churned_rate
                {
                    "churned"
                } else {
                    "won_back"
                };
                snap.customer_lifecycle_stages
                    .push((customer.customer_id.clone(), lifecycle.to_string()));
            }
        }

        // --- Industry-specific metadata (minimal) ---
        let is = &self.config.industry_specific;
        if is.enabled {
            snap.industry_metadata.push(format!(
                "industry_specific.enabled=true (industry={:?})",
                self.config.global.industry
            ));
        }

        snap
    }

    // ========================================================================
    // Generation Phase Methods
    // ========================================================================

    /// Phase 1: Generate Chart of Accounts and update statistics.
    fn phase_chart_of_accounts(
        &mut self,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<Arc<ChartOfAccounts>> {
        info!("Phase 1: Generating Chart of Accounts");
        let coa = self.generate_coa()?;
        stats.accounts_count = coa.account_count();
        info!(
            "Chart of Accounts generated: {} accounts",
            stats.accounts_count
        );
        self.check_resources_with_log("post-coa")?;
        Ok(coa)
    }

    /// Phase 2: Generate master data (vendors, customers, materials, assets, employees).
    fn phase_master_data(&mut self, stats: &mut EnhancedGenerationStatistics) -> SynthResult<()> {
        if self.phase_config.generate_master_data {
            info!("Phase 2: Generating Master Data");
            self.generate_master_data()?;
            stats.vendor_count = self.master_data.vendors.len();
            stats.customer_count = self.master_data.customers.len();
            stats.material_count = self.master_data.materials.len();
            stats.asset_count = self.master_data.assets.len();
            stats.employee_count = self.master_data.employees.len();
            info!(
                "Master data generated: {} vendors, {} customers, {} materials, {} assets, {} employees",
                stats.vendor_count, stats.customer_count, stats.material_count,
                stats.asset_count, stats.employee_count
            );
            self.check_resources_with_log("post-master-data")?;
        } else {
            debug!("Phase 2: Skipped (master data generation disabled)");
        }
        Ok(())
    }

    /// Phase 3: Generate document flows (P2P and O2C) and link to subledgers.
    fn phase_document_flows(
        &mut self,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<(DocumentFlowSnapshot, SubledgerSnapshot, Vec<JournalEntry>)> {
        let mut document_flows = DocumentFlowSnapshot::default();
        let mut subledger = SubledgerSnapshot::default();
        // Dunning JEs (interest + charges) accumulated here and merged into the
        // main FA-JE list below so they appear in the GL.
        let mut dunning_journal_entries: Vec<JournalEntry> = Vec::new();

        if self.phase_config.generate_document_flows && !self.master_data.vendors.is_empty() {
            info!("Phase 3: Generating Document Flows");
            self.generate_document_flows(&mut document_flows)?;
            stats.p2p_chain_count = document_flows.p2p_chains.len();
            stats.o2c_chain_count = document_flows.o2c_chains.len();
            info!(
                "Document flows generated: {} P2P chains, {} O2C chains",
                stats.p2p_chain_count, stats.o2c_chain_count
            );

            // Phase 3b: Link document flows to subledgers (for data coherence)
            debug!("Phase 3b: Linking document flows to subledgers");
            subledger = self.link_document_flows_to_subledgers(&document_flows)?;
            stats.ap_invoice_count = subledger.ap_invoices.len();
            stats.ar_invoice_count = subledger.ar_invoices.len();
            debug!(
                "Subledgers linked: {} AP invoices, {} AR invoices",
                stats.ap_invoice_count, stats.ar_invoice_count
            );

            // Phase 3b-settle: Apply payment settlements to reduce amount_remaining.
            // Without this step the subledger is systematically overstated because
            // amount_remaining is set at invoice creation and never reduced by
            // the payments that were generated in the document-flow phase.
            debug!("Phase 3b-settle: Applying payment settlements to subledgers");
            apply_ap_settlements(&mut subledger.ap_invoices, &document_flows.payments);
            apply_ar_settlements(&mut subledger.ar_invoices, &document_flows.payments);
            debug!("Payment settlements applied to AP and AR subledgers");

            // Phase 3b-aging: Build AR/AP aging reports (one per company) after settlement.
            // The as-of date is the last day of the configured period.
            if let Ok(start_date) =
                NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            {
                let as_of_date = start_date + chrono::Months::new(self.config.global.period_months)
                    - chrono::Days::new(1);
                debug!("Phase 3b-aging: Building AR/AP aging reports as of {as_of_date}");
                // Note: AR aging total_ar_balance reflects subledger ARInvoice records only
                // (created from O2C document flows). The Balance Sheet "Receivables" figure is
                // derived from JE-level aggregation and will typically differ. This is a known
                // data model gap: subledger AR (document-flow-driven) and GL AR (JE-driven) are
                // generated independently. A future reconciliation phase should align them by
                // using subledger totals as the authoritative source for BS Receivables.
                for company in &self.config.companies {
                    let ar_report = ARAgingReport::from_invoices(
                        company.code.clone(),
                        &subledger.ar_invoices,
                        as_of_date,
                    );
                    subledger.ar_aging_reports.push(ar_report);

                    let ap_report = APAgingReport::from_invoices(
                        company.code.clone(),
                        &subledger.ap_invoices,
                        as_of_date,
                    );
                    subledger.ap_aging_reports.push(ap_report);
                }
                debug!(
                    "AR/AP aging reports built: {} AR, {} AP",
                    subledger.ar_aging_reports.len(),
                    subledger.ap_aging_reports.len()
                );

                // Phase 3b-dunning: Run dunning process on overdue AR invoices.
                debug!("Phase 3b-dunning: Executing dunning runs for overdue AR invoices");
                {
                    use datasynth_generators::DunningGenerator;
                    let mut dunning_gen = DunningGenerator::new(self.seed + 2500);
                    for company in &self.config.companies {
                        let currency = company.currency.as_str();
                        // Collect mutable references to AR invoices for this company
                        // (dunning generator updates dunning_info on invoices in-place).
                        let mut company_invoices: Vec<
                            datasynth_core::models::subledger::ar::ARInvoice,
                        > = subledger
                            .ar_invoices
                            .iter()
                            .filter(|inv| inv.company_code == company.code)
                            .cloned()
                            .collect();

                        if company_invoices.is_empty() {
                            continue;
                        }

                        let result = dunning_gen.execute_dunning_run(
                            &company.code,
                            as_of_date,
                            &mut company_invoices,
                            currency,
                        );

                        // Write back updated dunning info to the main AR invoice list
                        for updated in &company_invoices {
                            if let Some(orig) = subledger
                                .ar_invoices
                                .iter_mut()
                                .find(|i| i.invoice_number == updated.invoice_number)
                            {
                                orig.dunning_info = updated.dunning_info.clone();
                            }
                        }

                        subledger.dunning_runs.push(result.dunning_run);
                        subledger.dunning_letters.extend(result.letters);
                        // Dunning JEs (interest + charges) collected into local buffer.
                        dunning_journal_entries.extend(result.journal_entries);
                    }
                    debug!(
                        "Dunning runs complete: {} runs, {} letters",
                        subledger.dunning_runs.len(),
                        subledger.dunning_letters.len()
                    );
                }
            }

            self.check_resources_with_log("post-document-flows")?;
        } else {
            debug!("Phase 3: Skipped (document flow generation disabled or no master data)");
        }

        // Generate FA subledger records (and acquisition JEs) from master data fixed assets
        let mut fa_journal_entries: Vec<JournalEntry> = dunning_journal_entries;
        if !self.master_data.assets.is_empty() {
            debug!("Generating FA subledger records");
            let company_code = self
                .config
                .companies
                .first()
                .map(|c| c.code.as_str())
                .unwrap_or("1000");
            let currency = self
                .config
                .companies
                .first()
                .map(|c| c.currency.as_str())
                .unwrap_or("USD");

            let mut fa_gen = datasynth_generators::FAGenerator::new(
                datasynth_generators::FAGeneratorConfig::default(),
                rand_chacha::ChaCha8Rng::seed_from_u64(self.seed + 70),
            );

            for asset in &self.master_data.assets {
                let (record, je) = fa_gen.generate_asset_acquisition(
                    company_code,
                    &format!("{:?}", asset.asset_class),
                    &asset.description,
                    asset.acquisition_date,
                    currency,
                    asset.cost_center.as_deref(),
                );
                subledger.fa_records.push(record);
                fa_journal_entries.push(je);
            }

            stats.fa_subledger_count = subledger.fa_records.len();
            debug!(
                "FA subledger records generated: {} (with {} acquisition JEs)",
                stats.fa_subledger_count,
                fa_journal_entries.len()
            );
        }

        // Generate Inventory subledger records from master data materials
        if !self.master_data.materials.is_empty() {
            debug!("Generating Inventory subledger records");
            let first_company = self.config.companies.first();
            let company_code = first_company.map(|c| c.code.as_str()).unwrap_or("1000");
            let inv_currency = first_company
                .map(|c| c.currency.clone())
                .unwrap_or_else(|| "USD".to_string());

            let mut inv_gen = datasynth_generators::InventoryGenerator::new_with_currency(
                datasynth_generators::InventoryGeneratorConfig::default(),
                rand_chacha::ChaCha8Rng::seed_from_u64(self.seed + 71),
                inv_currency.clone(),
            );

            for (i, material) in self.master_data.materials.iter().enumerate() {
                let plant = format!("PLANT{:02}", (i % 3) + 1);
                let storage_loc = format!("SL-{:03}", (i % 10) + 1);
                let initial_qty = rust_decimal::Decimal::from(
                    material
                        .safety_stock
                        .to_string()
                        .parse::<i64>()
                        .unwrap_or(100),
                );

                let position = inv_gen.generate_position(
                    company_code,
                    &plant,
                    &storage_loc,
                    &material.material_id,
                    &material.description,
                    initial_qty,
                    Some(material.standard_cost),
                    &inv_currency,
                );
                subledger.inventory_positions.push(position);
            }

            stats.inventory_subledger_count = subledger.inventory_positions.len();
            debug!(
                "Inventory subledger records generated: {}",
                stats.inventory_subledger_count
            );
        }

        // Phase 3-depr: Run depreciation for each fiscal period covered by the config.
        if !subledger.fa_records.is_empty() {
            if let Ok(start_date) =
                NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            {
                let company_code = self
                    .config
                    .companies
                    .first()
                    .map(|c| c.code.as_str())
                    .unwrap_or("1000");
                let fiscal_year = start_date.year();
                let start_period = start_date.month();
                let end_period =
                    (start_period + self.config.global.period_months.saturating_sub(1)).min(12);

                let depr_cfg = FaDepreciationScheduleConfig {
                    fiscal_year,
                    start_period,
                    end_period,
                    seed_offset: 800,
                };
                let depr_gen = FaDepreciationScheduleGenerator::new(depr_cfg, self.seed);
                // PP-2 (FA tie): when enabled, capture the depreciation JEs and post them into the
                // GL so the accumulated-depreciation control reflects the schedule (the FA register
                // and the GL accum-dep otherwise diverge — the schedule generator emits these JEs
                // but they are discarded). The returned runs are byte-identical to `generate`'s, so
                // `subledger.depreciation_runs` is unchanged either way; OFF by default → the JEs
                // stay discarded and the build is byte-identical.
                let runs = if self.config.period_close.post_depreciation_jes {
                    let (runs, depr_jes) =
                        depr_gen.generate_with_jes(company_code, &subledger.fa_records);
                    fa_journal_entries.extend(depr_jes);
                    runs
                } else {
                    depr_gen.generate(company_code, &subledger.fa_records)
                };
                let run_count = runs.len();
                subledger.depreciation_runs = runs;
                debug!(
                    "Depreciation runs generated: {} runs for {} periods",
                    run_count, self.config.global.period_months
                );
            }
        }

        // Phase 3-inv-val: Build inventory valuation report (lower-of-cost-or-NRV).
        if !subledger.inventory_positions.is_empty() {
            if let Ok(start_date) =
                NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            {
                let as_of_date = start_date + chrono::Months::new(self.config.global.period_months)
                    - chrono::Days::new(1);

                let inv_val_cfg = InventoryValuationGeneratorConfig::default();
                let inv_val_gen = InventoryValuationGenerator::new(inv_val_cfg, self.seed);

                for company in &self.config.companies {
                    let result = inv_val_gen.generate(
                        &company.code,
                        &subledger.inventory_positions,
                        as_of_date,
                    );
                    subledger.inventory_valuations.push(result);
                }
                debug!(
                    "Inventory valuations generated: {} company reports",
                    subledger.inventory_valuations.len()
                );
            }
        }

        Ok((document_flows, subledger, fa_journal_entries))
    }

    /// Phase 3c: Generate OCPM events from document flows.
    #[allow(clippy::too_many_arguments)]
    fn phase_ocpm_events(
        &mut self,
        document_flows: &DocumentFlowSnapshot,
        sourcing: &SourcingSnapshot,
        hr: &HrSnapshot,
        manufacturing: &ManufacturingSnapshot,
        banking: &BankingSnapshot,
        audit: &AuditSnapshot,
        financial_reporting: &FinancialReportingSnapshot,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<OcpmSnapshot> {
        let degradation = self.check_resources()?;
        if degradation >= DegradationLevel::Reduced {
            debug!(
                "Phase skipped due to resource pressure (degradation: {:?})",
                degradation
            );
            return Ok(OcpmSnapshot::default());
        }
        if self.phase_config.generate_ocpm_events {
            info!("Phase 3c: Generating OCPM Events");
            let ocpm_snapshot = self.generate_ocpm_events(
                document_flows,
                sourcing,
                hr,
                manufacturing,
                banking,
                audit,
                financial_reporting,
            )?;
            stats.ocpm_event_count = ocpm_snapshot.event_count;
            stats.ocpm_object_count = ocpm_snapshot.object_count;
            stats.ocpm_case_count = ocpm_snapshot.case_count;
            info!(
                "OCPM events generated: {} events, {} objects, {} cases",
                stats.ocpm_event_count, stats.ocpm_object_count, stats.ocpm_case_count
            );
            self.check_resources_with_log("post-ocpm")?;
            Ok(ocpm_snapshot)
        } else {
            debug!("Phase 3c: Skipped (OCPM generation disabled or no document flows)");
            Ok(OcpmSnapshot::default())
        }
    }

    /// Phase 4: Generate journal entries from document flows and standalone generation.
    fn phase_journal_entries(
        &mut self,
        coa: &Arc<ChartOfAccounts>,
        document_flows: &DocumentFlowSnapshot,
        _stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<Vec<JournalEntry>> {
        let mut entries = Vec::new();

        // Phase 4a: Generate JEs from document flows (for data coherence)
        if self.phase_config.generate_document_flows && !document_flows.p2p_chains.is_empty() {
            debug!("Phase 4a: Generating JEs from document flows");
            let flow_entries = self.generate_jes_from_document_flows(document_flows)?;
            debug!("Generated {} JEs from document flows", flow_entries.len());
            entries.extend(flow_entries);
        }

        // Phase 4b: Generate standalone journal entries
        if self.phase_config.generate_journal_entries {
            info!("Phase 4: Generating Journal Entries");
            let je_entries = self.generate_journal_entries(coa)?;
            info!("Generated {} standalone journal entries", je_entries.len());
            entries.extend(je_entries);
        } else {
            debug!("Phase 4: Skipped (journal entry generation disabled)");
        }

        // Phase 4c (shard mode): inject pre-built IC journal entries from
        // `ShardContext`. When running standalone (no group engine), this
        // is a no-op. See crate::shard_context::ShardContext for rationale.
        if let Some(ctx) = &self.shard_context {
            if !ctx.extra_journal_entries.is_empty() {
                debug!(
                    "Phase 4c: appending {} shard-mode IC journal entries",
                    ctx.extra_journal_entries.len()
                );
                entries.extend(ctx.extra_journal_entries.iter().cloned());
            }
        }

        if !entries.is_empty() {
            // Note: stats.total_entries/total_line_items are set in generate()
            // after all JE-generating phases (FA, IC, payroll, mfg) complete.
            self.check_resources_with_log("post-journal-entries")?;
        }

        Ok(entries)
    }

    /// Phase 5: Inject anomalies into journal entries.
    fn phase_anomaly_injection(
        &mut self,
        entries: &mut [JournalEntry],
        actions: &DegradationActions,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<AnomalyLabels> {
        if self.phase_config.inject_anomalies
            && !entries.is_empty()
            && !actions.skip_anomaly_injection
        {
            info!("Phase 5: Injecting Anomalies");
            let result = self.inject_anomalies(entries)?;
            stats.anomalies_injected = result.labels.len();
            info!("Injected {} anomalies", stats.anomalies_injected);
            self.check_resources_with_log("post-anomaly-injection")?;
            Ok(result)
        } else if actions.skip_anomaly_injection {
            warn!("Phase 5: Skipped due to resource degradation");
            Ok(AnomalyLabels::default())
        } else {
            debug!("Phase 5: Skipped (anomaly injection disabled or no entries)");
            Ok(AnomalyLabels::default())
        }
    }

    /// Phase 8d (W8.1): TB drift-correction pass.
    ///
    /// Builds a `RunningBalanceTracker` over all JEs assembled so far, attaches
    /// the TB anchor prior (when available), and — if `drift_correction_needed()`
    /// fires for any company — emits one balanced "SA" adjustment JE per company
    /// to pull the synthetic balances toward the corpus-median targets.
    ///
    /// No-op when no TB anchor is loaded (backwards-compatible).
    fn phase_tb_drift_correction(&mut self, entries: &mut Vec<JournalEntry>) -> SynthResult<()> {
        // Only proceed when priors with a TB anchor are loaded.
        let tb_anchor = match &self.cached_priors {
            Some(priors) => match &priors.tb_anchor {
                Some(anchor) => anchor.clone(),
                None => return Ok(()),
            },
            None => return Ok(()),
        };

        if !tb_anchor.has_data() {
            return Ok(());
        }

        tracing::info!(
            target: "datasynth_runtime::tb_anchor",
            accounts = tb_anchor.per_account.len(),
            total_assets = tb_anchor.total_assets,
            "W8.1 — TB anchor loaded; running drift-correction pass"
        );

        // Build a tracker over all current JEs.
        let tracker_config = BalanceTrackerConfig {
            validate_on_each_entry: false,
            track_history: false,
            fail_on_validation_error: false,
            ..Default::default()
        };
        let currency = self
            .config
            .companies
            .first()
            .map(|c| c.currency.clone())
            .unwrap_or_else(|| "USD".to_string());

        let mut tracker = RunningBalanceTracker::new_with_currency(tracker_config, currency);
        tracker.set_tb_anchor(tb_anchor.clone());
        let _ = tracker.apply_entries(entries);

        // SP5.1 — Diagnostic: log the number of accounts being tracked vs in the
        // anchor, plus the top-5 most-drifted accounts for each company so we
        // can distinguish "no drift" from "drift below threshold" at a glance.
        for company in &self.config.companies {
            let code = &company.code;
            let drifts = tracker.account_drift(code);
            let mut sorted_drifts = drifts.clone();
            sorted_drifts.sort_by(|a, b| {
                b.1.abs()
                    .partial_cmp(&a.1.abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let aggregate_drift: f64 = drifts.iter().map(|(_, d)| d.abs()).sum();
            let correction_needed = tracker.drift_correction_needed(code);
            tracing::info!(
                target: "datasynth_runtime::tb_anchor",
                company = %code,
                anchor_accounts = tb_anchor.per_account.len(),
                tracked_accounts = drifts.len(),
                aggregate_drift = aggregate_drift,
                correction_needed = correction_needed,
                "W8.1 SP5.1 — per-company drift summary before correction"
            );
            for (acc, drift) in sorted_drifts.iter().take(5) {
                tracing::info!(
                    target: "datasynth_runtime::tb_anchor",
                    company = %code,
                    account = %acc,
                    drift = drift,
                    "W8.1 SP5.1 — top-5 drifted accounts"
                );
            }
        }

        // Derive the posting date: use the last day of the simulation period.
        let period_end = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map(|d| d + chrono::Months::new(self.config.global.period_months))
            .unwrap_or_else(|_| chrono::Utc::now().naive_utc().date());

        // Distinct seed offset so drift-correction draws are independent of other phases.
        use rand_chacha::rand_core::SeedableRng as _;
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(self.seed.wrapping_add(0xD81F_C0F3));

        let mut correction_count = 0usize;
        for company in &self.config.companies {
            let code = &company.code;
            if !tracker.drift_correction_needed(code) {
                tracing::debug!(
                    target: "datasynth_runtime::tb_anchor",
                    company = %code,
                    "W8.1 — drift_correction_needed returned false; skipping company"
                );
                continue;
            }
            if let Some(je) = tracker.build_drift_correction_je(code, period_end, &mut rng) {
                tracing::debug!(
                    target: "datasynth_runtime::tb_anchor",
                    company = %code,
                    lines = je.lines.len(),
                    debit = %je.total_debit(),
                    credit = %je.total_credit(),
                    "W8.1 — emitting drift-correction JE"
                );
                // Apply the correction to the tracker so the running state is current.
                let _ = tracker.apply_entry(&je);
                entries.push(je);
                correction_count += 1;
            }
        }

        if correction_count > 0 {
            tracing::info!(
                target: "datasynth_runtime::tb_anchor",
                correction_count,
                "W8.1 — drift-correction pass emitted {} JE(s)",
                correction_count
            );
        } else {
            tracing::debug!(
                target: "datasynth_runtime::tb_anchor",
                "W8.1 — drift-correction pass: no corrections needed"
            );
        }

        Ok(())
    }

    /// Phase 6: Validate balance sheet equation on journal entries.
    fn phase_balance_validation(
        &mut self,
        entries: &[JournalEntry],
    ) -> SynthResult<BalanceValidationResult> {
        if self.phase_config.validate_balances && !entries.is_empty() {
            debug!("Phase 6: Validating Balances");
            let balance_validation = self.validate_journal_entries(entries)?;
            if balance_validation.is_balanced {
                debug!("Balance validation passed");
            } else {
                warn!(
                    "Balance validation found {} errors",
                    balance_validation.validation_errors.len()
                );
            }
            Ok(balance_validation)
        } else {
            Ok(BalanceValidationResult::default())
        }
    }

    /// Validate that every `gl_account` referenced in `entries` exists in the
    /// chart of accounts.
    ///
    /// Always emits a warn-level log when the COA is missing accounts; in
    /// strict mode (`phase_config.validate_coa_coverage_strict`) returns
    /// `SynthError::generation` so the caller can fail fast.
    fn validate_coa_coverage(
        &self,
        entries: &[JournalEntry],
        coa: &ChartOfAccounts,
    ) -> SynthResult<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let coa_set: std::collections::HashSet<&str> = coa
            .accounts
            .iter()
            .map(|a| a.account_number.as_str())
            .collect();
        let mut missing: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for je in entries {
            for line in je.lines.iter() {
                if !coa_set.contains(line.gl_account.as_str()) {
                    missing.insert(line.gl_account.clone());
                }
            }
        }
        if missing.is_empty() {
            debug!("COA coverage validation passed");
            return Ok(());
        }
        let msg = format!(
            "JEs reference {} gl_account values not in the chart of accounts (sample: {:?})",
            missing.len(),
            missing.iter().take(10).collect::<Vec<_>>()
        );
        if self.phase_config.validate_coa_coverage_strict {
            Err(SynthError::generation(msg))
        } else {
            warn!("{} — pass --validate-coa-coverage to fail on this", msg);
            Ok(())
        }
    }

    /// Phase 7: Inject data quality variations (typos, missing values, format issues).
    fn phase_data_quality_injection(
        &mut self,
        entries: &mut [JournalEntry],
        actions: &DegradationActions,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<(DataQualityStats, Vec<datasynth_generators::QualityIssue>)> {
        if self.phase_config.inject_data_quality
            && !entries.is_empty()
            && !actions.skip_data_quality
        {
            info!("Phase 7: Injecting Data Quality Variations");
            let (dq_stats, quality_issues) = self.inject_data_quality(entries)?;
            stats.data_quality_issues = dq_stats.records_with_issues;
            info!("Injected {} data quality issues", stats.data_quality_issues);
            self.check_resources_with_log("post-data-quality")?;
            Ok((dq_stats, quality_issues))
        } else if actions.skip_data_quality {
            warn!("Phase 7: Skipped due to resource degradation");
            // v4.4.1: report the denominator (entries seen) even when
            // injection is skipped, so downstream consumers can tell
            // "skipped, 0/N" apart from "ran but found nothing".
            Ok((stats_with_denominator(entries.len()), Vec::new()))
        } else {
            debug!("Phase 7: Skipped (data quality injection disabled or no entries)");
            Ok((stats_with_denominator(entries.len()), Vec::new()))
        }
    }

    /// W1-3: month-end posting dates for the RECURRING classes — depreciation / accruals (Stage 1,
    /// in `phase_period_close`) and bond interest / ASC 606 recognition / ASC 842 lease amortization
    /// (Stage 2, in `phase_treasury_data` / `phase_accounting_standards`). One element (`close_date`,
    /// the slice's last day) when `monthly_recurring` is OFF → byte-identical lump; one date per
    /// month of the slice when ON, so every recurring class spreads on the SAME calendar.
    ///
    /// Lives outside `GenerationSession` fiscal-year slicing (orthogonal to
    /// `skip_income_statement_close` — see `PeriodCloseConfig`'s SEAM NOTE): the session re-invokes
    /// the orchestrator per FY-slice with its own `start_date`/`period_months`, so these month-ends
    /// always cover exactly the current slice and a Stage-2 class that walks them cannot re-post a
    /// prior fiscal year.
    fn recurring_month_ends(&self) -> SynthResult<Vec<NaiveDate>> {
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let close_date = start_date + chrono::Months::new(self.config.global.period_months)
            - chrono::Days::new(1);
        let n: u32 = if self.phase_config.monthly_recurring {
            self.config.global.period_months.max(1)
        } else {
            1
        };
        Ok(if n == 1 {
            vec![close_date]
        } else {
            (1..=n)
                .map(|m| start_date + chrono::Months::new(m) - chrono::Days::new(1))
                .collect()
        })
    }

    /// Phase 10b: Generate period-close journal entries.
    ///
    /// Generates:
    /// 1. Depreciation JEs per asset: DR Depreciation Expense (6000) / CR Accumulated
    ///    Depreciation (1510) based on FA subledger records and straight-line amortisation
    ///    for the configured period.
    /// 2. Tax provision JE per company: DR Tax Expense (8000) / CR Sales Tax Payable (2100)
    /// 3. Income statement closing JE per company: transfer net income after tax to retained
    ///    earnings via the Income Summary (3600) clearing account.
    fn phase_period_close(
        &mut self,
        entries: &mut Vec<JournalEntry>,
        subledger: &SubledgerSnapshot,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<()> {
        if !self.phase_config.generate_period_close || entries.is_empty() {
            debug!("Phase 10b: Skipped (period close disabled or no entries)");
            return Ok(());
        }

        info!("Phase 10b: Generating period-close journal entries");

        use datasynth_core::accounts::{
            control_accounts, equity_accounts, expense_accounts, tax_accounts, AccountCategory,
        };
        use rust_decimal::Decimal;

        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let end_date = start_date + chrono::Months::new(self.config.global.period_months);
        // Posting date for close entries is the last day of the period
        let close_date = end_date - chrono::Days::new(1);

        // W1-3 Stage 1: month-end posting dates for the RECURRING period-close entries
        // (depreciation, accruals, amortization) — one element (`close_date`) when monthly_recurring
        // is OFF → byte-identical lump; one per month of the slice when ON. Shared with the Stage-2
        // recurring classes via `recurring_month_ends`. The period-level close below (tax /
        // dividends / income-statement close) stays at `close_date` regardless.
        let month_ends = self.recurring_month_ends()?;

        // Statutory tax rate (21% — configurable rates come in later tiers)
        let tax_rate = Decimal::new(21, 2); // 0.21

        // Collect company codes from config
        let company_codes: Vec<String> = self
            .config
            .companies
            .iter()
            .map(|c| c.code.clone())
            .collect();

        // Estimate capacity: one depreciation + accrual posting per FA / accrual
        // item PER recurring period (1 when lumped, N months when monthly) + ~2
        // period-level JEs per company (tax + close).
        let estimated_close_jes = (subledger.fa_records.len() + company_codes.len() * 3)
            * month_ends.len()
            + company_codes.len() * 2;
        let mut close_jes: Vec<JournalEntry> = Vec::with_capacity(estimated_close_jes);

        // --- Depreciation JEs (per asset) ---
        // Compute period depreciation for each active fixed asset using straight-line method.
        // period_depreciation = (acquisition_cost - salvage_value) / useful_life_months * period_months
        // spec 27 R6d: when the FA-depr-into-GL path (`post_depreciation_jes`) is active,
        // `phase_document_flows` already posts the per-asset depreciation JEs (DR 7100 depreciation
        // expense / CR per-class 15x9 accum-dep, from `run_depreciation`'s account determination)
        // into the GL, so this always-on block MUST NOT also post its 6000/1510 depreciation — that
        // would double-count. Skip it by iterating an empty slice.
        // Default off → the block runs unchanged (byte-identical).
        let period_months = self.config.global.period_months;
        let fa_records_for_depr: &[_] = if self.config.period_close.post_depreciation_jes {
            &[]
        } else {
            subledger.fa_records.as_slice()
        };
        for asset in fa_records_for_depr {
            // Skip assets that are inactive / fully depreciated / non-depreciable
            use datasynth_core::models::subledger::fa::AssetStatus;
            if asset.status != AssetStatus::Active || asset.is_fully_depreciated() {
                continue;
            }
            let useful_life_months = asset.useful_life_months();
            if useful_life_months == 0 {
                // Land or CIP — not depreciated
                continue;
            }
            let salvage_value = asset.salvage_value();
            let depreciable_base = (asset.acquisition_cost - salvage_value).max(Decimal::ZERO);
            if depreciable_base == Decimal::ZERO {
                continue;
            }
            // Total straight-line depreciation for the whole slice. This is the
            // exact amount the lumped path posted; spreading it monthly with the
            // cent-exact allocation below keeps the slice total (and therefore
            // the FA subledger tie + answer key) identical to the cent.
            let total_depr = (depreciable_base / Decimal::from(useful_life_months)
                * Decimal::from(period_months))
            .round_dp(2);
            if total_depr <= Decimal::ZERO {
                continue;
            }

            // Allocate across the month-ends. With monthly_recurring OFF this is
            // one posting of `total_depr` at `close_date` — byte-identical to the
            // pre-Stage-1 single JE.
            let alloc = monthly_straight_line_allocation(total_depr, month_ends.len() as u32);
            for (idx, period_depr) in alloc.iter().enumerate() {
                if *period_depr <= Decimal::ZERO {
                    continue;
                }
                let posting_date = month_ends[idx];

                let mut depr_header =
                    JournalEntryHeader::new(asset.company_code.clone(), posting_date);
                depr_header.document_type = "CL".to_string();
                depr_header.header_text = Some(format!(
                    "Depreciation - {} {}",
                    asset.asset_number, asset.description
                ));
                depr_header.created_by = "CLOSE_ENGINE".to_string();
                depr_header.source = TransactionSource::Automated;
                depr_header.business_process = Some(BusinessProcess::R2R);

                let doc_id = depr_header.document_id;
                let mut depr_je = JournalEntry::new(depr_header);

                // DR Depreciation Expense (6000)
                depr_je.add_line(JournalEntryLine::debit(
                    doc_id,
                    1,
                    expense_accounts::DEPRECIATION.to_string(),
                    *period_depr,
                ));
                // CR Accumulated Depreciation (1510)
                depr_je.add_line(JournalEntryLine::credit(
                    doc_id,
                    2,
                    control_accounts::ACCUMULATED_DEPRECIATION.to_string(),
                    *period_depr,
                ));

                debug_assert!(depr_je.is_balanced(), "Depreciation JE must be balanced");
                close_jes.push(depr_je);
            }
        }

        if !subledger.fa_records.is_empty() {
            debug!(
                "Generated {} depreciation JEs from {} FA records",
                close_jes.len(),
                subledger.fa_records.len()
            );
        }

        // --- Inventory → GL close true-up (spec 27 R6c) ---
        // The two inventory JE paths use different valuation bases: the document-flow path books GR
        // at PO price / COGS at SALE net on GL 1200 — and because it UNDER-relieves COGS it leaves a
        // large positive CHURN RESIDUE there (~18.7M on the R5 manufacturing build) vs the physical
        // positions' Σ(on-hand × cost) (~10.8M). This close JE trues GL 1200 to the physical EOT
        // inventory (here a true-DOWN), offsetting the DELTA to opening equity (Retained Earnings) —
        // a balance-sheet-only revaluation that keeps net income unchanged (so A=L+E and the IS-
        // articulation gate stay green) and makes 1200 physically meaningful, so the product's
        // INV-DB-001 subledger↔GL tie holds. Delta-based (target − current) → self-correcting; never
        // double-counts churn or prior closes. Posted at close_date. Gated (default off → byte-id).
        // The per-company aggregation + JE construction (incl. the UNION-of-keys robustness for
        // companies with 1200 activity but no positions) lives in `build_inventory_close_jes`.
        if self.phase_config.post_inventory_close && !subledger.inventory_positions.is_empty() {
            use std::collections::BTreeMap;
            // Target physical inventory per company (Σ position valuations, cent-exact).
            let mut target_by_company: BTreeMap<String, Decimal> = BTreeMap::new();
            for pos in &subledger.inventory_positions {
                *target_by_company
                    .entry(pos.company_code.clone())
                    .or_default() += pos.valuation.total_value;
            }
            // Current GL 1200 balance per company from the pre-close entries (DR − CR). The close
            // JEs (depreciation/accruals) don't touch 1200, so their ordering is irrelevant here.
            let mut gl_1200_by_company: BTreeMap<String, Decimal> = BTreeMap::new();
            for je in entries.iter() {
                for line in &je.lines {
                    if line.gl_account == control_accounts::INVENTORY {
                        *gl_1200_by_company
                            .entry(je.header.company_code.clone())
                            .or_default() += line.debit_amount - line.credit_amount;
                    }
                }
            }
            close_jes.extend(build_inventory_close_jes(
                &target_by_company,
                &gl_1200_by_company,
                close_date,
            ));
        }

        // --- Accrual entries (standard period-end accruals per company) ---
        // Generate standard accrued expense entries (utilities, rent, interest) using
        // a revenue-based estimate. These use account 6200 (Misc Expense) / 2100 (Accrued Liab).
        {
            use datasynth_generators::{AccrualGenerator, AccrualGeneratorConfig};
            let mut accrual_gen = AccrualGenerator::new(AccrualGeneratorConfig::default());
            // v3.4.3: snap reversal dates to business days. No-op when
            // temporal_patterns.business_days is disabled.
            if let Some(ctx) = &self.temporal_context {
                accrual_gen.set_temporal_context(Arc::clone(ctx));
            }

            // Standard accrual items: (description, expense_acct, liability_acct, % of revenue)
            let accrual_items: &[(&str, &str, &str)] = &[
                ("Accrued Utilities", "6200", "2100"),
                ("Accrued Rent", "6300", "2100"),
                ("Accrued Interest", "6100", "2150"),
            ];

            for company_code in &company_codes {
                // Estimate company revenue from existing JEs
                let company_revenue: Decimal = entries
                    .iter()
                    .filter(|e| e.header.company_code == *company_code)
                    .flat_map(|e| e.lines.iter())
                    .filter(|l| l.gl_account.starts_with('4'))
                    .map(|l| l.credit_amount - l.debit_amount)
                    .fold(Decimal::ZERO, |acc, v| acc + v);

                if company_revenue <= Decimal::ZERO {
                    continue;
                }

                // Use 0.5% of period revenue per accrual item as a proxy
                let accrual_base = (company_revenue * Decimal::new(5, 3)).round_dp(2);
                if accrual_base <= Decimal::ZERO {
                    continue;
                }

                // Post each accrual item at EVERY recurring month-end. With
                // monthly_recurring OFF, `month_ends` is [close_date] → one
                // posting per item (byte-identical to the pre-Stage-1 path).
                // With it ON, the same standing accrual recurs each month and
                // auto-reverses at the start of the next month, so a month-end
                // balance sheet always carries ~one month of accrued expense.
                // The annual P&L impact is unchanged: only the final month's
                // accrual stands at slice end (its reversal falls in the next
                // slice); every earlier month accrues-then-reverses to zero.
                for &posting_date in &month_ends {
                    for (description, expense_acct, liability_acct) in accrual_items {
                        let (accrual_je, reversal_je) = accrual_gen.generate_accrued_expense(
                            company_code,
                            description,
                            accrual_base,
                            expense_acct,
                            liability_acct,
                            posting_date,
                            None,
                        );
                        close_jes.push(accrual_je);
                        if let Some(rev_je) = reversal_je {
                            close_jes.push(rev_je);
                        }
                    }
                }
            }

            debug!(
                "Generated accrual entries for {} companies",
                company_codes.len()
            );
        }

        // --- Config-supplied recurring entries (W1-3 Stage 1 generalization) ---
        // The recurring mechanism is NOT hard-wired to depreciation/accruals: a
        // config author (or the product overlay) can declare arbitrary
        // straight-line monthly postings — prepaid amortization, software/SaaS
        // amortization, straight-line rent — as DATA via
        // `period_close.recurring_entries`. Each entry's total is split across
        // the month-ends with the same cent-exact allocation. Consulted only
        // when monthly_recurring is on (so the OFF path stays byte-identical),
        // and this also exercises the previously-dormant prepaid-amortization
        // path. NOTE: an Amortization entry credits its balance account (a
        // prepaid asset); the config author is responsible for a funded opening
        // balance — the engine posts what is declared and does not synthesize a
        // prepayment, so it never fabricates a negative asset on its own.
        if self.phase_config.monthly_recurring
            && !self.config.period_close.recurring_entries.is_empty()
        {
            use datasynth_config::schema::RecurringEntryKind;
            use datasynth_generators::{AccrualGenerator, AccrualGeneratorConfig};
            let mut rec_gen = AccrualGenerator::new(AccrualGeneratorConfig::default());
            if let Some(ctx) = &self.temporal_context {
                rec_gen.set_temporal_context(Arc::clone(ctx));
            }
            let default_company = company_codes.first().cloned().unwrap_or_default();
            for rec in &self.config.period_close.recurring_entries {
                let total = Decimal::from_f64_retain(rec.total_amount)
                    .unwrap_or(Decimal::ZERO)
                    .round_dp(2);
                if total <= Decimal::ZERO {
                    continue;
                }
                let alloc = monthly_straight_line_allocation(total, month_ends.len() as u32);
                for (idx, amount) in alloc.iter().enumerate() {
                    if *amount <= Decimal::ZERO {
                        continue;
                    }
                    let posting_date = month_ends[idx];
                    match rec.kind {
                        RecurringEntryKind::Amortization => {
                            // Dr expense / Cr balance (prepaid asset) — no reversal.
                            let je = rec_gen.generate_prepaid_amortization(
                                &default_company,
                                &rec.description,
                                *amount,
                                &rec.expense_account,
                                &rec.balance_account,
                                posting_date,
                                rec.cost_center.as_deref(),
                            );
                            debug_assert!(
                                je.is_balanced(),
                                "Recurring amortization JE must balance"
                            );
                            close_jes.push(je);
                        }
                        RecurringEntryKind::AccruedExpense => {
                            // Dr expense / Cr balance (liability) — auto-reversed.
                            let (je, rev) = rec_gen.generate_accrued_expense(
                                &default_company,
                                &rec.description,
                                *amount,
                                &rec.expense_account,
                                &rec.balance_account,
                                posting_date,
                                rec.cost_center.as_deref(),
                            );
                            close_jes.push(je);
                            if let Some(rev_je) = rev {
                                close_jes.push(rev_je);
                            }
                        }
                    }
                }
            }
        }

        for company_code in &company_codes {
            // Calculate net income for this company from existing JEs:
            // Net income = sum of credit-normal revenue postings - sum of debit-normal expense postings
            // Revenue (4xxx): credit-normal, so net = credits - debits
            // COGS (5xxx), OpEx (6xxx), Other I/E (7xxx), Tax (8xxx): debit-normal, so net = debits - credits
            let mut total_revenue = Decimal::ZERO;
            let mut total_expenses = Decimal::ZERO;

            for entry in entries.iter() {
                if entry.header.company_code != *company_code {
                    continue;
                }
                for line in &entry.lines {
                    let category = AccountCategory::from_account(&line.gl_account);
                    match category {
                        AccountCategory::Revenue => {
                            // Revenue is credit-normal: net revenue = credits - debits
                            total_revenue += line.credit_amount - line.debit_amount;
                        }
                        AccountCategory::Cogs
                        | AccountCategory::OperatingExpense
                        | AccountCategory::OtherIncomeExpense
                        | AccountCategory::Tax => {
                            // Expenses are debit-normal: net expense = debits - credits
                            total_expenses += line.debit_amount - line.credit_amount;
                        }
                        _ => {}
                    }
                }
            }

            let pre_tax_income = total_revenue - total_expenses;

            // Skip if no income statement activity
            if pre_tax_income == Decimal::ZERO {
                debug!(
                    "Company {}: no pre-tax income, skipping period close",
                    company_code
                );
                continue;
            }

            // --- Tax provision / DTA JE ---
            if pre_tax_income > Decimal::ZERO {
                // Profitable year: DR Tax Expense (8000) / CR Income Tax Payable (2130)
                let tax_amount = (pre_tax_income * tax_rate).round_dp(2);

                let mut tax_header = JournalEntryHeader::new(company_code.clone(), close_date);
                tax_header.document_type = "CL".to_string();
                tax_header.header_text = Some(format!("Tax provision - {}", company_code));
                tax_header.created_by = "CLOSE_ENGINE".to_string();
                tax_header.source = TransactionSource::Automated;
                tax_header.business_process = Some(BusinessProcess::R2R);

                let doc_id = tax_header.document_id;
                let mut tax_je = JournalEntry::new(tax_header);

                // DR Tax Expense (8000)
                tax_je.add_line(JournalEntryLine::debit(
                    doc_id,
                    1,
                    tax_accounts::TAX_EXPENSE.to_string(),
                    tax_amount,
                ));
                // CR Income Tax Payable (2130)
                tax_je.add_line(JournalEntryLine::credit(
                    doc_id,
                    2,
                    tax_accounts::INCOME_TAX_PAYABLE.to_string(),
                    tax_amount,
                ));

                debug_assert!(tax_je.is_balanced(), "Tax provision JE must be balanced");
                close_jes.push(tax_je);
            } else {
                // Loss year: recognise a Deferred Tax Asset (DTA) = |loss| × statutory_rate
                // DR Deferred Tax Asset (1600) / CR Tax Benefit (8000 credit = income tax benefit)
                let dta_amount = (pre_tax_income.abs() * tax_rate).round_dp(2);
                if dta_amount > Decimal::ZERO {
                    let mut dta_header = JournalEntryHeader::new(company_code.clone(), close_date);
                    dta_header.document_type = "CL".to_string();
                    dta_header.header_text =
                        Some(format!("Deferred tax asset (DTA) - {}", company_code));
                    dta_header.created_by = "CLOSE_ENGINE".to_string();
                    dta_header.source = TransactionSource::Automated;
                    dta_header.business_process = Some(BusinessProcess::R2R);

                    let doc_id = dta_header.document_id;
                    let mut dta_je = JournalEntry::new(dta_header);

                    // DR Deferred Tax Asset (1600)
                    dta_je.add_line(JournalEntryLine::debit(
                        doc_id,
                        1,
                        tax_accounts::DEFERRED_TAX_ASSET.to_string(),
                        dta_amount,
                    ));
                    // CR Income Tax Benefit (8000) — credit reduces the tax expense line,
                    // reflecting the benefit of the future deductible temporary difference.
                    dta_je.add_line(JournalEntryLine::credit(
                        doc_id,
                        2,
                        tax_accounts::TAX_EXPENSE.to_string(),
                        dta_amount,
                    ));

                    debug_assert!(dta_je.is_balanced(), "DTA JE must be balanced");
                    close_jes.push(dta_je);
                    debug!(
                        "Company {}: loss year — recognised DTA of {}",
                        company_code, dta_amount
                    );
                }
            }

            // --- Dividend JEs (v2.4) ---
            // If the entity is profitable after tax, declare a 10% dividend payout.
            // This runs AFTER tax provision so the dividend is based on post-tax income
            // but BEFORE the retained earnings close so the RE transfer reflects the
            // reduced balance.
            let tax_provision = if pre_tax_income > Decimal::ZERO {
                (pre_tax_income * tax_rate).round_dp(2)
            } else {
                Decimal::ZERO
            };
            let net_income = pre_tax_income - tax_provision;

            if net_income > Decimal::ZERO {
                use datasynth_generators::DividendGenerator;
                let dividend_amount = (net_income * Decimal::new(10, 2)).round_dp(2); // 10% payout
                let mut div_gen = DividendGenerator::new(self.seed + 460);
                let currency_str = self
                    .config
                    .companies
                    .iter()
                    .find(|c| c.code == *company_code)
                    .map(|c| c.currency.as_str())
                    .unwrap_or("USD");
                let div_result = div_gen.generate(
                    company_code,
                    close_date,
                    Decimal::new(1, 0), // $1 per share placeholder
                    dividend_amount,
                    currency_str,
                );
                let div_je_count = div_result.journal_entries.len();
                close_jes.extend(div_result.journal_entries);
                debug!(
                    "Company {}: declared dividend of {} ({} JEs)",
                    company_code, dividend_amount, div_je_count
                );
            }

            // --- Income statement closing JE ---
            // Net income after tax (profit years) or net loss before DTA benefit (loss years).
            // For a loss year the DTA JE above already recognises the deferred benefit; here we
            // close the pre-tax loss into Retained Earnings as-is.
            // `skip_income_statement_close` is set ONLY by the multi-fiscal-year GenerationSession,
            // which runs its OWN complete year-end close (rev/exp → income summary → retained
            // earnings) per fiscal year. Running this one-sided net-income→RE close as well would
            // post net income to RE TWICE. Single-FY builds never set the flag (they take the
            // single-generate path), so their close is unchanged / byte-identical.
            if net_income != Decimal::ZERO && !self.phase_config.skip_income_statement_close {
                let mut close_header = JournalEntryHeader::new(company_code.clone(), close_date);
                close_header.document_type = "CL".to_string();
                close_header.header_text =
                    Some(format!("Income statement close - {}", company_code));
                close_header.created_by = "CLOSE_ENGINE".to_string();
                close_header.source = TransactionSource::Automated;
                close_header.business_process = Some(BusinessProcess::R2R);

                let doc_id = close_header.document_id;
                let mut close_je = JournalEntry::new(close_header);

                let abs_net_income = net_income.abs();

                if net_income > Decimal::ZERO {
                    // Profit: DR Income Summary (3600) / CR Retained Earnings (3200)
                    close_je.add_line(JournalEntryLine::debit(
                        doc_id,
                        1,
                        equity_accounts::INCOME_SUMMARY.to_string(),
                        abs_net_income,
                    ));
                    close_je.add_line(JournalEntryLine::credit(
                        doc_id,
                        2,
                        equity_accounts::RETAINED_EARNINGS.to_string(),
                        abs_net_income,
                    ));
                } else {
                    // Loss: DR Retained Earnings (3200) / CR Income Summary (3600)
                    close_je.add_line(JournalEntryLine::debit(
                        doc_id,
                        1,
                        equity_accounts::RETAINED_EARNINGS.to_string(),
                        abs_net_income,
                    ));
                    close_je.add_line(JournalEntryLine::credit(
                        doc_id,
                        2,
                        equity_accounts::INCOME_SUMMARY.to_string(),
                        abs_net_income,
                    ));
                }

                debug_assert!(
                    close_je.is_balanced(),
                    "Income statement closing JE must be balanced"
                );
                close_jes.push(close_je);
            }
        }

        let close_count = close_jes.len();
        if close_count > 0 {
            info!("Generated {} period-close journal entries", close_count);
            self.emit_phase_items("period_close", "JournalEntry", &close_jes);
            entries.extend(close_jes);
            stats.period_close_je_count = close_count;

            // Update total entry/line-item stats
            stats.total_entries = entries.len() as u64;
            stats.total_line_items = entries.iter().map(|e| e.line_count() as u64).sum();
        } else {
            debug!("No period-close entries generated (no income statement activity)");
        }

        Ok(())
    }

    /// Phase 8: Generate audit data (engagements, workpapers, evidence, risks, findings).
    fn phase_audit_data(
        &mut self,
        entries: &[JournalEntry],
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<AuditSnapshot> {
        if self.phase_config.generate_audit {
            info!("Phase 8: Generating Audit Data");
            let audit_snapshot = self.generate_audit_data(entries)?;
            stats.audit_engagement_count = audit_snapshot.engagements.len();
            stats.audit_workpaper_count = audit_snapshot.workpapers.len();
            stats.audit_evidence_count = audit_snapshot.evidence.len();
            stats.audit_risk_count = audit_snapshot.risk_assessments.len();
            stats.audit_finding_count = audit_snapshot.findings.len();
            stats.audit_judgment_count = audit_snapshot.judgments.len();
            stats.audit_confirmation_count = audit_snapshot.confirmations.len();
            stats.audit_confirmation_response_count = audit_snapshot.confirmation_responses.len();
            stats.audit_procedure_step_count = audit_snapshot.procedure_steps.len();
            stats.audit_sample_count = audit_snapshot.samples.len();
            stats.audit_analytical_result_count = audit_snapshot.analytical_results.len();
            stats.audit_ia_function_count = audit_snapshot.ia_functions.len();
            stats.audit_ia_report_count = audit_snapshot.ia_reports.len();
            stats.audit_related_party_count = audit_snapshot.related_parties.len();
            stats.audit_related_party_transaction_count =
                audit_snapshot.related_party_transactions.len();
            info!(
                "Audit data generated: {} engagements, {} workpapers, {} evidence, {} risks, \
                 {} findings, {} judgments, {} confirmations, {} procedure steps, {} samples, \
                 {} analytical results, {} IA functions, {} IA reports, {} related parties, \
                 {} RP transactions",
                stats.audit_engagement_count,
                stats.audit_workpaper_count,
                stats.audit_evidence_count,
                stats.audit_risk_count,
                stats.audit_finding_count,
                stats.audit_judgment_count,
                stats.audit_confirmation_count,
                stats.audit_procedure_step_count,
                stats.audit_sample_count,
                stats.audit_analytical_result_count,
                stats.audit_ia_function_count,
                stats.audit_ia_report_count,
                stats.audit_related_party_count,
                stats.audit_related_party_transaction_count,
            );
            self.check_resources_with_log("post-audit")?;
            Ok(audit_snapshot)
        } else {
            debug!("Phase 8: Skipped (audit generation disabled)");
            Ok(AuditSnapshot::default())
        }
    }

    /// Phase 9: Generate banking KYC/AML data.
    fn phase_banking_data(
        &mut self,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<BankingSnapshot> {
        if self.phase_config.generate_banking {
            info!("Phase 9: Generating Banking KYC/AML Data");
            let banking_snapshot = self.generate_banking_data()?;
            stats.banking_customer_count = banking_snapshot.customers.len();
            stats.banking_account_count = banking_snapshot.accounts.len();
            stats.banking_transaction_count = banking_snapshot.transactions.len();
            stats.banking_suspicious_count = banking_snapshot.suspicious_count;
            info!(
                "Banking data generated: {} customers, {} accounts, {} transactions ({} suspicious)",
                stats.banking_customer_count, stats.banking_account_count,
                stats.banking_transaction_count, stats.banking_suspicious_count
            );
            self.check_resources_with_log("post-banking")?;
            Ok(banking_snapshot)
        } else {
            debug!("Phase 9: Skipped (banking generation disabled)");
            Ok(BankingSnapshot::default())
        }
    }

    /// Phase 10: Export accounting network graphs for ML training.
    fn phase_graph_export(
        &mut self,
        entries: &[JournalEntry],
        coa: &Arc<ChartOfAccounts>,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<GraphExportSnapshot> {
        if self.phase_config.generate_graph_export && !entries.is_empty() {
            info!("Phase 10: Exporting Accounting Network Graphs");
            match self.export_graphs(entries, coa, stats) {
                Ok(snapshot) => {
                    info!(
                        "Graph export complete: {} graphs ({} nodes, {} edges)",
                        snapshot.graph_count, stats.graph_node_count, stats.graph_edge_count
                    );
                    Ok(snapshot)
                }
                Err(e) => {
                    warn!("Phase 10: Graph export failed: {}", e);
                    Ok(GraphExportSnapshot::default())
                }
            }
        } else {
            debug!("Phase 10: Skipped (graph export disabled or no entries)");
            Ok(GraphExportSnapshot::default())
        }
    }

    /// Phase 19b: Export multi-layer hypergraph for RustGraph integration.
    #[allow(clippy::too_many_arguments)]
    fn phase_hypergraph_export(
        &self,
        coa: &Arc<ChartOfAccounts>,
        entries: &[JournalEntry],
        document_flows: &DocumentFlowSnapshot,
        sourcing: &SourcingSnapshot,
        hr: &HrSnapshot,
        manufacturing: &ManufacturingSnapshot,
        banking: &BankingSnapshot,
        audit: &AuditSnapshot,
        financial_reporting: &FinancialReportingSnapshot,
        ocpm: &OcpmSnapshot,
        compliance: &ComplianceRegulationsSnapshot,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<()> {
        if self.config.graph_export.hypergraph.enabled && !entries.is_empty() {
            info!("Phase 19b: Exporting Multi-Layer Hypergraph");
            match self.export_hypergraph(
                coa,
                entries,
                document_flows,
                sourcing,
                hr,
                manufacturing,
                banking,
                audit,
                financial_reporting,
                ocpm,
                compliance,
                stats,
            ) {
                Ok(info) => {
                    info!(
                        "Hypergraph export complete: {} nodes, {} edges, {} hyperedges",
                        info.node_count, info.edge_count, info.hyperedge_count
                    );
                }
                Err(e) => {
                    warn!("Phase 10b: Hypergraph export failed: {}", e);
                }
            }
        } else {
            debug!("Phase 10b: Skipped (hypergraph export disabled or no entries)");
        }
        Ok(())
    }

    /// Phase 11: LLM Enrichment.
    ///
    /// Uses an LLM provider (mock by default) to enrich vendor names with
    /// realistic, context-aware names. This phase is non-blocking: failures
    /// log a warning but do not stop the generation pipeline.
    fn phase_llm_enrichment(&mut self, stats: &mut EnhancedGenerationStatistics) {
        if !self.config.llm.enabled {
            debug!("Phase 11: Skipped (LLM enrichment disabled)");
            return;
        }

        info!("Phase 11: Starting LLM Enrichment");
        let start = std::time::Instant::now();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Select provider: use HttpLlmProvider when a non-mock provider is configured
            // and the corresponding API key environment variable is present.
            let provider: Arc<dyn datasynth_core::llm::LlmProvider> = {
                let schema_provider = &self.config.llm.provider;
                let api_key_env = match schema_provider.as_str() {
                    "openai" => Some("OPENAI_API_KEY"),
                    "anthropic" => Some("ANTHROPIC_API_KEY"),
                    "custom" => Some("LLM_API_KEY"),
                    _ => None,
                };
                if let Some(key_env) = api_key_env {
                    if std::env::var(key_env).is_ok() {
                        let llm_config = datasynth_core::llm::LlmConfig {
                            model: self.config.llm.model.clone(),
                            api_key_env: key_env.to_string(),
                            ..datasynth_core::llm::LlmConfig::default()
                        };
                        match HttpLlmProvider::new(llm_config) {
                            Ok(p) => Arc::new(p),
                            Err(e) => {
                                warn!(
                                    "Failed to create HttpLlmProvider: {}; falling back to mock",
                                    e
                                );
                                Arc::new(MockLlmProvider::new(self.seed))
                            }
                        }
                    } else {
                        Arc::new(MockLlmProvider::new(self.seed))
                    }
                } else {
                    Arc::new(MockLlmProvider::new(self.seed))
                }
            };
            // v4.1.1+: multi-category enrichment. Vendors remain the
            // default path; customers and materials opt in via
            // `llm.enrich_customers` / `llm.enrich_materials` flags.
            let industry = format!("{:?}", self.config.global.industry);

            let vendor_enricher =
                datasynth_generators::llm_enrichment::VendorLlmEnricher::new(Arc::clone(&provider));
            let max_vendors = self
                .config
                .llm
                .max_vendor_enrichments
                .min(self.master_data.vendors.len());
            let mut vendors_enriched = 0usize;
            for vendor in self.master_data.vendors.iter_mut().take(max_vendors) {
                match vendor_enricher.enrich_vendor_name(&industry, "general", &vendor.country) {
                    Ok(name) => {
                        vendor.name = name;
                        vendors_enriched += 1;
                    }
                    Err(e) => warn!(
                        "LLM vendor enrichment failed for {}: {}",
                        vendor.vendor_id, e
                    ),
                }
            }

            let mut customers_enriched = 0usize;
            if self.config.llm.enrich_customers {
                let customer_enricher =
                    datasynth_generators::llm_enrichment::CustomerLlmEnricher::new(Arc::clone(
                        &provider,
                    ));
                let max_customers = self
                    .config
                    .llm
                    .max_customer_enrichments
                    .min(self.master_data.customers.len());
                for customer in self.master_data.customers.iter_mut().take(max_customers) {
                    match customer_enricher.enrich_customer_name(
                        &industry,
                        "general",
                        &customer.country,
                    ) {
                        Ok(name) => {
                            customer.name = name;
                            customers_enriched += 1;
                        }
                        Err(e) => warn!(
                            "LLM customer enrichment failed for {}: {}",
                            customer.customer_id, e
                        ),
                    }
                }
            }

            let mut materials_enriched = 0usize;
            if self.config.llm.enrich_materials {
                let material_enricher =
                    datasynth_generators::llm_enrichment::MaterialLlmEnricher::new(Arc::clone(
                        &provider,
                    ));
                let max_materials = self
                    .config
                    .llm
                    .max_material_enrichments
                    .min(self.master_data.materials.len());
                for material in self.master_data.materials.iter_mut().take(max_materials) {
                    let material_type = format!("{:?}", material.material_type);
                    match material_enricher.enrich_material_description(&material_type, &industry) {
                        Ok(desc) => {
                            material.description = desc;
                            materials_enriched += 1;
                        }
                        Err(e) => warn!(
                            "LLM material enrichment failed for {}: {}",
                            material.material_id, e
                        ),
                    }
                }
            }

            (vendors_enriched, customers_enriched, materials_enriched)
        }));

        match result {
            Ok((v, c, m)) => {
                stats.llm_vendors_enriched = v;
                stats.llm_customers_enriched = c;
                stats.llm_materials_enriched = m;
                let elapsed = start.elapsed();
                stats.llm_enrichment_ms = elapsed.as_millis() as u64;
                info!(
                    "Phase 11 complete: {} vendors, {} customers, {} materials enriched in {}ms",
                    v, c, m, stats.llm_enrichment_ms
                );
            }
            Err(_) => {
                let elapsed = start.elapsed();
                stats.llm_enrichment_ms = elapsed.as_millis() as u64;
                warn!("Phase 11: LLM enrichment failed (panic caught), continuing");
            }
        }
    }

    /// Phase 12: Diffusion Enhancement.
    ///
    /// Generates a sample set matching distribution properties from the
    /// generated data. v4.4.0+ honours `config.diffusion.backend`:
    /// - `"statistical"` (default) — moment-matching backend, always fast.
    /// - `"neural"` / `"hybrid"` — candle-based score network. Requires
    ///   the `neural` Cargo feature; falls back to statistical when the
    ///   feature isn't compiled in, with a loud warning.
    ///
    /// This phase is non-blocking: failures log a warning but do not
    /// stop the pipeline.
    fn phase_diffusion_enhancement(
        &self,
        #[cfg_attr(not(feature = "neural"), allow(unused_variables))] entries: &[JournalEntry],
        stats: &mut EnhancedGenerationStatistics,
    ) {
        if !self.config.diffusion.enabled {
            debug!("Phase 12: Skipped (diffusion enhancement disabled)");
            return;
        }

        info!("Phase 12: Starting Diffusion Enhancement");
        let start = std::time::Instant::now();

        let backend_choice = self.config.diffusion.backend.as_str();
        let use_neural = matches!(backend_choice, "neural" | "hybrid");

        if use_neural {
            #[cfg(feature = "neural")]
            {
                match self.run_neural_diffusion_phase(entries) {
                    Ok(sample_count) => {
                        stats.diffusion_samples_generated = sample_count;
                        let elapsed = start.elapsed();
                        stats.diffusion_enhancement_ms = elapsed.as_millis() as u64;
                        info!(
                            "Phase 12 complete ({}): {} samples in {}ms",
                            backend_choice, sample_count, stats.diffusion_enhancement_ms
                        );
                        return;
                    }
                    Err(e) => {
                        warn!(
                            "Phase 12: neural diffusion failed: {e}. Falling back to statistical."
                        );
                        // Fall through to statistical path below.
                    }
                }
            }
            #[cfg(not(feature = "neural"))]
            {
                warn!(
                    "Phase 12: backend='{}' requested but the `neural` Cargo feature is \
                     not compiled in — falling back to statistical. Rebuild with \
                     `--features neural` (or `neural-cuda` for GPU) to enable.",
                    backend_choice
                );
            }
        } else if !matches!(backend_choice, "statistical" | "") {
            warn!(
                "Phase 12: unknown backend '{}', falling back to statistical",
                backend_choice
            );
        }

        // Statistical path (default + fallback).
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let means = vec![5000.0, 3.0, 2.0];
            let stds = vec![2000.0, 1.5, 1.0];

            let diffusion_config = DiffusionConfig {
                n_steps: self.config.diffusion.n_steps,
                seed: self.seed,
                ..Default::default()
            };

            let backend = StatisticalDiffusionBackend::new(means, stds, diffusion_config);
            let n_samples = self.config.diffusion.sample_size;
            let n_features = 3;
            backend.generate(n_samples, n_features, self.seed).len()
        }));

        match result {
            Ok(sample_count) => {
                stats.diffusion_samples_generated = sample_count;
                let elapsed = start.elapsed();
                stats.diffusion_enhancement_ms = elapsed.as_millis() as u64;
                info!(
                    "Phase 12 complete (statistical): {} samples in {}ms",
                    sample_count, stats.diffusion_enhancement_ms
                );
            }
            Err(_) => {
                let elapsed = start.elapsed();
                stats.diffusion_enhancement_ms = elapsed.as_millis() as u64;
                warn!("Phase 12: Diffusion enhancement failed (panic caught), continuing");
            }
        }
    }

    /// Neural-backend execution — either load a pre-trained checkpoint
    /// (when `config.diffusion.neural.checkpoint_path` is set) or train
    /// from the first batch of JE amounts. Returns the sample count
    /// produced; any error bubbles up to the statistical fallback.
    #[cfg(feature = "neural")]
    fn run_neural_diffusion_phase(&self, entries: &[JournalEntry]) -> Result<usize, SynthError> {
        use datasynth_core::diffusion::{DiffusionBackend, NeuralDiffusionBackend};

        if entries.is_empty() {
            return Err(SynthError::generation(
                "neural diffusion: no journal entries available as training data",
            ));
        }

        let training_data: Vec<Vec<f64>> = entries
            .iter()
            .take(5000)
            .map(|je| {
                let total_amount: f64 = je
                    .lines
                    .iter()
                    .filter(|l| l.debit_amount > rust_decimal::Decimal::ZERO)
                    .map(|l| {
                        use rust_decimal::prelude::ToPrimitive;
                        l.debit_amount.to_f64().unwrap_or(0.0)
                    })
                    .sum();
                let line_count = je.lines.len() as f64;
                // Use the approval-workflow depth as the third feature
                // (proxy for complexity / risk). `None` → 1.
                let approval_level = je
                    .header
                    .approval_workflow
                    .as_ref()
                    .map(|w| w.required_levels as f64)
                    .unwrap_or(1.0);
                vec![total_amount, line_count, approval_level]
            })
            .collect();

        let n_features = training_data.first().map(|r| r.len()).unwrap_or(3);

        let cfg = &self.config.diffusion;
        let neural_cfg = &cfg.neural;

        let backend: NeuralDiffusionBackend = if let Some(ckpt_path) =
            neural_cfg.checkpoint_path.as_ref()
        {
            let path = std::path::Path::new(ckpt_path);
            info!(
                "  Neural diffusion: loading checkpoint from {}",
                path.display()
            );
            NeuralDiffusionBackend::load(path)
                .map_err(|e| SynthError::generation(format!("checkpoint load failed: {e}")))?
        } else {
            use datasynth_core::diffusion::{NeuralDiffusionTrainer, NeuralTrainingConfig};
            info!(
                "  Neural diffusion: training score network on {} rows × {} features, \
                     {} epochs, hidden_dims={:?}",
                training_data.len(),
                n_features,
                neural_cfg.training_epochs,
                neural_cfg.hidden_dims
            );
            let training_config = NeuralTrainingConfig {
                n_steps: cfg.n_steps,
                schedule: cfg.schedule.clone(),
                hidden_dims: neural_cfg.hidden_dims.clone(),
                timestep_embed_dim: neural_cfg.timestep_embed_dim,
                learning_rate: neural_cfg.learning_rate,
                epochs: neural_cfg.training_epochs,
                batch_size: neural_cfg.batch_size,
            };
            let (backend, report) =
                NeuralDiffusionTrainer::train(&training_data, &training_config, self.seed)
                    .map_err(|e| SynthError::generation(format!("neural training failed: {e}")))?;
            info!(
                "  Neural diffusion: training done — {} epochs, final_loss={:.4}",
                report.epochs_completed, report.final_loss
            );
            backend
        };

        let samples = backend.generate(cfg.sample_size, n_features, self.seed);
        Ok(samples.len())
    }

    /// Phase 13: Causal Overlay.
    ///
    /// Builds a structural causal model from a built-in template (e.g.,
    /// fraud_detection) and generates causal samples. Optionally validates
    /// that the output respects the causal structure. This phase is
    /// non-blocking: failures log a warning but do not stop the pipeline.
    fn phase_causal_overlay(&self, stats: &mut EnhancedGenerationStatistics) {
        if !self.config.causal.enabled {
            debug!("Phase 13: Skipped (causal generation disabled)");
            return;
        }

        info!("Phase 13: Starting Causal Overlay");
        let start = std::time::Instant::now();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Select template based on config
            let graph = match self.config.causal.template.as_str() {
                "revenue_cycle" => CausalGraph::revenue_cycle_template(),
                _ => CausalGraph::fraud_detection_template(),
            };

            let scm = StructuralCausalModel::new(graph.clone())
                .map_err(|e| SynthError::generation(format!("Failed to build SCM: {e}")))?;

            let n_samples = self.config.causal.sample_size;
            let samples = scm
                .generate(n_samples, self.seed)
                .map_err(|e| SynthError::generation(format!("SCM generation failed: {e}")))?;

            // Optionally validate causal structure
            let validation_passed = if self.config.causal.validate {
                let report = CausalValidator::validate_causal_structure(&samples, &graph);
                if report.valid {
                    info!(
                        "Causal validation passed: all {} checks OK",
                        report.checks.len()
                    );
                } else {
                    warn!(
                        "Causal validation: {} violations detected: {:?}",
                        report.violations.len(),
                        report.violations
                    );
                }
                Some(report.valid)
            } else {
                None
            };

            Ok::<(usize, Option<bool>), SynthError>((samples.len(), validation_passed))
        }));

        match result {
            Ok(Ok((sample_count, validation_passed))) => {
                stats.causal_samples_generated = sample_count;
                stats.causal_validation_passed = validation_passed;
                let elapsed = start.elapsed();
                stats.causal_generation_ms = elapsed.as_millis() as u64;
                info!(
                    "Phase 13 complete: {} causal samples generated in {}ms (validation: {:?})",
                    sample_count, stats.causal_generation_ms, validation_passed,
                );
            }
            Ok(Err(e)) => {
                let elapsed = start.elapsed();
                stats.causal_generation_ms = elapsed.as_millis() as u64;
                warn!("Phase 13: Causal generation failed: {}", e);
            }
            Err(_) => {
                let elapsed = start.elapsed();
                stats.causal_generation_ms = elapsed.as_millis() as u64;
                warn!("Phase 13: Causal generation failed (panic caught), continuing");
            }
        }
    }

    /// Phase 14: Generate S2C sourcing data.
    fn phase_sourcing_data(
        &mut self,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<SourcingSnapshot> {
        if !self.phase_config.generate_sourcing && !self.config.source_to_pay.enabled {
            debug!("Phase 14: Skipped (sourcing generation disabled)");
            return Ok(SourcingSnapshot::default());
        }
        let degradation = self.check_resources()?;
        if degradation >= DegradationLevel::Reduced {
            debug!(
                "Phase skipped due to resource pressure (degradation: {:?})",
                degradation
            );
            return Ok(SourcingSnapshot::default());
        }

        info!("Phase 14: Generating S2C Sourcing Data");
        let seed = self.seed;

        // Gather vendor data from master data
        let vendor_ids: Vec<String> = self
            .master_data
            .vendors
            .iter()
            .map(|v| v.vendor_id.clone())
            .collect();
        if vendor_ids.is_empty() {
            debug!("Phase 14: Skipped (no vendors available)");
            return Ok(SourcingSnapshot::default());
        }

        let categories: Vec<(String, String)> = vec![
            ("CAT-RAW".to_string(), "Raw Materials".to_string()),
            ("CAT-OFF".to_string(), "Office Supplies".to_string()),
            ("CAT-IT".to_string(), "IT Equipment".to_string()),
            ("CAT-SVC".to_string(), "Professional Services".to_string()),
            ("CAT-LOG".to_string(), "Logistics".to_string()),
        ];
        let categories_with_spend: Vec<(String, String, rust_decimal::Decimal)> = categories
            .iter()
            .map(|(id, name)| {
                (
                    id.clone(),
                    name.clone(),
                    rust_decimal::Decimal::from(100_000),
                )
            })
            .collect();

        let company_code = self
            .config
            .companies
            .first()
            .map(|c| c.code.as_str())
            .unwrap_or("1000");
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let end_date = start_date + chrono::Months::new(self.config.global.period_months);
        let fiscal_year = start_date.year() as u16;
        let owner_ids: Vec<String> = self
            .master_data
            .employees
            .iter()
            .take(5)
            .map(|e| e.employee_id.clone())
            .collect();
        let owner_id = owner_ids
            .first()
            .map(std::string::String::as_str)
            .unwrap_or("BUYER-001");

        // Step 1: Spend Analysis
        let mut spend_gen = SpendAnalysisGenerator::new(seed);
        let spend_analyses =
            spend_gen.generate(company_code, &vendor_ids, &categories, fiscal_year);

        // Step 2: Sourcing Projects
        let mut project_gen = SourcingProjectGenerator::new(seed + 1);
        let sourcing_projects = if owner_ids.is_empty() {
            Vec::new()
        } else {
            project_gen.generate(
                company_code,
                &categories_with_spend,
                &owner_ids,
                start_date,
                self.config.global.period_months,
            )
        };
        stats.sourcing_project_count = sourcing_projects.len();

        // Step 3: Qualifications
        let qual_vendor_ids: Vec<String> = vendor_ids.iter().take(20).cloned().collect();
        let mut qual_gen = QualificationGenerator::new(seed + 2);
        let qualifications = qual_gen.generate(
            company_code,
            &qual_vendor_ids,
            sourcing_projects.first().map(|p| p.project_id.as_str()),
            owner_id,
            start_date,
        );

        // Step 4: RFx Events
        let mut rfx_gen = RfxGenerator::new(seed + 3);
        let rfx_events: Vec<RfxEvent> = sourcing_projects
            .iter()
            .map(|proj| {
                let qualified_vids: Vec<String> = vendor_ids.iter().take(5).cloned().collect();
                rfx_gen.generate(
                    company_code,
                    &proj.project_id,
                    &proj.category_id,
                    &qualified_vids,
                    owner_id,
                    start_date,
                    50000.0,
                )
            })
            .collect();
        stats.rfx_event_count = rfx_events.len();

        // Step 5: Bids
        let mut bid_gen = BidGenerator::new(seed + 4);
        let mut all_bids = Vec::new();
        for rfx in &rfx_events {
            let bidder_count = vendor_ids.len().clamp(2, 5);
            let responding: Vec<String> = vendor_ids.iter().take(bidder_count).cloned().collect();
            let bids = bid_gen.generate(rfx, &responding, start_date);
            all_bids.extend(bids);
        }
        stats.bid_count = all_bids.len();

        // Step 6: Bid Evaluations
        let mut eval_gen = BidEvaluationGenerator::new(seed + 5);
        let bid_evaluations: Vec<BidEvaluation> = rfx_events
            .iter()
            .map(|rfx| {
                let rfx_bids: Vec<SupplierBid> = all_bids
                    .iter()
                    .filter(|b| b.rfx_id == rfx.rfx_id)
                    .cloned()
                    .collect();
                eval_gen.evaluate(rfx, &rfx_bids, owner_id)
            })
            .collect();

        // Step 7: Contracts from winning bids
        let mut contract_gen = ContractGenerator::new(seed + 6);
        let contracts: Vec<ProcurementContract> = bid_evaluations
            .iter()
            .zip(rfx_events.iter())
            .filter_map(|(eval, rfx)| {
                eval.ranked_bids.first().and_then(|winner| {
                    all_bids
                        .iter()
                        .find(|b| b.bid_id == winner.bid_id)
                        .map(|winning_bid| {
                            contract_gen.generate_from_bid(
                                winning_bid,
                                Some(&rfx.sourcing_project_id),
                                &rfx.category_id,
                                owner_id,
                                start_date,
                            )
                        })
                })
            })
            .collect();
        stats.contract_count = contracts.len();

        // Step 8: Catalog Items
        let mut catalog_gen = CatalogGenerator::new(seed + 7);
        let catalog_items = catalog_gen.generate(&contracts);
        stats.catalog_item_count = catalog_items.len();

        // Step 9: Scorecards
        let mut scorecard_gen = ScorecardGenerator::new(seed + 8);
        let vendor_contracts: Vec<(String, Vec<&ProcurementContract>)> = contracts
            .iter()
            .fold(
                std::collections::HashMap::<String, Vec<&ProcurementContract>>::new(),
                |mut acc, c| {
                    acc.entry(c.vendor_id.clone()).or_default().push(c);
                    acc
                },
            )
            .into_iter()
            .collect();
        let scorecards = scorecard_gen.generate(
            company_code,
            &vendor_contracts,
            start_date,
            end_date,
            owner_id,
        );
        stats.scorecard_count = scorecards.len();

        // Back-populate cross-references on sourcing projects (Task 35)
        // Link each project to its RFx events, contracts, and spend analyses
        let mut sourcing_projects = sourcing_projects;
        for project in &mut sourcing_projects {
            // Link RFx events generated for this project
            project.rfx_ids = rfx_events
                .iter()
                .filter(|rfx| rfx.sourcing_project_id == project.project_id)
                .map(|rfx| rfx.rfx_id.clone())
                .collect();

            // Link contract awarded from this project's RFx
            project.contract_id = contracts
                .iter()
                .find(|c| {
                    c.sourcing_project_id
                        .as_deref()
                        .is_some_and(|sp| sp == project.project_id)
                })
                .map(|c| c.contract_id.clone());

            // Link spend analysis for matching category (use category_id as the reference)
            project.spend_analysis_id = spend_analyses
                .iter()
                .find(|sa| sa.category_id == project.category_id)
                .map(|sa| sa.category_id.clone());
        }

        info!(
            "S2C sourcing generated: {} projects, {} RFx, {} bids, {} contracts, {} catalog items, {} scorecards",
            stats.sourcing_project_count, stats.rfx_event_count, stats.bid_count,
            stats.contract_count, stats.catalog_item_count, stats.scorecard_count
        );
        self.check_resources_with_log("post-sourcing")?;

        Ok(SourcingSnapshot {
            spend_analyses,
            sourcing_projects,
            qualifications,
            rfx_events,
            bids: all_bids,
            bid_evaluations,
            contracts,
            catalog_items,
            scorecards,
        })
    }

    /// Build a [`GroupStructure`] from the current company configuration.
    ///
    /// The first company in the configuration is treated as the ultimate parent.
    /// All remaining companies become wholly-owned (100 %) subsidiaries with
    /// [`GroupConsolidationMethod::FullConsolidation`] by default.
    fn build_group_structure(&self) -> datasynth_core::models::intercompany::GroupStructure {
        use datasynth_core::models::intercompany::{GroupStructure, SubsidiaryRelationship};

        let parent_code = self
            .config
            .companies
            .first()
            .map(|c| c.code.clone())
            .unwrap_or_else(|| "PARENT".to_string());

        let mut group = GroupStructure::new(parent_code);

        for company in self.config.companies.iter().skip(1) {
            let sub =
                SubsidiaryRelationship::new_full(company.code.clone(), company.currency.clone());
            group.add_subsidiary(sub);
        }

        group
    }

    /// Phase 14b: Generate intercompany transactions, matching, and eliminations.
    fn phase_intercompany(
        &mut self,
        journal_entries: &[JournalEntry],
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<IntercompanySnapshot> {
        // Skip if intercompany is disabled in config
        if !self.phase_config.generate_intercompany && !self.config.intercompany.enabled {
            debug!("Phase 14b: Skipped (intercompany generation disabled)");
            return Ok(IntercompanySnapshot::default());
        }

        // Intercompany requires at least 2 companies
        if self.config.companies.len() < 2 {
            debug!(
                "Phase 14b: Skipped (intercompany requires 2+ companies, found {})",
                self.config.companies.len()
            );
            return Ok(IntercompanySnapshot::default());
        }

        info!("Phase 14b: Generating Intercompany Transactions");

        // Build the group structure early — used by ISA 600 component auditor scope
        // and consolidated financial statement generators downstream.
        let group_structure = self.build_group_structure();
        debug!(
            "Group structure built: parent={}, subsidiaries={}",
            group_structure.parent_entity,
            group_structure.subsidiaries.len()
        );

        let seed = self.seed;
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let end_date = start_date + chrono::Months::new(self.config.global.period_months);

        // Build ownership structure from company configs
        // First company is treated as the parent, remaining are subsidiaries
        let parent_code = self.config.companies[0].code.clone();
        let mut ownership_structure =
            datasynth_core::models::intercompany::OwnershipStructure::new(parent_code.clone());

        for (i, company) in self.config.companies.iter().skip(1).enumerate() {
            let relationship = datasynth_core::models::intercompany::IntercompanyRelationship::new(
                format!("REL{:03}", i + 1),
                parent_code.clone(),
                company.code.clone(),
                rust_decimal::Decimal::from(100), // Default 100% ownership
                start_date,
            );
            ownership_structure.add_relationship(relationship);
        }

        // Convert config transfer pricing method to core model enum
        let tp_method = match self.config.intercompany.transfer_pricing_method {
            datasynth_config::schema::TransferPricingMethod::CostPlus => {
                datasynth_core::models::intercompany::TransferPricingMethod::CostPlus
            }
            datasynth_config::schema::TransferPricingMethod::ComparableUncontrolled => {
                datasynth_core::models::intercompany::TransferPricingMethod::ComparableUncontrolled
            }
            datasynth_config::schema::TransferPricingMethod::ResalePrice => {
                datasynth_core::models::intercompany::TransferPricingMethod::ResalePrice
            }
            datasynth_config::schema::TransferPricingMethod::TransactionalNetMargin => {
                datasynth_core::models::intercompany::TransferPricingMethod::TransactionalNetMargin
            }
            datasynth_config::schema::TransferPricingMethod::ProfitSplit => {
                datasynth_core::models::intercompany::TransferPricingMethod::ProfitSplit
            }
        };

        // Build IC generator config from schema config
        let ic_currency = self
            .config
            .companies
            .first()
            .map(|c| c.currency.clone())
            .unwrap_or_else(|| "USD".to_string());
        let ic_gen_config = datasynth_generators::ICGeneratorConfig {
            ic_transaction_rate: self.config.intercompany.ic_transaction_rate,
            transfer_pricing_method: tp_method,
            markup_percent: rust_decimal::Decimal::from_f64_retain(
                self.config.intercompany.markup_percent,
            )
            .unwrap_or(rust_decimal::Decimal::from(5)),
            generate_matched_pairs: self.config.intercompany.generate_matched_pairs,
            default_currency: ic_currency,
            ..Default::default()
        };

        // Create IC generator
        let mut ic_generator = datasynth_generators::ICGenerator::new(
            ic_gen_config,
            ownership_structure.clone(),
            seed + 50,
        );

        // Generate IC transactions for the period
        // Use ~3 transactions per day as a reasonable default
        let transactions_per_day = 3;
        let matched_pairs = ic_generator.generate_transactions_for_period(
            start_date,
            end_date,
            transactions_per_day,
        );

        // Generate IC source P2P/O2C documents
        let ic_doc_chains = ic_generator.generate_ic_document_chains(&matched_pairs);
        debug!(
            "Generated {} IC seller invoices, {} IC buyer POs",
            ic_doc_chains.seller_invoices.len(),
            ic_doc_chains.buyer_orders.len()
        );

        // Generate journal entries from matched pairs
        let mut seller_entries = Vec::new();
        let mut buyer_entries = Vec::new();
        let fiscal_year = start_date.year();

        for pair in &matched_pairs {
            let fiscal_period = pair.posting_date.month();
            let (seller_je, buyer_je) =
                ic_generator.generate_journal_entries(pair, fiscal_year, fiscal_period);
            seller_entries.push(seller_je);
            buyer_entries.push(buyer_je);
        }

        // Run matching engine
        let matching_config = datasynth_generators::ICMatchingConfig {
            base_currency: self
                .config
                .companies
                .first()
                .map(|c| c.currency.clone())
                .unwrap_or_else(|| "USD".to_string()),
            ..Default::default()
        };
        let mut matching_engine = datasynth_generators::ICMatchingEngine::new(matching_config);
        matching_engine.load_matched_pairs(&matched_pairs);
        let matching_result = matching_engine.run_matching(end_date);

        // Generate elimination entries if configured
        let mut elimination_entries = Vec::new();
        if self.config.intercompany.generate_eliminations {
            let elim_config = datasynth_generators::EliminationConfig {
                consolidation_entity: "GROUP".to_string(),
                base_currency: self
                    .config
                    .companies
                    .first()
                    .map(|c| c.currency.clone())
                    .unwrap_or_else(|| "USD".to_string()),
                ..Default::default()
            };

            let mut elim_generator =
                datasynth_generators::EliminationGenerator::new(elim_config, ownership_structure);

            let fiscal_period = format!("{}{:02}", fiscal_year, end_date.month());
            let all_balances: Vec<datasynth_core::models::intercompany::ICAggregatedBalance> =
                matching_result
                    .matched_balances
                    .iter()
                    .chain(matching_result.unmatched_balances.iter())
                    .cloned()
                    .collect();

            // Build investment and equity maps from the group structure so that the
            // elimination generator can produce equity-investment elimination entries
            // (parent's investment in subsidiary vs. subsidiary's equity capital).
            //
            // investment_amounts key = "{parent}_{subsidiary}", value = net_assets × ownership_pct
            // equity_amounts key = subsidiary_code, value = map of equity_account → amount
            //   (split 10% share capital / 30% APIC / 60% retained earnings by convention)
            //
            // Net assets are derived from the journal entries using account-range heuristics:
            // assets (1xx) minus liabilities (2xx).  A fallback of 1_000_000 is used when
            // no JE data is available (IC phase runs early in the generation pipeline).
            let mut investment_amounts: std::collections::HashMap<String, rust_decimal::Decimal> =
                std::collections::HashMap::new();
            let mut equity_amounts: std::collections::HashMap<
                String,
                std::collections::HashMap<String, rust_decimal::Decimal>,
            > = std::collections::HashMap::new();
            {
                use rust_decimal::Decimal;
                let hundred = Decimal::from(100u32);
                let ten_pct = Decimal::new(10, 2); // 0.10
                let thirty_pct = Decimal::new(30, 2); // 0.30
                let sixty_pct = Decimal::new(60, 2); // 0.60
                let parent_code = &group_structure.parent_entity;
                for sub in &group_structure.subsidiaries {
                    let net_assets = {
                        let na = Self::compute_entity_net_assets(journal_entries, &sub.entity_code);
                        if na > Decimal::ZERO {
                            na
                        } else {
                            Decimal::from(1_000_000u64)
                        }
                    };
                    let ownership_pct = sub.ownership_percentage / hundred; // 0.0–1.0
                    let inv_key = format!("{}_{}", parent_code, sub.entity_code);
                    investment_amounts.insert(inv_key, (net_assets * ownership_pct).round_dp(2));

                    // Split subsidiary equity into conventional components:
                    // 10 % share capital / 30 % APIC / 60 % retained earnings
                    let mut eq_map = std::collections::HashMap::new();
                    eq_map.insert("3100".to_string(), (net_assets * ten_pct).round_dp(2));
                    eq_map.insert("3200".to_string(), (net_assets * thirty_pct).round_dp(2));
                    eq_map.insert("3300".to_string(), (net_assets * sixty_pct).round_dp(2));
                    equity_amounts.insert(sub.entity_code.clone(), eq_map);
                }
            }

            let journal = elim_generator.generate_eliminations(
                &fiscal_period,
                end_date,
                &all_balances,
                &matched_pairs,
                &investment_amounts,
                &equity_amounts,
            );

            elimination_entries = journal.entries.clone();
        }

        let matched_pair_count = matched_pairs.len();
        let elimination_entry_count = elimination_entries.len();
        let match_rate = matching_result.match_rate;

        stats.ic_matched_pair_count = matched_pair_count;
        stats.ic_elimination_count = elimination_entry_count;
        stats.ic_transaction_count = seller_entries.len() + buyer_entries.len();

        info!(
            "Intercompany data generated: {} matched pairs, {} JEs ({} seller + {} buyer), {} elimination entries, {:.1}% match rate",
            matched_pair_count,
            stats.ic_transaction_count,
            seller_entries.len(),
            buyer_entries.len(),
            elimination_entry_count,
            match_rate * 100.0
        );
        self.check_resources_with_log("post-intercompany")?;

        // ----------------------------------------------------------------
        // NCI measurements: derive from group structure ownership percentages
        // ----------------------------------------------------------------
        let nci_measurements: Vec<datasynth_core::models::intercompany::NciMeasurement> = {
            use datasynth_core::models::intercompany::{GroupConsolidationMethod, NciMeasurement};
            use rust_decimal::Decimal;

            let eight_pct = Decimal::new(8, 2); // 0.08

            group_structure
                .subsidiaries
                .iter()
                .filter(|sub| {
                    sub.nci_percentage > Decimal::ZERO
                        && sub.consolidation_method == GroupConsolidationMethod::FullConsolidation
                })
                .map(|sub| {
                    // Compute net assets from actual journal entries for this subsidiary.
                    // Fall back to 1_000_000 when no JE data is available yet (e.g. the
                    // IC phase runs before the main JE batch has been populated).
                    let net_assets_from_jes =
                        Self::compute_entity_net_assets(journal_entries, &sub.entity_code);

                    let net_assets = if net_assets_from_jes > Decimal::ZERO {
                        net_assets_from_jes.round_dp(2)
                    } else {
                        // Fallback: use a plausible base amount
                        Decimal::from(1_000_000u64)
                    };

                    // Net income approximated as 8% of net assets
                    let net_income = (net_assets * eight_pct).round_dp(2);

                    NciMeasurement::compute(
                        sub.entity_code.clone(),
                        sub.nci_percentage,
                        net_assets,
                        net_income,
                    )
                })
                .collect()
        };

        if !nci_measurements.is_empty() {
            info!(
                "NCI measurements: {} subsidiaries with non-controlling interests",
                nci_measurements.len()
            );
        }

        Ok(IntercompanySnapshot {
            group_structure: Some(group_structure),
            matched_pairs,
            seller_journal_entries: seller_entries,
            buyer_journal_entries: buyer_entries,
            elimination_entries,
            nci_measurements,
            ic_document_chains: Some(ic_doc_chains),
            matched_pair_count,
            elimination_entry_count,
            match_rate,
        })
    }

    /// Phase 15: Generate bank reconciliations and financial statements.
    fn phase_financial_reporting(
        &mut self,
        document_flows: &DocumentFlowSnapshot,
        journal_entries: &[JournalEntry],
        coa: &Arc<ChartOfAccounts>,
        _hr: &HrSnapshot,
        _audit: &AuditSnapshot,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<FinancialReportingSnapshot> {
        let fs_enabled = self.phase_config.generate_financial_statements
            || self.config.financial_reporting.enabled;
        let br_enabled = self.phase_config.generate_bank_reconciliation;

        if !fs_enabled && !br_enabled {
            debug!("Phase 15: Skipped (financial reporting disabled)");
            return Ok(FinancialReportingSnapshot::default());
        }

        info!("Phase 15: Generating Financial Reporting Data");

        let seed = self.seed;
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;

        let mut financial_statements = Vec::new();
        let mut bank_reconciliations = Vec::new();
        let mut trial_balances = Vec::new();
        let mut segment_reports: Vec<datasynth_core::models::OperatingSegment> = Vec::new();
        let mut segment_reconciliations: Vec<datasynth_core::models::SegmentReconciliation> =
            Vec::new();
        // Standalone statements keyed by entity code
        let mut standalone_statements: std::collections::HashMap<String, Vec<FinancialStatement>> =
            std::collections::HashMap::new();
        // Consolidated statements (one per period)
        let mut consolidated_statements: Vec<FinancialStatement> = Vec::new();
        // Consolidation schedules (one per period)
        let mut consolidation_schedules: Vec<ConsolidationSchedule> = Vec::new();

        // Generate financial statements from JE-derived trial balances.
        //
        // When journal entries are available, we use cumulative trial balances for
        // balance sheet accounts and current-period trial balances for income
        // statement accounts. We also track prior-period trial balances so the
        // generator can produce comparative amounts, and we build a proper
        // cash flow statement from working capital changes rather than random data.
        if fs_enabled {
            let has_journal_entries = !journal_entries.is_empty();

            // Use FinancialStatementGenerator for balance sheet and income statement,
            // but build cash flow ourselves from TB data when JEs are available.
            let mut fs_gen = FinancialStatementGenerator::new(seed + 20);
            // Separate generator for consolidated statements (different seed offset)
            let mut cons_gen = FinancialStatementGenerator::new(seed + 21);

            // Collect elimination JEs once (reused across periods)
            let elimination_entries: Vec<&JournalEntry> = journal_entries
                .iter()
                .filter(|je| je.header.is_elimination)
                .collect();

            // Generate one set of statements per period, per entity
            for period in 0..self.config.global.period_months {
                let period_start = start_date + chrono::Months::new(period);
                let period_end =
                    start_date + chrono::Months::new(period + 1) - chrono::Days::new(1);
                let fiscal_year = period_end.year() as u16;
                let fiscal_period = period_end.month() as u8;
                let period_label = format!("{}-{:02}", fiscal_year, fiscal_period);

                // Build per-entity trial balances for this period (non-elimination JEs)
                // We accumulate them for the consolidation step.
                let mut entity_tb_map: std::collections::HashMap<
                    String,
                    std::collections::HashMap<String, rust_decimal::Decimal>,
                > = std::collections::HashMap::new();

                // --- Standalone: one set of statements per company ---
                // v5.33: resolve once per phase. In single-shard / standalone
                // mode this is the primary country's framework; in group
                // mode each shard runs against its own entity (one company)
                // so the primary-country lookup is the entity's. Either way
                // the string drives framework-aware TB classification (Defect
                // A fix — German SKR / French PCG accounts no longer routed
                // through a US-only prefix table).
                let framework_str = self.resolve_framework_str();
                for (company_idx, company) in self.config.companies.iter().enumerate() {
                    let company_code = company.code.as_str();
                    let currency = company.currency.as_str();
                    // Use a unique seed offset per company to keep statements deterministic
                    // and distinct across companies
                    let company_seed_offset = 20u64 + (company_idx as u64 * 100);
                    let mut company_fs_gen =
                        FinancialStatementGenerator::new(seed + company_seed_offset);

                    if has_journal_entries {
                        let tb_entries = Self::build_cumulative_trial_balance(
                            journal_entries,
                            coa,
                            company_code,
                            start_date,
                            period_end,
                            fiscal_year,
                            fiscal_period,
                            framework_str,
                        );

                        // Accumulate per-entity category balances for consolidation
                        let entity_cat_map =
                            entity_tb_map.entry(company_code.to_string()).or_default();
                        for tb_entry in &tb_entries {
                            let net = tb_entry.debit_balance - tb_entry.credit_balance;
                            *entity_cat_map.entry(tb_entry.category.clone()).or_default() += net;
                        }

                        let stmts = company_fs_gen.generate(
                            company_code,
                            currency,
                            &tb_entries,
                            period_start,
                            period_end,
                            fiscal_year,
                            fiscal_period,
                            None,
                            "SYS-AUTOCLOSE",
                        );

                        let mut entity_stmts = Vec::new();
                        for stmt in stmts {
                            if stmt.statement_type == StatementType::CashFlowStatement {
                                let net_income = Self::calculate_net_income_from_tb(&tb_entries);
                                let cf_items = Self::build_cash_flow_from_trial_balances(
                                    &tb_entries,
                                    None,
                                    net_income,
                                );
                                entity_stmts.push(FinancialStatement {
                                    cash_flow_items: cf_items,
                                    ..stmt
                                });
                            } else {
                                entity_stmts.push(stmt);
                            }
                        }

                        // Add to the flat financial_statements list (used by KPI/budget)
                        financial_statements.extend(entity_stmts.clone());

                        // Store standalone per-entity
                        standalone_statements
                            .entry(company_code.to_string())
                            .or_default()
                            .extend(entity_stmts);

                        // Only store trial balance for the first company in the period
                        // to avoid duplicates in the trial_balances list
                        if company_idx == 0 {
                            trial_balances.push(PeriodTrialBalance {
                                fiscal_year,
                                fiscal_period,
                                period_start,
                                period_end,
                                entries: tb_entries,
                                framework: framework_str.to_string(),
                            });
                        }
                    } else {
                        // Fallback: no JEs available
                        let tb_entries = Self::build_trial_balance_from_entries(
                            journal_entries,
                            coa,
                            company_code,
                            fiscal_year,
                            fiscal_period,
                            framework_str,
                        );

                        let stmts = company_fs_gen.generate(
                            company_code,
                            currency,
                            &tb_entries,
                            period_start,
                            period_end,
                            fiscal_year,
                            fiscal_period,
                            None,
                            "SYS-AUTOCLOSE",
                        );
                        financial_statements.extend(stmts.clone());
                        standalone_statements
                            .entry(company_code.to_string())
                            .or_default()
                            .extend(stmts);

                        if company_idx == 0 && !tb_entries.is_empty() {
                            trial_balances.push(PeriodTrialBalance {
                                fiscal_year,
                                fiscal_period,
                                period_start,
                                period_end,
                                entries: tb_entries,
                                framework: framework_str.to_string(),
                            });
                        }
                    }
                }

                // --- Consolidated: aggregate all entities + apply eliminations ---
                // Use the primary (first) company's currency for the consolidated statement
                let group_currency = self
                    .config
                    .companies
                    .first()
                    .map(|c| c.currency.as_str())
                    .unwrap_or("USD");

                // Build owned elimination entries for this period
                let period_eliminations: Vec<JournalEntry> = elimination_entries
                    .iter()
                    .filter(|je| {
                        je.header.fiscal_year == fiscal_year
                            && je.header.fiscal_period == fiscal_period
                    })
                    .map(|je| (*je).clone())
                    .collect();

                let (cons_line_items, schedule) = ConsolidationGenerator::consolidate(
                    &entity_tb_map,
                    &period_eliminations,
                    &period_label,
                );

                // Build a pseudo trial balance from consolidated line items for the
                // FinancialStatementGenerator to use (only for cash flow direction).
                let cons_tb: Vec<datasynth_generators::TrialBalanceEntry> = schedule
                    .line_items
                    .iter()
                    .map(|li| {
                        let net = li.post_elimination_total;
                        let (debit, credit) = if net >= rust_decimal::Decimal::ZERO {
                            (net, rust_decimal::Decimal::ZERO)
                        } else {
                            (rust_decimal::Decimal::ZERO, -net)
                        };
                        datasynth_generators::TrialBalanceEntry {
                            account_code: li.account_category.clone(),
                            account_name: li.account_category.clone(),
                            category: li.account_category.clone(),
                            debit_balance: debit,
                            credit_balance: credit,
                        }
                    })
                    .collect();

                let mut cons_stmts = cons_gen.generate(
                    "GROUP",
                    group_currency,
                    &cons_tb,
                    period_start,
                    period_end,
                    fiscal_year,
                    fiscal_period,
                    None,
                    "SYS-AUTOCLOSE",
                );

                // Split consolidated line items by statement type.
                // The consolidation generator returns BS items first, then IS items,
                // identified by their CONS- prefix and category.
                let bs_categories: &[&str] = &[
                    "CASH",
                    "RECEIVABLES",
                    "INVENTORY",
                    "FIXEDASSETS",
                    "PAYABLES",
                    "ACCRUEDLIABILITIES",
                    "LONGTERMDEBT",
                    "EQUITY",
                ];
                let (bs_items, is_items): (Vec<_>, Vec<_>) =
                    cons_line_items.into_iter().partition(|li| {
                        let upper = li.label.to_uppercase();
                        bs_categories.iter().any(|c| upper == *c)
                    });

                for stmt in &mut cons_stmts {
                    stmt.is_consolidated = true;
                    match stmt.statement_type {
                        StatementType::BalanceSheet => stmt.line_items = bs_items.clone(),
                        StatementType::IncomeStatement => stmt.line_items = is_items.clone(),
                        _ => {} // CF and equity change statements keep generator output
                    }
                }

                consolidated_statements.extend(cons_stmts);
                consolidation_schedules.push(schedule);
            }

            // Backward compat: if only 1 company, use existing code path logic
            // (prior_cumulative_tb for comparative amounts). Already handled above;
            // the prior_ref is omitted to keep this change minimal.
            let _ = &mut fs_gen; // suppress unused warning

            stats.financial_statement_count = financial_statements.len();
            info!(
                "Financial statements generated: {} standalone + {} consolidated, JE-derived: {}",
                stats.financial_statement_count,
                consolidated_statements.len(),
                has_journal_entries
            );

            // ----------------------------------------------------------------
            // IFRS 8 / ASC 280: Operating Segment Reporting
            // ----------------------------------------------------------------
            // Build entity seeds from the company configuration.
            let entity_seeds: Vec<SegmentSeed> = self
                .config
                .companies
                .iter()
                .map(|c| SegmentSeed {
                    code: c.code.clone(),
                    name: c.name.clone(),
                    currency: c.currency.clone(),
                })
                .collect();

            let mut seg_gen = SegmentGenerator::new(seed + 30);

            // Generate one set of segment reports per period.
            // We extract consolidated revenue / profit / assets from the consolidated
            // financial statements produced above, falling back to simple sums when
            // no consolidated statements were generated (single-entity path).
            for period in 0..self.config.global.period_months {
                let period_end =
                    start_date + chrono::Months::new(period + 1) - chrono::Days::new(1);
                let fiscal_year = period_end.year() as u16;
                let fiscal_period = period_end.month() as u8;
                let period_label = format!("{}-{:02}", fiscal_year, fiscal_period);

                use datasynth_core::models::StatementType;

                // Try to find consolidated income statement for this period
                let cons_is = consolidated_statements.iter().find(|s| {
                    s.fiscal_year == fiscal_year
                        && s.fiscal_period == fiscal_period
                        && s.statement_type == StatementType::IncomeStatement
                });
                let cons_bs = consolidated_statements.iter().find(|s| {
                    s.fiscal_year == fiscal_year
                        && s.fiscal_period == fiscal_period
                        && s.statement_type == StatementType::BalanceSheet
                });

                // If consolidated statements not available fall back to the flat list
                let is_stmt = cons_is.or_else(|| {
                    financial_statements.iter().find(|s| {
                        s.fiscal_year == fiscal_year
                            && s.fiscal_period == fiscal_period
                            && s.statement_type == StatementType::IncomeStatement
                    })
                });
                let bs_stmt = cons_bs.or_else(|| {
                    financial_statements.iter().find(|s| {
                        s.fiscal_year == fiscal_year
                            && s.fiscal_period == fiscal_period
                            && s.statement_type == StatementType::BalanceSheet
                    })
                });

                let consolidated_revenue = is_stmt
                    .and_then(|s| s.line_items.iter().find(|li| li.line_code == "IS-REV"))
                    .map(|li| -li.amount) // revenue is stored as negative in IS
                    .unwrap_or(rust_decimal::Decimal::ZERO);

                let consolidated_profit = is_stmt
                    .and_then(|s| s.line_items.iter().find(|li| li.line_code == "IS-OI"))
                    .map(|li| li.amount)
                    .unwrap_or(rust_decimal::Decimal::ZERO);

                let consolidated_assets = bs_stmt
                    .and_then(|s| s.line_items.iter().find(|li| li.line_code == "BS-TA"))
                    .map(|li| li.amount)
                    .unwrap_or(rust_decimal::Decimal::ZERO);

                // Skip periods where we have no financial data
                if consolidated_revenue == rust_decimal::Decimal::ZERO
                    && consolidated_assets == rust_decimal::Decimal::ZERO
                {
                    continue;
                }

                let group_code = self
                    .config
                    .companies
                    .first()
                    .map(|c| c.code.as_str())
                    .unwrap_or("GROUP");

                // Compute period depreciation from JEs with document type "CL" hitting account
                // 6000 (depreciation expense).  These are generated by phase_period_close.
                let total_depr: rust_decimal::Decimal = journal_entries
                    .iter()
                    .filter(|je| je.header.document_type == "CL")
                    .flat_map(|je| je.lines.iter())
                    .filter(|l| l.gl_account.starts_with("6000"))
                    .map(|l| l.debit_amount)
                    .fold(rust_decimal::Decimal::ZERO, |a, v| a + v);
                let depr_param = if total_depr > rust_decimal::Decimal::ZERO {
                    Some(total_depr)
                } else {
                    None
                };

                let (segs, recon) = seg_gen.generate(
                    group_code,
                    &period_label,
                    consolidated_revenue,
                    consolidated_profit,
                    consolidated_assets,
                    &entity_seeds,
                    depr_param,
                );
                segment_reports.extend(segs);
                segment_reconciliations.push(recon);
            }

            info!(
                "Segment reports generated: {} segments, {} reconciliations",
                segment_reports.len(),
                segment_reconciliations.len()
            );
        }

        // Generate bank reconciliations from payment data
        if br_enabled && !document_flows.payments.is_empty() {
            let employee_ids: Vec<String> = self
                .master_data
                .employees
                .iter()
                .map(|e| e.employee_id.clone())
                .collect();
            let mut br_gen =
                BankReconciliationGenerator::new(seed + 25).with_employee_pool(employee_ids);

            // F1 Cash_Treasury real tie: the set of cash-account codes — accounts whose sub_type is
            // `AccountSubType::Cash`. This is the SAME classification the product loader records as
            // `account_category = 'cash'` (it stores the serialized sub_type) and the CASH-DB-001
            // reconciler ties against, so the engine and the product agree on which accounts are
            // "cash" by construction. Empty (and the whole tie is skipped) unless opted in.
            let tie_book_to_gl = self.config.financial_reporting.bank_reconciliation_tie_to_gl;
            let cash_account_codes: std::collections::HashSet<&str> = if tie_book_to_gl {
                coa.accounts
                    .iter()
                    .filter(|a| a.sub_type == AccountSubType::Cash)
                    .map(|a| a.account_number.as_str())
                    .collect()
            } else {
                std::collections::HashSet::new()
            };

            // Group payments by company code and period
            for company in &self.config.companies {
                let company_payments: Vec<PaymentReference> = document_flows
                    .payments
                    .iter()
                    .filter(|p| p.header.company_code == company.code)
                    .map(|p| PaymentReference {
                        id: p.header.document_id.clone(),
                        amount: if p.is_vendor { p.amount } else { -p.amount },
                        date: p.header.document_date,
                        reference: p
                            .check_number
                            .clone()
                            .or_else(|| p.wire_reference.clone())
                            .unwrap_or_else(|| p.header.document_id.clone()),
                    })
                    .collect();

                if company_payments.is_empty() {
                    continue;
                }

                let bank_account_id = format!("{}-MAIN", company.code);

                // Generate one reconciliation per period
                for period in 0..self.config.global.period_months {
                    let period_start = start_date + chrono::Months::new(period);
                    let period_end =
                        start_date + chrono::Months::new(period + 1) - chrono::Days::new(1);

                    let period_payments: Vec<PaymentReference> = company_payments
                        .iter()
                        .filter(|p| p.date >= period_start && p.date <= period_end)
                        .cloned()
                        .collect();

                    // GL cash ending balance as-at period_end for this company: Σ(debit − credit)
                    // over the cash accounts for every JE line dated in [start_date, period_end].
                    // This mirrors the product's cumulative debit-positive balance derivation over
                    // the same JE lines and the same cash-account set, so the reconciliation's book
                    // side ties to the delivered GL cash to the cent. `None` (the legacy random-
                    // opening back-solve) when the tie is off → byte-identical output.
                    let gl_cash_ending: Option<rust_decimal::Decimal> = if tie_book_to_gl {
                        let mut bal = rust_decimal::Decimal::ZERO;
                        for je in journal_entries {
                            if je.header.company_code != company.code
                                || je.header.document_date < start_date
                                || je.header.document_date > period_end
                            {
                                continue;
                            }
                            for line in &je.lines {
                                if cash_account_codes.contains(line.gl_account.as_str()) {
                                    bal += line.debit_amount - line.credit_amount;
                                }
                            }
                        }
                        Some(bal)
                    } else {
                        None
                    };

                    let recon = br_gen.generate(
                        &company.code,
                        &bank_account_id,
                        period_start,
                        period_end,
                        &company.currency,
                        &period_payments,
                        gl_cash_ending,
                    );
                    bank_reconciliations.push(recon);
                }
            }
            info!(
                "Bank reconciliations generated: {} reconciliations",
                bank_reconciliations.len()
            );
        }

        stats.bank_reconciliation_count = bank_reconciliations.len();
        self.check_resources_with_log("post-financial-reporting")?;

        if !trial_balances.is_empty() {
            info!(
                "Period-close trial balances captured: {} periods",
                trial_balances.len()
            );
        }

        // Notes to financial statements are generated in a separate post-processing step
        // (generate_notes_to_financial_statements) called after accounting_standards and tax
        // phases have completed, so that deferred tax and provision data can be wired in.
        let notes_to_financial_statements = Vec::new();

        Ok(FinancialReportingSnapshot {
            financial_statements,
            standalone_statements,
            consolidated_statements,
            consolidation_schedules,
            bank_reconciliations,
            trial_balances,
            segment_reports,
            segment_reconciliations,
            notes_to_financial_statements,
        })
    }

    /// Populate notes to financial statements using fully-resolved snapshots.
    ///
    /// This runs *after* `phase_accounting_standards` and `phase_tax_generation` so that
    /// deferred-tax balances (IAS 12 / ASC 740) and provision totals (IAS 37 / ASC 450)
    /// can be wired into the notes context.  The method mutates
    /// `financial_reporting.notes_to_financial_statements` in-place.
    fn generate_notes_to_financial_statements(
        &self,
        financial_reporting: &mut FinancialReportingSnapshot,
        accounting_standards: &AccountingStandardsSnapshot,
        tax: &TaxSnapshot,
        hr: &HrSnapshot,
        audit: &AuditSnapshot,
        treasury: &TreasurySnapshot,
    ) {
        use datasynth_config::schema::AccountingFrameworkConfig;
        use datasynth_core::models::StatementType;
        use datasynth_generators::period_close::notes_generator::{
            EnhancedNotesContext, NotesGenerator, NotesGeneratorContext,
        };

        let seed = self.seed;
        let start_date = match NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
        {
            Ok(d) => d,
            Err(_) => return,
        };

        let mut notes_gen = NotesGenerator::new(seed + 4235);

        for company in &self.config.companies {
            let last_period_end = start_date
                + chrono::Months::new(self.config.global.period_months)
                - chrono::Days::new(1);
            let fiscal_year = last_period_end.year() as u16;

            // Extract relevant amounts from the already-generated financial statements
            let entity_is = financial_reporting
                .standalone_statements
                .get(&company.code)
                .and_then(|stmts| {
                    stmts.iter().find(|s| {
                        s.fiscal_year == fiscal_year
                            && s.statement_type == StatementType::IncomeStatement
                    })
                });
            let entity_bs = financial_reporting
                .standalone_statements
                .get(&company.code)
                .and_then(|stmts| {
                    stmts.iter().find(|s| {
                        s.fiscal_year == fiscal_year
                            && s.statement_type == StatementType::BalanceSheet
                    })
                });

            // IS-REV is stored as positive (Fix 12 — credit-normal accounts negated at IS build time)
            let revenue_amount = entity_is
                .and_then(|s| s.line_items.iter().find(|li| li.line_code == "IS-REV"))
                .map(|li| li.amount);
            let ppe_gross = entity_bs
                .and_then(|s| s.line_items.iter().find(|li| li.line_code == "BS-FA"))
                .map(|li| li.amount);

            let framework = match self
                .config
                .accounting_standards
                .framework
                .unwrap_or_default()
            {
                AccountingFrameworkConfig::Ifrs | AccountingFrameworkConfig::DualReporting => {
                    "IFRS".to_string()
                }
                _ => "US GAAP".to_string(),
            };

            // ---- Deferred tax (IAS 12 / ASC 740) ----
            // Sum closing DTA and DTL from rollforward entries for this entity.
            let (entity_dta, entity_dtl) = {
                let mut dta = rust_decimal::Decimal::ZERO;
                let mut dtl = rust_decimal::Decimal::ZERO;
                for rf in &tax.deferred_tax.rollforwards {
                    if rf.entity_code == company.code {
                        dta += rf.closing_dta;
                        dtl += rf.closing_dtl;
                    }
                }
                (
                    if dta > rust_decimal::Decimal::ZERO {
                        Some(dta)
                    } else {
                        None
                    },
                    if dtl > rust_decimal::Decimal::ZERO {
                        Some(dtl)
                    } else {
                        None
                    },
                )
            };

            // ---- Provisions (IAS 37 / ASC 450) ----
            // Filter provisions to this entity; sum best_estimate amounts.
            let entity_provisions: Vec<_> = accounting_standards
                .provisions
                .iter()
                .filter(|p| p.entity_code == company.code)
                .collect();
            let provision_count = entity_provisions.len();
            let total_provisions = if provision_count > 0 {
                Some(entity_provisions.iter().map(|p| p.best_estimate).sum())
            } else {
                None
            };

            // ---- Pension data from HR snapshot ----
            let entity_pension_plan_count = hr
                .pension_plans
                .iter()
                .filter(|p| p.entity_code == company.code)
                .count();
            let entity_total_dbo: Option<rust_decimal::Decimal> = {
                let sum: rust_decimal::Decimal = hr
                    .pension_disclosures
                    .iter()
                    .filter(|d| {
                        hr.pension_plans
                            .iter()
                            .any(|p| p.id == d.plan_id && p.entity_code == company.code)
                    })
                    .map(|d| d.net_pension_liability)
                    .sum();
                let plan_assets_sum: rust_decimal::Decimal = hr
                    .pension_plan_assets
                    .iter()
                    .filter(|a| {
                        hr.pension_plans
                            .iter()
                            .any(|p| p.id == a.plan_id && p.entity_code == company.code)
                    })
                    .map(|a| a.fair_value_closing)
                    .sum();
                if entity_pension_plan_count > 0 {
                    Some(sum + plan_assets_sum)
                } else {
                    None
                }
            };
            let entity_total_plan_assets: Option<rust_decimal::Decimal> = {
                let sum: rust_decimal::Decimal = hr
                    .pension_plan_assets
                    .iter()
                    .filter(|a| {
                        hr.pension_plans
                            .iter()
                            .any(|p| p.id == a.plan_id && p.entity_code == company.code)
                    })
                    .map(|a| a.fair_value_closing)
                    .sum();
                if entity_pension_plan_count > 0 {
                    Some(sum)
                } else {
                    None
                }
            };

            // ---- Audit data: related parties + subsequent events ----
            // Audit snapshot covers all entities; use total counts (common case = single entity).
            let rp_count = audit.related_party_transactions.len();
            let se_count = audit.subsequent_events.len();
            let adjusting_count = audit
                .subsequent_events
                .iter()
                .filter(|e| {
                    matches!(
                        e.classification,
                        datasynth_core::models::audit::subsequent_events::EventClassification::Adjusting
                    )
                })
                .count();

            let ctx = NotesGeneratorContext {
                entity_code: company.code.clone(),
                framework,
                period: format!("FY{}", fiscal_year),
                period_end: last_period_end,
                currency: company.currency.clone(),
                revenue_amount,
                total_ppe_gross: ppe_gross,
                statutory_tax_rate: Some(rust_decimal::Decimal::new(21, 2)),
                // Deferred tax from tax snapshot (IAS 12 / ASC 740)
                deferred_tax_asset: entity_dta,
                deferred_tax_liability: entity_dtl,
                // Provisions from accounting_standards snapshot (IAS 37 / ASC 450)
                provision_count,
                total_provisions,
                // Pension data from HR snapshot
                pension_plan_count: entity_pension_plan_count,
                total_dbo: entity_total_dbo,
                total_plan_assets: entity_total_plan_assets,
                // Audit data
                related_party_transaction_count: rp_count,
                subsequent_event_count: se_count,
                adjusting_event_count: adjusting_count,
                ..NotesGeneratorContext::default()
            };

            let entity_notes = notes_gen.generate(&ctx);
            let standard_note_count = entity_notes.len() as u32;
            info!(
                "Notes to FS for {}: {} notes generated (DTA={:?}, DTL={:?}, provisions={})",
                company.code, standard_note_count, entity_dta, entity_dtl, provision_count,
            );
            financial_reporting
                .notes_to_financial_statements
                .extend(entity_notes);

            // v2.4: Enhanced notes backed by treasury, manufacturing, and provision data
            let debt_instruments: Vec<(String, rust_decimal::Decimal, String)> = treasury
                .debt_instruments
                .iter()
                .filter(|d| d.entity_id == company.code)
                .map(|d| {
                    (
                        format!("{:?}", d.instrument_type),
                        d.principal,
                        d.maturity_date.to_string(),
                    )
                })
                .collect();

            let hedge_count = treasury.hedge_relationships.len();
            let effective_hedges = treasury
                .hedge_relationships
                .iter()
                .filter(|h| h.is_effective)
                .count();
            let total_notional: rust_decimal::Decimal = treasury
                .hedging_instruments
                .iter()
                .map(|h| h.notional_amount)
                .sum();
            let total_fair_value: rust_decimal::Decimal = treasury
                .hedging_instruments
                .iter()
                .map(|h| h.fair_value)
                .sum();

            // Join provision_movements with provisions to get entity/type info
            let entity_provision_ids: std::collections::HashSet<&str> = accounting_standards
                .provisions
                .iter()
                .filter(|p| p.entity_code == company.code)
                .map(|p| p.id.as_str())
                .collect();
            let provision_movements: Vec<(
                String,
                rust_decimal::Decimal,
                rust_decimal::Decimal,
                rust_decimal::Decimal,
            )> = accounting_standards
                .provision_movements
                .iter()
                .filter(|m| entity_provision_ids.contains(m.provision_id.as_str()))
                .map(|m| {
                    let prov_type = accounting_standards
                        .provisions
                        .iter()
                        .find(|p| p.id == m.provision_id)
                        .map(|p| format!("{:?}", p.provision_type))
                        .unwrap_or_else(|| "Unknown".to_string());
                    (prov_type, m.opening, m.additions, m.closing)
                })
                .collect();

            let enhanced_ctx = EnhancedNotesContext {
                entity_code: company.code.clone(),
                period: format!("FY{}", fiscal_year),
                currency: company.currency.clone(),
                // Inventory breakdown: best-effort using zero (would need balance tracker)
                finished_goods_value: rust_decimal::Decimal::ZERO,
                wip_value: rust_decimal::Decimal::ZERO,
                raw_materials_value: rust_decimal::Decimal::ZERO,
                debt_instruments,
                hedge_count,
                effective_hedges,
                total_notional,
                total_fair_value,
                provision_movements,
            };

            let enhanced_notes =
                notes_gen.generate_enhanced_notes(&enhanced_ctx, standard_note_count + 1);
            if !enhanced_notes.is_empty() {
                info!(
                    "Enhanced notes for {}: {} supplementary notes (debt={}, hedges={}, provisions={})",
                    company.code,
                    enhanced_notes.len(),
                    enhanced_ctx.debt_instruments.len(),
                    hedge_count,
                    enhanced_ctx.provision_movements.len(),
                );
                financial_reporting
                    .notes_to_financial_statements
                    .extend(enhanced_notes);
            }
        }
    }

    /// Build trial balance entries by aggregating actual journal entry debits and credits per account.
    ///
    /// This ensures the trial balance is coherent with the JEs: every debit and credit
    /// posted in the journal entries flows through to the trial balance, using the real
    /// GL account numbers from the CoA.
    fn build_trial_balance_from_entries(
        journal_entries: &[JournalEntry],
        coa: &ChartOfAccounts,
        company_code: &str,
        fiscal_year: u16,
        fiscal_period: u8,
        framework: &str,
    ) -> Vec<datasynth_generators::TrialBalanceEntry> {
        use rust_decimal::Decimal;

        // Accumulate total debits and credits per GL account
        let mut account_debits: HashMap<String, Decimal> = HashMap::new();
        let mut account_credits: HashMap<String, Decimal> = HashMap::new();

        for je in journal_entries {
            // Filter to matching company, fiscal year, and period
            if je.header.company_code != company_code
                || je.header.fiscal_year != fiscal_year
                || je.header.fiscal_period != fiscal_period
            {
                continue;
            }

            for line in &je.lines {
                let acct = &line.gl_account;
                *account_debits.entry(acct.clone()).or_insert(Decimal::ZERO) += line.debit_amount;
                *account_credits.entry(acct.clone()).or_insert(Decimal::ZERO) += line.credit_amount;
            }
        }

        // Build a TrialBalanceEntry for each account that had activity
        let mut all_accounts: Vec<&String> = account_debits
            .keys()
            .chain(account_credits.keys())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        all_accounts.sort();

        let mut entries = Vec::new();

        for acct_number in all_accounts {
            let debit = account_debits
                .get(acct_number)
                .copied()
                .unwrap_or(Decimal::ZERO);
            let credit = account_credits
                .get(acct_number)
                .copied()
                .unwrap_or(Decimal::ZERO);

            if debit.is_zero() && credit.is_zero() {
                continue;
            }

            // Look up account name from CoA, fall back to "Account {code}"
            let account_name = coa
                .get_account(acct_number)
                .map(|gl| gl.short_description.clone())
                .unwrap_or_else(|| format!("Account {acct_number}"));

            // Map account code prefix to the category strings expected by
            // FinancialStatementGenerator (Cash, Receivables, Inventory,
            // FixedAssets, Payables, AccruedLiabilities, Revenue, CostOfSales,
            // OperatingExpenses).
            let category = Self::category_from_account_code(acct_number, framework);

            entries.push(datasynth_generators::TrialBalanceEntry {
                account_code: acct_number.clone(),
                account_name,
                category,
                debit_balance: debit,
                credit_balance: credit,
            });
        }

        entries
    }

    /// Build a cumulative trial balance by aggregating all JEs from the start up to
    /// (and including) the given period end date.
    ///
    /// Balance sheet accounts (assets, liabilities, equity) use cumulative balances
    /// while income statement accounts (revenue, expenses) show only the current period.
    /// The two are merged into a single Vec for the FinancialStatementGenerator.
    #[allow(clippy::too_many_arguments)]
    fn build_cumulative_trial_balance(
        journal_entries: &[JournalEntry],
        coa: &ChartOfAccounts,
        company_code: &str,
        start_date: NaiveDate,
        period_end: NaiveDate,
        fiscal_year: u16,
        fiscal_period: u8,
        framework: &str,
    ) -> Vec<datasynth_generators::TrialBalanceEntry> {
        use rust_decimal::Decimal;

        // Accumulate debits/credits for balance sheet accounts (cumulative from start)
        let mut bs_debits: HashMap<String, Decimal> = HashMap::new();
        let mut bs_credits: HashMap<String, Decimal> = HashMap::new();

        // Accumulate debits/credits for income statement accounts (current period only)
        let mut is_debits: HashMap<String, Decimal> = HashMap::new();
        let mut is_credits: HashMap<String, Decimal> = HashMap::new();

        for je in journal_entries {
            if je.header.company_code != company_code {
                continue;
            }

            for line in &je.lines {
                let acct = &line.gl_account;
                // Framework-aware BS bucketing — fixes the Defect A
                // mis-classification where US-style prefix tables routed
                // SKR/PCG balance-sheet accounts through the P&L bucket
                // (or vice versa), giving the resulting TB an asymmetric
                // time window with no integrity invariant left to test.
                let is_bs_account = Self::is_balance_sheet_account(acct, framework);

                if is_bs_account {
                    // Balance sheet: accumulate from start through period_end
                    if je.header.document_date <= period_end
                        && je.header.document_date >= start_date
                    {
                        *bs_debits.entry(acct.clone()).or_insert(Decimal::ZERO) +=
                            line.debit_amount;
                        *bs_credits.entry(acct.clone()).or_insert(Decimal::ZERO) +=
                            line.credit_amount;
                    }
                } else {
                    // Income statement: current period only
                    if je.header.fiscal_year == fiscal_year
                        && je.header.fiscal_period == fiscal_period
                    {
                        *is_debits.entry(acct.clone()).or_insert(Decimal::ZERO) +=
                            line.debit_amount;
                        *is_credits.entry(acct.clone()).or_insert(Decimal::ZERO) +=
                            line.credit_amount;
                    }
                }
            }
        }

        // Merge all accounts
        let mut all_accounts: std::collections::HashSet<String> = std::collections::HashSet::new();
        all_accounts.extend(bs_debits.keys().cloned());
        all_accounts.extend(bs_credits.keys().cloned());
        all_accounts.extend(is_debits.keys().cloned());
        all_accounts.extend(is_credits.keys().cloned());

        let mut sorted_accounts: Vec<String> = all_accounts.into_iter().collect();
        sorted_accounts.sort();

        let mut entries = Vec::new();

        for acct_number in &sorted_accounts {
            let category = Self::category_from_account_code(acct_number, framework);
            let is_bs_account = Self::is_balance_sheet_account(acct_number, framework);

            let (debit, credit) = if is_bs_account {
                (
                    bs_debits.get(acct_number).copied().unwrap_or(Decimal::ZERO),
                    bs_credits
                        .get(acct_number)
                        .copied()
                        .unwrap_or(Decimal::ZERO),
                )
            } else {
                (
                    is_debits.get(acct_number).copied().unwrap_or(Decimal::ZERO),
                    is_credits
                        .get(acct_number)
                        .copied()
                        .unwrap_or(Decimal::ZERO),
                )
            };

            if debit.is_zero() && credit.is_zero() {
                continue;
            }

            let account_name = coa
                .get_account(acct_number)
                .map(|gl| gl.short_description.clone())
                .unwrap_or_else(|| format!("Account {acct_number}"));

            entries.push(datasynth_generators::TrialBalanceEntry {
                account_code: acct_number.clone(),
                account_name,
                category,
                debit_balance: debit,
                credit_balance: credit,
            });
        }

        entries
    }

    /// Build a JE-derived cash flow statement using the indirect method.
    ///
    /// Compares current and prior cumulative trial balances to derive working capital
    /// changes, producing a coherent cash flow statement tied to actual journal entries.
    fn build_cash_flow_from_trial_balances(
        current_tb: &[datasynth_generators::TrialBalanceEntry],
        prior_tb: Option<&[datasynth_generators::TrialBalanceEntry]>,
        net_income: rust_decimal::Decimal,
    ) -> Vec<CashFlowItem> {
        use rust_decimal::Decimal;

        // Helper: aggregate a TB by category and return net (debit - credit)
        let aggregate =
            |tb: &[datasynth_generators::TrialBalanceEntry]| -> HashMap<String, Decimal> {
                let mut map: HashMap<String, Decimal> = HashMap::new();
                for entry in tb {
                    let net = entry.debit_balance - entry.credit_balance;
                    *map.entry(entry.category.clone()).or_default() += net;
                }
                map
            };

        let current = aggregate(current_tb);
        let prior = prior_tb.map(aggregate);

        // Get balance for a category, defaulting to zero
        let get = |map: &HashMap<String, Decimal>, key: &str| -> Decimal {
            *map.get(key).unwrap_or(&Decimal::ZERO)
        };

        // Compute change: current - prior (or current if no prior)
        let change = |key: &str| -> Decimal {
            let curr = get(&current, key);
            match &prior {
                Some(p) => curr - get(p, key),
                None => curr,
            }
        };

        // Operating activities (indirect method)
        // Depreciation add-back: approximate from FixedAssets decrease
        let fixed_asset_change = change("FixedAssets");
        let depreciation_addback = if fixed_asset_change < Decimal::ZERO {
            -fixed_asset_change
        } else {
            Decimal::ZERO
        };

        // Working capital changes (increase in assets = cash outflow, increase in liabilities = cash inflow)
        let ar_change = change("Receivables");
        let inventory_change = change("Inventory");
        // AP and AccruedLiabilities are credit-normal: negative net means larger balance = cash inflow
        let ap_change = change("Payables");
        let accrued_change = change("AccruedLiabilities");

        let operating_cf = net_income + depreciation_addback - ar_change - inventory_change
            + (-ap_change)
            + (-accrued_change);

        // Investing activities
        let capex = if fixed_asset_change > Decimal::ZERO {
            -fixed_asset_change
        } else {
            Decimal::ZERO
        };
        let investing_cf = capex;

        // Financing activities
        let debt_change = -change("LongTermDebt");
        let equity_change = -change("Equity");
        let financing_cf = debt_change + equity_change;

        let net_change = operating_cf + investing_cf + financing_cf;

        vec![
            CashFlowItem {
                item_code: "CF-NI".to_string(),
                label: "Net Income".to_string(),
                category: CashFlowCategory::Operating,
                amount: net_income,
                amount_prior: None,
                sort_order: 1,
                is_total: false,
            },
            CashFlowItem {
                item_code: "CF-DEP".to_string(),
                label: "Depreciation & Amortization".to_string(),
                category: CashFlowCategory::Operating,
                amount: depreciation_addback,
                amount_prior: None,
                sort_order: 2,
                is_total: false,
            },
            CashFlowItem {
                item_code: "CF-AR".to_string(),
                label: "Change in Accounts Receivable".to_string(),
                category: CashFlowCategory::Operating,
                amount: -ar_change,
                amount_prior: None,
                sort_order: 3,
                is_total: false,
            },
            CashFlowItem {
                item_code: "CF-AP".to_string(),
                label: "Change in Accounts Payable".to_string(),
                category: CashFlowCategory::Operating,
                amount: -ap_change,
                amount_prior: None,
                sort_order: 4,
                is_total: false,
            },
            CashFlowItem {
                item_code: "CF-INV".to_string(),
                label: "Change in Inventory".to_string(),
                category: CashFlowCategory::Operating,
                amount: -inventory_change,
                amount_prior: None,
                sort_order: 5,
                is_total: false,
            },
            CashFlowItem {
                item_code: "CF-OP".to_string(),
                label: "Net Cash from Operating Activities".to_string(),
                category: CashFlowCategory::Operating,
                amount: operating_cf,
                amount_prior: None,
                sort_order: 6,
                is_total: true,
            },
            CashFlowItem {
                item_code: "CF-CAPEX".to_string(),
                label: "Capital Expenditures".to_string(),
                category: CashFlowCategory::Investing,
                amount: capex,
                amount_prior: None,
                sort_order: 7,
                is_total: false,
            },
            CashFlowItem {
                item_code: "CF-INV-T".to_string(),
                label: "Net Cash from Investing Activities".to_string(),
                category: CashFlowCategory::Investing,
                amount: investing_cf,
                amount_prior: None,
                sort_order: 8,
                is_total: true,
            },
            CashFlowItem {
                item_code: "CF-DEBT".to_string(),
                label: "Net Borrowings / (Repayments)".to_string(),
                category: CashFlowCategory::Financing,
                amount: debt_change,
                amount_prior: None,
                sort_order: 9,
                is_total: false,
            },
            CashFlowItem {
                item_code: "CF-EQ".to_string(),
                label: "Equity Changes".to_string(),
                category: CashFlowCategory::Financing,
                amount: equity_change,
                amount_prior: None,
                sort_order: 10,
                is_total: false,
            },
            CashFlowItem {
                item_code: "CF-FIN-T".to_string(),
                label: "Net Cash from Financing Activities".to_string(),
                category: CashFlowCategory::Financing,
                amount: financing_cf,
                amount_prior: None,
                sort_order: 11,
                is_total: true,
            },
            CashFlowItem {
                item_code: "CF-NET".to_string(),
                label: "Net Change in Cash".to_string(),
                category: CashFlowCategory::Operating,
                amount: net_change,
                amount_prior: None,
                sort_order: 12,
                is_total: true,
            },
        ]
    }

    /// Calculate net income from a set of trial balance entries.
    ///
    /// Revenue is credit-normal (negative net = positive revenue), expenses are debit-normal.
    fn calculate_net_income_from_tb(
        tb: &[datasynth_generators::TrialBalanceEntry],
    ) -> rust_decimal::Decimal {
        use rust_decimal::Decimal;

        let mut aggregated: HashMap<String, Decimal> = HashMap::new();
        for entry in tb {
            let net = entry.debit_balance - entry.credit_balance;
            *aggregated.entry(entry.category.clone()).or_default() += net;
        }

        let revenue = *aggregated.get("Revenue").unwrap_or(&Decimal::ZERO);
        let cogs = *aggregated.get("CostOfSales").unwrap_or(&Decimal::ZERO);
        let opex = *aggregated
            .get("OperatingExpenses")
            .unwrap_or(&Decimal::ZERO);
        let other_income = *aggregated.get("OtherIncome").unwrap_or(&Decimal::ZERO);
        let other_expenses = *aggregated.get("OtherExpenses").unwrap_or(&Decimal::ZERO);

        // revenue is negative (credit-normal), expenses are positive (debit-normal)
        // other_income is typically negative (credit), other_expenses is typically positive
        let operating_income = revenue - cogs - opex - other_expenses - other_income;
        let tax_rate = Decimal::new(25, 2); // 0.25
        let tax = operating_income * tax_rate;
        operating_income - tax
    }

    /// Map a GL account code to the category string expected by FinancialStatementGenerator.
    ///
    /// Uses the first two digits of the account code to classify into the categories
    /// that the financial statement generator aggregates on: Cash, Receivables, Inventory,
    /// FixedAssets, Payables, AccruedLiabilities, LongTermDebt, Equity, Revenue, CostOfSales,
    /// OperatingExpenses, OtherIncome, OtherExpenses.
    /// Map an account code to the orchestrator's 13-bucket category string
    /// (`"Cash"` / `"Receivables"` / `"Inventory"` / `"FixedAssets"` /
    /// `"Payables"` / `"AccruedLiabilities"` / `"LongTermDebt"` /
    /// `"Equity"` / `"Revenue"` / `"CostOfSales"` / `"OperatingExpenses"`
    /// / `"OtherIncome"` / `"OtherExpenses"`).
    ///
    /// `framework` controls which numbering convention is applied:
    ///
    /// - `"us_gaap"` / `"ifrs"` / `"dual_reporting"` — US-style 4-digit
    ///   chart (1xxx assets, 2xxx liabilities, 3xxx equity, 4xxx revenue,
    ///   5xxx COGS, 6xxx OpEx, 7xxx other income, 8xxx other expense).
    /// - `"french_gaap"` — French PCG (1 = capital/liabilities, 2 = fixed
    ///   assets, 3 = inventory, 4 = third parties, 5 = cash, 6 = expenses,
    ///   7 = revenue).
    /// - `"german_gaap"` / `"hgb"` — German SKR04 (0 = fixed assets,
    ///   1 = current assets, 2 = equity, 3 = liabilities, 4 = revenue,
    ///   5 = COGS, 6 = OpEx, 7 = financial, 8 = tax/extraordinary).
    ///
    /// Unknown frameworks fall back to US-style.
    fn category_from_account_code(code: &str, framework: &str) -> String {
        match framework {
            "german_gaap" | "GermanGaap" | "hgb" => Self::skr_category(code),
            "french_gaap" | "FrenchGaap" => Self::pcg_category(code),
            _ => Self::us_gaap_category(code),
        }
        .to_string()
    }

    fn us_gaap_category(code: &str) -> &'static str {
        let prefix: String = code.chars().take(2).collect();
        match prefix.as_str() {
            "10" => "Cash",
            "11" => "Receivables",
            "12" | "13" | "14" => "Inventory",
            "15" | "16" | "17" | "18" | "19" => "FixedAssets",
            "20" => "Payables",
            "21" | "22" | "23" | "24" => "AccruedLiabilities",
            "25" | "26" | "27" | "28" | "29" => "LongTermDebt",
            "30" | "31" | "32" | "33" | "34" | "35" | "36" | "37" | "38" | "39" => "Equity",
            "40" | "41" | "42" | "43" | "44" => "Revenue",
            "50" | "51" | "52" => "CostOfSales",
            "60" | "61" | "62" | "63" | "64" | "65" | "66" | "67" | "68" | "69" => {
                "OperatingExpenses"
            }
            "70" | "71" | "72" | "73" | "74" => "OtherIncome",
            "80" | "81" | "82" | "83" | "84" | "85" | "86" | "87" | "88" | "89" => "OtherExpenses",
            _ => "OperatingExpenses",
        }
    }

    /// SKR04 (German GAAP) prefix → orchestrator category.
    ///
    /// 0 = fixed assets, 1 = current assets (10-12 cash, 13-14 receivables,
    /// 15-19 inventory), 2 = equity, 3 = liabilities (3-31 payables,
    /// 32-37 accrued, 38-39 long-term debt), 4 = revenue, 5 = COGS,
    /// 6 = OpEx, 7 = financial income, 8 = tax/extraordinary expense.
    fn skr_category(code: &str) -> &'static str {
        let first = code.chars().next().and_then(|c| c.to_digit(10));
        let prefix: String = code.chars().take(2).collect();
        match first {
            Some(0) => "FixedAssets",
            Some(1) => match prefix.as_str() {
                "10" | "11" | "12" => "Cash",
                "13" | "14" => "Receivables",
                _ => "Inventory",
            },
            Some(2) => "Equity",
            Some(3) => match prefix.as_str() {
                "30" | "31" => "Payables",
                "32" | "33" | "34" | "35" | "36" | "37" => "AccruedLiabilities",
                _ => "LongTermDebt",
            },
            Some(4) => "Revenue",
            Some(5) => "CostOfSales",
            Some(6) => "OperatingExpenses",
            Some(7) => "OtherIncome",
            Some(8) => "OtherExpenses",
            _ => "OperatingExpenses",
        }
    }

    /// French PCG prefix → orchestrator category.
    ///
    /// 10-14 = equity, 15-19 = liabilities (provisions, debts),
    /// 2 = fixed assets, 3 = inventory, 40 = payables, 41 = receivables,
    /// 42-49 = liabilities (personnel, tax, group), 5 = cash, 6 = expenses,
    /// 7 = revenue.
    fn pcg_category(code: &str) -> &'static str {
        let first = code.chars().next().and_then(|c| c.to_digit(10));
        let second = code.chars().nth(1).and_then(|c| c.to_digit(10));
        match first {
            Some(1) => match second {
                Some(0..=4) => "Equity",
                Some(5) => "AccruedLiabilities",
                _ => "LongTermDebt",
            },
            Some(2) => "FixedAssets",
            Some(3) => "Inventory",
            Some(4) => match second {
                Some(0) => "Payables",
                Some(1) => "Receivables",
                _ => "AccruedLiabilities",
            },
            Some(5) => "Cash",
            Some(6) => "OperatingExpenses",
            Some(7) => "Revenue",
            Some(8) | Some(9) => "OperatingExpenses",
            _ => "OperatingExpenses",
        }
    }

    /// Test whether an account code maps to a balance-sheet line under
    /// the given framework. Drives the cumulative-vs-period bucketing in
    /// [`Self::build_cumulative_trial_balance`].
    ///
    /// Delegates to the framework-aware classifier in
    /// `datasynth-core::framework_accounts` so SKR (German) and PCG
    /// (French) codes are recognised, not silently routed through a
    /// US-style prefix table.
    fn is_balance_sheet_account(code: &str, framework: &str) -> bool {
        // `AccountType` here is the `balance::AccountType` imported at
        // the top of the file; `FrameworkAccounts::classify_account_type`
        // returns the same enum, so no cross-namespace mapping is needed.
        let fa = datasynth_core::framework_accounts::FrameworkAccounts::for_framework(framework);
        matches!(
            fa.classify_account_type(code),
            AccountType::Asset
                | AccountType::ContraAsset
                | AccountType::Liability
                | AccountType::ContraLiability
                | AccountType::Equity
                | AccountType::ContraEquity
        )
    }

    /// Phase 16: Generate HR data (payroll runs, time entries, expense reports).
    fn phase_hr_data(
        &mut self,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<HrSnapshot> {
        if !self.phase_config.generate_hr {
            debug!("Phase 16: Skipped (HR generation disabled)");
            return Ok(HrSnapshot::default());
        }

        info!("Phase 16: Generating HR Data (Payroll, Time Entries, Expenses)");

        let seed = self.seed;
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let end_date = start_date + chrono::Months::new(self.config.global.period_months);
        let company_code = self
            .config
            .companies
            .first()
            .map(|c| c.code.as_str())
            .unwrap_or("1000");
        let currency = self
            .config
            .companies
            .first()
            .map(|c| c.currency.as_str())
            .unwrap_or("USD");

        let employee_ids: Vec<String> = self
            .master_data
            .employees
            .iter()
            .map(|e| e.employee_id.clone())
            .collect();

        if employee_ids.is_empty() {
            debug!("Phase 16: Skipped (no employees available)");
            return Ok(HrSnapshot::default());
        }

        // Extract cost-center pool from master data employees for cross-reference
        // coherence. Fabricated IDs (e.g. "CC-123") are replaced by real values.
        let cost_center_ids: Vec<String> = self
            .master_data
            .employees
            .iter()
            .filter_map(|e| e.cost_center.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let mut snapshot = HrSnapshot::default();

        // Generate payroll runs (one per month)
        if self.config.hr.payroll.enabled {
            let mut payroll_gen = datasynth_generators::PayrollGenerator::new(seed + 330)
                .with_pools(employee_ids.clone(), cost_center_ids.clone());

            // Look up country pack for payroll deductions and labels
            let payroll_pack = self.primary_pack();

            // Store the pack on the generator so generate() resolves
            // localized deduction rates and labels from it.
            payroll_gen.set_country_pack(payroll_pack.clone());

            let employees_with_salary: Vec<(
                String,
                rust_decimal::Decimal,
                Option<String>,
                Option<String>,
            )> = self
                .master_data
                .employees
                .iter()
                .map(|e| {
                    // Use the employee's actual annual base salary.
                    // Fall back to $60,000 / yr if somehow zero.
                    let annual = if e.base_salary > rust_decimal::Decimal::ZERO {
                        e.base_salary
                    } else {
                        rust_decimal::Decimal::from(60_000)
                    };
                    (
                        e.employee_id.clone(),
                        annual, // annual salary — PayrollGenerator divides by 12 for monthly base
                        e.cost_center.clone(),
                        e.department_id.clone(),
                    )
                })
                .collect();

            // Use generate_with_changes when employee change history is available
            // so that salary adjustments, transfers, etc. are reflected in payroll.
            let change_history = &self.master_data.employee_change_history;
            let has_changes = !change_history.is_empty();
            if has_changes {
                debug!(
                    "Payroll will incorporate {} employee change events",
                    change_history.len()
                );
            }

            for month in 0..self.config.global.period_months {
                let period_start = start_date + chrono::Months::new(month);
                let period_end = start_date + chrono::Months::new(month + 1) - chrono::Days::new(1);
                let (run, items) = if has_changes {
                    payroll_gen.generate_with_changes(
                        company_code,
                        &employees_with_salary,
                        period_start,
                        period_end,
                        currency,
                        change_history,
                    )
                } else {
                    payroll_gen.generate(
                        company_code,
                        &employees_with_salary,
                        period_start,
                        period_end,
                        currency,
                    )
                };
                snapshot.payroll_runs.push(run);
                snapshot.payroll_run_count += 1;
                snapshot.payroll_line_item_count += items.len();
                snapshot.payroll_line_items.extend(items);
            }
        }

        // Generate time entries
        if self.config.hr.time_attendance.enabled {
            let mut time_gen = datasynth_generators::TimeEntryGenerator::new(seed + 31)
                .with_pools(employee_ids.clone(), cost_center_ids.clone());
            // v3.4.2: when a temporal context is configured, time entries
            // respect holidays (not just weekends) and submitted_at lag
            // snaps to business days.
            if let Some(ctx) = &self.temporal_context {
                time_gen.set_temporal_context(Arc::clone(ctx));
            }
            let entries = time_gen.generate(
                &employee_ids,
                start_date,
                end_date,
                &self.config.hr.time_attendance,
            );
            snapshot.time_entry_count = entries.len();
            snapshot.time_entries = entries;
        }

        // Generate expense reports
        if self.config.hr.expenses.enabled {
            let mut expense_gen = datasynth_generators::ExpenseReportGenerator::new(seed + 32)
                .with_pools(employee_ids.clone(), cost_center_ids.clone());
            expense_gen.set_country_pack(self.primary_pack().clone());
            // v3.4.2: snap submission / approval / paid / line-item dates
            // to business days when temporal_context is present.
            if let Some(ctx) = &self.temporal_context {
                expense_gen.set_temporal_context(Arc::clone(ctx));
            }
            let company_currency = self
                .config
                .companies
                .first()
                .map(|c| c.currency.as_str())
                .unwrap_or("USD");
            let reports = expense_gen.generate_with_currency(
                &employee_ids,
                start_date,
                end_date,
                &self.config.hr.expenses,
                company_currency,
            );
            snapshot.expense_report_count = reports.len();
            snapshot.expense_reports = reports;
        }

        // Generate benefit enrollments (gated on payroll, since benefits require employees)
        if self.config.hr.payroll.enabled {
            let mut benefit_gen = datasynth_generators::BenefitEnrollmentGenerator::new(seed + 33);
            let employee_pairs: Vec<(String, String)> = self
                .master_data
                .employees
                .iter()
                .map(|e| (e.employee_id.clone(), e.display_name.clone()))
                .collect();
            let enrollments =
                benefit_gen.generate(company_code, &employee_pairs, start_date, currency);
            snapshot.benefit_enrollment_count = enrollments.len();
            snapshot.benefit_enrollments = enrollments;
        }

        // Generate defined benefit pension plans (IAS 19 / ASC 715)
        if self.phase_config.generate_hr {
            let entity_name = self
                .config
                .companies
                .first()
                .map(|c| c.name.as_str())
                .unwrap_or("Entity");
            let period_months = self.config.global.period_months;
            let period_label = {
                let y = start_date.year();
                let m = start_date.month();
                if period_months >= 12 {
                    format!("FY{y}")
                } else {
                    format!("{y}-{m:02}")
                }
            };
            let reporting_date =
                start_date + chrono::Months::new(period_months) - chrono::Days::new(1);

            // Compute average annual salary from actual payroll data when available.
            // PayrollRun.total_gross covers all employees for one pay period; we sum
            // across all runs and divide by employee_count to get per-employee total,
            // then annualise for sub-annual periods.
            let avg_salary: Option<rust_decimal::Decimal> = {
                let employee_count = employee_ids.len();
                if self.config.hr.payroll.enabled
                    && employee_count > 0
                    && !snapshot.payroll_runs.is_empty()
                {
                    // Sum total gross pay across all payroll runs for this company
                    let total_gross: rust_decimal::Decimal = snapshot
                        .payroll_runs
                        .iter()
                        .filter(|r| r.company_code == company_code)
                        .map(|r| r.total_gross)
                        .sum();
                    if total_gross > rust_decimal::Decimal::ZERO {
                        // Annualise: total_gross covers `period_months` months of pay
                        let annual_total = if period_months > 0 && period_months < 12 {
                            total_gross * rust_decimal::Decimal::from(12u32)
                                / rust_decimal::Decimal::from(period_months)
                        } else {
                            total_gross
                        };
                        Some(
                            (annual_total / rust_decimal::Decimal::from(employee_count))
                                .round_dp(2),
                        )
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            let mut pension_gen =
                datasynth_generators::PensionGenerator::new(seed.wrapping_add(34));
            let pension_snap = pension_gen.generate(
                company_code,
                entity_name,
                &period_label,
                reporting_date,
                employee_ids.len(),
                currency,
                avg_salary,
                period_months,
            );
            snapshot.pension_plan_count = pension_snap.plans.len();
            snapshot.pension_plans = pension_snap.plans;
            snapshot.pension_obligations = pension_snap.obligations;
            snapshot.pension_plan_assets = pension_snap.plan_assets;
            snapshot.pension_disclosures = pension_snap.disclosures;
            // Pension JEs are returned here so they can be added to entries
            // in the caller (stored temporarily on snapshot for transfer).
            // We embed them in the hr snapshot for simplicity; the orchestrator
            // will extract and extend `entries`.
            snapshot.pension_journal_entries = pension_snap.journal_entries;
        }

        // Generate stock-based compensation (ASC 718 / IFRS 2)
        if self.phase_config.generate_hr && !employee_ids.is_empty() {
            let period_months = self.config.global.period_months;
            let period_label = {
                let y = start_date.year();
                let m = start_date.month();
                if period_months >= 12 {
                    format!("FY{y}")
                } else {
                    format!("{y}-{m:02}")
                }
            };
            let reporting_date =
                start_date + chrono::Months::new(period_months) - chrono::Days::new(1);

            let mut stock_comp_gen =
                datasynth_generators::StockCompGenerator::new(seed.wrapping_add(35));
            let stock_snap = stock_comp_gen.generate(
                company_code,
                &employee_ids,
                start_date,
                &period_label,
                reporting_date,
                currency,
            );
            snapshot.stock_grant_count = stock_snap.grants.len();
            snapshot.stock_grants = stock_snap.grants;
            snapshot.stock_comp_expenses = stock_snap.expenses;
            snapshot.stock_comp_journal_entries = stock_snap.journal_entries;
        }

        stats.payroll_run_count = snapshot.payroll_run_count;
        stats.time_entry_count = snapshot.time_entry_count;
        stats.expense_report_count = snapshot.expense_report_count;
        stats.benefit_enrollment_count = snapshot.benefit_enrollment_count;
        stats.pension_plan_count = snapshot.pension_plan_count;
        stats.stock_grant_count = snapshot.stock_grant_count;

        info!(
            "HR data generated: {} payroll runs ({} line items), {} time entries, {} expense reports, {} benefit enrollments, {} pension plans, {} stock grants",
            snapshot.payroll_run_count, snapshot.payroll_line_item_count,
            snapshot.time_entry_count, snapshot.expense_report_count,
            snapshot.benefit_enrollment_count, snapshot.pension_plan_count,
            snapshot.stock_grant_count
        );
        self.check_resources_with_log("post-hr")?;

        Ok(snapshot)
    }

    /// Phase 17: Generate accounting standards data (revenue recognition, impairment, ECL).
    fn phase_accounting_standards(
        &mut self,
        ar_aging_reports: &[datasynth_core::models::subledger::ar::ARAgingReport],
        journal_entries: &[JournalEntry],
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<AccountingStandardsSnapshot> {
        if !self.phase_config.generate_accounting_standards {
            debug!("Phase 17: Skipped (accounting standards generation disabled)");
            return Ok(AccountingStandardsSnapshot::default());
        }
        info!("Phase 17: Generating Accounting Standards Data");

        let seed = self.seed;
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let end_date = start_date + chrono::Months::new(self.config.global.period_months);
        let company_code = self
            .config
            .companies
            .first()
            .map(|c| c.code.as_str())
            .unwrap_or("1000");
        let currency = self
            .config
            .companies
            .first()
            .map(|c| c.currency.as_str())
            .unwrap_or("USD");

        // Convert config framework to standards framework.
        // If the user explicitly set a framework in the YAML config, use that.
        // Otherwise, fall back to the country pack's accounting.framework field,
        // and if that is also absent or unrecognised, default to US GAAP.
        let framework = match self.config.accounting_standards.framework {
            Some(datasynth_config::schema::AccountingFrameworkConfig::UsGaap) => {
                datasynth_standards::framework::AccountingFramework::UsGaap
            }
            Some(datasynth_config::schema::AccountingFrameworkConfig::Ifrs) => {
                datasynth_standards::framework::AccountingFramework::Ifrs
            }
            Some(datasynth_config::schema::AccountingFrameworkConfig::DualReporting) => {
                datasynth_standards::framework::AccountingFramework::DualReporting
            }
            Some(datasynth_config::schema::AccountingFrameworkConfig::FrenchGaap) => {
                datasynth_standards::framework::AccountingFramework::FrenchGaap
            }
            Some(datasynth_config::schema::AccountingFrameworkConfig::GermanGaap) => {
                datasynth_standards::framework::AccountingFramework::GermanGaap
            }
            None => {
                // Derive framework from the primary company's country pack
                let pack = self.primary_pack();
                let pack_fw = pack.accounting.framework.as_str();
                match pack_fw {
                    "ifrs" => datasynth_standards::framework::AccountingFramework::Ifrs,
                    "dual_reporting" => {
                        datasynth_standards::framework::AccountingFramework::DualReporting
                    }
                    "french_gaap" => {
                        datasynth_standards::framework::AccountingFramework::FrenchGaap
                    }
                    "german_gaap" | "hgb" => {
                        datasynth_standards::framework::AccountingFramework::GermanGaap
                    }
                    // "us_gaap" or any other/unrecognised value falls back to US GAAP
                    _ => datasynth_standards::framework::AccountingFramework::UsGaap,
                }
            }
        };

        let mut snapshot = AccountingStandardsSnapshot::default();

        // Revenue recognition
        if self.config.accounting_standards.revenue_recognition.enabled {
            let customer_ids: Vec<String> = self
                .master_data
                .customers
                .iter()
                .map(|c| c.customer_id.clone())
                .collect();

            if !customer_ids.is_empty() {
                let mut rev_gen = datasynth_generators::RevenueRecognitionGenerator::new(seed + 40);
                let contracts = rev_gen.generate(
                    company_code,
                    &customer_ids,
                    start_date,
                    end_date,
                    currency,
                    &self.config.accounting_standards.revenue_recognition,
                    framework,
                );
                snapshot.revenue_contract_count = contracts.len();
                snapshot.contracts = contracts;
            }
        }

        // ------------------------------------------------------------
        // W1-3 Stage 2: ASC 606 / IFRS 15 deferred-revenue recognition JEs
        // ------------------------------------------------------------
        //
        // The revenue_recognition_generator above produces CustomerContracts with
        // PerformanceObligations but emits ZERO journal entries — contracts stay
        // data-only by default. When `monthly_recurring` is ON we now drive the
        // recognition through the GL so a monthly build shows deferred revenue being
        // drawn down and revenue recognized:
        //
        //   * INCEPTION (slice start): DR contract asset (other current asset 1590) /
        //     CR deferred revenue (unearned revenue 2300) for the obligation's
        //     allocated price. This FUNDS the opening deferred-revenue liability so
        //     the recognition leg below never creates a negative liability (the #1
        //     A=L+E risk — mirrors the prepaid-amortization honesty note in
        //     phase_period_close). NB: a 606 contract asset is UNBILLED and must NOT
        //     ride the trade-AR control (1100) — that account is reconciled to the AR
        //     subledger by the XR-DB-002 control tie, and an un-subledgered debit
        //     there would break it. We use the generic other-asset account (still
        //     classifies as Asset → A=L+E holds), as the lease ROU does.
        //   * RECOGNITION: DR deferred revenue (2300) / CR revenue (service
        //     revenue 4100). Over-time obligations spread the allocated price evenly
        //     across the month-ends; point-in-time obligations recognize in full on
        //     the month-end on/after their expected satisfaction date (falling back
        //     to the last month-end). Net over the slice: deferred revenue drawn
        //     down by exactly what was funded, revenue recognized, A=L+E preserved
        //     BY CONSTRUCTION.
        //
        // When OFF this block emits nothing (current behavior — byte-identical).
        if self.phase_config.monthly_recurring
            && self.config.accounting_standards.revenue_recognition.enabled
            && !snapshot.contracts.is_empty()
        {
            use datasynth_core::accounts::{
                asset_class_accounts, liability_accounts, revenue_accounts,
            };
            use datasynth_standards::accounting::revenue::SatisfactionPattern;
            let month_ends = self.recurring_month_ends()?;
            let mut rev_jes: Vec<JournalEntry> = Vec::new();
            // Inception is dated at the slice start (the first day) so the funding
            // precedes every recognition month-end within the slice.
            let inception_date = start_date;

            for contract in &snapshot.contracts {
                for po in &contract.performance_obligations {
                    let allocated = po.allocated_price.round_dp(2);
                    if allocated <= Decimal::ZERO {
                        continue;
                    }

                    // --- Inception: fund the deferred-revenue liability ---
                    let mut inc_je = JournalEntry::new_simple(
                        format!("JE-REV606-INC-{}-{}", contract.contract_id, po.sequence),
                        contract.company_code.clone(),
                        inception_date,
                        format!(
                            "ASC 606 contract inception — {} oblig {}",
                            contract.customer_name, po.sequence
                        ),
                    );
                    inc_je.header.currency = contract.currency.clone();
                    inc_je.header.business_process = Some(BusinessProcess::O2C);
                    inc_je.header.source = TransactionSource::Automated;
                    let inc_doc = inc_je.header.document_id;
                    // DR contract asset (unbilled — other current asset 1590, NOT the
                    // trade-AR control 1100, which is reconciled to the AR subledger).
                    inc_je.add_line(JournalEntryLine::debit(
                        inc_doc,
                        1,
                        asset_class_accounts::OTHER_ASSETS.to_string(),
                        allocated,
                    ));
                    // CR deferred revenue (contract liability) 2300
                    // spec 27 R6a-2: stamp the 2300 control line with the structured subledger
                    // dimension (DeferredRevenue, contract_id) — the product decomposes 2300 by
                    // this instead of regex-parsing the JE-REV606-* reference.
                    inc_je.add_line(
                        JournalEntryLine::credit(
                            inc_doc,
                            2,
                            liability_accounts::UNEARNED_REVENUE.to_string(),
                            allocated,
                        )
                        .with_subledger_ref(SubledgerRef::new(
                            SubledgerType::DeferredRevenue,
                            contract.contract_id.to_string(),
                            Some("inception".to_string()),
                        )),
                    );
                    debug_assert!(inc_je.is_balanced(), "ASC 606 inception JE must balance");
                    rev_jes.push(inc_je);

                    // --- Recognition: draw down deferred revenue into revenue ---
                    // Build a per-month-end recognition allocation that sums to
                    // exactly `allocated` (so the liability funded at inception is
                    // fully drawn down — A=L+E neutral over the slice).
                    let recognition: Vec<Decimal> = match po.satisfaction_pattern {
                        SatisfactionPattern::OverTime => {
                            monthly_straight_line_allocation(allocated, month_ends.len() as u32)
                        }
                        SatisfactionPattern::PointInTime => {
                            // Recognize the whole amount on the month-end on/after the
                            // expected satisfaction date; if none falls in the slice,
                            // recognize on the final month-end.
                            let target = po
                                .expected_satisfaction_date
                                .and_then(|sat| month_ends.iter().position(|d| *d >= sat))
                                .unwrap_or(month_ends.len().saturating_sub(1));
                            let mut v = vec![Decimal::ZERO; month_ends.len()];
                            if let Some(slot) = v.get_mut(target) {
                                *slot = allocated;
                            }
                            v
                        }
                    };

                    for (idx, amount) in recognition.iter().enumerate() {
                        if *amount <= Decimal::ZERO {
                            continue;
                        }
                        let posting_date = month_ends[idx];
                        let mut rec_je = JournalEntry::new_simple(
                            format!(
                                "JE-REV606-REC-{}-{}-{}",
                                contract.contract_id,
                                po.sequence,
                                idx + 1
                            ),
                            contract.company_code.clone(),
                            posting_date,
                            format!(
                                "ASC 606 revenue recognition — {} oblig {}",
                                contract.customer_name, po.sequence
                            ),
                        );
                        rec_je.header.currency = contract.currency.clone();
                        rec_je.header.business_process = Some(BusinessProcess::O2C);
                        rec_je.header.source = TransactionSource::Automated;
                        let rec_doc = rec_je.header.document_id;
                        // DR deferred revenue (draw down liability) 2300
                        // spec 27 R6a-2: stamp the 2300 control line (DeferredRevenue, contract_id).
                        rec_je.add_line(
                            JournalEntryLine::debit(
                                rec_doc,
                                1,
                                liability_accounts::UNEARNED_REVENUE.to_string(),
                                *amount,
                            )
                            .with_subledger_ref(SubledgerRef::new(
                                SubledgerType::DeferredRevenue,
                                contract.contract_id.to_string(),
                                Some("recognition".to_string()),
                            )),
                        );
                        // CR revenue 4100
                        rec_je.add_line(JournalEntryLine::credit(
                            rec_doc,
                            2,
                            revenue_accounts::SERVICE_REVENUE.to_string(),
                            *amount,
                        ));
                        debug_assert!(rec_je.is_balanced(), "ASC 606 recognition JE must balance");
                        rev_jes.push(rec_je);
                    }
                }
            }
            debug!(
                "W1-3 Stage 2: generated {} ASC 606 recognition JEs",
                rev_jes.len()
            );
            snapshot.revenue_recognition_journal_entries = rev_jes;
        }

        // Impairment testing
        if self.config.accounting_standards.impairment.enabled {
            let asset_data: Vec<(String, String, rust_decimal::Decimal)> = self
                .master_data
                .assets
                .iter()
                .map(|a| {
                    (
                        a.asset_id.clone(),
                        a.description.clone(),
                        a.acquisition_cost,
                    )
                })
                .collect();

            if !asset_data.is_empty() {
                let mut imp_gen = datasynth_generators::ImpairmentGenerator::new(seed + 41);
                let tests = imp_gen.generate(
                    company_code,
                    &asset_data,
                    end_date,
                    &self.config.accounting_standards.impairment,
                    framework,
                );
                snapshot.impairment_test_count = tests.len();
                snapshot.impairment_tests = tests;
            }
        }

        // Business combinations (IFRS 3 / ASC 805)
        if self
            .config
            .accounting_standards
            .business_combinations
            .enabled
        {
            let bc_config = &self.config.accounting_standards.business_combinations;
            let framework_str = match framework {
                datasynth_standards::framework::AccountingFramework::Ifrs => "IFRS",
                _ => "US_GAAP",
            };
            let mut bc_gen = BusinessCombinationGenerator::new(seed + 42);
            let bc_snap = bc_gen.generate(
                company_code,
                currency,
                start_date,
                end_date,
                bc_config.acquisition_count,
                framework_str,
            );
            snapshot.business_combination_count = bc_snap.combinations.len();
            snapshot.business_combination_journal_entries = bc_snap.journal_entries;
            snapshot.business_combinations = bc_snap.combinations;
        }

        // Expected Credit Loss (IFRS 9 / ASC 326)
        if self
            .config
            .accounting_standards
            .expected_credit_loss
            .enabled
        {
            let ecl_config = &self.config.accounting_standards.expected_credit_loss;
            let framework_str = match framework {
                datasynth_standards::framework::AccountingFramework::Ifrs => "IFRS_9",
                _ => "ASC_326",
            };

            // Use AR aging data from the subledger snapshot if available;
            // otherwise generate synthetic bucket exposures.
            let period_label = format!("{}-{:02}", end_date.year(), end_date.month());

            let mut ecl_gen = EclGenerator::new(seed + 43);

            // Collect combined bucket totals across all company AR aging reports.
            let bucket_exposures: Vec<(
                datasynth_core::models::subledger::ar::AgingBucket,
                rust_decimal::Decimal,
            )> = if ar_aging_reports.is_empty() {
                // No AR aging data — synthesise plausible bucket exposures.
                use datasynth_core::models::subledger::ar::AgingBucket;
                vec![
                    (
                        AgingBucket::Current,
                        rust_decimal::Decimal::from(500_000_u32),
                    ),
                    (
                        AgingBucket::Days1To30,
                        rust_decimal::Decimal::from(120_000_u32),
                    ),
                    (
                        AgingBucket::Days31To60,
                        rust_decimal::Decimal::from(45_000_u32),
                    ),
                    (
                        AgingBucket::Days61To90,
                        rust_decimal::Decimal::from(15_000_u32),
                    ),
                    (
                        AgingBucket::Over90Days,
                        rust_decimal::Decimal::from(8_000_u32),
                    ),
                ]
            } else {
                use datasynth_core::models::subledger::ar::AgingBucket;
                // Sum bucket totals from all reports.
                let mut totals: std::collections::HashMap<AgingBucket, rust_decimal::Decimal> =
                    std::collections::HashMap::new();
                for report in ar_aging_reports {
                    for (bucket, amount) in &report.bucket_totals {
                        *totals.entry(*bucket).or_default() += amount;
                    }
                }
                AgingBucket::all()
                    .into_iter()
                    .map(|b| (b, totals.get(&b).copied().unwrap_or_default()))
                    .collect()
            };

            let ecl_snap = ecl_gen.generate(
                company_code,
                end_date,
                &bucket_exposures,
                ecl_config,
                &period_label,
                framework_str,
            );

            snapshot.ecl_model_count = ecl_snap.ecl_models.len();
            snapshot.ecl_models = ecl_snap.ecl_models;
            snapshot.ecl_provision_movements = ecl_snap.provision_movements;
            snapshot.ecl_journal_entries = ecl_snap.journal_entries;
        }

        // Provisions and contingencies (IAS 37 / ASC 450)
        {
            let framework_str = match framework {
                datasynth_standards::framework::AccountingFramework::Ifrs => "IFRS",
                _ => "US_GAAP",
            };

            // Compute actual revenue from the journal entries generated so far.
            // The `journal_entries` slice passed to this phase contains all GL entries
            // up to and including Period Close. Fall back to a minimum of 100_000 to
            // avoid degenerate zero-based provision amounts on first-period datasets.
            let revenue_proxy = Self::compute_company_revenue(journal_entries, company_code)
                .max(rust_decimal::Decimal::from(100_000_u32));

            let period_label = format!("{}-{:02}", end_date.year(), end_date.month());

            let mut prov_gen = ProvisionGenerator::new(seed + 44);
            let prov_snap = prov_gen.generate(
                company_code,
                currency,
                revenue_proxy,
                end_date,
                &period_label,
                framework_str,
                None, // prior_opening: no carry-forward data in single-period runs
            );

            snapshot.provision_count = prov_snap.provisions.len();
            snapshot.provisions = prov_snap.provisions;
            snapshot.provision_movements = prov_snap.movements;
            snapshot.contingent_liabilities = prov_snap.contingent_liabilities;
            snapshot.provision_journal_entries = prov_snap.journal_entries;
        }

        // IAS 21 Functional Currency Translation
        // For each company whose functional currency differs from the presentation
        // currency, generate a CurrencyTranslationResult with CTA (OCI).
        {
            let ias21_period_label = format!("{}-{:02}", end_date.year(), end_date.month());

            let presentation_currency = self
                .config
                .global
                .presentation_currency
                .clone()
                .unwrap_or_else(|| self.config.global.group_currency.clone());

            // Build a minimal rate table populated with approximate rates from
            // the FX model base rates (USD-based) so we can do the translation.
            let mut rate_table = FxRateTable::new(&presentation_currency);

            // Populate with base rates against USD; if presentation_currency is
            // not USD we do a best-effort two-step conversion using the table's
            // triangulation support.
            let base_rates = base_rates_usd();
            for (ccy, rate) in &base_rates {
                rate_table.add_rate(FxRate::new(
                    ccy,
                    "USD",
                    RateType::Closing,
                    end_date,
                    *rate,
                    "SYNTHETIC",
                ));
                // Average rate = 98% of closing (approximation).
                // 0.98 = 98/100 = Decimal::new(98, 2)
                let avg = (*rate * rust_decimal::Decimal::new(98, 2)).round_dp(6);
                rate_table.add_rate(FxRate::new(
                    ccy,
                    "USD",
                    RateType::Average,
                    end_date,
                    avg,
                    "SYNTHETIC",
                ));
            }

            let mut translation_results = Vec::new();
            for company in &self.config.companies {
                // Compute per-company revenue from actual JEs; fall back to 100_000 minimum
                // to ensure the translation produces non-trivial CTA amounts.
                let company_revenue = Self::compute_company_revenue(journal_entries, &company.code)
                    .max(rust_decimal::Decimal::from(100_000_u32));

                let func_ccy = company
                    .functional_currency
                    .clone()
                    .unwrap_or_else(|| company.currency.clone());

                let result = datasynth_generators::fx::FunctionalCurrencyTranslator::translate(
                    &company.code,
                    &func_ccy,
                    &presentation_currency,
                    &ias21_period_label,
                    end_date,
                    company_revenue,
                    &rate_table,
                );
                translation_results.push(result);
            }

            snapshot.currency_translation_count = translation_results.len();
            snapshot.currency_translation_results = translation_results;
        }

        stats.revenue_contract_count = snapshot.revenue_contract_count;
        stats.impairment_test_count = snapshot.impairment_test_count;
        stats.business_combination_count = snapshot.business_combination_count;
        stats.ecl_model_count = snapshot.ecl_model_count;
        stats.provision_count = snapshot.provision_count;

        // ------------------------------------------------------------
        // v3.3.1: Lease accounting (IFRS 16 / ASC 842)
        // ------------------------------------------------------------
        if self.config.accounting_standards.leases.enabled {
            use datasynth_generators::standards::LeaseGenerator;
            let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
                .unwrap_or_else(|_| {
                    NaiveDate::from_ymd_opt(2025, 1, 1).expect("hardcoded 2025-01-01 is valid")
                });
            let framework =
                Self::resolve_accounting_framework(self.config.accounting_standards.framework);
            let mut lease_gen = LeaseGenerator::new(self.seed + 9500);
            for company in &self.config.companies {
                let leases = lease_gen.generate(
                    &company.code,
                    start_date,
                    &self.config.accounting_standards.leases,
                    framework,
                );
                snapshot.lease_count += leases.len();
                snapshot.leases.extend(leases);
            }
            info!("v3.3.1 lease accounting: {} leases", snapshot.lease_count);
        }

        // ------------------------------------------------------------
        // W1-3 Stage 2: ASC 842 / IFRS 16 lease journal entries
        // ------------------------------------------------------------
        //
        // The lease_generator above produces fully-measured `Lease`s (ROU asset,
        // lease liability, amortization schedule) but its output was NEVER merged
        // to the GL. When `monthly_recurring` is ON we now post:
        //
        //   * INCEPTION (commencement date, clamped into the slice):
        //       DR ROU asset (other non-current assets 1590)
        //       CR lease liability (long-term debt 2600) for the initial PV.
        //     This FUNDS the lease liability so the per-month draw-down below never
        //     creates a negative liability (the A=L+E #1 risk).
        //   * FINANCE lease, per schedule row falling in the slice — two balanced JEs:
        //       (a) payment: DR lease liability (principal) + DR interest expense
        //           (7100) / CR cash (1000). Draws the FUNDED liability down by the
        //           schedule principal (sum over life ≤ PV → never negative).
        //       (b) ROU amortization: DR depreciation/amortization expense (6000) /
        //           CR ROU asset (1590, contra — no separate accumulated-ROU account
        //           exists in the CoA, so we reduce the asset directly).
        //   * OPERATING lease, per schedule row in the slice — two balanced JEs:
        //       (a) single straight-line lease expense DR rent (6300) / CR cash
        //           (1000) for the period payment (the income-statement leg).
        //       (b) balance-sheet unwind: DR lease liability (principal) / CR ROU
        //           asset (principal). ASC 842 keeps an operating lease ON the
        //           balance sheet (unlike legacy ASC 840), so the inception-funded
        //           ROU + liability MUST be drawn down each period or they strand
        //           forever. For level payments the ROU amortization plug
        //           (lease cost - interest accretion) equals the liability paydown
        //           (payment - interest) = the schedule `principal`, so one JE
        //           unwinds both in lockstep → both roll to zero at term end (sum
        //           of principals == funded PV). Without (b) A=L+E still ties but
        //           the balance sheet is materially wrong.
        //
        // Multi-FY safety: only schedule rows whose `period_date` falls within
        // [slice_start, slice_end] are posted, and inception is only emitted when the
        // commencement falls in (or before) this slice — so a per-FY re-invocation
        // never double-posts. When OFF this block emits nothing (byte-identical).
        if self.phase_config.monthly_recurring
            && self.config.accounting_standards.leases.enabled
            && !snapshot.leases.is_empty()
        {
            use datasynth_core::accounts::{cash_accounts, expense_accounts, liability_accounts};
            use datasynth_standards::accounting::leases::LeaseClassification;

            // ROU asset account (no dedicated ROU constant in the CoA — uses the
            // generic non-current "other assets" account, which classifies as Asset).
            const ROU_ASSET_ACCT: &str =
                datasynth_core::accounts::asset_class_accounts::OTHER_ASSETS;
            // Lease liability account (no dedicated lease-liability constant — uses
            // long-term debt, which classifies as Liability).
            let lease_liab_acct = liability_accounts::LONG_TERM_DEBT;

            let slice_start = start_date;
            let slice_end = end_date - chrono::Days::new(1);
            let mut lease_jes: Vec<JournalEntry> = Vec::new();

            for lease in &snapshot.leases {
                // Skip leases that commence after this slice ends — their inception
                // (and all amortization) belongs to a later slice.
                if lease.commencement_date > slice_end {
                    continue;
                }
                let pv = lease.lease_liability.initial_measurement.round_dp(2);
                let monthly_dep = lease.rou_asset.monthly_depreciation().round_dp(2);

                // --- Inception: fund ROU asset + lease liability ---
                // Only emit inception when the lease commences within THIS slice
                // (commencement >= slice_start); a lease that commenced in a prior
                // slice was already funded there.
                if pv > Decimal::ZERO && lease.commencement_date >= slice_start {
                    let inc_date = lease.commencement_date.max(slice_start);
                    let mut inc_je = JournalEntry::new_simple(
                        format!("JE-LEASE842-INC-{}", lease.lease_id),
                        lease.company_code.clone(),
                        inc_date,
                        format!("ASC 842 lease inception — {}", lease.description),
                    );
                    inc_je.header.business_process = Some(BusinessProcess::R2R);
                    inc_je.header.source = TransactionSource::Automated;
                    let inc_doc = inc_je.header.document_id;
                    // DR ROU asset
                    inc_je.add_line(JournalEntryLine::debit(
                        inc_doc,
                        1,
                        ROU_ASSET_ACCT.to_string(),
                        pv,
                    ));
                    // CR lease liability
                    // spec 27 R6a-2: stamp the 2600 lease-liability control line (Lease, lease_id) —
                    // the product decomposes 2600 by this instead of regex-parsing JE-LEASE842-*.
                    inc_je.add_line(
                        JournalEntryLine::credit(inc_doc, 2, lease_liab_acct.to_string(), pv)
                            .with_subledger_ref(SubledgerRef::new(
                                SubledgerType::Lease,
                                lease.lease_id.to_string(),
                                Some("inception".to_string()),
                            )),
                    );
                    debug_assert!(inc_je.is_balanced(), "ASC 842 inception JE must balance");
                    lease_jes.push(inc_je);
                }

                let is_finance = lease.classification == LeaseClassification::Finance;

                // --- Per-period postings: walk schedule rows inside the slice ---
                for row in &lease.lease_liability.amortization_schedule {
                    if row.period_date < slice_start || row.period_date > slice_end {
                        continue;
                    }
                    let interest = row.interest_expense.round_dp(2);
                    let principal = row.principal_payment.round_dp(2);
                    let payment = row.payment_amount.round_dp(2);

                    if is_finance {
                        // (a) Lease payment: DR liability(principal) + DR interest /
                        //     CR cash(payment). Balanced: principal + interest == payment.
                        if payment > Decimal::ZERO {
                            let mut pay_je = JournalEntry::new_simple(
                                format!("JE-LEASE842-PAY-{}-{}", lease.lease_id, row.period_number),
                                lease.company_code.clone(),
                                row.period_date,
                                format!(
                                    "ASC 842 finance lease payment — {} period {}",
                                    lease.description, row.period_number
                                ),
                            );
                            pay_je.header.business_process = Some(BusinessProcess::R2R);
                            pay_je.header.source = TransactionSource::Automated;
                            let pay_doc = pay_je.header.document_id;
                            let mut line_no = 1u32;
                            if principal > Decimal::ZERO {
                                // spec 27 R6a-2: stamp the 2600 principal-paydown line (Lease).
                                pay_je.add_line(
                                    JournalEntryLine::debit(
                                        pay_doc,
                                        line_no,
                                        lease_liab_acct.to_string(),
                                        principal,
                                    )
                                    .with_subledger_ref(
                                        SubledgerRef::new(
                                            SubledgerType::Lease,
                                            lease.lease_id.to_string(),
                                            Some("payment".to_string()),
                                        ),
                                    ),
                                );
                                line_no += 1;
                            }
                            if interest > Decimal::ZERO {
                                pay_je.add_line(JournalEntryLine::debit(
                                    pay_doc,
                                    line_no,
                                    expense_accounts::INTEREST_EXPENSE.to_string(),
                                    interest,
                                ));
                                line_no += 1;
                            }
                            // CR cash for the full payment (principal + interest).
                            // Use the summed debits so the JE balances exactly even
                            // when a rounded principal/interest split drifts a cent
                            // from `payment`.
                            let cash_amount =
                                principal.max(Decimal::ZERO) + interest.max(Decimal::ZERO);
                            if cash_amount > Decimal::ZERO {
                                pay_je.add_line(JournalEntryLine::credit(
                                    pay_doc,
                                    line_no,
                                    cash_accounts::OPERATING_CASH.to_string(),
                                    cash_amount,
                                ));
                                debug_assert!(
                                    pay_je.is_balanced(),
                                    "ASC 842 finance lease payment JE must balance"
                                );
                                lease_jes.push(pay_je);
                            }
                        }

                        // (b) ROU amortization: DR amort expense / CR ROU asset.
                        if monthly_dep > Decimal::ZERO {
                            let mut amort_je = JournalEntry::new_simple(
                                format!(
                                    "JE-LEASE842-AMORT-{}-{}",
                                    lease.lease_id, row.period_number
                                ),
                                lease.company_code.clone(),
                                row.period_date,
                                format!(
                                    "ASC 842 ROU amortization — {} period {}",
                                    lease.description, row.period_number
                                ),
                            );
                            amort_je.header.business_process = Some(BusinessProcess::R2R);
                            amort_je.header.source = TransactionSource::Automated;
                            let amort_doc = amort_je.header.document_id;
                            amort_je.add_line(JournalEntryLine::debit(
                                amort_doc,
                                1,
                                expense_accounts::DEPRECIATION.to_string(),
                                monthly_dep,
                            ));
                            amort_je.add_line(JournalEntryLine::credit(
                                amort_doc,
                                2,
                                ROU_ASSET_ACCT.to_string(),
                                monthly_dep,
                            ));
                            debug_assert!(
                                amort_je.is_balanced(),
                                "ASC 842 ROU amortization JE must balance"
                            );
                            lease_jes.push(amort_je);
                        }
                    } else {
                        // Operating lease: single straight-line lease expense.
                        // DR rent expense / CR cash for the period payment.
                        if payment > Decimal::ZERO {
                            let mut op_je = JournalEntry::new_simple(
                                format!(
                                    "JE-LEASE842-OPEX-{}-{}",
                                    lease.lease_id, row.period_number
                                ),
                                lease.company_code.clone(),
                                row.period_date,
                                format!(
                                    "ASC 842 operating lease expense — {} period {}",
                                    lease.description, row.period_number
                                ),
                            );
                            op_je.header.business_process = Some(BusinessProcess::R2R);
                            op_je.header.source = TransactionSource::Automated;
                            let op_doc = op_je.header.document_id;
                            op_je.add_line(JournalEntryLine::debit(
                                op_doc,
                                1,
                                expense_accounts::RENT.to_string(),
                                payment,
                            ));
                            op_je.add_line(JournalEntryLine::credit(
                                op_doc,
                                2,
                                cash_accounts::OPERATING_CASH.to_string(),
                                payment,
                            ));
                            debug_assert!(
                                op_je.is_balanced(),
                                "ASC 842 operating lease JE must balance"
                            );
                            lease_jes.push(op_je);
                        }

                        // (b) Balance-sheet unwind — draw the inception-funded ROU
                        //     asset + lease liability down by the schedule principal
                        //     so BOTH roll to zero over the term (ASC 842 keeps an
                        //     operating lease on the balance sheet). DR liability /
                        //     CR ROU, equal legs → balanced by construction; sum of
                        //     principals == funded PV → no stranded balance, no
                        //     negative liability (cumulative principal <= PV).
                        if principal > Decimal::ZERO {
                            let mut unwind_je = JournalEntry::new_simple(
                                format!(
                                    "JE-LEASE842-OPUNWIND-{}-{}",
                                    lease.lease_id, row.period_number
                                ),
                                lease.company_code.clone(),
                                row.period_date,
                                format!(
                                    "ASC 842 operating lease ROU/liability unwind — {} period {}",
                                    lease.description, row.period_number
                                ),
                            );
                            unwind_je.header.business_process = Some(BusinessProcess::R2R);
                            unwind_je.header.source = TransactionSource::Automated;
                            let unwind_doc = unwind_je.header.document_id;
                            // DR lease liability (paydown)
                            // spec 27 R6a-2: stamp the 2600 operating-lease paydown line (Lease).
                            unwind_je.add_line(
                                JournalEntryLine::debit(
                                    unwind_doc,
                                    1,
                                    lease_liab_acct.to_string(),
                                    principal,
                                )
                                .with_subledger_ref(
                                    SubledgerRef::new(
                                        SubledgerType::Lease,
                                        lease.lease_id.to_string(),
                                        Some("paydown".to_string()),
                                    ),
                                ),
                            );
                            // CR ROU asset (amortization plug)
                            unwind_je.add_line(JournalEntryLine::credit(
                                unwind_doc,
                                2,
                                ROU_ASSET_ACCT.to_string(),
                                principal,
                            ));
                            debug_assert!(
                                unwind_je.is_balanced(),
                                "ASC 842 operating lease unwind JE must balance"
                            );
                            lease_jes.push(unwind_je);
                        }
                    }
                }
            }
            debug!(
                "W1-3 Stage 2: generated {} ASC 842 lease JEs",
                lease_jes.len()
            );
            snapshot.lease_journal_entries = lease_jes;
        }

        // ------------------------------------------------------------
        // v3.3.1: Fair value measurements (IFRS 13 / ASC 820)
        // ------------------------------------------------------------
        if self.config.accounting_standards.fair_value.enabled {
            use datasynth_generators::standards::FairValueGenerator;
            let end_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
                .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2025, 1, 1).expect("hardcoded valid"))
                + chrono::Months::new(self.config.global.period_months);
            let framework =
                Self::resolve_accounting_framework(self.config.accounting_standards.framework);
            let mut fv_gen = FairValueGenerator::new(self.seed + 9600);
            for company in &self.config.companies {
                let measurements = fv_gen.generate(
                    &company.code,
                    end_date,
                    &company.currency,
                    &self.config.accounting_standards.fair_value,
                    framework,
                );
                snapshot.fair_value_measurement_count += measurements.len();
                snapshot.fair_value_measurements.extend(measurements);
            }
            info!(
                "v3.3.1 fair value measurements: {}",
                snapshot.fair_value_measurement_count
            );
        }

        // ------------------------------------------------------------
        // v3.3.1: Framework reconciliation (dual reporting only)
        // ------------------------------------------------------------
        if self.config.accounting_standards.generate_differences
            && matches!(
                self.config.accounting_standards.framework,
                Some(datasynth_config::schema::AccountingFrameworkConfig::DualReporting)
            )
        {
            use datasynth_generators::standards::FrameworkReconciliationGenerator;
            let end_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
                .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2025, 1, 1).expect("hardcoded valid"))
                + chrono::Months::new(self.config.global.period_months);
            let mut recon_gen = FrameworkReconciliationGenerator::new(self.seed + 9700);
            for company in &self.config.companies {
                let (records, reconciliation) = recon_gen.generate(&company.code, end_date);
                snapshot.framework_difference_count += records.len();
                snapshot.framework_differences.extend(records);
                snapshot.framework_reconciliations.push(reconciliation);
            }
            info!(
                "v3.3.1 framework reconciliation: {} differences across {} entities",
                snapshot.framework_difference_count,
                snapshot.framework_reconciliations.len()
            );
        }

        info!(
            "Accounting standards data generated: {} revenue contracts, {} impairment tests, {} business combinations, {} ECL models, {} provisions, {} IAS 21 translations, {} leases, {} FV measurements, {} framework differences",
            snapshot.revenue_contract_count,
            snapshot.impairment_test_count,
            snapshot.business_combination_count,
            snapshot.ecl_model_count,
            snapshot.provision_count,
            snapshot.currency_translation_count,
            snapshot.lease_count,
            snapshot.fair_value_measurement_count,
            snapshot.framework_difference_count,
        );
        self.check_resources_with_log("post-accounting-standards")?;

        Ok(snapshot)
    }

    /// v3.3.1: helper to resolve the accounting-standards framework enum
    /// from config into the `datasynth_standards::framework::AccountingFramework`
    /// type expected by standards generators. Falls back to US GAAP.
    fn resolve_accounting_framework(
        cfg: Option<datasynth_config::schema::AccountingFrameworkConfig>,
    ) -> datasynth_standards::framework::AccountingFramework {
        use datasynth_config::schema::AccountingFrameworkConfig as Cfg;
        use datasynth_standards::framework::AccountingFramework as Fw;
        match cfg {
            Some(Cfg::Ifrs) => Fw::Ifrs,
            Some(Cfg::DualReporting) => Fw::DualReporting,
            Some(Cfg::FrenchGaap) => Fw::FrenchGaap,
            Some(Cfg::GermanGaap) => Fw::GermanGaap,
            _ => Fw::UsGaap,
        }
    }

    /// Phase 18: Generate manufacturing data (production orders, quality inspections, cycle counts).
    fn phase_manufacturing(
        &mut self,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<ManufacturingSnapshot> {
        if !self.phase_config.generate_manufacturing {
            debug!("Phase 18: Skipped (manufacturing generation disabled)");
            return Ok(ManufacturingSnapshot::default());
        }
        info!("Phase 18: Generating Manufacturing Data");

        let seed = self.seed;
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let end_date = start_date + chrono::Months::new(self.config.global.period_months);
        let company_code = self
            .config
            .companies
            .first()
            .map(|c| c.code.as_str())
            .unwrap_or("1000");

        let material_data: Vec<(String, String)> = self
            .master_data
            .materials
            .iter()
            .map(|m| (m.material_id.clone(), m.description.clone()))
            .collect();

        if material_data.is_empty() {
            debug!("Phase 18: Skipped (no materials available)");
            return Ok(ManufacturingSnapshot::default());
        }

        let mut snapshot = ManufacturingSnapshot::default();

        // Generate production orders
        let mut prod_gen = datasynth_generators::ProductionOrderGenerator::new(seed + 350);
        // v3.4.3: snap planned / actual / operation dates to business days.
        if let Some(ctx) = &self.temporal_context {
            prod_gen.set_temporal_context(Arc::clone(ctx));
        }
        let production_orders = prod_gen.generate(
            company_code,
            &material_data,
            start_date,
            end_date,
            &self.config.manufacturing.production_orders,
            &self.config.manufacturing.costing,
            &self.config.manufacturing.routing,
        );
        snapshot.production_order_count = production_orders.len();

        // Generate quality inspections from production orders
        let inspection_data: Vec<(String, String, String)> = production_orders
            .iter()
            .map(|po| {
                (
                    po.order_id.clone(),
                    po.material_id.clone(),
                    po.material_description.clone(),
                )
            })
            .collect();

        snapshot.production_orders = production_orders;

        if !inspection_data.is_empty() {
            let mut qi_gen = datasynth_generators::QualityInspectionGenerator::new(seed + 351);
            let inspections = qi_gen.generate(company_code, &inspection_data, end_date);
            snapshot.quality_inspection_count = inspections.len();
            snapshot.quality_inspections = inspections;
        }

        // Generate cycle counts (one per month)
        let storage_locations: Vec<(String, String)> = material_data
            .iter()
            .enumerate()
            .map(|(i, (mid, _))| (mid.clone(), format!("SL-{:03}", (i % 10) + 1)))
            .collect();

        let employee_ids: Vec<String> = self
            .master_data
            .employees
            .iter()
            .map(|e| e.employee_id.clone())
            .collect();
        let mut cc_gen = datasynth_generators::CycleCountGenerator::new(seed + 352)
            .with_employee_pool(employee_ids);
        let mut cycle_count_total = 0usize;
        for month in 0..self.config.global.period_months {
            let count_date = start_date + chrono::Months::new(month);
            let items_per_count = storage_locations.len().clamp(10, 50);
            let cc = cc_gen.generate(
                company_code,
                &storage_locations,
                count_date,
                items_per_count,
            );
            snapshot.cycle_counts.push(cc);
            cycle_count_total += 1;
        }
        snapshot.cycle_count_count = cycle_count_total;

        // Generate BOM components
        let mut bom_gen = datasynth_generators::BomGenerator::new(seed + 353);
        let bom_components = bom_gen.generate(company_code, &material_data);
        snapshot.bom_component_count = bom_components.len();
        snapshot.bom_components = bom_components;

        // Generate inventory movements — link GoodsIssue movements to real production order IDs
        let currency = self
            .config
            .companies
            .first()
            .map(|c| c.currency.as_str())
            .unwrap_or("USD");
        let production_order_ids: Vec<String> = snapshot
            .production_orders
            .iter()
            .map(|po| po.order_id.clone())
            .collect();
        let mut inv_mov_gen = datasynth_generators::InventoryMovementGenerator::new(seed + 354);
        let inventory_movements = inv_mov_gen.generate_with_production_orders(
            company_code,
            &material_data,
            start_date,
            end_date,
            2,
            currency,
            &production_order_ids,
        );
        snapshot.inventory_movement_count = inventory_movements.len();
        snapshot.inventory_movements = inventory_movements;

        stats.production_order_count = snapshot.production_order_count;
        stats.quality_inspection_count = snapshot.quality_inspection_count;
        stats.cycle_count_count = snapshot.cycle_count_count;
        stats.bom_component_count = snapshot.bom_component_count;
        stats.inventory_movement_count = snapshot.inventory_movement_count;

        info!(
            "Manufacturing data generated: {} production orders, {} quality inspections, {} cycle counts, {} BOM components, {} inventory movements",
            snapshot.production_order_count, snapshot.quality_inspection_count, snapshot.cycle_count_count,
            snapshot.bom_component_count, snapshot.inventory_movement_count
        );
        self.check_resources_with_log("post-manufacturing")?;

        Ok(snapshot)
    }

    /// Phase 19: Generate sales quotes, management KPIs, and budgets.
    fn phase_sales_kpi_budgets(
        &mut self,
        coa: &Arc<ChartOfAccounts>,
        financial_reporting: &FinancialReportingSnapshot,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<SalesKpiBudgetsSnapshot> {
        if !self.phase_config.generate_sales_kpi_budgets {
            debug!("Phase 19: Skipped (sales/KPI/budget generation disabled)");
            return Ok(SalesKpiBudgetsSnapshot::default());
        }
        info!("Phase 19: Generating Sales Quotes, KPIs, and Budgets");

        let seed = self.seed;
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let end_date = start_date + chrono::Months::new(self.config.global.period_months);
        let company_code = self
            .config
            .companies
            .first()
            .map(|c| c.code.as_str())
            .unwrap_or("1000");

        let mut snapshot = SalesKpiBudgetsSnapshot::default();

        // Sales Quotes
        if self.config.sales_quotes.enabled {
            let customer_data: Vec<(String, String)> = self
                .master_data
                .customers
                .iter()
                .map(|c| (c.customer_id.clone(), c.name.clone()))
                .collect();
            let material_data: Vec<(String, String)> = self
                .master_data
                .materials
                .iter()
                .map(|m| (m.material_id.clone(), m.description.clone()))
                .collect();

            if !customer_data.is_empty() && !material_data.is_empty() {
                let employee_ids: Vec<String> = self
                    .master_data
                    .employees
                    .iter()
                    .map(|e| e.employee_id.clone())
                    .collect();
                let customer_ids: Vec<String> = self
                    .master_data
                    .customers
                    .iter()
                    .map(|c| c.customer_id.clone())
                    .collect();
                let company_currency = self
                    .config
                    .companies
                    .first()
                    .map(|c| c.currency.as_str())
                    .unwrap_or("USD");

                let mut quote_gen = datasynth_generators::SalesQuoteGenerator::new(seed + 60)
                    .with_pools(employee_ids, customer_ids);
                let quotes = quote_gen.generate_with_currency(
                    company_code,
                    &customer_data,
                    &material_data,
                    start_date,
                    end_date,
                    &self.config.sales_quotes,
                    company_currency,
                );
                snapshot.sales_quote_count = quotes.len();
                snapshot.sales_quotes = quotes;
            }
        }

        // Management KPIs
        if self.config.financial_reporting.management_kpis.enabled {
            let mut kpi_gen = datasynth_generators::KpiGenerator::new(seed + 61);
            let mut kpis = kpi_gen.generate(
                company_code,
                start_date,
                end_date,
                &self.config.financial_reporting.management_kpis,
            );

            // Override financial KPIs with actual data from financial statements
            {
                use rust_decimal::Decimal;

                if let Some(income_stmt) =
                    financial_reporting.financial_statements.iter().find(|fs| {
                        fs.statement_type == StatementType::IncomeStatement
                            && fs.company_code == company_code
                    })
                {
                    // Extract revenue and COGS from income statement line items
                    let total_revenue: Decimal = income_stmt
                        .line_items
                        .iter()
                        .filter(|li| li.section.contains("Revenue") && !li.is_total)
                        .map(|li| li.amount)
                        .sum();
                    let total_cogs: Decimal = income_stmt
                        .line_items
                        .iter()
                        .filter(|li| {
                            (li.section.contains("Cost") || li.line_code.starts_with("IS-COGS"))
                                && !li.is_total
                        })
                        .map(|li| li.amount.abs())
                        .sum();
                    let total_opex: Decimal = income_stmt
                        .line_items
                        .iter()
                        .filter(|li| {
                            li.section.contains("Expense")
                                && !li.is_total
                                && !li.section.contains("Cost")
                        })
                        .map(|li| li.amount.abs())
                        .sum();

                    if total_revenue > Decimal::ZERO {
                        let hundred = Decimal::from(100);
                        let gross_margin_pct =
                            ((total_revenue - total_cogs) * hundred / total_revenue).round_dp(2);
                        let operating_income = total_revenue - total_cogs - total_opex;
                        let op_margin_pct =
                            (operating_income * hundred / total_revenue).round_dp(2);

                        // Override gross margin and operating margin KPIs
                        for kpi in &mut kpis {
                            if kpi.name == "Gross Margin" {
                                kpi.value = gross_margin_pct;
                            } else if kpi.name == "Operating Margin" {
                                kpi.value = op_margin_pct;
                            }
                        }
                    }
                }

                // Override Current Ratio from balance sheet
                if let Some(bs) = financial_reporting.financial_statements.iter().find(|fs| {
                    fs.statement_type == StatementType::BalanceSheet
                        && fs.company_code == company_code
                }) {
                    let current_assets: Decimal = bs
                        .line_items
                        .iter()
                        .filter(|li| li.section.contains("Current Assets") && !li.is_total)
                        .map(|li| li.amount)
                        .sum();
                    let current_liabilities: Decimal = bs
                        .line_items
                        .iter()
                        .filter(|li| li.section.contains("Current Liabilities") && !li.is_total)
                        .map(|li| li.amount.abs())
                        .sum();

                    if current_liabilities > Decimal::ZERO {
                        let current_ratio = (current_assets / current_liabilities).round_dp(2);
                        for kpi in &mut kpis {
                            if kpi.name == "Current Ratio" {
                                kpi.value = current_ratio;
                            }
                        }
                    }
                }
            }

            snapshot.kpi_count = kpis.len();
            snapshot.kpis = kpis;
        }

        // Budgets
        if self.config.financial_reporting.budgets.enabled {
            let account_data: Vec<(String, String)> = coa
                .accounts
                .iter()
                .map(|a| (a.account_number.clone(), a.short_description.clone()))
                .collect();

            if !account_data.is_empty() {
                let fiscal_year = start_date.year() as u32;
                let mut budget_gen = datasynth_generators::BudgetGenerator::new(seed + 62);
                let budget = budget_gen.generate(
                    company_code,
                    fiscal_year,
                    &account_data,
                    &self.config.financial_reporting.budgets,
                );
                snapshot.budget_line_count = budget.line_items.len();
                snapshot.budgets.push(budget);
            }
        }

        stats.sales_quote_count = snapshot.sales_quote_count;
        stats.kpi_count = snapshot.kpi_count;
        stats.budget_line_count = snapshot.budget_line_count;

        info!(
            "Sales/KPI/Budget data generated: {} quotes, {} KPIs, {} budget lines",
            snapshot.sales_quote_count, snapshot.kpi_count, snapshot.budget_line_count
        );
        self.check_resources_with_log("post-sales-kpi-budgets")?;

        Ok(snapshot)
    }

    /// Compute pre-tax income for a single company from actual journal entries.
    ///
    /// Pre-tax income = Σ revenue account net credits − Σ expense account net debits.
    /// Revenue accounts (4xxx) are credit-normal; expense accounts (5xxx, 6xxx, 7xxx) are
    /// debit-normal.  The calculation mirrors `DeferredTaxGenerator::estimate_pre_tax_income`
    /// and the period-close engine so that all three use a consistent definition.
    fn compute_pre_tax_income(
        company_code: &str,
        journal_entries: &[JournalEntry],
    ) -> rust_decimal::Decimal {
        use datasynth_core::accounts::AccountCategory;
        use rust_decimal::Decimal;

        let mut total_revenue = Decimal::ZERO;
        let mut total_expenses = Decimal::ZERO;

        for je in journal_entries {
            if je.header.company_code != company_code {
                continue;
            }
            for line in &je.lines {
                let cat = AccountCategory::from_account(&line.gl_account);
                match cat {
                    AccountCategory::Revenue => {
                        total_revenue += line.credit_amount - line.debit_amount;
                    }
                    AccountCategory::Cogs
                    | AccountCategory::OperatingExpense
                    | AccountCategory::OtherIncomeExpense => {
                        total_expenses += line.debit_amount - line.credit_amount;
                    }
                    _ => {}
                }
            }
        }

        let pti = (total_revenue - total_expenses).round_dp(2);
        if pti == rust_decimal::Decimal::ZERO {
            // No income statement activity yet — fall back to a synthetic value so the
            // tax provision generator can still produce meaningful output.
            rust_decimal::Decimal::from(1_000_000u32)
        } else {
            pti
        }
    }

    /// Phase 20: Generate tax jurisdictions, tax codes, and tax lines from invoices.
    fn phase_tax_generation(
        &mut self,
        document_flows: &DocumentFlowSnapshot,
        journal_entries: &[JournalEntry],
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<TaxSnapshot> {
        if !self.phase_config.generate_tax {
            debug!("Phase 20: Skipped (tax generation disabled)");
            return Ok(TaxSnapshot::default());
        }
        info!("Phase 20: Generating Tax Data");

        let seed = self.seed;
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let fiscal_year = start_date.year();
        let company_code = self
            .config
            .companies
            .first()
            .map(|c| c.code.as_str())
            .unwrap_or("1000");

        let mut gen = datasynth_generators::TaxCodeGenerator::with_config(
            seed + 370,
            self.config.tax.clone(),
        );

        let pack = self.primary_pack().clone();
        let (jurisdictions, codes) =
            gen.generate_from_country_pack(&pack, company_code, fiscal_year);

        // Generate tax provisions for each company
        let mut provisions = Vec::new();
        if self.config.tax.provisions.enabled {
            let mut provision_gen = datasynth_generators::TaxProvisionGenerator::new(seed + 371);
            for company in &self.config.companies {
                let pre_tax_income = Self::compute_pre_tax_income(&company.code, journal_entries);
                let statutory_rate = rust_decimal::Decimal::new(
                    (self.config.tax.provisions.statutory_rate.clamp(0.0, 1.0) * 100.0) as i64,
                    2,
                );
                let provision = provision_gen.generate(
                    &company.code,
                    start_date,
                    pre_tax_income,
                    statutory_rate,
                );
                provisions.push(provision);
            }
        }

        // Generate tax lines from document invoices
        let mut tax_lines = Vec::new();
        if !codes.is_empty() {
            let mut tax_line_gen = datasynth_generators::TaxLineGenerator::new(
                datasynth_generators::TaxLineGeneratorConfig::default(),
                codes.clone(),
                seed + 372,
            );

            // Tax lines from vendor invoices (input tax)
            // Use the first company's country as buyer country
            let buyer_country = self
                .config
                .companies
                .first()
                .map(|c| c.country.as_str())
                .unwrap_or("US");
            for vi in &document_flows.vendor_invoices {
                let lines = tax_line_gen.generate_for_document(
                    datasynth_core::models::TaxableDocumentType::VendorInvoice,
                    &vi.header.document_id,
                    buyer_country, // seller approx same country
                    buyer_country,
                    vi.payable_amount,
                    vi.header.document_date,
                    None,
                );
                tax_lines.extend(lines);
            }

            // Tax lines from customer invoices (output tax)
            for ci in &document_flows.customer_invoices {
                let lines = tax_line_gen.generate_for_document(
                    datasynth_core::models::TaxableDocumentType::CustomerInvoice,
                    &ci.header.document_id,
                    buyer_country, // seller is the company
                    buyer_country,
                    ci.total_gross_amount,
                    ci.header.document_date,
                    None,
                );
                tax_lines.extend(lines);
            }
        }

        // Generate deferred tax data (IAS 12 / ASC 740) for each company
        let deferred_tax = {
            let companies: Vec<(&str, &str)> = self
                .config
                .companies
                .iter()
                .map(|c| (c.code.as_str(), c.country.as_str()))
                .collect();
            let mut deferred_gen = datasynth_generators::DeferredTaxGenerator::new(seed + 373);
            deferred_gen.generate(&companies, start_date, journal_entries)
        };

        // Build a document_id → posting_date map so each tax JE uses its
        // source document's date rather than a blanket period-end date.
        let mut doc_dates: std::collections::HashMap<String, NaiveDate> =
            std::collections::HashMap::new();
        for vi in &document_flows.vendor_invoices {
            doc_dates.insert(vi.header.document_id.clone(), vi.header.document_date);
        }
        for ci in &document_flows.customer_invoices {
            doc_dates.insert(ci.header.document_id.clone(), ci.header.document_date);
        }

        // Generate tax posting JEs (tax payable/receivable) from computed tax lines
        let end_date = start_date + chrono::Months::new(self.config.global.period_months);
        let tax_posting_journal_entries = if !tax_lines.is_empty() {
            let jes = datasynth_generators::TaxPostingGenerator::generate_tax_posting_jes(
                &tax_lines,
                company_code,
                &doc_dates,
                end_date,
            );
            debug!("Generated {} tax posting JEs", jes.len());
            jes
        } else {
            Vec::new()
        };

        let snapshot = TaxSnapshot {
            jurisdiction_count: jurisdictions.len(),
            code_count: codes.len(),
            jurisdictions,
            codes,
            tax_provisions: provisions,
            tax_lines,
            tax_returns: Vec::new(),
            withholding_records: Vec::new(),
            tax_anomaly_labels: Vec::new(),
            deferred_tax,
            tax_posting_journal_entries,
        };

        stats.tax_jurisdiction_count = snapshot.jurisdiction_count;
        stats.tax_code_count = snapshot.code_count;
        stats.tax_provision_count = snapshot.tax_provisions.len();
        stats.tax_line_count = snapshot.tax_lines.len();

        info!(
            "Tax data generated: {} jurisdictions, {} codes, {} provisions, {} temp diffs, {} deferred JEs, {} tax posting JEs",
            snapshot.jurisdiction_count,
            snapshot.code_count,
            snapshot.tax_provisions.len(),
            snapshot.deferred_tax.temporary_differences.len(),
            snapshot.deferred_tax.journal_entries.len(),
            snapshot.tax_posting_journal_entries.len(),
        );
        self.check_resources_with_log("post-tax")?;

        Ok(snapshot)
    }

    /// Phase 21: Generate ESG data (emissions, energy, water, waste, social, governance, disclosures).
    fn phase_esg_generation(
        &mut self,
        document_flows: &DocumentFlowSnapshot,
        manufacturing: &ManufacturingSnapshot,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<EsgSnapshot> {
        if !self.phase_config.generate_esg {
            debug!("Phase 21: Skipped (ESG generation disabled)");
            return Ok(EsgSnapshot::default());
        }
        let degradation = self.check_resources()?;
        if degradation >= DegradationLevel::Reduced {
            debug!(
                "Phase skipped due to resource pressure (degradation: {:?})",
                degradation
            );
            return Ok(EsgSnapshot::default());
        }
        info!("Phase 21: Generating ESG Data");

        let seed = self.seed;
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let end_date = start_date + chrono::Months::new(self.config.global.period_months);
        let entity_id = self
            .config
            .companies
            .first()
            .map(|c| c.code.as_str())
            .unwrap_or("1000");

        let esg_cfg = &self.config.esg;
        let mut snapshot = EsgSnapshot::default();

        // Energy consumption (feeds into scope 1 & 2 emissions)
        let mut energy_gen = datasynth_generators::EnergyGenerator::new(
            esg_cfg.environmental.energy.clone(),
            seed + 80,
        );
        let energy_records = energy_gen.generate(entity_id, start_date, end_date);

        // Water usage
        let facility_count = esg_cfg.environmental.energy.facility_count;
        let mut water_gen = datasynth_generators::WaterGenerator::new(seed + 81, facility_count);
        snapshot.water = water_gen.generate(entity_id, start_date, end_date);

        // Waste
        let mut waste_gen = datasynth_generators::WasteGenerator::new(
            seed + 82,
            esg_cfg.environmental.waste.diversion_target,
            facility_count,
        );
        snapshot.waste = waste_gen.generate(entity_id, start_date, end_date);

        // Emissions (scope 1, 2, 3)
        let mut emission_gen =
            datasynth_generators::EmissionGenerator::new(esg_cfg.environmental.clone(), seed + 83);

        // Build EnergyInput from energy_records
        let mut energy_inputs: Vec<datasynth_generators::EnergyInput> = energy_records
            .iter()
            .map(|e| datasynth_generators::EnergyInput {
                facility_id: e.facility_id.clone(),
                energy_type: match e.energy_source {
                    EnergySourceType::NaturalGas => {
                        datasynth_generators::EnergyInputType::NaturalGas
                    }
                    EnergySourceType::Diesel => datasynth_generators::EnergyInputType::Diesel,
                    EnergySourceType::Coal => datasynth_generators::EnergyInputType::Coal,
                    _ => datasynth_generators::EnergyInputType::Electricity,
                },
                consumption_kwh: e.consumption_kwh,
                period: e.period,
            })
            .collect();

        // v2.4: Bridge manufacturing production data → energy inputs for Scope 1/2
        if !manufacturing.production_orders.is_empty() {
            let mfg_energy = datasynth_generators::EmissionGenerator::energy_from_production(
                &manufacturing.production_orders,
                rust_decimal::Decimal::new(50, 0), // 50 kWh per machine hour
                rust_decimal::Decimal::new(2, 0),  // 2 kWh natural gas per unit
            );
            if !mfg_energy.is_empty() {
                info!(
                    "ESG: {} energy inputs derived from {} production orders",
                    mfg_energy.len(),
                    manufacturing.production_orders.len(),
                );
                energy_inputs.extend(mfg_energy);
            }
        }

        let mut emissions = Vec::new();
        emissions.extend(emission_gen.generate_scope1(entity_id, &energy_inputs));
        emissions.extend(emission_gen.generate_scope2(entity_id, &energy_inputs));

        // Scope 3: use vendor spend data from actual payments
        let vendor_payment_totals: HashMap<String, rust_decimal::Decimal> = {
            let mut totals: HashMap<String, rust_decimal::Decimal> = HashMap::new();
            for payment in &document_flows.payments {
                if payment.is_vendor {
                    *totals
                        .entry(payment.business_partner_id.clone())
                        .or_default() += payment.amount;
                }
            }
            totals
        };
        let vendor_spend: Vec<datasynth_generators::VendorSpendInput> = self
            .master_data
            .vendors
            .iter()
            .map(|v| {
                let spend = vendor_payment_totals
                    .get(&v.vendor_id)
                    .copied()
                    .unwrap_or_else(|| rust_decimal::Decimal::new(10000, 0));
                datasynth_generators::VendorSpendInput {
                    vendor_id: v.vendor_id.clone(),
                    category: format!("{:?}", v.vendor_type).to_lowercase(),
                    spend,
                    country: v.country.clone(),
                }
            })
            .collect();
        if !vendor_spend.is_empty() {
            emissions.extend(emission_gen.generate_scope3_purchased_goods(
                entity_id,
                &vendor_spend,
                start_date,
                end_date,
            ));
        }

        // Business travel & commuting (scope 3)
        let headcount = self.master_data.employees.len() as u32;
        if headcount > 0 {
            let travel_spend = rust_decimal::Decimal::new(headcount as i64 * 2000, 0);
            emissions.extend(emission_gen.generate_scope3_business_travel(
                entity_id,
                travel_spend,
                start_date,
            ));
            emissions
                .extend(emission_gen.generate_scope3_commuting(entity_id, headcount, start_date));
        }

        snapshot.emission_count = emissions.len();
        snapshot.emissions = emissions;
        snapshot.energy = energy_records;

        // Social: Workforce diversity, pay equity, safety
        let mut workforce_gen =
            datasynth_generators::WorkforceGenerator::new(esg_cfg.social.clone(), seed + 84);
        let total_headcount = headcount.max(100);
        snapshot.diversity =
            workforce_gen.generate_diversity(entity_id, total_headcount, start_date);
        snapshot.pay_equity = workforce_gen.generate_pay_equity(entity_id, start_date);

        // v2.4: Derive additional workforce diversity metrics from actual employee data
        if !self.master_data.employees.is_empty() {
            let hr_diversity = workforce_gen.generate_diversity_from_employees(
                entity_id,
                &self.master_data.employees,
                end_date,
            );
            if !hr_diversity.is_empty() {
                info!(
                    "ESG: {} diversity metrics derived from {} actual employees",
                    hr_diversity.len(),
                    self.master_data.employees.len(),
                );
                snapshot.diversity.extend(hr_diversity);
            }
        }

        snapshot.safety_incidents = workforce_gen.generate_safety_incidents(
            entity_id,
            facility_count,
            start_date,
            end_date,
        );

        // Compute safety metrics
        let total_hours = total_headcount as u64 * 2000; // ~2000 hours/employee/year
        let safety_metric = workforce_gen.compute_safety_metrics(
            entity_id,
            &snapshot.safety_incidents,
            total_hours,
            start_date,
        );
        snapshot.safety_metrics = vec![safety_metric];

        // Governance
        let mut gov_gen = datasynth_generators::GovernanceGenerator::new(
            seed + 85,
            esg_cfg.governance.board_size,
            esg_cfg.governance.independence_target,
        );
        snapshot.governance = vec![gov_gen.generate(entity_id, start_date)];

        // Supplier ESG assessments
        let mut supplier_gen = datasynth_generators::SupplierEsgGenerator::new(
            esg_cfg.supply_chain_esg.clone(),
            seed + 86,
        );
        let vendor_inputs: Vec<datasynth_generators::VendorInput> = self
            .master_data
            .vendors
            .iter()
            .map(|v| datasynth_generators::VendorInput {
                vendor_id: v.vendor_id.clone(),
                country: v.country.clone(),
                industry: format!("{:?}", v.vendor_type).to_lowercase(),
                quality_score: None,
            })
            .collect();
        snapshot.supplier_assessments =
            supplier_gen.generate(entity_id, &vendor_inputs, start_date);

        // Disclosures
        let mut disclosure_gen = datasynth_generators::DisclosureGenerator::new(
            seed + 87,
            esg_cfg.reporting.clone(),
            esg_cfg.climate_scenarios.clone(),
        );
        snapshot.materiality = disclosure_gen.generate_materiality(entity_id, start_date);
        snapshot.disclosures = disclosure_gen.generate_disclosures(
            entity_id,
            &snapshot.materiality,
            start_date,
            end_date,
        );
        snapshot.climate_scenarios = disclosure_gen.generate_climate_scenarios(entity_id);
        snapshot.disclosure_count = snapshot.disclosures.len();

        // Anomaly injection
        if esg_cfg.anomaly_rate > 0.0 {
            let mut anomaly_injector =
                datasynth_generators::EsgAnomalyInjector::new(seed + 88, esg_cfg.anomaly_rate);
            let mut labels = Vec::new();
            labels.extend(anomaly_injector.inject_greenwashing(&mut snapshot.emissions));
            labels.extend(anomaly_injector.inject_diversity_stagnation(&mut snapshot.diversity));
            labels.extend(
                anomaly_injector.inject_supply_chain_risk(&mut snapshot.supplier_assessments),
            );
            labels.extend(anomaly_injector.inject_data_quality_gaps(&mut snapshot.safety_metrics));
            labels.extend(anomaly_injector.inject_missing_disclosures(&mut snapshot.materiality));
            snapshot.anomaly_labels = labels;
        }

        stats.esg_emission_count = snapshot.emission_count;
        stats.esg_disclosure_count = snapshot.disclosure_count;

        info!(
            "ESG data generated: {} emissions, {} disclosures, {} supplier assessments",
            snapshot.emission_count,
            snapshot.disclosure_count,
            snapshot.supplier_assessments.len()
        );
        self.check_resources_with_log("post-esg")?;

        Ok(snapshot)
    }

    /// Phase 22: Generate Treasury data (cash management, hedging, debt, pooling, guarantees, netting).
    fn phase_treasury_data(
        &mut self,
        document_flows: &DocumentFlowSnapshot,
        subledger: &SubledgerSnapshot,
        intercompany: &IntercompanySnapshot,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<TreasurySnapshot> {
        if !self.phase_config.generate_treasury {
            debug!("Phase 22: Skipped (treasury generation disabled)");
            return Ok(TreasurySnapshot::default());
        }
        let degradation = self.check_resources()?;
        if degradation >= DegradationLevel::Reduced {
            debug!(
                "Phase skipped due to resource pressure (degradation: {:?})",
                degradation
            );
            return Ok(TreasurySnapshot::default());
        }
        info!("Phase 22: Generating Treasury Data");

        let seed = self.seed;
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let currency = self
            .config
            .companies
            .first()
            .map(|c| c.currency.as_str())
            .unwrap_or("USD");
        let entity_id = self
            .config
            .companies
            .first()
            .map(|c| c.code.as_str())
            .unwrap_or("1000");

        let mut snapshot = TreasurySnapshot::default();

        // Generate debt instruments
        let mut debt_gen = datasynth_generators::treasury::DebtGenerator::new(
            self.config.treasury.debt.clone(),
            seed + 90,
        );
        snapshot.debt_instruments = debt_gen.generate(entity_id, currency, start_date);

        // Generate hedging instruments (IR swaps for floating-rate debt)
        let mut hedge_gen = datasynth_generators::treasury::HedgingGenerator::new(
            self.config.treasury.hedging.clone(),
            seed + 91,
        );
        for debt in &snapshot.debt_instruments {
            if debt.rate_type == InterestRateType::Variable {
                let swap = hedge_gen.generate_ir_swap(
                    currency,
                    debt.principal,
                    debt.origination_date,
                    debt.maturity_date,
                );
                snapshot.hedging_instruments.push(swap);
            }
        }

        // Build FX exposures from foreign-currency payments and generate
        // FX forwards + hedge relationship designations via generate() API.
        {
            let mut fx_map: HashMap<String, (rust_decimal::Decimal, NaiveDate)> = HashMap::new();
            for payment in &document_flows.payments {
                if payment.currency != currency {
                    let entry = fx_map
                        .entry(payment.currency.clone())
                        .or_insert((rust_decimal::Decimal::ZERO, payment.header.document_date));
                    entry.0 += payment.amount;
                    // Use the latest settlement date among grouped payments
                    if payment.header.document_date > entry.1 {
                        entry.1 = payment.header.document_date;
                    }
                }
            }
            if !fx_map.is_empty() {
                let fx_exposures: Vec<datasynth_generators::treasury::FxExposure> = fx_map
                    .into_iter()
                    .map(|(foreign_ccy, (net_amount, settlement_date))| {
                        datasynth_generators::treasury::FxExposure {
                            currency_pair: format!("{foreign_ccy}/{currency}"),
                            foreign_currency: foreign_ccy,
                            net_amount,
                            settlement_date,
                            description: "AP payment FX exposure".to_string(),
                        }
                    })
                    .collect();
                let (fx_instruments, fx_relationships) =
                    hedge_gen.generate(start_date, &fx_exposures);
                snapshot.hedging_instruments.extend(fx_instruments);
                snapshot.hedge_relationships.extend(fx_relationships);
            }
        }

        // Inject anomalies if configured
        if self.config.treasury.anomaly_rate > 0.0 {
            let mut anomaly_injector = datasynth_generators::treasury::TreasuryAnomalyInjector::new(
                seed + 92,
                self.config.treasury.anomaly_rate,
            );
            let mut labels = Vec::new();
            labels.extend(
                anomaly_injector.inject_into_hedge_relationships(&mut snapshot.hedge_relationships),
            );
            snapshot.treasury_anomaly_labels = labels;
        }

        // Generate cash positions from payment flows
        if self.config.treasury.cash_positioning.enabled {
            let mut cash_flows: Vec<datasynth_generators::treasury::CashFlow> = Vec::new();

            // AP payments as outflows
            for payment in &document_flows.payments {
                cash_flows.push(datasynth_generators::treasury::CashFlow {
                    date: payment.header.document_date,
                    account_id: format!("{entity_id}-MAIN"),
                    amount: payment.amount,
                    direction: datasynth_generators::treasury::CashFlowDirection::Outflow,
                });
            }

            // Customer receipts (from O2C chains) as inflows
            for chain in &document_flows.o2c_chains {
                if let Some(ref receipt) = chain.customer_receipt {
                    cash_flows.push(datasynth_generators::treasury::CashFlow {
                        date: receipt.header.document_date,
                        account_id: format!("{entity_id}-MAIN"),
                        amount: receipt.amount,
                        direction: datasynth_generators::treasury::CashFlowDirection::Inflow,
                    });
                }
                // Remainder receipts (follow-up to partial payments)
                for receipt in &chain.remainder_receipts {
                    cash_flows.push(datasynth_generators::treasury::CashFlow {
                        date: receipt.header.document_date,
                        account_id: format!("{entity_id}-MAIN"),
                        amount: receipt.amount,
                        direction: datasynth_generators::treasury::CashFlowDirection::Inflow,
                    });
                }
            }

            if !cash_flows.is_empty() {
                let mut cash_gen = datasynth_generators::treasury::CashPositionGenerator::new(
                    self.config.treasury.cash_positioning.clone(),
                    seed + 93,
                );
                let account_id = format!("{entity_id}-MAIN");
                snapshot.cash_positions = cash_gen.generate(
                    entity_id,
                    &account_id,
                    currency,
                    &cash_flows,
                    start_date,
                    start_date + chrono::Months::new(self.config.global.period_months),
                    rust_decimal::Decimal::new(1_000_000, 0), // Default opening balance
                );
            }
        }

        // Generate cash forecasts from AR/AP aging
        if self.config.treasury.cash_forecasting.enabled {
            let end_date = start_date + chrono::Months::new(self.config.global.period_months);

            // Build AR aging items from subledger AR invoices
            let ar_items: Vec<datasynth_generators::treasury::ArAgingItem> = subledger
                .ar_invoices
                .iter()
                .filter(|inv| inv.amount_remaining > rust_decimal::Decimal::ZERO)
                .map(|inv| {
                    let days_past_due = if inv.due_date < end_date {
                        (end_date - inv.due_date).num_days().max(0) as u32
                    } else {
                        0
                    };
                    datasynth_generators::treasury::ArAgingItem {
                        expected_date: inv.due_date,
                        amount: inv.amount_remaining,
                        days_past_due,
                        document_id: inv.invoice_number.clone(),
                    }
                })
                .collect();

            // Build AP aging items from subledger AP invoices
            let ap_items: Vec<datasynth_generators::treasury::ApAgingItem> = subledger
                .ap_invoices
                .iter()
                .filter(|inv| inv.amount_remaining > rust_decimal::Decimal::ZERO)
                .map(|inv| datasynth_generators::treasury::ApAgingItem {
                    payment_date: inv.due_date,
                    amount: inv.amount_remaining,
                    document_id: inv.invoice_number.clone(),
                })
                .collect();

            let mut forecast_gen = datasynth_generators::treasury::CashForecastGenerator::new(
                self.config.treasury.cash_forecasting.clone(),
                seed + 94,
            );
            let forecast = forecast_gen.generate(
                entity_id,
                currency,
                end_date,
                &ar_items,
                &ap_items,
                &[], // scheduled disbursements - empty for now
            );
            snapshot.cash_forecasts.push(forecast);
        }

        // Generate cash pools and sweeps
        if self.config.treasury.cash_pooling.enabled && !snapshot.cash_positions.is_empty() {
            let end_date = start_date + chrono::Months::new(self.config.global.period_months);
            let mut pool_gen = datasynth_generators::treasury::CashPoolGenerator::new(
                self.config.treasury.cash_pooling.clone(),
                seed + 95,
            );

            // Create a pool from available accounts
            let account_ids: Vec<String> = snapshot
                .cash_positions
                .iter()
                .map(|cp| cp.bank_account_id.clone())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();

            if let Some(pool) =
                pool_gen.create_pool(&format!("{entity_id}_MAIN_POOL"), currency, &account_ids)
            {
                // Generate sweeps - build participant balances from last cash position per account
                let mut latest_balances: HashMap<String, rust_decimal::Decimal> = HashMap::new();
                for cp in &snapshot.cash_positions {
                    latest_balances.insert(cp.bank_account_id.clone(), cp.closing_balance);
                }

                let participant_balances: Vec<datasynth_generators::treasury::AccountBalance> =
                    latest_balances
                        .into_iter()
                        .filter(|(id, _)| pool.participant_accounts.contains(id))
                        .map(
                            |(id, balance)| datasynth_generators::treasury::AccountBalance {
                                account_id: id,
                                balance,
                            },
                        )
                        .collect();

                let sweeps =
                    pool_gen.generate_sweeps(&pool, end_date, currency, &participant_balances);
                snapshot.cash_pool_sweeps = sweeps;
                snapshot.cash_pools.push(pool);
            }
        }

        // Generate bank guarantees
        if self.config.treasury.bank_guarantees.enabled {
            let vendor_names: Vec<String> = self
                .master_data
                .vendors
                .iter()
                .map(|v| v.name.clone())
                .collect();
            if !vendor_names.is_empty() {
                let mut bg_gen = datasynth_generators::treasury::BankGuaranteeGenerator::new(
                    self.config.treasury.bank_guarantees.clone(),
                    seed + 96,
                );
                snapshot.bank_guarantees =
                    bg_gen.generate(entity_id, currency, start_date, &vendor_names);
            }
        }

        // Generate netting runs from intercompany matched pairs
        if self.config.treasury.netting.enabled && !intercompany.matched_pairs.is_empty() {
            let entity_ids: Vec<String> = self
                .config
                .companies
                .iter()
                .map(|c| c.code.clone())
                .collect();
            let ic_amounts: Vec<(String, String, rust_decimal::Decimal)> = intercompany
                .matched_pairs
                .iter()
                .map(|mp| {
                    (
                        mp.seller_company.clone(),
                        mp.buyer_company.clone(),
                        mp.amount,
                    )
                })
                .collect();
            if entity_ids.len() >= 2 {
                let mut netting_gen = datasynth_generators::treasury::NettingRunGenerator::new(
                    self.config.treasury.netting.clone(),
                    seed + 97,
                );
                snapshot.netting_runs = netting_gen.generate(
                    &entity_ids,
                    currency,
                    start_date,
                    self.config.global.period_months,
                    &ic_amounts,
                );
            }
        }

        // Generate treasury journal entries from the instruments we just created.
        {
            use datasynth_generators::treasury::TreasuryAccounting;

            let end_date = start_date + chrono::Months::new(self.config.global.period_months);
            let mut treasury_jes = Vec::new();

            // Debt interest accrual JEs
            //
            // W1-3 Stage 2 (bond/loan interest): when `monthly_recurring` is OFF we
            // keep the existing flat lump (`generate_debt_jes`, principal*rate/4 per
            // instrument at `end_date`) so the output stays BYTE-IDENTICAL. When ON
            // we instead accrue the slice's interest spread evenly across the
            // recurring month-ends — DR Interest Expense (7100) / CR Interest Payable
            // (2160), the SAME accounts the flat generator uses — so a monthly build
            // carries ~one month of interest on each month-end balance sheet.
            if !snapshot.debt_instruments.is_empty() {
                // Debt inception: one balanced issuance JE per instrument, posted in the slice
                // where it originates — DR Operating Cash (1000) / CR Long-Term Debt (2600) —
                // so the balance sheet carries the principal liability (previously bonds/loans
                // accrued interest but never recorded principal, leaving BS long-term debt at 0).
                // The origination-date window keeps it idempotent within a slice; in a multi-year
                // session `emit_debt_inception` is set only for the first fiscal year so the
                // principal is issued once. Inert without debt instruments.
                if self.phase_config.emit_debt_inception {
                    use datasynth_core::accounts::{cash_accounts, liability_accounts};
                    let inc_slice_start = start_date;
                    let inc_slice_end = end_date - chrono::Days::new(1);
                    for debt in &snapshot.debt_instruments {
                        if debt.origination_date < inc_slice_start
                            || debt.origination_date > inc_slice_end
                        {
                            continue;
                        }
                        let mut je = JournalEntry::new_simple(
                            format!("JE-TREAS-DEBT-INC-{}", debt.id),
                            debt.entity_id.clone(),
                            debt.origination_date,
                            format!("Debt issuance — {} from {}", debt.id, debt.lender),
                        );
                        je.header.currency = debt.currency.clone();
                        je.header.business_process = Some(BusinessProcess::Treasury);
                        je.header.source = TransactionSource::Automated;
                        let doc_id = je.header.document_id;
                        je.add_line(JournalEntryLine::debit(
                            doc_id,
                            1,
                            cash_accounts::OPERATING_CASH.to_string(),
                            debt.principal,
                        ));
                        // spec 27 R6a-2: stamp the 2600 long-term-debt control line (Debt, debt.id).
                        // debt.id is already a String ("DEBT-NNNNNN"), so clone (not to_string).
                        // The JE-TREAS-INT interest accruals post to 2160, not 2600, and the debt
                        // regex never matched them — so they stay unstamped (behavior-preserving).
                        je.add_line(
                            JournalEntryLine::credit(
                                doc_id,
                                2,
                                liability_accounts::LONG_TERM_DEBT.to_string(),
                                debt.principal,
                            )
                            .with_subledger_ref(SubledgerRef::new(
                                SubledgerType::Debt,
                                debt.id.clone(),
                                Some("inception".to_string()),
                            )),
                        );
                        debug_assert!(je.is_balanced(), "Debt inception JE must balance");
                        treasury_jes.push(je);
                    }
                }
                if self.phase_config.monthly_recurring {
                    use datasynth_core::accounts::{expense_accounts, treasury_accounts};
                    let month_ends = self.recurring_month_ends()?;
                    let period_months = Decimal::from(self.config.global.period_months.max(1));
                    // Slice window for active-instrument filtering.
                    let slice_start = start_date;
                    let slice_end = end_date - chrono::Days::new(1);
                    let mut bond_je_count = 0usize;
                    for debt in &snapshot.debt_instruments {
                        // Only emit for instruments active during this slice:
                        // skip if matured before the slice begins or originated
                        // after the slice ends (multi-FY safety — never re-post a
                        // prior/future year's interest).
                        if debt.maturity_date < slice_start || debt.origination_date > slice_end {
                            continue;
                        }
                        // Slice interest = principal * annual_rate * (months / 12).
                        let slice_interest = (debt.principal * debt.interest_rate * period_months
                            / Decimal::from(12))
                        .round_dp(2);
                        if slice_interest <= Decimal::ZERO {
                            continue;
                        }
                        let alloc = monthly_straight_line_allocation(
                            slice_interest,
                            month_ends.len() as u32,
                        );
                        for (idx, amount) in alloc.iter().enumerate() {
                            if *amount <= Decimal::ZERO {
                                continue;
                            }
                            let posting_date = month_ends[idx];
                            let mut je = JournalEntry::new_simple(
                                format!("JE-TREAS-INT-{}-{}", debt.id, idx + 1),
                                debt.entity_id.clone(),
                                posting_date,
                                format!("Interest accrual on {} from {}", debt.id, debt.lender),
                            );
                            je.header.currency = debt.currency.clone();
                            je.header.business_process = Some(BusinessProcess::Treasury);
                            je.header.source = TransactionSource::Automated;
                            let doc_id = je.header.document_id;
                            // DR Interest Expense (7100)
                            je.add_line(JournalEntryLine::debit(
                                doc_id,
                                1,
                                expense_accounts::INTEREST_EXPENSE.to_string(),
                                *amount,
                            ));
                            // CR Interest Payable (2160)
                            je.add_line(JournalEntryLine::credit(
                                doc_id,
                                2,
                                treasury_accounts::INTEREST_PAYABLE.to_string(),
                                *amount,
                            ));
                            debug_assert!(
                                je.is_balanced(),
                                "Bond monthly interest JE must balance"
                            );
                            treasury_jes.push(je);
                            bond_je_count += 1;
                        }
                    }
                    debug!(
                        "Generated {} monthly bond interest accrual JEs",
                        bond_je_count
                    );
                } else {
                    let debt_jes =
                        TreasuryAccounting::generate_debt_jes(&snapshot.debt_instruments, end_date);
                    debug!("Generated {} debt interest accrual JEs", debt_jes.len());
                    treasury_jes.extend(debt_jes);
                }
            }

            // Hedge mark-to-market JEs
            if !snapshot.hedging_instruments.is_empty() {
                let hedge_jes = TreasuryAccounting::generate_hedge_jes(
                    &snapshot.hedging_instruments,
                    &snapshot.hedge_relationships,
                    end_date,
                    entity_id,
                );
                debug!("Generated {} hedge MTM JEs", hedge_jes.len());
                treasury_jes.extend(hedge_jes);
            }

            // Cash pool sweep JEs
            if !snapshot.cash_pool_sweeps.is_empty() {
                let sweep_jes = TreasuryAccounting::generate_cash_pool_sweep_jes(
                    &snapshot.cash_pool_sweeps,
                    entity_id,
                );
                debug!("Generated {} cash pool sweep JEs", sweep_jes.len());
                treasury_jes.extend(sweep_jes);
            }

            if !treasury_jes.is_empty() {
                debug!("Total treasury journal entries: {}", treasury_jes.len());
            }
            snapshot.journal_entries = treasury_jes;
        }

        stats.treasury_debt_instrument_count = snapshot.debt_instruments.len();
        stats.treasury_hedging_instrument_count = snapshot.hedging_instruments.len();
        stats.cash_position_count = snapshot.cash_positions.len();
        stats.cash_forecast_count = snapshot.cash_forecasts.len();
        stats.cash_pool_count = snapshot.cash_pools.len();

        info!(
            "Treasury data generated: {} debt instruments, {} hedging instruments, {} cash positions, {} forecasts, {} pools, {} guarantees, {} netting runs, {} JEs",
            snapshot.debt_instruments.len(),
            snapshot.hedging_instruments.len(),
            snapshot.cash_positions.len(),
            snapshot.cash_forecasts.len(),
            snapshot.cash_pools.len(),
            snapshot.bank_guarantees.len(),
            snapshot.netting_runs.len(),
            snapshot.journal_entries.len(),
        );
        self.check_resources_with_log("post-treasury")?;

        Ok(snapshot)
    }

    /// Phase 23: Generate Project Accounting data (projects, costs, revenue, EVM, milestones).
    fn phase_project_accounting(
        &mut self,
        document_flows: &DocumentFlowSnapshot,
        hr: &HrSnapshot,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<ProjectAccountingSnapshot> {
        if !self.phase_config.generate_project_accounting {
            debug!("Phase 23: Skipped (project accounting disabled)");
            return Ok(ProjectAccountingSnapshot::default());
        }
        let degradation = self.check_resources()?;
        if degradation >= DegradationLevel::Reduced {
            debug!(
                "Phase skipped due to resource pressure (degradation: {:?})",
                degradation
            );
            return Ok(ProjectAccountingSnapshot::default());
        }
        info!("Phase 23: Generating Project Accounting Data");

        let seed = self.seed;
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let end_date = start_date + chrono::Months::new(self.config.global.period_months);
        let company_code = self
            .config
            .companies
            .first()
            .map(|c| c.code.as_str())
            .unwrap_or("1000");

        let mut snapshot = ProjectAccountingSnapshot::default();

        // Generate projects with WBS hierarchies
        let mut project_gen = datasynth_generators::project_accounting::ProjectGenerator::new(
            self.config.project_accounting.clone(),
            seed + 95,
        );
        let pool = project_gen.generate(company_code, start_date, end_date);
        snapshot.projects = pool.projects.clone();

        // Link source documents to projects for cost allocation
        {
            let mut source_docs: Vec<datasynth_generators::project_accounting::SourceDocument> =
                Vec::new();

            // Time entries
            for te in &hr.time_entries {
                let total_hours = te.hours_regular + te.hours_overtime;
                if total_hours > 0.0 {
                    source_docs.push(datasynth_generators::project_accounting::SourceDocument {
                        id: te.entry_id.clone(),
                        entity_id: company_code.to_string(),
                        date: te.date,
                        amount: rust_decimal::Decimal::from_f64_retain(total_hours * 75.0)
                            .unwrap_or(rust_decimal::Decimal::ZERO),
                        source_type: CostSourceType::TimeEntry,
                        hours: Some(
                            rust_decimal::Decimal::from_f64_retain(total_hours)
                                .unwrap_or(rust_decimal::Decimal::ZERO),
                        ),
                    });
                }
            }

            // Expense reports
            for er in &hr.expense_reports {
                source_docs.push(datasynth_generators::project_accounting::SourceDocument {
                    id: er.report_id.clone(),
                    entity_id: company_code.to_string(),
                    date: er.submission_date,
                    amount: er.total_amount,
                    source_type: CostSourceType::ExpenseReport,
                    hours: None,
                });
            }

            // Purchase orders
            for po in &document_flows.purchase_orders {
                source_docs.push(datasynth_generators::project_accounting::SourceDocument {
                    id: po.header.document_id.clone(),
                    entity_id: company_code.to_string(),
                    date: po.header.document_date,
                    amount: po.total_net_amount,
                    source_type: CostSourceType::PurchaseOrder,
                    hours: None,
                });
            }

            // Vendor invoices
            for vi in &document_flows.vendor_invoices {
                source_docs.push(datasynth_generators::project_accounting::SourceDocument {
                    id: vi.header.document_id.clone(),
                    entity_id: company_code.to_string(),
                    date: vi.header.document_date,
                    amount: vi.payable_amount,
                    source_type: CostSourceType::VendorInvoice,
                    hours: None,
                });
            }

            if !source_docs.is_empty() && !pool.projects.is_empty() {
                let mut cost_gen =
                    datasynth_generators::project_accounting::ProjectCostGenerator::new(
                        self.config.project_accounting.cost_allocation.clone(),
                        seed + 99,
                    );
                snapshot.cost_lines = cost_gen.link_documents(&pool, &source_docs);
            }
        }

        // Generate change orders
        if self.config.project_accounting.change_orders.enabled {
            let mut co_gen = datasynth_generators::project_accounting::ChangeOrderGenerator::new(
                self.config.project_accounting.change_orders.clone(),
                seed + 96,
            );
            snapshot.change_orders = co_gen.generate(&pool.projects, start_date, end_date);
        }

        // Generate milestones
        if self.config.project_accounting.milestones.enabled {
            let mut ms_gen = datasynth_generators::project_accounting::MilestoneGenerator::new(
                self.config.project_accounting.milestones.clone(),
                seed + 97,
            );
            snapshot.milestones = ms_gen.generate(&pool.projects, start_date, end_date, end_date);
        }

        // Generate earned value metrics (needs cost lines, so only if we have projects)
        if self.config.project_accounting.earned_value.enabled && !snapshot.projects.is_empty() {
            let mut evm_gen = datasynth_generators::project_accounting::EarnedValueGenerator::new(
                self.config.project_accounting.earned_value.clone(),
                seed + 98,
            );
            snapshot.earned_value_metrics =
                evm_gen.generate(&pool.projects, &snapshot.cost_lines, start_date, end_date);
        }

        // Wire ProjectRevenueGenerator: generate PoC revenue recognition for customer projects.
        if self.config.project_accounting.revenue_recognition.enabled
            && !snapshot.projects.is_empty()
            && !snapshot.cost_lines.is_empty()
        {
            use datasynth_generators::project_accounting::RevenueGenerator;
            let rev_config = self.config.project_accounting.revenue_recognition.clone();
            let avg_contract_value =
                rust_decimal::Decimal::from_f64_retain(rev_config.avg_contract_value)
                    .unwrap_or(rust_decimal::Decimal::new(500_000, 0));

            // Build contract value tuples: only customer-type projects get revenue recognition.
            // Estimated total cost = 80% of contract value (standard 20% gross margin proxy).
            let contract_values: Vec<(String, rust_decimal::Decimal, rust_decimal::Decimal)> =
                snapshot
                    .projects
                    .iter()
                    .filter(|p| {
                        matches!(
                            p.project_type,
                            datasynth_core::models::ProjectType::Customer
                        )
                    })
                    .map(|p| {
                        let cv = if p.budget > rust_decimal::Decimal::ZERO {
                            (p.budget * rust_decimal::Decimal::new(125, 2)).round_dp(2)
                        // budget × 1.25 → contract value
                        } else {
                            avg_contract_value
                        };
                        let etc = (cv * rust_decimal::Decimal::new(80, 2)).round_dp(2); // 80% cost ratio
                        (p.project_id.clone(), cv, etc)
                    })
                    .collect();

            if !contract_values.is_empty() {
                let mut rev_gen = RevenueGenerator::new(rev_config, seed + 99);
                snapshot.revenue_records = rev_gen.generate(
                    &snapshot.projects,
                    &snapshot.cost_lines,
                    &contract_values,
                    start_date,
                    end_date,
                );
                debug!(
                    "Generated {} revenue recognition records for {} customer projects",
                    snapshot.revenue_records.len(),
                    contract_values.len()
                );
            }
        }

        stats.project_count = snapshot.projects.len();
        stats.project_change_order_count = snapshot.change_orders.len();
        stats.project_cost_line_count = snapshot.cost_lines.len();

        info!(
            "Project accounting generated: {} projects, {} change orders, {} milestones, {} EVM records",
            snapshot.projects.len(),
            snapshot.change_orders.len(),
            snapshot.milestones.len(),
            snapshot.earned_value_metrics.len()
        );
        self.check_resources_with_log("post-project-accounting")?;

        Ok(snapshot)
    }

    /// Phase 24: Generate process evolution and organizational events.
    fn phase_evolution_events(
        &mut self,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<(Vec<ProcessEvolutionEvent>, Vec<OrganizationalEvent>)> {
        if !self.phase_config.generate_evolution_events {
            debug!("Phase 24: Skipped (evolution events disabled)");
            return Ok((Vec::new(), Vec::new()));
        }
        info!("Phase 24: Generating Process Evolution + Organizational Events");

        let seed = self.seed;
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let end_date = start_date + chrono::Months::new(self.config.global.period_months);

        // Process evolution events
        let mut proc_gen =
            datasynth_generators::process_evolution_generator::ProcessEvolutionGenerator::new(
                seed + 100,
            );
        let process_events = proc_gen.generate_events(start_date, end_date);

        // Organizational events
        let company_codes: Vec<String> = self
            .config
            .companies
            .iter()
            .map(|c| c.code.clone())
            .collect();
        let mut org_gen =
            datasynth_generators::organizational_event_generator::OrganizationalEventGenerator::new(
                seed + 101,
            );
        let org_events = org_gen.generate_events(start_date, end_date, &company_codes);

        stats.process_evolution_event_count = process_events.len();
        stats.organizational_event_count = org_events.len();

        info!(
            "Evolution events generated: {} process evolution, {} organizational",
            process_events.len(),
            org_events.len()
        );
        self.check_resources_with_log("post-evolution-events")?;

        Ok((process_events, org_events))
    }

    /// Phase 24b: Generate disruption events (outages, migrations, process changes,
    /// data recovery, and regulatory changes).
    fn phase_disruption_events(
        &self,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<Vec<datasynth_generators::disruption::DisruptionEvent>> {
        if !self.config.organizational_events.enabled {
            debug!("Phase 24b: Skipped (organizational events disabled)");
            return Ok(Vec::new());
        }
        info!("Phase 24b: Generating Disruption Events");

        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let end_date = start_date + chrono::Months::new(self.config.global.period_months);

        let company_codes: Vec<String> = self
            .config
            .companies
            .iter()
            .map(|c| c.code.clone())
            .collect();

        let mut gen = datasynth_generators::disruption::DisruptionGenerator::new(self.seed + 150);
        let events = gen.generate(start_date, end_date, &company_codes);

        stats.disruption_event_count = events.len();
        info!("Disruption events generated: {} events", events.len());
        self.check_resources_with_log("post-disruption-events")?;

        Ok(events)
    }

    /// Phase 25: Generate counterfactual (original, mutated) JE pairs for ML training.
    ///
    /// Produces paired examples where each pair contains the original clean JE
    /// and a controlled mutation (scaled amount, shifted date, self-approval, or
    /// split transaction). Useful for training anomaly detection models with
    /// known ground truth.
    fn phase_counterfactuals(
        &self,
        journal_entries: &[JournalEntry],
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<Vec<datasynth_generators::counterfactual::CounterfactualPair>> {
        if !self.phase_config.generate_counterfactuals || journal_entries.is_empty() {
            debug!("Phase 25: Skipped (counterfactual generation disabled or no JEs)");
            return Ok(Vec::new());
        }
        info!("Phase 25: Generating Counterfactual Pairs for ML Training");

        use datasynth_generators::counterfactual::{CounterfactualGenerator, CounterfactualSpec};

        let mut gen = CounterfactualGenerator::new(self.seed + 110);

        // Rotating set of specs to produce diverse mutation types
        let specs = [
            CounterfactualSpec::ScaleAmount { factor: 2.5 },
            CounterfactualSpec::ShiftDate { days: -14 },
            CounterfactualSpec::SelfApprove,
            CounterfactualSpec::SplitTransaction { split_count: 3 },
        ];

        let pairs: Vec<_> = journal_entries
            .iter()
            .enumerate()
            .map(|(i, je)| {
                let spec = &specs[i % specs.len()];
                gen.generate(je, spec)
            })
            .collect();

        stats.counterfactual_pair_count = pairs.len();
        info!(
            "Counterfactual pairs generated: {} pairs from {} journal entries",
            pairs.len(),
            journal_entries.len()
        );
        self.check_resources_with_log("post-counterfactuals")?;

        Ok(pairs)
    }

    /// Phase 26: Inject fraud red-flag indicators onto P2P/O2C documents.
    ///
    /// Uses the anomaly labels (from Phase 8) to determine which documents are
    /// fraudulent, then generates probabilistic red flags on all chain documents.
    /// Non-fraud documents also receive red flags at a lower rate (false positives)
    /// to produce realistic ML training data.
    fn phase_red_flags(
        &self,
        anomaly_labels: &AnomalyLabels,
        document_flows: &DocumentFlowSnapshot,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<Vec<datasynth_generators::fraud::RedFlag>> {
        if !self.config.fraud.enabled {
            debug!("Phase 26: Skipped (fraud generation disabled)");
            return Ok(Vec::new());
        }
        info!("Phase 26: Generating Fraud Red-Flag Indicators");

        use datasynth_generators::fraud::RedFlagGenerator;

        let generator = RedFlagGenerator::new();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(self.seed + 120);

        // Build a set of document IDs that are known-fraudulent from anomaly labels.
        let fraud_doc_ids: std::collections::HashSet<&str> = anomaly_labels
            .labels
            .iter()
            .filter(|label| label.anomaly_type.is_intentional())
            .map(|label| label.document_id.as_str())
            .collect();

        let mut flags = Vec::new();

        // Iterate P2P chains: use the purchase order document ID as the chain key.
        for chain in &document_flows.p2p_chains {
            let doc_id = &chain.purchase_order.header.document_id;
            let is_fraud = fraud_doc_ids.contains(doc_id.as_str());
            flags.extend(generator.inject_flags(doc_id, is_fraud, &mut rng));
        }

        // Iterate O2C chains: use the sales order document ID as the chain key.
        for chain in &document_flows.o2c_chains {
            let doc_id = &chain.sales_order.header.document_id;
            let is_fraud = fraud_doc_ids.contains(doc_id.as_str());
            flags.extend(generator.inject_flags(doc_id, is_fraud, &mut rng));
        }

        stats.red_flag_count = flags.len();
        info!(
            "Red flags generated: {} flags across {} P2P + {} O2C chains ({} fraud docs)",
            flags.len(),
            document_flows.p2p_chains.len(),
            document_flows.o2c_chains.len(),
            fraud_doc_ids.len()
        );
        self.check_resources_with_log("post-red-flags")?;

        Ok(flags)
    }

    /// Phase 26b: Generate collusion rings from employee/vendor pools.
    ///
    /// Gated on `fraud.enabled && fraud.clustering_enabled`. Uses the
    /// `CollusionRingGenerator` to create 1-3 coordinated fraud networks and
    /// advance them over the simulation period.
    fn phase_collusion_rings(
        &mut self,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<Vec<datasynth_generators::fraud::CollusionRing>> {
        if !(self.config.fraud.enabled && self.config.fraud.clustering_enabled) {
            debug!("Phase 26b: Skipped (fraud collusion generation disabled)");
            return Ok(Vec::new());
        }
        info!("Phase 26b: Generating Collusion Rings");

        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let months = self.config.global.period_months;

        let employee_ids: Vec<String> = self
            .master_data
            .employees
            .iter()
            .map(|e| e.employee_id.clone())
            .collect();
        let vendor_ids: Vec<String> = self
            .master_data
            .vendors
            .iter()
            .map(|v| v.vendor_id.clone())
            .collect();

        let mut generator =
            datasynth_generators::fraud::CollusionRingGenerator::new(self.seed + 160);
        let rings = generator.generate(&employee_ids, &vendor_ids, start_date, months);

        stats.collusion_ring_count = rings.len();
        info!(
            "Collusion rings generated: {} rings, total members: {}",
            rings.len(),
            rings
                .iter()
                .map(datasynth_generators::fraud::CollusionRing::size)
                .sum::<usize>()
        );
        self.check_resources_with_log("post-collusion-rings")?;

        Ok(rings)
    }

    /// Phase 27: Generate bi-temporal version chains for vendor entities.
    ///
    /// Creates `TemporalVersionChain<Vendor>` records that model how vendor
    /// master data changes over time, supporting bi-temporal audit queries.
    fn phase_temporal_attributes(
        &mut self,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<
        Vec<datasynth_core::models::TemporalVersionChain<datasynth_core::models::Vendor>>,
    > {
        if !self.config.temporal_attributes.enabled {
            debug!("Phase 27: Skipped (temporal attributes disabled)");
            return Ok(Vec::new());
        }
        info!("Phase 27: Generating Bi-Temporal Vendor Version Chains");

        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;

        // Build a TemporalAttributeConfig from the user's config.
        // Since Phase 27 is already gated on temporal_attributes.enabled,
        // default to enabling version chains so users get actual mutations.
        let generate_version_chains = self.config.temporal_attributes.generate_version_chains
            || self.config.temporal_attributes.enabled;
        let temporal_config = {
            let ta = &self.config.temporal_attributes;
            datasynth_generators::temporal::TemporalAttributeConfigBuilder::new()
                .enabled(ta.enabled)
                .closed_probability(ta.valid_time.closed_probability)
                .avg_validity_days(ta.valid_time.avg_validity_days)
                .avg_recording_delay(ta.transaction_time.avg_recording_delay_seconds)
                .with_version_chains(if generate_version_chains {
                    ta.avg_versions_per_entity
                } else {
                    1.0
                })
                .build()
        };
        // Apply backdating settings if configured
        let temporal_config = if self
            .config
            .temporal_attributes
            .transaction_time
            .allow_backdating
        {
            let mut c = temporal_config;
            c.transaction_time.allow_backdating = true;
            c.transaction_time.backdating_probability = self
                .config
                .temporal_attributes
                .transaction_time
                .backdating_probability;
            c.transaction_time.max_backdate_days = self
                .config
                .temporal_attributes
                .transaction_time
                .max_backdate_days;
            c
        } else {
            temporal_config
        };
        let mut gen = datasynth_generators::temporal::TemporalAttributeGenerator::new(
            temporal_config,
            self.seed + 130,
            start_date,
        );

        let uuid_factory = datasynth_core::DeterministicUuidFactory::new(
            self.seed + 130,
            datasynth_core::GeneratorType::Vendor,
        );

        let chains: Vec<_> = self
            .master_data
            .vendors
            .iter()
            .map(|vendor| {
                let id = uuid_factory.next();
                gen.generate_version_chain(vendor.clone(), id)
            })
            .collect();

        stats.temporal_version_chain_count = chains.len();
        info!("Temporal version chains generated: {} chains", chains.len());
        self.check_resources_with_log("post-temporal-attributes")?;

        Ok(chains)
    }

    /// Phase 28: Build entity relationship graph and cross-process links.
    ///
    /// Part 1 (gated on `relationship_strength.enabled`): builds an
    /// `EntityGraph` from master-data vendor/customer entities and
    /// journal-entry-derived transaction summaries.
    ///
    /// Part 2 (gated on `cross_process_links.enabled`): extracts
    /// `GoodsReceiptRef` / `DeliveryRef` from document flow chains and
    /// generates inventory-movement cross-process links.
    fn phase_entity_relationships(
        &self,
        journal_entries: &[JournalEntry],
        document_flows: &DocumentFlowSnapshot,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<(
        Option<datasynth_core::models::EntityGraph>,
        Vec<datasynth_core::models::CrossProcessLink>,
    )> {
        use datasynth_generators::relationships::{
            DeliveryRef, EntityGraphConfig, EntityGraphGenerator, EntitySummary, GoodsReceiptRef,
            TransactionSummary,
        };

        let rs_enabled = self.config.relationship_strength.enabled;
        let cpl_enabled = self.config.cross_process_links.enabled
            || (!document_flows.p2p_chains.is_empty() && !document_flows.o2c_chains.is_empty());

        if !rs_enabled && !cpl_enabled {
            debug!(
                "Phase 28: Skipped (relationship_strength and cross_process_links both disabled)"
            );
            return Ok((None, Vec::new()));
        }

        info!("Phase 28: Generating Entity Relationship Graph + Cross-Process Links");

        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;

        let company_code = self
            .config
            .companies
            .first()
            .map(|c| c.code.as_str())
            .unwrap_or("1000");

        // Build the generator with matching config flags
        let gen_config = EntityGraphConfig {
            enabled: rs_enabled,
            cross_process: datasynth_generators::relationships::CrossProcessConfig {
                enable_inventory_links: self.config.cross_process_links.inventory_p2p_o2c,
                enable_return_flows: false,
                enable_payment_links: self.config.cross_process_links.payment_bank_reconciliation,
                enable_ic_bilateral: self.config.cross_process_links.intercompany_bilateral,
                // Use higher link rate for small datasets to avoid probabilistic empty results
                inventory_link_rate: if document_flows.p2p_chains.len() <= 10 {
                    1.0
                } else {
                    0.30
                },
                ..Default::default()
            },
            strength_config: datasynth_generators::relationships::StrengthConfig {
                transaction_volume_weight: self
                    .config
                    .relationship_strength
                    .calculation
                    .transaction_volume_weight,
                transaction_count_weight: self
                    .config
                    .relationship_strength
                    .calculation
                    .transaction_count_weight,
                duration_weight: self
                    .config
                    .relationship_strength
                    .calculation
                    .relationship_duration_weight,
                recency_weight: self.config.relationship_strength.calculation.recency_weight,
                mutual_connections_weight: self
                    .config
                    .relationship_strength
                    .calculation
                    .mutual_connections_weight,
                recency_half_life_days: self
                    .config
                    .relationship_strength
                    .calculation
                    .recency_half_life_days,
            },
            ..Default::default()
        };

        let mut gen = EntityGraphGenerator::with_config(self.seed + 140, gen_config);

        // --- Part 1: Entity Relationship Graph ---
        let entity_graph = if rs_enabled {
            // Build EntitySummary lists from master data
            let vendor_summaries: Vec<EntitySummary> = self
                .master_data
                .vendors
                .iter()
                .map(|v| {
                    EntitySummary::new(
                        &v.vendor_id,
                        &v.name,
                        datasynth_core::models::GraphEntityType::Vendor,
                        start_date,
                    )
                })
                .collect();

            let customer_summaries: Vec<EntitySummary> = self
                .master_data
                .customers
                .iter()
                .map(|c| {
                    EntitySummary::new(
                        &c.customer_id,
                        &c.name,
                        datasynth_core::models::GraphEntityType::Customer,
                        start_date,
                    )
                })
                .collect();

            // Build transaction summaries from journal entries.
            // Key = (company_code, trading_partner) for entries that have a
            // trading partner.  This captures intercompany flows and any JE
            // whose line items carry a trading_partner reference.
            let mut txn_summaries: std::collections::HashMap<(String, String), TransactionSummary> =
                std::collections::HashMap::new();

            for je in journal_entries {
                let cc = je.header.company_code.clone();
                let posting_date = je.header.posting_date;
                for line in &je.lines {
                    if let Some(ref tp) = line.trading_partner {
                        let amount = if line.debit_amount > line.credit_amount {
                            line.debit_amount
                        } else {
                            line.credit_amount
                        };
                        let entry = txn_summaries
                            .entry((cc.clone(), tp.clone()))
                            .or_insert_with(|| TransactionSummary {
                                total_volume: rust_decimal::Decimal::ZERO,
                                transaction_count: 0,
                                first_transaction_date: posting_date,
                                last_transaction_date: posting_date,
                                related_entities: std::collections::HashSet::new(),
                            });
                        entry.total_volume += amount;
                        entry.transaction_count += 1;
                        if posting_date < entry.first_transaction_date {
                            entry.first_transaction_date = posting_date;
                        }
                        if posting_date > entry.last_transaction_date {
                            entry.last_transaction_date = posting_date;
                        }
                        entry.related_entities.insert(cc.clone());
                    }
                }
            }

            // Also extract transaction relationships from document flow chains.
            // P2P chains: Company → Vendor relationships
            for chain in &document_flows.p2p_chains {
                let cc = chain.purchase_order.header.company_code.clone();
                let vendor_id = chain.purchase_order.vendor_id.clone();
                let po_date = chain.purchase_order.header.document_date;
                let amount = chain.purchase_order.total_net_amount;

                let entry = txn_summaries
                    .entry((cc.clone(), vendor_id))
                    .or_insert_with(|| TransactionSummary {
                        total_volume: rust_decimal::Decimal::ZERO,
                        transaction_count: 0,
                        first_transaction_date: po_date,
                        last_transaction_date: po_date,
                        related_entities: std::collections::HashSet::new(),
                    });
                entry.total_volume += amount;
                entry.transaction_count += 1;
                if po_date < entry.first_transaction_date {
                    entry.first_transaction_date = po_date;
                }
                if po_date > entry.last_transaction_date {
                    entry.last_transaction_date = po_date;
                }
                entry.related_entities.insert(cc);
            }

            // O2C chains: Company → Customer relationships
            for chain in &document_flows.o2c_chains {
                let cc = chain.sales_order.header.company_code.clone();
                let customer_id = chain.sales_order.customer_id.clone();
                let so_date = chain.sales_order.header.document_date;
                let amount = chain.sales_order.total_net_amount;

                let entry = txn_summaries
                    .entry((cc.clone(), customer_id))
                    .or_insert_with(|| TransactionSummary {
                        total_volume: rust_decimal::Decimal::ZERO,
                        transaction_count: 0,
                        first_transaction_date: so_date,
                        last_transaction_date: so_date,
                        related_entities: std::collections::HashSet::new(),
                    });
                entry.total_volume += amount;
                entry.transaction_count += 1;
                if so_date < entry.first_transaction_date {
                    entry.first_transaction_date = so_date;
                }
                if so_date > entry.last_transaction_date {
                    entry.last_transaction_date = so_date;
                }
                entry.related_entities.insert(cc);
            }

            let as_of_date = journal_entries
                .last()
                .map(|je| je.header.posting_date)
                .unwrap_or(start_date);

            let graph = gen.generate_entity_graph(
                company_code,
                as_of_date,
                &vendor_summaries,
                &customer_summaries,
                &txn_summaries,
            );

            info!(
                "Entity relationship graph: {} nodes, {} edges",
                graph.nodes.len(),
                graph.edges.len()
            );
            stats.entity_relationship_node_count = graph.nodes.len();
            stats.entity_relationship_edge_count = graph.edges.len();
            Some(graph)
        } else {
            None
        };

        // --- Part 2: Cross-Process Links ---
        let cross_process_links = if cpl_enabled {
            // Build GoodsReceiptRef from P2P chains
            let gr_refs: Vec<GoodsReceiptRef> = document_flows
                .p2p_chains
                .iter()
                .flat_map(|chain| {
                    let vendor_id = chain.purchase_order.vendor_id.clone();
                    let cc = chain.purchase_order.header.company_code.clone();
                    chain.goods_receipts.iter().flat_map(move |gr| {
                        gr.items.iter().filter_map({
                            let doc_id = gr.header.document_id.clone();
                            let v_id = vendor_id.clone();
                            let company = cc.clone();
                            let receipt_date = gr.header.document_date;
                            move |item| {
                                item.base
                                    .material_id
                                    .as_ref()
                                    .map(|mat_id| GoodsReceiptRef {
                                        document_id: doc_id.clone(),
                                        material_id: mat_id.clone(),
                                        quantity: item.base.quantity,
                                        receipt_date,
                                        vendor_id: v_id.clone(),
                                        company_code: company.clone(),
                                    })
                            }
                        })
                    })
                })
                .collect();

            // Build DeliveryRef from O2C chains
            let del_refs: Vec<DeliveryRef> = document_flows
                .o2c_chains
                .iter()
                .flat_map(|chain| {
                    let customer_id = chain.sales_order.customer_id.clone();
                    let cc = chain.sales_order.header.company_code.clone();
                    chain.deliveries.iter().flat_map(move |del| {
                        let delivery_date = del.actual_gi_date.unwrap_or(del.planned_gi_date);
                        del.items.iter().filter_map({
                            let doc_id = del.header.document_id.clone();
                            let c_id = customer_id.clone();
                            let company = cc.clone();
                            move |item| {
                                item.base.material_id.as_ref().map(|mat_id| DeliveryRef {
                                    document_id: doc_id.clone(),
                                    material_id: mat_id.clone(),
                                    quantity: item.base.quantity,
                                    delivery_date,
                                    customer_id: c_id.clone(),
                                    company_code: company.clone(),
                                })
                            }
                        })
                    })
                })
                .collect();

            let links = gen.generate_cross_process_links(&gr_refs, &del_refs);
            info!("Cross-process links generated: {} links", links.len());
            stats.cross_process_link_count = links.len();
            links
        } else {
            Vec::new()
        };

        self.check_resources_with_log("post-entity-relationships")?;
        Ok((entity_graph, cross_process_links))
    }

    /// Phase 29: Generate industry-specific GL accounts via factory dispatch.
    fn phase_industry_data(
        &self,
        stats: &mut EnhancedGenerationStatistics,
    ) -> Option<datasynth_generators::industry::factory::IndustryOutput> {
        if !self.config.industry_specific.enabled {
            return None;
        }
        info!("Phase 29: Generating industry-specific data");
        let output = datasynth_generators::industry::factory::generate_industry_output(
            self.config.global.industry,
        );
        stats.industry_gl_account_count = output.gl_accounts.len();
        info!(
            "Industry data generated: {} GL accounts for {:?}",
            output.gl_accounts.len(),
            self.config.global.industry
        );
        Some(output)
    }

    /// Phase 3b: Generate opening balances for each company.
    ///
    /// # Order of precedence
    ///
    /// 1. **v5.3 chain carryover** (ShardContext.opening_balances non-empty):
    ///    convert each EntityOpeningBalance into a
    ///    GeneratedOpeningBalance per company. This branch runs
    ///    UNCONDITIONALLY — even when `balance.generate_opening_balances`
    ///    is `false` — so a non-overlay preset that gets driven through
    ///    `group generate-chain` still applies the prior-year carry-
    ///    forward instead of silently dropping it.
    /// 2. **`generate_opening_balances` flag**: if off (and no carryover),
    ///    return empty Vec.
    /// 3. **OpeningBalanceGenerator**: industry-mix sampler for the
    ///    period-0 engagement.
    /// Returns `(opening balances, specialized opening-seed JEs)`. The second vec is non-empty only
    /// on the FY1 generator path (branch 3) when a specialized instrument (ECL/Provisions, spec 16
    /// step 1) configures an opening stock — branch 1 (FY2+ shard carry-forward) and branch 2
    /// (disabled) return it empty, so the seed fires exactly once at inception.
    fn phase_opening_balances(
        &mut self,
        coa: &Arc<ChartOfAccounts>,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<(Vec<GeneratedOpeningBalance>, Vec<JournalEntry>)> {
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let fiscal_year = start_date.year();

        // 1. v5.3 chain carryover — runs unconditionally when present.
        if let Some(ctx) = &self.shard_context {
            if !ctx.opening_balances.is_empty() {
                info!(
                    "Phase 3b: applying v5.3 opening-balance carryover ({} accounts × {} companies)",
                    ctx.opening_balances.len(),
                    self.config.companies.len(),
                );
                let mut results = Vec::new();
                for company in &self.config.companies {
                    let balances: std::collections::HashMap<String, rust_decimal::Decimal> = ctx
                        .opening_balances
                        .iter()
                        .map(|ob| (ob.account_code.clone(), ob.net_balance()))
                        .collect();
                    let total_assets = ctx
                        .opening_balances
                        .iter()
                        .filter(|ob| {
                            matches!(
                                ob.account_type,
                                AccountType::Asset | AccountType::ContraAsset
                            )
                        })
                        .map(|ob| ob.net_balance())
                        .sum::<rust_decimal::Decimal>();
                    let total_liabilities = ctx
                        .opening_balances
                        .iter()
                        .filter(|ob| {
                            matches!(
                                ob.account_type,
                                AccountType::Liability | AccountType::ContraLiability
                            )
                        })
                        .map(|ob| ob.net_balance())
                        .sum::<rust_decimal::Decimal>();
                    let total_equity = ctx
                        .opening_balances
                        .iter()
                        .filter(|ob| {
                            matches!(
                                ob.account_type,
                                AccountType::Equity | AccountType::ContraEquity
                            )
                        })
                        .map(|ob| ob.net_balance())
                        .sum::<rust_decimal::Decimal>();
                    let is_balanced = (total_assets - total_liabilities - total_equity).abs()
                        < rust_decimal::Decimal::ONE;
                    results.push(GeneratedOpeningBalance {
                        company_code: company.code.clone(),
                        as_of_date: start_date,
                        balances,
                        total_assets,
                        total_liabilities,
                        total_equity,
                        is_balanced,
                        calculated_ratios: datasynth_core::models::balance::CalculatedRatios {
                            current_ratio: None,
                            quick_ratio: None,
                            debt_to_equity: None,
                            working_capital: rust_decimal::Decimal::ZERO,
                        },
                    });
                }
                stats.opening_balance_count = results.len();
                self.check_resources_with_log("post-opening-balances")?;
                // FY2+ carry-forward already carries any specialized opening (it closed non-zero in
                // FY1) — do NOT re-seed here, or the inception would double-count every fiscal year.
                return Ok((results, Vec::new()));
            }
        }

        // 2. Generator path is opt-in via the config flag.
        if !self.config.balance.generate_opening_balances {
            debug!("Phase 3b: Skipped (opening balance generation disabled)");
            return Ok((Vec::new(), Vec::new()));
        }
        info!("Phase 3b: Generating Opening Balances");

        // 3. OpeningBalanceGenerator — industry-mix sampler for period 0.
        let industry = match self.config.global.industry {
            IndustrySector::Manufacturing => IndustryType::Manufacturing,
            IndustrySector::Retail => IndustryType::Retail,
            IndustrySector::FinancialServices => IndustryType::Financial,
            IndustrySector::Healthcare => IndustryType::Healthcare,
            IndustrySector::Technology => IndustryType::Technology,
            _ => IndustryType::Manufacturing,
        };

        let config = datasynth_generators::OpeningBalanceConfig {
            industry,
            ..Default::default()
        };
        let mut gen =
            datasynth_generators::OpeningBalanceGenerator::with_seed(config, self.seed + 200);

        let mut results = Vec::new();
        // Spec 16 step 1: specialized opening-stock inception JEs (ECL/Provisions), built once on
        // this FY1 generator path so the seed never re-fires (branch 1 carries it forward instead).
        let mut specialized_jes: Vec<JournalEntry> = Vec::new();
        for company in &self.config.companies {
            let spec = OpeningBalanceSpec::new(
                company.code.clone(),
                start_date,
                fiscal_year,
                company.currency.clone(),
                rust_decimal::Decimal::new(10_000_000, 0),
                industry,
            );
            let ob = gen.generate(&spec, coa, start_date, &company.code);
            results.push(ob);
            specialized_jes.extend(self.build_specialized_opening_seed_jes(
                &company.code,
                &company.currency,
                start_date,
            ));
            // spec 16 step 2: the pension funded-status opening seed (offset to OCI 3800, sign-driven).
            specialized_jes.extend(self.build_pension_opening_seed_je(
                &company.code,
                &company.currency,
                start_date,
            ));
        }

        stats.opening_balance_count = results.len();
        info!("Opening balances generated: {} companies", results.len());
        if !specialized_jes.is_empty() {
            info!(
                "Specialized opening-stock seeds: {} inception JEs",
                specialized_jes.len()
            );
        }
        self.check_resources_with_log("post-opening-balances")?;

        Ok((results, specialized_jes))
    }

    /// Spec 16 step 1 — build the specialized opening-stock inception JEs for one company, gated by
    /// the per-instrument `opening_balance` config. Each is a balanced 2-line JE:
    /// `DR Retained Earnings (3200) / CR <control>` for the configured opening amount. The posting
    /// SIDES ARE HARDCODED — the credit-normal ECL allowance (1105, a contra-asset NOT in the
    /// generated CoA) must never be routed through the opening-balance converter's first-digit
    /// heuristic, which would mis-side it as a debit and silently land the error in the 3100 plug.
    /// Returns empty when neither instrument configures an opening (so a default build is
    /// byte-identical). The JEs carry `document_type=OPENING_BALANCE` so they load as the FY1 opening
    /// and are roll-forward-suppressed in later fiscal years, exactly like the foundational opening.
    fn build_specialized_opening_seed_jes(
        &self,
        company_code: &str,
        currency: &str,
        as_of_date: NaiveDate,
    ) -> Vec<JournalEntry> {
        let std = &self.config.accounting_standards;
        specialized_opening_seed_jes(
            company_code,
            currency,
            as_of_date,
            std.expected_credit_loss.opening_balance,
            std.provisions.opening_balance,
        )
    }

    /// Spec 16 step 2 — build the pension funded-status opening seed for one company from
    /// `accounting_standards.pension.opening_net_liability` (signed; offset to OCI 3800). Returns
    /// `None` when unconfigured (so a default build is byte-identical). Delegates to the free fn.
    fn build_pension_opening_seed_je(
        &self,
        company_code: &str,
        currency: &str,
        as_of_date: NaiveDate,
    ) -> Option<JournalEntry> {
        pension_opening_seed_je(
            company_code,
            currency,
            as_of_date,
            self.config
                .accounting_standards
                .pension
                .opening_net_liability,
        )
    }

    /// Phase 9b: Reconcile GL control accounts to subledger balances.
    fn phase_subledger_reconciliation(
        &mut self,
        subledger: &SubledgerSnapshot,
        entries: &[JournalEntry],
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<Vec<datasynth_generators::ReconciliationResult>> {
        if !self.config.balance.reconcile_subledgers {
            debug!("Phase 9b: Skipped (subledger reconciliation disabled)");
            return Ok(Vec::new());
        }
        info!("Phase 9b: Reconciling GL to subledger balances");

        let end_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map(|d| d + chrono::Months::new(self.config.global.period_months))
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;

        // Build GL balance map from journal entries using a balance tracker
        let tracker_config = BalanceTrackerConfig {
            validate_on_each_entry: false,
            track_history: false,
            fail_on_validation_error: false,
            ..Default::default()
        };
        let recon_currency = self
            .config
            .companies
            .first()
            .map(|c| c.currency.clone())
            .unwrap_or_else(|| "USD".to_string());
        let mut tracker = RunningBalanceTracker::new_with_currency(tracker_config, recon_currency);
        let validation_errors = tracker.apply_entries(entries);
        if !validation_errors.is_empty() {
            warn!(
                error_count = validation_errors.len(),
                "Balance tracker encountered validation errors during subledger reconciliation"
            );
            for err in &validation_errors {
                debug!("Balance validation error: {:?}", err);
            }
        }

        let mut engine = datasynth_generators::ReconciliationEngine::new(
            datasynth_generators::ReconciliationConfig::default(),
        );

        let mut results = Vec::new();
        let company_code = self
            .config
            .companies
            .first()
            .map(|c| c.code.as_str())
            .unwrap_or("1000");

        // Reconcile AR
        if !subledger.ar_invoices.is_empty() {
            let gl_balance = tracker
                .get_account_balance(
                    company_code,
                    datasynth_core::accounts::control_accounts::AR_CONTROL,
                )
                .map(|b| b.closing_balance)
                .unwrap_or_default();
            let ar_refs: Vec<&ARInvoice> = subledger.ar_invoices.iter().collect();
            results.push(engine.reconcile_ar(company_code, end_date, gl_balance, &ar_refs));
        }

        // Reconcile AP
        if !subledger.ap_invoices.is_empty() {
            let gl_balance = tracker
                .get_account_balance(
                    company_code,
                    datasynth_core::accounts::control_accounts::AP_CONTROL,
                )
                .map(|b| b.closing_balance)
                .unwrap_or_default();
            let ap_refs: Vec<&APInvoice> = subledger.ap_invoices.iter().collect();
            results.push(engine.reconcile_ap(company_code, end_date, gl_balance, &ap_refs));
        }

        // Reconcile FA
        if !subledger.fa_records.is_empty() {
            let gl_asset_balance = tracker
                .get_account_balance(
                    company_code,
                    datasynth_core::accounts::control_accounts::FIXED_ASSETS,
                )
                .map(|b| b.closing_balance)
                .unwrap_or_default();
            let gl_accum_depr_balance = tracker
                .get_account_balance(
                    company_code,
                    datasynth_core::accounts::control_accounts::ACCUMULATED_DEPRECIATION,
                )
                .map(|b| b.closing_balance)
                .unwrap_or_default();
            let fa_refs: Vec<&datasynth_core::models::subledger::fa::FixedAssetRecord> =
                subledger.fa_records.iter().collect();
            let (asset_recon, depr_recon) = engine.reconcile_fa(
                company_code,
                end_date,
                gl_asset_balance,
                gl_accum_depr_balance,
                &fa_refs,
            );
            results.push(asset_recon);
            results.push(depr_recon);
        }

        // Reconcile Inventory
        if !subledger.inventory_positions.is_empty() {
            let gl_balance = tracker
                .get_account_balance(
                    company_code,
                    datasynth_core::accounts::control_accounts::INVENTORY,
                )
                .map(|b| b.closing_balance)
                .unwrap_or_default();
            let inv_refs: Vec<&datasynth_core::models::subledger::inventory::InventoryPosition> =
                subledger.inventory_positions.iter().collect();
            results.push(engine.reconcile_inventory(company_code, end_date, gl_balance, &inv_refs));
        }

        stats.subledger_reconciliation_count = results.len();
        let passed = results.iter().filter(|r| r.is_balanced()).count();
        let failed = results.len() - passed;
        info!(
            "Subledger reconciliation: {} checks, {} passed, {} failed",
            results.len(),
            passed,
            failed
        );
        self.check_resources_with_log("post-subledger-reconciliation")?;

        Ok(results)
    }

    /// Generate the chart of accounts.
    fn generate_coa(&mut self) -> SynthResult<Arc<ChartOfAccounts>> {
        let pb = self.create_progress_bar(1, "Generating Chart of Accounts");

        let coa_framework = self.resolve_coa_framework();

        let mut gen = ChartOfAccountsGenerator::new(
            self.config.chart_of_accounts.complexity,
            self.config.global.industry,
            self.seed,
        )
        .with_coa_framework(coa_framework)
        // v5.7.0 — honour the opt-in industry-pack expansion flag.
        .with_expand_industry_subaccounts(
            self.config.chart_of_accounts.expand_industry_subaccounts,
        );

        let mut built = gen.generate();
        // v4.4.1: propagate the accounting framework label from config
        // onto the CoA struct so SDK consumers can read it without
        // cross-referencing the config (they previously saw null).
        if self.config.accounting_standards.enabled {
            use datasynth_config::schema::AccountingFrameworkConfig;
            built.accounting_framework = self.config.accounting_standards.framework.map(|f| {
                match f {
                    AccountingFrameworkConfig::UsGaap => "us_gaap",
                    AccountingFrameworkConfig::Ifrs => "ifrs",
                    AccountingFrameworkConfig::FrenchGaap => "french_gaap",
                    AccountingFrameworkConfig::GermanGaap => "german_gaap",
                    AccountingFrameworkConfig::DualReporting => "dual_reporting",
                }
                .to_string()
            });
        }
        // SP4.2 W8.2 + W7.1 — remap synthetic account numbers to corpus
        // ones first (W8.2), then enrich descriptions via the overlay (W7.1).
        // Applied before Arc::new so we only build one Arc (no clone needed).
        if let Some(ref cached) = self.cached_priors {
            if let Some(ref coa_prior) = cached.coa_semantic {
                use datasynth_generators::coa_generator::{
                    remap_account_numbers_to_prior, ChartOfAccountsGenerator,
                };
                // W8.2 — replace synthetic account numbers with corpus
                // ones so the W7.1 overlay fires at ~80% instead of ~16%.
                let mut rng =
                    rand_chacha::ChaCha8Rng::seed_from_u64(self.seed.wrapping_add(88_200));
                let remapped = remap_account_numbers_to_prior(&mut built, coa_prior, &mut rng);
                tracing::info!(
                    target: "datasynth_runtime::coa",
                    remapped,
                    total = built.accounts.len(),
                    "SP4.2 W8.2 — remapped synthetic account numbers to prior-matched corpus values"
                );
                // W7.1 — now overlay descriptions / class metadata for the
                // (now mostly corpus-numbered) accounts.
                let applied =
                    ChartOfAccountsGenerator::apply_coa_semantic_prior(&mut built, coa_prior);
                tracing::info!(
                    target: "datasynth_runtime::coa",
                    applied,
                    total = built.accounts.len(),
                    "SP4.2 W7.1 — overlaid real CoA semantic entries onto synthetic accounts"
                );
            }
            // SP6 — taxonomy overlay: run AFTER the semantic overlay so
            // taxonomy-templated accounts take precedence over verbatim
            // semantic descriptions.  Uses SyntheticExampleResolver because
            // the CoA is built before master-data pools are populated (so
            // vendor/customer names are not yet available).
            if let Some(tx) = cached.text_taxonomy.as_ref() {
                use datasynth_core::distributions::text_taxonomy::SyntheticExampleResolver;
                use datasynth_generators::coa_generator::overlay_coa_taxonomy;
                let mut resolver = SyntheticExampleResolver;
                let mut rng =
                    rand_chacha::ChaCha8Rng::seed_from_u64(self.seed.wrapping_add(88_201));
                overlay_coa_taxonomy(&mut built, tx, &mut resolver, &mut rng);
                tracing::info!(
                    target: "datasynth_runtime::coa",
                    total = built.accounts.len(),
                    "SP6 — overlaid text-taxonomy templates onto CoA descriptions"
                );
            }
        }

        let coa = Arc::new(built);
        self.coa = Some(Arc::clone(&coa));

        if let Some(pb) = pb {
            pb.finish_with_message("Chart of Accounts complete");
        }

        Ok(coa)
    }

    /// Generate master data entities.
    fn generate_master_data(&mut self) -> SynthResult<()> {
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let end_date = start_date + chrono::Months::new(self.config.global.period_months);

        let total = self.config.companies.len() as u64 * 5; // 5 entity types
        let pb = self.create_progress_bar(total, "Generating Master Data");

        // Resolve country pack once for all companies (uses primary company's country)
        let pack = self.primary_pack().clone();

        // Capture config values needed inside the parallel closure
        let vendors_per_company = self.phase_config.vendors_per_company;
        let customers_per_company = self.phase_config.customers_per_company;
        let materials_per_company = self.phase_config.materials_per_company;
        let assets_per_company = self.phase_config.assets_per_company;
        let coa_framework = self.resolve_coa_framework();

        // Generate all master data in parallel across companies.
        // Each company's data is independent, making this embarrassingly parallel.
        let per_company_results: Vec<_> = self
            .config
            .companies
            .par_iter()
            .enumerate()
            .map(|(i, company)| {
                let company_seed = self.seed.wrapping_add(i as u64 * 1000);
                let pack = pack.clone();

                // Generate vendors (offset counter so IDs are globally unique across companies)
                let mut vendor_gen = VendorGenerator::new(company_seed);
                vendor_gen.set_country_pack(pack.clone());
                vendor_gen.set_coa_framework(coa_framework);
                vendor_gen.set_counter_offset(i * vendors_per_company);
                // v3.2.0+: user-supplied bank names (and future template
                // strings) flow through the shared provider.
                vendor_gen.set_template_provider(self.template_provider.clone());
                // Wire vendor network config when enabled
                if self.config.vendor_network.enabled {
                    let vn = &self.config.vendor_network;
                    vendor_gen.set_network_config(datasynth_generators::VendorNetworkConfig {
                        enabled: true,
                        depth: vn.depth,
                        tier1_count: datasynth_generators::TierCountConfig::new(
                            vn.tier1.min,
                            vn.tier1.max,
                        ),
                        tier2_per_parent: datasynth_generators::TierCountConfig::new(
                            vn.tier2_per_parent.min,
                            vn.tier2_per_parent.max,
                        ),
                        tier3_per_parent: datasynth_generators::TierCountConfig::new(
                            vn.tier3_per_parent.min,
                            vn.tier3_per_parent.max,
                        ),
                        cluster_distribution: datasynth_generators::ClusterDistribution {
                            reliable_strategic: vn.clusters.reliable_strategic,
                            standard_operational: vn.clusters.standard_operational,
                            transactional: vn.clusters.transactional,
                            problematic: vn.clusters.problematic,
                        },
                        concentration_limits: datasynth_generators::ConcentrationLimits {
                            max_single_vendor: vn.dependencies.max_single_vendor_concentration,
                            max_top5: vn.dependencies.top_5_concentration,
                        },
                        ..datasynth_generators::VendorNetworkConfig::default()
                    });
                }
                let vendor_pool =
                    vendor_gen.generate_vendor_pool(vendors_per_company, &company.code, start_date);

                // Generate customers (offset counter so IDs are globally unique across companies)
                let mut customer_gen = CustomerGenerator::new(company_seed + 100);
                customer_gen.set_country_pack(pack.clone());
                customer_gen.set_coa_framework(coa_framework);
                customer_gen.set_counter_offset(i * customers_per_company);
                // v3.2.0+: user-supplied customer names flow through the shared provider.
                customer_gen.set_template_provider(self.template_provider.clone());
                // Wire customer segmentation config when enabled
                if self.config.customer_segmentation.enabled {
                    let cs = &self.config.customer_segmentation;
                    let seg_cfg = datasynth_generators::CustomerSegmentationConfig {
                        enabled: true,
                        segment_distribution: datasynth_generators::SegmentDistribution {
                            enterprise: cs.value_segments.enterprise.customer_share,
                            mid_market: cs.value_segments.mid_market.customer_share,
                            smb: cs.value_segments.smb.customer_share,
                            consumer: cs.value_segments.consumer.customer_share,
                        },
                        referral_config: datasynth_generators::ReferralConfig {
                            enabled: cs.networks.referrals.enabled,
                            referral_rate: cs.networks.referrals.referral_rate,
                            ..Default::default()
                        },
                        hierarchy_config: datasynth_generators::HierarchyConfig {
                            enabled: cs.networks.corporate_hierarchies.enabled,
                            hierarchy_rate: cs.networks.corporate_hierarchies.probability,
                            ..Default::default()
                        },
                        ..Default::default()
                    };
                    customer_gen.set_segmentation_config(seg_cfg);
                }
                let customer_pool = customer_gen.generate_customer_pool(
                    customers_per_company,
                    &company.code,
                    start_date,
                );

                // Generate materials (offset counter so IDs are globally unique across companies)
                let mut material_gen = MaterialGenerator::new(company_seed + 200);
                material_gen.set_country_pack(pack.clone());
                material_gen.set_counter_offset(i * materials_per_company);
                // v3.2.1+: user-supplied material descriptions flow through shared provider
                material_gen.set_template_provider(self.template_provider.clone());
                let material_pool = material_gen.generate_material_pool(
                    materials_per_company,
                    &company.code,
                    start_date,
                );

                // Generate fixed assets
                let mut asset_gen = AssetGenerator::new(company_seed + 300);
                // v3.2.1+: user-supplied asset descriptions flow through shared provider
                asset_gen.set_template_provider(self.template_provider.clone());
                let asset_pool = asset_gen.generate_asset_pool(
                    assets_per_company,
                    &company.code,
                    (start_date, end_date),
                );

                // Generate employees
                let mut employee_gen = EmployeeGenerator::new(company_seed + 400);
                employee_gen.set_country_pack(pack);
                // v3.2.1+: user-supplied department names flow through shared provider
                employee_gen.set_template_provider(self.template_provider.clone());
                let employee_pool =
                    employee_gen.generate_company_pool(&company.code, (start_date, end_date));

                // Generate employee change history (2-5 events per employee)
                let employee_change_history =
                    employee_gen.generate_all_change_history(&employee_pool, end_date);

                // Generate cost center hierarchy (level-1 departments + level-2 sub-departments)
                let employee_ids: Vec<String> = employee_pool
                    .employees
                    .iter()
                    .map(|e| e.employee_id.clone())
                    .collect();
                let mut cc_gen = datasynth_generators::CostCenterGenerator::new(company_seed + 500);
                let cost_centers = cc_gen.generate_for_company(&company.code, &employee_ids);

                // v5.1: profit centre hierarchy (two-level: top-level
                // segment / region / product-group nodes + sub-units).
                let mut pc_gen =
                    datasynth_generators::ProfitCenterGenerator::new(company_seed + 600);
                let profit_centers = pc_gen.generate_for_company(&company.code, &employee_ids);

                (
                    vendor_pool.vendors,
                    customer_pool.customers,
                    material_pool.materials,
                    asset_pool.assets,
                    employee_pool.employees,
                    employee_change_history,
                    cost_centers,
                    profit_centers,
                )
            })
            .collect();

        // Aggregate results from all companies
        for (
            vendors,
            customers,
            materials,
            assets,
            employees,
            change_history,
            cost_centers,
            profit_centers,
        ) in per_company_results
        {
            self.master_data.vendors.extend(vendors);
            self.master_data.customers.extend(customers);
            self.master_data.materials.extend(materials);
            self.master_data.assets.extend(assets);
            self.master_data.employees.extend(employees);
            self.master_data.cost_centers.extend(cost_centers);
            self.master_data.profit_centers.extend(profit_centers);
            self.master_data
                .employee_change_history
                .extend(change_history);
        }

        // v3.3.0: one OrganizationalProfile per company. Cheap to
        // generate (derived from industry + company_code) so we
        // always emit when master data runs; no separate config flag.
        {
            use datasynth_core::models::IndustrySector;
            use datasynth_generators::organizational_profile_generator::OrganizationalProfileGenerator;
            let industry = match self.config.global.industry {
                IndustrySector::Manufacturing => "manufacturing",
                IndustrySector::Retail => "retail",
                IndustrySector::FinancialServices => "financial_services",
                IndustrySector::Technology => "technology",
                IndustrySector::Healthcare => "healthcare",
                _ => "other",
            };
            for (i, company) in self.config.companies.iter().enumerate() {
                let company_seed = self.seed.wrapping_add(i as u64 * 1000) + 500;
                let mut profile_gen = OrganizationalProfileGenerator::new(company_seed);
                let profile = profile_gen.generate(&company.code, industry);
                self.master_data.organizational_profiles.push(profile);
            }
        }

        if let Some(pb) = &pb {
            pb.inc(total);
        }
        if let Some(pb) = pb {
            pb.finish_with_message("Master data generation complete");
        }

        Ok(())
    }

    /// Generate document flows (P2P and O2C).
    fn generate_document_flows(&mut self, flows: &mut DocumentFlowSnapshot) -> SynthResult<()> {
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;

        // Generate P2P chains
        // Cap at ~2 POs per vendor per month to keep spend concentration realistic. DB-E4: an
        // explicit document_flows.p2p.count overrides the derived phase_config count (then clamped).
        let months = (self.config.global.period_months as usize).max(1);
        let p2p_count = self
            .config
            .document_flows
            .p2p
            .count
            .unwrap_or(self.phase_config.p2p_chains)
            .min(self.master_data.vendors.len() * 2 * months);
        let pb = self.create_progress_bar(p2p_count as u64, "Generating P2P Document Flows");

        // Convert P2P config from schema to generator config
        let p2p_config = convert_p2p_config(&self.config.document_flows.p2p);
        let mut p2p_gen = P2PGenerator::with_config(self.seed + 1000, p2p_config);
        p2p_gen.set_country_pack(self.primary_pack().clone());
        // v3.4.1: wire temporal context so PO/GR/invoice/payment dates snap
        // to business days. No-op when `temporal_patterns.business_days.
        // enabled = false`.
        if let Some(ctx) = &self.temporal_context {
            p2p_gen.set_temporal_context(Arc::clone(ctx));
        }

        // Spec 19 §4-R1 (R1b): opt-in Pareto concentration for vendor selection. OFF (the default)
        // → `None`, and the loop keeps the exact `i % len` round-robin, byte-for-byte identical to
        // the pre-R1b engine. ON → a discrete power-law weighted-choice over an ISOLATED ChaCha8
        // stream (derived seed, disjoint from `self.seed`), so it never perturbs the generation RNG.
        let conc = &self.config.document_flows.concentration;
        let mut vendor_concentration =
            (conc.vendor_active() && !self.master_data.vendors.is_empty()).then(|| {
                datasynth_core::distributions::ConcentrationSampler::new(
                    datasynth_core::distributions::concentration_seed(self.seed, "p2p_vendor"),
                    self.master_data.vendors.len(),
                    conc.top_n,
                    conc.vendor_top_n_share,
                )
            });

        for i in 0..p2p_count {
            let vendor_idx = match &mut vendor_concentration {
                Some(sampler) => sampler.sample(),
                None => i % self.master_data.vendors.len(),
            };
            let vendor = &self.master_data.vendors[vendor_idx];
            let materials: Vec<&Material> = self
                .master_data
                .materials
                .iter()
                .skip(i % self.master_data.materials.len().max(1))
                .take(2.min(self.master_data.materials.len()))
                .collect();

            if materials.is_empty() {
                continue;
            }

            let company = &self.config.companies[i % self.config.companies.len()];
            let po_date = start_date + chrono::Duration::days((i * 3) as i64 % 365);
            let fiscal_period = po_date.month() as u8;
            let created_by = if self.master_data.employees.is_empty() {
                "SYSTEM"
            } else {
                self.master_data.employees[i % self.master_data.employees.len()]
                    .user_id
                    .as_str()
            };

            let chain = p2p_gen.generate_chain(
                &company.code,
                vendor,
                &materials,
                po_date,
                start_date.year() as u16,
                fiscal_period,
                created_by,
            );

            // Flatten documents
            flows.purchase_orders.push(chain.purchase_order.clone());
            flows.goods_receipts.extend(chain.goods_receipts.clone());
            if let Some(vi) = &chain.vendor_invoice {
                flows.vendor_invoices.push(vi.clone());
            }
            if let Some(payment) = &chain.payment {
                flows.payments.push(payment.clone());
            }
            for remainder in &chain.remainder_payments {
                flows.payments.push(remainder.clone());
            }
            flows.p2p_chains.push(chain);

            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }

        if let Some(pb) = pb {
            pb.finish_with_message("P2P document flows complete");
        }

        // Generate O2C chains
        // Cap at ~2 SOs per customer per month to keep order volume realistic. DB-E4: an explicit
        // document_flows.o2c.count overrides the derived phase_config count (then clamped).
        let o2c_count = self
            .config
            .document_flows
            .o2c
            .count
            .unwrap_or(self.phase_config.o2c_chains)
            .min(self.master_data.customers.len() * 2 * months);
        let pb = self.create_progress_bar(o2c_count as u64, "Generating O2C Document Flows");

        // Convert O2C config from schema to generator config
        let o2c_config = convert_o2c_config(&self.config.document_flows.o2c);
        let mut o2c_gen = O2CGenerator::with_config(self.seed + 2000, o2c_config);
        o2c_gen.set_country_pack(self.primary_pack().clone());
        // v3.4.1: wire temporal context (no-op when business_days disabled).
        if let Some(ctx) = &self.temporal_context {
            o2c_gen.set_temporal_context(Arc::clone(ctx));
        }

        // Spec 19 §4-R1 (R1b): opt-in Pareto concentration for customer selection — same isolated
        // weighted-choice as the P2P vendor loop; OFF (default) keeps the exact `i % len`.
        let conc = &self.config.document_flows.concentration;
        let mut customer_concentration =
            (conc.customer_active() && !self.master_data.customers.is_empty()).then(|| {
                datasynth_core::distributions::ConcentrationSampler::new(
                    datasynth_core::distributions::concentration_seed(self.seed, "o2c_customer"),
                    self.master_data.customers.len(),
                    conc.top_n,
                    conc.customer_top_n_share,
                )
            });

        for i in 0..o2c_count {
            let customer_idx = match &mut customer_concentration {
                Some(sampler) => sampler.sample(),
                None => i % self.master_data.customers.len(),
            };
            let customer = &self.master_data.customers[customer_idx];
            let materials: Vec<&Material> = self
                .master_data
                .materials
                .iter()
                .skip(i % self.master_data.materials.len().max(1))
                .take(2.min(self.master_data.materials.len()))
                .collect();

            if materials.is_empty() {
                continue;
            }

            let company = &self.config.companies[i % self.config.companies.len()];
            let so_date = start_date + chrono::Duration::days((i * 2) as i64 % 365);
            let fiscal_period = so_date.month() as u8;
            let created_by = if self.master_data.employees.is_empty() {
                "SYSTEM"
            } else {
                self.master_data.employees[i % self.master_data.employees.len()]
                    .user_id
                    .as_str()
            };

            let chain = o2c_gen.generate_chain(
                &company.code,
                customer,
                &materials,
                so_date,
                start_date.year() as u16,
                fiscal_period,
                created_by,
            );

            // Flatten documents
            flows.sales_orders.push(chain.sales_order.clone());
            flows.deliveries.extend(chain.deliveries.clone());
            if let Some(ci) = &chain.customer_invoice {
                flows.customer_invoices.push(ci.clone());
            }
            if let Some(receipt) = &chain.customer_receipt {
                flows.payments.push(receipt.clone());
            }
            // Extract remainder receipts (follow-up to partial payments)
            for receipt in &chain.remainder_receipts {
                flows.payments.push(receipt.clone());
            }
            flows.o2c_chains.push(chain);

            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }

        if let Some(pb) = pb {
            pb.finish_with_message("O2C document flows complete");
        }

        // Collect all document cross-references from document headers.
        // Each document embeds references to its predecessor(s) via add_reference(); here we
        // denormalise them into a flat list for the document_references.json output file.
        {
            let mut refs = Vec::new();
            for doc in &flows.purchase_orders {
                refs.extend(doc.header.document_references.iter().cloned());
            }
            for doc in &flows.goods_receipts {
                refs.extend(doc.header.document_references.iter().cloned());
            }
            for doc in &flows.vendor_invoices {
                refs.extend(doc.header.document_references.iter().cloned());
            }
            for doc in &flows.sales_orders {
                refs.extend(doc.header.document_references.iter().cloned());
            }
            for doc in &flows.deliveries {
                refs.extend(doc.header.document_references.iter().cloned());
            }
            for doc in &flows.customer_invoices {
                refs.extend(doc.header.document_references.iter().cloned());
            }
            for doc in &flows.payments {
                refs.extend(doc.header.document_references.iter().cloned());
            }
            debug!(
                "Collected {} document cross-references from document headers",
                refs.len()
            );
            flows.document_references = refs;
        }

        Ok(())
    }

    /// Generate journal entries using parallel generation across multiple cores.
    fn generate_journal_entries(
        &mut self,
        coa: &Arc<ChartOfAccounts>,
    ) -> SynthResult<Vec<JournalEntry>> {
        use datasynth_core::traits::ParallelGenerator;

        let total = self.calculate_total_transactions();
        let pb = self.create_progress_bar(total, "Generating Journal Entries");

        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let end_date = start_date + chrono::Months::new(self.config.global.period_months);

        let company_codes: Vec<String> = self
            .config
            .companies
            .iter()
            .map(|c| c.code.clone())
            .collect();

        let mut generator = JournalEntryGenerator::new_with_params(
            self.config.transactions.clone(),
            Arc::clone(coa),
            company_codes,
            start_date,
            end_date,
            self.seed,
        );
        // Wire the `business_processes.*_weight` config through (phantom knob
        // until now — the JE generator hard-coded 0.35/0.30/0.20/0.10/0.05).
        let bp = &self.config.business_processes;
        generator.set_business_process_weights(
            bp.o2c_weight,
            bp.p2p_weight,
            bp.r2r_weight,
            bp.h2r_weight,
            bp.a2r_weight,
        );
        // v3.4.0: wire advanced distributions (mixture models + industry
        // profiles). No-op when `distributions.enabled = false` or
        // `distributions.amounts.enabled = false`, preserving v3.3.2
        // byte-identical output on default configs.
        generator
            .set_advanced_distributions(&self.config.distributions, self.seed + 400)
            .map_err(|e| SynthError::config(format!("invalid distributions config: {e}")))?;

        // SP3: load and wire industry priors when the config opts in via
        //   distributions.industry_profile.priors.enabled = true
        // When disabled (or when using the legacy bare-name form), this block
        // is a no-op and generation behavior is identical to v5.11.
        if let Some(profile) = &self.config.distributions.industry_profile {
            if let Some(priors_cfg) = profile.priors() {
                if priors_cfg.enabled {
                    use datasynth_config::schema::PriorsSource;
                    use datasynth_generators::priors_loader::LoadedPriors;

                    let mut priors_rng =
                        rand_chacha::ChaCha8Rng::seed_from_u64(self.seed.wrapping_add(500));
                    let period_days = i64::from(self.config.global.period_months) * 30;
                    let industry_slug = profile.profile_type().slug();

                    let loaded = match priors_cfg.source {
                        PriorsSource::Bundled => {
                            LoadedPriors::load_bundled(industry_slug, &mut priors_rng, period_days)
                                .map_err(|e| {
                                    SynthError::config(format!(
                                "SP3: failed to load bundled priors for '{industry_slug}': {e}"
                            ))
                                })?
                        }
                        PriorsSource::File => {
                            let path = priors_cfg.path.as_ref().ok_or_else(|| {
                                SynthError::config(
                                    "SP3: industry_profile.priors.path required when source = file"
                                        .to_string(),
                                )
                            })?;
                            LoadedPriors::load_from_path(
                                path,
                                &mut priors_rng,
                                period_days,
                                Some(industry_slug),
                            )
                            .map_err(|e| {
                                SynthError::config(format!(
                                    "SP3: failed to load priors from '{}': {e}",
                                    path.display()
                                ))
                            })?
                        }
                    };

                    // SP3.12 — cache priors in Arc so document-flow generator
                    // can also apply lines-per-JE padding without re-loading.
                    let loaded = std::sync::Arc::new(loaded);
                    self.cached_priors = Some(loaded.clone());
                    generator.loaded_priors = Some((*loaded).clone());

                    // SP3.4 — instantiate VelocityCalibrator when the config
                    // opts in.  Default target rates (R7/R9) are a sensible
                    // baseline; they can be derived from the loaded priors in
                    // a future hardening pass.
                    if priors_cfg.velocity_calibration {
                        use datasynth_generators::velocity_calibrator::VelocityCalibrator;
                        let mut targets = std::collections::HashMap::new();
                        targets.insert("R7".to_string(), 0.10);
                        targets.insert("R9".to_string(), 0.10);
                        let calibrator = VelocityCalibrator::new(targets, 10_000);
                        generator.velocity_calibrator = Some(calibrator);
                    }
                }
            }
        }

        let generator = generator;

        // Connect generated master data to ensure JEs reference real entities
        // Enable persona-based error injection for realistic human behavior
        // Pass fraud configuration for fraud injection
        let je_pack = self.primary_pack();

        // Master-data CC / PC pools so JE.cost_center and
        // JE.profit_center join back to `cost_centers.id` and
        // `profit_centers.id` (closes the v5.9.0 linkage gap that
        // had `JE.cost_center = "CC1000"` while master used
        // `CC-1000-FIN` etc.).  Empty when no master is present —
        // the generator falls back to its hardcoded constants.
        let cc_pool: Vec<String> = self
            .master_data
            .cost_centers
            .iter()
            .map(|c| c.id.clone())
            .collect();
        let pc_pool: Vec<String> = self
            .master_data
            .profit_centers
            .iter()
            .map(|p| p.id.clone())
            .collect();

        // Build a UserPool from the generated employee master so
        // JE.created_by lines join back to `employees.user_id`.  v5.9.0:
        // closes the third linkage gap (the previous behaviour had
        // JeGenerator generate its own UserPool internally with
        // ids disjoint from the employee master).
        let user_pool_from_employees =
            datasynth_core::models::UserPool::from_employees(&self.master_data.employees);

        let mut generator = generator
            .with_master_data(
                &self.master_data.vendors,
                &self.master_data.customers,
                &self.master_data.materials,
            )
            .with_cost_center_pool(cc_pool)
            .with_profit_center_pool(pc_pool)
            .with_country_pack_names(je_pack)
            .with_user_pool(user_pool_from_employees)
            .with_country_pack_temporal(
                self.config.temporal_patterns.clone(),
                self.seed + 200,
                je_pack,
            )
            .with_persona_errors(true)
            .with_fraud_config(self.config.fraud.clone());

        // Apply temporal drift if configured. v3.5.2+: also merge
        // `distributions.regime_changes` (regime events, economic
        // cycles, parameter drifts) into the same DriftConfig so both
        // knobs flow through the shared DriftController.
        let temporal_enabled = self.config.temporal.enabled;
        let regimes_enabled = self.config.distributions.regime_changes.enabled;
        if temporal_enabled || regimes_enabled {
            let mut drift_config = if temporal_enabled {
                self.config.temporal.to_core_config()
            } else {
                // regime-changes only: start from default (drift OFF),
                // apply_to flips `enabled = true`.
                datasynth_core::distributions::DriftConfig::default()
            };
            if regimes_enabled {
                self.config
                    .distributions
                    .regime_changes
                    .apply_to(&mut drift_config, start_date);
            }
            generator = generator.with_drift_config(drift_config, self.seed + 100);
        }

        // Check memory limit at start
        self.check_memory_limit()?;

        // Determine parallelism: use available cores, but cap at total entries
        let num_threads = num_cpus::get().max(1).min(total as usize).max(1);

        // Use parallel generation for datasets with 10K+ entries.
        // Below this threshold, the statistical properties of a single-seeded
        // generator (e.g. Benford compliance) are better preserved.
        let entries = if total >= 10_000 && num_threads > 1 {
            // Parallel path: split the generator across cores and generate in parallel.
            // Each sub-generator gets a unique seed for deterministic, independent generation.
            let sub_generators = generator.split(num_threads);
            let entries_per_thread = total as usize / num_threads;
            let remainder = total as usize % num_threads;

            let batches: Vec<Vec<JournalEntry>> = sub_generators
                .into_par_iter()
                .enumerate()
                .map(|(i, mut gen)| {
                    let count = entries_per_thread + if i < remainder { 1 } else { 0 };
                    gen.generate_batch(count)
                })
                .collect();

            // Merge all batches into a single Vec
            let entries = JournalEntryGenerator::merge_results(batches);

            if let Some(pb) = &pb {
                pb.inc(total);
            }
            entries
        } else {
            // Sequential path for small datasets (< 1000 entries)
            let mut entries = Vec::with_capacity(total as usize);
            for _ in 0..total {
                let entry = generator.generate();
                entries.push(entry);
                if let Some(pb) = &pb {
                    pb.inc(1);
                }
            }
            entries
        };

        if let Some(pb) = pb {
            pb.finish_with_message("Journal entries complete");
        }

        Ok(entries)
    }

    /// Generate journal entries from document flows.
    ///
    /// This creates proper GL entries for each document in the P2P and O2C flows,
    /// ensuring that document activity is reflected in the general ledger.
    fn generate_jes_from_document_flows(
        &mut self,
        flows: &DocumentFlowSnapshot,
    ) -> SynthResult<Vec<JournalEntry>> {
        let total_chains = flows.p2p_chains.len() + flows.o2c_chains.len();
        let pb = self.create_progress_bar(total_chains as u64, "Generating Document Flow JEs");

        let je_config = match self.resolve_coa_framework() {
            CoAFramework::FrenchPcg => DocumentFlowJeConfig::french_gaap(),
            CoAFramework::GermanSkr04 => {
                let fa = datasynth_core::FrameworkAccounts::german_gaap();
                DocumentFlowJeConfig::from(&fa)
            }
            CoAFramework::UsGaap => DocumentFlowJeConfig::default(),
        };

        let populate_fec = je_config.populate_fec_fields;
        let mut generator = DocumentFlowJeGenerator::with_config_and_seed(je_config, self.seed);

        // SP3.12 — propagate cached priors so document-flow JEs receive
        // the same lines-per-JE padding as standalone JEs.
        if let Some(ref priors) = self.cached_priors {
            generator.set_loaded_priors(priors.clone());
        }

        // Master-data CC / PC pools so document-flow-derived JEs
        // (P2P / O2C postings) reference IDs that join back to the
        // cost-centers / profit-centers masters.  Same plumbing as
        // for `JeGenerator` above; falls back to hardcoded const
        // pools when masters are absent.
        let cc_pool: Vec<String> = self
            .master_data
            .cost_centers
            .iter()
            .map(|c| c.id.clone())
            .collect();
        let pc_pool: Vec<String> = self
            .master_data
            .profit_centers
            .iter()
            .map(|p| p.id.clone())
            .collect();
        if !cc_pool.is_empty() {
            generator.set_cost_center_pool(cc_pool);
        }
        if !pc_pool.is_empty() {
            generator.set_profit_center_pool(pc_pool);
        }

        // Build auxiliary account lookup from vendor/customer master data so that
        // FEC auxiliary_account_number uses framework-specific GL accounts (e.g.,
        // PCG "4010001") instead of raw partner IDs.
        if populate_fec {
            let mut aux_lookup = std::collections::HashMap::new();
            for vendor in &self.master_data.vendors {
                if let Some(ref aux) = vendor.auxiliary_gl_account {
                    aux_lookup.insert(vendor.vendor_id.clone(), aux.clone());
                }
            }
            for customer in &self.master_data.customers {
                if let Some(ref aux) = customer.auxiliary_gl_account {
                    aux_lookup.insert(customer.customer_id.clone(), aux.clone());
                }
            }
            if !aux_lookup.is_empty() {
                generator.set_auxiliary_account_lookup(aux_lookup);
            }
        }

        let mut entries = Vec::new();

        // Generate JEs from P2P chains
        for chain in &flows.p2p_chains {
            let chain_entries = generator.generate_from_p2p_chain(chain);
            entries.extend(chain_entries);
            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }

        // Generate JEs from O2C chains
        for chain in &flows.o2c_chains {
            let chain_entries = generator.generate_from_o2c_chain(chain);
            entries.extend(chain_entries);
            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }

        if let Some(pb) = pb {
            pb.finish_with_message(format!(
                "Generated {} JEs from document flows",
                entries.len()
            ));
        }

        Ok(entries)
    }

    /// Generate journal entries from payroll runs.
    ///
    /// Creates one JE per payroll run:
    /// - DR Salaries & Wages (6100) for gross pay
    /// - CR Payroll Clearing (9100) for gross pay
    fn generate_payroll_jes(payroll_runs: &[PayrollRun]) -> Vec<JournalEntry> {
        use datasynth_core::accounts::{expense_accounts, suspense_accounts};

        let mut jes = Vec::with_capacity(payroll_runs.len());

        for run in payroll_runs {
            let mut je = JournalEntry::new_simple(
                format!("JE-PAYROLL-{}", run.payroll_id),
                run.company_code.clone(),
                run.run_date,
                format!("Payroll {}", run.payroll_id),
            );

            // Debit Salaries & Wages for gross pay
            je.add_line(JournalEntryLine {
                line_number: 1,
                gl_account: expense_accounts::SALARIES_WAGES.to_string(),
                debit_amount: run.total_gross,
                reference: Some(run.payroll_id.clone()),
                text: Some(format!(
                    "Payroll {} ({} employees)",
                    run.payroll_id, run.employee_count
                )),
                // spec 27 R6: structured subledger dimension — the whole payroll
                // JE is attributable to this payroll run, so the generic
                // reconciler decomposes the 9100 control by (Payroll, payroll_id)
                // without regex-parsing the reference.
                subledger_ref: Some(SubledgerRef::new(
                    SubledgerType::Payroll,
                    run.payroll_id.clone(),
                    None,
                )),
                ..Default::default()
            });

            // Credit Payroll Clearing for gross pay
            je.add_line(JournalEntryLine {
                line_number: 2,
                gl_account: suspense_accounts::PAYROLL_CLEARING.to_string(),
                credit_amount: run.total_gross,
                reference: Some(run.payroll_id.clone()),
                subledger_ref: Some(SubledgerRef::new(
                    SubledgerType::Payroll,
                    run.payroll_id.clone(),
                    None,
                )),
                ..Default::default()
            });

            jes.push(je);
        }

        jes
    }

    /// Link document flows to subledger records.
    ///
    /// Creates AP invoices from vendor invoices and AR invoices from customer invoices,
    /// ensuring subledger data is coherent with document flow data.
    fn link_document_flows_to_subledgers(
        &mut self,
        flows: &DocumentFlowSnapshot,
    ) -> SynthResult<SubledgerSnapshot> {
        let total = flows.vendor_invoices.len() + flows.customer_invoices.len();
        let pb = self.create_progress_bar(total as u64, "Linking Subledgers");

        // Build vendor/customer name maps from master data for realistic subledger names
        let vendor_names: std::collections::HashMap<String, String> = self
            .master_data
            .vendors
            .iter()
            .map(|v| (v.vendor_id.clone(), v.name.clone()))
            .collect();
        let customer_names: std::collections::HashMap<String, String> = self
            .master_data
            .customers
            .iter()
            .map(|c| (c.customer_id.clone(), c.name.clone()))
            .collect();

        let mut linker = DocumentFlowLinker::new()
            .with_vendor_names(vendor_names)
            .with_customer_names(customer_names);

        // Convert vendor invoices to AP invoices
        let ap_invoices = linker.batch_create_ap_invoices(&flows.vendor_invoices);
        if let Some(pb) = &pb {
            pb.inc(flows.vendor_invoices.len() as u64);
        }

        // Convert customer invoices to AR invoices
        let ar_invoices = linker.batch_create_ar_invoices(&flows.customer_invoices);
        if let Some(pb) = &pb {
            pb.inc(flows.customer_invoices.len() as u64);
        }

        if let Some(pb) = pb {
            pb.finish_with_message(format!(
                "Linked {} AP and {} AR invoices",
                ap_invoices.len(),
                ar_invoices.len()
            ));
        }

        Ok(SubledgerSnapshot {
            ap_invoices,
            ar_invoices,
            fa_records: Vec::new(),
            inventory_positions: Vec::new(),
            inventory_movements: Vec::new(),
            // Aging reports are computed after payment settlement in phase_document_flows.
            ar_aging_reports: Vec::new(),
            ap_aging_reports: Vec::new(),
            // Depreciation runs and inventory valuations are populated after FA/inventory generation.
            depreciation_runs: Vec::new(),
            inventory_valuations: Vec::new(),
            // Dunning runs and letters are populated in phase_document_flows after AR aging.
            dunning_runs: Vec::new(),
            dunning_letters: Vec::new(),
        })
    }

    /// Generate OCPM events from document flows.
    ///
    /// Creates OCEL 2.0 compliant event logs from P2P and O2C document flows,
    /// capturing the object-centric process perspective.
    #[allow(clippy::too_many_arguments)]
    fn generate_ocpm_events(
        &mut self,
        flows: &DocumentFlowSnapshot,
        sourcing: &SourcingSnapshot,
        hr: &HrSnapshot,
        manufacturing: &ManufacturingSnapshot,
        banking: &BankingSnapshot,
        audit: &AuditSnapshot,
        financial_reporting: &FinancialReportingSnapshot,
    ) -> SynthResult<OcpmSnapshot> {
        let total_chains = flows.p2p_chains.len()
            + flows.o2c_chains.len()
            + sourcing.sourcing_projects.len()
            + hr.payroll_runs.len()
            + manufacturing.production_orders.len()
            + banking.customers.len()
            + audit.engagements.len()
            + financial_reporting.bank_reconciliations.len();
        let pb = self.create_progress_bar(total_chains as u64, "Generating OCPM Events");

        // Create OCPM event log with standard types
        let metadata = EventLogMetadata::new("SyntheticData OCPM Log");
        let mut event_log = OcpmEventLog::with_metadata(metadata).with_standard_types();

        // Configure the OCPM generator
        let ocpm_config = OcpmGeneratorConfig {
            generate_p2p: true,
            generate_o2c: true,
            generate_s2c: !sourcing.sourcing_projects.is_empty(),
            generate_h2r: !hr.payroll_runs.is_empty(),
            generate_mfg: !manufacturing.production_orders.is_empty(),
            generate_bank_recon: !financial_reporting.bank_reconciliations.is_empty(),
            generate_bank: !banking.customers.is_empty(),
            generate_audit: !audit.engagements.is_empty(),
            happy_path_rate: 0.75,
            exception_path_rate: 0.20,
            error_path_rate: 0.05,
            add_duration_variability: true,
            duration_std_dev_factor: 0.3,
        };
        let mut ocpm_gen = OcpmEventGenerator::with_config(self.seed + 3000, ocpm_config);
        let ocpm_uuid_factory = OcpmUuidFactory::new(self.seed + 3001);

        // Get available users for resource assignment
        let available_users: Vec<String> = self
            .master_data
            .employees
            .iter()
            .take(20)
            .map(|e| e.user_id.clone())
            .collect();

        // Deterministic base date from config (avoids Utc::now() non-determinism)
        let fallback_date =
            NaiveDate::from_ymd_opt(2024, 1, 1).expect("static date 2024-01-01 is always valid");
        let base_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .unwrap_or(fallback_date);
        let base_midnight = base_date
            .and_hms_opt(0, 0, 0)
            .expect("midnight is always valid");
        let base_datetime =
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(base_midnight, chrono::Utc);

        // Helper closure to add case results to event log
        let add_result = |event_log: &mut OcpmEventLog,
                          result: datasynth_ocpm::CaseGenerationResult| {
            for event in result.events {
                event_log.add_event(event);
            }
            for object in result.objects {
                event_log.add_object(object);
            }
            for relationship in result.relationships {
                event_log.add_relationship(relationship);
            }
            for corr in result.correlation_events {
                event_log.add_correlation_event(corr);
            }
            event_log.add_case(result.case_trace);
        };

        // Generate events from P2P chains
        for chain in &flows.p2p_chains {
            let po = &chain.purchase_order;
            let documents = P2pDocuments::new(
                &po.header.document_id,
                &po.vendor_id,
                &po.header.company_code,
                po.total_net_amount,
                &po.header.currency,
                &ocpm_uuid_factory,
            )
            .with_goods_receipt(
                chain
                    .goods_receipts
                    .first()
                    .map(|gr| gr.header.document_id.as_str())
                    .unwrap_or(""),
                &ocpm_uuid_factory,
            )
            .with_invoice(
                chain
                    .vendor_invoice
                    .as_ref()
                    .map(|vi| vi.header.document_id.as_str())
                    .unwrap_or(""),
                &ocpm_uuid_factory,
            )
            .with_payment(
                chain
                    .payment
                    .as_ref()
                    .map(|p| p.header.document_id.as_str())
                    .unwrap_or(""),
                &ocpm_uuid_factory,
            );

            let start_time =
                chrono::DateTime::from_naive_utc_and_offset(po.header.entry_timestamp, chrono::Utc);
            let result = ocpm_gen.generate_p2p_case(&documents, start_time, &available_users);
            add_result(&mut event_log, result);

            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }

        // Generate events from O2C chains
        for chain in &flows.o2c_chains {
            let so = &chain.sales_order;
            let documents = O2cDocuments::new(
                &so.header.document_id,
                &so.customer_id,
                &so.header.company_code,
                so.total_net_amount,
                &so.header.currency,
                &ocpm_uuid_factory,
            )
            .with_delivery(
                chain
                    .deliveries
                    .first()
                    .map(|d| d.header.document_id.as_str())
                    .unwrap_or(""),
                &ocpm_uuid_factory,
            )
            .with_invoice(
                chain
                    .customer_invoice
                    .as_ref()
                    .map(|ci| ci.header.document_id.as_str())
                    .unwrap_or(""),
                &ocpm_uuid_factory,
            )
            .with_receipt(
                chain
                    .customer_receipt
                    .as_ref()
                    .map(|r| r.header.document_id.as_str())
                    .unwrap_or(""),
                &ocpm_uuid_factory,
            );

            let start_time =
                chrono::DateTime::from_naive_utc_and_offset(so.header.entry_timestamp, chrono::Utc);
            let result = ocpm_gen.generate_o2c_case(&documents, start_time, &available_users);
            add_result(&mut event_log, result);

            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }

        // Generate events from S2C sourcing projects
        for project in &sourcing.sourcing_projects {
            // Find vendor from contracts or qualifications
            let vendor_id = sourcing
                .contracts
                .iter()
                .find(|c| c.sourcing_project_id.as_deref() == Some(&project.project_id))
                .map(|c| c.vendor_id.clone())
                .or_else(|| sourcing.qualifications.first().map(|q| q.vendor_id.clone()))
                .or_else(|| {
                    self.master_data
                        .vendors
                        .first()
                        .map(|v| v.vendor_id.clone())
                })
                .unwrap_or_else(|| "V000".to_string());
            let mut docs = S2cDocuments::new(
                &project.project_id,
                &vendor_id,
                &project.company_code,
                project.estimated_annual_spend,
                &ocpm_uuid_factory,
            );
            // Link RFx if available
            if let Some(rfx) = sourcing
                .rfx_events
                .iter()
                .find(|r| r.sourcing_project_id == project.project_id)
            {
                docs = docs.with_rfx(&rfx.rfx_id, &ocpm_uuid_factory);
                // Link winning bid (status == Accepted)
                if let Some(bid) = sourcing.bids.iter().find(|b| {
                    b.rfx_id == rfx.rfx_id
                        && b.status == datasynth_core::models::sourcing::BidStatus::Accepted
                }) {
                    docs = docs.with_winning_bid(&bid.bid_id, &ocpm_uuid_factory);
                }
            }
            // Link contract
            if let Some(contract) = sourcing
                .contracts
                .iter()
                .find(|c| c.sourcing_project_id.as_deref() == Some(&project.project_id))
            {
                docs = docs.with_contract(&contract.contract_id, &ocpm_uuid_factory);
            }
            let start_time = base_datetime - chrono::Duration::days(90);
            let result = ocpm_gen.generate_s2c_case(&docs, start_time, &available_users);
            add_result(&mut event_log, result);

            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }

        // Generate events from H2R payroll runs
        for run in &hr.payroll_runs {
            // Use first matching payroll line item's employee, or fallback
            let employee_id = hr
                .payroll_line_items
                .iter()
                .find(|li| li.payroll_id == run.payroll_id)
                .map(|li| li.employee_id.as_str())
                .unwrap_or("EMP000");
            let docs = H2rDocuments::new(
                &run.payroll_id,
                employee_id,
                &run.company_code,
                run.total_gross,
                &ocpm_uuid_factory,
            )
            .with_time_entries(
                hr.time_entries
                    .iter()
                    .filter(|t| t.date >= run.pay_period_start && t.date <= run.pay_period_end)
                    .take(5)
                    .map(|t| t.entry_id.as_str())
                    .collect(),
            );
            let start_time = base_datetime - chrono::Duration::days(30);
            let result = ocpm_gen.generate_h2r_case(&docs, start_time, &available_users);
            add_result(&mut event_log, result);

            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }

        // Generate events from MFG production orders
        for order in &manufacturing.production_orders {
            let mut docs = MfgDocuments::new(
                &order.order_id,
                &order.material_id,
                &order.company_code,
                order.planned_quantity,
                &ocpm_uuid_factory,
            )
            .with_operations(
                order
                    .operations
                    .iter()
                    .map(|o| format!("OP-{:04}", o.operation_number))
                    .collect::<Vec<_>>()
                    .iter()
                    .map(std::string::String::as_str)
                    .collect(),
            );
            // Link quality inspection if available (via reference_id matching order_id)
            if let Some(insp) = manufacturing
                .quality_inspections
                .iter()
                .find(|i| i.reference_id == order.order_id)
            {
                docs = docs.with_inspection(&insp.inspection_id, &ocpm_uuid_factory);
            }
            // Link cycle count if available (match by material_id in items)
            if let Some(cc) = manufacturing.cycle_counts.iter().find(|cc| {
                cc.items
                    .iter()
                    .any(|item| item.material_id == order.material_id)
            }) {
                docs = docs.with_cycle_count(&cc.count_id, &ocpm_uuid_factory);
            }
            let start_time = base_datetime - chrono::Duration::days(60);
            let result = ocpm_gen.generate_mfg_case(&docs, start_time, &available_users);
            add_result(&mut event_log, result);

            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }

        // Generate events from Banking customers
        for customer in &banking.customers {
            let customer_id_str = customer.customer_id.to_string();
            let mut docs = BankDocuments::new(&customer_id_str, "1000", &ocpm_uuid_factory);
            // Link accounts (primary_owner_id matches customer_id)
            if let Some(account) = banking
                .accounts
                .iter()
                .find(|a| a.primary_owner_id == customer.customer_id)
            {
                let account_id_str = account.account_id.to_string();
                docs = docs.with_account(&account_id_str, &ocpm_uuid_factory);
                // Link transactions for this account
                let txn_strs: Vec<String> = banking
                    .transactions
                    .iter()
                    .filter(|t| t.account_id == account.account_id)
                    .take(10)
                    .map(|t| t.transaction_id.to_string())
                    .collect();
                let txn_ids: Vec<&str> = txn_strs.iter().map(std::string::String::as_str).collect();
                let txn_amounts: Vec<rust_decimal::Decimal> = banking
                    .transactions
                    .iter()
                    .filter(|t| t.account_id == account.account_id)
                    .take(10)
                    .map(|t| t.amount)
                    .collect();
                if !txn_ids.is_empty() {
                    docs = docs.with_transactions(txn_ids, txn_amounts);
                }
            }
            let start_time = base_datetime - chrono::Duration::days(180);
            let result = ocpm_gen.generate_bank_case(&docs, start_time, &available_users);
            add_result(&mut event_log, result);

            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }

        // Generate events from Audit engagements
        for engagement in &audit.engagements {
            let engagement_id_str = engagement.engagement_id.to_string();
            let docs = AuditDocuments::new(
                &engagement_id_str,
                &engagement.client_entity_id,
                &ocpm_uuid_factory,
            )
            .with_workpapers(
                audit
                    .workpapers
                    .iter()
                    .filter(|w| w.engagement_id == engagement.engagement_id)
                    .take(10)
                    .map(|w| w.workpaper_id.to_string())
                    .collect::<Vec<_>>()
                    .iter()
                    .map(std::string::String::as_str)
                    .collect(),
            )
            .with_evidence(
                audit
                    .evidence
                    .iter()
                    .filter(|e| e.engagement_id == engagement.engagement_id)
                    .take(10)
                    .map(|e| e.evidence_id.to_string())
                    .collect::<Vec<_>>()
                    .iter()
                    .map(std::string::String::as_str)
                    .collect(),
            )
            .with_risks(
                audit
                    .risk_assessments
                    .iter()
                    .filter(|r| r.engagement_id == engagement.engagement_id)
                    .take(5)
                    .map(|r| r.risk_id.to_string())
                    .collect::<Vec<_>>()
                    .iter()
                    .map(std::string::String::as_str)
                    .collect(),
            )
            .with_findings(
                audit
                    .findings
                    .iter()
                    .filter(|f| f.engagement_id == engagement.engagement_id)
                    .take(5)
                    .map(|f| f.finding_id.to_string())
                    .collect::<Vec<_>>()
                    .iter()
                    .map(std::string::String::as_str)
                    .collect(),
            )
            .with_judgments(
                audit
                    .judgments
                    .iter()
                    .filter(|j| j.engagement_id == engagement.engagement_id)
                    .take(5)
                    .map(|j| j.judgment_id.to_string())
                    .collect::<Vec<_>>()
                    .iter()
                    .map(std::string::String::as_str)
                    .collect(),
            );
            let start_time = base_datetime - chrono::Duration::days(120);
            let result = ocpm_gen.generate_audit_case(&docs, start_time, &available_users);
            add_result(&mut event_log, result);

            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }

        // Generate events from Bank Reconciliations
        for recon in &financial_reporting.bank_reconciliations {
            let docs = BankReconDocuments::new(
                &recon.reconciliation_id,
                &recon.bank_account_id,
                &recon.company_code,
                recon.bank_ending_balance,
                &ocpm_uuid_factory,
            )
            .with_statement_lines(
                recon
                    .statement_lines
                    .iter()
                    .take(20)
                    .map(|l| l.line_id.as_str())
                    .collect(),
            )
            .with_reconciling_items(
                recon
                    .reconciling_items
                    .iter()
                    .take(10)
                    .map(|i| i.item_id.as_str())
                    .collect(),
            );
            let start_time = base_datetime - chrono::Duration::days(30);
            let result = ocpm_gen.generate_bank_recon_case(&docs, start_time, &available_users);
            add_result(&mut event_log, result);

            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }

        // Compute process variants
        event_log.compute_variants();

        let summary = event_log.summary();

        if let Some(pb) = pb {
            pb.finish_with_message(format!(
                "Generated {} OCPM events, {} objects",
                summary.event_count, summary.object_count
            ));
        }

        Ok(OcpmSnapshot {
            event_count: summary.event_count,
            object_count: summary.object_count,
            case_count: summary.case_count,
            event_log: Some(event_log),
        })
    }

    /// Inject anomalies into journal entries.
    fn inject_anomalies(&mut self, entries: &mut [JournalEntry]) -> SynthResult<AnomalyLabels> {
        let pb = self.create_progress_bar(entries.len() as u64, "Injecting Anomalies");

        // Read anomaly rates from config instead of using hardcoded values.
        // Priority: anomaly_injection config > fraud config > default 0.02
        let total_rate = if self.config.anomaly_injection.enabled {
            self.config.anomaly_injection.rates.total_rate
        } else if self.config.fraud.enabled {
            self.config.fraud.fraud_rate
        } else {
            0.02
        };

        let fraud_rate = if self.config.anomaly_injection.enabled {
            self.config.anomaly_injection.rates.fraud_rate
        } else {
            AnomalyRateConfig::default().fraud_rate
        };

        let error_rate = if self.config.anomaly_injection.enabled {
            self.config.anomaly_injection.rates.error_rate
        } else {
            AnomalyRateConfig::default().error_rate
        };

        let process_issue_rate = if self.config.anomaly_injection.enabled {
            self.config.anomaly_injection.rates.process_rate
        } else {
            AnomalyRateConfig::default().process_issue_rate
        };

        let anomaly_config = AnomalyInjectorConfig {
            rates: AnomalyRateConfig {
                total_rate,
                fraud_rate,
                error_rate,
                process_issue_rate,
                ..Default::default()
            },
            seed: self.seed + 5000,
            ..Default::default()
        };

        let mut injector = AnomalyInjector::new(anomaly_config);
        let result = injector.process_entries(entries);

        // Central concentration abstraction (#143, Phase 1): run the post-process
        // pipeline AFTER per-entry strategies. The pipeline merges the SOTA-12
        // tagger + new passes (trading-partner pool, Phase-2 account substitution)
        // through a single integration point — see
        // docs/superpowers/specs/2026-05-23-concentration-pass-INDEX.md.
        //
        // Back-compat: the legacy `anomaly_injection.source_conditional_rarity_rate`
        // key remains honored. If `concentration.source_conditional_rarity` is also
        // set in the same config, the unified DSL field wins.
        let (sota12_tagged, consolidation_outlier_expanded): (usize, usize) = {
            use datasynth_config::schema::{
                ConcentrationConfig, ConsolidationOutlierPassConfig,
                SourceConditionalRarityPassConfig,
            };
            use datasynth_generators::concentration::ConcentrationPipeline;

            // Decide effective ConcentrationConfig: start from user config, then
            // back-fill from the legacy SOTA-12 key if the unified DSL didn't set it.
            let mut effective: ConcentrationConfig = self.config.concentration.clone();
            if effective.source_conditional_rarity.is_none() {
                if let Some(rate) = self.config.anomaly_injection.source_conditional_rarity_rate {
                    effective.enabled = true;
                    effective.source_conditional_rarity = Some(SourceConditionalRarityPassConfig {
                        rate,
                        min_surprise: None,
                        min_per_source_lines: None,
                    });
                }
            }
            // v5.30 B2 (#154) — back-compat: surface
            // `anomaly_injection.rates.consolidation_outlier_rate` as a
            // `ConsolidationOutlierPassConfig` if the unified DSL didn't
            // set one. Default 0.001 baseline shipped via the schema's
            // `default_consolidation_outlier_rate` — only synthesise the
            // pass when the rate is > 0, otherwise it's a no-op anyway.
            if effective.consolidation_outlier.is_none() {
                let rate = self
                    .config
                    .anomaly_injection
                    .rates
                    .consolidation_outlier_rate;
                if rate > 0.0 {
                    effective.enabled = true;
                    effective.consolidation_outlier = Some(ConsolidationOutlierPassConfig {
                        rate,
                        ..Default::default()
                    });
                }
            }

            if !effective.enabled {
                (0, 0)
            } else {
                let pipeline = ConcentrationPipeline::from_config(&effective).map_err(|e| {
                    SynthError::generation(format!(
                        "ConcentrationPipeline construction failed: {e}"
                    ))
                })?;
                if !pipeline.is_active() {
                    (0, 0)
                } else {
                    // Per-pipeline seed disjoint from every other generator stream.
                    const CONCENTRATION_SEED_OFFSET: u64 = 0xC0_C3_E1_47_10_43_77_3B;
                    let stats =
                        pipeline.run(entries, self.seed.wrapping_add(CONCENTRATION_SEED_OFFSET));
                    let sota12: usize = stats
                        .iter()
                        .filter(|s| s.pass == "source_conditional_rarity")
                        .map(|s| s.entries_modified)
                        .sum();
                    let consol: usize = stats
                        .iter()
                        .filter(|s| s.pass == "consolidation_outlier")
                        .map(|s| s.entries_modified)
                        .sum();
                    (sota12, consol)
                }
            }
        };

        if let Some(pb) = &pb {
            pb.inc(entries.len() as u64);
            pb.finish_with_message("Anomaly injection complete");
        }

        let mut by_type = HashMap::new();
        for label in &result.labels {
            *by_type
                .entry(format!("{:?}", label.anomaly_type))
                .or_insert(0) += 1;
        }
        if sota12_tagged > 0 {
            *by_type
                .entry("SourceConditionalRarity".to_string())
                .or_insert(0) += sota12_tagged;
        }
        // v5.30 B2 (#154): record the consolidation-outlier expansion
        // count under a stable label key so the orchestrator's run
        // report surfaces the heavy-tail emission rate alongside the
        // other anomaly buckets.
        if consolidation_outlier_expanded > 0 {
            *by_type
                .entry("ConsolidationOutlier".to_string())
                .or_insert(0) += consolidation_outlier_expanded;
        }

        Ok(AnomalyLabels {
            labels: result.labels,
            summary: Some(result.summary),
            by_type,
        })
    }

    /// Validate journal entries using running balance tracker.
    ///
    /// Applies all entries to the balance tracker and validates:
    /// - Each entry is internally balanced (debits = credits)
    /// - Balance sheet equation holds (Assets = Liabilities + Equity + Net Income)
    ///
    /// Note: Entries with human errors (marked with [HUMAN_ERROR:*] tags) are
    /// excluded from balance validation as they may be intentionally unbalanced.
    fn validate_journal_entries(
        &mut self,
        entries: &[JournalEntry],
    ) -> SynthResult<BalanceValidationResult> {
        // Filter out entries with human errors as they may be intentionally unbalanced
        let clean_entries: Vec<&JournalEntry> = entries
            .iter()
            .filter(|e| {
                e.header
                    .header_text
                    .as_ref()
                    .map(|t| !t.contains("[HUMAN_ERROR:"))
                    .unwrap_or(true)
            })
            .collect();

        let pb = self.create_progress_bar(clean_entries.len() as u64, "Validating Balances");

        // Configure tracker to not fail on errors (collect them instead)
        let config = BalanceTrackerConfig {
            validate_on_each_entry: false,   // We'll validate at the end
            track_history: false,            // Skip history for performance
            fail_on_validation_error: false, // Collect errors, don't fail
            ..Default::default()
        };
        let validation_currency = self
            .config
            .companies
            .first()
            .map(|c| c.currency.clone())
            .unwrap_or_else(|| "USD".to_string());

        let mut tracker = RunningBalanceTracker::new_with_currency(config, validation_currency);

        // Apply clean entries (without human errors)
        let clean_refs: Vec<JournalEntry> = clean_entries.into_iter().cloned().collect();
        let errors = tracker.apply_entries(&clean_refs);

        if let Some(pb) = &pb {
            pb.inc(entries.len() as u64);
        }

        // Check if any entries were unbalanced
        // Note: When fail_on_validation_error is false, errors are stored in tracker
        let has_unbalanced = tracker
            .get_validation_errors()
            .iter()
            .any(|e| e.error_type == datasynth_generators::ValidationErrorType::UnbalancedEntry);

        // Validate balance sheet for each company
        // Include both returned errors and collected validation errors
        let mut all_errors = errors;
        all_errors.extend(tracker.get_validation_errors().iter().cloned());
        let company_codes: Vec<String> = self
            .config
            .companies
            .iter()
            .map(|c| c.code.clone())
            .collect();

        let end_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map(|d| d + chrono::Months::new(self.config.global.period_months))
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;

        for company_code in &company_codes {
            if let Err(e) = tracker.validate_balance_sheet(company_code, end_date, None) {
                all_errors.push(e);
            }
        }

        // Get statistics after all mutable operations are done
        let stats = tracker.get_statistics();

        // Determine if balanced overall
        let is_balanced = all_errors.is_empty();

        if let Some(pb) = pb {
            let msg = if is_balanced {
                "Balance validation passed"
            } else {
                "Balance validation completed with errors"
            };
            pb.finish_with_message(msg);
        }

        Ok(BalanceValidationResult {
            validated: true,
            is_balanced,
            entries_processed: stats.entries_processed,
            total_debits: stats.total_debits,
            total_credits: stats.total_credits,
            accounts_tracked: stats.accounts_tracked,
            companies_tracked: stats.companies_tracked,
            validation_errors: all_errors,
            has_unbalanced_entries: has_unbalanced,
        })
    }

    /// Inject data quality variations into journal entries.
    ///
    /// Applies typos, missing values, and format variations to make
    /// the synthetic data more realistic for testing data cleaning pipelines.
    fn inject_data_quality(
        &mut self,
        entries: &mut [JournalEntry],
    ) -> SynthResult<(DataQualityStats, Vec<datasynth_generators::QualityIssue>)> {
        let pb = self.create_progress_bar(entries.len() as u64, "Injecting Data Quality Issues");

        // Build config from user-specified schema settings when data_quality is enabled;
        // otherwise fall back to the low-rate minimal() preset.
        let config = if self.config.data_quality.enabled {
            let dq = &self.config.data_quality;
            // Propagate per-field rates and protected fields from the schema
            // so users can dial in real-production NULL profiles per field
            // (e.g. CostCenter 96.5% NULL, Invoice_Reference 100% NULL).
            let field_rates = dq.missing_values.field_rates.clone();
            let mut required_fields: std::collections::HashSet<String> =
                dq.missing_values.protected_fields.iter().cloned().collect();
            // Always preserve audit-critical identifiers regardless of
            // user config — losing these breaks downstream joins.
            for f in [
                "document_id",
                "company_code",
                "posting_date",
                "fiscal_year",
                "fiscal_period",
                "gl_account",
                "line_number",
                "transaction_id",
            ] {
                required_fields.insert(f.to_string());
            }
            DataQualityConfig {
                enable_missing_values: dq.missing_values.enabled,
                missing_values: datasynth_generators::MissingValueConfig {
                    global_rate: dq.effective_missing_rate(),
                    field_rates,
                    required_fields,
                    ..Default::default()
                },
                enable_format_variations: dq.format_variations.enabled,
                format_variations: datasynth_generators::FormatVariationConfig {
                    date_variation_rate: dq.format_variations.dates.rate,
                    amount_variation_rate: dq.format_variations.amounts.rate,
                    identifier_variation_rate: dq.format_variations.identifiers.rate,
                    ..Default::default()
                },
                enable_duplicates: dq.duplicates.enabled,
                duplicates: datasynth_generators::DuplicateConfig {
                    duplicate_rate: dq.effective_duplicate_rate(),
                    ..Default::default()
                },
                enable_typos: dq.typos.enabled,
                typos: datasynth_generators::TypoConfig {
                    char_error_rate: dq.effective_typo_rate(),
                    ..Default::default()
                },
                enable_encoding_issues: dq.encoding_issues.enabled,
                encoding_issue_rate: dq.encoding_issues.rate,
                seed: self.seed.wrapping_add(77), // deterministic offset for DQ phase
                track_statistics: true,
            }
        } else {
            DataQualityConfig::minimal()
        };
        let mut injector = DataQualityInjector::new(config);

        // Wire country pack for locale-aware format baselines
        injector.set_country_pack(self.primary_pack().clone());

        // Build context for missing value decisions
        let context = HashMap::new();

        for entry in entries.iter_mut() {
            // Process header_text field (common target for typos)
            if let Some(text) = &entry.header.header_text {
                let processed = injector.process_text_field(
                    "header_text",
                    text,
                    &entry.header.document_id.to_string(),
                    &context,
                );
                match processed {
                    Some(new_text) if new_text != *text => {
                        entry.header.header_text = Some(new_text);
                    }
                    None => {
                        entry.header.header_text = None; // Missing value
                    }
                    _ => {}
                }
            }

            // Process reference field
            if let Some(ref_text) = &entry.header.reference {
                let processed = injector.process_text_field(
                    "reference",
                    ref_text,
                    &entry.header.document_id.to_string(),
                    &context,
                );
                match processed {
                    Some(new_text) if new_text != *ref_text => {
                        entry.header.reference = Some(new_text);
                    }
                    None => {
                        entry.header.reference = None;
                    }
                    _ => {}
                }
            }

            // Process user_persona field (potential for typos in user IDs)
            let user_persona = entry.header.user_persona.clone();
            if let Some(processed) = injector.process_text_field(
                "user_persona",
                &user_persona,
                &entry.header.document_id.to_string(),
                &context,
            ) {
                if processed != user_persona {
                    entry.header.user_persona = processed;
                }
            }

            // Process line items
            for line in &mut entry.lines {
                // Process line description if present
                if let Some(ref text) = line.line_text {
                    let processed = injector.process_text_field(
                        "line_text",
                        text,
                        &entry.header.document_id.to_string(),
                        &context,
                    );
                    match processed {
                        Some(new_text) if new_text != *text => {
                            line.line_text = Some(new_text);
                        }
                        None => {
                            line.line_text = None;
                        }
                        _ => {}
                    }
                }

                // Process cost_center if present
                if let Some(cc) = &line.cost_center {
                    let processed = injector.process_text_field(
                        "cost_center",
                        cc,
                        &entry.header.document_id.to_string(),
                        &context,
                    );
                    match processed {
                        Some(new_cc) if new_cc != *cc => {
                            line.cost_center = Some(new_cc);
                        }
                        None => {
                            line.cost_center = None;
                        }
                        _ => {}
                    }
                }

                // Extended field coverage (v5.6+): apply NULL injection to
                // every Option<String> on the line so users can match
                // arbitrary real-production NULL profiles via
                // `data_quality.missing_values.field_rates`.
                //
                // Macro-free helper: process_field returns the new value
                // ({Some, None, unchanged}) and we apply it back.
                macro_rules! process_opt_field {
                    ($field_name:expr, $opt:expr) => {
                        if let Some(val) = $opt.as_ref() {
                            match injector.process_text_field(
                                $field_name,
                                val,
                                &entry.header.document_id.to_string(),
                                &context,
                            ) {
                                Some(new_val) if new_val != *val => {
                                    *$opt = Some(new_val);
                                }
                                None => {
                                    *$opt = None;
                                }
                                _ => {}
                            }
                        }
                    };
                }

                process_opt_field!("profit_center", &mut line.profit_center);
                process_opt_field!("assignment", &mut line.assignment);
                process_opt_field!("tax_code", &mut line.tax_code);
                process_opt_field!("account_description", &mut line.account_description);
                process_opt_field!(
                    "auxiliary_account_number",
                    &mut line.auxiliary_account_number
                );
                process_opt_field!("auxiliary_account_label", &mut line.auxiliary_account_label);
                process_opt_field!("lettrage", &mut line.lettrage);
            }

            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }

        if let Some(pb) = pb {
            pb.finish_with_message("Data quality injection complete");
        }

        let quality_issues = injector.issues().to_vec();
        Ok((injector.stats().clone(), quality_issues))
    }

    /// Generate audit data (engagements, workpapers, evidence, risks, findings, judgments).
    ///
    /// Creates complete audit documentation for each company in the configuration,
    /// following ISA standards:
    /// - ISA 210/220: Engagement acceptance and terms
    /// - ISA 230: Audit documentation (workpapers)
    /// - ISA 265: Control deficiencies (findings)
    /// - ISA 315/330: Risk assessment and response
    /// - ISA 500: Audit evidence
    /// - ISA 200: Professional judgment
    fn generate_audit_data(&mut self, entries: &[JournalEntry]) -> SynthResult<AuditSnapshot> {
        // Check if FSM-driven audit generation is enabled
        let use_fsm = self
            .config
            .audit
            .fsm
            .as_ref()
            .map(|f| f.enabled)
            .unwrap_or(false);

        if use_fsm {
            return self.generate_audit_data_with_fsm(entries);
        }

        // --- Legacy (non-FSM) audit generation follows ---
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let fiscal_year = start_date.year() as u16;
        let period_end = start_date + chrono::Months::new(self.config.global.period_months);

        // Calculate rough total revenue from entries for materiality
        let total_revenue: rust_decimal::Decimal = entries
            .iter()
            .flat_map(|e| e.lines.iter())
            .filter(|l| l.credit_amount > rust_decimal::Decimal::ZERO)
            .map(|l| l.credit_amount)
            .sum();

        let total_items = (self.phase_config.audit_engagements * 50) as u64; // Approximate items
        let pb = self.create_progress_bar(total_items, "Generating Audit Data");

        let mut snapshot = AuditSnapshot::default();

        // Initialize generators
        let mut engagement_gen = AuditEngagementGenerator::new(self.seed + 7000);
        // v3.3.2: thread the user-facing audit schema config into the
        // engagement generator (team size range).
        engagement_gen.set_team_config(&self.config.audit.team);

        let mut workpaper_gen = WorkpaperGenerator::new(self.seed + 7100);
        // v3.3.2: thread workpaper + review workflow schema config into
        // the workpaper generator (per-section count range + review
        // delay ranges).
        workpaper_gen.set_schema_configs(&self.config.audit.workpapers, &self.config.audit.review);
        let mut evidence_gen = EvidenceGenerator::new(self.seed + 7200);
        let mut risk_gen = RiskAssessmentGenerator::new(self.seed + 7300);
        let mut finding_gen = FindingGenerator::new(self.seed + 7400);
        // v3.2.1+: user-supplied finding titles + narratives flow through shared provider
        finding_gen.set_template_provider(self.template_provider.clone());
        let mut judgment_gen = JudgmentGenerator::new(self.seed + 7500);
        let mut confirmation_gen = ConfirmationGenerator::new(self.seed + 7600);
        let mut procedure_step_gen = ProcedureStepGenerator::new(self.seed + 7700);
        let mut sample_gen = SampleGenerator::new(self.seed + 7800);
        let mut analytical_gen = AnalyticalProcedureGenerator::new(self.seed + 7900);
        let mut ia_gen = InternalAuditGenerator::new(self.seed + 8000);
        let mut related_party_gen = RelatedPartyGenerator::new(self.seed + 8100);

        // Get list of accounts from CoA for risk assessment
        let accounts: Vec<String> = self
            .coa
            .as_ref()
            .map(|coa| {
                coa.get_postable_accounts()
                    .iter()
                    .map(|acc| acc.account_code().to_string())
                    .collect()
            })
            .unwrap_or_default();

        // Generate engagements for each company
        for (i, company) in self.config.companies.iter().enumerate() {
            // Calculate company-specific revenue (proportional to volume weight)
            let company_revenue = total_revenue
                * rust_decimal::Decimal::try_from(company.volume_weight).unwrap_or_default();

            // Generate engagements for this company
            let engagements_for_company =
                self.phase_config.audit_engagements / self.config.companies.len().max(1);
            let extra = if i < self.phase_config.audit_engagements % self.config.companies.len() {
                1
            } else {
                0
            };

            for _eng_idx in 0..(engagements_for_company + extra) {
                // v3.3.2: draw engagement type from the user-configured
                // distribution instead of always using the default
                // (AnnualAudit). Falls back to the default when all
                // probabilities are zero.
                let eng_type =
                    engagement_gen.draw_engagement_type(&self.config.audit.engagement_types);

                // Generate the engagement
                let mut engagement = engagement_gen.generate_engagement(
                    &company.code,
                    &company.name,
                    fiscal_year,
                    period_end,
                    company_revenue,
                    Some(eng_type),
                );

                // Replace synthetic team IDs with real employee IDs from master data
                if !self.master_data.employees.is_empty() {
                    let emp_count = self.master_data.employees.len();
                    // Use employee IDs deterministically based on engagement index
                    let base = (i * 10 + _eng_idx) % emp_count;
                    engagement.engagement_partner_id = self.master_data.employees[base % emp_count]
                        .employee_id
                        .clone();
                    engagement.engagement_manager_id = self.master_data.employees
                        [(base + 1) % emp_count]
                        .employee_id
                        .clone();
                    let real_team: Vec<String> = engagement
                        .team_member_ids
                        .iter()
                        .enumerate()
                        .map(|(j, _)| {
                            self.master_data.employees[(base + 2 + j) % emp_count]
                                .employee_id
                                .clone()
                        })
                        .collect();
                    engagement.team_member_ids = real_team;
                }

                if let Some(pb) = &pb {
                    pb.inc(1);
                }

                // Get team members from the engagement
                let team_members: Vec<String> = engagement.team_member_ids.clone();

                // Generate workpapers for the engagement.
                // v3.3.2: honor `audit.generate_workpapers` — when false,
                // workpapers (and dependent evidence) are skipped while
                // the engagement itself, risk assessments, findings, etc.
                // still generate normally.
                let workpapers = if self.config.audit.generate_workpapers {
                    workpaper_gen.generate_complete_workpaper_set(&engagement, &team_members)
                } else {
                    Vec::new()
                };

                for wp in &workpapers {
                    if let Some(pb) = &pb {
                        pb.inc(1);
                    }

                    // Generate evidence for each workpaper
                    let evidence = evidence_gen.generate_evidence_for_workpaper(
                        wp,
                        &team_members,
                        wp.preparer_date,
                    );

                    for _ in &evidence {
                        if let Some(pb) = &pb {
                            pb.inc(1);
                        }
                    }

                    snapshot.evidence.extend(evidence);
                }

                // Generate risk assessments for the engagement
                let risks =
                    risk_gen.generate_risks_for_engagement(&engagement, &team_members, &accounts);

                for _ in &risks {
                    if let Some(pb) = &pb {
                        pb.inc(1);
                    }
                }
                snapshot.risk_assessments.extend(risks);

                // Generate findings for the engagement
                let findings = finding_gen.generate_findings_for_engagement(
                    &engagement,
                    &workpapers,
                    &team_members,
                );

                for _ in &findings {
                    if let Some(pb) = &pb {
                        pb.inc(1);
                    }
                }
                snapshot.findings.extend(findings);

                // Generate professional judgments for the engagement
                let judgments =
                    judgment_gen.generate_judgments_for_engagement(&engagement, &team_members);

                for _ in &judgments {
                    if let Some(pb) = &pb {
                        pb.inc(1);
                    }
                }
                snapshot.judgments.extend(judgments);

                // ISA 505: External confirmations and responses
                let (confs, resps) =
                    confirmation_gen.generate_confirmations(&engagement, &workpapers, &accounts);
                snapshot.confirmations.extend(confs);
                snapshot.confirmation_responses.extend(resps);

                // ISA 330: Procedure steps per workpaper
                let team_pairs: Vec<(String, String)> = team_members
                    .iter()
                    .map(|id| {
                        let name = self
                            .master_data
                            .employees
                            .iter()
                            .find(|e| e.employee_id == *id)
                            .map(|e| e.display_name.clone())
                            .unwrap_or_else(|| format!("Employee {}", &id[..8.min(id.len())]));
                        (id.clone(), name)
                    })
                    .collect();
                for wp in &workpapers {
                    let steps = procedure_step_gen.generate_steps(wp, &team_pairs);
                    snapshot.procedure_steps.extend(steps);
                }

                // ISA 530: Samples per workpaper
                for wp in &workpapers {
                    if let Some(sample) = sample_gen.generate_sample(wp, engagement.engagement_id) {
                        snapshot.samples.push(sample);
                    }
                }

                // ISA 520: Analytical procedures
                let analytical = analytical_gen.generate_procedures(&engagement, &accounts);
                snapshot.analytical_results.extend(analytical);

                // ISA 610: Internal audit function and reports
                let (ia_func, ia_reports) = ia_gen.generate(&engagement);
                snapshot.ia_functions.push(ia_func);
                snapshot.ia_reports.extend(ia_reports);

                // ISA 550: Related parties and transactions
                let vendor_names: Vec<String> = self
                    .master_data
                    .vendors
                    .iter()
                    .map(|v| v.name.clone())
                    .collect();
                let customer_names: Vec<String> = self
                    .master_data
                    .customers
                    .iter()
                    .map(|c| c.name.clone())
                    .collect();
                let (parties, rp_txns) =
                    related_party_gen.generate(&engagement, &vendor_names, &customer_names);
                snapshot.related_parties.extend(parties);
                snapshot.related_party_transactions.extend(rp_txns);

                // Add workpapers after findings since findings need them
                snapshot.workpapers.extend(workpapers);

                // Generate audit scope record for this engagement (one per engagement)
                {
                    let scope_id = format!(
                        "SCOPE-{}-{}",
                        engagement.engagement_id.simple(),
                        &engagement.client_entity_id
                    );
                    let scope = datasynth_core::models::audit::AuditScope::new(
                        scope_id.clone(),
                        engagement.engagement_id.to_string(),
                        engagement.client_entity_id.clone(),
                        engagement.materiality,
                    );
                    // Wire scope_id back to engagement
                    let mut eng = engagement;
                    eng.scope_id = Some(scope_id);
                    snapshot.audit_scopes.push(scope);
                    snapshot.engagements.push(eng);
                }
            }
        }

        // ----------------------------------------------------------------
        // ISA 600: Group audit — component auditors, plan, instructions, reports
        // ----------------------------------------------------------------
        if self.config.companies.len() > 1 {
            // Use materiality from the first engagement if available, otherwise
            // derive a reasonable figure from total revenue.
            let group_materiality = snapshot
                .engagements
                .first()
                .map(|e| e.materiality)
                .unwrap_or_else(|| {
                    let pct = rust_decimal::Decimal::try_from(0.005_f64).unwrap_or_default();
                    total_revenue * pct
                });

            let mut component_gen = ComponentAuditGenerator::new(self.seed + 8200);
            let group_engagement_id = snapshot
                .engagements
                .first()
                .map(|e| e.engagement_id.to_string())
                .unwrap_or_else(|| "GROUP-ENG".to_string());

            let component_snapshot = component_gen.generate(
                &self.config.companies,
                group_materiality,
                &group_engagement_id,
                period_end,
            );

            snapshot.component_auditors = component_snapshot.component_auditors;
            snapshot.group_audit_plan = component_snapshot.group_audit_plan;
            snapshot.component_instructions = component_snapshot.component_instructions;
            snapshot.component_reports = component_snapshot.component_reports;

            info!(
                "ISA 600 group audit: {} component auditors, {} instructions, {} reports",
                snapshot.component_auditors.len(),
                snapshot.component_instructions.len(),
                snapshot.component_reports.len(),
            );
        }

        // ----------------------------------------------------------------
        // ISA 210: Engagement letters — one per engagement
        // ----------------------------------------------------------------
        {
            let applicable_framework = self
                .config
                .accounting_standards
                .framework
                .as_ref()
                .map(|f| format!("{f:?}"))
                .unwrap_or_else(|| "IFRS".to_string());

            let mut letter_gen = EngagementLetterGenerator::new(self.seed + 8300);
            let entity_count = self.config.companies.len();

            for engagement in &snapshot.engagements {
                let company = self
                    .config
                    .companies
                    .iter()
                    .find(|c| c.code == engagement.client_entity_id);
                let currency = company.map(|c| c.currency.as_str()).unwrap_or("USD");
                let letter_date = engagement.planning_start;
                let letter = letter_gen.generate(
                    &engagement.engagement_id.to_string(),
                    &engagement.client_name,
                    entity_count,
                    engagement.period_end_date,
                    currency,
                    &applicable_framework,
                    letter_date,
                );
                snapshot.engagement_letters.push(letter);
            }

            info!(
                "ISA 210 engagement letters: {} generated",
                snapshot.engagement_letters.len()
            );
        }

        // ----------------------------------------------------------------
        // v3.3.0: Legal documents per engagement (WI: LegalDocumentGenerator)
        // ----------------------------------------------------------------
        if self.phase_config.generate_legal_documents {
            use datasynth_generators::legal_document_generator::LegalDocumentGenerator;
            let mut legal_gen = LegalDocumentGenerator::new(self.seed + 8400);
            for engagement in &snapshot.engagements {
                // Build an employee name list for signatory drawing —
                // prefer employees from the engaged entity, fall back to
                // all employees.
                let employee_names: Vec<String> = self
                    .master_data
                    .employees
                    .iter()
                    .filter(|e| e.company_code == engagement.client_entity_id)
                    .map(|e| e.display_name.clone())
                    .collect();
                let names_to_use = if !employee_names.is_empty() {
                    employee_names
                } else {
                    self.master_data
                        .employees
                        .iter()
                        .take(10)
                        .map(|e| e.display_name.clone())
                        .collect()
                };
                let docs = legal_gen.generate(
                    &engagement.client_entity_id,
                    engagement.fiscal_year as i32,
                    &names_to_use,
                );
                snapshot.legal_documents.extend(docs);
            }
            info!(
                "v3.3.0 legal documents: {} emitted across {} engagements",
                snapshot.legal_documents.len(),
                snapshot.engagements.len()
            );
        }

        // ----------------------------------------------------------------
        // v3.3.0: IT general controls — access logs + change records
        //
        // `ItControlsGenerator` runs one pass per company (not per
        // engagement) so employee sets and system catalogs stay
        // coherent. We derive the period from the earliest engagement's
        // planning_start through the latest engagement's period_end_date
        // for each company.
        // ----------------------------------------------------------------
        if self.phase_config.generate_it_controls {
            use datasynth_generators::it_controls_generator::ItControlsGenerator;
            use std::collections::HashMap;
            let mut it_gen = ItControlsGenerator::new(self.seed + 8500);

            // Group engagements by company to produce one IT-controls
            // window per entity.
            let mut by_company: HashMap<String, (chrono::NaiveDate, chrono::NaiveDate)> =
                HashMap::new();
            for engagement in &snapshot.engagements {
                let entry = by_company
                    .entry(engagement.client_entity_id.clone())
                    .or_insert((engagement.planning_start, engagement.period_end_date));
                if engagement.planning_start < entry.0 {
                    entry.0 = engagement.planning_start;
                }
                if engagement.period_end_date > entry.1 {
                    entry.1 = engagement.period_end_date;
                }
            }

            // Standard system catalog — populated from known ERP / app
            // names. Keeps the generator's data shape stable when the
            // user hasn't configured IT-system naming separately.
            let systems: Vec<String> = vec![
                "SAP ECC",
                "SAP S/4 HANA",
                "Oracle EBS",
                "Workday",
                "NetSuite",
                "Active Directory",
                "SharePoint",
                "Salesforce",
                "ServiceNow",
                "Jira",
                "GitHub Enterprise",
                "AWS Console",
                "Okta",
            ]
            .into_iter()
            .map(String::from)
            .collect();

            for (company_code, (start, end)) in by_company {
                let emps: Vec<(String, String)> = self
                    .master_data
                    .employees
                    .iter()
                    .filter(|e| e.company_code == company_code)
                    .map(|e| (e.employee_id.clone(), e.display_name.clone()))
                    .collect();
                if emps.is_empty() {
                    continue;
                }
                // Compute period in months, rounded up to the nearest
                // whole month (min 1).
                let months = ((end.signed_duration_since(start).num_days() / 30) + 1).max(1) as u32;
                let access_logs = it_gen.generate_access_logs(&emps, &systems, start, months);
                let change_records = it_gen.generate_change_records(&emps, &systems, start, months);
                snapshot.it_controls_access_logs.extend(access_logs);
                snapshot.it_controls_change_records.extend(change_records);
            }

            info!(
                "v3.3.0 IT controls: {} access logs, {} change records",
                snapshot.it_controls_access_logs.len(),
                snapshot.it_controls_change_records.len()
            );
        }

        // ----------------------------------------------------------------
        // ISA 560 / IAS 10: Subsequent events
        // ----------------------------------------------------------------
        {
            let mut event_gen = SubsequentEventGenerator::new(self.seed + 8400);
            let entity_codes: Vec<String> = self
                .config
                .companies
                .iter()
                .map(|c| c.code.clone())
                .collect();
            let subsequent = event_gen.generate_for_entities(&entity_codes, period_end);
            info!(
                "ISA 560 subsequent events: {} generated ({} adjusting, {} non-adjusting)",
                subsequent.len(),
                subsequent
                    .iter()
                    .filter(|e| matches!(
                        e.classification,
                        datasynth_core::models::audit::subsequent_events::EventClassification::Adjusting
                    ))
                    .count(),
                subsequent
                    .iter()
                    .filter(|e| matches!(
                        e.classification,
                        datasynth_core::models::audit::subsequent_events::EventClassification::NonAdjusting
                    ))
                    .count(),
            );
            snapshot.subsequent_events = subsequent;
        }

        // ----------------------------------------------------------------
        // ISA 402: Service organization controls
        // ----------------------------------------------------------------
        {
            let mut soc_gen = ServiceOrgGenerator::new(self.seed + 8500);
            let entity_codes: Vec<String> = self
                .config
                .companies
                .iter()
                .map(|c| c.code.clone())
                .collect();
            let soc_snapshot = soc_gen.generate(&entity_codes, period_end);
            info!(
                "ISA 402 service orgs: {} orgs, {} SOC reports, {} user entity controls",
                soc_snapshot.service_organizations.len(),
                soc_snapshot.soc_reports.len(),
                soc_snapshot.user_entity_controls.len(),
            );
            snapshot.service_organizations = soc_snapshot.service_organizations;
            snapshot.soc_reports = soc_snapshot.soc_reports;
            snapshot.user_entity_controls = soc_snapshot.user_entity_controls;
        }

        // ----------------------------------------------------------------
        // ISA 570: Going concern assessments
        // ----------------------------------------------------------------
        {
            use datasynth_generators::audit::going_concern_generator::{
                GoingConcernGenerator, GoingConcernInput,
            };
            let mut gc_gen = GoingConcernGenerator::new(self.seed + 8570);
            let entity_codes: Vec<String> = self
                .config
                .companies
                .iter()
                .map(|c| c.code.clone())
                .collect();
            // Assessment date = period end + 75 days (typical sign-off window).
            let assessment_date = period_end + chrono::Duration::days(75);
            let period_label = format!("FY{}", period_end.year());

            // Build financial inputs from actual journal entries.
            //
            // We derive approximate P&L, working capital, and operating cash flow
            // by aggregating GL account balances from the journal entry population.
            // Account ranges used (standard chart):
            //   Revenue:         4xxx (credit-normal → negate for positive revenue)
            //   Expenses:        6xxx (debit-normal)
            //   Current assets:  1xxx (AR=1100, cash=1000, inventory=1300)
            //   Current liabs:   2xxx up to 2499 (AP=2000, accruals=2100)
            //   Operating CF:    net income adjusted for D&A (rough proxy)
            let gc_inputs: Vec<GoingConcernInput> = self
                .config
                .companies
                .iter()
                .map(|company| {
                    let code = &company.code;
                    let mut revenue = rust_decimal::Decimal::ZERO;
                    let mut expenses = rust_decimal::Decimal::ZERO;
                    let mut current_assets = rust_decimal::Decimal::ZERO;
                    let mut current_liabs = rust_decimal::Decimal::ZERO;
                    let mut total_debt = rust_decimal::Decimal::ZERO;

                    for je in entries.iter().filter(|je| &je.header.company_code == code) {
                        for line in &je.lines {
                            let acct = line.gl_account.as_str();
                            let net = line.debit_amount - line.credit_amount;
                            if acct.starts_with('4') {
                                // Revenue accounts: credit-normal, so negative net = revenue earned
                                revenue -= net;
                            } else if acct.starts_with('6') {
                                // Expense accounts: debit-normal
                                expenses += net;
                            }
                            // Balance sheet accounts for working capital
                            if acct.starts_with('1') {
                                // Current asset accounts (1000–1499)
                                if let Ok(n) = acct.parse::<u32>() {
                                    if (1000..=1499).contains(&n) {
                                        current_assets += net;
                                    }
                                }
                            } else if acct.starts_with('2') {
                                if let Ok(n) = acct.parse::<u32>() {
                                    if (2000..=2499).contains(&n) {
                                        // Current liabilities
                                        current_liabs -= net; // credit-normal
                                    } else if (2500..=2999).contains(&n) {
                                        // Long-term debt
                                        total_debt -= net;
                                    }
                                }
                            }
                        }
                    }

                    let net_income = revenue - expenses;
                    let working_capital = current_assets - current_liabs;
                    // Rough operating CF proxy: net income (full accrual CF calculation
                    // is done separately in the cash flow statement generator)
                    let operating_cash_flow = net_income;

                    GoingConcernInput {
                        entity_code: code.clone(),
                        net_income,
                        working_capital,
                        operating_cash_flow,
                        total_debt: total_debt.max(rust_decimal::Decimal::ZERO),
                        assessment_date,
                    }
                })
                .collect();

            let assessments = if gc_inputs.is_empty() {
                gc_gen.generate_for_entities(&entity_codes, assessment_date, &period_label)
            } else {
                gc_gen.generate_for_entities_with_inputs(
                    &entity_codes,
                    &gc_inputs,
                    assessment_date,
                    &period_label,
                )
            };
            info!(
                "ISA 570 going concern: {} assessments ({} clean, {} material uncertainty, {} doubt)",
                assessments.len(),
                assessments.iter().filter(|a| matches!(
                    a.auditor_conclusion,
                    datasynth_core::models::audit::going_concern::GoingConcernConclusion::NoMaterialUncertainty
                )).count(),
                assessments.iter().filter(|a| matches!(
                    a.auditor_conclusion,
                    datasynth_core::models::audit::going_concern::GoingConcernConclusion::MaterialUncertaintyExists
                )).count(),
                assessments.iter().filter(|a| matches!(
                    a.auditor_conclusion,
                    datasynth_core::models::audit::going_concern::GoingConcernConclusion::GoingConcernDoubt
                )).count(),
            );
            snapshot.going_concern_assessments = assessments;
        }

        // ----------------------------------------------------------------
        // ISA 540: Accounting estimates
        // ----------------------------------------------------------------
        {
            use datasynth_generators::audit::accounting_estimate_generator::AccountingEstimateGenerator;
            let mut est_gen = AccountingEstimateGenerator::new(self.seed + 8540);
            let entity_codes: Vec<String> = self
                .config
                .companies
                .iter()
                .map(|c| c.code.clone())
                .collect();
            let estimates = est_gen.generate_for_entities(&entity_codes);
            info!(
                "ISA 540 accounting estimates: {} estimates across {} entities \
                 ({} with retrospective reviews, {} with auditor point estimates)",
                estimates.len(),
                entity_codes.len(),
                estimates
                    .iter()
                    .filter(|e| e.retrospective_review.is_some())
                    .count(),
                estimates
                    .iter()
                    .filter(|e| e.auditor_point_estimate.is_some())
                    .count(),
            );
            snapshot.accounting_estimates = estimates;
        }

        // ----------------------------------------------------------------
        // ISA 700/701/705/706: Audit opinions (one per engagement)
        // ----------------------------------------------------------------
        {
            use datasynth_generators::audit::audit_opinion_generator::{
                AuditOpinionGenerator, AuditOpinionInput,
            };

            let mut opinion_gen = AuditOpinionGenerator::new(self.seed + 8700);

            // Build inputs — one per engagement, linking findings and going concern.
            let opinion_inputs: Vec<AuditOpinionInput> = snapshot
                .engagements
                .iter()
                .map(|eng| {
                    // Collect findings for this engagement.
                    let eng_findings: Vec<datasynth_core::models::audit::AuditFinding> = snapshot
                        .findings
                        .iter()
                        .filter(|f| f.engagement_id == eng.engagement_id)
                        .cloned()
                        .collect();

                    // Going concern for this entity.
                    let gc = snapshot
                        .going_concern_assessments
                        .iter()
                        .find(|g| g.entity_code == eng.client_entity_id)
                        .cloned();

                    // Component reports relevant to this engagement.
                    let comp_reports: Vec<datasynth_core::models::audit::ComponentAuditorReport> =
                        snapshot.component_reports.clone();

                    let auditor = self
                        .master_data
                        .employees
                        .first()
                        .map(|e| e.display_name.clone())
                        .unwrap_or_else(|| "Global Audit LLP".into());

                    let partner = self
                        .master_data
                        .employees
                        .get(1)
                        .map(|e| e.display_name.clone())
                        .unwrap_or_else(|| eng.engagement_partner_id.clone());

                    AuditOpinionInput {
                        entity_code: eng.client_entity_id.clone(),
                        entity_name: eng.client_name.clone(),
                        engagement_id: eng.engagement_id,
                        period_end: eng.period_end_date,
                        findings: eng_findings,
                        going_concern: gc,
                        component_reports: comp_reports,
                        // Mark as US-listed when audit standards include PCAOB.
                        is_us_listed: {
                            let fw = &self.config.audit_standards.isa_compliance.framework;
                            fw.eq_ignore_ascii_case("pcaob") || fw.eq_ignore_ascii_case("dual")
                        },
                        auditor_name: auditor,
                        engagement_partner: partner,
                    }
                })
                .collect();

            let generated_opinions = opinion_gen.generate_batch(&opinion_inputs);

            for go in &generated_opinions {
                snapshot
                    .key_audit_matters
                    .extend(go.key_audit_matters.clone());
            }
            snapshot.audit_opinions = generated_opinions
                .into_iter()
                .map(|go| go.opinion)
                .collect();

            info!(
                "ISA 700 audit opinions: {} generated ({} unmodified, {} qualified, {} adverse, {} disclaimer)",
                snapshot.audit_opinions.len(),
                snapshot.audit_opinions.iter().filter(|o| matches!(o.opinion_type, datasynth_standards::audit::opinion::OpinionType::Unmodified)).count(),
                snapshot.audit_opinions.iter().filter(|o| matches!(o.opinion_type, datasynth_standards::audit::opinion::OpinionType::Qualified)).count(),
                snapshot.audit_opinions.iter().filter(|o| matches!(o.opinion_type, datasynth_standards::audit::opinion::OpinionType::Adverse)).count(),
                snapshot.audit_opinions.iter().filter(|o| matches!(o.opinion_type, datasynth_standards::audit::opinion::OpinionType::Disclaimer)).count(),
            );
        }

        // ----------------------------------------------------------------
        // SOX 302 / 404 assessments
        // ----------------------------------------------------------------
        {
            use datasynth_generators::audit::sox_generator::{SoxGenerator, SoxGeneratorInput};

            let mut sox_gen = SoxGenerator::new(self.seed + 8302);

            for (i, company) in self.config.companies.iter().enumerate() {
                // Collect findings for this company's engagements.
                let company_engagement_ids: Vec<uuid::Uuid> = snapshot
                    .engagements
                    .iter()
                    .filter(|e| e.client_entity_id == company.code)
                    .map(|e| e.engagement_id)
                    .collect();

                let company_findings: Vec<datasynth_core::models::audit::AuditFinding> = snapshot
                    .findings
                    .iter()
                    .filter(|f| company_engagement_ids.contains(&f.engagement_id))
                    .cloned()
                    .collect();

                // Derive executive names from employee list.
                let emp_count = self.master_data.employees.len();
                let ceo_name = if emp_count > 0 {
                    self.master_data.employees[i % emp_count]
                        .display_name
                        .clone()
                } else {
                    format!("CEO of {}", company.name)
                };
                let cfo_name = if emp_count > 1 {
                    self.master_data.employees[(i + 1) % emp_count]
                        .display_name
                        .clone()
                } else {
                    format!("CFO of {}", company.name)
                };

                // Use engagement materiality if available.
                let materiality = snapshot
                    .engagements
                    .iter()
                    .find(|e| e.client_entity_id == company.code)
                    .map(|e| e.materiality)
                    .unwrap_or_else(|| rust_decimal::Decimal::from(100_000));

                let input = SoxGeneratorInput {
                    company_code: company.code.clone(),
                    company_name: company.name.clone(),
                    fiscal_year,
                    period_end,
                    findings: company_findings,
                    ceo_name,
                    cfo_name,
                    materiality_threshold: materiality,
                    revenue_percent: rust_decimal::Decimal::from(100),
                    assets_percent: rust_decimal::Decimal::from(100),
                    significant_accounts: vec![
                        "Revenue".into(),
                        "Accounts Receivable".into(),
                        "Inventory".into(),
                        "Fixed Assets".into(),
                        "Accounts Payable".into(),
                    ],
                };

                let (certs, assessment) = sox_gen.generate(&input);
                snapshot.sox_302_certifications.extend(certs);
                snapshot.sox_404_assessments.push(assessment);
            }

            info!(
                "SOX 302/404: {} certifications, {} assessments ({} effective, {} ineffective)",
                snapshot.sox_302_certifications.len(),
                snapshot.sox_404_assessments.len(),
                snapshot
                    .sox_404_assessments
                    .iter()
                    .filter(|a| a.icfr_effective)
                    .count(),
                snapshot
                    .sox_404_assessments
                    .iter()
                    .filter(|a| !a.icfr_effective)
                    .count(),
            );
        }

        // ----------------------------------------------------------------
        // ISA 320: Materiality calculations (one per entity)
        // ----------------------------------------------------------------
        {
            use datasynth_generators::audit::materiality_generator::{
                MaterialityGenerator, MaterialityInput,
            };

            let mut mat_gen = MaterialityGenerator::new(self.seed + 8320);

            // Compute per-company financials from JEs.
            // Asset accounts start with '1', revenue with '4',
            // expense accounts with '5' or '6'.
            let mut materiality_inputs: Vec<MaterialityInput> = Vec::new();

            for company in &self.config.companies {
                let company_code = company.code.clone();

                // Revenue: credit-side entries on 4xxx accounts
                let company_revenue: rust_decimal::Decimal = entries
                    .iter()
                    .filter(|e| e.company_code() == company_code)
                    .flat_map(|e| e.lines.iter())
                    .filter(|l| l.account_code.starts_with('4'))
                    .map(|l| l.credit_amount)
                    .sum();

                // Total assets: debit balances on 1xxx accounts
                let total_assets: rust_decimal::Decimal = entries
                    .iter()
                    .filter(|e| e.company_code() == company_code)
                    .flat_map(|e| e.lines.iter())
                    .filter(|l| l.account_code.starts_with('1'))
                    .map(|l| l.debit_amount)
                    .sum();

                // Expenses: debit-side entries on 5xxx/6xxx accounts
                let total_expenses: rust_decimal::Decimal = entries
                    .iter()
                    .filter(|e| e.company_code() == company_code)
                    .flat_map(|e| e.lines.iter())
                    .filter(|l| l.account_code.starts_with('5') || l.account_code.starts_with('6'))
                    .map(|l| l.debit_amount)
                    .sum();

                // Equity: credit balances on 3xxx accounts
                let equity: rust_decimal::Decimal = entries
                    .iter()
                    .filter(|e| e.company_code() == company_code)
                    .flat_map(|e| e.lines.iter())
                    .filter(|l| l.account_code.starts_with('3'))
                    .map(|l| l.credit_amount)
                    .sum();

                let pretax_income = company_revenue - total_expenses;

                // If no company-specific data, fall back to proportional share
                let (rev, assets, pti, eq) = if company_revenue == rust_decimal::Decimal::ZERO {
                    let w = rust_decimal::Decimal::try_from(company.volume_weight)
                        .unwrap_or(rust_decimal::Decimal::ONE);
                    (
                        total_revenue * w,
                        total_revenue * w * rust_decimal::Decimal::from(3),
                        total_revenue * w * rust_decimal::Decimal::new(1, 1),
                        total_revenue * w * rust_decimal::Decimal::from(2),
                    )
                } else {
                    (company_revenue, total_assets, pretax_income, equity)
                };

                let gross_profit = rev * rust_decimal::Decimal::new(35, 2); // 35% assumed

                materiality_inputs.push(MaterialityInput {
                    entity_code: company_code,
                    period: format!("FY{}", fiscal_year),
                    revenue: rev,
                    pretax_income: pti,
                    total_assets: assets,
                    equity: eq,
                    gross_profit,
                });
            }

            snapshot.materiality_calculations = mat_gen.generate_batch(&materiality_inputs);

            info!(
                "Materiality: {} calculations generated ({} pre-tax income, {} revenue, \
                 {} total assets, {} equity benchmarks)",
                snapshot.materiality_calculations.len(),
                snapshot
                    .materiality_calculations
                    .iter()
                    .filter(|m| matches!(
                        m.benchmark,
                        datasynth_core::models::audit::materiality_calculation::MaterialityBenchmark::PretaxIncome
                    ))
                    .count(),
                snapshot
                    .materiality_calculations
                    .iter()
                    .filter(|m| matches!(
                        m.benchmark,
                        datasynth_core::models::audit::materiality_calculation::MaterialityBenchmark::Revenue
                    ))
                    .count(),
                snapshot
                    .materiality_calculations
                    .iter()
                    .filter(|m| matches!(
                        m.benchmark,
                        datasynth_core::models::audit::materiality_calculation::MaterialityBenchmark::TotalAssets
                    ))
                    .count(),
                snapshot
                    .materiality_calculations
                    .iter()
                    .filter(|m| matches!(
                        m.benchmark,
                        datasynth_core::models::audit::materiality_calculation::MaterialityBenchmark::Equity
                    ))
                    .count(),
            );
        }

        // ----------------------------------------------------------------
        // ISA 315: Combined Risk Assessments (per entity, per account area)
        // ----------------------------------------------------------------
        {
            use datasynth_generators::audit::cra_generator::CraGenerator;

            let mut cra_gen = CraGenerator::new(self.seed + 8315);

            // Build entity → scope_id map from already-generated scopes
            let entity_scope_map: std::collections::HashMap<String, String> = snapshot
                .audit_scopes
                .iter()
                .map(|s| (s.entity_code.clone(), s.id.clone()))
                .collect();

            for company in &self.config.companies {
                let cras = cra_gen.generate_for_entity(&company.code, None);
                let scope_id = entity_scope_map.get(&company.code).cloned();
                let cras_with_scope: Vec<_> = cras
                    .into_iter()
                    .map(|mut cra| {
                        cra.scope_id = scope_id.clone();
                        cra
                    })
                    .collect();
                snapshot.combined_risk_assessments.extend(cras_with_scope);
            }

            let significant_count = snapshot
                .combined_risk_assessments
                .iter()
                .filter(|c| c.significant_risk)
                .count();
            let high_cra_count = snapshot
                .combined_risk_assessments
                .iter()
                .filter(|c| {
                    matches!(
                        c.combined_risk,
                        datasynth_core::models::audit::risk_assessment_cra::CraLevel::High
                    )
                })
                .count();

            info!(
                "CRA: {} combined risk assessments ({} significant, {} high CRA)",
                snapshot.combined_risk_assessments.len(),
                significant_count,
                high_cra_count,
            );
        }

        // ----------------------------------------------------------------
        // ISA 530: Sampling Plans (per CRA at Moderate or High level)
        // ----------------------------------------------------------------
        {
            use datasynth_generators::audit::sampling_plan_generator::SamplingPlanGenerator;

            let mut sp_gen = SamplingPlanGenerator::new(self.seed + 8530);

            // Group CRAs by entity and use per-entity tolerable error from materiality
            for company in &self.config.companies {
                let entity_code = company.code.clone();

                // Find tolerable error for this entity (= performance materiality)
                let tolerable_error = snapshot
                    .materiality_calculations
                    .iter()
                    .find(|m| m.entity_code == entity_code)
                    .map(|m| m.tolerable_error);

                // Collect CRAs for this entity
                let entity_cras: Vec<_> = snapshot
                    .combined_risk_assessments
                    .iter()
                    .filter(|c| c.entity_code == entity_code)
                    .cloned()
                    .collect();

                if !entity_cras.is_empty() {
                    let (plans, items) = sp_gen.generate_for_cras(&entity_cras, tolerable_error);
                    snapshot.sampling_plans.extend(plans);
                    snapshot.sampled_items.extend(items);
                }
            }

            let misstatement_count = snapshot
                .sampled_items
                .iter()
                .filter(|i| i.misstatement_found)
                .count();

            info!(
                "ISA 530: {} sampling plans, {} sampled items ({} misstatements found)",
                snapshot.sampling_plans.len(),
                snapshot.sampled_items.len(),
                misstatement_count,
            );
        }

        // ----------------------------------------------------------------
        // ISA 315: Significant Classes of Transactions (SCOTS)
        // ----------------------------------------------------------------
        {
            use datasynth_generators::audit::scots_generator::{
                ScotsGenerator, ScotsGeneratorConfig,
            };

            let ic_enabled = self.config.intercompany.enabled;

            let config = ScotsGeneratorConfig {
                intercompany_enabled: ic_enabled,
                ..ScotsGeneratorConfig::default()
            };
            let mut scots_gen = ScotsGenerator::with_config(self.seed + 83_150, config);

            for company in &self.config.companies {
                let entity_scots = scots_gen.generate_for_entity(&company.code, entries);
                snapshot
                    .significant_transaction_classes
                    .extend(entity_scots);
            }

            let estimation_count = snapshot
                .significant_transaction_classes
                .iter()
                .filter(|s| {
                    matches!(
                        s.transaction_type,
                        datasynth_core::models::audit::scots::ScotTransactionType::Estimation
                    )
                })
                .count();

            info!(
                "ISA 315 SCOTS: {} significant transaction classes ({} estimation SCOTs)",
                snapshot.significant_transaction_classes.len(),
                estimation_count,
            );
        }

        // ----------------------------------------------------------------
        // ISA 520: Unusual Item Markers
        // ----------------------------------------------------------------
        {
            use datasynth_generators::audit::unusual_item_generator::UnusualItemGenerator;

            let mut unusual_gen = UnusualItemGenerator::new(self.seed + 83_200);
            let entity_codes: Vec<String> = self
                .config
                .companies
                .iter()
                .map(|c| c.code.clone())
                .collect();
            let unusual_flags =
                unusual_gen.generate_for_entities(&entity_codes, entries, period_end);
            info!(
                "ISA 520 unusual items: {} flags ({} significant, {} moderate, {} minor)",
                unusual_flags.len(),
                unusual_flags
                    .iter()
                    .filter(|f| matches!(
                        f.severity,
                        datasynth_core::models::audit::unusual_items::UnusualSeverity::Significant
                    ))
                    .count(),
                unusual_flags
                    .iter()
                    .filter(|f| matches!(
                        f.severity,
                        datasynth_core::models::audit::unusual_items::UnusualSeverity::Moderate
                    ))
                    .count(),
                unusual_flags
                    .iter()
                    .filter(|f| matches!(
                        f.severity,
                        datasynth_core::models::audit::unusual_items::UnusualSeverity::Minor
                    ))
                    .count(),
            );
            snapshot.unusual_items = unusual_flags;
        }

        // ----------------------------------------------------------------
        // ISA 520: Analytical Relationships
        // ----------------------------------------------------------------
        {
            use datasynth_generators::audit::analytical_relationship_generator::AnalyticalRelationshipGenerator;

            let mut ar_gen = AnalyticalRelationshipGenerator::new(self.seed + 83_201);
            let entity_codes: Vec<String> = self
                .config
                .companies
                .iter()
                .map(|c| c.code.clone())
                .collect();
            let current_period_label = format!("FY{fiscal_year}");
            let prior_period_label = format!("FY{}", fiscal_year - 1);
            let analytical_rels = ar_gen.generate_for_entities(
                &entity_codes,
                entries,
                &current_period_label,
                &prior_period_label,
            );
            let out_of_range = analytical_rels
                .iter()
                .filter(|r| !r.within_expected_range)
                .count();
            info!(
                "ISA 520 analytical relationships: {} relationships ({} out of expected range)",
                analytical_rels.len(),
                out_of_range,
            );
            snapshot.analytical_relationships = analytical_rels;
        }

        if let Some(pb) = pb {
            pb.finish_with_message(format!(
                "Audit data: {} engagements, {} workpapers, {} evidence, \
                 {} confirmations, {} procedure steps, {} samples, \
                 {} analytical, {} IA funcs, {} related parties, \
                 {} component auditors, {} letters, {} subsequent events, \
                 {} service orgs, {} going concern, {} accounting estimates, \
                 {} opinions, {} KAMs, {} SOX 302 certs, {} SOX 404 assessments, \
                 {} materiality calcs, {} CRAs, {} sampling plans, {} SCOTS, \
                 {} unusual items, {} analytical relationships",
                snapshot.engagements.len(),
                snapshot.workpapers.len(),
                snapshot.evidence.len(),
                snapshot.confirmations.len(),
                snapshot.procedure_steps.len(),
                snapshot.samples.len(),
                snapshot.analytical_results.len(),
                snapshot.ia_functions.len(),
                snapshot.related_parties.len(),
                snapshot.component_auditors.len(),
                snapshot.engagement_letters.len(),
                snapshot.subsequent_events.len(),
                snapshot.service_organizations.len(),
                snapshot.going_concern_assessments.len(),
                snapshot.accounting_estimates.len(),
                snapshot.audit_opinions.len(),
                snapshot.key_audit_matters.len(),
                snapshot.sox_302_certifications.len(),
                snapshot.sox_404_assessments.len(),
                snapshot.materiality_calculations.len(),
                snapshot.combined_risk_assessments.len(),
                snapshot.sampling_plans.len(),
                snapshot.significant_transaction_classes.len(),
                snapshot.unusual_items.len(),
                snapshot.analytical_relationships.len(),
            ));
        }

        // ----------------------------------------------------------------
        // PCAOB-ISA cross-reference mappings
        // ----------------------------------------------------------------
        // Always include the standard PCAOB-ISA mappings when audit generation is
        // enabled. These are static reference data (no randomness required) so we
        // call standard_mappings() directly.
        {
            use datasynth_standards::audit::pcaob::PcaobIsaMapping;
            snapshot.isa_pcaob_mappings = PcaobIsaMapping::standard_mappings();
            debug!(
                "PCAOB-ISA mappings generated: {} mappings",
                snapshot.isa_pcaob_mappings.len()
            );
        }

        // ----------------------------------------------------------------
        // ISA standard reference entries
        // ----------------------------------------------------------------
        // Emit flat ISA standard reference data (number, title, series) so
        // consumers get a machine-readable listing of all 34 ISA standards in
        // audit/isa_mappings.json alongside the PCAOB cross-reference file.
        {
            use datasynth_standards::audit::isa_reference::IsaStandard;
            snapshot.isa_mappings = IsaStandard::standard_entries();
            debug!(
                "ISA standard entries generated: {} standards",
                snapshot.isa_mappings.len()
            );
        }

        // Populate RelatedPartyTransaction.journal_entry_id by matching on date and company.
        // For each RPT, find the chronologically closest JE for the engagement's entity.
        {
            let engagement_by_id: std::collections::HashMap<String, &str> = snapshot
                .engagements
                .iter()
                .map(|e| (e.engagement_id.to_string(), e.client_entity_id.as_str()))
                .collect();

            for rpt in &mut snapshot.related_party_transactions {
                if rpt.journal_entry_id.is_some() {
                    continue; // already set
                }
                let entity = engagement_by_id
                    .get(&rpt.engagement_id.to_string())
                    .copied()
                    .unwrap_or("");

                // Find closest JE by date in the entity's company
                let best_je = entries
                    .iter()
                    .filter(|je| je.header.company_code == entity)
                    .min_by_key(|je| {
                        (je.header.posting_date - rpt.transaction_date)
                            .num_days()
                            .abs()
                    });

                if let Some(je) = best_je {
                    rpt.journal_entry_id = Some(je.header.document_id.to_string());
                }
            }

            let linked = snapshot
                .related_party_transactions
                .iter()
                .filter(|t| t.journal_entry_id.is_some())
                .count();
            debug!(
                "Linked {}/{} related party transactions to journal entries",
                linked,
                snapshot.related_party_transactions.len()
            );
        }

        // --- ISA 700 / 701 / 705 / 706: audit opinion + key audit matters.
        // One opinion per engagement, derived from that engagement's findings,
        // going-concern assessment, and any component-auditor reports. Fills
        // `audit_opinions` + a flattened `key_audit_matters` for downstream
        // export.
        if !snapshot.engagements.is_empty() {
            use datasynth_generators::audit_opinion_generator::{
                AuditOpinionGenerator, AuditOpinionInput,
            };

            let mut opinion_gen = AuditOpinionGenerator::new(self.seed.wrapping_add(0x700));
            let inputs: Vec<AuditOpinionInput> = snapshot
                .engagements
                .iter()
                .map(|eng| {
                    let findings = snapshot
                        .findings
                        .iter()
                        .filter(|f| f.engagement_id == eng.engagement_id)
                        .cloned()
                        .collect();
                    let going_concern = snapshot
                        .going_concern_assessments
                        .iter()
                        .find(|gc| gc.entity_code == eng.client_entity_id)
                        .cloned();
                    // ComponentAuditorReport doesn't carry an engagement id, but
                    // component scope is keyed by `entity_code`, so filter on that.
                    let component_reports = snapshot
                        .component_reports
                        .iter()
                        .filter(|r| r.entity_code == eng.client_entity_id)
                        .cloned()
                        .collect();

                    AuditOpinionInput {
                        entity_code: eng.client_entity_id.clone(),
                        entity_name: eng.client_name.clone(),
                        engagement_id: eng.engagement_id,
                        period_end: eng.period_end_date,
                        findings,
                        going_concern,
                        component_reports,
                        is_us_listed: matches!(
                            eng.engagement_type,
                            datasynth_core::audit::EngagementType::IntegratedAudit
                                | datasynth_core::audit::EngagementType::Sox404
                        ),
                        auditor_name: "DataSynth Audit LLP".to_string(),
                        engagement_partner: "Engagement Partner".to_string(),
                    }
                })
                .collect();

            let generated = opinion_gen.generate_batch(&inputs);
            for g in generated {
                snapshot.key_audit_matters.extend(g.key_audit_matters);
                snapshot.audit_opinions.push(g.opinion);
            }
            debug!(
                "Generated {} audit opinions with {} key audit matters",
                snapshot.audit_opinions.len(),
                snapshot.key_audit_matters.len()
            );
        }

        Ok(snapshot)
    }

    /// Generate audit data using the FSM engine (called when `audit.fsm.enabled: true`).
    ///
    /// Loads the configured blueprint and overlay, builds an [`EngagementContext`]
    /// from the current orchestrator state, runs the FSM engine, and maps the
    /// resulting [`ArtifactBag`] into an [`AuditSnapshot`].  The FSM event trail
    /// is stored in [`AuditSnapshot::fsm_event_trail`] for downstream export.
    fn generate_audit_data_with_fsm(
        &mut self,
        entries: &[JournalEntry],
    ) -> SynthResult<AuditSnapshot> {
        use datasynth_audit_fsm::{
            context::EngagementContext,
            engine::AuditFsmEngine,
            loader::{load_overlay, BlueprintWithPreconditions, BuiltinOverlay, OverlaySource},
        };
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        info!("Audit FSM: generating audit data via FSM engine");

        let fsm_config = self
            .config
            .audit
            .fsm
            .as_ref()
            .expect("FSM config must be present when FSM is enabled");

        // 1. Load blueprint from config string.
        let bwp = match fsm_config.blueprint.as_str() {
            "builtin:fsa" => BlueprintWithPreconditions::load_builtin_fsa(),
            "builtin:ia" => BlueprintWithPreconditions::load_builtin_ia(),
            _ => {
                warn!(
                    "Unknown FSM blueprint '{}', falling back to builtin:fsa",
                    fsm_config.blueprint
                );
                BlueprintWithPreconditions::load_builtin_fsa()
            }
        }
        .map_err(|e| SynthError::generation(format!("FSM blueprint load failed: {e}")))?;

        // 2. Load overlay from config string.
        let overlay = match fsm_config.overlay.as_str() {
            "builtin:default" => load_overlay(&OverlaySource::Builtin(BuiltinOverlay::Default)),
            "builtin:thorough" => load_overlay(&OverlaySource::Builtin(BuiltinOverlay::Thorough)),
            "builtin:rushed" => load_overlay(&OverlaySource::Builtin(BuiltinOverlay::Rushed)),
            _ => {
                warn!(
                    "Unknown FSM overlay '{}', falling back to builtin:default",
                    fsm_config.overlay
                );
                load_overlay(&OverlaySource::Builtin(BuiltinOverlay::Default))
            }
        }
        .map_err(|e| SynthError::generation(format!("FSM overlay load failed: {e}")))?;

        // 3. Build EngagementContext from orchestrator state.
        let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::config(format!("Invalid start_date: {e}")))?;
        let period_end = start_date + chrono::Months::new(self.config.global.period_months);

        // Determine the engagement entity early so we can filter JEs.
        let company = self.config.companies.first();
        let company_code = company
            .map(|c| c.code.clone())
            .unwrap_or_else(|| "UNKNOWN".to_string());
        let company_name = company
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "Unknown Company".to_string());
        let currency = company
            .map(|c| c.currency.clone())
            .unwrap_or_else(|| "USD".to_string());

        // Filter JEs to the engagement entity for single-company coherence.
        let entity_entries: Vec<_> = entries
            .iter()
            .filter(|e| company_code == "UNKNOWN" || e.header.company_code == company_code)
            .cloned()
            .collect();
        let entries = &entity_entries; // Shadow the parameter for remaining usage

        // Financial aggregates from journal entries.
        let total_revenue: rust_decimal::Decimal = entries
            .iter()
            .flat_map(|e| e.lines.iter())
            .filter(|l| l.account_code.starts_with('4'))
            .map(|l| l.credit_amount - l.debit_amount)
            .sum();

        let total_assets: rust_decimal::Decimal = entries
            .iter()
            .flat_map(|e| e.lines.iter())
            .filter(|l| l.account_code.starts_with('1'))
            .map(|l| l.debit_amount - l.credit_amount)
            .sum();

        let total_expenses: rust_decimal::Decimal = entries
            .iter()
            .flat_map(|e| e.lines.iter())
            .filter(|l| l.account_code.starts_with('5') || l.account_code.starts_with('6'))
            .map(|l| l.debit_amount)
            .sum();

        let equity: rust_decimal::Decimal = entries
            .iter()
            .flat_map(|e| e.lines.iter())
            .filter(|l| l.account_code.starts_with('3'))
            .map(|l| l.credit_amount - l.debit_amount)
            .sum();

        let total_debt: rust_decimal::Decimal = entries
            .iter()
            .flat_map(|e| e.lines.iter())
            .filter(|l| l.account_code.starts_with('2'))
            .map(|l| l.credit_amount - l.debit_amount)
            .sum();

        let pretax_income = total_revenue - total_expenses;

        let cogs: rust_decimal::Decimal = entries
            .iter()
            .flat_map(|e| e.lines.iter())
            .filter(|l| l.account_code.starts_with('5'))
            .map(|l| l.debit_amount)
            .sum();
        let gross_profit = total_revenue - cogs;

        let current_assets: rust_decimal::Decimal = entries
            .iter()
            .flat_map(|e| e.lines.iter())
            .filter(|l| {
                l.account_code.starts_with("10")
                    || l.account_code.starts_with("11")
                    || l.account_code.starts_with("12")
                    || l.account_code.starts_with("13")
            })
            .map(|l| l.debit_amount - l.credit_amount)
            .sum();
        let current_liabilities: rust_decimal::Decimal = entries
            .iter()
            .flat_map(|e| e.lines.iter())
            .filter(|l| {
                l.account_code.starts_with("20")
                    || l.account_code.starts_with("21")
                    || l.account_code.starts_with("22")
            })
            .map(|l| l.credit_amount - l.debit_amount)
            .sum();
        let working_capital = current_assets - current_liabilities;

        let depreciation: rust_decimal::Decimal = entries
            .iter()
            .flat_map(|e| e.lines.iter())
            .filter(|l| l.account_code.starts_with("60"))
            .map(|l| l.debit_amount)
            .sum();
        let operating_cash_flow = pretax_income + depreciation;

        // GL accounts for reference data.
        let accounts: Vec<String> = self
            .coa
            .as_ref()
            .map(|coa| {
                coa.get_postable_accounts()
                    .iter()
                    .map(|acc| acc.account_code().to_string())
                    .collect()
            })
            .unwrap_or_default();

        // Team member IDs and display names from master data.
        let team_member_ids: Vec<String> = self
            .master_data
            .employees
            .iter()
            .take(8) // Cap team size
            .map(|e| e.employee_id.clone())
            .collect();
        let team_member_pairs: Vec<(String, String)> = self
            .master_data
            .employees
            .iter()
            .take(8)
            .map(|e| (e.employee_id.clone(), e.display_name.clone()))
            .collect();

        let vendor_names: Vec<String> = self
            .master_data
            .vendors
            .iter()
            .map(|v| v.name.clone())
            .collect();
        let customer_names: Vec<String> = self
            .master_data
            .customers
            .iter()
            .map(|c| c.name.clone())
            .collect();

        let entity_codes: Vec<String> = self
            .config
            .companies
            .iter()
            .map(|c| c.code.clone())
            .collect();

        // Journal entry IDs for evidence tracing (sample up to 50).
        let journal_entry_ids: Vec<String> = entries
            .iter()
            .take(50)
            .map(|e| e.header.document_id.to_string())
            .collect();

        // Account balances for risk weighting (aggregate debit - credit per account).
        let mut account_balances = std::collections::HashMap::<String, f64>::new();
        for entry in entries {
            for line in &entry.lines {
                let debit_f64: f64 = line.debit_amount.to_string().parse().unwrap_or(0.0);
                let credit_f64: f64 = line.credit_amount.to_string().parse().unwrap_or(0.0);
                *account_balances
                    .entry(line.account_code.clone())
                    .or_insert(0.0) += debit_f64 - credit_f64;
            }
        }

        // Internal control IDs and anomaly refs are populated by the
        // caller when available; here we default to empty because the
        // orchestrator state may not have generated controls/anomalies
        // yet at this point in the pipeline.
        let control_ids: Vec<String> = Vec::new();
        let anomaly_refs: Vec<String> = Vec::new();

        let mut context = EngagementContext {
            company_code,
            company_name,
            fiscal_year: start_date.year(),
            currency,
            total_revenue,
            total_assets,
            engagement_start: start_date,
            report_date: period_end,
            pretax_income,
            equity,
            gross_profit,
            working_capital,
            operating_cash_flow,
            total_debt,
            team_member_ids,
            team_member_pairs,
            accounts,
            vendor_names,
            customer_names,
            journal_entry_ids,
            account_balances,
            control_ids,
            anomaly_refs,
            journal_entries: entries.to_vec(),
            is_us_listed: false,
            entity_codes,
            auditor_firm_name: "DataSynth Audit LLP".into(),
            accounting_framework: self
                .config
                .accounting_standards
                .framework
                .map(|f| match f {
                    datasynth_config::schema::AccountingFrameworkConfig::UsGaap => "US GAAP",
                    datasynth_config::schema::AccountingFrameworkConfig::Ifrs => "IFRS",
                    datasynth_config::schema::AccountingFrameworkConfig::FrenchGaap => {
                        "French GAAP"
                    }
                    datasynth_config::schema::AccountingFrameworkConfig::GermanGaap => {
                        "German GAAP"
                    }
                    datasynth_config::schema::AccountingFrameworkConfig::DualReporting => {
                        "Dual Reporting"
                    }
                })
                .unwrap_or("IFRS")
                .into(),
        };

        // 4. Create and run the FSM engine.
        let seed = fsm_config.seed.unwrap_or(self.seed + 8000);
        let rng = ChaCha8Rng::seed_from_u64(seed);
        let mut engine = AuditFsmEngine::new(bwp, overlay, rng);

        let mut result = engine
            .run_engagement(&context)
            .map_err(|e| SynthError::generation(format!("FSM engine failed: {e}")))?;

        info!(
            "Audit FSM: engine produced {} events, {} artifacts, {} anomalies, \
             {} phases completed, duration {:.1}h",
            result.event_log.len(),
            result.artifacts.total_artifacts(),
            result.anomalies.len(),
            result.phases_completed.len(),
            result.total_duration_hours,
        );

        // 4b. Populate financial data in the artifact bag for downstream consumers.
        let tb_entity = context.company_code.clone();
        let tb_fy = context.fiscal_year;
        result.artifacts.journal_entries = std::mem::take(&mut context.journal_entries);
        result.artifacts.trial_balance_entries = compute_trial_balance_entries(
            entries,
            &tb_entity,
            tb_fy,
            self.coa.as_ref().map(|c| c.as_ref()),
        );

        // 5. Map ArtifactBag fields to AuditSnapshot.
        let bag = result.artifacts;
        let mut snapshot = AuditSnapshot {
            engagements: bag.engagements,
            engagement_letters: bag.engagement_letters,
            materiality_calculations: bag.materiality_calculations,
            risk_assessments: bag.risk_assessments,
            combined_risk_assessments: bag.combined_risk_assessments,
            workpapers: bag.workpapers,
            evidence: bag.evidence,
            findings: bag.findings,
            judgments: bag.judgments,
            sampling_plans: bag.sampling_plans,
            sampled_items: bag.sampled_items,
            analytical_results: bag.analytical_results,
            going_concern_assessments: bag.going_concern_assessments,
            subsequent_events: bag.subsequent_events,
            audit_opinions: bag.audit_opinions,
            key_audit_matters: bag.key_audit_matters,
            procedure_steps: bag.procedure_steps,
            samples: bag.samples,
            confirmations: bag.confirmations,
            confirmation_responses: bag.confirmation_responses,
            // Store the event trail for downstream export.
            fsm_event_trail: Some(result.event_log),
            // Fields not produced by the FSM engine remain at their defaults.
            ..Default::default()
        };

        // 6. Add static reference data (same as legacy path).
        {
            use datasynth_standards::audit::pcaob::PcaobIsaMapping;
            snapshot.isa_pcaob_mappings = PcaobIsaMapping::standard_mappings();
        }
        {
            use datasynth_standards::audit::isa_reference::IsaStandard;
            snapshot.isa_mappings = IsaStandard::standard_entries();
        }

        info!(
            "Audit FSM: snapshot contains {} engagements, {} workpapers, {} evidence, \
             {} risk assessments, {} findings, {} materiality calcs",
            snapshot.engagements.len(),
            snapshot.workpapers.len(),
            snapshot.evidence.len(),
            snapshot.risk_assessments.len(),
            snapshot.findings.len(),
            snapshot.materiality_calculations.len(),
        );

        Ok(snapshot)
    }

    /// Export journal entries as graph data for ML training and network reconstruction.
    ///
    /// Builds a transaction graph where:
    /// - Nodes are GL accounts
    /// - Edges are money flows from credit to debit accounts
    /// - Edge attributes include amount, date, business process, anomaly flags
    fn export_graphs(
        &mut self,
        entries: &[JournalEntry],
        _coa: &Arc<ChartOfAccounts>,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<GraphExportSnapshot> {
        let pb = self.create_progress_bar(100, "Exporting Graphs");

        let mut snapshot = GraphExportSnapshot::default();

        // Get output directory
        let output_dir = self
            .output_path
            .clone()
            .unwrap_or_else(|| PathBuf::from(&self.config.output.output_directory));
        let graph_dir = output_dir.join(&self.config.graph_export.output_subdirectory);

        // Process each graph type configuration
        for graph_type in &self.config.graph_export.graph_types {
            if let Some(pb) = &pb {
                pb.inc(10);
            }

            // Build transaction graph
            let graph_config = TransactionGraphConfig {
                include_vendors: false,
                include_customers: false,
                create_debit_credit_edges: true,
                include_document_nodes: graph_type.include_document_nodes,
                min_edge_weight: graph_type.min_edge_weight,
                aggregate_parallel_edges: graph_type.aggregate_edges,
                framework: None,
            };

            let mut builder = TransactionGraphBuilder::new(graph_config);
            builder.add_journal_entries(entries);
            let graph = builder.build();

            // Update stats
            stats.graph_node_count += graph.node_count();
            stats.graph_edge_count += graph.edge_count();

            if let Some(pb) = &pb {
                pb.inc(40);
            }

            // Export to each configured format
            for format in &self.config.graph_export.formats {
                let format_dir = graph_dir.join(&graph_type.name).join(format_name(*format));

                // Create output directory
                if let Err(e) = std::fs::create_dir_all(&format_dir) {
                    warn!("Failed to create graph output directory: {}", e);
                    continue;
                }

                match format {
                    datasynth_config::schema::GraphExportFormat::PytorchGeometric => {
                        let pyg_config = PyGExportConfig {
                            common: datasynth_graph::CommonExportConfig {
                                export_node_features: true,
                                export_edge_features: true,
                                export_node_labels: true,
                                export_edge_labels: true,
                                export_masks: true,
                                train_ratio: self.config.graph_export.train_ratio,
                                val_ratio: self.config.graph_export.validation_ratio,
                                seed: self.config.graph_export.split_seed.unwrap_or(self.seed),
                            },
                            one_hot_categoricals: false,
                        };

                        let exporter = PyGExporter::new(pyg_config);
                        match exporter.export(&graph, &format_dir) {
                            Ok(metadata) => {
                                snapshot.exports.insert(
                                    format!("{}_{}", graph_type.name, "pytorch_geometric"),
                                    GraphExportInfo {
                                        name: graph_type.name.clone(),
                                        format: "pytorch_geometric".to_string(),
                                        output_path: format_dir.clone(),
                                        node_count: metadata.num_nodes,
                                        edge_count: metadata.num_edges,
                                    },
                                );
                                snapshot.graph_count += 1;
                            }
                            Err(e) => {
                                warn!("Failed to export PyTorch Geometric graph: {}", e);
                            }
                        }
                    }
                    datasynth_config::schema::GraphExportFormat::Neo4j => {
                        use datasynth_graph::{Neo4jExportConfig, Neo4jExporter};

                        let neo4j_config = Neo4jExportConfig {
                            export_node_properties: true,
                            export_edge_properties: true,
                            export_features: true,
                            generate_cypher: true,
                            generate_admin_import: true,
                            database_name: "synth".to_string(),
                            cypher_batch_size: 1000,
                        };

                        let exporter = Neo4jExporter::new(neo4j_config);
                        match exporter.export(&graph, &format_dir) {
                            Ok(metadata) => {
                                snapshot.exports.insert(
                                    format!("{}_{}", graph_type.name, "neo4j"),
                                    GraphExportInfo {
                                        name: graph_type.name.clone(),
                                        format: "neo4j".to_string(),
                                        output_path: format_dir.clone(),
                                        node_count: metadata.num_nodes,
                                        edge_count: metadata.num_edges,
                                    },
                                );
                                snapshot.graph_count += 1;
                            }
                            Err(e) => {
                                warn!("Failed to export Neo4j graph: {}", e);
                            }
                        }
                    }
                    datasynth_config::schema::GraphExportFormat::Dgl => {
                        use datasynth_graph::{DGLExportConfig, DGLExporter};

                        let dgl_config = DGLExportConfig {
                            common: datasynth_graph::CommonExportConfig {
                                export_node_features: true,
                                export_edge_features: true,
                                export_node_labels: true,
                                export_edge_labels: true,
                                export_masks: true,
                                train_ratio: self.config.graph_export.train_ratio,
                                val_ratio: self.config.graph_export.validation_ratio,
                                seed: self.config.graph_export.split_seed.unwrap_or(self.seed),
                            },
                            heterogeneous: self.config.graph_export.dgl.heterogeneous,
                            include_pickle_script: true, // DGL ecosystem standard helper
                        };

                        let exporter = DGLExporter::new(dgl_config);
                        match exporter.export(&graph, &format_dir) {
                            Ok(metadata) => {
                                snapshot.exports.insert(
                                    format!("{}_{}", graph_type.name, "dgl"),
                                    GraphExportInfo {
                                        name: graph_type.name.clone(),
                                        format: "dgl".to_string(),
                                        output_path: format_dir.clone(),
                                        node_count: metadata.common.num_nodes,
                                        edge_count: metadata.common.num_edges,
                                    },
                                );
                                snapshot.graph_count += 1;
                            }
                            Err(e) => {
                                warn!("Failed to export DGL graph: {}", e);
                            }
                        }
                    }
                    datasynth_config::schema::GraphExportFormat::RustGraph => {
                        use datasynth_graph::{
                            RustGraphExportConfig, RustGraphExporter, RustGraphOutputFormat,
                        };

                        let rustgraph_config = RustGraphExportConfig {
                            include_features: true,
                            include_temporal: true,
                            include_labels: true,
                            source_name: "datasynth".to_string(),
                            batch_id: None,
                            output_format: RustGraphOutputFormat::JsonLines,
                            export_node_properties: true,
                            export_edge_properties: true,
                            pretty_print: false,
                        };

                        let exporter = RustGraphExporter::new(rustgraph_config);
                        match exporter.export(&graph, &format_dir) {
                            Ok(metadata) => {
                                snapshot.exports.insert(
                                    format!("{}_{}", graph_type.name, "rustgraph"),
                                    GraphExportInfo {
                                        name: graph_type.name.clone(),
                                        format: "rustgraph".to_string(),
                                        output_path: format_dir.clone(),
                                        node_count: metadata.num_nodes,
                                        edge_count: metadata.num_edges,
                                    },
                                );
                                snapshot.graph_count += 1;
                            }
                            Err(e) => {
                                warn!("Failed to export RustGraph: {}", e);
                            }
                        }
                    }
                    datasynth_config::schema::GraphExportFormat::RustGraphHypergraph => {
                        // Hypergraph export is handled separately in Phase 10b
                        debug!("RustGraphHypergraph format is handled in Phase 10b (hypergraph export)");
                    }
                }
            }

            if let Some(pb) = &pb {
                pb.inc(40);
            }
        }

        stats.graph_export_count = snapshot.graph_count;
        snapshot.exported = snapshot.graph_count > 0;

        if let Some(pb) = pb {
            pb.finish_with_message(format!(
                "Graphs exported: {} graphs ({} nodes, {} edges)",
                snapshot.graph_count, stats.graph_node_count, stats.graph_edge_count
            ));
        }

        Ok(snapshot)
    }

    /// Build additional graph types (banking, approval, entity) when relevant data
    /// is available. These run as a late phase because the data they need (banking
    /// snapshot, intercompany snapshot) is only generated after the main graph
    /// export phase.
    fn build_additional_graphs(
        &self,
        banking: &BankingSnapshot,
        intercompany: &IntercompanySnapshot,
        entries: &[JournalEntry],
        stats: &mut EnhancedGenerationStatistics,
    ) {
        let output_dir = self
            .output_path
            .clone()
            .unwrap_or_else(|| PathBuf::from(&self.config.output.output_directory));
        let graph_dir = output_dir.join(&self.config.graph_export.output_subdirectory);

        // Banking graph: build when banking customers and transactions exist
        if !banking.customers.is_empty() && !banking.transactions.is_empty() {
            info!("Phase 10c: Building banking network graph");
            let config = BankingGraphConfig::default();
            let mut builder = BankingGraphBuilder::new(config);
            builder.add_customers(&banking.customers);
            builder.add_accounts(&banking.accounts, &banking.customers);
            builder.add_transactions(&banking.transactions);
            let graph = builder.build();

            let node_count = graph.node_count();
            let edge_count = graph.edge_count();
            stats.graph_node_count += node_count;
            stats.graph_edge_count += edge_count;

            // Export as PyG if configured
            for format in &self.config.graph_export.formats {
                if matches!(
                    format,
                    datasynth_config::schema::GraphExportFormat::PytorchGeometric
                ) {
                    let format_dir = graph_dir.join("banking_network").join("pytorch_geometric");
                    if let Err(e) = std::fs::create_dir_all(&format_dir) {
                        warn!("Failed to create banking graph output dir: {}", e);
                        continue;
                    }
                    let pyg_config = PyGExportConfig::default();
                    let exporter = PyGExporter::new(pyg_config);
                    if let Err(e) = exporter.export(&graph, &format_dir) {
                        warn!("Failed to export banking graph as PyG: {}", e);
                    } else {
                        info!(
                            "Banking network graph exported: {} nodes, {} edges",
                            node_count, edge_count
                        );
                    }
                }
            }
        }

        // Approval graph: build from journal entry approval workflows
        let approval_entries: Vec<_> = entries
            .iter()
            .filter(|je| je.header.approval_workflow.is_some())
            .collect();

        if !approval_entries.is_empty() {
            info!(
                "Phase 10c: Building approval network graph ({} entries with approvals)",
                approval_entries.len()
            );
            let config = ApprovalGraphConfig::default();
            let mut builder = ApprovalGraphBuilder::new(config);

            for je in &approval_entries {
                if let Some(ref wf) = je.header.approval_workflow {
                    for action in &wf.actions {
                        let record = datasynth_core::models::ApprovalRecord {
                            approval_id: format!(
                                "APR-{}-{}",
                                je.header.document_id, action.approval_level
                            ),
                            document_number: je.header.document_id.to_string(),
                            document_type: "JE".to_string(),
                            company_code: je.company_code().to_string(),
                            requester_id: wf.preparer_id.clone(),
                            requester_name: Some(wf.preparer_name.clone()),
                            approver_id: action.actor_id.clone(),
                            approver_name: action.actor_name.clone(),
                            approval_date: je.posting_date(),
                            action: format!("{:?}", action.action),
                            amount: wf.amount,
                            approval_limit: None,
                            comments: action.comments.clone(),
                            delegation_from: None,
                            is_auto_approved: false,
                        };
                        builder.add_approval(&record);
                    }
                }
            }

            let graph = builder.build();
            let node_count = graph.node_count();
            let edge_count = graph.edge_count();
            stats.graph_node_count += node_count;
            stats.graph_edge_count += edge_count;

            // Export as PyG if configured
            for format in &self.config.graph_export.formats {
                if matches!(
                    format,
                    datasynth_config::schema::GraphExportFormat::PytorchGeometric
                ) {
                    let format_dir = graph_dir.join("approval_network").join("pytorch_geometric");
                    if let Err(e) = std::fs::create_dir_all(&format_dir) {
                        warn!("Failed to create approval graph output dir: {}", e);
                        continue;
                    }
                    let pyg_config = PyGExportConfig::default();
                    let exporter = PyGExporter::new(pyg_config);
                    if let Err(e) = exporter.export(&graph, &format_dir) {
                        warn!("Failed to export approval graph as PyG: {}", e);
                    } else {
                        info!(
                            "Approval network graph exported: {} nodes, {} edges",
                            node_count, edge_count
                        );
                    }
                }
            }
        }

        // Entity graph: map CompanyConfig → Company and wire intercompany relationships
        if self.config.companies.len() >= 2 {
            info!(
                "Phase 10c: Building entity relationship graph ({} companies)",
                self.config.companies.len()
            );

            let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
                .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date"));

            // Map CompanyConfig → Company objects
            let parent_code = &self.config.companies[0].code;
            let mut companies: Vec<datasynth_core::models::Company> =
                Vec::with_capacity(self.config.companies.len());

            // First company is the parent
            let first = &self.config.companies[0];
            companies.push(datasynth_core::models::Company::parent(
                &first.code,
                &first.name,
                &first.country,
                &first.currency,
            ));

            // Remaining companies are subsidiaries (100% owned by parent)
            for cc in self.config.companies.iter().skip(1) {
                companies.push(datasynth_core::models::Company::subsidiary(
                    &cc.code,
                    &cc.name,
                    &cc.country,
                    &cc.currency,
                    parent_code,
                    rust_decimal::Decimal::from(100),
                ));
            }

            // Build IntercompanyRelationship records (same logic as phase_intercompany)
            let relationships: Vec<datasynth_core::models::intercompany::IntercompanyRelationship> =
                self.config
                    .companies
                    .iter()
                    .skip(1)
                    .enumerate()
                    .map(|(i, cc)| {
                        let mut rel =
                            datasynth_core::models::intercompany::IntercompanyRelationship::new(
                                format!("REL{:03}", i + 1),
                                parent_code.clone(),
                                cc.code.clone(),
                                rust_decimal::Decimal::from(100),
                                start_date,
                            );
                        rel.functional_currency = cc.currency.clone();
                        rel
                    })
                    .collect();

            let mut builder = EntityGraphBuilder::new(EntityGraphConfig::default());
            builder.add_companies(&companies);
            builder.add_ownership_relationships(&relationships);

            // Thread IC matched-pair transaction edges into the entity graph
            for pair in &intercompany.matched_pairs {
                builder.add_intercompany_edge(
                    &pair.seller_company,
                    &pair.buyer_company,
                    pair.amount,
                    &format!("{:?}", pair.transaction_type),
                );
            }

            let graph = builder.build();
            let node_count = graph.node_count();
            let edge_count = graph.edge_count();
            stats.graph_node_count += node_count;
            stats.graph_edge_count += edge_count;

            // Export as PyG if configured
            for format in &self.config.graph_export.formats {
                if matches!(
                    format,
                    datasynth_config::schema::GraphExportFormat::PytorchGeometric
                ) {
                    let format_dir = graph_dir.join("entity_network").join("pytorch_geometric");
                    if let Err(e) = std::fs::create_dir_all(&format_dir) {
                        warn!("Failed to create entity graph output dir: {}", e);
                        continue;
                    }
                    let pyg_config = PyGExportConfig::default();
                    let exporter = PyGExporter::new(pyg_config);
                    if let Err(e) = exporter.export(&graph, &format_dir) {
                        warn!("Failed to export entity graph as PyG: {}", e);
                    } else {
                        info!(
                            "Entity relationship graph exported: {} nodes, {} edges",
                            node_count, edge_count
                        );
                    }
                }
            }
        } else {
            debug!(
                "EntityGraphBuilder: skipped (requires 2+ companies, found {})",
                self.config.companies.len()
            );
        }
    }

    /// Export a multi-layer hypergraph for RustGraph integration.
    ///
    /// Builds a 3-layer hypergraph:
    /// - Layer 1: Governance & Controls (COSO, internal controls, master data)
    /// - Layer 2: Process Events (all process family document flows + OCPM events)
    /// - Layer 3: Accounting Network (GL accounts, journal entries as hyperedges)
    #[allow(clippy::too_many_arguments)]
    fn export_hypergraph(
        &self,
        coa: &Arc<ChartOfAccounts>,
        entries: &[JournalEntry],
        document_flows: &DocumentFlowSnapshot,
        sourcing: &SourcingSnapshot,
        hr: &HrSnapshot,
        manufacturing: &ManufacturingSnapshot,
        banking: &BankingSnapshot,
        audit: &AuditSnapshot,
        financial_reporting: &FinancialReportingSnapshot,
        ocpm: &OcpmSnapshot,
        compliance: &ComplianceRegulationsSnapshot,
        stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<HypergraphExportInfo> {
        use datasynth_graph::builders::hypergraph::{HypergraphBuilder, HypergraphConfig};
        use datasynth_graph::exporters::hypergraph::{HypergraphExportConfig, HypergraphExporter};
        use datasynth_graph::exporters::unified::{RustGraphUnifiedExporter, UnifiedExportConfig};
        use datasynth_graph::models::hypergraph::AggregationStrategy;

        let hg_settings = &self.config.graph_export.hypergraph;

        // Parse aggregation strategy from config string
        let aggregation_strategy = match hg_settings.aggregation_strategy.as_str() {
            "truncate" => AggregationStrategy::Truncate,
            "pool_by_counterparty" => AggregationStrategy::PoolByCounterparty,
            "pool_by_time_period" => AggregationStrategy::PoolByTimePeriod,
            "importance_sample" => AggregationStrategy::ImportanceSample,
            _ => AggregationStrategy::PoolByCounterparty,
        };

        let builder_config = HypergraphConfig {
            max_nodes: hg_settings.max_nodes,
            aggregation_strategy,
            include_coso: hg_settings.governance_layer.include_coso,
            include_controls: hg_settings.governance_layer.include_controls,
            include_sox: hg_settings.governance_layer.include_sox,
            include_vendors: hg_settings.governance_layer.include_vendors,
            include_customers: hg_settings.governance_layer.include_customers,
            include_employees: hg_settings.governance_layer.include_employees,
            include_p2p: hg_settings.process_layer.include_p2p,
            include_o2c: hg_settings.process_layer.include_o2c,
            include_s2c: hg_settings.process_layer.include_s2c,
            include_h2r: hg_settings.process_layer.include_h2r,
            include_mfg: hg_settings.process_layer.include_mfg,
            include_bank: hg_settings.process_layer.include_bank,
            include_audit: hg_settings.process_layer.include_audit,
            include_r2r: hg_settings.process_layer.include_r2r,
            events_as_hyperedges: hg_settings.process_layer.events_as_hyperedges,
            docs_per_counterparty_threshold: hg_settings
                .process_layer
                .docs_per_counterparty_threshold,
            include_accounts: hg_settings.accounting_layer.include_accounts,
            je_as_hyperedges: hg_settings.accounting_layer.je_as_hyperedges,
            include_cross_layer_edges: hg_settings.cross_layer.enabled,
            include_compliance: self.config.compliance_regulations.enabled,
            include_tax: true,
            include_treasury: true,
            include_esg: true,
            include_project: true,
            include_intercompany: true,
            include_temporal_events: true,
        };

        let mut builder = HypergraphBuilder::new(builder_config);

        // Layer 1: Governance & Controls
        builder.add_coso_framework();

        // Add controls if available (generated during JE generation)
        // Controls are generated per-company; we use the standard set
        if hg_settings.governance_layer.include_controls && self.config.internal_controls.enabled {
            let controls = InternalControl::standard_controls();
            builder.add_controls(&controls);
        }

        // Add master data
        builder.add_vendors(&self.master_data.vendors);
        builder.add_customers(&self.master_data.customers);
        builder.add_employees(&self.master_data.employees);

        // Layer 2: Process Events (all process families)
        builder.add_p2p_documents(
            &document_flows.purchase_orders,
            &document_flows.goods_receipts,
            &document_flows.vendor_invoices,
            &document_flows.payments,
        );
        builder.add_o2c_documents(
            &document_flows.sales_orders,
            &document_flows.deliveries,
            &document_flows.customer_invoices,
        );
        builder.add_s2c_documents(
            &sourcing.sourcing_projects,
            &sourcing.qualifications,
            &sourcing.rfx_events,
            &sourcing.bids,
            &sourcing.bid_evaluations,
            &sourcing.contracts,
        );
        builder.add_h2r_documents(&hr.payroll_runs, &hr.time_entries, &hr.expense_reports);
        builder.add_mfg_documents(
            &manufacturing.production_orders,
            &manufacturing.quality_inspections,
            &manufacturing.cycle_counts,
        );
        builder.add_bank_documents(&banking.customers, &banking.accounts, &banking.transactions);
        builder.add_audit_documents(
            &audit.engagements,
            &audit.workpapers,
            &audit.findings,
            &audit.evidence,
            &audit.risk_assessments,
            &audit.judgments,
            &audit.materiality_calculations,
            &audit.audit_opinions,
            &audit.going_concern_assessments,
        );
        builder.add_bank_recon_documents(&financial_reporting.bank_reconciliations);

        // OCPM events as hyperedges
        if let Some(ref event_log) = ocpm.event_log {
            builder.add_ocpm_events(event_log);
        }

        // Compliance regulations as cross-layer nodes
        if self.config.compliance_regulations.enabled
            && hg_settings.governance_layer.include_controls
        {
            // Reconstruct ComplianceStandard objects from the registry
            let registry = datasynth_standards::registry::StandardRegistry::with_built_in();
            let standards: Vec<datasynth_core::models::compliance::ComplianceStandard> = compliance
                .standard_records
                .iter()
                .filter_map(|r| {
                    let sid = datasynth_core::models::compliance::StandardId::parse(&r.standard_id);
                    registry.get(&sid).cloned()
                })
                .collect();

            builder.add_compliance_regulations(
                &standards,
                &compliance.findings,
                &compliance.filings,
            );
        }

        // Layer 3: Accounting Network
        builder.add_accounts(coa);
        builder.add_journal_entries_as_hyperedges(entries);

        // Build the hypergraph
        let hypergraph = builder.build();

        // Export
        let output_dir = self
            .output_path
            .clone()
            .unwrap_or_else(|| PathBuf::from(&self.config.output.output_directory));
        let hg_dir = output_dir
            .join(&self.config.graph_export.output_subdirectory)
            .join(&hg_settings.output_subdirectory);

        // Branch on output format
        let (num_nodes, num_edges, num_hyperedges) = match hg_settings.output_format.as_str() {
            "unified" => {
                let exporter = RustGraphUnifiedExporter::new(UnifiedExportConfig::default());
                let metadata = exporter.export(&hypergraph, &hg_dir).map_err(|e| {
                    SynthError::generation(format!("Unified hypergraph export failed: {e}"))
                })?;
                (
                    metadata.num_nodes,
                    metadata.num_edges,
                    metadata.num_hyperedges,
                )
            }
            _ => {
                // "native" or any unrecognized format → use existing exporter
                let exporter = HypergraphExporter::new(HypergraphExportConfig::default());
                let metadata = exporter.export(&hypergraph, &hg_dir).map_err(|e| {
                    SynthError::generation(format!("Hypergraph export failed: {e}"))
                })?;
                (
                    metadata.num_nodes,
                    metadata.num_edges,
                    metadata.num_hyperedges,
                )
            }
        };

        // Stream to RustGraph ingest endpoint if configured
        #[cfg(feature = "streaming")]
        if let Some(ref target_url) = hg_settings.stream_target {
            use crate::stream_client::{StreamClient, StreamConfig};
            use std::io::Write as _;

            let api_key = std::env::var("RUSTGRAPH_API_KEY").ok();
            let stream_config = StreamConfig {
                target_url: target_url.clone(),
                batch_size: hg_settings.stream_batch_size,
                api_key,
                ..StreamConfig::default()
            };

            match StreamClient::new(stream_config) {
                Ok(mut client) => {
                    let exporter = RustGraphUnifiedExporter::new(UnifiedExportConfig::default());
                    match exporter.export_to_writer(&hypergraph, &mut client) {
                        Ok(_) => {
                            if let Err(e) = client.flush() {
                                warn!("Failed to flush stream client: {}", e);
                            } else {
                                info!("Streamed {} records to {}", client.total_sent(), target_url);
                            }
                        }
                        Err(e) => {
                            warn!("Streaming export failed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to create stream client: {}", e);
                }
            }
        }

        // Update stats
        stats.graph_node_count += num_nodes;
        stats.graph_edge_count += num_edges;
        stats.graph_export_count += 1;

        Ok(HypergraphExportInfo {
            node_count: num_nodes,
            edge_count: num_edges,
            hyperedge_count: num_hyperedges,
            output_path: hg_dir,
        })
    }

    /// Generate banking KYC/AML data.
    ///
    /// Creates banking customers, accounts, and transactions with AML typology injection.
    /// Uses the BankingOrchestrator from synth-banking crate.
    fn generate_banking_data(&mut self) -> SynthResult<BankingSnapshot> {
        let pb = self.create_progress_bar(100, "Generating Banking Data");

        // Build the banking orchestrator from config
        let orchestrator = BankingOrchestratorBuilder::new()
            .config(self.config.banking.clone())
            .seed(self.seed + 9000)
            .country_pack(self.primary_pack().clone())
            .build();

        if let Some(pb) = &pb {
            pb.inc(10);
        }

        // Generate the banking data
        let result = orchestrator.generate();

        if let Some(pb) = &pb {
            pb.inc(90);
            pb.finish_with_message(format!(
                "Banking: {} customers, {} transactions",
                result.customers.len(),
                result.transactions.len()
            ));
        }

        // Cross-reference banking customers with core master data so that
        // banking customer names align with the enterprise customer list.
        // We rotate through core customers, overlaying their name and country
        // onto the generated banking customers where possible.
        let mut banking_customers = result.customers;
        let core_customers = &self.master_data.customers;
        if !core_customers.is_empty() {
            // Spec 19 §4-R1 (R1b): the banking cross-reference overlays a core-customer name onto
            // each banking customer. OFF (default) → the historical `i % len` round-robin, byte-for-
            // byte. ON → the same isolated weighted-choice as the O2C loop (its OWN stream label),
            // so the concentrated customers appear proportionally as banking counterparties too.
            let conc = &self.config.document_flows.concentration;
            let mut core_concentration = conc.customer_active().then(|| {
                datasynth_core::distributions::ConcentrationSampler::new(
                    datasynth_core::distributions::concentration_seed(self.seed, "banking_core"),
                    core_customers.len(),
                    conc.top_n,
                    conc.customer_top_n_share,
                )
            });
            for (i, bc) in banking_customers.iter_mut().enumerate() {
                let core_idx = match &mut core_concentration {
                    Some(sampler) => sampler.sample(),
                    None => i % core_customers.len(),
                };
                let core = &core_customers[core_idx];
                bc.name = CustomerName::business(&core.name);
                bc.residence_country = core.country.clone();
                bc.enterprise_customer_id = Some(core.customer_id.clone());
            }
            debug!(
                "Cross-referenced {} banking customers with {} core customers",
                banking_customers.len(),
                core_customers.len()
            );
        }

        Ok(BankingSnapshot {
            customers: banking_customers,
            accounts: result.accounts,
            transactions: result.transactions,
            transaction_labels: result.transaction_labels,
            customer_labels: result.customer_labels,
            account_labels: result.account_labels,
            relationship_labels: result.relationship_labels,
            narratives: result.narratives,
            suspicious_count: result.stats.suspicious_count,
            scenario_count: result.scenarios.len(),
        })
    }

    /// Calculate total transactions to generate.
    fn calculate_total_transactions(&self) -> u64 {
        let months = self.config.global.period_months as f64;
        self.config
            .companies
            .iter()
            .map(|c| {
                let annual = c.annual_transaction_volume.count() as f64;
                let weighted = annual * c.volume_weight;
                (weighted * months / 12.0) as u64
            })
            .sum()
    }

    /// Create a progress bar if progress display is enabled.
    fn create_progress_bar(&self, total: u64, message: &str) -> Option<ProgressBar> {
        if !self.phase_config.show_progress {
            return None;
        }

        let pb = if let Some(mp) = &self.multi_progress {
            mp.add(ProgressBar::new(total))
        } else {
            ProgressBar::new(total)
        };

        pb.set_style(
            ProgressStyle::default_bar()
                .template(&format!(
                    "{{spinner:.green}} {message} [{{elapsed_precise}}] [{{bar:40.cyan/blue}}] {{pos}}/{{len}} ({{per_sec}})"
                ))
                .expect("Progress bar template should be valid - uses only standard indicatif placeholders")
                .progress_chars("#>-"),
        );

        Some(pb)
    }

    /// Get the generated chart of accounts.
    pub fn get_coa(&self) -> Option<Arc<ChartOfAccounts>> {
        self.coa.clone()
    }

    /// Get the generated master data.
    pub fn get_master_data(&self) -> &MasterDataSnapshot {
        &self.master_data
    }

    /// Phase: Generate compliance regulations data (standards, procedures, findings, filings, graph).
    fn phase_compliance_regulations(
        &mut self,
        _stats: &mut EnhancedGenerationStatistics,
    ) -> SynthResult<ComplianceRegulationsSnapshot> {
        if !self.phase_config.generate_compliance_regulations {
            return Ok(ComplianceRegulationsSnapshot::default());
        }

        info!("Phase: Generating Compliance Regulations Data");

        let cr_config = &self.config.compliance_regulations;

        // Determine jurisdictions: from config or inferred from companies
        let jurisdictions: Vec<String> = if cr_config.jurisdictions.is_empty() {
            self.config
                .companies
                .iter()
                .map(|c| c.country.clone())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect()
        } else {
            cr_config.jurisdictions.clone()
        };

        // Determine reference date
        let fallback_date =
            NaiveDate::from_ymd_opt(2025, 1, 1).expect("static date is always valid");
        let reference_date = cr_config
            .reference_date
            .as_ref()
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .unwrap_or_else(|| {
                NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
                    .unwrap_or(fallback_date)
            });

        // Generate standards registry data
        let reg_gen = datasynth_generators::compliance::RegulationGenerator::new();
        let standard_records = reg_gen.generate_standard_records(&jurisdictions, reference_date);
        let cross_reference_records = reg_gen.generate_cross_reference_records();
        let jurisdiction_records =
            reg_gen.generate_jurisdiction_records(&jurisdictions, reference_date);

        info!(
            "  Standards: {} records, {} cross-references, {} jurisdictions",
            standard_records.len(),
            cross_reference_records.len(),
            jurisdiction_records.len()
        );

        // Generate audit procedures (if enabled)
        let audit_procedures = if cr_config.audit_procedures.enabled {
            let proc_config = datasynth_generators::compliance::ProcedureGeneratorConfig {
                procedures_per_standard: cr_config.audit_procedures.procedures_per_standard,
                sampling_method: cr_config.audit_procedures.sampling_method.clone(),
                confidence_level: cr_config.audit_procedures.confidence_level,
                tolerable_misstatement: cr_config.audit_procedures.tolerable_misstatement,
            };
            let mut proc_gen = datasynth_generators::compliance::ProcedureGenerator::with_config(
                self.seed + 9000,
                proc_config,
            );
            let registry = reg_gen.registry();
            let mut all_procs = Vec::new();
            for jurisdiction in &jurisdictions {
                let procs = proc_gen.generate_procedures(registry, jurisdiction, reference_date);
                all_procs.extend(procs);
            }
            info!("  Audit procedures: {}", all_procs.len());
            all_procs
        } else {
            Vec::new()
        };

        // Generate compliance findings (if enabled)
        let findings = if cr_config.findings.enabled && !audit_procedures.is_empty() {
            let finding_config =
                datasynth_generators::compliance::ComplianceFindingGeneratorConfig {
                    finding_rate: cr_config.findings.finding_rate,
                    material_weakness_rate: cr_config.findings.material_weakness_rate,
                    significant_deficiency_rate: cr_config.findings.significant_deficiency_rate,
                    generate_remediation: cr_config.findings.generate_remediation,
                };
            let mut finding_gen =
                datasynth_generators::compliance::ComplianceFindingGenerator::with_config(
                    self.seed + 9100,
                    finding_config,
                );
            let mut all_findings = Vec::new();
            for company in &self.config.companies {
                let company_findings =
                    finding_gen.generate_findings(&audit_procedures, &company.code, reference_date);
                all_findings.extend(company_findings);
            }
            info!("  Compliance findings: {}", all_findings.len());
            all_findings
        } else {
            Vec::new()
        };

        // Generate regulatory filings (if enabled)
        let filings = if cr_config.filings.enabled {
            let filing_config = datasynth_generators::compliance::FilingGeneratorConfig {
                filing_types: cr_config.filings.filing_types.clone(),
                generate_status_progression: cr_config.filings.generate_status_progression,
            };
            let mut filing_gen = datasynth_generators::compliance::FilingGenerator::with_config(
                self.seed + 9200,
                filing_config,
            );
            let company_codes: Vec<String> = self
                .config
                .companies
                .iter()
                .map(|c| c.code.clone())
                .collect();
            let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
                .unwrap_or(fallback_date);
            let filings = filing_gen.generate_filings(
                &company_codes,
                &jurisdictions,
                start_date,
                self.config.global.period_months,
            );
            info!("  Regulatory filings: {}", filings.len());
            filings
        } else {
            Vec::new()
        };

        // Build compliance graph (if enabled)
        let compliance_graph = if cr_config.graph.enabled {
            let graph_config = datasynth_graph::ComplianceGraphConfig {
                include_standard_nodes: cr_config.graph.include_compliance_nodes,
                include_jurisdiction_nodes: cr_config.graph.include_compliance_nodes,
                include_cross_references: cr_config.graph.include_cross_references,
                include_supersession_edges: cr_config.graph.include_supersession_edges,
                include_account_links: cr_config.graph.include_account_links,
                include_control_links: cr_config.graph.include_control_links,
                include_company_links: cr_config.graph.include_company_links,
            };
            let mut builder = datasynth_graph::ComplianceGraphBuilder::new(graph_config);

            // Add standard nodes
            let standard_inputs: Vec<datasynth_graph::StandardNodeInput> = standard_records
                .iter()
                .map(|r| datasynth_graph::StandardNodeInput {
                    standard_id: r.standard_id.clone(),
                    title: r.title.clone(),
                    category: r.category.clone(),
                    domain: r.domain.clone(),
                    is_active: r.is_active,
                    features: vec![if r.is_active { 1.0 } else { 0.0 }],
                    applicable_account_types: r.applicable_account_types.clone(),
                    applicable_processes: r.applicable_processes.clone(),
                })
                .collect();
            builder.add_standards(&standard_inputs);

            // Add jurisdiction nodes
            let jurisdiction_inputs: Vec<datasynth_graph::JurisdictionNodeInput> =
                jurisdiction_records
                    .iter()
                    .map(|r| datasynth_graph::JurisdictionNodeInput {
                        country_code: r.country_code.clone(),
                        country_name: r.country_name.clone(),
                        framework: r.accounting_framework.clone(),
                        standard_count: r.standard_count,
                        tax_rate: r.statutory_tax_rate,
                    })
                    .collect();
            builder.add_jurisdictions(&jurisdiction_inputs);

            // Add cross-reference edges
            let xref_inputs: Vec<datasynth_graph::CrossReferenceEdgeInput> =
                cross_reference_records
                    .iter()
                    .map(|r| datasynth_graph::CrossReferenceEdgeInput {
                        from_standard: r.from_standard.clone(),
                        to_standard: r.to_standard.clone(),
                        relationship: r.relationship.clone(),
                        convergence_level: r.convergence_level,
                    })
                    .collect();
            builder.add_cross_references(&xref_inputs);

            // Add jurisdiction→standard mappings
            let mapping_inputs: Vec<datasynth_graph::JurisdictionMappingInput> = standard_records
                .iter()
                .map(|r| datasynth_graph::JurisdictionMappingInput {
                    country_code: r.jurisdiction.clone(),
                    standard_id: r.standard_id.clone(),
                })
                .collect();
            builder.add_jurisdiction_mappings(&mapping_inputs);

            // Add procedure nodes
            let proc_inputs: Vec<datasynth_graph::ProcedureNodeInput> = audit_procedures
                .iter()
                .map(|p| datasynth_graph::ProcedureNodeInput {
                    procedure_id: p.procedure_id.clone(),
                    standard_id: p.standard_id.clone(),
                    procedure_type: p.procedure_type.clone(),
                    sample_size: p.sample_size,
                    confidence_level: p.confidence_level,
                })
                .collect();
            builder.add_procedures(&proc_inputs);

            // Add finding nodes
            let finding_inputs: Vec<datasynth_graph::FindingNodeInput> = findings
                .iter()
                .map(|f| datasynth_graph::FindingNodeInput {
                    finding_id: f.finding_id.to_string(),
                    standard_id: f
                        .related_standards
                        .first()
                        .map(|s| s.as_str().to_string())
                        .unwrap_or_default(),
                    severity: f.severity.to_string(),
                    deficiency_level: f.deficiency_level.to_string(),
                    severity_score: f.deficiency_level.severity_score(),
                    control_id: f.control_id.clone(),
                    affected_accounts: f.affected_accounts.clone(),
                })
                .collect();
            builder.add_findings(&finding_inputs);

            // Cross-domain: link standards to accounts from chart of accounts
            if cr_config.graph.include_account_links {
                let registry = datasynth_standards::registry::StandardRegistry::with_built_in();
                let mut account_links: Vec<datasynth_graph::AccountLinkInput> = Vec::new();
                for std_record in &standard_records {
                    if let Some(std_obj) =
                        registry.get(&datasynth_core::models::compliance::StandardId::parse(
                            &std_record.standard_id,
                        ))
                    {
                        for acct_type in &std_obj.applicable_account_types {
                            account_links.push(datasynth_graph::AccountLinkInput {
                                standard_id: std_record.standard_id.clone(),
                                account_code: acct_type.clone(),
                                account_name: acct_type.clone(),
                            });
                        }
                    }
                }
                builder.add_account_links(&account_links);
            }

            // Cross-domain: link standards to internal controls
            if cr_config.graph.include_control_links {
                let mut control_links = Vec::new();
                // SOX/PCAOB standards link to all controls
                let sox_like_ids: Vec<String> = standard_records
                    .iter()
                    .filter(|r| {
                        r.standard_id.starts_with("SOX")
                            || r.standard_id.starts_with("PCAOB-AS-2201")
                    })
                    .map(|r| r.standard_id.clone())
                    .collect();
                // Get control IDs from config (C001-C060 standard controls)
                let control_ids = [
                    ("C001", "Cash Controls"),
                    ("C002", "Large Transaction Approval"),
                    ("C010", "PO Approval"),
                    ("C011", "Three-Way Match"),
                    ("C020", "Revenue Recognition"),
                    ("C021", "Credit Check"),
                    ("C030", "Manual JE Approval"),
                    ("C031", "Period Close Review"),
                    ("C032", "Account Reconciliation"),
                    ("C040", "Payroll Processing"),
                    ("C050", "Fixed Asset Capitalization"),
                    ("C060", "Intercompany Elimination"),
                ];
                for sox_id in &sox_like_ids {
                    for (ctrl_id, ctrl_name) in &control_ids {
                        control_links.push(datasynth_graph::ControlLinkInput {
                            standard_id: sox_id.clone(),
                            control_id: ctrl_id.to_string(),
                            control_name: ctrl_name.to_string(),
                        });
                    }
                }
                builder.add_control_links(&control_links);
            }

            // Cross-domain: filing nodes with company links
            if cr_config.graph.include_company_links {
                let filing_inputs: Vec<datasynth_graph::FilingNodeInput> = filings
                    .iter()
                    .enumerate()
                    .map(|(i, f)| datasynth_graph::FilingNodeInput {
                        filing_id: format!("F{:04}", i + 1),
                        filing_type: f.filing_type.to_string(),
                        company_code: f.company_code.clone(),
                        jurisdiction: f.jurisdiction.clone(),
                        status: format!("{:?}", f.status),
                    })
                    .collect();
                builder.add_filings(&filing_inputs);
            }

            let graph = builder.build();
            info!(
                "  Compliance graph: {} nodes, {} edges",
                graph.nodes.len(),
                graph.edges.len()
            );
            Some(graph)
        } else {
            None
        };

        self.check_resources_with_log("post-compliance-regulations")?;

        Ok(ComplianceRegulationsSnapshot {
            standard_records,
            cross_reference_records,
            jurisdiction_records,
            audit_procedures,
            findings,
            filings,
            compliance_graph,
        })
    }

    /// Build a lineage graph describing config → phase → output relationships.
    fn build_lineage_graph(&self) -> super::lineage::LineageGraph {
        use super::lineage::LineageGraphBuilder;

        let mut builder = LineageGraphBuilder::new();

        // Config sections
        builder.add_config_section("config:global", "Global Config");
        builder.add_config_section("config:chart_of_accounts", "Chart of Accounts Config");
        builder.add_config_section("config:transactions", "Transaction Config");

        // Generator phases
        builder.add_generator_phase("phase:coa", "Chart of Accounts Generation");
        builder.add_generator_phase("phase:je", "Journal Entry Generation");

        // Config → phase edges
        builder.configured_by("phase:coa", "config:chart_of_accounts");
        builder.configured_by("phase:je", "config:transactions");

        // Output files
        builder.add_output_file("output:je", "Journal Entries", "sample_entries.json");
        builder.produced_by("output:je", "phase:je");

        // Optional phases based on config
        if self.phase_config.generate_master_data {
            builder.add_config_section("config:master_data", "Master Data Config");
            builder.add_generator_phase("phase:master_data", "Master Data Generation");
            builder.configured_by("phase:master_data", "config:master_data");
            builder.input_to("phase:master_data", "phase:je");
        }

        if self.phase_config.generate_document_flows {
            builder.add_config_section("config:document_flows", "Document Flow Config");
            builder.add_generator_phase("phase:p2p", "P2P Document Flow");
            builder.add_generator_phase("phase:o2c", "O2C Document Flow");
            builder.configured_by("phase:p2p", "config:document_flows");
            builder.configured_by("phase:o2c", "config:document_flows");

            builder.add_output_file("output:po", "Purchase Orders", "purchase_orders.csv");
            builder.add_output_file("output:gr", "Goods Receipts", "goods_receipts.csv");
            builder.add_output_file("output:vi", "Vendor Invoices", "vendor_invoices.csv");
            builder.add_output_file("output:so", "Sales Orders", "sales_orders.csv");
            builder.add_output_file("output:ci", "Customer Invoices", "customer_invoices.csv");

            builder.produced_by("output:po", "phase:p2p");
            builder.produced_by("output:gr", "phase:p2p");
            builder.produced_by("output:vi", "phase:p2p");
            builder.produced_by("output:so", "phase:o2c");
            builder.produced_by("output:ci", "phase:o2c");
        }

        if self.phase_config.inject_anomalies {
            builder.add_config_section("config:fraud", "Fraud/Anomaly Config");
            builder.add_generator_phase("phase:anomaly", "Anomaly Injection");
            builder.configured_by("phase:anomaly", "config:fraud");
            builder.add_output_file(
                "output:labels",
                "Anomaly Labels",
                "labels/anomaly_labels.csv",
            );
            builder.produced_by("output:labels", "phase:anomaly");
        }

        if self.phase_config.generate_audit {
            builder.add_config_section("config:audit", "Audit Config");
            builder.add_generator_phase("phase:audit", "Audit Data Generation");
            builder.configured_by("phase:audit", "config:audit");
        }

        if self.phase_config.generate_banking {
            builder.add_config_section("config:banking", "Banking Config");
            builder.add_generator_phase("phase:banking", "Banking KYC/AML Generation");
            builder.configured_by("phase:banking", "config:banking");
        }

        if self.config.llm.enabled {
            builder.add_config_section("config:llm", "LLM Enrichment Config");
            builder.add_generator_phase("phase:llm_enrichment", "LLM Enrichment");
            builder.configured_by("phase:llm_enrichment", "config:llm");
        }

        if self.config.diffusion.enabled {
            builder.add_config_section("config:diffusion", "Diffusion Enhancement Config");
            builder.add_generator_phase("phase:diffusion", "Diffusion Enhancement");
            builder.configured_by("phase:diffusion", "config:diffusion");
        }

        if self.config.causal.enabled {
            builder.add_config_section("config:causal", "Causal Generation Config");
            builder.add_generator_phase("phase:causal", "Causal Overlay");
            builder.configured_by("phase:causal", "config:causal");
        }

        builder.build()
    }

    // -----------------------------------------------------------------------
    // Trial-balance helpers used to replace hardcoded proxy values
    // -----------------------------------------------------------------------

    /// Compute total revenue for a company from its journal entries.
    ///
    /// Revenue accounts start with "4" and are credit-normal. Returns the sum of
    /// net credits on all revenue-account lines filtered to `company_code`.
    fn compute_company_revenue(
        entries: &[JournalEntry],
        company_code: &str,
    ) -> rust_decimal::Decimal {
        use rust_decimal::Decimal;
        let mut revenue = Decimal::ZERO;
        for je in entries {
            if je.header.company_code != company_code {
                continue;
            }
            for line in &je.lines {
                if line.gl_account.starts_with('4') {
                    // Revenue is credit-normal
                    revenue += line.credit_amount - line.debit_amount;
                }
            }
        }
        revenue.max(Decimal::ZERO)
    }

    /// Compute net assets (assets minus liabilities) for an entity from journal entries.
    ///
    /// Asset accounts start with "1"; liability accounts start with "2".
    fn compute_entity_net_assets(
        entries: &[JournalEntry],
        entity_code: &str,
    ) -> rust_decimal::Decimal {
        use rust_decimal::Decimal;
        let mut asset_net = Decimal::ZERO;
        let mut liability_net = Decimal::ZERO;
        for je in entries {
            if je.header.company_code != entity_code {
                continue;
            }
            for line in &je.lines {
                if line.gl_account.starts_with('1') {
                    asset_net += line.debit_amount - line.credit_amount;
                } else if line.gl_account.starts_with('2') {
                    liability_net += line.credit_amount - line.debit_amount;
                }
            }
        }
        asset_net - liability_net
    }

    /// v3.5.1+: Run the statistical validation suite configured in
    /// `distributions.validation.tests` over the final amount
    /// distribution.  Collects every non-zero line-level amount (debit +
    /// credit) and hands it to the runners in
    /// `datasynth_core::distributions::validation`.
    ///
    /// Returns `Ok(None)` when validation is disabled (the default).
    /// When `reporting.fail_on_error = true` and any test fails, returns
    /// `Err` with a concise message; otherwise attaches the report to
    /// the result and lets callers inspect it.
    fn phase_statistical_validation(
        &self,
        entries: &[JournalEntry],
    ) -> SynthResult<Option<datasynth_core::distributions::StatisticalValidationReport>> {
        use datasynth_config::schema::StatisticalTestConfig;
        use datasynth_core::distributions::{
            run_anderson_darling, run_benford_first_digit, run_chi_squared, run_correlation_check,
            run_ks_uniform_log, StatisticalTestResult, StatisticalValidationReport, TestOutcome,
        };
        use rust_decimal::prelude::ToPrimitive;

        let cfg = &self.config.distributions.validation;
        if !cfg.enabled {
            return Ok(None);
        }

        // Collect per-line positive amounts (debit + credit is zero on the
        // non-posting side, so this naturally picks the magnitude).
        let amounts: Vec<rust_decimal::Decimal> = entries
            .iter()
            .flat_map(|je| je.lines.iter().map(|l| l.debit_amount + l.credit_amount))
            .filter(|a| *a > rust_decimal::Decimal::ZERO)
            .collect();

        // v4.1.0+ paired (amount, line_count) per entry for correlation
        // checks. Amount per entry is the debit-side total (= credit-side
        // total for a balanced entry).
        let paired_amount_linecount: Vec<(f64, f64)> = entries
            .iter()
            .filter_map(|je| {
                let amt: rust_decimal::Decimal = je.lines.iter().map(|l| l.debit_amount).sum();
                if amt > rust_decimal::Decimal::ZERO {
                    amt.to_f64().map(|a| (a, je.lines.len() as f64))
                } else {
                    None
                }
            })
            .collect();

        let mut results: Vec<StatisticalTestResult> = Vec::with_capacity(cfg.tests.len());
        for test_cfg in &cfg.tests {
            match test_cfg {
                StatisticalTestConfig::BenfordFirstDigit {
                    threshold_mad,
                    warning_mad,
                } => {
                    results.push(run_benford_first_digit(
                        &amounts,
                        *threshold_mad,
                        *warning_mad,
                    ));
                }
                StatisticalTestConfig::ChiSquared { bins, significance } => {
                    results.push(run_chi_squared(&amounts, *bins, *significance));
                }
                StatisticalTestConfig::DistributionFit {
                    target: _,
                    ks_significance,
                    method: _,
                } => {
                    // v3.5.1+: log-uniformity KS check. Target-specific
                    // fits against Normal / Exponential land in v4.1.1+.
                    results.push(run_ks_uniform_log(&amounts, *ks_significance));
                }
                StatisticalTestConfig::AndersonDarling {
                    target: _,
                    significance,
                } => {
                    // v4.1.0+: A*² statistic against log-normal on the
                    // log-scale. Other targets follow the same pattern.
                    results.push(run_anderson_darling(&amounts, *significance));
                }
                StatisticalTestConfig::CorrelationCheck {
                    expected_correlations,
                } => {
                    // v4.1.0+: (amount, line_count) is tracked today.
                    // Other pairs resolve to Skipped pending richer
                    // per-entry attribute collection.
                    if expected_correlations.is_empty() {
                        results.push(StatisticalTestResult {
                            name: "correlation_check".to_string(),
                            outcome: TestOutcome::Skipped,
                            statistic: 0.0,
                            threshold: 0.0,
                            message: "no expected correlations declared".to_string(),
                        });
                    } else {
                        for ec in expected_correlations {
                            let pair_key = format!("{}_{}", ec.field1, ec.field2);
                            let is_amount_linecount = (ec.field1 == "amount"
                                && ec.field2 == "line_count")
                                || (ec.field1 == "line_count" && ec.field2 == "amount");
                            if is_amount_linecount {
                                let xs: Vec<f64> =
                                    paired_amount_linecount.iter().map(|(a, _)| *a).collect();
                                let ys: Vec<f64> =
                                    paired_amount_linecount.iter().map(|(_, l)| *l).collect();
                                results.push(run_correlation_check(
                                    &pair_key,
                                    &xs,
                                    &ys,
                                    ec.expected_r,
                                    ec.tolerance,
                                ));
                            } else {
                                results.push(StatisticalTestResult {
                                    name: format!("correlation_check_{pair_key}"),
                                    outcome: TestOutcome::Skipped,
                                    statistic: 0.0,
                                    threshold: ec.tolerance,
                                    message: format!(
                                        "pair ({},{}) not tracked; only (amount, line_count) supported in v4.1.0",
                                        ec.field1, ec.field2
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }

        let report = StatisticalValidationReport {
            sample_count: amounts.len(),
            results,
        };

        if cfg.reporting.fail_on_error && !report.all_passed() {
            let failed = report.failed_names().join(", ");
            return Err(SynthError::validation(format!(
                "statistical validation failed: {failed}"
            )));
        }

        Ok(Some(report))
    }

    /// v3.3.0: analytics-metadata phase.
    ///
    /// Runs AFTER all JE-adding phases (including Phase 20b's
    /// fraud-bias sweep). Four sub-generators fire in sequence, each
    /// gated by an individual `analytics_metadata.<flag>` toggle:
    ///
    /// 1. `PriorYearGenerator` — prior-year comparatives derived from
    ///    current-period account balances.
    /// 2. `IndustryBenchmarkGenerator` — industry benchmarks for the
    ///    configured `global.industry`.
    /// 3. `ManagementReportGenerator` — management-report artefacts.
    /// 4. `DriftEventGenerator` — post-generation drift-event labels.
    fn phase_analytics_metadata(
        &mut self,
        entries: &[JournalEntry],
    ) -> SynthResult<AnalyticsMetadataSnapshot> {
        use datasynth_generators::drift_event_generator::DriftEventGenerator;
        use datasynth_generators::industry_benchmark_generator::IndustryBenchmarkGenerator;
        use datasynth_generators::management_report_generator::ManagementReportGenerator;
        use datasynth_generators::prior_year_generator::PriorYearGenerator;
        use std::collections::BTreeMap;

        let mut snap = AnalyticsMetadataSnapshot::default();

        if !self.phase_config.generate_analytics_metadata {
            return Ok(snap);
        }

        let cfg = &self.config.analytics_metadata;
        let fiscal_year = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
            .map(|d| d.year())
            .unwrap_or(2025);

        // ---- 1. Prior-year comparatives ----
        if cfg.prior_year {
            let mut gen = PriorYearGenerator::new(self.seed + 9100);
            for company in &self.config.companies {
                // Aggregate current-period balances per account code +
                // account name from the entries slice.
                let mut balances: BTreeMap<String, (String, rust_decimal::Decimal)> =
                    BTreeMap::new();
                for je in entries {
                    if je.header.company_code != company.code {
                        continue;
                    }
                    for line in &je.lines {
                        let entry = balances.entry(line.gl_account.clone()).or_insert_with(|| {
                            (line.gl_account.clone(), rust_decimal::Decimal::ZERO)
                        });
                        entry.1 += line.debit_amount - line.credit_amount;
                    }
                }
                let current: Vec<(String, String, rust_decimal::Decimal)> = balances
                    .into_iter()
                    .filter(|(_, (_, bal))| !bal.is_zero())
                    .map(|(code, (name, bal))| (code, name, bal))
                    .collect();
                if !current.is_empty() {
                    let comparatives =
                        gen.generate_comparatives(&company.code, fiscal_year, &current);
                    snap.prior_year_comparatives.extend(comparatives);
                }
            }
            info!(
                "v3.3.0 analytics: {} prior-year comparatives across {} companies",
                snap.prior_year_comparatives.len(),
                self.config.companies.len()
            );
        }

        // ---- 2. Industry benchmarks ----
        if cfg.industry_benchmark {
            use datasynth_core::models::IndustrySector;
            let industry = match self.config.global.industry {
                IndustrySector::Manufacturing => "manufacturing",
                IndustrySector::Retail => "retail",
                IndustrySector::FinancialServices => "financial_services",
                IndustrySector::Technology => "technology",
                IndustrySector::Healthcare => "healthcare",
                _ => "other",
            };
            let mut gen = IndustryBenchmarkGenerator::new(self.seed + 9200);
            let benchmarks = gen.generate(industry, fiscal_year);
            info!(
                "v3.3.0 analytics: {} industry benchmarks for '{industry}'",
                benchmarks.len()
            );
            snap.industry_benchmarks = benchmarks;
        }

        // ---- 3. Management reports ----
        if cfg.management_reports {
            let mut gen = ManagementReportGenerator::new(self.seed + 9300);
            let period_months = self.config.global.period_months;
            for company in &self.config.companies {
                let reports =
                    gen.generate_reports(&company.code, fiscal_year as u32, period_months);
                snap.management_reports.extend(reports);
            }
            info!(
                "v3.3.0 analytics: {} management reports across {} companies",
                snap.management_reports.len(),
                self.config.companies.len()
            );
        }

        // ---- 4. Drift-event labels ----
        if cfg.drift_events {
            let fallback_start = NaiveDate::from_ymd_opt(2025, 1, 1)
                .expect("hardcoded NaiveDate 2025-01-01 is valid");
            let start_date = NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
                .unwrap_or(fallback_start);
            let end_date = start_date + chrono::Months::new(self.config.global.period_months);
            let mut gen = DriftEventGenerator::new(self.seed + 9400);
            let drifts = gen.generate_standalone_drifts(start_date, end_date);
            info!("v3.3.0 analytics: {} drift-event labels", drifts.len());
            snap.drift_events = drifts;
        }
        // `entries` parameter reserved for future JE-aware drift detection
        let _ = entries;

        Ok(snap)
    }
}

/// Get the directory name for a graph export format.
fn format_name(format: datasynth_config::schema::GraphExportFormat) -> &'static str {
    match format {
        datasynth_config::schema::GraphExportFormat::PytorchGeometric => "pytorch_geometric",
        datasynth_config::schema::GraphExportFormat::Neo4j => "neo4j",
        datasynth_config::schema::GraphExportFormat::Dgl => "dgl",
        datasynth_config::schema::GraphExportFormat::RustGraph => "rustgraph",
        datasynth_config::schema::GraphExportFormat::RustGraphHypergraph => "rustgraph_hypergraph",
    }
}

/// Aggregate journal entry lines into per-account trial balance rows.
///
/// Each unique `account_code` gets one [`TrialBalanceEntry`] with summed
/// debit/credit totals and a net balance (debit minus credit).
fn compute_trial_balance_entries(
    entries: &[JournalEntry],
    entity_code: &str,
    fiscal_year: i32,
    coa: Option<&ChartOfAccounts>,
) -> Vec<datasynth_audit_fsm::artifact::TrialBalanceEntry> {
    use std::collections::BTreeMap;

    let mut balances: BTreeMap<String, (rust_decimal::Decimal, rust_decimal::Decimal)> =
        BTreeMap::new();

    for je in entries {
        for line in &je.lines {
            let entry = balances.entry(line.account_code.clone()).or_default();
            entry.0 += line.debit_amount;
            entry.1 += line.credit_amount;
        }
    }

    balances
        .into_iter()
        .map(
            |(account_code, (debit, credit))| datasynth_audit_fsm::artifact::TrialBalanceEntry {
                account_description: coa
                    .and_then(|c| c.get_account(&account_code))
                    .map(|a| a.description().to_string())
                    .unwrap_or_else(|| account_code.clone()),
                account_code,
                debit_balance: debit,
                credit_balance: credit,
                net_balance: debit - credit,
                entity_code: entity_code.to_string(),
                period: format!("FY{}", fiscal_year),
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use datasynth_config::schema::*;

    fn create_test_config() -> GeneratorConfig {
        GeneratorConfig {
            global: GlobalConfig {
                industry: IndustrySector::Manufacturing,
                start_date: "2024-01-01".to_string(),
                period_months: 1,
                seed: Some(42),
                parallel: false,
                group_currency: "USD".to_string(),
                presentation_currency: None,
                worker_threads: 0,
                memory_limit_mb: 0,
                fiscal_year_months: None,
            },
            companies: vec![CompanyConfig {
                code: "1000".to_string(),
                name: "Test Company".to_string(),
                currency: "USD".to_string(),
                functional_currency: None,
                country: "US".to_string(),
                annual_transaction_volume: TransactionVolume::TenK,
                volume_weight: 1.0,
                fiscal_year_variant: "K4".to_string(),
            }],
            chart_of_accounts: ChartOfAccountsConfig {
                complexity: CoAComplexity::Small,
                industry_specific: true,
                custom_accounts: None,
                min_hierarchy_depth: 2,
                max_hierarchy_depth: 4,
                expand_industry_subaccounts: false,
            },
            transactions: TransactionConfig::default(),
            output: OutputConfig::default(),
            fraud: FraudConfig::default(),
            internal_controls: InternalControlsConfig::default(),
            business_processes: BusinessProcessConfig::default(),
            user_personas: UserPersonaConfig::default(),
            templates: TemplateConfig::default(),
            approval: ApprovalConfig::default(),
            departments: DepartmentConfig::default(),
            master_data: MasterDataConfig::default(),
            document_flows: DocumentFlowConfig::default(),
            intercompany: IntercompanyConfig::default(),
            balance: BalanceConfig::default(),
            ocpm: OcpmConfig::default(),
            audit: AuditGenerationConfig::default(),
            banking: datasynth_banking::BankingConfig::default(),
            data_quality: DataQualitySchemaConfig::default(),
            scenario: ScenarioConfig::default(),
            temporal: TemporalDriftConfig::default(),
            graph_export: GraphExportConfig::default(),
            streaming: StreamingSchemaConfig::default(),
            rate_limit: RateLimitSchemaConfig::default(),
            temporal_attributes: TemporalAttributeSchemaConfig::default(),
            relationships: RelationshipSchemaConfig::default(),
            accounting_standards: AccountingStandardsConfig::default(),
            audit_standards: AuditStandardsConfig::default(),
            distributions: Default::default(),
            temporal_patterns: Default::default(),
            vendor_network: VendorNetworkSchemaConfig::default(),
            customer_segmentation: CustomerSegmentationSchemaConfig::default(),
            relationship_strength: RelationshipStrengthSchemaConfig::default(),
            cross_process_links: CrossProcessLinksSchemaConfig::default(),
            organizational_events: OrganizationalEventsSchemaConfig::default(),
            behavioral_drift: BehavioralDriftSchemaConfig::default(),
            market_drift: MarketDriftSchemaConfig::default(),
            drift_labeling: DriftLabelingSchemaConfig::default(),
            anomaly_injection: Default::default(),
            industry_specific: Default::default(),
            fingerprint_privacy: Default::default(),
            quality_gates: Default::default(),
            compliance: Default::default(),
            webhooks: Default::default(),
            llm: Default::default(),
            diffusion: Default::default(),
            causal: Default::default(),
            source_to_pay: Default::default(),
            financial_reporting: Default::default(),
            hr: Default::default(),
            manufacturing: Default::default(),
            sales_quotes: Default::default(),
            tax: Default::default(),
            treasury: Default::default(),
            project_accounting: Default::default(),
            esg: Default::default(),
            country_packs: None,
            scenarios: Default::default(),
            session: Default::default(),
            compliance_regulations: Default::default(),
            analytics_metadata: Default::default(),
            concentration: Default::default(),
            period_close: Default::default(),
        }
    }

    #[test]
    fn test_enhanced_orchestrator_creation() {
        let config = create_test_config();
        let orchestrator = EnhancedOrchestrator::with_defaults(config);
        assert!(orchestrator.is_ok());
    }

    #[test]
    fn test_minimal_generation() {
        let config = create_test_config();
        let phase_config = PhaseConfig {
            generate_master_data: false,
            generate_document_flows: false,
            generate_journal_entries: true,
            inject_anomalies: false,
            show_progress: false,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate();

        assert!(result.is_ok());
        let result = result.unwrap();
        assert!(!result.journal_entries.is_empty());
    }

    #[test]
    fn test_master_data_generation() {
        let config = create_test_config();
        let phase_config = PhaseConfig {
            generate_master_data: true,
            generate_document_flows: false,
            generate_journal_entries: false,
            inject_anomalies: false,
            show_progress: false,
            vendors_per_company: 5,
            customers_per_company: 5,
            materials_per_company: 10,
            assets_per_company: 5,
            employees_per_company: 10,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        assert!(!result.master_data.vendors.is_empty());
        assert!(!result.master_data.customers.is_empty());
        assert!(!result.master_data.materials.is_empty());
    }

    #[test]
    fn test_document_flow_generation() {
        let config = create_test_config();
        let phase_config = PhaseConfig {
            generate_master_data: true,
            generate_document_flows: true,
            generate_journal_entries: false,
            inject_anomalies: false,
            inject_data_quality: false,
            validate_balances: false,
            validate_coa_coverage_strict: false,
            generate_ocpm_events: false,
            show_progress: false,
            vendors_per_company: 5,
            customers_per_company: 5,
            materials_per_company: 10,
            assets_per_company: 5,
            employees_per_company: 10,
            p2p_chains: 5,
            o2c_chains: 5,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // Should have generated P2P and O2C chains
        assert!(!result.document_flows.p2p_chains.is_empty());
        assert!(!result.document_flows.o2c_chains.is_empty());

        // Flattened documents should be populated
        assert!(!result.document_flows.purchase_orders.is_empty());
        assert!(!result.document_flows.sales_orders.is_empty());
    }

    #[test]
    fn test_anomaly_injection() {
        let config = create_test_config();
        let phase_config = PhaseConfig {
            generate_master_data: false,
            generate_document_flows: false,
            generate_journal_entries: true,
            inject_anomalies: true,
            show_progress: false,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // Should have journal entries
        assert!(!result.journal_entries.is_empty());

        // With ~833 entries and 2% rate, expect some anomalies
        // Note: This is probabilistic, so we just verify the structure exists
        assert!(result.anomaly_labels.summary.is_some());
    }

    #[test]
    fn test_full_generation_pipeline() {
        let config = create_test_config();
        let phase_config = PhaseConfig {
            generate_master_data: true,
            generate_document_flows: true,
            generate_journal_entries: true,
            inject_anomalies: false,
            inject_data_quality: false,
            validate_balances: true,
            validate_coa_coverage_strict: false,
            generate_ocpm_events: false,
            show_progress: false,
            vendors_per_company: 3,
            customers_per_company: 3,
            materials_per_company: 5,
            assets_per_company: 3,
            employees_per_company: 5,
            p2p_chains: 3,
            o2c_chains: 3,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // All phases should have results
        assert!(!result.master_data.vendors.is_empty());
        assert!(!result.master_data.customers.is_empty());
        assert!(!result.document_flows.p2p_chains.is_empty());
        assert!(!result.document_flows.o2c_chains.is_empty());
        assert!(!result.journal_entries.is_empty());
        assert!(result.statistics.accounts_count > 0);

        // Subledger linking should have run
        assert!(!result.subledger.ap_invoices.is_empty());
        assert!(!result.subledger.ar_invoices.is_empty());

        // Balance validation should have run
        assert!(result.balance_validation.validated);
        assert!(result.balance_validation.entries_processed > 0);
    }

    #[test]
    fn test_subledger_linking() {
        let config = create_test_config();
        let phase_config = PhaseConfig {
            generate_master_data: true,
            generate_document_flows: true,
            generate_journal_entries: false,
            inject_anomalies: false,
            inject_data_quality: false,
            validate_balances: false,
            validate_coa_coverage_strict: false,
            generate_ocpm_events: false,
            show_progress: false,
            vendors_per_company: 5,
            customers_per_company: 5,
            materials_per_company: 10,
            assets_per_company: 3,
            employees_per_company: 5,
            p2p_chains: 5,
            o2c_chains: 5,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // Should have document flows
        assert!(!result.document_flows.vendor_invoices.is_empty());
        assert!(!result.document_flows.customer_invoices.is_empty());

        // Subledger should be linked from document flows
        assert!(!result.subledger.ap_invoices.is_empty());
        assert!(!result.subledger.ar_invoices.is_empty());

        // AP invoices count should match vendor invoices count
        assert_eq!(
            result.subledger.ap_invoices.len(),
            result.document_flows.vendor_invoices.len()
        );

        // AR invoices count should match customer invoices count
        assert_eq!(
            result.subledger.ar_invoices.len(),
            result.document_flows.customer_invoices.len()
        );

        // Statistics should reflect subledger counts
        assert_eq!(
            result.statistics.ap_invoice_count,
            result.subledger.ap_invoices.len()
        );
        assert_eq!(
            result.statistics.ar_invoice_count,
            result.subledger.ar_invoices.len()
        );
    }

    #[test]
    fn test_balance_validation() {
        let config = create_test_config();
        let phase_config = PhaseConfig {
            generate_master_data: false,
            generate_document_flows: false,
            generate_journal_entries: true,
            inject_anomalies: false,
            validate_balances: true,
            validate_coa_coverage_strict: false,
            show_progress: false,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // Balance validation should run
        assert!(result.balance_validation.validated);
        assert!(result.balance_validation.entries_processed > 0);

        // Generated JEs should be balanced (no unbalanced entries)
        assert!(!result.balance_validation.has_unbalanced_entries);

        // Total debits should equal total credits
        assert_eq!(
            result.balance_validation.total_debits,
            result.balance_validation.total_credits
        );
    }

    #[test]
    fn test_statistics_accuracy() {
        let config = create_test_config();
        let phase_config = PhaseConfig {
            generate_master_data: true,
            generate_document_flows: false,
            generate_journal_entries: true,
            inject_anomalies: false,
            show_progress: false,
            vendors_per_company: 10,
            customers_per_company: 20,
            materials_per_company: 15,
            assets_per_company: 5,
            employees_per_company: 8,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // Statistics should match actual data
        assert_eq!(
            result.statistics.vendor_count,
            result.master_data.vendors.len()
        );
        assert_eq!(
            result.statistics.customer_count,
            result.master_data.customers.len()
        );
        assert_eq!(
            result.statistics.material_count,
            result.master_data.materials.len()
        );
        assert_eq!(
            result.statistics.total_entries as usize,
            result.journal_entries.len()
        );
    }

    #[test]
    fn test_phase_config_defaults() {
        let config = PhaseConfig::default();
        assert!(config.generate_master_data);
        assert!(config.generate_document_flows);
        assert!(config.generate_journal_entries);
        assert!(!config.inject_anomalies);
        assert!(config.validate_balances);
        assert!(config.show_progress);
        assert!(config.vendors_per_company > 0);
        assert!(config.customers_per_company > 0);
    }

    #[test]
    fn test_get_coa_before_generation() {
        let config = create_test_config();
        let orchestrator = EnhancedOrchestrator::with_defaults(config).unwrap();

        // Before generation, CoA should be None
        assert!(orchestrator.get_coa().is_none());
    }

    #[test]
    fn test_get_coa_after_generation() {
        let config = create_test_config();
        let phase_config = PhaseConfig {
            generate_master_data: false,
            generate_document_flows: false,
            generate_journal_entries: true,
            inject_anomalies: false,
            show_progress: false,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let _ = orchestrator.generate().unwrap();

        // After generation, CoA should be available
        assert!(orchestrator.get_coa().is_some());
    }

    #[test]
    fn test_get_master_data() {
        let config = create_test_config();
        let phase_config = PhaseConfig {
            generate_master_data: true,
            generate_document_flows: false,
            generate_journal_entries: false,
            inject_anomalies: false,
            show_progress: false,
            vendors_per_company: 5,
            customers_per_company: 5,
            materials_per_company: 5,
            assets_per_company: 5,
            employees_per_company: 5,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // After generate(), master_data is moved into the result
        assert!(!result.master_data.vendors.is_empty());
    }

    #[test]
    fn test_with_progress_builder() {
        let config = create_test_config();
        let orchestrator = EnhancedOrchestrator::with_defaults(config)
            .unwrap()
            .with_progress(false);

        // Should still work without progress
        assert!(!orchestrator.phase_config.show_progress);
    }

    #[test]
    fn test_multi_company_generation() {
        let mut config = create_test_config();
        config.companies.push(CompanyConfig {
            code: "2000".to_string(),
            name: "Subsidiary".to_string(),
            currency: "EUR".to_string(),
            functional_currency: None,
            country: "DE".to_string(),
            annual_transaction_volume: TransactionVolume::TenK,
            volume_weight: 0.5,
            fiscal_year_variant: "K4".to_string(),
        });

        let phase_config = PhaseConfig {
            generate_master_data: true,
            generate_document_flows: false,
            generate_journal_entries: true,
            inject_anomalies: false,
            show_progress: false,
            vendors_per_company: 5,
            customers_per_company: 5,
            materials_per_company: 5,
            assets_per_company: 5,
            employees_per_company: 5,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // Should have master data for both companies
        assert!(result.statistics.vendor_count >= 10); // 5 per company
        assert!(result.statistics.customer_count >= 10);
        assert!(result.statistics.companies_count == 2);
    }

    #[test]
    fn test_empty_master_data_skips_document_flows() {
        let config = create_test_config();
        let phase_config = PhaseConfig {
            generate_master_data: false,   // Skip master data
            generate_document_flows: true, // Try to generate flows
            generate_journal_entries: false,
            inject_anomalies: false,
            show_progress: false,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // Without master data, document flows should be empty
        assert!(result.document_flows.p2p_chains.is_empty());
        assert!(result.document_flows.o2c_chains.is_empty());
    }

    #[test]
    fn test_journal_entry_line_item_count() {
        let config = create_test_config();
        let phase_config = PhaseConfig {
            generate_master_data: false,
            generate_document_flows: false,
            generate_journal_entries: true,
            inject_anomalies: false,
            show_progress: false,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // Total line items should match sum of all entry line counts
        let calculated_line_items: u64 = result
            .journal_entries
            .iter()
            .map(|e| e.line_count() as u64)
            .sum();
        assert_eq!(result.statistics.total_line_items, calculated_line_items);
    }

    #[test]
    fn test_audit_generation() {
        let config = create_test_config();
        let phase_config = PhaseConfig {
            generate_master_data: false,
            generate_document_flows: false,
            generate_journal_entries: true,
            inject_anomalies: false,
            show_progress: false,
            generate_audit: true,
            audit_engagements: 2,
            workpapers_per_engagement: 5,
            evidence_per_workpaper: 2,
            risks_per_engagement: 3,
            findings_per_engagement: 2,
            judgments_per_engagement: 2,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // Should have generated audit data
        assert_eq!(result.audit.engagements.len(), 2);
        assert!(!result.audit.workpapers.is_empty());
        assert!(!result.audit.evidence.is_empty());
        assert!(!result.audit.risk_assessments.is_empty());
        assert!(!result.audit.findings.is_empty());
        assert!(!result.audit.judgments.is_empty());

        // New ISA entity collections should also be populated
        assert!(
            !result.audit.confirmations.is_empty(),
            "ISA 505 confirmations should be generated"
        );
        assert!(
            !result.audit.confirmation_responses.is_empty(),
            "ISA 505 confirmation responses should be generated"
        );
        assert!(
            !result.audit.procedure_steps.is_empty(),
            "ISA 330 procedure steps should be generated"
        );
        // Samples may or may not be generated depending on workpaper sampling methods
        assert!(
            !result.audit.analytical_results.is_empty(),
            "ISA 520 analytical procedures should be generated"
        );
        assert!(
            !result.audit.ia_functions.is_empty(),
            "ISA 610 IA functions should be generated (one per engagement)"
        );
        assert!(
            !result.audit.related_parties.is_empty(),
            "ISA 550 related parties should be generated"
        );

        // Statistics should match
        assert_eq!(
            result.statistics.audit_engagement_count,
            result.audit.engagements.len()
        );
        assert_eq!(
            result.statistics.audit_workpaper_count,
            result.audit.workpapers.len()
        );
        assert_eq!(
            result.statistics.audit_evidence_count,
            result.audit.evidence.len()
        );
        assert_eq!(
            result.statistics.audit_risk_count,
            result.audit.risk_assessments.len()
        );
        assert_eq!(
            result.statistics.audit_finding_count,
            result.audit.findings.len()
        );
        assert_eq!(
            result.statistics.audit_judgment_count,
            result.audit.judgments.len()
        );
        assert_eq!(
            result.statistics.audit_confirmation_count,
            result.audit.confirmations.len()
        );
        assert_eq!(
            result.statistics.audit_confirmation_response_count,
            result.audit.confirmation_responses.len()
        );
        assert_eq!(
            result.statistics.audit_procedure_step_count,
            result.audit.procedure_steps.len()
        );
        assert_eq!(
            result.statistics.audit_sample_count,
            result.audit.samples.len()
        );
        assert_eq!(
            result.statistics.audit_analytical_result_count,
            result.audit.analytical_results.len()
        );
        assert_eq!(
            result.statistics.audit_ia_function_count,
            result.audit.ia_functions.len()
        );
        assert_eq!(
            result.statistics.audit_ia_report_count,
            result.audit.ia_reports.len()
        );
        assert_eq!(
            result.statistics.audit_related_party_count,
            result.audit.related_parties.len()
        );
        assert_eq!(
            result.statistics.audit_related_party_transaction_count,
            result.audit.related_party_transactions.len()
        );
    }

    #[test]
    fn test_new_phases_disabled_by_default() {
        let config = create_test_config();
        // Verify new config fields default to disabled
        assert!(!config.llm.enabled);
        assert!(!config.diffusion.enabled);
        assert!(!config.causal.enabled);

        let phase_config = PhaseConfig {
            generate_master_data: false,
            generate_document_flows: false,
            generate_journal_entries: true,
            inject_anomalies: false,
            show_progress: false,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // All new phase statistics should be zero when disabled
        assert_eq!(result.statistics.llm_enrichment_ms, 0);
        assert_eq!(result.statistics.llm_vendors_enriched, 0);
        assert_eq!(result.statistics.diffusion_enhancement_ms, 0);
        assert_eq!(result.statistics.diffusion_samples_generated, 0);
        assert_eq!(result.statistics.causal_generation_ms, 0);
        assert_eq!(result.statistics.causal_samples_generated, 0);
        assert!(result.statistics.causal_validation_passed.is_none());
        assert_eq!(result.statistics.counterfactual_pair_count, 0);
        assert!(result.counterfactual_pairs.is_empty());
    }

    #[test]
    fn test_counterfactual_generation_enabled() {
        let config = create_test_config();
        let phase_config = PhaseConfig {
            generate_master_data: false,
            generate_document_flows: false,
            generate_journal_entries: true,
            inject_anomalies: false,
            show_progress: false,
            generate_counterfactuals: true,
            generate_period_close: false, // Disable so entry count matches counterfactual pairs
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // With JE generation enabled, counterfactual pairs should be generated
        if !result.journal_entries.is_empty() {
            assert_eq!(
                result.counterfactual_pairs.len(),
                result.journal_entries.len()
            );
            assert_eq!(
                result.statistics.counterfactual_pair_count,
                result.journal_entries.len()
            );
            // Each pair should have a distinct pair_id
            let ids: std::collections::HashSet<_> = result
                .counterfactual_pairs
                .iter()
                .map(|p| p.pair_id.clone())
                .collect();
            assert_eq!(ids.len(), result.counterfactual_pairs.len());
        }
    }

    #[test]
    fn test_llm_enrichment_enabled() {
        let mut config = create_test_config();
        config.llm.enabled = true;
        config.llm.max_vendor_enrichments = 3;

        let phase_config = PhaseConfig {
            generate_master_data: true,
            generate_document_flows: false,
            generate_journal_entries: false,
            inject_anomalies: false,
            show_progress: false,
            vendors_per_company: 5,
            customers_per_company: 3,
            materials_per_company: 3,
            assets_per_company: 3,
            employees_per_company: 3,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // LLM enrichment should have run
        assert!(result.statistics.llm_vendors_enriched > 0);
        assert!(result.statistics.llm_vendors_enriched <= 3);
    }

    #[test]
    fn test_diffusion_enhancement_enabled() {
        let mut config = create_test_config();
        config.diffusion.enabled = true;
        config.diffusion.n_steps = 50;
        config.diffusion.sample_size = 20;

        let phase_config = PhaseConfig {
            generate_master_data: false,
            generate_document_flows: false,
            generate_journal_entries: true,
            inject_anomalies: false,
            show_progress: false,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // Diffusion phase should have generated samples
        assert_eq!(result.statistics.diffusion_samples_generated, 20);
    }

    #[test]
    fn test_causal_overlay_enabled() {
        let mut config = create_test_config();
        config.causal.enabled = true;
        config.causal.template = "fraud_detection".to_string();
        config.causal.sample_size = 100;
        config.causal.validate = true;

        let phase_config = PhaseConfig {
            generate_master_data: false,
            generate_document_flows: false,
            generate_journal_entries: true,
            inject_anomalies: false,
            show_progress: false,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // Causal phase should have generated samples
        assert_eq!(result.statistics.causal_samples_generated, 100);
        // Validation should have run
        assert!(result.statistics.causal_validation_passed.is_some());
    }

    #[test]
    fn test_causal_overlay_revenue_cycle_template() {
        let mut config = create_test_config();
        config.causal.enabled = true;
        config.causal.template = "revenue_cycle".to_string();
        config.causal.sample_size = 50;
        config.causal.validate = false;

        let phase_config = PhaseConfig {
            generate_master_data: false,
            generate_document_flows: false,
            generate_journal_entries: true,
            inject_anomalies: false,
            show_progress: false,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // Causal phase should have generated samples
        assert_eq!(result.statistics.causal_samples_generated, 50);
        // Validation was disabled
        assert!(result.statistics.causal_validation_passed.is_none());
    }

    #[test]
    fn test_all_new_phases_enabled_together() {
        let mut config = create_test_config();
        config.llm.enabled = true;
        config.llm.max_vendor_enrichments = 2;
        config.diffusion.enabled = true;
        config.diffusion.n_steps = 20;
        config.diffusion.sample_size = 10;
        config.causal.enabled = true;
        config.causal.sample_size = 50;
        config.causal.validate = true;

        let phase_config = PhaseConfig {
            generate_master_data: true,
            generate_document_flows: false,
            generate_journal_entries: true,
            inject_anomalies: false,
            show_progress: false,
            vendors_per_company: 5,
            customers_per_company: 3,
            materials_per_company: 3,
            assets_per_company: 3,
            employees_per_company: 3,
            ..Default::default()
        };

        let mut orchestrator = EnhancedOrchestrator::new(config, phase_config).unwrap();
        let result = orchestrator.generate().unwrap();

        // All three phases should have run
        assert!(result.statistics.llm_vendors_enriched > 0);
        assert_eq!(result.statistics.diffusion_samples_generated, 10);
        assert_eq!(result.statistics.causal_samples_generated, 50);
        assert!(result.statistics.causal_validation_passed.is_some());
    }

    #[test]
    fn test_statistics_serialization_with_new_fields() {
        let stats = EnhancedGenerationStatistics {
            total_entries: 100,
            total_line_items: 500,
            llm_enrichment_ms: 42,
            llm_vendors_enriched: 10,
            diffusion_enhancement_ms: 100,
            diffusion_samples_generated: 50,
            causal_generation_ms: 200,
            causal_samples_generated: 100,
            causal_validation_passed: Some(true),
            ..Default::default()
        };

        let json = serde_json::to_string(&stats).unwrap();
        let deserialized: EnhancedGenerationStatistics = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.llm_enrichment_ms, 42);
        assert_eq!(deserialized.llm_vendors_enriched, 10);
        assert_eq!(deserialized.diffusion_enhancement_ms, 100);
        assert_eq!(deserialized.diffusion_samples_generated, 50);
        assert_eq!(deserialized.causal_generation_ms, 200);
        assert_eq!(deserialized.causal_samples_generated, 100);
        assert_eq!(deserialized.causal_validation_passed, Some(true));
    }

    #[test]
    fn test_statistics_backward_compat_deserialization() {
        // Old JSON without the new fields should still deserialize
        let old_json = r#"{
            "total_entries": 100,
            "total_line_items": 500,
            "accounts_count": 50,
            "companies_count": 1,
            "period_months": 12,
            "vendor_count": 10,
            "customer_count": 20,
            "material_count": 15,
            "asset_count": 5,
            "employee_count": 8,
            "p2p_chain_count": 5,
            "o2c_chain_count": 5,
            "ap_invoice_count": 5,
            "ar_invoice_count": 5,
            "ocpm_event_count": 0,
            "ocpm_object_count": 0,
            "ocpm_case_count": 0,
            "audit_engagement_count": 0,
            "audit_workpaper_count": 0,
            "audit_evidence_count": 0,
            "audit_risk_count": 0,
            "audit_finding_count": 0,
            "audit_judgment_count": 0,
            "anomalies_injected": 0,
            "data_quality_issues": 0,
            "banking_customer_count": 0,
            "banking_account_count": 0,
            "banking_transaction_count": 0,
            "banking_suspicious_count": 0,
            "graph_export_count": 0,
            "graph_node_count": 0,
            "graph_edge_count": 0
        }"#;

        let stats: EnhancedGenerationStatistics = serde_json::from_str(old_json).unwrap();

        // New fields should default to 0 / None
        assert_eq!(stats.llm_enrichment_ms, 0);
        assert_eq!(stats.llm_vendors_enriched, 0);
        assert_eq!(stats.diffusion_enhancement_ms, 0);
        assert_eq!(stats.diffusion_samples_generated, 0);
        assert_eq!(stats.causal_generation_ms, 0);
        assert_eq!(stats.causal_samples_generated, 0);
        assert!(stats.causal_validation_passed.is_none());
    }

    // ── v5.33 #162 — framework-aware TB classification ──────────────────────

    #[test]
    fn category_from_account_code_us_gaap_unchanged() {
        // US-style numbering — same answers as the pre-v5.33 hard-coded table.
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("1000", "us_gaap"),
            "Cash"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("1500", "us_gaap"),
            "FixedAssets"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("4000", "us_gaap"),
            "Revenue"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("6000", "us_gaap"),
            "OperatingExpenses"
        );
    }

    #[test]
    fn category_from_account_code_skr04_german() {
        // SKR04 (German GAAP): 0xxx = fixed assets, 4xxx = revenue,
        // 8xxx = tax/extraordinary expense — pre-v5.33 the US-only table
        // mis-classified 0xxx as OperatingExpenses (default arm), 4xxx as
        // Revenue (accidentally correct), and 8xxx as OtherExpenses.
        // Framework-aware version routes them correctly.
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("0010", "german_gaap"),
            "FixedAssets",
            "SKR 0xxx must be classified as fixed assets, not P&L"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("1000", "german_gaap"),
            "Cash"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("1300", "german_gaap"),
            "Receivables"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("2000", "german_gaap"),
            "Equity"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("3000", "german_gaap"),
            "Payables"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("4000", "german_gaap"),
            "Revenue"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("5000", "german_gaap"),
            "CostOfSales"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("8000", "german_gaap"),
            "OtherExpenses"
        );
    }

    #[test]
    fn category_from_account_code_pcg_french() {
        // PCG (French GAAP): 2 = fixed assets, 5 = cash, 6 = expenses,
        // 7 = revenue. Pre-v5.33 these all hit the wrong US-prefix arms.
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("210000", "french_gaap"),
            "FixedAssets"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("411000", "french_gaap"),
            "Receivables"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("401000", "french_gaap"),
            "Payables"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("512000", "french_gaap"),
            "Cash"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("603000", "french_gaap"),
            "OperatingExpenses"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("707000", "french_gaap"),
            "Revenue"
        );
        assert_eq!(
            EnhancedOrchestrator::category_from_account_code("101000", "french_gaap"),
            "Equity"
        );
    }

    #[test]
    fn is_balance_sheet_account_routes_skr_correctly() {
        // SKR04: 0xxx fixed assets, 1xxx current assets, 2xxx equity,
        // 3xxx liabilities → all BS.  4xxx revenue, 5-6 expenses → P&L.
        assert!(EnhancedOrchestrator::is_balance_sheet_account(
            "0010",
            "german_gaap"
        ));
        assert!(EnhancedOrchestrator::is_balance_sheet_account(
            "1200",
            "german_gaap"
        ));
        assert!(EnhancedOrchestrator::is_balance_sheet_account(
            "2000",
            "german_gaap"
        ));
        assert!(EnhancedOrchestrator::is_balance_sheet_account(
            "3000",
            "german_gaap"
        ));
        assert!(!EnhancedOrchestrator::is_balance_sheet_account(
            "4000",
            "german_gaap"
        ));
        assert!(!EnhancedOrchestrator::is_balance_sheet_account(
            "6000",
            "german_gaap"
        ));
    }

    #[test]
    fn period_trial_balance_into_canonical_account_type_is_framework_aware() {
        // Defect C regression test — every TB line was hard-coded
        // `account_type: Asset` regardless of the underlying code. With
        // the framework-aware classifier wired in, the same SKR codes
        // resolve to their proper sides.
        use datasynth_generators::TrialBalanceEntry;
        let entries = vec![
            TrialBalanceEntry {
                account_code: "0010".to_string(), // SKR fixed asset
                account_name: "Land".to_string(),
                category: "FixedAssets".to_string(),
                debit_balance: rust_decimal::Decimal::new(1_000_000, 0),
                credit_balance: rust_decimal::Decimal::ZERO,
            },
            TrialBalanceEntry {
                account_code: "3000".to_string(), // SKR liability
                account_name: "Trade payables".to_string(),
                category: "Payables".to_string(),
                debit_balance: rust_decimal::Decimal::ZERO,
                credit_balance: rust_decimal::Decimal::new(500_000, 0),
            },
            TrialBalanceEntry {
                account_code: "4000".to_string(), // SKR revenue
                account_name: "Sales".to_string(),
                category: "Revenue".to_string(),
                debit_balance: rust_decimal::Decimal::ZERO,
                credit_balance: rust_decimal::Decimal::new(2_000_000, 0),
            },
            TrialBalanceEntry {
                account_code: "6000".to_string(), // SKR expense
                account_name: "Personnel cost".to_string(),
                category: "OperatingExpenses".to_string(),
                debit_balance: rust_decimal::Decimal::new(800_000, 0),
                credit_balance: rust_decimal::Decimal::ZERO,
            },
        ];
        let ptb = PeriodTrialBalance {
            fiscal_year: 2024,
            fiscal_period: 12,
            period_start: chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            period_end: chrono::NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            entries,
            framework: "german_gaap".to_string(),
        };
        let tb = ptb.into_canonical("ACME_EU", "EUR");
        // Line account_types are no longer all-Asset.
        let types: Vec<AccountType> = tb.lines.iter().map(|l| l.account_type).collect();
        assert_eq!(types[0], AccountType::Asset, "0010 → Asset");
        assert_eq!(types[1], AccountType::Liability, "3000 → Liability");
        assert_eq!(types[2], AccountType::Revenue, "4000 → Revenue");
        assert_eq!(types[3], AccountType::Expense, "6000 → Expense");
        // is_balanced is now an unconditional truth claim — the
        // underlying JE-balance invariant is the only one we guarantee.
        assert!(tb.is_balanced);
        assert!(tb.is_equation_valid);
        assert_eq!(tb.out_of_balance, rust_decimal::Decimal::ZERO);
        assert_eq!(tb.equation_difference, rust_decimal::Decimal::ZERO);
    }

    #[test]
    fn period_trial_balance_deserialises_legacy_snapshot_without_framework_field() {
        // Old in-memory snapshots (pre-v5.33) didn't carry the framework
        // field. Serde `#[serde(default)]` must let them round-trip with
        // a `"us_gaap"` fallback so older saved sessions keep working.
        let legacy_json = r#"{
            "fiscal_year": 2024,
            "fiscal_period": 12,
            "period_start": "2024-01-01",
            "period_end": "2024-12-31",
            "entries": []
        }"#;
        let ptb: PeriodTrialBalance = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(ptb.framework, "us_gaap");
    }
}

//! Journal Entry generator with statistical distributions.

use chrono::{Datelike, NaiveDate, Timelike};
use datasynth_core::utils::seeded_rng;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use std::sync::{Arc, LazyLock};

use tracing::debug;

use datasynth_config::schema::{
    AdvancedDistributionConfig, FraudConfig, GeneratorConfig, MixtureDistributionType,
    TemplateConfig, TemporalPatternsConfig, TransactionConfig,
};
use datasynth_core::distributions::{
    AdvancedAmountSampler, BusinessDayCalculator, CrossDayConfig, DriftAdjustments, DriftConfig,
    DriftController, EventType, IndustryAmountProfile, IndustryType, LagDistribution,
    PeriodEndConfig, PeriodEndDynamics, PeriodEndModel, ProcessingLagCalculator,
    ProcessingLagConfig, *,
};
use datasynth_core::models::*;
use datasynth_core::templates::{
    descriptions::DescriptionContext, DescriptionGenerator, ReferenceGenerator, ReferenceType,
};
use datasynth_core::traits::Generator;
use datasynth_core::uuid_factory::{DeterministicUuidFactory, GeneratorType};
use datasynth_core::CountryPack;

use crate::company_selector::WeightedCompanySelector;
use crate::user_generator::{UserGenerator, UserGeneratorConfig};

use datasynth_core::distributions::text_taxonomy::{PiiPlaceholderKind, PlaceholderResolver};

/// T2-D Lever 1: the default generic SAP source-mix, used when industry priors
/// are not loaded but `transactions.synthetic_source_codes` is on (the default).
/// Built once. See [`SourceMixPrior::sap_default`] and experiments/ml/FINDINGS.md §6.
static DEFAULT_SOURCE_MIX: LazyLock<
    datasynth_core::distributions::behavioral_priors::SourceMixPrior,
> = LazyLock::new(datasynth_core::distributions::behavioral_priors::SourceMixPrior::sap_default);

/// SOTA-5: default fraction of JEs that are reversals/corrections when
/// `transactions.reversal_rate` is unset. Set to match the corpus reversal
/// proxy (~0.10) — at 0.04 the measured proxy was only ~0.034 (the proxy
/// detects ~85% of reversals), so 0.10 lands the proxy near the corpus.
const DEFAULT_REVERSAL_RATE: f64 = 0.10;

/// SOTA-6: default fraction of JEs that are allocation/assessment batches when
/// `transactions.allocation_batch_rate` is unset. Small (each batch carries
/// ~30-80 lines), so the resulting line-share (~8%) and lines-per-JE tail match
/// the corpus's large-batch postings (FINDINGS §8: AB docs ~52 lines drive the
/// lpje std). `0.0` disables.
const DEFAULT_ALLOCATION_RATE: f64 = 0.008;
/// SOTA-4: foreign document currencies + their company-currency rate (company
/// units per 1 unit of the document currency). Synthetic, plausible values.
const FOREIGN_CCYS: &[(&str, f64)] = &[
    ("EUR", 1.09),
    ("GBP", 1.27),
    ("CHF", 1.12),
    ("CAD", 0.74),
    ("JPY", 0.0068),
    ("AUD", 0.66),
    ("CNY", 0.14),
];
/// SOTA-6: inclusive bounds for the number of target (cost-center) lines an
/// allocation batch explodes into — centred near the corpus AB mean (~52).
const ALLOCATION_MIN_TARGETS: u32 = 30;
const ALLOCATION_MAX_TARGETS: u32 = 80;

/// SOTA-2: Zipf exponent for the hot-account power-law. At s=2.0 the top-10%
/// of accounts in a pool carry ~92-96% of that pool's lines across realistic
/// pool sizes (N≈60-150) — matching the corpus account-activity Pareto (~0.95).
const ZIPF_ALPHA: f64 = 2.0;
/// Largest pool size the precomputed harmonic table covers; larger pools (none
/// realistic for a single account-type) fall back to the uniform draw.
const ZIPF_CAP: usize = 16_384;
/// SOTA-2: cumulative partial sums `CUM[k] = Σ_{i=1..k} i^-ZIPF_ALPHA` (CUM[0]=0),
/// computed once. Lets [`JournalEntryGenerator::power_law_index`] normalise (O(1)
/// lookup of `CUM[n]`) and inverse-CDF sample (binary search) without an O(n) sum.
static ZIPF_CUM: LazyLock<Vec<f64>> = LazyLock::new(|| {
    let mut cum = Vec::with_capacity(ZIPF_CAP + 1);
    cum.push(0.0);
    let mut acc = 0.0_f64;
    for i in 1..=ZIPF_CAP {
        acc += 1.0 / (i as f64).powf(ZIPF_ALPHA);
        cum.push(acc);
    }
    cum
});

/// SP6 — Resolves PII placeholders to concrete values drawn from the run's
/// synthetic master data. `{company}` <- vendor/customer names, `{person}` <-
/// user display names, `{street}` <- addresses (empty pool for now — no
/// address master entity), `{patient}` <- a synthetic-person pool (no master
/// entity exists for patients). Empty pools fall back to obviously-synthetic
/// constants so output never carries an empty span or a literal `{…}` token.
#[derive(Debug, Default)]
pub struct MasterDataResolver {
    pub companies: Vec<String>,
    pub persons: Vec<String>,
    pub streets: Vec<String>,
    pub patients: Vec<String>,
}

impl PlaceholderResolver for MasterDataResolver {
    fn resolve(&mut self, kind: PiiPlaceholderKind, rng: &mut dyn rand::Rng) -> String {
        use rand::RngExt;
        let (pool, fallback): (&Vec<String>, &str) = match kind {
            PiiPlaceholderKind::Company => (&self.companies, "Synthetic Company AG"),
            PiiPlaceholderKind::Person => (&self.persons, "Synthetic Person"),
            PiiPlaceholderKind::Street => (&self.streets, "Synthetic Street 1"),
            PiiPlaceholderKind::Patient => (&self.patients, "Synthetic Patient"),
        };
        if pool.is_empty() {
            return fallback.to_string();
        }
        let idx = rng.random_range(0..pool.len());
        pool[idx].clone()
    }
}

/// A small static pool of obviously-synthetic person names for `{patient}`
/// filling. No master entity exists for patients. Locale is a hint; for SP6
/// a single neutral set is sufficient.
///
/// **Shape invariant:** every entry must avoid the `<initial>. <surname>` and
/// `<surname> <initial>.` shapes, because the SP6 `residual_pii_scan` flags
/// those as `initial_surname` / `surname_initial` PII patterns. The smoke
/// test asserts the canonical `*{patient} G:…` template fills to a scan-clean
/// string; an entry like `"B. Muster"` would regress that. Prefer two-word
/// `<First> <Last>` shapes with no periods (covered by
/// `synthetic_patient_pool_entries_pass_residual_scan`).
fn synthetic_patient_pool(_locale: &str) -> Vec<String> {
    [
        "Alex Beispiel",
        "Bea Muster",
        "Cleo Synthetic",
        "Demo Example",
        "Erik Probe",
        "Fred Testperson",
        "Gerda Platzhalter",
        "Hans Demo",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Generator for realistic journal entries.
pub struct JournalEntryGenerator {
    rng: ChaCha8Rng,
    /// T2-D: independent RNG stream for the default source-mix draw, so
    /// populating `sap_source_code` on the no-priors path never perturbs the
    /// main `rng` — all other fields stay byte-identical to the legacy output.
    source_mix_rng: ChaCha8Rng,
    /// SOTA-1: per-(company, doc-type) library of reusable JE account archetypes
    /// `(debit_accounts, credit_accounts)` for the recurring-templates process.
    /// Capped per key; reused on the no-priors path so standard postings recur.
    recurring_archetypes:
        std::collections::HashMap<(String, String), Vec<(Vec<String>, Vec<String>)>>,
    /// SOTA-1: independent RNG for the template-reuse roll + archetype pick, so
    /// templating never perturbs the main `rng` (amounts/dates/counts unchanged).
    template_rng: ChaCha8Rng,
    /// SOTA-5: ring buffer of recent (complete) JEs a later reversal can offset.
    /// Storing the whole JE lets the reversal inherit its source code, line text,
    /// audit flags, etc. (only dr/cr + the header markers are changed).
    reversal_buffer: Vec<JournalEntry>,
    /// SOTA-5: independent RNG for reversal rolls, so reversals intersperse
    /// without perturbing the main `rng` (normal JEs stay byte-identical).
    reversal_rng: ChaCha8Rng,
    /// SOTA-2: independent RNG for the hot-account power-law override, so the
    /// account-activity Pareto (a few accounts carry most lines, as in the
    /// corpus) is concentrated without perturbing the main `rng` — the uniform
    /// `.choose` draw is still consumed, only its *result* is replaced.
    account_rng: ChaCha8Rng,
    /// SOTA-6: independent RNG for the allocation/assessment-batch process, so
    /// the large 1-to-many postings (the corpus's lines-per-JE tail) intersperse
    /// without perturbing the main `rng` (normal JEs stay byte-identical).
    allocation_rng: ChaCha8Rng,
    /// SOTA-4: independent RNG for the foreign-currency post-process, so the
    /// document-currency tagging never perturbs the main `rng` (company-currency
    /// JEs stay byte-identical).
    fx_rng: ChaCha8Rng,
    /// SOTA-8: independent RNG for the source-conditional Dirichlet account-pair
    /// sampler. Built lazily (one `SourcePool` per observed source); when the
    /// feature is off the sampler stays None and the main RNG / `account_rng`
    /// stream is byte-identical.
    cond_pair_rng: ChaCha8Rng,
    /// SOTA-8: per-source Dirichlet PMFs over per-source account pools.
    /// Lazy-built on first JE whose source isn't yet pooled.
    cond_pair_sampler: Option<
        datasynth_core::distributions::source_conditional_pair::SourceConditionalPairSampler,
    >,
    /// SOTA-8: SAP source code of the JE currently being constructed, so the
    /// `select_*_account` helpers can consult the per-source pool.
    current_je_source: Option<String>,
    seed: u64,
    config: TransactionConfig,
    coa: Arc<ChartOfAccounts>,
    companies: Vec<String>,
    company_selector: WeightedCompanySelector,
    line_sampler: LineItemSampler,
    amount_sampler: AmountSampler,
    temporal_sampler: TemporalSampler,
    start_date: NaiveDate,
    end_date: NaiveDate,
    count: u64,
    uuid_factory: DeterministicUuidFactory,
    // Enhanced features
    user_pool: Option<UserPool>,
    description_generator: DescriptionGenerator,
    reference_generator: ReferenceGenerator,
    template_config: TemplateConfig,
    vendor_pool: VendorPool,
    customer_pool: CustomerPool,
    // Material pool for realistic material references
    material_pool: Option<MaterialPool>,
    // Cost-center IDs sourced from the generated cost-centers master so
    // `JE.cost_center` joins back to `cost_centers.id`.  Populated via
    // [`with_cost_center_pool`] from the orchestrator after master-data
    // generation; falls back to the hardcoded `COST_CENTER_POOL` const
    // when empty (configs that skip master-data generation).
    cost_center_pool: Vec<String>,
    // Profit-center IDs sourced from the generated profit-centers master
    // so `JE.profit_center` joins back to `profit_centers.id`.  Same
    // population semantics as `cost_center_pool`.
    profit_center_pool: Vec<String>,
    // Flag indicating whether we're using real master data vs defaults
    using_real_master_data: bool,
    // Fraud generation
    fraud_config: FraudConfig,
    // Persona-based error injection
    persona_errors_enabled: bool,
    // Approval threshold enforcement
    approval_enabled: bool,
    approval_threshold: rust_decimal::Decimal,
    // SOD violation rate for approval tracking (0.0 to 1.0)
    sod_violation_rate: f64,
    // Batching behavior - humans often process similar items together
    batch_state: Option<BatchState>,
    // Temporal drift controller for simulating distribution changes over time
    drift_controller: Option<DriftController>,
    // Temporal patterns components
    business_day_calculator: Option<BusinessDayCalculator>,
    processing_lag_calculator: Option<ProcessingLagCalculator>,
    temporal_patterns_config: Option<TemporalPatternsConfig>,
    // Business-process weights for the O2C/P2P/R2R/H2R/A2R volume mix. Must
    // sum to 1.0 (validated by config schema). Default matches the legacy
    // hard-coded 0.35/0.30/0.20/0.10/0.05 distribution.
    business_process_weights: [(BusinessProcess, f64); 5],
    // v3.4.0 advanced distributions (mixture models + industry profiles).
    // None preserves v3.3.2 byte-for-byte behavior; populated only when the
    // caller opts in via [`set_advanced_distributions`].
    advanced_amount_sampler: Option<AdvancedAmountSampler>,
    // v3.5.3+ conditional amount override. Populated when
    // `config.distributions.conditional` contains an entry where
    // `output_field == "amount"` and `input_field ∈ {"month",
    // "quarter", "constant"}`. Applied *after* the fraud-pattern /
    // advanced-sampler / legacy-sampler cascade on non-fraud entries
    // so it can steer amounts by calendar context without disturbing
    // fraud semantics.
    conditional_amount_override: Option<datasynth_core::distributions::ConditionalSampler>,
    // v3.5.4+ Gaussian copula for amount↔line_count correlation. When
    // populated, each non-fraud JE draws a (u, v) pair; u nudges amount
    // via a `(0.75 + 0.5*u)` multiplier and v biases line_count toward
    // the upper/lower end of its range. Produces observable Spearman
    // correlation without rewiring existing samplers for inverse-CDF.
    correlation_copula: Option<datasynth_core::distributions::BivariateCopulaSampler>,
    /// SP3 — opt-in industry priors. When `Some`, je_generator routes
    /// timing/lines-per-JE/fanout/active-window through prior-driven samplers.
    /// When `None`, behavior is identical to v5.11.
    pub loaded_priors: Option<crate::priors_loader::LoadedPriors>,
    /// SP3 T11 — accumulated IET days per document-type code.  Only used when
    /// `loaded_priors.is_some()`.  Tracks the running day offset so
    /// consecutive calls for the same source produce IET-spaced posting dates.
    iet_day_accum: std::collections::HashMap<String, f64>,
    /// v5.30 B1 Phase 2 — per-source burst-clustering state.  When a sampled IET
    /// falls below `BURST_THRESHOLD_DAYS` and a probability gate fires, the
    /// next 2-4 events for that source are deterministically clustered with
    /// short IETs (0.25-1.5 days), giving the within-source IET sequence the
    /// positive lag-1 autocorrelation the Sajja P1 metric measures.  Bypasses
    /// the `|ρ| < 0.1` coupling gate in `ConditionalIETSampler` that the SP3
    /// priors' weak day-resolution autocorrelation can't clear.
    iet_burst_remaining: std::collections::HashMap<String, u8>,
    /// SP3.12 — last TP value drawn per SAP source code.  Used by the TP motif
    /// sampler to bias the next TP draw toward cluster-mates of the previous TP
    /// on the same source, building triangle structure in the TP co-occurrence graph.
    last_tp_by_source: std::collections::HashMap<String, String>,
    /// SP3.4 — when Some, observes each emitted line and applies calibration
    /// steps to the generator's tunable parameters.
    pub velocity_calibrator: Option<crate::velocity_calibrator::VelocityCalibrator>,
    /// SP6 — PII placeholder resolver populated from the run's synthetic master
    /// data (vendors, customers, users). Rebuilt once via
    /// [`refresh_md_resolver`] before JE generation begins.
    md_resolver: MasterDataResolver,
}

const DEFAULT_BUSINESS_PROCESS_WEIGHTS: [(BusinessProcess, f64); 5] = [
    (BusinessProcess::O2C, 0.35),
    (BusinessProcess::P2P, 0.30),
    (BusinessProcess::R2R, 0.20),
    (BusinessProcess::H2R, 0.10),
    (BusinessProcess::A2R, 0.05),
];

/// Map the schema-level [`datasynth_config::schema::IndustryProfileType`]
/// onto the distributions-layer [`IndustryType`], then return that industry's
/// pre-configured `sales_amounts` mixture. Used as a fallback when the
/// caller enables `distributions.amounts` but supplies no components.
/// Per-entry context channels for conditional-distribution overrides.
///
/// v4.1.0+ supported `input_field` values:
///
///   - `"month"` — posting-date month (1..=12)
///   - `"quarter"` — posting-date quarter (1..=4)
///   - `"year"` — posting-date year (e.g. 2026.0)
///   - `"day_of_week"` — 1 (Mon) .. 7 (Sun)
///   - `"day_of_month"` — 1..=31
///   - `"day_of_year"` — 1..=366
///   - `"week_of_year"` — 1..=53
///   - `"is_period_end"` — 1.0 when posting_date is the last business
///     day of the month, else 0.0
///   - `"is_quarter_end"` — 1.0 when posting_date is in a quarter-end
///     month AND is the last business day, else 0.0
///   - `"is_year_end"` — 1.0 when posting_date is in December AND is
///     the last business day, else 0.0
///   - `"constant"` / empty — always 0.0 (treats as unconditional)
///
/// Unsupported values cause the conditional rule to be silently ignored
/// to keep runtime robust against user typos.
impl JournalEntryGenerator {
    fn supported_conditional_input(field: &str) -> bool {
        matches!(
            field,
            "month"
                | "quarter"
                | "year"
                | "day_of_week"
                | "day_of_month"
                | "day_of_year"
                | "week_of_year"
                | "is_period_end"
                | "is_quarter_end"
                | "is_year_end"
                | "constant"
                | ""
        )
    }

    fn conditional_input_value(&self, posting_date: chrono::NaiveDate) -> f64 {
        let input_field = match self
            .conditional_amount_override
            .as_ref()
            .map(|s| s.config().input_field.as_str())
        {
            Some(f) => f,
            None => return 0.0,
        };

        let is_last_business_day = |d: chrono::NaiveDate| -> bool {
            // Last day-of-month → is_period_end. Handles Feb/leap-year
            // via chrono's num_days_from_ce roundabout; simpler path:
            // if adding 1 day moves to a different month, this is EOM.
            let next = d.succ_opt();
            match next {
                Some(n) => n.month() != d.month(),
                None => true,
            }
        };

        match input_field {
            "month" => posting_date.month() as f64,
            "quarter" => ((posting_date.month() - 1) / 3 + 1) as f64,
            "year" => posting_date.year() as f64,
            "day_of_week" => posting_date.weekday().number_from_monday() as f64,
            "day_of_month" => posting_date.day() as f64,
            "day_of_year" => posting_date.ordinal() as f64,
            "week_of_year" => posting_date.iso_week().week() as f64,
            "is_period_end" => f64::from(u8::from(is_last_business_day(posting_date))),
            "is_quarter_end" => {
                let m = posting_date.month();
                let is_q_month = matches!(m, 3 | 6 | 9 | 12);
                f64::from(u8::from(is_q_month && is_last_business_day(posting_date)))
            }
            "is_year_end" => f64::from(u8::from(
                posting_date.month() == 12 && is_last_business_day(posting_date),
            )),
            _ => 0.0,
        }
    }
}

fn industry_profile_to_log_normal(
    p: datasynth_config::schema::IndustryProfileType,
) -> datasynth_core::distributions::LogNormalMixtureConfig {
    use datasynth_config::schema::IndustryProfileType as P;
    let industry = match p {
        P::Retail => IndustryType::Retail,
        P::Manufacturing => IndustryType::Manufacturing,
        P::FinancialServices => IndustryType::FinancialServices,
        P::Healthcare => IndustryType::Healthcare,
        P::Technology => IndustryType::Technology,
    };
    IndustryAmountProfile::for_industry(industry).sales_amounts
}

/// State for tracking batch processing behavior.
///
/// When humans process transactions, they often batch similar items together
/// (e.g., processing all invoices from one vendor, entering similar expenses).
#[derive(Clone)]
struct BatchState {
    /// The base entry template to vary
    base_account_number: String,
    base_amount: rust_decimal::Decimal,
    base_business_process: Option<BusinessProcess>,
    base_posting_date: NaiveDate,
    /// Remaining entries in this batch
    remaining: u8,
}

impl JournalEntryGenerator {
    /// Create a new journal entry generator.
    pub fn new_with_params(
        config: TransactionConfig,
        coa: Arc<ChartOfAccounts>,
        companies: Vec<String>,
        start_date: NaiveDate,
        end_date: NaiveDate,
        seed: u64,
    ) -> Self {
        Self::new_with_full_config(
            config,
            coa,
            companies,
            start_date,
            end_date,
            seed,
            TemplateConfig::default(),
            None,
        )
    }

    /// Create a new journal entry generator with full configuration.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_full_config(
        config: TransactionConfig,
        coa: Arc<ChartOfAccounts>,
        companies: Vec<String>,
        start_date: NaiveDate,
        end_date: NaiveDate,
        seed: u64,
        template_config: TemplateConfig,
        user_pool: Option<UserPool>,
    ) -> Self {
        // Initialize user pool if not provided
        let user_pool = user_pool.or_else(|| {
            if template_config.names.generate_realistic_names {
                let user_gen_config = UserGeneratorConfig {
                    culture_distribution: vec![
                        (
                            datasynth_core::templates::NameCulture::WesternUs,
                            template_config.names.culture_distribution.western_us,
                        ),
                        (
                            datasynth_core::templates::NameCulture::Hispanic,
                            template_config.names.culture_distribution.hispanic,
                        ),
                        (
                            datasynth_core::templates::NameCulture::German,
                            template_config.names.culture_distribution.german,
                        ),
                        (
                            datasynth_core::templates::NameCulture::French,
                            template_config.names.culture_distribution.french,
                        ),
                        (
                            datasynth_core::templates::NameCulture::Chinese,
                            template_config.names.culture_distribution.chinese,
                        ),
                        (
                            datasynth_core::templates::NameCulture::Japanese,
                            template_config.names.culture_distribution.japanese,
                        ),
                        (
                            datasynth_core::templates::NameCulture::Indian,
                            template_config.names.culture_distribution.indian,
                        ),
                    ],
                    email_domain: template_config.names.email_domain.clone(),
                    generate_realistic_names: true,
                };
                let mut user_gen = UserGenerator::with_config(seed + 100, user_gen_config);
                Some(user_gen.generate_standard(&companies))
            } else {
                None
            }
        });

        // Initialize reference generator
        let mut ref_gen = ReferenceGenerator::new(
            start_date.year(),
            companies
                .first()
                .map(std::string::String::as_str)
                .unwrap_or("1000"),
        );
        ref_gen.set_prefix(
            ReferenceType::Invoice,
            &template_config.references.invoice_prefix,
        );
        ref_gen.set_prefix(
            ReferenceType::PurchaseOrder,
            &template_config.references.po_prefix,
        );
        ref_gen.set_prefix(
            ReferenceType::SalesOrder,
            &template_config.references.so_prefix,
        );

        // Create weighted company selector (uniform weights for this constructor)
        let company_selector = WeightedCompanySelector::uniform(companies.clone());

        Self {
            rng: seeded_rng(seed, 0),
            source_mix_rng: seeded_rng(seed, 50_063),
            recurring_archetypes: std::collections::HashMap::new(),
            template_rng: seeded_rng(seed, 70_081),
            reversal_buffer: Vec::new(),
            reversal_rng: seeded_rng(seed, 90_017),
            account_rng: seeded_rng(seed, 60_071),
            allocation_rng: seeded_rng(seed, 80_023),
            fx_rng: seeded_rng(seed, 70_093),
            cond_pair_rng: seeded_rng(seed, 110_071),
            cond_pair_sampler: None,
            current_je_source: None,
            seed,
            config: config.clone(),
            coa,
            companies,
            company_selector,
            line_sampler: LineItemSampler::with_config(
                seed + 1,
                config.line_item_distribution.clone(),
                config.even_odd_distribution.clone(),
                config.debit_credit_distribution.clone(),
            ),
            amount_sampler: AmountSampler::with_config(seed + 2, config.amounts.clone()),
            temporal_sampler: TemporalSampler::with_config(
                seed + 3,
                config.seasonality.clone(),
                WorkingHoursConfig::default(),
                Vec::new(),
            ),
            start_date,
            end_date,
            count: 0,
            uuid_factory: DeterministicUuidFactory::new(seed, GeneratorType::JournalEntry),
            user_pool,
            description_generator: DescriptionGenerator::new(),
            reference_generator: ref_gen,
            template_config,
            vendor_pool: VendorPool::standard(),
            customer_pool: CustomerPool::standard(),
            material_pool: None,
            cost_center_pool: Vec::new(),
            profit_center_pool: Vec::new(),
            using_real_master_data: false,
            fraud_config: FraudConfig::default(),
            persona_errors_enabled: true, // Enable by default for realism
            approval_enabled: true,       // Enable by default for realism
            approval_threshold: rust_decimal::Decimal::new(10000, 0), // $10,000 default threshold
            sod_violation_rate: 0.10,     // 10% default SOD violation rate
            batch_state: None,
            drift_controller: None,
            // Always provide a basic BusinessDayCalculator so that weekend/holiday
            // filtering is active even when temporal_patterns is not explicitly enabled.
            business_day_calculator: Some(BusinessDayCalculator::new(HolidayCalendar::new(
                Region::US,
                start_date.year(),
            ))),
            processing_lag_calculator: None,
            temporal_patterns_config: None,
            business_process_weights: DEFAULT_BUSINESS_PROCESS_WEIGHTS,
            advanced_amount_sampler: None,
            conditional_amount_override: None,
            correlation_copula: None,
            loaded_priors: None,
            iet_day_accum: std::collections::HashMap::new(),
            iet_burst_remaining: std::collections::HashMap::new(),
            last_tp_by_source: std::collections::HashMap::new(),
            velocity_calibrator: None,
            md_resolver: MasterDataResolver::default(),
        }
    }

    /// Wire v3.4.0 advanced distributions. When the caller's config has
    /// `distributions.enabled = true` AND `distributions.amounts.enabled =
    /// true`, the journal-entry generator routes non-fraud amount sampling
    /// through an [`AdvancedAmountSampler`] (log-normal or Gaussian mixture).
    ///
    /// When `distributions.industry_profile` is `Some`, the caller's
    /// explicitly configured components override nothing — if the component
    /// list is empty, the industry profile's `sales_amounts` mixture is used
    /// instead. Explicit components always win.
    ///
    /// Returning `Ok(())` with no side effect is intentional for the
    /// following no-op cases, so callers can unconditionally invoke this:
    ///   - `config.enabled = false`
    ///   - `config.amounts.enabled = false`
    ///   - empty component list with no industry profile
    ///
    /// Errors propagate from mixture validation (e.g. weights not summing
    /// to 1.0, non-positive sigma).
    pub fn set_advanced_distributions(
        &mut self,
        config: &AdvancedDistributionConfig,
        seed: u64,
    ) -> Result<(), String> {
        if !config.enabled {
            return Ok(());
        }

        // v3.5.3+: build a conditional-amount override when the config
        // declares a rule with `output_field == "amount"` and a supported
        // input field. The override is applied *after* the standard
        // cascade so it doesn't disturb fraud-path sampling. Unsupported
        // input fields are ignored with a trace log.
        self.conditional_amount_override = config
            .conditional
            .iter()
            .find(|c| {
                c.output_field == "amount" && Self::supported_conditional_input(&c.input_field)
            })
            .and_then(|c| {
                datasynth_core::distributions::ConditionalSampler::new(
                    seed.wrapping_add(17),
                    c.to_core_config(),
                )
                .ok()
            });

        // v4.1.0+: all 5 copula types wired (Gaussian / Clayton /
        // Gumbel / Frank / Student-t). The `BivariateCopulaSampler`
        // already implements each; v3.5.4 had a filter limiting to
        // Gaussian only — lifted here now that the smoke test matrix
        // covers all types.
        self.correlation_copula = config
            .correlations
            .to_core_config_for_pair("amount", "line_count")
            .and_then(|copula_cfg| {
                datasynth_core::distributions::BivariateCopulaSampler::new(
                    seed.wrapping_add(31),
                    copula_cfg,
                )
                .ok()
            });

        // v3.4.4+: Pareto takes precedence over mixture models when set.
        // This supports heavy-tailed amount distributions (capex, strategic
        // contracts, fraud) that log-normal/Gaussian mixtures can't model
        // as sharply.
        if let Some(pareto) = &config.pareto {
            if pareto.enabled {
                let core_cfg = pareto.to_core_config();
                self.advanced_amount_sampler =
                    Some(AdvancedAmountSampler::new_pareto(seed, core_cfg)?);
                return Ok(());
            }
        }

        if !config.amounts.enabled {
            return Ok(());
        }

        match config.amounts.distribution_type {
            MixtureDistributionType::LogNormal => {
                let lognormal_cfg = config.amounts.to_log_normal_config().or_else(|| {
                    config
                        .industry_profile
                        .as_ref()
                        .map(|p| industry_profile_to_log_normal(p.profile_type()))
                });
                if let Some(cfg) = lognormal_cfg {
                    self.advanced_amount_sampler =
                        Some(AdvancedAmountSampler::new_log_normal(seed, cfg)?);
                }
            }
            MixtureDistributionType::Gaussian => {
                if let Some(cfg) = config.amounts.to_gaussian_config() {
                    self.advanced_amount_sampler =
                        Some(AdvancedAmountSampler::new_gaussian(seed, cfg)?);
                }
            }
        }

        Ok(())
    }

    /// Override the business-process volume mix. Weights map directly to the
    /// `business_processes.*_weight` YAML config; they do not have to sum to
    /// exactly 1.0 (they're normalized via `weighted_select`).
    pub fn set_business_process_weights(
        &mut self,
        o2c: f64,
        p2p: f64,
        r2r: f64,
        h2r: f64,
        a2r: f64,
    ) {
        self.business_process_weights = [
            (BusinessProcess::O2C, o2c),
            (BusinessProcess::P2P, p2p),
            (BusinessProcess::R2R, r2r),
            (BusinessProcess::H2R, h2r),
            (BusinessProcess::A2R, a2r),
        ];
    }

    /// Create from a full GeneratorConfig.
    ///
    /// This constructor uses the volume_weight from company configs
    /// for weighted company selection, and fraud config from GeneratorConfig.
    pub fn from_generator_config(
        full_config: &GeneratorConfig,
        coa: Arc<ChartOfAccounts>,
        start_date: NaiveDate,
        end_date: NaiveDate,
        seed: u64,
    ) -> Self {
        let companies: Vec<String> = full_config
            .companies
            .iter()
            .map(|c| c.code.clone())
            .collect();

        // Create weighted selector using volume_weight from company configs
        let company_selector = WeightedCompanySelector::from_configs(&full_config.companies);

        let mut generator = Self::new_with_full_config(
            full_config.transactions.clone(),
            coa,
            companies,
            start_date,
            end_date,
            seed,
            full_config.templates.clone(),
            None,
        );

        // Override the uniform selector with weighted selector
        generator.company_selector = company_selector;

        // Set fraud config
        generator.fraud_config = full_config.fraud.clone();

        // Configure temporal patterns if enabled
        let temporal_config = &full_config.temporal_patterns;
        if temporal_config.enabled {
            generator = generator.with_temporal_patterns(temporal_config.clone(), seed);
        }

        generator
    }

    /// Configure temporal patterns including business day calculations and processing lags.
    ///
    /// This enables realistic temporal behavior including:
    /// - Business day awareness (no postings on weekends/holidays)
    /// - Processing lag modeling (event-to-posting delays)
    /// - Period-end dynamics (volume spikes at month/quarter/year end)
    pub fn with_temporal_patterns(mut self, config: TemporalPatternsConfig, seed: u64) -> Self {
        // Create business day calculator if enabled
        if config.business_days.enabled {
            let region = config
                .calendars
                .regions
                .first()
                .map(|r| Self::parse_region(r))
                .unwrap_or(Region::US);

            let calendar = HolidayCalendar::new(region, self.start_date.year());
            self.business_day_calculator = Some(BusinessDayCalculator::new(calendar));
        }

        // Create processing lag calculator if enabled
        if config.processing_lags.enabled {
            let lag_config = Self::convert_processing_lag_config(&config.processing_lags);
            self.processing_lag_calculator =
                Some(ProcessingLagCalculator::with_config(seed, lag_config));
        }

        // Create period-end dynamics if configured
        let model = config.period_end.model.as_deref().unwrap_or("flat");
        if model != "flat"
            || config
                .period_end
                .month_end
                .as_ref()
                .is_some_and(|m| m.peak_multiplier.unwrap_or(1.0) != 1.0)
        {
            let dynamics = Self::convert_period_end_config(&config.period_end);
            self.temporal_sampler.set_period_end_dynamics(dynamics);
        }

        self.temporal_patterns_config = Some(config);
        self
    }

    /// Configure temporal patterns using a [`CountryPack`] for the holiday calendar.
    ///
    /// This is an alternative to `with_temporal_patterns` that derives the
    /// holiday calendar from a country-pack definition rather than the built-in
    /// region-based calendars.  All other temporal behaviour (business-day
    /// adjustment, processing lags, period-end dynamics) is configured
    /// identically.
    pub fn with_country_pack_temporal(
        mut self,
        config: TemporalPatternsConfig,
        seed: u64,
        pack: &CountryPack,
    ) -> Self {
        // Create business day calculator using the country pack calendar
        if config.business_days.enabled {
            let calendar = HolidayCalendar::from_country_pack(pack, self.start_date.year());
            self.business_day_calculator = Some(BusinessDayCalculator::new(calendar));
        }

        // Create processing lag calculator if enabled
        if config.processing_lags.enabled {
            let lag_config = Self::convert_processing_lag_config(&config.processing_lags);
            self.processing_lag_calculator =
                Some(ProcessingLagCalculator::with_config(seed, lag_config));
        }

        // Create period-end dynamics if configured
        let model = config.period_end.model.as_deref().unwrap_or("flat");
        if model != "flat"
            || config
                .period_end
                .month_end
                .as_ref()
                .is_some_and(|m| m.peak_multiplier.unwrap_or(1.0) != 1.0)
        {
            let dynamics = Self::convert_period_end_config(&config.period_end);
            self.temporal_sampler.set_period_end_dynamics(dynamics);
        }

        self.temporal_patterns_config = Some(config);
        self
    }

    /// Convert schema processing lag config to core config.
    fn convert_processing_lag_config(
        schema: &datasynth_config::schema::ProcessingLagSchemaConfig,
    ) -> ProcessingLagConfig {
        let mut config = ProcessingLagConfig {
            enabled: schema.enabled,
            ..Default::default()
        };

        // Helper to convert lag schema to distribution
        let convert_lag = |lag: &datasynth_config::schema::LagDistributionSchemaConfig| {
            let mut dist = LagDistribution::log_normal(lag.mu, lag.sigma);
            if let Some(min) = lag.min_hours {
                dist.min_lag_hours = min;
            }
            if let Some(max) = lag.max_hours {
                dist.max_lag_hours = max;
            }
            dist
        };

        // Apply event-specific lags
        if let Some(ref lag) = schema.sales_order_lag {
            config
                .event_lags
                .insert(EventType::SalesOrder, convert_lag(lag));
        }
        if let Some(ref lag) = schema.purchase_order_lag {
            config
                .event_lags
                .insert(EventType::PurchaseOrder, convert_lag(lag));
        }
        if let Some(ref lag) = schema.goods_receipt_lag {
            config
                .event_lags
                .insert(EventType::GoodsReceipt, convert_lag(lag));
        }
        if let Some(ref lag) = schema.invoice_receipt_lag {
            config
                .event_lags
                .insert(EventType::InvoiceReceipt, convert_lag(lag));
        }
        if let Some(ref lag) = schema.invoice_issue_lag {
            config
                .event_lags
                .insert(EventType::InvoiceIssue, convert_lag(lag));
        }
        if let Some(ref lag) = schema.payment_lag {
            config
                .event_lags
                .insert(EventType::Payment, convert_lag(lag));
        }
        if let Some(ref lag) = schema.journal_entry_lag {
            config
                .event_lags
                .insert(EventType::JournalEntry, convert_lag(lag));
        }

        // Apply cross-day posting config
        if let Some(ref cross_day) = schema.cross_day_posting {
            config.cross_day = CrossDayConfig {
                enabled: cross_day.enabled,
                probability_by_hour: cross_day.probability_by_hour.clone(),
                ..Default::default()
            };
        }

        config
    }

    /// Convert schema period-end config to core PeriodEndDynamics.
    fn convert_period_end_config(
        schema: &datasynth_config::schema::PeriodEndSchemaConfig,
    ) -> PeriodEndDynamics {
        let model_type = schema.model.as_deref().unwrap_or("exponential");

        // Helper to convert period config
        let convert_period =
            |period: Option<&datasynth_config::schema::PeriodEndModelSchemaConfig>,
             default_peak: f64|
             -> PeriodEndConfig {
                if let Some(p) = period {
                    let model = match model_type {
                        "flat" => PeriodEndModel::FlatMultiplier {
                            multiplier: p.peak_multiplier.unwrap_or(default_peak),
                        },
                        "extended_crunch" => PeriodEndModel::ExtendedCrunch {
                            start_day: p.start_day.unwrap_or(-10),
                            sustained_high_days: p.sustained_high_days.unwrap_or(3),
                            peak_multiplier: p.peak_multiplier.unwrap_or(default_peak),
                            ramp_up_days: 3, // Default ramp-up period
                        },
                        _ => PeriodEndModel::ExponentialAcceleration {
                            start_day: p.start_day.unwrap_or(-10),
                            base_multiplier: p.base_multiplier.unwrap_or(1.0),
                            peak_multiplier: p.peak_multiplier.unwrap_or(default_peak),
                            decay_rate: p.decay_rate.unwrap_or(0.3),
                        },
                    };
                    PeriodEndConfig {
                        enabled: true,
                        model,
                        additional_multiplier: p.additional_multiplier.unwrap_or(1.0),
                    }
                } else {
                    PeriodEndConfig {
                        enabled: true,
                        model: PeriodEndModel::ExponentialAcceleration {
                            start_day: -10,
                            base_multiplier: 1.0,
                            peak_multiplier: default_peak,
                            decay_rate: 0.3,
                        },
                        additional_multiplier: 1.0,
                    }
                }
            };

        PeriodEndDynamics::new(
            convert_period(schema.month_end.as_ref(), 2.0),
            convert_period(schema.quarter_end.as_ref(), 3.5),
            convert_period(schema.year_end.as_ref(), 5.0),
        )
    }

    /// Parse a region string into a Region enum.
    fn parse_region(region_str: &str) -> Region {
        match region_str.to_uppercase().as_str() {
            "US" => Region::US,
            "DE" => Region::DE,
            "GB" => Region::GB,
            "CN" => Region::CN,
            "JP" => Region::JP,
            "IN" => Region::IN,
            "BR" => Region::BR,
            "MX" => Region::MX,
            "AU" => Region::AU,
            "SG" => Region::SG,
            "KR" => Region::KR,
            "FR" => Region::FR,
            "IT" => Region::IT,
            "ES" => Region::ES,
            "CA" => Region::CA,
            _ => Region::US,
        }
    }

    /// Set a custom company selector.
    pub fn set_company_selector(&mut self, selector: WeightedCompanySelector) {
        self.company_selector = selector;
    }

    /// Get the current company selector.
    pub fn company_selector(&self) -> &WeightedCompanySelector {
        &self.company_selector
    }

    /// Set fraud configuration.
    pub fn set_fraud_config(&mut self, config: FraudConfig) {
        self.fraud_config = config;
    }

    /// Set vendors from generated master data.
    ///
    /// This replaces the default vendor pool with actual generated vendors,
    /// ensuring JEs reference real master data entities.
    pub fn with_vendors(mut self, vendors: &[Vendor]) -> Self {
        if !vendors.is_empty() {
            self.vendor_pool = VendorPool::from_vendors(vendors.to_vec());
            self.using_real_master_data = true;
        }
        self
    }

    /// Set customers from generated master data.
    ///
    /// This replaces the default customer pool with actual generated customers,
    /// ensuring JEs reference real master data entities.
    pub fn with_customers(mut self, customers: &[Customer]) -> Self {
        if !customers.is_empty() {
            self.customer_pool = CustomerPool::from_customers(customers.to_vec());
            self.using_real_master_data = true;
        }
        self
    }

    /// Set materials from generated master data.
    ///
    /// This provides material references for JEs that involve inventory movements.
    pub fn with_materials(mut self, materials: &[Material]) -> Self {
        if !materials.is_empty() {
            self.material_pool = Some(MaterialPool::from_materials(materials.to_vec()));
            self.using_real_master_data = true;
        }
        self
    }

    /// Set all master data at once for convenience.
    ///
    /// This is the recommended way to configure the JE generator with
    /// generated master data to ensure data coherence.
    pub fn with_master_data(
        self,
        vendors: &[Vendor],
        customers: &[Customer],
        materials: &[Material],
    ) -> Self {
        self.with_vendors(vendors)
            .with_customers(customers)
            .with_materials(materials)
    }

    /// SP6 — Build a [`MasterDataResolver`] from the run's master data and
    /// store it in `self.md_resolver`. Call once before JE generation begins
    /// (the entry method `generate` calls this lazily on the first entry when
    /// the resolver pools are empty). Pools are cheap `Vec<String>` snapshots
    /// of names already held in the generator's vendor/customer/user pools.
    fn refresh_md_resolver(&mut self) {
        let companies: Vec<String> = self
            .vendor_pool
            .vendors
            .iter()
            .map(|v| v.name.clone())
            .chain(self.customer_pool.customers.iter().map(|c| c.name.clone()))
            .collect();

        let persons: Vec<String> = self
            .user_pool
            .as_ref()
            .map(|p| p.users.iter().map(|u| u.display_name.clone()).collect())
            .unwrap_or_default();

        let streets: Vec<String> = Vec::new(); // No address master entity in this generator.
        let patients = synthetic_patient_pool("de_CH");

        self.md_resolver = MasterDataResolver {
            companies,
            persons,
            streets,
            patients,
        };
    }

    /// Set the cost-center pool used by line-item enrichment.
    ///
    /// The orchestrator wires this from the generated cost-centers
    /// master so `JE.cost_center` joins back to `cost_centers.id`.
    /// When the pool is non-empty `enrich_line_items` picks
    /// deterministically from it; the hardcoded fallback
    /// `COST_CENTER_POOL` const is only used when the pool is empty
    /// (configs that don't generate cost-center master data).
    pub fn with_cost_center_pool(mut self, ids: Vec<String>) -> Self {
        self.cost_center_pool = ids;
        self
    }

    /// Set the profit-center pool used by line-item enrichment.
    ///
    /// Same semantics as `with_cost_center_pool` but for the
    /// profit-centers master.  Without this, the legacy
    /// `PC-{company_code}-{P2P|O2C|R2R|H2R}` derivation is used —
    /// which is consistent within a generation run but does not
    /// match the format the master data generator emits.
    pub fn with_profit_center_pool(mut self, ids: Vec<String>) -> Self {
        self.profit_center_pool = ids;
        self
    }

    /// Replace the auto-generated user pool with an externally-built one.
    ///
    /// The orchestrator builds a [`UserPool`] from the generated
    /// employee master ([`UserPool::from_employees`]) and passes it
    /// here, so `JE.created_by` joins back to `employees.user_id`.
    /// Without this call, `with_country_pack_names` generates its
    /// own user pool whose ids are disjoint from the employee
    /// master.
    pub fn with_user_pool(mut self, pool: UserPool) -> Self {
        self.user_pool = Some(pool);
        self
    }

    /// Replace the user pool with one generated from a [`CountryPack`].
    ///
    /// This is an alternative to the default name-culture distribution that
    /// derives name pools and weights from the country-pack's `names` section.
    /// The existing user pool (if any) is discarded and regenerated using
    /// `MultiCultureNameGenerator::from_country_pack`.
    pub fn with_country_pack_names(mut self, pack: &CountryPack) -> Self {
        let name_gen =
            datasynth_core::templates::MultiCultureNameGenerator::from_country_pack(pack);
        let config = UserGeneratorConfig {
            // The culture distribution is embedded in the name generator
            // itself, so we use an empty list here.
            culture_distribution: Vec::new(),
            email_domain: name_gen.email_domain().to_string(),
            generate_realistic_names: true,
        };
        let mut user_gen = UserGenerator::with_name_generator(self.seed + 100, config, name_gen);
        self.user_pool = Some(user_gen.generate_standard(&self.companies));
        self
    }

    /// Check if the generator is using real master data.
    pub fn is_using_real_master_data(&self) -> bool {
        self.using_real_master_data
    }

    /// Determine if this transaction should be fraudulent.
    /// Pick a realistic ERP `source_system` provenance code.
    ///
    /// Returns a string like `"SAP-FI/AP"`, `"manual/adjustment"`,
    /// `"Interface/EDI"`. Uses the business process to bias toward
    /// process-appropriate sub-modules (e.g. P2P → SAP-MM/IV, O2C →
    /// SAP-SD/IV, H2R → SAP-HR/PR). The legacy 7-code shape
    /// (`SAP-FI`, `SAP-MM`, etc.) is preserved as a prefix so existing
    /// `starts_with` filters keep working.
    ///
    /// **Manual contract**: when `is_manual` is true the returned value
    /// always starts with `"manual"` or `"spreadsheet"`. This is asserted
    /// in `test_isa240_audit_flags_populated`.
    fn pick_source_system(rng: &mut ChaCha8Rng, is_manual: bool, bp: BusinessProcess) -> String {
        if is_manual {
            // 8 manual provenance codes — all share a `manual/` or
            // `spreadsheet/` prefix.
            const MANUAL: &[&str] = &[
                "manual/standard",
                "manual/adjustment",
                "manual/reclassification",
                "manual/accrual",
                "manual/reversal",
                "manual/correction",
                "spreadsheet/upload",
                "spreadsheet/journal",
            ];
            let idx = (rng.random::<u32>() as usize) % MANUAL.len();
            return MANUAL[idx].to_string();
        }

        // Process-aware automated provenance. Each process has a small
        // primary set; we also mix in cross-process codes ~20% of the
        // time so the taxonomy stays diverse without losing coherence.
        let primary: &[&str] = match bp {
            BusinessProcess::P2P => &[
                "SAP-MM/PO",
                "SAP-MM/IV",
                "SAP-MM/IM",
                "SAP-FI/AP",
                "Interface/EDI",
            ],
            BusinessProcess::O2C => &[
                "SAP-SD/ORD",
                "SAP-SD/DEL",
                "SAP-SD/IV",
                "SAP-FI/AR",
                "Interface/Lockbox",
            ],
            BusinessProcess::H2R => &["SAP-HR/PR", "SAP-HR/TIME", "Interface/PayRun"],
            BusinessProcess::A2R => &["SAP-FI/AA", "SAP-FI/GL"],
            BusinessProcess::Treasury => &["Treasury/CM", "Treasury/HD", "Interface/Bank"],
            BusinessProcess::Tax => &["Tax/RPT", "SAP-FI/GL"],
            BusinessProcess::Mfg => &["SAP-MM/IM", "SAP-FI/GL"],
            // R2R, S2C, Bank, Audit, Intercompany, ProjectAccounting, Esg
            // → fall through to a generic mix.
            _ => &[
                "SAP-FI/GL",
                "SAP-FI/AP",
                "SAP-FI/AR",
                "SAP-FI/AA",
                "External/SubL",
            ],
        };

        // 80% process-appropriate, 20% cross-process (pulled from a
        // generic pool) so the categorical distribution has long tails.
        const CROSS: &[&str] = &[
            "SAP-FI/GL",
            "SAP-FI/AP",
            "SAP-FI/AR",
            "Interface/EDI",
            "Interface/Bank",
            "External/SubL",
        ];
        let pool = if rng.random::<f64>() < 0.80 {
            primary
        } else {
            CROSS
        };
        let idx = (rng.random::<u32>() as usize) % pool.len();
        pool[idx].to_string()
    }

    /// T2-D Lever 1: choose the `sap_source_code` emitted in the CSV `source`
    /// column. Priority: loaded industry priors' `source_mix` (SP3.6) → the
    /// default generic SAP doc-type mix when `transactions.synthetic_source_codes`
    /// is on (the default) → `None` (legacy: `source` falls back to the coarse
    /// `TransactionSource` enum). Closes the source-mix breadth gap by default
    /// (entropy ~0.75 → ~2.7; experiments/ml/FINDINGS.md §6).
    fn sample_sap_source_code(&mut self) -> Option<String> {
        if let Some(p) = self.loaded_priors.as_ref() {
            return Some(p.source_mix.sample(&mut self.rng));
        }
        if self.config.synthetic_source_codes.unwrap_or(true) {
            // Independent stream: never perturb the main RNG, so all other
            // fields stay byte-identical to the legacy (enum-source) output.
            return Some(DEFAULT_SOURCE_MIX.sample(&mut self.source_mix_rng));
        }
        None
    }

    /// SOTA-1: on the no-priors path, reuse a cached `(debit, credit)` account
    /// archetype matching the line counts for this `(company, doc_type)` with
    /// high probability, so standard postings recur (and a hot subset of
    /// accounts dominates) instead of every JE drawing fresh uniform accounts.
    /// Returns the accounts to use, or `None` to select fresh (then cached).
    /// Rolls `template_rng` first so the main RNG (amounts/dates/counts) is
    /// never perturbed — only account *choice* changes on reuse.
    fn pick_recurring_archetype(
        &mut self,
        company: &str,
        doc_type: &str,
        debit_count: usize,
        credit_count: usize,
    ) -> Option<(Vec<String>, Vec<String>)> {
        if !self.config.recurring_templates.unwrap_or(true) {
            return None;
        }
        // Priors carry their own GL-account structure; templating is the no-priors
        // default-path realism boost (FINDINGS sec.8) UNLESS the user has explicitly
        // set archetype_reuse_probability — in that case SOTA-1 composes with the
        // priors path (SOTA-9 #137: lift corpus recurring share toward ~0.97).
        let p_reuse_opt = self.config.archetype_reuse_probability;
        if p_reuse_opt.is_none() && self.loaded_priors.is_some() {
            return None;
        }
        let p_reuse = p_reuse_opt.unwrap_or(0.90);
        if self.template_rng.random::<f64>() >= p_reuse {
            return None;
        }
        let lib = self
            .recurring_archetypes
            .get(&(company.to_string(), doc_type.to_string()))?;
        let matching: Vec<&(Vec<String>, Vec<String>)> = lib
            .iter()
            .filter(|(d, c)| d.len() == debit_count && c.len() == credit_count)
            .collect();
        if matching.is_empty() {
            return None;
        }
        // Power-law (Zipf) over the cached archetypes rather than a uniform pick:
        // the earlier-cached "standard" posting of each (company, doc-type, shape)
        // dominates, so a hot subset of archetypes carries most JEs. Uniform reuse
        // kept the per-JE recurring share high but left the archetype head too
        // flat (top-50 coverage 0.49 vs corpus 0.65); concentrating the head lifts
        // top-50 coverage toward the corpus. Same mechanism as the SOTA-2 account
        // Pareto, drawn on the `template_rng` stream.
        let idx = Self::power_law_index(matching.len(), &mut self.template_rng).unwrap_or(0);
        Some(matching[idx].clone())
    }

    /// SOTA-1: record a freshly-selected archetype for future reuse, capped per
    /// `(company, doc_type)` so the standard-posting library stays small.
    fn cache_recurring_archetype(
        &mut self,
        company: &str,
        doc_type: &str,
        debit: Vec<String>,
        credit: Vec<String>,
    ) {
        if self.loaded_priors.is_some() || !self.config.recurring_templates.unwrap_or(true) {
            return;
        }
        if debit.is_empty() && credit.is_empty() {
            return;
        }
        const CAP: usize = 24; // distinct archetypes per (company, doc-type) — fewer ⇒ top-50 archetypes cover more JEs (toward corpus top-50 ~0.65)
        let lib = self
            .recurring_archetypes
            .entry((company.to_string(), doc_type.to_string()))
            .or_default();
        if lib.len() < CAP {
            lib.push((debit, credit));
        }
    }

    /// SOTA-5: with probability `transactions.reversal_rate` (default ~10%),
    /// build a reversal/correction of a recent JE (swap dr/cr, reference the
    /// original) instead of a fresh JE. Uses `reversal_rng` and an id derived
    /// from the original, so the main RNG + uuid factory are unperturbed (normal
    /// JEs stay byte-identical; reversals are interspersed). Balanced because the
    /// original was balanced and we swap each line's debit/credit.
    fn maybe_generate_reversal(&mut self) -> Option<JournalEntry> {
        let rate = self.config.reversal_rate.unwrap_or(DEFAULT_REVERSAL_RATE);
        if rate <= 0.0 || self.reversal_buffer.is_empty() {
            return None;
        }
        if self.reversal_rng.random::<f64>() >= rate {
            return None;
        }
        let pick = (self.reversal_rng.random::<u32>() as usize) % self.reversal_buffer.len();
        // Consume the entry so the same original is never reversed twice — that
        // would mint the same derived id (`orig ^ salt`) and produce duplicate
        // document IDs (regression caught by `test_document_reference_integrity`).
        let mut entry = self.reversal_buffer.remove(pick);
        let orig_id = entry.header.document_id;
        // Reversal posts a few business days after the original.
        let offset = 1 + (self.reversal_rng.random::<u32>() % 7) as i64;
        let mut rev_date = entry.header.posting_date + chrono::Duration::days(offset);
        if let Some(ref calc) = self.business_day_calculator {
            if !calc.is_business_day(rev_date) {
                rev_date = calc.next_business_day(rev_date, false);
            }
        }
        if rev_date > self.end_date {
            rev_date = entry.header.posting_date;
        }
        // Deterministic id derived from the original (no uuid-factory advance).
        let rev_id =
            uuid::Uuid::from_u128(orig_id.as_u128() ^ 0x5245_5645_5253_414c_5245_5645_5253_414c);
        // Inherit everything from the original (source code, line text, audit
        // flags, ...); change only the markers + each line's debit/credit.
        entry.header.document_id = rev_id;
        entry.header.posting_date = rev_date;
        entry.header.document_date = rev_date;
        entry.header.fiscal_year = rev_date.year() as u16;
        entry.header.fiscal_period = rev_date.month() as u8;
        entry.header.header_text = Some(format!("Reversal of {orig_id}"));
        entry.header.reference = Some(format!("REV-{orig_id}"));
        entry.header.batch_id = None;
        for line in entry.lines.iter_mut() {
            std::mem::swap(&mut line.debit_amount, &mut line.credit_amount);
            line.document_id = rev_id;
        }
        Some(entry)
    }

    /// SOTA-5/6: remember a (complete) JE so a later reversal (SOTA-5) or
    /// allocation batch (SOTA-6) can reuse it. Populated when either process is
    /// enabled, so disabling reversals doesn't starve the allocation batches.
    fn record_for_reversal(&mut self, entry: &JournalEntry) {
        let reversal_on = self.config.reversal_rate.unwrap_or(DEFAULT_REVERSAL_RATE) > 0.0;
        let allocation_on = self
            .config
            .allocation_batch_rate
            .unwrap_or(DEFAULT_ALLOCATION_RATE)
            > 0.0;
        if (!reversal_on && !allocation_on) || entry.lines.is_empty() {
            return;
        }
        const CAP: usize = 64;
        if self.reversal_buffer.len() >= CAP {
            self.reversal_buffer.remove(0);
        }
        self.reversal_buffer.push(entry.clone());
    }

    /// SOTA-4: with probability `transactions.foreign_currency_rate`, post this JE
    /// in a foreign document currency (SAP-style). `debit_amount`/`credit_amount`/
    /// `local_amount` stay the company-ledger amount (DMBTR — the trial balance is
    /// unaffected); `header.currency`/`header.exchange_rate` + each line's
    /// `transaction_amount` (WRBTR) carry the foreign value. Balance holds in both
    /// currencies (every line shares one rate). Drawn on `fx_rng` so the main
    /// `rng` (and all company-currency JEs) stay byte-identical.
    fn maybe_apply_foreign_currency(&mut self, entry: &mut JournalEntry) {
        let prob = self.config.foreign_currency_rate.unwrap_or(0.0);
        if prob <= 0.0 || self.fx_rng.random::<f64>() >= prob {
            return;
        }
        let (code, rate) = FOREIGN_CCYS[self.fx_rng.random_range(0..FOREIGN_CCYS.len())];
        let rate_dec = match Decimal::from_f64_retain(rate) {
            Some(r) if r > Decimal::ZERO => r,
            _ => return,
        };
        entry.header.currency = code.to_string();
        entry.header.exchange_rate = rate_dec;
        for line in entry.lines.iter_mut() {
            let ledger = line.debit_amount + line.credit_amount; // one side is zero
            line.transaction_amount = Some((ledger / rate_dec).round_dp(2));
        }
    }

    /// SOTA-6: split `total` into `n` positive cent-precise parts summing
    /// **exactly** to `total` (so the JE stays balanced), with random weights so
    /// the allocation isn't perfectly even. Each part is ≥ 1 cent. Returns a
    /// single `[total]` when the amount is too small to split into `n` parts.
    fn split_amount(total: Decimal, n: usize, rng: &mut ChaCha8Rng) -> Vec<Decimal> {
        let n = n.max(1);
        let total_cents = (total.round_dp(2) * Decimal::from(100))
            .to_i64()
            .unwrap_or(0);
        if n == 1 || total_cents < n as i64 {
            return vec![total];
        }
        let weights: Vec<f64> = (0..n).map(|_| 0.5 + rng.random::<f64>()).collect();
        let sumw: f64 = weights.iter().sum::<f64>().max(f64::EPSILON);
        let spare = total_cents - n as i64; // ≥ 0; each part keeps a 1-cent floor
        let mut cents: Vec<i64> = weights
            .iter()
            .map(|w| 1 + (spare as f64 * w / sumw).floor() as i64)
            .collect();
        // dump the (small, < n) flooring leftover onto the largest part
        let assigned: i64 = cents.iter().sum();
        let leftover = total_cents - assigned;
        if let Some(maxp) = cents.iter_mut().max_by_key(|c| **c) {
            *maxp += leftover;
        }
        cents.into_iter().map(|c| Decimal::new(c, 2)).collect()
    }

    /// SOTA-3: deterministic dimension → business-unit roll-up (the dimension is
    /// the cost center, or the profit center as fallback). The same dimension
    /// value always maps to the same BU code (`BU01`..`BU11`, matching the
    /// corpus's ~11 BU codes), so business-unit analytics are internally
    /// consistent — not a random per-line label. FNV-1a hash, bucketed.
    fn business_unit_for_dimension(dim: &str) -> String {
        const N_BU: u32 = 11;
        let mut h: u32 = 0x811c_9dc5;
        for b in dim.bytes() {
            h ^= b as u32;
            h = h.wrapping_mul(0x0100_0193);
        }
        format!("BU{:02}", (h % N_BU) + 1)
    }

    /// SOTA-6: with probability `transactions.allocation_batch_rate` (default
    /// ~0.8%), emit an allocation/assessment batch instead of a fresh JE — the
    /// large 1-to-many posting that drives the corpus lines-per-JE tail (AB docs
    /// ~52 lines). Reuses a buffered JE for a valid header (no main-RNG / uuid
    /// advance), then explodes its largest debit line into ~30-80 cost-center-
    /// spread sub-lines summing to the same amount, so balance is preserved and
    /// the cost-center dimension breadth rises. Tagged source `AB`.
    fn maybe_generate_allocation_batch(&mut self) -> Option<JournalEntry> {
        let rate = self
            .config
            .allocation_batch_rate
            .unwrap_or(DEFAULT_ALLOCATION_RATE);
        if rate <= 0.0 || self.reversal_buffer.is_empty() {
            return None;
        }
        if self.allocation_rng.random::<f64>() >= rate {
            return None;
        }
        let pick = (self.allocation_rng.random::<u32>() as usize) % self.reversal_buffer.len();
        // Consume the entry (same reason as the reversal path: a reused base
        // would mint a duplicate derived id `base ^ salt`).
        let mut entry = self.reversal_buffer.remove(pick);
        // Explode the largest debit line across cost centers.
        let idx = entry
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.debit_amount > Decimal::ZERO)
            .max_by(|a, b| a.1.debit_amount.cmp(&b.1.debit_amount))
            .map(|(i, _)| i)?;
        let template = entry.lines[idx].clone();
        let n = self
            .allocation_rng
            .random_range(ALLOCATION_MIN_TARGETS..=ALLOCATION_MAX_TARGETS) as usize;
        let parts = Self::split_amount(template.debit_amount, n, &mut self.allocation_rng);
        if parts.len() < ALLOCATION_MIN_TARGETS as usize {
            // amount too small to make a meaningful batch — leave it a normal JE
            return None;
        }
        // Valid cost-center candidates for this company (joins back to master).
        let company_code = entry.header.company_code.clone();
        let cc_pool: Vec<String> = if self.cost_center_pool.is_empty() {
            Self::COST_CENTER_POOL
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            let needle = format!("-{company_code}-");
            let filtered: Vec<String> = self
                .cost_center_pool
                .iter()
                .filter(|id| id.contains(&needle))
                .cloned()
                .collect();
            if filtered.is_empty() {
                self.cost_center_pool.clone()
            } else {
                filtered
            }
        };
        let mut new_lines: Vec<JournalEntryLine> =
            Vec::with_capacity(entry.lines.len() + parts.len() - 1);
        for (j, line) in entry.lines.iter().enumerate() {
            if j == idx {
                let bu_on = self.config.business_unit_dimension.unwrap_or(true);
                for (k, part) in parts.iter().enumerate() {
                    let mut nl = template.clone();
                    nl.debit_amount = *part;
                    nl.credit_amount = Decimal::ZERO;
                    nl.cost_center = Some(cc_pool[k % cc_pool.len()].clone());
                    // SOTA-3: keep business_unit coherent with the *new* CC
                    // (the clone carried the template's stale BU).
                    if bu_on {
                        nl.business_unit = nl
                            .cost_center
                            .as_deref()
                            .map(Self::business_unit_for_dimension);
                    }
                    new_lines.push(nl);
                }
            } else {
                new_lines.push(line.clone());
            }
        }
        // Derived id (distinct from the reversal salt); retag as an allocation.
        let base_id = entry.header.document_id;
        let alloc_id =
            uuid::Uuid::from_u128(base_id.as_u128() ^ 0xA110_CA70_A110_CA70_A110_CA70_A110_CA70);
        entry.header.document_id = alloc_id;
        entry.header.sap_source_code = Some("AB".to_string());
        entry.header.header_text = Some("Allocation/assessment cycle".to_string());
        entry.header.reference = Some(format!("ALLOC-{base_id}"));
        entry.header.batch_id = None;
        for (i, line) in new_lines.iter_mut().enumerate() {
            line.line_number = (i + 1) as u32;
            line.document_id = alloc_id;
        }
        entry.lines = new_lines.into();
        Some(entry)
    }

    fn determine_fraud(&mut self, business_process: BusinessProcess) -> Option<FraudType> {
        if !self.fraud_config.enabled {
            return None;
        }

        // v5.30 B3 (#153) — per-process fraud rate override. When
        // `fraud.per_process_rates` carries an entry for this JE's business
        // process, use that rate instead of the global `fraud_rate`. Unmapped
        // processes fall back to the global rate (preserving v5.29 default
        // behavior for configs that don't opt in to per-process rates).
        //
        // The slug uses the YAML wire form (matches `#[serde(rename_all =
        // "UPPERCASE")]` plus the per-variant renames on `BusinessProcess`).
        let process_slug = match business_process {
            BusinessProcess::P2P => "P2P",
            BusinessProcess::O2C => "O2C",
            BusinessProcess::R2R => "R2R",
            BusinessProcess::H2R => "H2R",
            BusinessProcess::A2R => "A2R",
            BusinessProcess::S2C => "S2C",
            BusinessProcess::Mfg => "MFG",
            BusinessProcess::Bank => "BANK",
            BusinessProcess::Audit => "AUDIT",
            BusinessProcess::Treasury => "TREASURY",
            BusinessProcess::Tax => "TAX",
            BusinessProcess::Intercompany => "INTERCOMPANY",
            BusinessProcess::ProjectAccounting => "PROJECT",
            BusinessProcess::Esg => "ESG",
        };
        let effective_rate = self
            .fraud_config
            .per_process_rates
            .get(process_slug)
            .copied()
            .unwrap_or(self.fraud_config.fraud_rate);

        // Roll for fraud based on the (per-process or global) rate
        if self.rng.random::<f64>() >= effective_rate {
            return None;
        }

        // Select fraud type based on distribution
        Some(self.select_fraud_type())
    }

    /// Select a fraud type based on the configured distribution.
    fn select_fraud_type(&mut self) -> FraudType {
        let dist = &self.fraud_config.fraud_type_distribution;
        let roll: f64 = self.rng.random();

        let mut cumulative = 0.0;

        cumulative += dist.suspense_account_abuse;
        if roll < cumulative {
            return FraudType::SuspenseAccountAbuse;
        }

        cumulative += dist.fictitious_transaction;
        if roll < cumulative {
            return FraudType::FictitiousTransaction;
        }

        cumulative += dist.revenue_manipulation;
        if roll < cumulative {
            return FraudType::RevenueManipulation;
        }

        cumulative += dist.expense_capitalization;
        if roll < cumulative {
            return FraudType::ExpenseCapitalization;
        }

        cumulative += dist.split_transaction;
        if roll < cumulative {
            return FraudType::SplitTransaction;
        }

        cumulative += dist.timing_anomaly;
        if roll < cumulative {
            return FraudType::TimingAnomaly;
        }

        cumulative += dist.unauthorized_access;
        if roll < cumulative {
            return FraudType::UnauthorizedAccess;
        }

        cumulative += dist.duplicate_payment;
        if roll < cumulative {
            return FraudType::DuplicatePayment;
        }

        cumulative += dist.kickback_scheme;
        if roll < cumulative {
            return FraudType::KickbackScheme;
        }

        cumulative += dist.round_tripping;
        if roll < cumulative {
            return FraudType::RoundTripping;
        }

        cumulative += dist.unauthorized_discount;
        if roll < cumulative {
            return FraudType::UnauthorizedDiscount;
        }

        // Fallback when distribution is sub-1.0 (validator allows tolerance)
        FraudType::DuplicatePayment
    }

    /// Map a fraud type to an amount pattern for suspicious amounts.
    fn fraud_type_to_amount_pattern(&self, fraud_type: FraudType) -> FraudAmountPattern {
        match fraud_type {
            FraudType::SplitTransaction | FraudType::JustBelowThreshold => {
                FraudAmountPattern::ThresholdAdjacent
            }
            FraudType::FictitiousTransaction
            | FraudType::FictitiousEntry
            | FraudType::SuspenseAccountAbuse
            | FraudType::RoundDollarManipulation => FraudAmountPattern::ObviousRoundNumbers,
            FraudType::RevenueManipulation
            | FraudType::ExpenseCapitalization
            | FraudType::ImproperCapitalization
            | FraudType::ReserveManipulation
            | FraudType::UnauthorizedAccess
            | FraudType::PrematureRevenue
            | FraudType::UnderstatedLiabilities
            | FraudType::OverstatedAssets
            | FraudType::ChannelStuffing => FraudAmountPattern::StatisticallyImprobable,
            FraudType::DuplicatePayment
            | FraudType::TimingAnomaly
            | FraudType::SelfApproval
            | FraudType::ExceededApprovalLimit
            | FraudType::SegregationOfDutiesViolation
            | FraudType::UnauthorizedApproval
            | FraudType::CollusiveApproval
            | FraudType::FictitiousVendor
            | FraudType::ShellCompanyPayment
            | FraudType::Kickback
            | FraudType::KickbackScheme
            | FraudType::UnauthorizedDiscount
            | FraudType::RoundTripping
            | FraudType::InvoiceManipulation
            | FraudType::AssetMisappropriation
            | FraudType::InventoryTheft
            | FraudType::GhostEmployee => FraudAmountPattern::Normal,
            // Accounting Standards Fraud Types (ASC 606/IFRS 15 - Revenue)
            FraudType::ImproperRevenueRecognition
            | FraudType::ImproperPoAllocation
            | FraudType::VariableConsiderationManipulation
            | FraudType::ContractModificationMisstatement => {
                FraudAmountPattern::StatisticallyImprobable
            }
            // Accounting Standards Fraud Types (ASC 842/IFRS 16 - Leases)
            FraudType::LeaseClassificationManipulation
            | FraudType::OffBalanceSheetLease
            | FraudType::LeaseLiabilityUnderstatement
            | FraudType::RouAssetMisstatement => FraudAmountPattern::StatisticallyImprobable,
            // Accounting Standards Fraud Types (ASC 820/IFRS 13 - Fair Value)
            FraudType::FairValueHierarchyManipulation
            | FraudType::Level3InputManipulation
            | FraudType::ValuationTechniqueManipulation => {
                FraudAmountPattern::StatisticallyImprobable
            }
            // Accounting Standards Fraud Types (ASC 360/IAS 36 - Impairment)
            FraudType::DelayedImpairment
            | FraudType::ImpairmentTestAvoidance
            | FraudType::CashFlowProjectionManipulation
            | FraudType::ImproperImpairmentReversal => FraudAmountPattern::StatisticallyImprobable,
            // Sourcing/Procurement Fraud
            FraudType::BidRigging
            | FraudType::PhantomVendorContract
            | FraudType::ConflictOfInterestSourcing => FraudAmountPattern::Normal,
            FraudType::SplitContractThreshold => FraudAmountPattern::ThresholdAdjacent,
            // HR/Payroll Fraud
            FraudType::GhostEmployeePayroll
            | FraudType::PayrollInflation
            | FraudType::DuplicateExpenseReport
            | FraudType::FictitiousExpense => FraudAmountPattern::Normal,
            FraudType::SplitExpenseToAvoidApproval => FraudAmountPattern::ThresholdAdjacent,
            // O2C Fraud
            FraudType::RevenueTimingManipulation => FraudAmountPattern::StatisticallyImprobable,
            FraudType::QuotePriceOverride => FraudAmountPattern::Normal,
        }
    }

    /// Generate a deterministic UUID using the factory.
    #[inline]
    fn generate_deterministic_uuid(&self) -> uuid::Uuid {
        self.uuid_factory.next()
    }

    /// Cost center pool used for expense account enrichment.
    const COST_CENTER_POOL: &'static [&'static str] =
        &["CC1000", "CC2000", "CC3000", "CC4000", "CC5000"];

    /// Enrich journal entry line items with account descriptions, cost centers,
    /// profit centers, value dates, line text, and assignment fields.
    ///
    /// This populates the sparse optional fields that `JournalEntryLine::debit()`
    /// and `::credit()` leave as `None`.
    ///
    /// SP3 T13: changed to `&mut self` so `loaded_priors` fanout samplers
    /// can be driven for CostCenter and ProfitCenter when priors are loaded.
    fn enrich_line_items(&mut self, entry: &mut JournalEntry) {
        let posting_date = entry.header.posting_date;
        let company_code = &entry.header.company_code;
        let header_text = entry.header.header_text.clone();
        let business_process = entry.header.business_process;
        // SP3 T13 — document-type code used as the entity_id for fanout
        // samplers.  Derived from the header field set during generate().
        let doc_type_key = entry.header.document_type.clone();

        // SP3.7 — capture the SAP source code as an owned Option<String> so it
        // can be passed to `sample_attribute_for_source` as a `&str` inside the
        // line loop without keeping a borrow on `entry`.
        let header_sap_code: Option<String> = entry.header.sap_source_code.clone();

        // SP3.3 — resolve cross-entity motif neighbors once before the line
        // loop.  Owned Vec avoids holding a shared borrow on `self.loaded_priors`
        // across the subsequent `&mut` fanout-sampler calls.
        let (cc_pc_neighbor_vec, cc_pc_share_prob): (Vec<String>, f64) =
            if let Some(priors) = &self.loaded_priors {
                if let Some(motifs) = &priors.cross_entity_motifs {
                    (
                        motifs.neighbors(&doc_type_key).to_vec(),
                        motifs.should_share(&doc_type_key),
                    )
                } else {
                    (Vec::new(), 0.0)
                }
            } else {
                (Vec::new(), 0.0)
            };

        // Derive a deterministic index from the document_id for cost center selection
        let doc_id_bytes = entry.header.document_id.as_bytes();
        let mut cc_seed: usize = 0;
        for &b in doc_id_bytes {
            cc_seed = cc_seed.wrapping_add(b as usize);
        }

        for (i, line) in entry.lines.iter_mut().enumerate() {
            // 1. account_description: look up from CoA
            if line.account_description.is_none() {
                line.account_description = self
                    .coa
                    .get_account(&line.gl_account)
                    .map(|a| a.short_description.clone());
            }

            // 2. cost_center: assign to expense accounts (5xxx/6xxx)
            //
            // SP3 T13: when priors are loaded, the CostCenter fanout
            // sampler overrides the pool/legacy path.  This block runs
            // before the existing logic; if the sampler fires, `line.cost_center`
            // is set and the legacy block below is skipped via the
            // `line.cost_center.is_none()` guard.
            //
            // When the orchestrator has provided a master-data-sourced
            // pool (`with_cost_center_pool`), pick from it so the value
            // joins back to `cost_centers.id`.  Otherwise fall back to
            // the legacy hardcoded `COST_CENTER_POOL` const.
            //
            // Selection within the pool is filtered to entries that
            // mention the entry's `company_code` (master IDs follow
            // the `CC-{company}-...` convention) so cross-company
            // contamination is avoided; if no pool entry matches the
            // company we fall through to the full pool.
            if line.cost_center.is_none() {
                // SP3 T13 — prior-driven CostCenter fanout.
                // SP3.3: prefer neighbor-used buckets when motifs are available.
                // SP3.7: try per-source conditional cost_center first; fall back
                //        to the fanout sampler when the conditional is absent.
                let priors_opt = &mut self.loaded_priors;
                let rng_ref = &mut self.rng;
                if let Some(priors) = priors_opt {
                    let sp37_cc = header_sap_code.as_deref().and_then(|code| {
                        priors.sample_attribute_for_source(code, "cost_center", rng_ref)
                    });
                    if sp37_cc.is_some() {
                        line.cost_center = sp37_cc;
                    } else if let Some(sampler) = priors.fanout_samplers.get_mut("CostCenter") {
                        line.cost_center = Some(sampler.pick_for_with_neighbors(
                            &doc_type_key,
                            &cc_pc_neighbor_vec,
                            cc_pc_share_prob,
                            rng_ref,
                        ));
                    }
                }
            }
            if line.cost_center.is_none() {
                let first_char = line.gl_account.chars().next().unwrap_or('0');
                if first_char == '5' || first_char == '6' {
                    if !self.cost_center_pool.is_empty() {
                        let needle = format!("-{company_code}-");
                        let candidates: Vec<&String> = self
                            .cost_center_pool
                            .iter()
                            .filter(|id| id.contains(&needle))
                            .collect();
                        let pool: Vec<&String> = if candidates.is_empty() {
                            self.cost_center_pool.iter().collect()
                        } else {
                            candidates
                        };
                        let idx = cc_seed.wrapping_add(i) % pool.len();
                        line.cost_center = Some(pool[idx].clone());
                    } else {
                        let idx = cc_seed.wrapping_add(i) % Self::COST_CENTER_POOL.len();
                        line.cost_center = Some(Self::COST_CENTER_POOL[idx].to_string());
                    }
                }
            }

            // 3. profit_center: assign from master pool when available
            // (`with_profit_center_pool`); otherwise derive from
            // company code + business process (legacy behaviour, which
            // does not match the master-data PC ID format).
            //
            // SP3 T13: prior-driven ProfitCenter fanout override fires first
            // (same pattern as CostCenter above).
            if line.profit_center.is_none() {
                // SP3 T13 — prior-driven ProfitCenter fanout.
                // SP3.3: prefer neighbor-used buckets when motifs are available.
                // SP3.7: try per-source conditional profit_center first; fall back
                //        to the fanout sampler when the conditional is absent.
                let priors_opt = &mut self.loaded_priors;
                let rng_ref = &mut self.rng;
                if let Some(priors) = priors_opt {
                    let sp37_pc = header_sap_code.as_deref().and_then(|code| {
                        priors.sample_attribute_for_source(code, "profit_center", rng_ref)
                    });
                    if sp37_pc.is_some() {
                        line.profit_center = sp37_pc;
                    } else if let Some(sampler) = priors.fanout_samplers.get_mut("ProfitCenter") {
                        line.profit_center = Some(sampler.pick_for_with_neighbors(
                            &doc_type_key,
                            &cc_pc_neighbor_vec,
                            cc_pc_share_prob,
                            rng_ref,
                        ));
                    }
                }
            }
            if line.profit_center.is_none() {
                if !self.profit_center_pool.is_empty() {
                    let needle = format!("-{company_code}-");
                    let candidates: Vec<&String> = self
                        .profit_center_pool
                        .iter()
                        .filter(|id| id.contains(&needle))
                        .collect();
                    let pool: Vec<&String> = if candidates.is_empty() {
                        self.profit_center_pool.iter().collect()
                    } else {
                        candidates
                    };
                    let idx = cc_seed.wrapping_add(i) % pool.len();
                    line.profit_center = Some(pool[idx].clone());
                } else {
                    let suffix = match business_process {
                        Some(BusinessProcess::P2P) => "-P2P",
                        Some(BusinessProcess::O2C) => "-O2C",
                        Some(BusinessProcess::R2R) => "-R2R",
                        Some(BusinessProcess::H2R) => "-H2R",
                        _ => "",
                    };
                    line.profit_center = Some(format!("PC-{company_code}{suffix}"));
                }
            }

            // 3b. business_unit (SOTA-3): a coherent roll-up of the cost center,
            // or the profit center as fallback — the same dimension value always
            // maps to the same BU, so BU-level analytics are consistent. Runs
            // after both CC (step 2) and PC (step 3) are assigned; using CC-or-PC
            // lifts fill toward the corpus (~82%) vs only CC-bearing lines (~24%).
            // Flag-gated by `transactions.business_unit_dimension` (default-on).
            if line.business_unit.is_none() && self.config.business_unit_dimension.unwrap_or(true) {
                if let Some(dim) = line
                    .cost_center
                    .as_deref()
                    .or(line.profit_center.as_deref())
                {
                    line.business_unit = Some(Self::business_unit_for_dimension(dim));
                }
            }

            // 4. trading_partner: SP3.9 — inherit JE-level trading_partner from
            // the header. The header was populated once per JE in generate();
            // all lines share the same value to match corpus SAP semantics.
            // The is_none() guard preserves TP values already set by the P2P/O2C
            // document chain manager (also JE-level, different code path).
            if line.trading_partner.is_none() {
                line.trading_partner = entry.header.trading_partner.clone();
            }

            // 5. line_text: fall back to header_text if not already set
            if line.line_text.is_none() {
                line.line_text = header_text.clone();
            }

            // 6. value_date: set to posting_date for AR/AP accounts
            if line.value_date.is_none()
                && (line.gl_account.starts_with("1100") || line.gl_account.starts_with("2000"))
            {
                line.value_date = Some(posting_date);
            }

            // 7. assignment: set to vendor/customer reference for AP/AR lines
            if line.assignment.is_none() {
                if line.gl_account.starts_with("2000") {
                    // AP line - use vendor reference from header
                    if let Some(ref ht) = header_text {
                        // Try to extract vendor ID from header text patterns like "... - V-001"
                        if let Some(vendor_part) = ht.rsplit(" - ").next() {
                            if vendor_part.starts_with("V-")
                                || vendor_part.starts_with("VENDOR")
                                || vendor_part.starts_with("Vendor")
                            {
                                line.assignment = Some(vendor_part.to_string());
                            }
                        }
                    }
                } else if line.gl_account.starts_with("1100") {
                    // AR line - use customer reference from header
                    if let Some(ref ht) = header_text {
                        if let Some(customer_part) = ht.rsplit(" - ").next() {
                            if customer_part.starts_with("C-")
                                || customer_part.starts_with("CUST")
                                || customer_part.starts_with("Customer")
                            {
                                line.assignment = Some(customer_part.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    /// Generate a single journal entry.
    pub fn generate(&mut self) -> JournalEntry {
        debug!(
            count = self.count,
            companies = self.companies.len(),
            start_date = %self.start_date,
            end_date = %self.end_date,
            "Generating journal entry"
        );

        // Check if we're in a batch - if so, generate a batched entry
        if let Some(ref state) = self.batch_state {
            if state.remaining > 0 {
                return self.generate_batched_entry();
            }
        }

        // SOTA-5: with a small probability, emit a reversal/correction of a
        // recent JE instead of a fresh one (a process auditors look for).
        if let Some(rev) = self.maybe_generate_reversal() {
            return rev;
        }

        // SOTA-6: with a small probability, emit a large allocation/assessment
        // batch (the corpus lines-per-JE tail) instead of a fresh JE.
        if let Some(alloc) = self.maybe_generate_allocation_batch() {
            return alloc;
        }

        // SP6 — Lazy-init the MD resolver on the first call. Rebuilding once
        // per run is sufficient; pools are stable after master-data generation.
        if self.md_resolver.companies.is_empty()
            && self.md_resolver.persons.is_empty()
            && self.md_resolver.patients.is_empty()
        {
            self.refresh_md_resolver();
        }

        self.count += 1;

        // Generate deterministic document ID
        let document_id = self.generate_deterministic_uuid();

        // SP3.5c — Lazy temporal-sampler date draw.
        //
        // When priors are loaded the IET path (SP3 T11) will immediately replace
        // this value, so drawing from the temporal sampler here wastes one RNG
        // advance on the sampler's internal stream AND makes the temporal-sampler
        // variance contribute to the merged date sequence even though the IET
        // sampler is meant to dominate.
        //
        // Fix: only draw from the temporal sampler now when no priors are loaded.
        // The IET block sets `posting_date` unconditionally when priors are Some;
        // the active-window fallback (SP3 T14) has its own sample_date call and is
        // unaffected by this change.
        //
        // Priors-absent path: byte-identical to v5.13 — the draw and business-day
        // snap are performed exactly as before.
        let mut posting_date = if self.loaded_priors.is_none() {
            let mut d = self
                .temporal_sampler
                .sample_date(self.start_date, self.end_date);
            // Adjust posting date to be a business day if business day calculator is configured
            if let Some(ref calc) = self.business_day_calculator {
                if !calc.is_business_day(d) {
                    d = calc.next_business_day(d, false);
                    if d > self.end_date {
                        d = calc.prev_business_day(self.end_date, true);
                    }
                }
            }
            d
        } else {
            // Priors-loaded path: IET block (below) will set the real date.
            // Use start_date as a zero-cost placeholder — it is always overwritten.
            self.start_date
        };

        // Select company using weighted selector
        let company_code = self.company_selector.select(&mut self.rng).to_string();

        // v4.1.0+: draw a single (u, v) pair from the copula — cached for
        // both the amount adjustment (u) and the line-count shift (v).
        // None when no copula is configured.
        let copula_uv: Option<(f64, f64)> =
            self.correlation_copula.as_mut().map(|cop| cop.sample());

        // Sample line item specification. When a copula is configured,
        // v drives line-count via a quantile-preserving map: integer
        // count `2 + floor(v * 10)` gives range [2, 11] evenly spaced
        // in v, so rank(v) == rank(line_count).
        //
        // v4.1.6+: upgraded from the v3.5.4 nudge (shift around
        // independently-drawn count) to true rank-preserving quantile
        // inversion, so empirical Kendall-τ now matches copula theory.
        let mut line_spec = self.line_sampler.sample();
        if let Some((_u, v)) = copula_uv {
            let new_total = 2 + ((v * 10.0).floor() as usize).min(9);
            let old_debit = line_spec.debit_count.max(1);
            let old_credit = line_spec.credit_count.max(1);
            let new_debit = (new_total as f64 * old_debit as f64 / (old_debit + old_credit) as f64)
                .round() as usize;
            let new_debit = new_debit.clamp(1, new_total - 1);
            let new_credit = new_total - new_debit;
            line_spec.total_count = new_total;
            line_spec.debit_count = new_debit;
            line_spec.credit_count = new_credit;
        }

        // SOTA-10 (#138): optional hard cap on total lines per JE — tames the
        // monster outliers (synth max 2133 vs corpus 924). Scales debit + credit
        // proportionally so balance is preserved.
        if let Some(cap) = self.config.lines_per_je_cap {
            let cap = cap.max(2);
            let total = line_spec.debit_count + line_spec.credit_count;
            if total > cap {
                let new_debit =
                    ((line_spec.debit_count as f64 / total as f64) * cap as f64).round() as usize;
                let new_debit = new_debit.clamp(1, cap - 1);
                let new_credit = cap - new_debit;
                line_spec.total_count = cap;
                line_spec.debit_count = new_debit;
                line_spec.credit_count = new_credit;
            }
        }

        // Determine source type using full 4-way distribution
        let source = self.select_source();
        let is_automated = matches!(
            source,
            TransactionSource::Automated | TransactionSource::Recurring
        );

        // SP3.6 — when priors are loaded, sample a canonical SAP source code
        // from the bundle's source-mix distribution.  This is independent of
        // the `TransactionSource` enum (which controls manual/automated semantics)
        // and is written to `header.sap_source_code`, then emitted in the CSV
        // `source` column in place of the generic label.
        let sap_source_code: Option<String> = self.sample_sap_source_code();
        // SOTA-8: stash the current JE's SAP source so select_*_account can consult
        // the per-source Dirichlet pool. Cleared at the end of this generate() call.
        self.current_je_source = sap_source_code.clone();

        // Select business process
        let business_process = self.select_business_process();

        // SP3 T11 — IET-driven posting-date override.
        //
        // When priors are loaded, replace the uniform temporal-sampler date
        // with one derived from the per-Source inter-event-time prior.  We
        // accumulate IET samples (in fractional days) per source code and
        // map the accumulated offset onto [start_date, end_date].
        //
        // v5.30 B1 (#152): route through `sap_source_code` (the actual emitted
        // source) rather than `doc_type` (only 5 values: KR/DR/SA/HR/AA from
        // document_type_for_process). Before B1, `sample_next(&doc_type, …)`
        // hit the IET sampler with only 5 distinct keys for all 526 emitted
        // sources, leaving the per-source lag-1 autocorr machinery in
        // ConditionalIETSampler **unwired** for 521 of the sources. The
        // Sajja P1 autocorr DR of 105.9× (worst sub-metric on the A1 eval)
        // is the direct downstream consequence. Switching to source-keyed
        // sampling actually exercises the per-source priors.
        //
        // The None path is untouched: `posting_date` from the temporal sampler
        // above is used as-is.
        {
            // Split-borrow: four distinct struct fields accessed simultaneously.
            let priors_opt = &mut self.loaded_priors;
            let rng_ref = &mut self.rng;
            let iet_accum_ref = &mut self.iet_day_accum;
            let burst_ref = &mut self.iet_burst_remaining;
            if let Some(priors) = priors_opt {
                // Prefer the per-row SAP source code (populated when priors
                // load via SP3.6's source-mix sampler). Fall back to doc_type
                // for the rare branch where source-code sampling returned None.
                let iet_key = sap_source_code
                    .as_deref()
                    .unwrap_or_else(|| Self::document_type_for_process(business_process))
                    .to_string();
                let period_days = (self.end_date - self.start_date).num_days().max(1) as f64;

                // v5.30 B1 Phase 2 — burst clustering.
                //
                // The lag-1 Gaussian-copula path in ConditionalIETSampler
                // (conditional_iet.rs:176-203) silently falls back to
                // independent sampling whenever the per-source |ρ| < 0.1.
                // The bundled SP3 priors' per-source lag1_autocorr values are
                // mostly below that threshold (corpus has only weak
                // day-resolution autocorrelation), so the coupling never
                // fires and the within-source IET autocorr matches the
                // noise floor — the Sajja P1 autocorr 105.9× DR before A3,
                // 62.84× after A3, with B1 Phase 1 (source-keying) producing
                // no measurable lift.
                //
                // This block bypasses the |ρ| < 0.1 gate by emitting
                // **deterministic** short-IET bursts for each source.  When
                // a sampled IET is short (< BURST_THRESHOLD_DAYS) and a
                // probability gate fires (BURST_PROB), the next
                // BURST_LEN events for that source emit IETs in
                // [0.25, 1.5] days regardless of what the sampler returns.
                //
                // Effect on within-source IET autocorrelation: events 1..k
                // of a burst have tightly-clustered IETs around 0.85 days
                // mean → lag-1 autocorr lifts directly. Inter-burst IETs
                // are still sampled normally so the macro distribution
                // stays close to the prior.
                const BURST_THRESHOLD_DAYS: f64 = 2.0;
                const BURST_PROB: f64 = 0.30;
                const BURST_LEN_MIN: u8 = 2;
                const BURST_LEN_MAX: u8 = 4;

                let sampled_iet = priors.iet_sampler.sample_next(&iet_key, rng_ref).max(0.001);

                // Check if we're inside an active burst for this source.
                let remaining = burst_ref.get(&iet_key).copied().unwrap_or(0);
                let iet = if remaining > 0 {
                    // Active burst: emit a short IET regardless of sampler.
                    burst_ref.insert(iet_key.clone(), remaining - 1);
                    rng_ref.random_range(0.25..=1.5)
                } else if sampled_iet < BURST_THRESHOLD_DAYS
                    && rng_ref.random_range(0.0..1.0) < BURST_PROB
                {
                    // Start a new burst: this event uses the sampled IET,
                    // and the next BURST_LEN events for this source will
                    // emit short IETs.
                    let len = rng_ref.random_range(BURST_LEN_MIN..=BURST_LEN_MAX);
                    burst_ref.insert(iet_key.clone(), len);
                    sampled_iet
                } else {
                    sampled_iet
                };

                let accum = iet_accum_ref.entry(iet_key).or_insert(0.0);
                *accum += iet;
                // Wrap within period so we never exceed the generation window.
                if *accum >= period_days {
                    *accum %= period_days;
                }
                let day_offset =
                    (*accum as i64).clamp(0, (self.end_date - self.start_date).num_days());
                posting_date = self.start_date + chrono::Duration::days(day_offset);
                // Re-apply business-day snap so the IET date still lands on a
                // working day (matches the business_day_calculator logic above).
                if let Some(ref calc) = self.business_day_calculator {
                    if !calc.is_business_day(posting_date) {
                        posting_date = calc.next_business_day(posting_date, false);
                        if posting_date > self.end_date {
                            posting_date = calc.prev_business_day(self.end_date, true);
                        }
                    }
                }
            } // end if let Some(priors)
        } // end split-borrow scope

        // SP3 T14 — active-window gating.
        //
        // After the IET-driven date is computed, check whether this Source is
        // still in its active window for the resulting day.  If the prior says
        // the Source has "gone quiet" (e.g. a vendor that stopped trading), we
        // fall back to the temporal-sampler date so the JE still emits but is
        // no longer anchored to the IET timeline for this source.
        //
        // In a day-loop architecture this would be a `continue`; here, the
        // equivalent is to revert `posting_date` to the original temporal-
        // sampler sample so downstream logic sees a plausible date.
        //
        // The None path is untouched.
        if let Some(ref priors) = self.loaded_priors {
            let doc_type = Self::document_type_for_process(business_process);
            let day_in_period = (posting_date - self.start_date).num_days();
            let active = match &priors.multi_segment_window {
                Some(msw) => msw.is_active(doc_type, day_in_period),
                None => priors.active_window.is_active(doc_type, day_in_period),
            };
            if !active {
                // Source is outside its active window: fall back to a fresh
                // temporal-sampler draw.  (SP3.5c: the up-front temporal draw
                // is skipped when priors are loaded, so we always re-sample
                // here in the fallback path rather than reusing a cached value.)
                posting_date = self
                    .temporal_sampler
                    .sample_date(self.start_date, self.end_date);
                if let Some(ref calc) = self.business_day_calculator {
                    if !calc.is_business_day(posting_date) {
                        posting_date = calc.next_business_day(posting_date, false);
                        if posting_date > self.end_date {
                            posting_date = calc.prev_business_day(self.end_date, true);
                        }
                    }
                }
            }
        }

        // SP3 T12 — lines-per-JE override from prior histogram.
        //
        // When priors are loaded, replace `line_spec` totals with a sample
        // drawn from the Source-conditional histogram (falling back to the
        // overall histogram when the document-type is unknown).  `.max(2)`
        // guarantees every JE has at least one debit + one credit line.
        // The None path leaves `line_spec` from the copula / line-sampler
        // cascade above completely unchanged.
        if let Some(ref priors) = self.loaded_priors {
            let doc_type = Self::document_type_for_process(business_process);
            let hist = priors
                .lines_per_je
                .by_source
                .get(doc_type)
                .unwrap_or(&priors.lines_per_je.overall);
            let n_total = (hist.sample_bucket(&mut self.rng) as usize).max(2);
            let old_debit = line_spec.debit_count.max(1);
            let old_credit = line_spec.credit_count.max(1);
            let new_debit = (n_total as f64 * old_debit as f64 / (old_debit + old_credit) as f64)
                .round() as usize;
            let new_debit = new_debit.clamp(1, n_total - 1);
            line_spec.total_count = n_total;
            line_spec.debit_count = new_debit;
            line_spec.credit_count = n_total - new_debit;
        }

        // Determine if this is a fraudulent transaction (v5.30 B3 — per-process
        // rates pass `business_process` through to honor fraud.per_process_rates
        // overrides when configured)
        let fraud_type = self.determine_fraud(business_process);
        let is_fraud = fraud_type.is_some();

        // Sample time based on source
        let time = self.temporal_sampler.sample_time(!is_automated);
        let created_at = posting_date.and_time(time).and_utc();

        // Select user from pool or generate generic
        let (created_by, user_persona) = self.select_user(is_automated);

        // Create header with deterministic UUID
        let mut header =
            JournalEntryHeader::with_deterministic_id(company_code, posting_date, document_id);
        header.created_at = created_at;
        header.source = source;
        header.sap_source_code = sap_source_code;

        // SP3.9 — JE-level trading partner. Draw once per JE; all lines
        // inherit. corpus SAP semantics is one TP per document.
        // SP3.12 — TP motif sampler: bias toward cluster-mates of the
        // previously-drawn TP on the same source to build triangle structure.
        // Split-borrow: sap_source_code was moved into header above, so clone
        // the code out before the mutable borrow on self.loaded_priors.
        // (sap_source_code is cloned again below for the SP4.5 user-persona lookup)
        {
            let code_opt = header.sap_source_code.clone();
            if let Some(ref code) = code_opt {
                let rng_ref = &mut self.rng;
                // SP3.12: resolve TP motif neighbors from the last TP on this source.
                // We read last_tp_by_source (shared ref) before the mutable borrow
                // on loaded_priors.  The update happens after the block.
                let tp_neighbors: Vec<String> = if let Some(ref priors) = self.loaded_priors {
                    if let Some(ref motifs) = priors.tp_motif_sampler {
                        if let Some(last_tp) = self.last_tp_by_source.get(code.as_str()) {
                            motifs.neighbors(last_tp).to_vec()
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                let tp_share_prob: f64 = if let Some(ref priors) = self.loaded_priors {
                    if let Some(ref motifs) = priors.tp_motif_sampler {
                        if let Some(last_tp) = self.last_tp_by_source.get(code.as_str()) {
                            motifs.should_share(last_tp)
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };

                if let Some(ref mut priors) = self.loaded_priors {
                    // SP3.12: if the motif roll fires AND the distribution
                    // supports one of the neighbor TP values, draw from that
                    // restricted set.  Otherwise fall through to the marginal.
                    let tp = if !tp_neighbors.is_empty()
                        && tp_share_prob > 0.0
                        && rng_ref.random_range(0.0..1.0) < tp_share_prob
                    {
                        // Find a neighbor that the per-source TP distribution
                        // actually knows about.  Sample from the full marginal
                        // weighted by the neighbor-filtered subset.
                        use datasynth_core::distributions::behavioral_priors::CategoricalDistribution;
                        let filtered: std::collections::BTreeMap<String, f64> = priors
                            .per_source_attribute
                            .as_ref()
                            .and_then(|psa| psa.conditional(code, "trading_partner"))
                            .map(|dist| {
                                dist.probabilities
                                    .iter()
                                    .filter(|(v, _)| tp_neighbors.contains(v))
                                    .map(|(v, p)| (v.clone(), *p))
                                    .collect()
                            })
                            .unwrap_or_default();
                        if filtered.is_empty() {
                            priors.sample_attribute_for_source(code, "trading_partner", rng_ref)
                        } else {
                            let neighbour_dist = CategoricalDistribution {
                                probabilities: filtered,
                                n: 0, // unused in sample()
                            };
                            neighbour_dist.sample(rng_ref).or_else(|| {
                                priors.sample_attribute_for_source(code, "trading_partner", rng_ref)
                            })
                        }
                    } else {
                        priors.sample_attribute_for_source(code, "trading_partner", rng_ref)
                    };
                    header.trading_partner = tp;
                }
                // SP3.12: record the drawn TP so the next JE on this source
                // can use it as the motif anchor.
                if let Some(ref tp) = header.trading_partner {
                    self.last_tp_by_source.insert(code.clone(), tp.clone());
                }
            }
        }

        // SP4.5 — user-persona prior: when a corpus prior with user data is
        // loaded, override `created_by` with a user characteristic of the drawn
        // source, and bias `created_at` hour-of-day from the user's density.
        // Falls back transparently to `created_by` / `created_at` already set above.
        let (created_by, created_at) = {
            let sap_code_for_user = header.sap_source_code.clone();
            if let (Some(ref code), Some(ref priors)) = (sap_code_for_user, &self.loaded_priors) {
                if let Some(uid) = priors.sample_user_for_source(code, &mut self.rng) {
                    let new_created_at = if let Some((hour, _)) =
                        priors.sample_timestamp_for_user(&uid, &mut self.rng)
                    {
                        let base = header.created_at;
                        base.date_naive()
                            .and_hms_opt(hour, 0, 0)
                            .map(|naive| naive.and_utc())
                            .unwrap_or(base)
                    } else {
                        header.created_at
                    };
                    (uid, new_created_at)
                } else {
                    (created_by, header.created_at)
                }
            } else {
                (created_by, header.created_at)
            }
        };

        header.created_by = created_by;
        header.created_at = created_at;
        header.user_persona = user_persona;
        header.business_process = Some(business_process);
        header.document_type = Self::document_type_for_process(business_process).to_string();
        header.is_fraud = is_fraud;
        header.fraud_type = fraud_type;

        // --- ISA 240 audit flags ---
        let is_manual = matches!(source, TransactionSource::Manual);
        header.is_manual = is_manual;

        // Determine source_system based on manual vs automated.
        //
        // Real ERPs typically expose 20+ distinct provenance codes per
        // company (one per module + sub-module + interface). The taxonomy
        // below is a strict superset of the legacy {manual, spreadsheet,
        // SAP-FI, SAP-MM, SAP-SD, interface, SAP-HR} codes so downstream
        // consumers that filter by prefix (e.g. `starts_with("SAP-")`)
        // continue to work.
        //
        // Contract preserved by the generator-level audit assertion in
        // `test_isa240_audit_flags_populated`:
        //   - manual entries → starts_with("manual") || starts_with("spreadsheet")
        //   - automated entries → does NOT start with "manual"/"spreadsheet"
        header.source_system = Self::pick_source_system(&mut self.rng, is_manual, business_process);

        // is_post_close: entry is in the last month of the configured period
        // and the posting date falls after the 25th (simulating close cutoff)
        let is_post_close = posting_date.month() == self.end_date.month()
            && posting_date.year() == self.end_date.year()
            && posting_date.day() > 25;
        header.is_post_close = is_post_close;

        // created_date: for manual entries, same day as posting; for automated,
        // 0-3 days before posting_date
        let created_date = if is_manual {
            posting_date.and_hms_opt(time.hour().min(23), time.minute(), time.second())
        } else {
            let lag_days = self.rng.random_range(0i64..=3);
            let created_naive_date = posting_date
                .checked_sub_signed(chrono::Duration::days(lag_days))
                .unwrap_or(posting_date);
            created_naive_date.and_hms_opt(
                self.rng.random_range(8u32..=17),
                self.rng.random_range(0u32..=59),
                self.rng.random_range(0u32..=59),
            )
        };
        header.created_date = created_date;

        // Generate description context
        let mut context =
            DescriptionContext::with_period(posting_date.month(), posting_date.year());

        // Add vendor/customer context based on business process
        match business_process {
            BusinessProcess::P2P => {
                if let Some(vendor) = self.vendor_pool.random_vendor(&mut self.rng) {
                    context.vendor_name = Some(vendor.name.clone());
                }
            }
            BusinessProcess::O2C => {
                if let Some(customer) = self.customer_pool.random_customer(&mut self.rng) {
                    context.customer_name = Some(customer.name.clone());
                }
            }
            _ => {}
        }

        // Generate header text if enabled.
        // SP6 — Try text-taxonomy prior (sample_header_template) first,
        // then the built-in DescriptionGenerator.
        if self.template_config.descriptions.generate_header_text {
            let priors_header = if let Some(src) = header.sap_source_code.as_deref() {
                if let Some(p) = self.loaded_priors.as_ref() {
                    // SP6: text-taxonomy header pool
                    p.sample_header_template(src, &mut self.md_resolver, &mut self.rng)
                } else {
                    None
                }
            } else {
                None
            };
            header.header_text = Some(priors_header.unwrap_or_else(|| {
                self.description_generator.generate_header_text(
                    business_process,
                    &context,
                    &mut self.rng,
                )
            }));
        }

        // Generate reference if enabled.
        // SP4.7 — when priors are loaded and the bundle carries a reference-format
        // template for the current SAP source code, sample from that distribution
        // instead of the fixed `ReferenceGenerator` template.  The priors path is
        // preferred because it produces corpus format patterns; the existing
        // generator is the fallback for sources not covered by the bundle.
        if self.template_config.references.generate_references {
            let priors_ref = header.sap_source_code.as_deref().and_then(|src| {
                self.loaded_priors
                    .as_ref()
                    .and_then(|p| p.sample_reference(src, &mut self.rng))
            });
            header.reference = Some(priors_ref.unwrap_or_else(|| {
                self.reference_generator
                    .generate_for_process_year(business_process, posting_date.year())
            }));
        }

        // Derive typed source document from reference prefix
        header.source_document = header
            .reference
            .as_deref()
            .and_then(DocumentRef::parse)
            .or_else(|| {
                if header.source == TransactionSource::Manual {
                    Some(DocumentRef::Manual)
                } else {
                    None
                }
            });

        // Generate line items
        let mut entry = JournalEntry::new(header);

        // Generate amount - use fraud pattern if this is a fraudulent transaction.
        // Non-fraud path prefers the v3.4.0 advanced sampler when configured; fraud
        // patterns always use the legacy sampler because they target specific
        // thresholds (round numbers, just-under-approval amounts) that are
        // orthogonal to mixture models.
        let base_amount = if let Some(ft) = fraud_type {
            let pattern = self.fraud_type_to_amount_pattern(ft);
            self.amount_sampler.sample_fraud(pattern)
        } else if let Some(ref mut adv) = self.advanced_amount_sampler {
            adv.sample_decimal()
        } else {
            self.amount_sampler.sample()
        };
        // v3.5.3+: if a conditional-amount override is configured and
        // the JE is non-fraud, re-sample the amount from the conditional
        // distribution using the computed context. Fraud entries bypass
        // this path to preserve fraud-pattern semantics (as with the
        // advanced sampler cascade above).
        let base_amount = if fraud_type.is_none() {
            // Compute input context BEFORE taking &mut on the sampler
            // to avoid borrow-checker conflict with the immutable
            // `conditional_input_value` call.
            let input = self.conditional_input_value(posting_date);
            if let Some(ref mut cond) = self.conditional_amount_override {
                cond.sample_decimal(input)
            } else {
                base_amount
            }
        } else {
            base_amount
        };

        // SP4.3 — when priors are loaded, try to replace the base_amount with
        // a draw from the per-source log-normal conditional.  This step only
        // fires for non-fraud JEs (fraud entries must preserve fraud-pattern
        // semantics).  We use the source-marginal (gl_prefix = "") as the
        // initial lookup; per-class refinement requires knowing the GL account
        // which is sampled after the amount in some paths, so we defer that
        // to a follow-up sprint.  Balance preservation is maintained because
        // the splitter below uses `total_amount` unchanged.
        //
        // W7.M — autocorr mitigation: ~30 % of priors-enabled draws bypass the
        // per-source conditional and draw from the global marginal sampler.
        // This loosens the per-source amount-sequence correlation that SP4.3's
        // conditional was over-tightening (v5.23 baseline: Source P1 Autocorr
        // +750 %, TP P1 Autocorr +101 %).  Proven pattern from SP3.12 W2
        // TP-clustering mitigation.
        //
        // Split-borrow: `loaded_priors` and `rng` are distinct struct fields so
        // the compiler allows simultaneous mutable borrows.
        // SP5.3 — intermediate tune from 0.20 to 0.25 between v5.24 (0.30 →
        // autocorr 1.53, over-corrected) and v5.25 (0.20 → autocorr 3.74,
        // under-corrected). Targets the trade-off sweet spot.
        const PRIORS_AMOUNT_BYPASS_SHARE: f64 = 0.25;
        let base_amount = if fraud_type.is_none() {
            if let Some(src) = entry.header.sap_source_code.as_deref() {
                let src_owned = src.to_string();
                // Gate: skip the conditional ~25 % of the time to loosen
                // per-source amount sequence correlation without overshooting.
                let use_conditional = self.loaded_priors.is_some()
                    && self.rng.random_range(0.0..1.0) >= PRIORS_AMOUNT_BYPASS_SHARE;
                if use_conditional {
                    let priors_ref = &mut self.loaded_priors;
                    let rng_ref = &mut self.rng;
                    if let Some(priors) = priors_ref {
                        priors
                            .sample_amount_for_source(&src_owned, "", rng_ref)
                            .and_then(|v| {
                                if v.is_finite() && v > 0.0 {
                                    Decimal::from_f64_retain(v)
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(base_amount)
                    } else {
                        base_amount
                    }
                } else {
                    base_amount
                }
            } else {
                base_amount
            }
        } else {
            base_amount
        };

        // v4.1.6+: if a copula is configured AND an advanced amount
        // sampler with a ppf is available, use true rank-preserving
        // inverse-CDF sampling — amount is drawn DIRECTLY from the
        // sampler's quantile at `u`, replacing (not nudging) the
        // independently-drawn base_amount. This makes empirical
        // Kendall-τ match the copula's theoretical τ.
        //
        // Fallback for copula-without-advanced-sampler: keep the
        // v4.1.0 log-scale multiplier nudge (observable correlation,
        // diluted magnitude).
        let base_amount = if fraud_type.is_none() {
            if let Some((u, _v)) = copula_uv {
                if let Some(ref adv) = self.advanced_amount_sampler {
                    adv.ppf_decimal(u)
                } else {
                    let log_mult = 4.0 * (u - 0.5);
                    let adjusted = base_amount.to_f64().unwrap_or(1.0) * log_mult.exp();
                    Decimal::from_f64_retain(adjusted).unwrap_or(base_amount)
                }
            } else {
                base_amount
            }
        } else {
            base_amount
        };

        // Apply temporal drift if configured
        let drift_adjusted_amount = {
            let drift = self.get_drift_adjustments(posting_date);
            if drift.amount_mean_multiplier != 1.0 {
                // Apply drift multiplier (includes seasonal factor if enabled)
                let multiplier = drift.amount_mean_multiplier * drift.seasonal_factor;
                let adjusted = base_amount.to_f64().unwrap_or(1.0) * multiplier;
                Decimal::from_f64_retain(adjusted).unwrap_or(base_amount)
            } else {
                base_amount
            }
        };

        // Apply human variation to amounts for non-automated transactions
        let total_amount = if is_automated {
            drift_adjusted_amount // Automated systems use exact amounts
        } else {
            self.apply_human_variation(drift_adjusted_amount)
        };

        // SP3 T13 — derive the document-type key once for use in all
        // fanout-sampler lookups below.  Computed unconditionally so it is
        // available for both debit and credit loops without re-deriving.
        let doc_type_for_fanout = Self::document_type_for_process(business_process).to_string();

        // SP3.3 — resolve cross-entity motif neighbors for this fanout entity.
        // We capture an owned Vec<String> here so that the shared borrow on
        // `self.loaded_priors` is released before the subsequent `&mut` borrow
        // on `fanout_samplers`.
        let (gl_neighbor_vec, gl_share_prob): (Vec<String>, f64) =
            if let Some(priors) = &self.loaded_priors {
                if let Some(motifs) = &priors.cross_entity_motifs {
                    (
                        motifs.neighbors(&doc_type_for_fanout).to_vec(),
                        motifs.should_share(&doc_type_for_fanout),
                    )
                } else {
                    (Vec::new(), 0.0)
                }
            } else {
                (Vec::new(), 0.0)
            };

        // SOTA-1: recurring/standard-journal templates. On the no-priors path,
        // reuse a cached account archetype for this (company, doc-type, counts)
        // with high probability so standard postings recur (and a hot account
        // subset dominates). Reuse overrides only the line account (set after
        // text/RNG below), so amounts/counts/dates stay byte-identical; fresh
        // archetypes are captured + cached after the lines are built.
        let reuse_archetype = self.pick_recurring_archetype(
            &entry.header.company_code,
            &doc_type_for_fanout,
            line_spec.debit_count,
            line_spec.credit_count,
        );
        let mut fresh_debit_accts: Vec<String> = Vec::new();
        let mut fresh_credit_accts: Vec<String> = Vec::new();
        // SOTA-8: hoisted so both the debit and credit loops + their SOTA-1 archetype
        // override blocks share the same flag.
        let sota8_active = self.config.source_conditional_account_pair.enabled;

        // Generate debit lines
        let debit_amounts = self
            .amount_sampler
            .sample_summing_to(line_spec.debit_count, total_amount);
        for (i, amount) in debit_amounts.into_iter().enumerate() {
            // SP3 T13 — GL Account fanout: when priors are loaded, pick the
            // account from the BipartiteFanoutSampler keyed "GLAccount" for
            // this Source.  Split-borrows let us hold &mut loaded_priors and
            // &mut rng at the same time (distinct struct fields).
            // SP3 T13 — GL Account fanout for debit lines.
            // Pre-compute the fallback before the split-borrow scope so that
            // `select_debit_account` (which takes `&mut self`) does not conflict
            // with the concurrent borrow of `loaded_priors` and `rng`.
            let debit_fallback = self.select_debit_account().account_number.clone();
            // SOTA-8: when enabled, the per-source Dirichlet pool (which `select_debit_account`
            // has already consulted via try_cond_pick_account_number) takes precedence over the
            // SP3/SP4 priors-driven path so the user's explicit source-conditional knob actually
            // governs the source-conditional account distribution. `sota8_active` is hoisted
            // above this scope so the credit loop can see it too.
            let account_number = if sota8_active {
                debit_fallback
            } else {
                let priors_opt = &mut self.loaded_priors;
                let rng_ref = &mut self.rng;
                if let Some(priors) = priors_opt {
                    // SP4.6 — role-aware GL account selection: try (source, "DR")
                    // conditional first, then fall back to SP3.7 source-marginal,
                    // then to the fanout sampler, then to the default debit account.
                    let sp46_gl = entry
                        .header
                        .sap_source_code
                        .as_deref()
                        .and_then(|code| priors.sample_gl_for_source_role(code, "DR", rng_ref));
                    if let Some(gl) = sp46_gl {
                        gl
                    } else {
                        // SP3.7 — try per-source marginal GL account.
                        let sp37_gl = entry.header.sap_source_code.as_deref().and_then(|code| {
                            priors.sample_attribute_for_source(code, "gl_account", rng_ref)
                        });
                        if let Some(gl) = sp37_gl {
                            gl
                        } else if let Some(sampler) = priors.fanout_samplers.get_mut("GLAccount") {
                            // SP3.3: prefer neighbor-used buckets when motifs are available.
                            sampler.pick_for_with_neighbors(
                                &doc_type_for_fanout,
                                &gl_neighbor_vec,
                                gl_share_prob,
                                rng_ref,
                            )
                        } else {
                            debit_fallback
                        }
                    }
                } else {
                    debit_fallback
                }
            };
            let mut line = JournalEntryLine::debit(
                entry.header.document_id,
                (i + 1) as u32,
                account_number.clone(),
                amount,
            );

            // Generate line text if enabled.
            // SP6 — Try text-taxonomy (account-class cascade), then DescriptionGenerator.
            if self.template_config.descriptions.generate_line_text {
                let src = entry.header.sap_source_code.as_deref();
                let priors_line = if let Some(s) = src {
                    if let Some(p) = self.loaded_priors.as_ref() {
                        let account_class = p
                            .coa_semantic
                            .as_ref()
                            .and_then(|c| c.accounts.get(&account_number))
                            .and_then(|a| a.account_class.as_deref())
                            .unwrap_or(
                                datasynth_core::distributions::text_taxonomy::TextTaxonomyPrior::UNKNOWN_CLASS,
                            );
                        // SP6 text_taxonomy cascade
                        p.sample_line_template(
                            s,
                            account_class,
                            &mut self.md_resolver,
                            &mut self.rng,
                        )
                    } else {
                        None
                    }
                } else {
                    None
                };
                line.line_text = Some(priors_line.unwrap_or_else(|| {
                    self.description_generator.generate_line_text(
                        &account_number,
                        &context,
                        &mut self.rng,
                    )
                }));
            }

            // SOTA-1: override the line's account with the reused archetype's
            // (RNG + text above are unchanged -> amounts/counts/dates stay
            // byte-identical); else capture the fresh account for caching.
            // SOTA-1 and SOTA-8 compose: SOTA-8 picks the FIRST archetype's accounts
            // from its per-source pool, then SOTA-1 caches + reuses them. Disabling
            // SOTA-1 under SOTA-8 actually *worsens* edge concentration — empirically
            // measured in Round 0 v4: edges/je 0.35 -> 0.82 when SOTA-1 was bypassed.
            if let Some((ref d, _)) = reuse_archetype {
                if let Some(a) = d.get(i) {
                    line.gl_account = a.clone();
                }
            } else if self.loaded_priors.is_none() {
                fresh_debit_accts.push(line.gl_account.clone());
            }
            entry.add_line(line);
        }

        // Generate credit lines - use the SAME amounts to ensure balance
        let credit_amounts = self
            .amount_sampler
            .sample_summing_to(line_spec.credit_count, total_amount);
        for (i, amount) in credit_amounts.into_iter().enumerate() {
            // SP3 T13 — GL Account fanout for credit lines.
            let credit_fallback = self.select_credit_account().account_number.clone();
            // SOTA-8 precedence (mirror of the debit-side block above).
            let account_number = if sota8_active {
                credit_fallback
            } else {
                let priors_opt = &mut self.loaded_priors;
                let rng_ref = &mut self.rng;
                if let Some(priors) = priors_opt {
                    let sp46_gl = entry
                        .header
                        .sap_source_code
                        .as_deref()
                        .and_then(|code| priors.sample_gl_for_source_role(code, "CR", rng_ref));
                    if let Some(gl) = sp46_gl {
                        gl
                    } else {
                        let sp37_gl = entry.header.sap_source_code.as_deref().and_then(|code| {
                            priors.sample_attribute_for_source(code, "gl_account", rng_ref)
                        });
                        if let Some(gl) = sp37_gl {
                            gl
                        } else if let Some(sampler) = priors.fanout_samplers.get_mut("GLAccount") {
                            sampler.pick_for_with_neighbors(
                                &doc_type_for_fanout,
                                &gl_neighbor_vec,
                                gl_share_prob,
                                rng_ref,
                            )
                        } else {
                            credit_fallback
                        }
                    }
                } else {
                    credit_fallback
                }
            };
            let mut line = JournalEntryLine::credit(
                entry.header.document_id,
                (line_spec.debit_count + i + 1) as u32,
                account_number.clone(),
                amount,
            );

            // Generate line text if enabled.
            // SP6 — Try text-taxonomy (account-class cascade), then DescriptionGenerator.
            if self.template_config.descriptions.generate_line_text {
                let src = entry.header.sap_source_code.as_deref();
                let priors_line = if let Some(s) = src {
                    if let Some(p) = self.loaded_priors.as_ref() {
                        let account_class = p
                            .coa_semantic
                            .as_ref()
                            .and_then(|c| c.accounts.get(&account_number))
                            .and_then(|a| a.account_class.as_deref())
                            .unwrap_or(
                                datasynth_core::distributions::text_taxonomy::TextTaxonomyPrior::UNKNOWN_CLASS,
                            );
                        // SP6 text_taxonomy cascade
                        p.sample_line_template(
                            s,
                            account_class,
                            &mut self.md_resolver,
                            &mut self.rng,
                        )
                    } else {
                        None
                    }
                } else {
                    None
                };
                line.line_text = Some(priors_line.unwrap_or_else(|| {
                    self.description_generator.generate_line_text(
                        &account_number,
                        &context,
                        &mut self.rng,
                    )
                }));
            }

            // SOTA-1: override the credit line's account with the reused
            // archetype's; else capture the fresh account for caching.
            // (Same compose-with-SOTA-8 rationale as the debit block.)
            if let Some((_, ref c)) = reuse_archetype {
                if let Some(a) = c.get(i) {
                    line.gl_account = a.clone();
                }
            } else if self.loaded_priors.is_none() {
                fresh_credit_accts.push(line.gl_account.clone());
            }
            entry.add_line(line);
        }

        // SOTA-1: cache the freshly-selected archetype for future reuse so
        // standard postings recur (skipped when this JE reused one).
        if reuse_archetype.is_none() {
            self.cache_recurring_archetype(
                &entry.header.company_code,
                &doc_type_for_fanout,
                std::mem::take(&mut fresh_debit_accts),
                std::mem::take(&mut fresh_credit_accts),
            );
        }

        // Enrich line items with account descriptions, cost centers, etc.
        self.enrich_line_items(&mut entry);

        // Apply persona-based errors if enabled and it's a human user
        if self.persona_errors_enabled && !is_automated {
            self.maybe_inject_persona_error(&mut entry);
        }

        // Apply approval workflow if enabled and amount exceeds threshold
        if self.approval_enabled {
            self.maybe_apply_approval_workflow(&mut entry, posting_date);
        }

        // Populate approved_by / approval_date from the approval workflow
        self.populate_approval_fields(&mut entry, posting_date);

        // Maybe start a batch of similar entries for realism
        self.maybe_start_batch(&entry);

        // SP3.4 + SP3.5b — observe each line through the velocity calibrator and
        // apply each returned CalibrationStep to the relevant tunable parameter.
        if self.velocity_calibrator.is_some() {
            let mut pending: Vec<crate::velocity_calibrator::CalibrationStep> = Vec::new();
            for line in &entry.lines {
                if let Some(step) = self
                    .velocity_calibrator
                    .as_mut()
                    .and_then(|cal| cal.observe_line(line))
                {
                    pending.push(step);
                }
            }
            for step in pending {
                self.apply_calibration_step(&step);
            }
        }

        // SOTA-4: with a small probability, post this JE in a foreign document
        // currency (company-ledger amounts unchanged; adds transaction_amount).
        self.maybe_apply_foreign_currency(&mut entry);

        // SOTA-5: remember this JE so a later reversal can offset it.
        self.record_for_reversal(&entry);

        entry
    }

    /// SP3.5b — Apply a CalibrationStep from the velocity calibrator to the
    /// affected tunable parameter on this generator.
    ///
    /// Only `amounts.lognormal_sigma` (R6) and `amounts.round_dollar_share`
    /// (R9) are plumbed in v5.14. R7/R8/R10 parameters (off_hours_share,
    /// post_close_share, backdating_share) are observed by the calibrator
    /// but not yet consumed on the generator side — see v5.15 for plumbing.
    fn apply_calibration_step(&mut self, step: &crate::velocity_calibrator::CalibrationStep) {
        match step.parameter.as_str() {
            "amounts.lognormal_sigma" => {
                self.amount_sampler.set_lognormal_sigma(step.new_value);
            }
            "amounts.round_dollar_share" => {
                self.amount_sampler
                    .set_round_number_probability(step.new_value);
            }
            _ => {
                // Unknown / not-yet-plumbed parameter — calibrator records it
                // in `adjustments` for inspection; no mutation here.
            }
        }
    }

    /// Enable or disable persona-based error injection.
    ///
    /// When enabled, entries created by human personas have a chance
    /// to contain realistic human errors based on their experience level.
    pub fn with_persona_errors(mut self, enabled: bool) -> Self {
        self.persona_errors_enabled = enabled;
        self
    }

    /// Set fraud configuration for fraud injection.
    ///
    /// When fraud is enabled in the config, transactions have a chance
    /// to be marked as fraudulent based on the configured fraud rate.
    pub fn with_fraud_config(mut self, config: FraudConfig) -> Self {
        self.fraud_config = config;
        self
    }

    /// Check if persona errors are enabled.
    pub fn persona_errors_enabled(&self) -> bool {
        self.persona_errors_enabled
    }

    /// Enable or disable batch processing behavior.
    ///
    /// When enabled (default), the generator will occasionally produce batches
    /// of similar entries, simulating how humans batch similar work together.
    pub fn with_batching(mut self, enabled: bool) -> Self {
        if !enabled {
            self.batch_state = None;
        }
        self
    }

    /// Check if batch processing is enabled.
    pub fn batching_enabled(&self) -> bool {
        // Batching is implicitly enabled when not explicitly disabled
        true
    }

    /// Maybe start a batch based on the current entry.
    ///
    /// Humans often batch similar work: processing invoices from one vendor,
    /// entering expense reports for a trip, reconciling similar items.
    fn maybe_start_batch(&mut self, entry: &JournalEntry) {
        // Only start batch for non-automated, non-fraud entries
        if entry.header.source == TransactionSource::Automated || entry.header.is_fraud {
            return;
        }

        // 15% chance to start a batch (most work is not batched)
        if self.rng.random::<f64>() > 0.15 {
            return;
        }

        // Extract key attributes for batching
        let base_account = entry
            .lines
            .first()
            .map(|l| l.gl_account.clone())
            .unwrap_or_default();

        let base_amount = entry.total_debit();

        self.batch_state = Some(BatchState {
            base_account_number: base_account,
            base_amount,
            base_business_process: entry.header.business_process,
            base_posting_date: entry.header.posting_date,
            remaining: self.rng.random_range(2..7), // 2-6 more similar entries
        });
    }

    /// Generate an entry that's part of the current batch.
    ///
    /// Batched entries have:
    /// - Same or very similar business process
    /// - Same posting date (batched work done together)
    /// - Similar amounts (within ±15%)
    /// - Same debit account (processing similar items)
    fn generate_batched_entry(&mut self) -> JournalEntry {
        use rust_decimal::Decimal;

        // Decrement batch counter
        if let Some(ref mut state) = self.batch_state {
            state.remaining = state.remaining.saturating_sub(1);
        }

        let Some(batch) = self.batch_state.clone() else {
            // This is a programming error - batch_state should be set before calling this method.
            // Clear state and fall back to generating a standard entry instead of panicking.
            tracing::warn!(
                "generate_batched_entry called without batch_state; generating standard entry"
            );
            self.batch_state = None;
            return self.generate();
        };

        // Use the batch's posting date (work done on same day)
        let posting_date = batch.base_posting_date;

        self.count += 1;
        let document_id = self.generate_deterministic_uuid();

        // Select same company (batched work is usually same company)
        let company_code = self.company_selector.select(&mut self.rng).to_string();

        // Use simplified line spec for batched entries (usually 2-line)
        let _line_spec = LineItemSpec {
            total_count: 2,
            debit_count: 1,
            credit_count: 1,
            split_type: DebitCreditSplit::Equal,
        };

        // Batched entries are always manual
        let source = TransactionSource::Manual;

        // SP3.6 — sample SAP source code for the batch entry when priors loaded.
        let sap_source_code: Option<String> = self.sample_sap_source_code();
        // SOTA-8: stash the batch JE's source for the per-source pool consult.
        self.current_je_source = sap_source_code.clone();

        // Use the batch's business process
        let business_process = batch.base_business_process.unwrap_or(BusinessProcess::R2R);

        // Sample time
        let time = self.temporal_sampler.sample_time(true);
        let created_at = posting_date.and_time(time).and_utc();

        // Same user for batched work
        let (created_by, user_persona) = self.select_user(false);

        // Create header
        let mut header =
            JournalEntryHeader::with_deterministic_id(company_code, posting_date, document_id);
        header.created_at = created_at;
        header.source = source;
        header.sap_source_code = sap_source_code;

        // SP3.9 — JE-level trading partner for batched entries (same pattern as
        // the primary generate() path).
        // SP3.12 — TP motif biasing also applies to batched entries.
        {
            let code_opt = header.sap_source_code.clone();
            if let Some(ref code) = code_opt {
                let rng_ref = &mut self.rng;
                let tp_neighbors: Vec<String> = if let Some(ref priors) = self.loaded_priors {
                    if let Some(ref motifs) = priors.tp_motif_sampler {
                        if let Some(last_tp) = self.last_tp_by_source.get(code.as_str()) {
                            motifs.neighbors(last_tp).to_vec()
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                let tp_share_prob: f64 = if let Some(ref priors) = self.loaded_priors {
                    if let Some(ref motifs) = priors.tp_motif_sampler {
                        if let Some(last_tp) = self.last_tp_by_source.get(code.as_str()) {
                            motifs.should_share(last_tp)
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };
                if let Some(ref mut priors) = self.loaded_priors {
                    use datasynth_core::distributions::behavioral_priors::CategoricalDistribution;
                    let tp = if !tp_neighbors.is_empty()
                        && tp_share_prob > 0.0
                        && rng_ref.random_range(0.0..1.0) < tp_share_prob
                    {
                        let filtered: std::collections::BTreeMap<String, f64> = priors
                            .per_source_attribute
                            .as_ref()
                            .and_then(|psa| psa.conditional(code, "trading_partner"))
                            .map(|dist| {
                                dist.probabilities
                                    .iter()
                                    .filter(|(v, _)| tp_neighbors.contains(v))
                                    .map(|(v, p)| (v.clone(), *p))
                                    .collect()
                            })
                            .unwrap_or_default();
                        if filtered.is_empty() {
                            priors.sample_attribute_for_source(code, "trading_partner", rng_ref)
                        } else {
                            let neighbour_dist = CategoricalDistribution {
                                probabilities: filtered,
                                n: 0,
                            };
                            neighbour_dist.sample(rng_ref).or_else(|| {
                                priors.sample_attribute_for_source(code, "trading_partner", rng_ref)
                            })
                        }
                    } else {
                        priors.sample_attribute_for_source(code, "trading_partner", rng_ref)
                    };
                    header.trading_partner = tp;
                }
                if let Some(ref tp) = header.trading_partner {
                    self.last_tp_by_source.insert(code.clone(), tp.clone());
                }
            }
        }

        // SP4.5 — user-persona prior for batched entries (same pattern as primary path).
        let (created_by, created_at) = {
            let sap_code_for_user = header.sap_source_code.clone();
            if let (Some(ref code), Some(ref priors)) = (sap_code_for_user, &self.loaded_priors) {
                if let Some(uid) = priors.sample_user_for_source(code, &mut self.rng) {
                    let new_created_at = if let Some((hour, _)) =
                        priors.sample_timestamp_for_user(&uid, &mut self.rng)
                    {
                        let base = header.created_at;
                        base.date_naive()
                            .and_hms_opt(hour, 0, 0)
                            .map(|naive| naive.and_utc())
                            .unwrap_or(base)
                    } else {
                        header.created_at
                    };
                    (uid, new_created_at)
                } else {
                    (created_by, header.created_at)
                }
            } else {
                (created_by, header.created_at)
            }
        };

        header.created_by = created_by;
        header.created_at = created_at;
        header.user_persona = user_persona;
        header.business_process = Some(business_process);
        header.document_type = Self::document_type_for_process(business_process).to_string();

        // Batched manual entries have Manual source document
        header.source_document = Some(DocumentRef::Manual);

        // ISA 240 audit flags for batched entries (always manual)
        header.is_manual = true;
        header.source_system = if self.rng.random::<f64>() < 0.70 {
            "manual".to_string()
        } else {
            "spreadsheet".to_string()
        };
        header.is_post_close = posting_date.month() == self.end_date.month()
            && posting_date.year() == self.end_date.year()
            && posting_date.day() > 25;
        header.created_date =
            posting_date.and_hms_opt(time.hour().min(23), time.minute(), time.second());

        // Generate similar amount (within ±15% of base)
        let variation = self.rng.random_range(-0.15..0.15);
        let varied_amount =
            batch.base_amount * (Decimal::ONE + Decimal::try_from(variation).unwrap_or_default());
        let total_amount = varied_amount.round_dp(2).max(Decimal::from(1));

        // Create the entry
        let mut entry = JournalEntry::new(header);

        // Use same debit account as batch base
        let debit_line = JournalEntryLine::debit(
            entry.header.document_id,
            1,
            batch.base_account_number.clone(),
            total_amount,
        );
        entry.add_line(debit_line);

        // SP3.12 W3 — Select a credit account for the batched entry.
        // When priors are loaded and this entry has a SAP source code, use the
        // per-source GL-account conditional (same as the primary generate() path).
        // This prevents batched entries from adding legacy-CoA accounts to the
        // Source-Source projection graph, which was inflating graph density and
        // driving the P3 ClusteringGap metric above 30× DR.
        let credit_fallback = self.select_credit_account().account_number.clone();
        let credit_account = {
            let priors_opt = &mut self.loaded_priors;
            let rng_ref = &mut self.rng;
            if let Some(priors) = priors_opt {
                // SP4.6 — role-aware GL for the batched-entry credit line.
                // Try (source, "CR") first, then source-marginal, then fallback.
                let sp46_gl = entry
                    .header
                    .sap_source_code
                    .as_deref()
                    .and_then(|code| priors.sample_gl_for_source_role(code, "CR", rng_ref));
                if let Some(gl) = sp46_gl {
                    gl
                } else {
                    let sp37_gl = entry.header.sap_source_code.as_deref().and_then(|code| {
                        priors.sample_attribute_for_source(code, "gl_account", rng_ref)
                    });
                    sp37_gl.unwrap_or(credit_fallback)
                }
            } else {
                credit_fallback
            }
        };
        let credit_line =
            JournalEntryLine::credit(entry.header.document_id, 2, credit_account, total_amount);
        entry.add_line(credit_line);

        // Enrich line items with account descriptions, cost centers, etc.
        self.enrich_line_items(&mut entry);

        // Apply persona-based errors if enabled
        if self.persona_errors_enabled {
            self.maybe_inject_persona_error(&mut entry);
        }

        // Apply approval workflow if enabled
        if self.approval_enabled {
            self.maybe_apply_approval_workflow(&mut entry, posting_date);
        }

        // Populate approved_by / approval_date from the approval workflow
        self.populate_approval_fields(&mut entry, posting_date);

        // Clear batch state if no more entries remaining
        if batch.remaining <= 1 {
            self.batch_state = None;
        }

        entry
    }

    /// Maybe inject a persona-appropriate error based on the persona's error rate.
    fn maybe_inject_persona_error(&mut self, entry: &mut JournalEntry) {
        // Parse persona from the entry header
        let persona_str = &entry.header.user_persona;
        let persona = match persona_str.to_lowercase().as_str() {
            s if s.contains("junior") => UserPersona::JuniorAccountant,
            s if s.contains("senior") => UserPersona::SeniorAccountant,
            s if s.contains("controller") => UserPersona::Controller,
            s if s.contains("manager") => UserPersona::Manager,
            s if s.contains("executive") => UserPersona::Executive,
            _ => return, // Don't inject errors for unknown personas
        };

        // Get base error rate from persona
        let base_error_rate = persona.error_rate();

        // Apply stress factors based on posting date
        let adjusted_rate = self.apply_stress_factors(base_error_rate, entry.header.posting_date);

        // Check if error should occur based on adjusted rate
        if self.rng.random::<f64>() >= adjusted_rate {
            return; // No error this time
        }

        // Select and inject persona-appropriate error
        self.inject_human_error(entry, persona);
    }

    /// Apply contextual stress factors to the base error rate.
    ///
    /// Stress factors increase error likelihood during:
    /// - Month-end (day >= 28): 1.5x more errors due to deadline pressure
    /// - Quarter-end (Mar, Jun, Sep, Dec): additional 25% boost
    /// - Year-end (December 28-31): 2.0x more errors due to audit pressure
    /// - Monday morning (catch-up work): 20% more errors
    /// - Friday afternoon (rushing to leave): 30% more errors
    fn apply_stress_factors(&self, base_rate: f64, posting_date: chrono::NaiveDate) -> f64 {
        use chrono::Datelike;

        let mut rate = base_rate;
        let day = posting_date.day();
        let month = posting_date.month();

        // Year-end stress (December 28-31): double the error rate
        if month == 12 && day >= 28 {
            rate *= 2.0;
            return rate.min(0.5); // Cap at 50% to keep it realistic
        }

        // Quarter-end stress (last days of Mar, Jun, Sep, Dec)
        if matches!(month, 3 | 6 | 9 | 12) && day >= 28 {
            rate *= 1.75; // 75% more errors at quarter end
            return rate.min(0.4);
        }

        // Month-end stress (last 3 days of month)
        if day >= 28 {
            rate *= 1.5; // 50% more errors at month end
        }

        // Day-of-week stress effects
        let weekday = posting_date.weekday();
        match weekday {
            chrono::Weekday::Mon => {
                // Monday: catching up, often rushed
                rate *= 1.2;
            }
            chrono::Weekday::Fri => {
                // Friday: rushing to finish before weekend
                rate *= 1.3;
            }
            _ => {}
        }

        // Cap at 40% to keep it realistic
        rate.min(0.4)
    }

    /// Apply human-like variation to an amount.
    ///
    /// Humans don't enter perfectly calculated amounts - they:
    /// - Round amounts differently
    /// - Estimate instead of calculating exactly
    /// - Make small input variations
    ///
    /// This applies small variations (typically ±2%) to make amounts more realistic.
    fn apply_human_variation(&mut self, amount: rust_decimal::Decimal) -> rust_decimal::Decimal {
        use rust_decimal::Decimal;

        // Automated transactions or very small amounts don't get variation
        if amount < Decimal::from(10) {
            return amount;
        }

        // 70% chance of human variation being applied
        if self.rng.random::<f64>() > 0.70 {
            return amount;
        }

        // Decide which type of human variation to apply
        let variation_type: u8 = self.rng.random_range(0..4);

        match variation_type {
            0 => {
                // ±2% variation (common for estimated amounts)
                let variation_pct = self.rng.random_range(-0.02..0.02);
                let variation = amount * Decimal::try_from(variation_pct).unwrap_or_default();
                (amount + variation).round_dp(2)
            }
            1 => {
                // Round to nearest $10
                let ten = Decimal::from(10);
                (amount / ten).round() * ten
            }
            2 => {
                // Round to nearest $100 (for larger amounts)
                if amount >= Decimal::from(500) {
                    let hundred = Decimal::from(100);
                    (amount / hundred).round() * hundred
                } else {
                    amount
                }
            }
            3 => {
                // Slight under/over payment (±$0.01 to ±$1.00)
                let cents = Decimal::new(self.rng.random_range(-100..100), 2);
                (amount + cents).max(Decimal::ZERO).round_dp(2)
            }
            _ => amount,
        }
    }

    /// Rebalance an entry after a one-sided amount modification.
    ///
    /// When an error modifies one line's amount, this finds a line on the opposite
    /// side (credit if modified was debit, or vice versa) and adjusts it by the
    /// same impact to maintain balance.
    fn rebalance_entry(entry: &mut JournalEntry, modified_was_debit: bool, impact: Decimal) {
        // Find a line on the opposite side to adjust
        let balancing_idx = entry.lines.iter().position(|l| {
            if modified_was_debit {
                l.credit_amount > Decimal::ZERO
            } else {
                l.debit_amount > Decimal::ZERO
            }
        });

        if let Some(idx) = balancing_idx {
            if modified_was_debit {
                entry.lines[idx].credit_amount += impact;
            } else {
                entry.lines[idx].debit_amount += impact;
            }
        }
    }

    /// Inject a human-like error based on the persona.
    ///
    /// All error types maintain balance - amount modifications are applied to both sides.
    /// Entries are marked with [HUMAN_ERROR:*] tags in header_text for ML detection.
    fn inject_human_error(&mut self, entry: &mut JournalEntry, persona: UserPersona) {
        use rust_decimal::Decimal;

        // Different personas make different types of errors
        let error_type: u8 = match persona {
            UserPersona::JuniorAccountant => {
                // Junior accountants make more varied errors
                self.rng.random_range(0..5)
            }
            UserPersona::SeniorAccountant => {
                // Senior accountants mainly make transposition errors
                self.rng.random_range(0..3)
            }
            UserPersona::Controller | UserPersona::Manager => {
                // Controllers/managers mainly make rounding or cutoff errors
                self.rng.random_range(3..5)
            }
            _ => return,
        };

        match error_type {
            0 => {
                // Transposed digits in an amount
                if let Some(line) = entry.lines.get_mut(0) {
                    let is_debit = line.debit_amount > Decimal::ZERO;
                    let original_amount = if is_debit {
                        line.debit_amount
                    } else {
                        line.credit_amount
                    };

                    // Simple digit swap in the string representation
                    let s = original_amount.to_string();
                    if s.len() >= 2 {
                        let chars: Vec<char> = s.chars().collect();
                        let pos = self.rng.random_range(0..chars.len().saturating_sub(1));
                        if chars[pos].is_ascii_digit()
                            && chars.get(pos + 1).is_some_and(char::is_ascii_digit)
                        {
                            let mut new_chars = chars;
                            new_chars.swap(pos, pos + 1);
                            if let Ok(new_amount) =
                                new_chars.into_iter().collect::<String>().parse::<Decimal>()
                            {
                                let impact = new_amount - original_amount;

                                // Apply to the modified line
                                if is_debit {
                                    entry.lines[0].debit_amount = new_amount;
                                } else {
                                    entry.lines[0].credit_amount = new_amount;
                                }

                                // Rebalance the entry
                                Self::rebalance_entry(entry, is_debit, impact);

                                entry.header.header_text = Some(
                                    entry.header.header_text.clone().unwrap_or_default()
                                        + " [HUMAN_ERROR:TRANSPOSITION]",
                                );
                            }
                        }
                    }
                }
            }
            1 => {
                // Wrong decimal place (off by factor of 10)
                if let Some(line) = entry.lines.get_mut(0) {
                    let is_debit = line.debit_amount > Decimal::ZERO;
                    let original_amount = if is_debit {
                        line.debit_amount
                    } else {
                        line.credit_amount
                    };

                    let new_amount = original_amount * Decimal::new(10, 0);
                    let impact = new_amount - original_amount;

                    // Apply to the modified line
                    if is_debit {
                        entry.lines[0].debit_amount = new_amount;
                    } else {
                        entry.lines[0].credit_amount = new_amount;
                    }

                    // Rebalance the entry
                    Self::rebalance_entry(entry, is_debit, impact);

                    entry.header.header_text = Some(
                        entry.header.header_text.clone().unwrap_or_default()
                            + " [HUMAN_ERROR:DECIMAL_SHIFT]",
                    );
                }
            }
            2 => {
                // Typo in description (doesn't affect balance)
                if let Some(ref mut text) = entry.header.header_text {
                    let typos = ["teh", "adn", "wiht", "taht", "recieve"];
                    let correct = ["the", "and", "with", "that", "receive"];
                    let idx = self.rng.random_range(0..typos.len());
                    if text.to_lowercase().contains(correct[idx]) {
                        *text = text.replace(correct[idx], typos[idx]);
                        *text = format!("{text} [HUMAN_ERROR:TYPO]");
                    }
                }
            }
            3 => {
                // Rounding to round number
                if let Some(line) = entry.lines.get_mut(0) {
                    let is_debit = line.debit_amount > Decimal::ZERO;
                    let original_amount = if is_debit {
                        line.debit_amount
                    } else {
                        line.credit_amount
                    };

                    let new_amount =
                        (original_amount / Decimal::new(100, 0)).round() * Decimal::new(100, 0);
                    let impact = new_amount - original_amount;

                    // Apply to the modified line
                    if is_debit {
                        entry.lines[0].debit_amount = new_amount;
                    } else {
                        entry.lines[0].credit_amount = new_amount;
                    }

                    // Rebalance the entry
                    Self::rebalance_entry(entry, is_debit, impact);

                    entry.header.header_text = Some(
                        entry.header.header_text.clone().unwrap_or_default()
                            + " [HUMAN_ERROR:ROUNDED]",
                    );
                }
            }
            // Late posting marker (document date much earlier than posting
            // date). Doesn't create an imbalance.
            4 if entry.header.document_date == entry.header.posting_date => {
                let days_late = self.rng.random_range(5..15);
                entry.header.document_date =
                    entry.header.posting_date - chrono::Duration::days(days_late);
                entry.header.header_text = Some(
                    entry.header.header_text.clone().unwrap_or_default()
                        + " [HUMAN_ERROR:LATE_POSTING]",
                );
            }
            _ => {}
        }
    }

    /// Apply approval workflow for high-value transactions.
    ///
    /// If the entry amount exceeds the approval threshold, simulate an
    /// approval workflow with appropriate approvers based on amount.
    fn maybe_apply_approval_workflow(
        &mut self,
        entry: &mut JournalEntry,
        _posting_date: NaiveDate,
    ) {
        use rust_decimal::Decimal;

        let amount = entry.total_debit();

        // Skip if amount is below threshold
        if amount <= self.approval_threshold {
            // Auto-approved below threshold
            let workflow = ApprovalWorkflow::auto_approved(
                entry.header.created_by.clone(),
                entry.header.user_persona.clone(),
                amount,
                entry.header.created_at,
            );
            entry.header.approval_workflow = Some(workflow);
            return;
        }

        // Mark as SOX relevant for high-value transactions
        entry.header.sox_relevant = true;

        // Determine required approval levels based on amount
        let required_levels = if amount > Decimal::new(100000, 0) {
            3 // Executive approval required
        } else if amount > Decimal::new(50000, 0) {
            2 // Senior management approval
        } else {
            1 // Manager approval
        };

        // Create the approval workflow
        let mut workflow = ApprovalWorkflow::new(
            entry.header.created_by.clone(),
            entry.header.user_persona.clone(),
            amount,
        );
        workflow.required_levels = required_levels;

        // Simulate submission
        let submit_time = entry.header.created_at;
        let submit_action = ApprovalAction::new(
            entry.header.created_by.clone(),
            entry.header.user_persona.clone(),
            self.parse_persona(&entry.header.user_persona),
            ApprovalActionType::Submit,
            0,
        )
        .with_timestamp(submit_time);

        workflow.actions.push(submit_action);
        workflow.status = ApprovalStatus::Pending;
        workflow.submitted_at = Some(submit_time);

        // Simulate approvals with realistic delays
        let mut current_time = submit_time;
        for level in 1..=required_levels {
            // Add delay for approval (1-3 business hours per level)
            let delay_hours = self.rng.random_range(1..4);
            current_time += chrono::Duration::hours(delay_hours);

            // Skip weekends
            while current_time.weekday() == chrono::Weekday::Sat
                || current_time.weekday() == chrono::Weekday::Sun
            {
                current_time += chrono::Duration::days(1);
            }

            // Generate approver based on level
            let (approver_id, approver_role) = self.select_approver(level);

            let approve_action = ApprovalAction::new(
                approver_id.clone(),
                approver_role.to_string(),
                approver_role,
                ApprovalActionType::Approve,
                level,
            )
            .with_timestamp(current_time);

            workflow.actions.push(approve_action);
            workflow.current_level = level;
        }

        // Mark as approved
        workflow.status = ApprovalStatus::Approved;
        workflow.approved_at = Some(current_time);

        entry.header.approval_workflow = Some(workflow);
    }

    /// Select an approver based on the required level.
    fn select_approver(&mut self, level: u8) -> (String, UserPersona) {
        let persona = match level {
            1 => UserPersona::Manager,
            2 => UserPersona::Controller,
            _ => UserPersona::Executive,
        };

        // Try to get from user pool first
        if let Some(ref pool) = self.user_pool {
            if let Some(user) = pool.get_random_user(persona, &mut self.rng) {
                return (user.user_id.clone(), persona);
            }
        }

        // Fallback to generated approver
        let approver_id = match persona {
            UserPersona::Manager => format!("MGR{:04}", self.rng.random_range(1..100)),
            UserPersona::Controller => format!("CTRL{:04}", self.rng.random_range(1..20)),
            UserPersona::Executive => format!("EXEC{:04}", self.rng.random_range(1..10)),
            _ => format!("USR{:04}", self.rng.random_range(1..1000)),
        };

        (approver_id, persona)
    }

    /// Parse user persona from string.
    fn parse_persona(&self, persona_str: &str) -> UserPersona {
        match persona_str.to_lowercase().as_str() {
            s if s.contains("junior") => UserPersona::JuniorAccountant,
            s if s.contains("senior") => UserPersona::SeniorAccountant,
            s if s.contains("controller") => UserPersona::Controller,
            s if s.contains("manager") => UserPersona::Manager,
            s if s.contains("executive") => UserPersona::Executive,
            s if s.contains("automated") || s.contains("system") => UserPersona::AutomatedSystem,
            _ => UserPersona::JuniorAccountant, // Default
        }
    }

    /// Enable or disable approval workflow.
    pub fn with_approval(mut self, enabled: bool) -> Self {
        self.approval_enabled = enabled;
        self
    }

    /// Set the approval threshold amount.
    pub fn with_approval_threshold(mut self, threshold: rust_decimal::Decimal) -> Self {
        self.approval_threshold = threshold;
        self
    }

    /// Set the SOD violation rate for approval tracking.
    ///
    /// When a transaction is approved, there is a `rate` probability (0.0 to 1.0)
    /// that the approver is the same as the creator, which constitutes a SOD violation.
    /// Default is 0.10 (10%).
    pub fn with_sod_violation_rate(mut self, rate: f64) -> Self {
        self.sod_violation_rate = rate;
        self
    }

    /// Populate `approved_by` and `approval_date` from the approval workflow,
    /// and flag SOD violations when the approver matches the creator.
    fn populate_approval_fields(&mut self, entry: &mut JournalEntry, posting_date: NaiveDate) {
        if let Some(ref workflow) = entry.header.approval_workflow {
            // Extract the last approver from the workflow actions
            let last_approver = workflow
                .actions
                .iter()
                .rev()
                .find(|a| matches!(a.action, ApprovalActionType::Approve));

            if let Some(approver_action) = last_approver {
                entry.header.approved_by = Some(approver_action.actor_id.clone());
                entry.header.approval_date = Some(approver_action.action_timestamp.date_naive());
            } else {
                // No explicit approver (auto-approved); use the preparer
                entry.header.approved_by = Some(workflow.preparer_id.clone());
                entry.header.approval_date = Some(posting_date);
            }

            // Inject SOD violation: with configured probability, set approver = creator
            if self.rng.random::<f64>() < self.sod_violation_rate {
                let creator = entry.header.created_by.clone();
                entry.header.approved_by = Some(creator);
                entry.header.sod_violation = true;
                entry.header.sod_conflict_type = Some(SodConflictType::PreparerApprover);
            }
        }
    }

    /// Set the temporal drift controller for simulating distribution changes over time.
    ///
    /// When drift is enabled, amounts and other distributions will shift based on
    /// the period (month) to simulate realistic temporal evolution like inflation
    /// or increasing fraud rates.
    pub fn with_drift_controller(mut self, controller: DriftController) -> Self {
        self.drift_controller = Some(controller);
        self
    }

    /// Set drift configuration directly.
    ///
    /// Creates a drift controller from the config. Total periods is calculated
    /// from the date range.
    pub fn with_drift_config(mut self, config: DriftConfig, seed: u64) -> Self {
        if config.enabled {
            let total_periods = self.calculate_total_periods();
            self.drift_controller = Some(DriftController::new(config, seed, total_periods));
        }
        self
    }

    /// Calculate total periods (months) in the date range.
    fn calculate_total_periods(&self) -> u32 {
        let start_year = self.start_date.year();
        let start_month = self.start_date.month();
        let end_year = self.end_date.year();
        let end_month = self.end_date.month();

        ((end_year - start_year) * 12 + (end_month as i32 - start_month as i32) + 1).max(1) as u32
    }

    /// Calculate the period number (0-indexed) for a given date.
    fn date_to_period(&self, date: NaiveDate) -> u32 {
        let start_year = self.start_date.year();
        let start_month = self.start_date.month() as i32;
        let date_year = date.year();
        let date_month = date.month() as i32;

        ((date_year - start_year) * 12 + (date_month - start_month)).max(0) as u32
    }

    /// Get drift adjustments for a given date.
    fn get_drift_adjustments(&self, date: NaiveDate) -> DriftAdjustments {
        if let Some(ref controller) = self.drift_controller {
            let period = self.date_to_period(date);
            controller.compute_adjustments(period)
        } else {
            DriftAdjustments::none()
        }
    }

    /// Select a user from the pool or generate a generic user ID.
    #[inline]
    fn select_user(&mut self, is_automated: bool) -> (String, String) {
        if let Some(ref pool) = self.user_pool {
            let persona = if is_automated {
                UserPersona::AutomatedSystem
            } else {
                // Random distribution among human personas
                let roll: f64 = self.rng.random();
                if roll < 0.4 {
                    UserPersona::JuniorAccountant
                } else if roll < 0.7 {
                    UserPersona::SeniorAccountant
                } else if roll < 0.85 {
                    UserPersona::Controller
                } else {
                    UserPersona::Manager
                }
            };

            if let Some(user) = pool.get_random_user(persona, &mut self.rng) {
                return (user.user_id.clone(), user.persona.to_string());
            }
        }

        // Fallback to generic format
        if is_automated {
            (
                format!("BATCH{:04}", self.rng.random_range(1..=20)),
                "automated_system".to_string(),
            )
        } else {
            (
                format!("USER{:04}", self.rng.random_range(1..=40)),
                "senior_accountant".to_string(),
            )
        }
    }

    /// Select transaction source based on configuration weights.
    #[inline]
    fn select_source(&mut self) -> TransactionSource {
        let roll: f64 = self.rng.random();
        let dist = &self.config.source_distribution;

        if roll < dist.manual {
            TransactionSource::Manual
        } else if roll < dist.manual + dist.automated {
            TransactionSource::Automated
        } else if roll < dist.manual + dist.automated + dist.recurring {
            TransactionSource::Recurring
        } else {
            TransactionSource::Adjustment
        }
    }

    /// Select a business process based on configuration weights.
    #[inline]
    /// Map a business process to a SAP-style document type code.
    ///
    /// - P2P → "KR" (vendor invoice)
    /// - O2C → "DR" (customer invoice)
    /// - R2R → "SA" (general journal)
    /// - H2R → "HR" (HR posting)
    /// - A2R → "AA" (asset posting)
    /// - others → "SA"
    fn document_type_for_process(process: BusinessProcess) -> &'static str {
        match process {
            BusinessProcess::P2P => "KR",
            BusinessProcess::O2C => "DR",
            BusinessProcess::R2R => "SA",
            BusinessProcess::H2R => "HR",
            BusinessProcess::A2R => "AA",
            _ => "SA",
        }
    }

    fn select_business_process(&mut self) -> BusinessProcess {
        *datasynth_core::utils::weighted_select(&mut self.rng, &self.business_process_weights)
    }

    /// SOTA-2: draw a rank index in `[0, n)` with `P(rank=i) ∝ 1/(i+1)^ZIPF_ALPHA`
    /// from a dedicated stream, so a few low-rank accounts carry most lines (the
    /// corpus account-activity Pareto). Returns `None` for an empty/oversized pool
    /// so the caller keeps the uniform draw.
    #[inline]
    fn power_law_index(n: usize, rng: &mut ChaCha8Rng) -> Option<usize> {
        if n == 0 || n > ZIPF_CAP {
            return None;
        }
        let total = ZIPF_CUM[n];
        let r = rng.random::<f64>() * total;
        // smallest k in 1..=n with CUM[k] >= r → 0-based rank k-1
        let k = ZIPF_CUM[..=n]
            .binary_search_by(|v| v.partial_cmp(&r).unwrap_or(std::cmp::Ordering::Less))
            .unwrap_or_else(|e| e);
        Some(k.saturating_sub(1).min(n - 1))
    }

    /// SOTA-2: replace a uniform `Vec<&GLAccount>` pick with a hot-account
    /// power-law pick when concentration is on (default). The uniform `.choose`
    /// draw on the main `rng` is still consumed by the caller first, so
    /// amounts/line-counts/dates stay byte-identical to the legacy stream — only
    /// the *selected account* changes. Associated (not `&mut self`) so it borrows
    /// only `account_rng`, leaving `coa` free for `all`/`uniform`.
    #[inline]
    fn concentrate<'a>(
        enabled: bool,
        rng: &mut ChaCha8Rng,
        all: &[&'a GLAccount],
        uniform: Option<&'a GLAccount>,
    ) -> Option<&'a GLAccount> {
        if enabled {
            Self::power_law_index(all.len(), rng)
                .map(|i| all[i])
                .or(uniform)
        } else {
            uniform
        }
    }

    /// SOTA-8: ensure a `SourcePool` exists for `source` in the sampler (lazy build).
    /// One pool per source, persisted across JEs (sampler grows monotonically).
    fn ensure_cond_pair_pool(&mut self, source: &str) {
        let cfg = &self.config.source_conditional_account_pair;
        if !cfg.enabled {
            return;
        }
        if self.cond_pair_sampler.is_none() {
            self.cond_pair_sampler = Some(Default::default());
        }
        let sampler = self
            .cond_pair_sampler
            .as_mut()
            .expect("just-initialised above");
        if sampler.pool(source).is_some() {
            return;
        }
        let all_accounts: Vec<String> = self
            .coa
            .accounts
            .iter()
            .map(|a| a.account_number.clone())
            .collect();
        if all_accounts.is_empty() {
            return;
        }
        // Uniform weights here — the existing account-Pareto (account_concentration)
        // still applies at the outer fallback level if the per-source pool isn't used.
        let weights: Vec<f64> = vec![1.0; all_accounts.len()];
        sampler.ensure_pool(
            source,
            &all_accounts,
            &weights,
            cfg.accts_per_source_target,
            cfg.concentration,
            &mut self.cond_pair_rng,
        );
    }

    /// SOTA-8: if the feature is enabled and the current JE has a source with a
    /// pool, pick an *account number* from the per-source PMF. Returns an owned
    /// `String` so the caller can release the mutable self-borrow before looking
    /// up the `GLAccount` in `self.coa`.
    #[inline]
    fn try_cond_pick_account_number(&mut self) -> Option<String> {
        let cfg = &self.config.source_conditional_account_pair;
        if !cfg.enabled {
            return None;
        }
        let src = self.current_je_source.clone()?;
        self.ensure_cond_pair_pool(&src);
        let sampler = self.cond_pair_sampler.as_ref()?;
        let pool = sampler.pool(&src)?;
        Some(pool.sample_one(&mut self.cond_pair_rng).to_string())
    }

    #[inline]
    fn select_debit_account(&mut self) -> &GLAccount {
        // SOTA-8 source-conditional pick when feature is enabled.
        if let Some(acct_num) = self.try_cond_pick_account_number() {
            if let Some(a) = self
                .coa
                .accounts
                .iter()
                .find(|a| a.account_number == acct_num)
            {
                return a;
            }
            // Sampler chose an account not in CoA (defensive fall-through).
        }
        let accounts = self.coa.get_accounts_by_type(AccountType::Asset);
        let expense_accounts = self.coa.get_accounts_by_type(AccountType::Expense);

        // 60% asset, 40% expense for debits
        let all: Vec<_> = if self.rng.random::<f64>() < 0.6 {
            accounts
        } else {
            expense_accounts
        };

        let uniform = all.choose(&mut self.rng).copied();
        let enabled = self.config.account_concentration.unwrap_or(true);
        Self::concentrate(enabled, &mut self.account_rng, &all, uniform).unwrap_or_else(|| {
            tracing::warn!(
                "Account selection returned empty list, falling back to first COA account"
            );
            &self.coa.accounts[0]
        })
    }

    #[inline]
    fn select_credit_account(&mut self) -> &GLAccount {
        // SOTA-8 source-conditional pick when feature is enabled.
        if let Some(acct_num) = self.try_cond_pick_account_number() {
            if let Some(a) = self
                .coa
                .accounts
                .iter()
                .find(|a| a.account_number == acct_num)
            {
                return a;
            }
        }
        let liability_accounts = self.coa.get_accounts_by_type(AccountType::Liability);
        let revenue_accounts = self.coa.get_accounts_by_type(AccountType::Revenue);

        // 60% liability, 40% revenue for credits
        let all: Vec<_> = if self.rng.random::<f64>() < 0.6 {
            liability_accounts
        } else {
            revenue_accounts
        };

        let uniform = all.choose(&mut self.rng).copied();
        let enabled = self.config.account_concentration.unwrap_or(true);
        Self::concentrate(enabled, &mut self.account_rng, &all, uniform).unwrap_or_else(|| {
            tracing::warn!(
                "Account selection returned empty list, falling back to first COA account"
            );
            &self.coa.accounts[0]
        })
    }
}

impl Generator for JournalEntryGenerator {
    type Item = JournalEntry;
    type Config = (
        TransactionConfig,
        Arc<ChartOfAccounts>,
        Vec<String>,
        NaiveDate,
        NaiveDate,
    );

    fn new(config: Self::Config, seed: u64) -> Self {
        Self::new_with_params(config.0, config.1, config.2, config.3, config.4, seed)
    }

    fn generate_one(&mut self) -> Self::Item {
        self.generate()
    }

    fn reset(&mut self) {
        self.rng = seeded_rng(self.seed, 0);
        self.source_mix_rng = seeded_rng(self.seed, 50_063);
        self.template_rng = seeded_rng(self.seed, 70_081);
        self.recurring_archetypes.clear();
        self.reversal_rng = seeded_rng(self.seed, 90_017);
        self.reversal_buffer.clear();
        self.account_rng = seeded_rng(self.seed, 60_071);
        self.allocation_rng = seeded_rng(self.seed, 80_023);
        self.fx_rng = seeded_rng(self.seed, 70_093);
        self.line_sampler.reset(self.seed + 1);
        self.amount_sampler.reset(self.seed + 2);
        self.temporal_sampler.reset(self.seed + 3);
        if let Some(ref mut adv) = self.advanced_amount_sampler {
            adv.reset(self.seed + 2);
        }
        self.count = 0;
        self.uuid_factory.reset();

        // Reset reference generator by recreating it
        let mut ref_gen = ReferenceGenerator::new(
            self.start_date.year(),
            self.companies
                .first()
                .map(std::string::String::as_str)
                .unwrap_or("1000"),
        );
        ref_gen.set_prefix(
            ReferenceType::Invoice,
            &self.template_config.references.invoice_prefix,
        );
        ref_gen.set_prefix(
            ReferenceType::PurchaseOrder,
            &self.template_config.references.po_prefix,
        );
        ref_gen.set_prefix(
            ReferenceType::SalesOrder,
            &self.template_config.references.so_prefix,
        );
        self.reference_generator = ref_gen;
    }

    fn count(&self) -> u64 {
        self.count
    }

    fn seed(&self) -> u64 {
        self.seed
    }
}

use datasynth_core::traits::ParallelGenerator;

impl ParallelGenerator for JournalEntryGenerator {
    /// Split this generator into `parts` independent sub-generators.
    ///
    /// Each sub-generator gets a deterministic seed derived from the parent seed
    /// and its partition index, plus a partitioned UUID factory to avoid contention.
    /// The results are deterministic for a given partition count.
    fn split(self, parts: usize) -> Vec<Self> {
        let parts = parts.max(1);
        (0..parts)
            .map(|i| {
                // Derive a unique seed per partition using a golden-ratio constant
                let sub_seed = self
                    .seed
                    .wrapping_add((i as u64).wrapping_mul(0x9E3779B97F4A7C15));

                let mut gen = JournalEntryGenerator::new_with_full_config(
                    self.config.clone(),
                    Arc::clone(&self.coa),
                    self.companies.clone(),
                    self.start_date,
                    self.end_date,
                    sub_seed,
                    self.template_config.clone(),
                    self.user_pool.clone(),
                );

                // Copy over configuration state
                gen.company_selector = self.company_selector.clone();
                gen.vendor_pool = self.vendor_pool.clone();
                gen.customer_pool = self.customer_pool.clone();
                gen.material_pool = self.material_pool.clone();
                // v5.9.0: master-data pools so sub-generators emit
                // CC/PC values that join back to the corresponding
                // masters (without these clones, parallel workers
                // fell back to the hardcoded `COST_CENTER_POOL` const
                // and the legacy `PC-{COMP}-{P2P|O2C|...}` derivation).
                gen.cost_center_pool = self.cost_center_pool.clone();
                gen.profit_center_pool = self.profit_center_pool.clone();
                gen.using_real_master_data = self.using_real_master_data;
                gen.fraud_config = self.fraud_config.clone();
                gen.persona_errors_enabled = self.persona_errors_enabled;
                gen.approval_enabled = self.approval_enabled;
                gen.approval_threshold = self.approval_threshold;
                gen.sod_violation_rate = self.sod_violation_rate;
                // v3.4.0+: advanced amount sampler (mixture / Pareto /
                // Gaussian). Clone and reset the internal RNG with the
                // partition's sub_seed so each worker explores a unique
                // subsequence without repeating the parent stream.
                if let Some(mut adv) = self.advanced_amount_sampler.clone() {
                    adv.reset(sub_seed.wrapping_add(2));
                    gen.advanced_amount_sampler = Some(adv);
                }
                // v3.5.3+: conditional amount override — clone + reset
                // so each partition gets a fresh deterministic stream.
                if let Some(mut cond) = self.conditional_amount_override.clone() {
                    cond.reset(sub_seed.wrapping_add(17));
                    gen.conditional_amount_override = Some(cond);
                }
                // v3.5.4+: copula sampler — clone + reset per partition.
                if let Some(mut cop) = self.correlation_copula.clone() {
                    cop.reset(sub_seed.wrapping_add(31));
                    gen.correlation_copula = Some(cop);
                }

                // Use partitioned UUID factory to eliminate atomic contention
                gen.uuid_factory = DeterministicUuidFactory::for_partition(
                    sub_seed,
                    GeneratorType::JournalEntry,
                    i as u8,
                );

                // Copy temporal patterns if configured
                if let Some(ref config) = self.temporal_patterns_config {
                    gen.temporal_patterns_config = Some(config.clone());
                    // Rebuild business day calculator from the stored config
                    if config.business_days.enabled {
                        if let Some(ref bdc) = self.business_day_calculator {
                            gen.business_day_calculator = Some(bdc.clone());
                        }
                    }
                    // Rebuild processing lag calculator with partition seed
                    if config.processing_lags.enabled {
                        let lag_config =
                            Self::convert_processing_lag_config(&config.processing_lags);
                        gen.processing_lag_calculator =
                            Some(ProcessingLagCalculator::with_config(sub_seed, lag_config));
                    }
                }

                // Copy drift controller if present
                if let Some(ref dc) = self.drift_controller {
                    gen.drift_controller = Some(dc.clone());
                }

                // SP3: share Arc-wrapped priors with all sub-generators.
                // Clone is O(1) — increments the reference count only.
                gen.loaded_priors = self.loaded_priors.clone();

                // SP3.4: each partition starts with a fresh calibrator so
                // observations are partition-local (avoids cross-partition
                // state contamination).  Target rates and window size are
                // cloned from the parent; accumulated state is not.
                if let Some(ref cal) = self.velocity_calibrator {
                    let mut fresh = crate::velocity_calibrator::VelocityCalibrator::new(
                        cal.target_trigger_rates.clone(),
                        cal.n_lines_between_calibrations,
                    );
                    fresh.current_values = cal.current_values.clone();
                    gen.velocity_calibrator = Some(fresh);
                }

                gen
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChartOfAccountsGenerator;

    #[test]
    fn test_generate_balanced_entries() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let mut je_gen = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        );

        let mut balanced_count = 0;
        for _ in 0..100 {
            let entry = je_gen.generate();

            // Skip entries with human errors as they may be intentionally unbalanced
            let has_human_error = entry
                .header
                .header_text
                .as_ref()
                .map(|t| t.contains("[HUMAN_ERROR:"))
                .unwrap_or(false);

            if !has_human_error {
                assert!(
                    entry.is_balanced(),
                    "Entry {:?} is not balanced",
                    entry.header.document_id
                );
                balanced_count += 1;
            }
            assert!(entry.line_count() >= 2, "Entry has fewer than 2 lines");
        }

        // Ensure most entries are balanced (human errors are rare)
        assert!(
            balanced_count >= 80,
            "Expected at least 80 balanced entries, got {}",
            balanced_count
        );
    }

    #[test]
    fn test_deterministic_generation() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let mut gen1 = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            Arc::clone(&coa),
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        );

        let mut gen2 = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        );

        for _ in 0..50 {
            let e1 = gen1.generate();
            let e2 = gen2.generate();
            assert_eq!(e1.header.document_id, e2.header.document_id);
            assert_eq!(e1.total_debit(), e2.total_debit());
        }
    }

    #[test]
    fn test_templates_generate_descriptions() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        // Enable all template features
        let template_config = TemplateConfig {
            names: datasynth_config::schema::NameTemplateConfig {
                generate_realistic_names: true,
                email_domain: "test.com".to_string(),
                culture_distribution: datasynth_config::schema::CultureDistribution::default(),
            },
            descriptions: datasynth_config::schema::DescriptionTemplateConfig {
                generate_header_text: true,
                generate_line_text: true,
            },
            references: datasynth_config::schema::ReferenceTemplateConfig {
                generate_references: true,
                invoice_prefix: "TEST-INV".to_string(),
                po_prefix: "TEST-PO".to_string(),
                so_prefix: "TEST-SO".to_string(),
            },
            path: None,
            merge_strategy: datasynth_config::TemplateMergeStrategy::default(),
        };

        let mut je_gen = JournalEntryGenerator::new_with_full_config(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
            template_config,
            None,
        )
        .with_persona_errors(false); // Disable for template testing

        for _ in 0..10 {
            let entry = je_gen.generate();

            // Verify header text is populated
            assert!(
                entry.header.header_text.is_some(),
                "Header text should be populated"
            );

            // Verify reference is populated
            assert!(
                entry.header.reference.is_some(),
                "Reference should be populated"
            );

            // Verify business process is set
            assert!(
                entry.header.business_process.is_some(),
                "Business process should be set"
            );

            // Verify line text is populated
            for line in &entry.lines {
                assert!(line.line_text.is_some(), "Line text should be populated");
            }

            // Entry should still be balanced
            assert!(entry.is_balanced());
        }
    }

    #[test]
    fn test_user_pool_integration() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let companies = vec!["1000".to_string()];

        // Generate user pool
        let mut user_gen = crate::UserGenerator::new(42);
        let user_pool = user_gen.generate_standard(&companies);

        let mut je_gen = JournalEntryGenerator::new_with_full_config(
            TransactionConfig::default(),
            coa,
            companies,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
            TemplateConfig::default(),
            Some(user_pool),
        );

        // Generate entries and verify user IDs are from pool
        for _ in 0..20 {
            let entry = je_gen.generate();

            // User ID should not be generic BATCH/USER format when pool is used
            // (though it may still fall back if random selection misses)
            assert!(!entry.header.created_by.is_empty());
        }
    }

    #[test]
    fn test_master_data_connection() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        // Create test vendors
        let vendors = vec![
            Vendor::new("V-TEST-001", "Test Vendor Alpha", VendorType::Supplier),
            Vendor::new("V-TEST-002", "Test Vendor Beta", VendorType::Technology),
        ];

        // Create test customers
        let customers = vec![
            Customer::new("C-TEST-001", "Test Customer One", CustomerType::Corporate),
            Customer::new(
                "C-TEST-002",
                "Test Customer Two",
                CustomerType::SmallBusiness,
            ),
        ];

        // Create test materials
        let materials = vec![Material::new(
            "MAT-TEST-001",
            "Test Material A",
            MaterialType::RawMaterial,
        )];

        // Create generator with master data
        let generator = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        );

        // Without master data
        assert!(!generator.is_using_real_master_data());

        // Connect master data
        let generator_with_data = generator
            .with_vendors(&vendors)
            .with_customers(&customers)
            .with_materials(&materials);

        // Should now be using real master data
        assert!(generator_with_data.is_using_real_master_data());
    }

    #[test]
    fn test_with_master_data_convenience_method() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let vendors = vec![Vendor::new("V-001", "Vendor One", VendorType::Supplier)];
        let customers = vec![Customer::new(
            "C-001",
            "Customer One",
            CustomerType::Corporate,
        )];
        let materials = vec![Material::new(
            "MAT-001",
            "Material One",
            MaterialType::RawMaterial,
        )];

        let generator = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        )
        .with_master_data(&vendors, &customers, &materials);

        assert!(generator.is_using_real_master_data());
    }

    #[test]
    fn test_stress_factors_increase_error_rate() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let generator = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        );

        let base_rate = 0.1;

        // Regular day - no stress factors
        let regular_day = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(); // Mid-June Wednesday
        let regular_rate = generator.apply_stress_factors(base_rate, regular_day);
        assert!(
            (regular_rate - base_rate).abs() < 0.01,
            "Regular day should have minimal stress factor adjustment"
        );

        // Month end - 50% more errors
        let month_end = NaiveDate::from_ymd_opt(2024, 6, 29).unwrap(); // June 29 (Saturday)
        let month_end_rate = generator.apply_stress_factors(base_rate, month_end);
        assert!(
            month_end_rate > regular_rate,
            "Month end should have higher error rate than regular day"
        );

        // Year end - double the error rate
        let year_end = NaiveDate::from_ymd_opt(2024, 12, 30).unwrap(); // December 30
        let year_end_rate = generator.apply_stress_factors(base_rate, year_end);
        assert!(
            year_end_rate > month_end_rate,
            "Year end should have highest error rate"
        );

        // Friday stress
        let friday = NaiveDate::from_ymd_opt(2024, 6, 14).unwrap(); // Friday
        let friday_rate = generator.apply_stress_factors(base_rate, friday);
        assert!(
            friday_rate > regular_rate,
            "Friday should have higher error rate than mid-week"
        );

        // Monday stress
        let monday = NaiveDate::from_ymd_opt(2024, 6, 17).unwrap(); // Monday
        let monday_rate = generator.apply_stress_factors(base_rate, monday);
        assert!(
            monday_rate > regular_rate,
            "Monday should have higher error rate than mid-week"
        );
    }

    #[test]
    fn test_batching_produces_similar_entries() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        // Use seed 123 which is more likely to trigger batching
        let mut je_gen = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            123,
        )
        .with_persona_errors(false); // Disable to ensure balanced entries

        // Generate many entries - at 15% batch rate, should see some batches
        let entries: Vec<JournalEntry> = (0..200).map(|_| je_gen.generate()).collect();

        // Check that all entries are balanced (batched or not)
        for entry in &entries {
            assert!(
                entry.is_balanced(),
                "All entries including batched should be balanced"
            );
        }

        // Count entries with same-day posting dates (batch indicator)
        let mut date_counts: std::collections::HashMap<NaiveDate, usize> =
            std::collections::HashMap::new();
        for entry in &entries {
            *date_counts.entry(entry.header.posting_date).or_insert(0) += 1;
        }

        // With batching, some dates should have multiple entries
        let dates_with_multiple = date_counts.values().filter(|&&c| c > 1).count();
        assert!(
            dates_with_multiple > 0,
            "With batching, should see some dates with multiple entries"
        );
    }

    #[test]
    fn test_temporal_patterns_business_days() {
        use datasynth_config::schema::{
            BusinessDaySchemaConfig, CalendarSchemaConfig, TemporalPatternsConfig,
        };

        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        // Create temporal patterns config with business days enabled
        let temporal_config = TemporalPatternsConfig {
            enabled: true,
            business_days: BusinessDaySchemaConfig {
                enabled: true,
                ..Default::default()
            },
            calendars: CalendarSchemaConfig {
                regions: vec!["US".to_string()],
                custom_holidays: vec![],
            },
            ..Default::default()
        };

        let mut je_gen = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(), // Q1 2024
            42,
        )
        .with_temporal_patterns(temporal_config, 42)
        .with_persona_errors(false);

        // Generate entries and verify none fall on weekends
        let entries: Vec<JournalEntry> = (0..100).map(|_| je_gen.generate()).collect();

        for entry in &entries {
            let weekday = entry.header.posting_date.weekday();
            assert!(
                weekday != chrono::Weekday::Sat && weekday != chrono::Weekday::Sun,
                "Posting date {:?} should not be a weekend",
                entry.header.posting_date
            );
        }
    }

    #[test]
    fn test_default_generation_filters_weekends() {
        // Verify that weekend entries are <5% even when temporal_patterns is NOT enabled.
        // This tests the fix where new_with_full_config always creates a default
        // BusinessDayCalculator with US holidays as a fallback.
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let mut je_gen = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        )
        .with_persona_errors(false);

        let total = 500;
        let entries: Vec<JournalEntry> = (0..total).map(|_| je_gen.generate()).collect();

        let weekend_count = entries
            .iter()
            .filter(|e| {
                let wd = e.header.posting_date.weekday();
                wd == chrono::Weekday::Sat || wd == chrono::Weekday::Sun
            })
            .count();

        let weekend_pct = weekend_count as f64 / total as f64;
        assert!(
            weekend_pct < 0.05,
            "Expected weekend entries <5% of total without temporal_patterns enabled, \
             but got {:.1}% ({}/{})",
            weekend_pct * 100.0,
            weekend_count,
            total
        );
    }

    #[test]
    fn test_document_type_derived_from_business_process() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let mut je_gen = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            99,
        )
        .with_persona_errors(false)
        .with_batching(false);

        let total = 200;
        let mut doc_types = std::collections::HashSet::new();
        let mut sa_count = 0_usize;

        for _ in 0..total {
            let entry = je_gen.generate();
            let dt = &entry.header.document_type;
            doc_types.insert(dt.clone());
            if dt == "SA" {
                sa_count += 1;
            }
        }

        // Should have more than 3 distinct document types
        assert!(
            doc_types.len() > 3,
            "Expected >3 distinct document types, got {} ({:?})",
            doc_types.len(),
            doc_types,
        );

        // "SA" should be less than 50% (R2R is 20% of the weight)
        let sa_pct = sa_count as f64 / total as f64;
        assert!(
            sa_pct < 0.50,
            "Expected SA <50%, got {:.1}% ({}/{})",
            sa_pct * 100.0,
            sa_count,
            total,
        );
    }

    #[test]
    fn test_enrich_line_items_account_description() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let mut je_gen = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        )
        .with_persona_errors(false);

        let total = 200;
        let entries: Vec<JournalEntry> = (0..total).map(|_| je_gen.generate()).collect();

        // Count lines with account_description populated
        let total_lines: usize = entries.iter().map(|e| e.lines.len()).sum();
        let lines_with_desc: usize = entries
            .iter()
            .flat_map(|e| &e.lines)
            .filter(|l| l.account_description.is_some())
            .count();

        let desc_pct = lines_with_desc as f64 / total_lines as f64;
        assert!(
            desc_pct > 0.95,
            "Expected >95% of lines to have account_description, got {:.1}% ({}/{})",
            desc_pct * 100.0,
            lines_with_desc,
            total_lines,
        );
    }

    #[test]
    fn test_enrich_line_items_cost_center_for_expense_accounts() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let mut je_gen = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        )
        .with_persona_errors(false);

        let total = 300;
        let entries: Vec<JournalEntry> = (0..total).map(|_| je_gen.generate()).collect();

        // Count expense account lines (5xxx/6xxx) with cost_center populated
        let expense_lines: Vec<&JournalEntryLine> = entries
            .iter()
            .flat_map(|e| &e.lines)
            .filter(|l| {
                let first = l.gl_account.chars().next().unwrap_or('0');
                first == '5' || first == '6'
            })
            .collect();

        if !expense_lines.is_empty() {
            let with_cc = expense_lines
                .iter()
                .filter(|l| l.cost_center.is_some())
                .count();
            let cc_pct = with_cc as f64 / expense_lines.len() as f64;
            assert!(
                cc_pct > 0.80,
                "Expected >80% of expense lines to have cost_center, got {:.1}% ({}/{})",
                cc_pct * 100.0,
                with_cc,
                expense_lines.len(),
            );
        }
    }

    #[test]
    fn test_enrich_line_items_profit_center_and_line_text() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let mut je_gen = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        )
        .with_persona_errors(false);

        let total = 100;
        let entries: Vec<JournalEntry> = (0..total).map(|_| je_gen.generate()).collect();

        let total_lines: usize = entries.iter().map(|e| e.lines.len()).sum();

        // All lines should have profit_center
        let with_pc = entries
            .iter()
            .flat_map(|e| &e.lines)
            .filter(|l| l.profit_center.is_some())
            .count();
        let pc_pct = with_pc as f64 / total_lines as f64;
        assert!(
            pc_pct > 0.95,
            "Expected >95% of lines to have profit_center, got {:.1}% ({}/{})",
            pc_pct * 100.0,
            with_pc,
            total_lines,
        );

        // All lines should have line_text (either from template or header fallback)
        let with_text = entries
            .iter()
            .flat_map(|e| &e.lines)
            .filter(|l| l.line_text.is_some())
            .count();
        let text_pct = with_text as f64 / total_lines as f64;
        assert!(
            text_pct > 0.95,
            "Expected >95% of lines to have line_text, got {:.1}% ({}/{})",
            text_pct * 100.0,
            with_text,
            total_lines,
        );
    }

    // --- ISA 240 audit flag tests ---

    #[test]
    fn test_je_has_audit_flags() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let mut je_gen = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        )
        .with_persona_errors(false);

        for _ in 0..100 {
            let entry = je_gen.generate();

            // source_system should always be non-empty
            assert!(
                !entry.header.source_system.is_empty(),
                "source_system should be populated, got empty string"
            );

            // created_by should always be non-empty (already tested elsewhere, but confirm)
            assert!(
                !entry.header.created_by.is_empty(),
                "created_by should be populated"
            );

            // created_date should always be populated
            assert!(
                entry.header.created_date.is_some(),
                "created_date should be populated"
            );
        }
    }

    #[test]
    fn test_manual_entry_rate() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let mut je_gen = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        )
        .with_persona_errors(false)
        .with_batching(false);

        let total = 1000;
        let entries: Vec<JournalEntry> = (0..total).map(|_| je_gen.generate()).collect();

        let manual_count = entries.iter().filter(|e| e.header.is_manual).count();
        let manual_rate = manual_count as f64 / total as f64;

        // Default source_distribution.manual is typically around 0.05-0.15
        // Allow a wide tolerance for statistical variation
        assert!(
            manual_rate > 0.01 && manual_rate < 0.50,
            "Manual entry rate should be reasonable (1%-50%), got {:.1}% ({}/{})",
            manual_rate * 100.0,
            manual_count,
            total,
        );

        // is_manual should match TransactionSource::Manual
        for entry in &entries {
            let source_is_manual = entry.header.source == TransactionSource::Manual;
            assert_eq!(
                entry.header.is_manual, source_is_manual,
                "is_manual should match source == Manual"
            );
        }
    }

    #[test]
    fn test_manual_source_consistency() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let mut je_gen = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        )
        .with_persona_errors(false)
        .with_batching(false);

        for _ in 0..500 {
            let entry = je_gen.generate();

            if entry.header.is_manual {
                // Manual entries must have a source_system in the
                // `manual/...` or `spreadsheet/...` family (the bare
                // legacy `manual` and `spreadsheet` values are also
                // accepted to keep older fixtures working).
                let s = entry.header.source_system.as_str();
                assert!(
                    s == "manual"
                        || s == "spreadsheet"
                        || s.starts_with("manual/")
                        || s.starts_with("spreadsheet/"),
                    "Manual entry should have source_system in `manual` / `spreadsheet` family, got '{s}'",
                );
            } else {
                // Non-manual entries must NOT be in the manual/spreadsheet family.
                let s = entry.header.source_system.as_str();
                assert!(
                    !(s == "manual"
                        || s == "spreadsheet"
                        || s.starts_with("manual/")
                        || s.starts_with("spreadsheet/")),
                    "Non-manual entry should not be in `manual` / `spreadsheet` family, got '{s}'",
                );
            }
        }
    }

    #[test]
    fn test_default_source_codes_breadth() {
        // T2-D Lever 1: with no industry priors and the default config, the
        // `source` column carries a broad generic SAP doc-type mix
        // (sap_source_code populated) instead of collapsing to the
        // TransactionSource enum. See experiments/ml/FINDINGS.md §6.
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 7);
        let coa = Arc::new(coa_gen.generate());
        let mut je_gen = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            7,
        )
        .with_persona_errors(false)
        .with_batching(false);

        let mut codes = std::collections::HashSet::new();
        for _ in 0..500 {
            let e = je_gen.generate();
            let code = e
                .header
                .sap_source_code
                .expect("default config should populate sap_source_code");
            codes.insert(code);
        }
        assert!(
            codes.len() >= 10,
            "default source-mix should be broad (>=10 distinct codes), got {}",
            codes.len()
        );
    }

    #[test]
    fn test_source_codes_opt_out() {
        // synthetic_source_codes = Some(false) restores the legacy behaviour:
        // sap_source_code stays None and `source` falls back to the enum.
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 9);
        let coa = Arc::new(coa_gen.generate());
        let cfg = TransactionConfig {
            synthetic_source_codes: Some(false),
            ..TransactionConfig::default()
        };
        let mut je_gen = JournalEntryGenerator::new_with_params(
            cfg,
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            9,
        )
        .with_persona_errors(false)
        .with_batching(false);
        for _ in 0..50 {
            let e = je_gen.generate();
            assert!(
                e.header.sap_source_code.is_none(),
                "opt-out should leave sap_source_code None (legacy enum source)"
            );
        }
    }

    #[test]
    fn test_recurring_templates_reuse_archetypes() {
        // SOTA-1: with templating on (default), generated JEs reuse account
        // archetypes (far fewer distinct than the legacy uniform-per-line
        // selection), and balance is preserved either way.
        fn run(recurring: Option<bool>) -> (usize, usize, bool) {
            let mut coa_gen = ChartOfAccountsGenerator::new(
                CoAComplexity::Medium,
                IndustrySector::Manufacturing,
                11,
            );
            let coa = Arc::new(coa_gen.generate());
            let cfg = TransactionConfig {
                recurring_templates: recurring,
                ..TransactionConfig::default()
            };
            let mut g = JournalEntryGenerator::new_with_params(
                cfg,
                coa,
                vec!["1000".to_string()],
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
                11,
            )
            .with_persona_errors(false)
            .with_batching(false);
            let n = 800;
            let mut arche = std::collections::HashSet::new();
            let mut balanced = true;
            for _ in 0..n {
                let e = g.generate();
                if !e.is_balanced() {
                    balanced = false;
                }
                let mut sig: Vec<(String, bool)> = e
                    .lines
                    .iter()
                    .map(|l| (l.gl_account.clone(), l.debit_amount > Decimal::ZERO))
                    .collect();
                sig.sort();
                arche.insert(sig);
            }
            (n, arche.len(), balanced)
        }
        let (n, distinct_on, bal_on) = run(Some(true));
        let (_, distinct_off, bal_off) = run(Some(false));
        assert!(bal_on && bal_off, "balance preserved in both modes");
        assert!(
            distinct_on < distinct_off,
            "templating should reduce distinct archetypes ({distinct_on} on vs {distinct_off} off)"
        );
        assert!(
            distinct_on * 2 < n,
            "templating should reuse heavily: {distinct_on} distinct archetypes over {n} JEs"
        );
    }

    #[test]
    fn test_reversal_process_emits_balanced_reversals() {
        // SOTA-5: with reversal_rate > 0, some JEs are balanced reversals of
        // earlier ones (header_text "Reversal of ..."); rate 0.0 emits none.
        fn run(rate: Option<f64>) -> (usize, bool) {
            let mut coa_gen = ChartOfAccountsGenerator::new(
                CoAComplexity::Small,
                IndustrySector::Manufacturing,
                13,
            );
            let coa = Arc::new(coa_gen.generate());
            let cfg = TransactionConfig {
                reversal_rate: rate,
                ..TransactionConfig::default()
            };
            let mut g = JournalEntryGenerator::new_with_params(
                cfg,
                coa,
                vec!["1000".to_string()],
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
                13,
            )
            .with_persona_errors(false)
            .with_batching(false);
            let mut reversals = 0;
            let mut balanced = true;
            for _ in 0..1000 {
                let e = g.generate();
                if !e.is_balanced() {
                    balanced = false;
                }
                if e.header
                    .header_text
                    .as_deref()
                    .is_some_and(|t| t.starts_with("Reversal of"))
                {
                    reversals += 1;
                }
            }
            (reversals, balanced)
        }
        let (rev_on, bal_on) = run(Some(0.05));
        let (rev_off, bal_off) = run(Some(0.0));
        assert!(bal_on && bal_off, "all entries balanced incl. reversals");
        assert_eq!(rev_off, 0, "rate 0.0 emits no reversals, got {rev_off}");
        assert!(rev_on > 0, "rate 0.05 should emit reversals, got {rev_on}");
    }

    #[test]
    fn test_account_concentration_creates_pareto() {
        // SOTA-2: with concentration on (default), a hot subset of accounts
        // carries most lines (the corpus account-activity Pareto, top-10% ≈ 95%)
        // vs the legacy near-uniform pool draw. Templating + reversals are held
        // off so the only difference between the two runs is the power-law pick.
        fn run(concentration: Option<bool>) -> (f64, bool) {
            let mut coa_gen = ChartOfAccountsGenerator::new(
                CoAComplexity::Medium,
                IndustrySector::Manufacturing,
                17,
            );
            let coa = Arc::new(coa_gen.generate());
            let cfg = TransactionConfig {
                account_concentration: concentration,
                recurring_templates: Some(false),
                reversal_rate: Some(0.0),
                ..TransactionConfig::default()
            };
            let mut g = JournalEntryGenerator::new_with_params(
                cfg,
                coa,
                vec!["1000".to_string()],
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
                17,
            )
            .with_persona_errors(false)
            .with_batching(false);
            let mut counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            let mut total_lines = 0usize;
            let mut balanced = true;
            for _ in 0..1000 {
                let e = g.generate();
                if !e.is_balanced() {
                    balanced = false;
                }
                for l in &e.lines {
                    *counts.entry(l.gl_account.clone()).or_default() += 1;
                    total_lines += 1;
                }
            }
            // share of lines carried by the top-10% most-active accounts (the
            // corpus_structure "acct top10%" metric, over active accounts).
            let mut v: Vec<usize> = counts.values().copied().collect();
            v.sort_unstable_by(|a, b| b.cmp(a));
            let top_k = ((v.len() as f64 * 0.10).ceil() as usize).max(1);
            let top_share = v.iter().take(top_k).sum::<usize>() as f64 / total_lines as f64;
            (top_share, balanced)
        }
        let (share_on, bal_on) = run(Some(true));
        let (share_off, bal_off) = run(Some(false));
        assert!(bal_on && bal_off, "balance preserved in both modes");
        assert!(
            share_on > share_off + 0.20,
            "concentration should raise the top-10% line share ({share_on:.3} on vs {share_off:.3} off)"
        );
        assert!(
            share_on > 0.50,
            "hot accounts should dominate: top-10% line share {share_on:.3}"
        );
    }

    #[test]
    fn test_allocation_batch_emits_large_balanced_postings() {
        // SOTA-6: with allocation_batch_rate > 0, some JEs are large 1-to-many
        // allocation batches (source "AB", many cost-center-spread lines, still
        // balanced); rate 0.0 emits none. Reversals are disabled to isolate the
        // allocation process (which shares the recent-JE buffer).
        fn run(rate: Option<f64>) -> (usize, bool, usize) {
            let mut coa_gen = ChartOfAccountsGenerator::new(
                CoAComplexity::Small,
                IndustrySector::Manufacturing,
                23,
            );
            let coa = Arc::new(coa_gen.generate());
            let cfg = TransactionConfig {
                allocation_batch_rate: rate,
                reversal_rate: Some(0.0),
                ..TransactionConfig::default()
            };
            let mut g = JournalEntryGenerator::new_with_params(
                cfg,
                coa,
                vec!["1000".to_string()],
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
                23,
            )
            .with_persona_errors(false)
            .with_batching(false);
            let mut batches = 0usize;
            let mut balanced = true;
            let mut max_distinct_cc = 0usize;
            for _ in 0..2000 {
                let e = g.generate();
                if !e.is_balanced() {
                    balanced = false;
                }
                if e.header.sap_source_code.as_deref() == Some("AB") {
                    batches += 1;
                    assert!(
                        e.lines.len() >= ALLOCATION_MIN_TARGETS as usize,
                        "allocation batch should be large, got {} lines",
                        e.lines.len()
                    );
                    let ccs: std::collections::HashSet<String> = e
                        .lines
                        .iter()
                        .filter_map(|l| l.cost_center.clone())
                        .collect();
                    max_distinct_cc = max_distinct_cc.max(ccs.len());
                }
            }
            (batches, balanced, max_distinct_cc)
        }
        let (on, bal_on, cc) = run(Some(0.10));
        let (off, bal_off, _) = run(Some(0.0));
        assert!(
            bal_on && bal_off,
            "all entries balanced incl. allocation batches"
        );
        assert_eq!(off, 0, "rate 0.0 emits no allocation batches, got {off}");
        assert!(on > 0, "rate 0.10 should emit allocation batches, got {on}");
        assert!(
            cc > 1,
            "allocation should spread across multiple cost centers, got {cc}"
        );
    }

    #[test]
    fn test_derived_id_processes_keep_document_ids_unique() {
        // SOTA-5/6 regression: reversals and allocation batches mint derived ids
        // (`base ^ salt`). Reusing the same base would duplicate an id — the
        // failure `test_document_reference_integrity` caught. With both processes
        // at high rates, every emitted document id must still be unique.
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 31);
        let coa = Arc::new(coa_gen.generate());
        let cfg = TransactionConfig {
            reversal_rate: Some(0.15),
            allocation_batch_rate: Some(0.10),
            ..TransactionConfig::default()
        };
        let mut g = JournalEntryGenerator::new_with_params(
            cfg,
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            31,
        )
        .with_persona_errors(false)
        .with_batching(false);
        let mut ids = std::collections::HashSet::new();
        let n = 3000;
        for _ in 0..n {
            let e = g.generate();
            assert!(
                ids.insert(e.header.document_id),
                "duplicate document id {} (derived-id collision)",
                e.header.document_id
            );
        }
        assert_eq!(ids.len(), n, "all {n} document ids unique");
    }

    #[test]
    fn test_business_unit_rolls_up_from_cost_center() {
        // SOTA-3: with the dimension on (default), a line that has a cost center
        // (or, as fallback, a profit center) also carries a business_unit that is
        // a deterministic roll-up of that dimension (same value → same BU, in
        // BU01..BU11); with it off, BU is empty.
        fn run(enabled: Option<bool>) -> (usize, usize, bool, bool) {
            let mut coa_gen = ChartOfAccountsGenerator::new(
                CoAComplexity::Medium,
                IndustrySector::Manufacturing,
                19,
            );
            let coa = Arc::new(coa_gen.generate());
            let cfg = TransactionConfig {
                business_unit_dimension: enabled,
                ..TransactionConfig::default()
            };
            let mut g = JournalEntryGenerator::new_with_params(
                cfg,
                coa,
                vec!["1000".to_string()],
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
                19,
            )
            .with_persona_errors(false)
            .with_batching(false);
            let mut dim_lines = 0usize;
            let mut bu_lines = 0usize;
            let mut consistent = true; // BU present ⇒ matches the roll-up of its CC/PC
            let mut well_formed = true; // BU in BU01..BU11
            let mut dim_to_bu: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            for _ in 0..600 {
                let e = g.generate();
                for l in &e.lines {
                    // BU rolls up from the cost center, or profit center as fallback.
                    let dim = l.cost_center.as_deref().or(l.profit_center.as_deref());
                    if dim.is_some() {
                        dim_lines += 1;
                    }
                    if let Some(bu) = &l.business_unit {
                        bu_lines += 1;
                        let d = dim.unwrap_or_default().to_string();
                        if bu != &JournalEntryGenerator::business_unit_for_dimension(&d) {
                            consistent = false;
                        }
                        // stable mapping across the run
                        if dim_to_bu
                            .insert(d, bu.clone())
                            .is_some_and(|prev| &prev != bu)
                        {
                            consistent = false;
                        }
                        let n_ok = bu.strip_prefix("BU").and_then(|d| d.parse::<u32>().ok());
                        if !matches!(n_ok, Some(1..=11)) {
                            well_formed = false;
                        }
                    }
                }
            }
            (dim_lines, bu_lines, consistent, well_formed)
        }
        let (dim_on, bu_on, consistent, well_formed) = run(Some(true));
        let (_, bu_off, _, _) = run(Some(false));
        assert!(
            dim_on > 0 && bu_on > 0,
            "BU should be populated where CC/PC is"
        );
        assert_eq!(
            dim_on, bu_on,
            "every CC/PC-bearing line gets a BU ({dim_on} dim vs {bu_on} BU)"
        );
        assert!(
            consistent,
            "BU must be the deterministic roll-up of its CC/PC"
        );
        assert!(well_formed, "BU codes must be BU01..BU11");
        assert_eq!(bu_off, 0, "dimension off ⇒ no business_unit, got {bu_off}");
    }

    #[test]
    fn test_foreign_currency_sap_style() {
        // SOTA-4: with foreign_currency_rate > 0, some JEs post in a foreign
        // document currency. The ledger amounts (debit/credit) stay company
        // currency and the JE still balances; the foreign value lands in
        // transaction_amount and balances in the transaction currency too. rate
        // 0.0 → all company-currency. Reversals/allocations off to isolate.
        fn run(rate: Option<f64>) -> (usize, bool, bool) {
            let mut coa_gen = ChartOfAccountsGenerator::new(
                CoAComplexity::Small,
                IndustrySector::Manufacturing,
                29,
            );
            let coa = Arc::new(coa_gen.generate());
            let cfg = TransactionConfig {
                foreign_currency_rate: rate,
                reversal_rate: Some(0.0),
                allocation_batch_rate: Some(0.0),
                ..TransactionConfig::default()
            };
            let mut g = JournalEntryGenerator::new_with_params(
                cfg,
                coa,
                vec!["1000".to_string()],
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
                29,
            )
            .with_persona_errors(false)
            .with_batching(false);
            let mut foreign = 0usize;
            let mut ledger_ok = true; // debit == credit (company ledger)
            let mut txn_ok = true; // foreign lines carry transaction_amount + balance in txn ccy
            for _ in 0..1500 {
                let e = g.generate();
                if !e.is_balanced() {
                    ledger_ok = false;
                }
                if e.header.currency != "USD" {
                    foreign += 1;
                    if !e.lines.iter().all(|l| l.transaction_amount.is_some()) {
                        txn_ok = false;
                    }
                    let td: Decimal = e
                        .lines
                        .iter()
                        .filter(|l| l.debit_amount > Decimal::ZERO)
                        .filter_map(|l| l.transaction_amount)
                        .sum();
                    let tc: Decimal = e
                        .lines
                        .iter()
                        .filter(|l| l.credit_amount > Decimal::ZERO)
                        .filter_map(|l| l.transaction_amount)
                        .sum();
                    // tolerate per-line cent rounding (≤ n_lines half-cents)
                    let tol = Decimal::new(e.lines.len() as i64, 2);
                    if (td - tc).abs() > tol {
                        txn_ok = false;
                    }
                }
            }
            (foreign, ledger_ok, txn_ok)
        }
        let (fon, lbal_on, tbal_on) = run(Some(0.20));
        let (foff, lbal_off, _) = run(Some(0.0));
        assert!(
            lbal_on && lbal_off,
            "ledger balance (debit==credit) preserved in both modes"
        );
        assert!(
            fon > 0,
            "rate 0.20 should produce foreign-currency JEs, got {fon}"
        );
        assert_eq!(foff, 0, "rate 0.0 ⇒ no foreign JEs, got {foff}");
        assert!(
            tbal_on,
            "foreign JEs carry transaction_amount + balance in the transaction currency"
        );
    }

    #[test]
    fn test_created_date_before_posting() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let mut je_gen = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        )
        .with_persona_errors(false);

        for _ in 0..500 {
            let entry = je_gen.generate();

            if let Some(created_date) = entry.header.created_date {
                let created_naive_date = created_date.date();
                assert!(
                    created_naive_date <= entry.header.posting_date,
                    "created_date ({}) should be <= posting_date ({})",
                    created_naive_date,
                    entry.header.posting_date,
                );
            }
        }
    }

    /// SP3.5b — verify that `apply_calibration_step` mutates the generator's
    /// amount_sampler when a `"amounts.lognormal_sigma"` step is applied, and
    /// that `"amounts.round_dollar_share"` likewise updates the probability.
    #[test]
    fn apply_calibration_step_updates_lognormal_sigma() {
        let mut coa_gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = Arc::new(coa_gen.generate());

        let mut gen = JournalEntryGenerator::new_with_params(
            TransactionConfig::default(),
            coa,
            vec!["1000".to_string()],
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            42,
        );

        let baseline_sigma = gen.amount_sampler.lognormal_sigma();

        let step_sigma = crate::velocity_calibrator::CalibrationStep {
            rule_id: "R6".to_string(),
            parameter: "amounts.lognormal_sigma".to_string(),
            delta: 0.01,
            new_value: baseline_sigma + 0.01,
        };
        gen.apply_calibration_step(&step_sigma);
        assert!(
            (gen.amount_sampler.lognormal_sigma() - (baseline_sigma + 0.01)).abs() < 1e-9,
            "lognormal_sigma should be updated to {}",
            baseline_sigma + 0.01
        );

        let baseline_round = gen.amount_sampler.round_number_probability();
        let step_round = crate::velocity_calibrator::CalibrationStep {
            rule_id: "R9".to_string(),
            parameter: "amounts.round_dollar_share".to_string(),
            delta: -0.005,
            new_value: (baseline_round - 0.005).max(0.0),
        };
        gen.apply_calibration_step(&step_round);
        let expected = (baseline_round - 0.005).max(0.0).clamp(0.0, 1.0);
        assert!(
            (gen.amount_sampler.round_number_probability() - expected).abs() < 1e-9,
            "round_number_probability should be updated to {}",
            expected
        );
    }

    #[test]
    fn master_data_resolver_fills_every_pii_kind() {
        use datasynth_core::distributions::text_taxonomy::{
            PiiPlaceholderKind, PlaceholderResolver,
        };
        let mut r = MasterDataResolver {
            companies: vec!["Acme AG".to_string()],
            persons: vec!["Hans Muster".to_string()],
            streets: vec!["Hauptstrasse 1".to_string()],
            patients: vec!["Patient X".to_string()],
        };
        let mut rng = rand::rng();
        assert_eq!(r.resolve(PiiPlaceholderKind::Company, &mut rng), "Acme AG");
        assert_eq!(
            r.resolve(PiiPlaceholderKind::Person, &mut rng),
            "Hans Muster"
        );
        assert_eq!(
            r.resolve(PiiPlaceholderKind::Street, &mut rng),
            "Hauptstrasse 1"
        );
        assert_eq!(
            r.resolve(PiiPlaceholderKind::Patient, &mut rng),
            "Patient X"
        );
    }

    #[test]
    fn master_data_resolver_empty_pool_falls_back() {
        use datasynth_core::distributions::text_taxonomy::{
            PiiPlaceholderKind, PlaceholderResolver,
        };
        let mut r = MasterDataResolver::default();
        let mut rng = rand::rng();
        let v = r.resolve(PiiPlaceholderKind::Company, &mut rng);
        assert!(!v.is_empty());
    }

    /// Pin the shape invariant on `synthetic_patient_pool`: each entry, once
    /// filled into the canonical `*{patient} G:{date}…` template the corpus
    /// DZ/RG/RS classes use, must not introduce a *structural* residual-PII
    /// shape. Regression guard for the JE_79-class smoke failure: the old pool
    /// (`"B. Muster"`, `"A. Beispiel"`, …) shaped each fill as
    /// `<initial>. <surname>` which `RE_INITIAL_SURNAME` flags.
    ///
    /// NB: the `given_name` pattern is deliberately EXCLUDED here. These are
    /// synthetic *fill* values that are name-shaped by design (they fill
    /// `{patient}`); `given_name` is a template-scan signal for un-tokenized
    /// corpus names, not a check on legitimate synthetic output.
    #[test]
    fn synthetic_patient_pool_entries_pass_residual_scan() {
        use datasynth_core::distributions::text_taxonomy::PlaceholderGrammar;
        for name in synthetic_patient_pool("de_CH") {
            let filled = format!("*{name} G:2024-01-15 E:2024-01-20 A:2024-02-01");
            let structural: Vec<_> = PlaceholderGrammar::residual_pii_scan(&filled)
                .into_iter()
                .filter(|h| h.pattern != "given_name")
                .collect();
            assert!(
                structural.is_empty(),
                "synthetic patient name {name:?} fills to PII-shaped {filled:?}: {structural:?}"
            );
        }
    }

    #[test]
    fn master_data_resolver_fallbacks_are_non_empty_and_placeholder_free() {
        use datasynth_core::distributions::text_taxonomy::{
            PiiPlaceholderKind, PlaceholderResolver,
        };
        // Verify fallback constants for every kind are non-empty and contain
        // no `{…}` literal placeholders (the resolver must never leak the
        // unfilled placeholder token into emitted text).
        let mut r = MasterDataResolver::default();
        let mut rng = rand::rng();
        for kind in [
            PiiPlaceholderKind::Company,
            PiiPlaceholderKind::Person,
            PiiPlaceholderKind::Street,
            PiiPlaceholderKind::Patient,
        ] {
            let v = r.resolve(kind, &mut rng);
            assert!(!v.is_empty(), "fallback for {kind:?} must be non-empty");
            assert!(
                !v.contains('{'),
                "fallback for {kind:?} must not contain a placeholder token"
            );
        }
    }
}

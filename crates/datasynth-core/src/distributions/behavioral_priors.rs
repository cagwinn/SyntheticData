//! Behavioral priors mined from corpus GL data (SP2).
//!
//! These types are defined in `datasynth-core` (not `datasynth-fingerprint`) so
//! that `datasynth-generators` can consume them without creating a package cycle.
//! `datasynth-fingerprint` re-exports them from here.
//!
//! Spec: `docs/superpowers/specs/2026-05-12-sp2-real-world-prior-extraction-design.md`

use std::collections::BTreeMap;

use rand::RngExt;
use serde::{Deserialize, Serialize};

use super::text_taxonomy::TextTaxonomyPrior;

// ---------------------------------------------------------------------------
// EmpiricalCdf
// ---------------------------------------------------------------------------

/// Empirical CDF representation used by IET and lag priors.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EmpiricalCdf {
    /// Column / source name (informational).
    pub column: String,
    /// Sorted quantile values (after privacy processing).
    pub values: Vec<f64>,
    /// Cumulative probabilities matching `values` (monotone in [0, 1]).
    pub probabilities: Vec<f64>,
}

impl EmpiricalCdf {
    /// Build an empirical CDF from pre-sorted values.
    pub fn from_sorted_values(column: impl Into<String>, values: Vec<f64>) -> Self {
        let n = values.len();
        let probabilities: Vec<f64> = (1..=n).map(|i| i as f64 / n as f64).collect();
        Self {
            column: column.into(),
            values,
            probabilities,
        }
    }

    /// Evaluate CDF at a value (linear interpolation between knots).
    pub fn cdf(&self, x: f64) -> f64 {
        match self.values.binary_search_by(|v| v.total_cmp(&x)) {
            Ok(i) => self.probabilities[i],
            Err(i) => {
                if i == 0 {
                    0.0
                } else if i >= self.values.len() {
                    1.0
                } else {
                    let (x0, x1) = (self.values[i - 1], self.values[i]);
                    let (p0, p1) = (self.probabilities[i - 1], self.probabilities[i]);
                    p0 + (p1 - p0) * (x - x0) / (x1 - x0)
                }
            }
        }
    }

    /// Evaluate inverse CDF (quantile function) at a probability.
    pub fn quantile(&self, p: f64) -> f64 {
        if p <= 0.0 {
            return *self.values.first().unwrap_or(&0.0);
        }
        if p >= 1.0 {
            return *self.values.last().unwrap_or(&0.0);
        }
        match self.probabilities.binary_search_by(|v| v.total_cmp(&p)) {
            Ok(i) => self.values[i],
            Err(i) => {
                if i == 0 {
                    self.values[0]
                } else if i >= self.probabilities.len() {
                    *self.values.last().unwrap_or(&0.0)
                } else {
                    let (p0, p1) = (self.probabilities[i - 1], self.probabilities[i]);
                    let (x0, x1) = (self.values[i - 1], self.values[i]);
                    x0 + (x1 - x0) * (p - p0) / (p1 - p0)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PerSourceAmountPrior (SP4.3)
// ---------------------------------------------------------------------------

/// SP4.3 — Per-(source, gl_prefix) amount distribution parameters.
///
/// Used by the generator to draw amounts conditional on the source code
/// (and optionally the first-4-digit GL account prefix) drawn at line-construction
/// time.  Log-normal mu/sigma per (source, gl_prefix) bucket, with fallback to
/// source-marginal when the specific pair isn't represented.
///
/// Storage as nested map `source → gl_prefix → params` for forward compatibility.
/// The `by_source` marginals serve as the primary fallback when a specific
/// `(source, gl_prefix)` pair is absent or unrecognised.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PerSourceAmountPrior {
    /// Keyed by `(source, gl_prefix)`.
    /// Outer key: source code (e.g. "KR").
    /// Inner key: first-4-chars of GL account number (e.g. "0041").
    pub by_source_and_class: BTreeMap<String, BTreeMap<String, LognormalAmount>>,
    /// Source-only marginal fallback when an `(source, gl_prefix)` pair isn't present.
    pub by_source: BTreeMap<String, LognormalAmount>,
}

/// Log-normal amount parameters derived from corpus GL data.
///
/// All values refer to `ln(|amount|)`.  Callers take the absolute value of the
/// raw corpus amount before fitting so that both debit and credit lines are
/// captured in the same distribution; sign/direction is assigned by the JE
/// balancer after sampling.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LognormalAmount {
    /// Mean of `ln(|amount|)`.
    pub mu: f64,
    /// Standard deviation of `ln(|amount|)`.
    pub sigma: f64,
    /// Number of observations underpinning these parameters.
    /// Parameters are only emitted when `n >= min_observations` (default 10).
    pub n: usize,
    /// Median absolute value of the raw amounts (for sanity / audit).
    pub median_abs: f64,
}

impl LognormalAmount {
    /// Draw a positive amount magnitude from `LogNormal(mu, sigma)`.
    ///
    /// Returns an `f64` > 0.  The caller decides sign (debit vs credit) and
    /// rounds to [`rust_decimal::Decimal`].
    ///
    /// Falls back to `LogNormal(0.0, 1.0)` when `sigma` is non-positive or
    /// the parameters are otherwise invalid — avoids panicking in production.
    pub fn sample<R: rand::Rng>(&self, rng: &mut R) -> f64 {
        use rand_distr::{Distribution, LogNormal};
        // Clamp sigma to a tiny positive value to avoid `LogNormal::new` error.
        let sigma = self.sigma.max(1e-6);
        let dist = LogNormal::new(self.mu, sigma)
            .unwrap_or_else(|_| LogNormal::new(0.0, 1.0).expect("fallback lognormal"));
        dist.sample(rng)
    }
}

// ---------------------------------------------------------------------------
// BehavioralPriors and sub-types
// ---------------------------------------------------------------------------

/// Root container for the SP2 behavioral priors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BehavioralPriors {
    pub schema_version: u32,
    pub generator_version: String,
    pub industry: String,
    pub n_client_inputs: usize,
    pub n_rows_aggregated: usize,
    pub source_mix: SourceMixPrior,
    pub per_source_iet: PerSourceIetPrior,
    pub lines_per_je: LinesPerJePrior,
    pub active_lifetime: ActiveLifetimePrior,
    pub fanout: FanoutPrior,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posting_lag: Option<PostingLagPrior>,
    /// SP3.2 — per-Source multi-segment active patterns. Optional, additive.
    /// When `Some`, supersedes `active_lifetime` for window placement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_segments: Option<ActiveSegmentsPrior>,
    /// SP3.3 — entity-cluster prior driving cross-entity motif preservation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_clusters: Option<EntityClustersPrior>,
    /// SP3.7 — per-source conditional attribute priors. Optional, additive.
    /// When `Some`, the generator samples GL account / cost center / profit
    /// center conditioned on the just-drawn source code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_source_attribute: Option<PerSourceAttributePrior>,
    /// SP3.12 — TP-entity cluster prior. Clusters TradingPartner values by
    /// shared GL/CC/PC attribute sets. The generator uses this to bias TP
    /// selection toward cluster-mates of recently-used TPs (same source),
    /// building triangle structure in the TP-GL co-occurrence graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tp_entity_clusters: Option<EntityClustersPrior>,
    /// SP4.2 — CoA semantic content extracted from corpus COA_XXX.parquet
    /// files. When `Some`, the CoA generator enriches account descriptions,
    /// account_class, and account_sub_class with corpus values instead of
    /// generic constants.  Old bundles load fine (deserialises as `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coa_semantic: Option<CoaSemanticPrior>,
    /// SP4.7 — Per-source reference-string format templates extracted from the
    /// corpus.  When `Some`, the JE generator samples a reference string
    /// from the template distribution rather than using the fixed
    /// `format!("...")` fallback.  Old bundles load fine (deserialises as `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_formats: Option<ReferenceFormatPrior>,
    /// SP6 — corpus text taxonomy. Replaces the old SP4.4 text_templates field. When `Some`,
    /// the generator samples line text keyed on `(source, account-class)`,
    /// header text on `source`, and CoA descriptions per account — all PII-safe
    /// templates filled at generation time. `None` for pre-SP6 bundles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_taxonomy: Option<TextTaxonomyPrior>,
    /// SP4.5 — Per-user behavioral patterns (source mix, hourly density,
    /// weekday density, volume share) mined from corpus GL data.
    /// When `Some` and non-empty, the JE generator biases `created_by` and
    /// `created_at` toward the characteristic patterns of each user.
    ///
    /// Note: the corpus GL files examined for this project carried no
    /// user column; the prior is included as an additive field so future data
    /// deliveries can populate it without any schema changes.  Old bundles
    /// load fine (deserialises as `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_personas: Option<UserPersonaPrior>,
    /// SP4.3 — Per-(source, gl_prefix) amount distribution parameters.
    ///
    /// When `Some`, the JE generator samples the JE total-amount from the
    /// log-normal conditional matching the current source code (and optionally
    /// GL-account prefix), rather than the global `AmountSampler` marginal.
    /// Fraud entries bypass this path to preserve fraud-pattern semantics.
    /// Old bundles load fine (deserialises as `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_amount_conditionals: Option<PerSourceAmountPrior>,
    /// SP4.6 — Per-(source, line_role) GL account conditional.
    ///
    /// line_role is "DR" (debit-dominant lines) or "CR" (credit-dominant lines).
    /// When `Some`, the generator draws GL accounts respecting the line role so
    /// that expense/asset classes appear on DR sides and liability/revenue classes
    /// appear on CR sides, matching corpus SAP doc-type shapes.
    ///
    /// Sources with no strong role bias (e.g. SA) emit whatever the corpus shows.
    /// Old bundles load fine (deserialises as `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_role_gl_conditionals: Option<PerSourceRolePrior>,
    /// SP4.1 — Trial-balance anchor prior.  Per-account opening/closing balances
    /// extracted from real `TB_XXX.parquet` files.  When `Some`, the generator's
    /// balance tracker uses these targets to nudge synthetic balances toward a
    /// corpus-shaped distribution via periodic drift-correction entries.
    ///
    /// Old bundles load fine (deserialises as `None`); the balance tracker
    /// continues with its existing free-drift behaviour when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tb_anchor: Option<TbAnchorPrior>,
}

impl BehavioralPriors {
    pub const SCHEMA_VERSION: u32 = 1;
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SourceMixPrior {
    pub probabilities: BTreeMap<String, f64>,
    pub other_fraction: f64,
    pub min_threshold: f64,
}

impl SourceMixPrior {
    /// Draw a source code from the weighted distribution.
    ///
    /// Returns a randomly selected key from `probabilities`, weighted by the
    /// associated probability mass.  Falls back to the first key (or `"SA"`) when
    /// the map is empty or all weights sum to zero.
    pub fn sample<R: rand::Rng>(&self, rng: &mut R) -> String {
        if self.probabilities.is_empty() {
            return "SA".to_string();
        }
        let r: f64 = rng.random_range(0.0..1.0);
        let total: f64 = self.probabilities.values().sum();
        if total <= 0.0 {
            return self
                .probabilities
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "SA".to_string());
        }
        let mut cum = 0.0;
        for (code, &weight) in &self.probabilities {
            cum += weight / total;
            if r <= cum {
                return code.clone();
            }
        }
        // Floating-point rounding: return the last key.
        self.probabilities
            .keys()
            .next_back()
            .cloned()
            .unwrap_or_else(|| "SA".to_string())
    }

    /// A privacy-safe **generic** SAP document-type source mix for the default
    /// (no industry-priors) generation path. Standard SAP FI/MM/SD document
    /// types with hand-chosen, plausible weights — **not** corpus-derived
    /// probabilities — so the emitted `source` column has realistic breadth
    /// (~25 codes, entropy ~2.7) instead of collapsing to the coarse
    /// `TransactionSource` enum (entropy ~0.75). See experiments/ml/FINDINGS.md
    /// §6 for the source-mix gap that motivates this. Weights are relative;
    /// `sample` normalises by their sum.
    pub fn sap_default() -> Self {
        let head = [
            ("RV", 0.16),
            ("KR", 0.12),
            ("DR", 0.10),
            ("SA", 0.09),
            ("DZ", 0.08),
            ("KZ", 0.07),
            ("WE", 0.06),
            ("RE", 0.05),
            ("DG", 0.04),
            ("KG", 0.035),
            ("WA", 0.03),
            // "AB" (accounting/allocation doc) is intentionally omitted: it is
            // reserved for the SOTA-6 allocation-batch process so synthetic "AB"
            // JEs carry the corpus's large lines-per-JE (~52), not a small mix.
            ("WL", 0.025),
            ("ZP", 0.02),
            ("SK", 0.018),
            ("AF", 0.015),
            ("AA", 0.012),
            ("ML", 0.010),
            ("PR", 0.008),
            ("RN", 0.007),
            ("WI", 0.006),
            ("AN", 0.005),
            ("UE", 0.004),
            ("ZV", 0.003),
            ("EU", 0.002),
        ];
        let mut probabilities: BTreeMap<String, f64> =
            head.into_iter().map(|(k, v)| (k.to_string(), v)).collect();

        // Long tail (Lever 2): a power-law of synthetic SAP custom doc-type
        // codes (Z-prefixed, the SAP custom-type convention). Synthetic only,
        // never corpus-derived. The rare tail codes draw few events, which also
        // lifts the per-source inter-event-time variance (FINDINGS sec.6: IET
        // variance is coupled to source breadth). Weight is proportional to
        // 1/rank^1.1, scaled to a fraction of the head's summed mass.
        const TAIL_N: usize = 500;
        const TAIL_MASS: f64 = 0.30;
        let zipf: f64 = (1..=TAIL_N).map(|r| 1.0 / (r as f64).powf(1.1)).sum();
        for r in 1..=TAIL_N {
            let w = TAIL_MASS * (1.0 / (r as f64).powf(1.1)) / zipf;
            probabilities.insert(format!("Z{r:03}"), w);
        }

        Self {
            probabilities,
            other_fraction: 0.0,
            min_threshold: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PerSourceIetPrior {
    pub by_source: BTreeMap<String, IetSummary>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct IetSummary {
    pub n: usize,
    pub empirical_cdf_days: EmpiricalCdf,
    pub lognormal_fit: Option<LognormalParams>,
    pub lag1_autocorr: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LognormalParams {
    pub mu: f64,
    pub sigma: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LinesPerJePrior {
    pub overall: LineCountHistogram,
    pub by_source: BTreeMap<String, LineCountHistogram>,
    pub min_jes_per_source: usize,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ActiveLifetimePrior {
    pub by_source: BTreeMap<String, LineCountHistogram>,
    pub overall: LineCountHistogram,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FanoutPrior {
    pub by_attribute: BTreeMap<String, LineCountHistogram>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PostingLagPrior {
    pub by_source: BTreeMap<String, LagSummary>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LagSummary {
    pub empirical_cdf_days: EmpiricalCdf,
    pub mean: f64,
    pub stddev: f64,
    pub n: usize,
}

// ---------------------------------------------------------------------------
// ActiveSegmentsPrior (SP3.2)
// ---------------------------------------------------------------------------

/// SP3.2 — Per-Source distribution of active-segment count + length + gap.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ActiveSegmentsPrior {
    pub by_source: BTreeMap<String, SourceSegmentSummary>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SourceSegmentSummary {
    pub segment_count_histogram: LineCountHistogram,
    pub segment_length_histogram: LineCountHistogram,
    pub gap_length_histogram: LineCountHistogram,
}

/// Bucket grid for "how many active segments per Source per period".
pub const SEGMENT_COUNT_BUCKETS: &[u32] = &[1, 2, 3, 4, 6, 8, 12, 16, 24];

/// Bucket grid for "gap between active segments, in days".
pub const SEGMENT_GAP_BUCKETS: &[u32] = &[1, 2, 3, 7, 14, 30, 60, 90];

// ---------------------------------------------------------------------------
// EntityClustersPrior (SP3.3)
// ---------------------------------------------------------------------------

/// SP3.3 — Clusters of Sources that share attribute pools heavily.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EntityClustersPrior {
    pub clusters: Vec<EntityCluster>,
    /// Fraction of Sources that landed in *some* cluster (vs. isolates).
    pub clustering_rate: f64,
}

// ---------------------------------------------------------------------------
// PerSourceAttributePrior + CategoricalDistribution (SP3.7)
// ---------------------------------------------------------------------------

/// SP3.7 — Per-source conditional distribution over downstream attributes
/// (GL account, cost center, profit center, ...).  When generation knows
/// the source code, the conditional CDF tells it which attribute values
/// are characteristic of that source.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PerSourceAttributePrior {
    /// Outer key: source code (e.g. "KR", "RV", "DZ").
    /// Inner key: attribute name ("gl_account", "cost_center", "profit_center").
    /// Value: categorical distribution over the observed attribute values
    ///        conditioned on `(source, attribute)`.
    pub by_source: BTreeMap<String, BTreeMap<String, CategoricalDistribution>>,
    /// Configured minimum number of observations per (source, attribute) pair
    /// before a conditional is retained. Smaller groups are dropped to avoid
    /// over-fitting low-volume sources.
    pub min_observations: usize,
}

impl PerSourceAttributePrior {
    /// Look up the conditional distribution for `(source, attribute)`.
    /// Returns `None` if the source or attribute isn't represented in the
    /// prior (e.g. the source had too few observations during extraction).
    pub fn conditional(&self, source: &str, attribute: &str) -> Option<&CategoricalDistribution> {
        self.by_source.get(source)?.get(attribute)
    }
}

// ---------------------------------------------------------------------------
// PerSourceRolePrior (SP4.6)
// ---------------------------------------------------------------------------

/// SP4.6 — Per-(source, line_role) GL account conditional.
///
/// Captures the observation that each SAP document type has a canonical
/// line-role structure:
/// - KR (vendor invoice): DR → expense (5/6xxx),  CR → AP (2xxx)
/// - RV (customer invoice): DR → AR (1xxx),       CR → revenue (4xxx)
/// - DZ (customer payment): DR → Bank (1xxx),     CR → AR (1xxx)
/// - KZ (vendor payment):  DR → AP (2xxx),        CR → Bank (1xxx)
/// - WE (goods receipt):   DR → Inventory (1xxx), CR → GR/IR (2xxx)
/// - SA (manual journal): weak role bias — distribution reflects corpus mix.
///
/// `role` is `"DR"` for debit-dominant lines, `"CR"` for credit-dominant lines.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PerSourceRolePrior {
    /// `(source → role → distribution over GL account values)`.
    /// Inner role key is `"DR"` or `"CR"`.
    pub by_source_and_role: BTreeMap<String, BTreeMap<String, CategoricalDistribution>>,
}

impl PerSourceRolePrior {
    /// Look up the conditional distribution for `(source, role)`.
    ///
    /// Returns `None` when the source or role isn't represented (e.g. too few
    /// observations during extraction, or an old bundle without SP4.6 data).
    /// Callers fall back to `per_source_attribute` marginal or to a default GL.
    pub fn conditional(&self, source: &str, role: &str) -> Option<&CategoricalDistribution> {
        self.by_source_and_role.get(source)?.get(role)
    }
}

// ---------------------------------------------------------------------------
// TbAnchorPrior (SP4.1)
// ---------------------------------------------------------------------------

/// SP4.1 — Trial-balance anchor prior.
///
/// Extracted from real `TB_XXX.parquet` files.  Each entry represents the
/// industry-median opening and closing balance for a GL account number.
/// The generator's balance tracker uses these values in target-aware mode
/// to emit drift-correction entries that keep the synthetic balance sheet
/// shaped like a corpus balance sheet.
///
/// **Schema note**: real TB files carry `Functional Beginning Balance` and
/// `Functional Ending Balance` columns but no explicit period-debit/credit
/// columns — those are derived as the difference between balances.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TbAnchorPrior {
    /// Per-account TB target.  Keyed by GL account number (normalised to
    /// the same format as the rest of the bundle, e.g. leading-zero-padded
    /// 10-digit strings for the current health corpus).
    pub per_account: BTreeMap<String, TbTarget>,
    /// Industry-median total assets (sum of asset-class closing balances).
    /// Used for aggregate drift-tolerance checks.
    pub total_assets: f64,
    /// Industry-median total liabilities (sum of liability-class closing balances).
    pub total_liabilities: f64,
    /// Industry-median total equity (sum of equity-class closing balances).
    pub total_equity: f64,
    /// Number of clients that contributed to this anchor.
    pub n_clients: usize,
}

impl TbAnchorPrior {
    /// Returns `true` when the prior carries at least one account with a
    /// non-zero closing balance — i.e. it was built from real TB data.
    pub fn has_data(&self) -> bool {
        self.per_account
            .values()
            .any(|t| t.closing_balance.abs() > 1e-9 || t.opening_balance.abs() > 1e-9)
    }
}

/// Target balance for a single GL account, representing the industry-median
/// values extracted from real `TB_XXX.parquet` files.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TbTarget {
    /// Industry-median opening balance for this account.
    pub opening_balance: f64,
    /// Industry-median closing balance for this account.
    pub closing_balance: f64,
    /// Derived period net activity (closing − opening), positive = net debit.
    /// Pre-computed at extraction time for convenience.
    pub period_net_activity: f64,
    /// Standard deviation of opening balance across contributing clients
    /// (in their respective functional currencies, normalised by client-total-assets).
    /// Smaller stdev = tighter target.  Zero when only one client contributed.
    pub opening_stdev: f64,
    /// Standard deviation of closing balance across contributing clients.
    pub closing_stdev: f64,
    /// Number of clients in which this account appeared.
    pub n_clients: usize,
}

/// A categorical distribution stored as a sparse `value → probability` map.
/// Used by SP3.7 for per-source attribute conditionals.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CategoricalDistribution {
    /// Value (e.g. an account number) → probability mass.  Probabilities
    /// sum to 1.0 (within float tolerance).
    pub probabilities: BTreeMap<String, f64>,
    /// Total observations underpinning this distribution.
    pub n: usize,
}

impl CategoricalDistribution {
    /// Construct from raw counts.  Normalises to probabilities.
    pub fn from_counts(counts: BTreeMap<String, usize>) -> Self {
        let n: usize = counts.values().sum();
        if n == 0 {
            return Self::default();
        }
        let probabilities = counts
            .into_iter()
            .map(|(k, v)| (k, v as f64 / n as f64))
            .collect();
        Self { probabilities, n }
    }

    /// Draw a value from the distribution.  Returns `None` when the
    /// distribution is empty (caller falls back to the marginal or to a
    /// default attribute value).
    pub fn sample<R: rand::Rng>(&self, rng: &mut R) -> Option<String> {
        if self.probabilities.is_empty() {
            return None;
        }
        let total: f64 = self.probabilities.values().sum();
        if total <= 0.0 {
            return None;
        }
        let r: f64 = rng.random_range(0.0..1.0);
        let mut cum = 0.0;
        for (value, &p) in &self.probabilities {
            cum += p / total;
            if r <= cum {
                return Some(value.clone());
            }
        }
        self.probabilities.keys().next_back().cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EntityCluster {
    pub members: Vec<String>,
    pub avg_jaccard: f64,
}

// ---------------------------------------------------------------------------
// CoaSemanticPrior (SP4.2)
// ---------------------------------------------------------------------------

/// SP4.2 — Per-account semantic content extracted from real CoA files.
/// Keyed by canonical GL account number (matching values in the per-source
/// attribute conditional). When `Some` on the parent `BehavioralPriors`, the
/// CoA generator emits real account names + descriptions + ISO 21378 hierarchy
/// from this prior instead of generic constants.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CoaSemanticPrior {
    pub accounts: std::collections::BTreeMap<String, AccountSemantic>,
}

/// Semantic metadata for a single GL account, sourced from a real CoA file.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AccountSemantic {
    /// Human-readable account description (e.g. "Kasse", "Bankkonto CHF 1").
    pub description: String,
    /// Account type as reported in the source file (e.g. "Assets", "Liabilities").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_type: Option<String>,
    /// ISO 21378 Level-2 class code or label (e.g. "C _ Cash", "01_Cash and cash equivalents").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_class: Option<String>,
    /// Human-readable name for `account_class`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_class_name: Option<String>,
    /// ISO 21378 Level-3 sub-class code or label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_sub_class: Option<String>,
    /// Human-readable name for `account_sub_class`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_sub_class_name: Option<String>,
    /// Parent account number in hierarchy (if available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_account: Option<String>,
}

// ---------------------------------------------------------------------------
// ReferenceFormatPrior (SP4.7)
// ---------------------------------------------------------------------------

/// SP4.7 — Per-source reference-string format templates extracted from real
/// corpus GL data.  Templates use placeholders for digit and alphabetic runs;
/// fixed punctuation / hyphens are preserved verbatim.
///
/// Example templates:
/// - `"{4 digits}-{4 digits}-{10 digits}"` (JE_3 client: all sources)
/// - `"RE-{4 digits}-{6 digits}"` (hypothetical KR/vendor-invoice client)
///
/// Only templates observed ≥ `min_occurrences` times (per client) and the top-10
/// by frequency per source are retained — preventing PII leakage.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ReferenceFormatPrior {
    pub by_source: BTreeMap<String, Vec<ReferenceTemplate>>,
}

/// A single reference-format template observed in corpus data.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ReferenceTemplate {
    /// Template string with `{N digits}` / `{N alpha}` placeholders.
    /// Fixed characters (hyphens, slashes, dots) are preserved verbatim.
    /// Example: `"{4 digits}-{4 digits}-{10 digits}"`
    pub template: String,
    /// Fraction of references for this source that match this template
    /// (after frequency filtering and renormalisation).
    pub probability: f64,
    /// A concrete example value from the corpus (for debugging / audit).
    pub example: String,
}

// ---------------------------------------------------------------------------
// UserPersonaPrior (SP4.5)
// ---------------------------------------------------------------------------

/// SP4.5 — Per-user behavioral patterns extracted from a corpus.
///
/// Each user has a characteristic source mix (AP clerks post mostly KR/KZ,
/// AR clerks RV/DZ, GL/closing staff SA), an hourly density, and a weekday
/// distribution.
///
/// **Corpus note**: The corpus GL files in this project carry no user column
/// (all 45 JE parquet files were examined; none had a Created-By field).
/// Extraction is therefore stubbed — `extract_user_personas` in
/// `user_extractor.rs` returns an empty `UserPersonaPrior`.  When a future data
/// delivery includes per-user columns, the extractor can be filled in without
/// any schema changes here.  Generators gate on `UserPersonaPrior::has_data()`
/// and fall back to the internal user pool when the prior is empty.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct UserPersonaPrior {
    /// User-ID → behavior.  Empty when no user-column data was available.
    pub users: BTreeMap<String, UserBehavior>,
    /// Distribution of distinct active-user count per generation run.
    /// Populated only when extraction had real data.
    pub user_count_distribution: LineCountHistogram,
}

impl UserPersonaPrior {
    /// Returns `true` when the prior carries at least one user with a non-empty
    /// source mix and positive volume share (i.e. built from real user-column data).
    pub fn has_data(&self) -> bool {
        self.users
            .values()
            .any(|u| !u.source_mix.is_empty() && u.volume_share > 0.0)
    }

    /// Sample a user ID likely to post the given `source` code.
    ///
    /// Weights each user by `source_mix[source] × volume_share`.  Returns
    /// `None` when the prior is empty or no user has a non-zero weight for
    /// `source` (caller falls back to the generic user pool).
    pub fn sample_user_for_source<R: rand::Rng>(
        &self,
        source: &str,
        rng: &mut R,
    ) -> Option<String> {
        use rand::RngExt;
        if self.users.is_empty() {
            return None;
        }
        let weights: Vec<(&String, f64)> = self
            .users
            .iter()
            .filter_map(|(uid, beh)| {
                let mix = beh.source_mix.get(source).copied().unwrap_or(0.0);
                let w = mix * beh.volume_share;
                if w > 0.0 {
                    Some((uid, w))
                } else {
                    None
                }
            })
            .collect();
        if weights.is_empty() {
            return None;
        }
        let total: f64 = weights.iter().map(|(_, w)| *w).sum();
        if total <= 0.0 {
            return None;
        }
        let r: f64 = rng.random_range(0.0..total);
        let mut cum = 0.0;
        for (uid, w) in &weights {
            cum += w;
            if r <= cum {
                return Some((*uid).clone());
            }
        }
        weights.last().map(|(uid, _)| (*uid).clone())
    }

    /// Given a `user_id`, sample an `(hour, weekday)` pair from the user's
    /// density arrays.  `hour` ∈ 0..24, `weekday` ∈ 0..7 (Monday = 0).
    ///
    /// Returns `None` when the user is unknown or all densities are zero.
    pub fn sample_timestamp_for_user<R: rand::Rng>(
        &self,
        user_id: &str,
        rng: &mut R,
    ) -> Option<(u32, u32)> {
        use rand::RngExt;
        let beh = self.users.get(user_id)?;

        let hour_total: f64 = beh.hourly_density.iter().sum();
        if hour_total <= 0.0 {
            return None;
        }
        let r: f64 = rng.random_range(0.0..hour_total);
        let mut cum = 0.0;
        let mut hour = 0u32;
        for (h, &p) in beh.hourly_density.iter().enumerate() {
            cum += p;
            if r <= cum {
                hour = h as u32;
                break;
            }
        }

        let weekday_total: f64 = beh.weekday_density.iter().sum();
        if weekday_total <= 0.0 {
            return None;
        }
        let r: f64 = rng.random_range(0.0..weekday_total);
        let mut cum = 0.0;
        let mut weekday = 0u32;
        for (d, &p) in beh.weekday_density.iter().enumerate() {
            cum += p;
            if r <= cum {
                weekday = d as u32;
                break;
            }
        }

        Some((hour, weekday))
    }
}

/// Behavioral fingerprint for a single corpus user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserBehavior {
    /// Source code → probability mass.  Sums to 1.0 (within float tolerance).
    /// Example: AP clerk → `{"KR": 0.6, "KZ": 0.3, "RE": 0.1}`.
    pub source_mix: BTreeMap<String, f64>,
    /// Hour-of-day (0 = midnight … 23 = 23:xx) → probability mass.
    /// Sums to 1.0.  All zeros when `created_at` was unavailable.
    pub hourly_density: [f64; 24],
    /// Day-of-week (0 = Monday … 6 = Sunday) → probability mass.  Sums to 1.0.
    pub weekday_density: [f64; 7],
    /// Fraction of total rows in the industry pool attributed to this user.
    pub volume_share: f64,
}

impl Default for UserBehavior {
    fn default() -> Self {
        Self {
            source_mix: BTreeMap::new(),
            hourly_density: [0.0; 24],
            weekday_density: [0.0; 7],
            volume_share: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// LineCountHistogram
// ---------------------------------------------------------------------------

/// Bucket histogram for counts such as lines-per-JE, fan-out, and active lifetime.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LineCountHistogram {
    pub buckets: Vec<u32>,
    pub probabilities: Vec<f64>,
    pub n: usize,
}

impl LineCountHistogram {
    /// Build a histogram on the given inclusive-lower-bound bucket grid.
    ///
    /// The bucket grid must be sorted ascending. Values >= `buckets.last()`
    /// fall into the last bucket. Values < `buckets[0]` are dropped (count
    /// returned for diagnostic use).
    pub fn build(values: &[u32], buckets: &[u32]) -> (Self, usize) {
        assert!(!buckets.is_empty(), "buckets must not be empty");
        let n_buckets = buckets.len();
        let mut counts = vec![0u64; n_buckets];
        let mut dropped = 0usize;
        for &v in values {
            if v < buckets[0] {
                dropped += 1;
                continue;
            }
            let bucket_idx = bucket_index(buckets, v);
            counts[bucket_idx] += 1;
        }
        let total: u64 = counts.iter().sum();
        let probabilities = if total == 0 {
            vec![0.0; n_buckets]
        } else {
            counts.iter().map(|&c| c as f64 / total as f64).collect()
        };
        (
            Self {
                buckets: buckets.to_vec(),
                probabilities,
                n: values.len(),
            },
            dropped,
        )
    }

    /// Sum bucket counts of `self` and `other` (same bucket grid) and
    /// renormalise. Returns `None` if grids mismatch.
    pub fn pool(&self, other: &Self) -> Option<Self> {
        if self.buckets != other.buckets {
            return None;
        }
        let total_n = self.n + other.n;
        if total_n == 0 {
            return Some(Self {
                buckets: self.buckets.clone(),
                probabilities: vec![0.0; self.buckets.len()],
                n: 0,
            });
        }
        let probabilities: Vec<f64> = self
            .probabilities
            .iter()
            .zip(other.probabilities.iter())
            .map(|(&pa, &pb)| (pa * self.n as f64 + pb * other.n as f64) / total_n as f64)
            .collect();
        Some(Self {
            buckets: self.buckets.clone(),
            probabilities,
            n: total_n,
        })
    }

    /// Median bucket — the smallest `buckets[i]` whose cumulative probability >= 0.5.
    pub fn median_bucket(&self) -> u32 {
        let mut cum = 0.0;
        for (i, &p) in self.probabilities.iter().enumerate() {
            cum += p;
            if cum >= 0.5 {
                return self.buckets[i];
            }
        }
        *self.buckets.last().unwrap_or(&0)
    }

    /// Sample a count from the histogram. Picks a bucket weighted by
    /// probability mass, then samples uniformly within the bucket (between
    /// `buckets[i]` inclusive and `buckets[i+1]` exclusive — for the last
    /// bucket, returns `buckets[i]`).
    pub fn sample_bucket<R: rand::Rng>(&self, rng: &mut R) -> u32 {
        if self.buckets.is_empty() {
            return 0;
        }
        let r: f64 = rng.random_range(0.0..1.0);
        let mut cum = 0.0;
        let mut chosen_idx = self.buckets.len() - 1;
        for (i, &p) in self.probabilities.iter().enumerate() {
            cum += p;
            if r <= cum {
                chosen_idx = i;
                break;
            }
        }
        let lo = self.buckets[chosen_idx];
        let hi = self.buckets.get(chosen_idx + 1).copied().unwrap_or(lo);
        if hi <= lo {
            lo
        } else {
            rng.random_range(lo..hi)
        }
    }
}

fn bucket_index(buckets: &[u32], v: u32) -> usize {
    match buckets.binary_search(&v) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

/// Canonical bucket grid for line counts (lines-per-JE, fan-out).
pub const LINE_COUNT_BUCKETS: &[u32] = &[1, 2, 3, 4, 5, 6, 8, 10, 16, 32, 64, 128, 256, 1024];

/// Canonical bucket grid for active-lifetime days.
pub const ACTIVE_LIFETIME_DAY_BUCKETS: &[u32] = &[0, 1, 7, 30, 90, 180, 365, 730, 1825];

/// Canonical bucket grid for fan-out values.
pub const FANOUT_BUCKETS: &[u32] = &[1, 2, 3, 5, 8, 16, 32, 64, 128, 256, 1024];

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn sap_default_has_broad_long_tail() {
        // Lever 2: the default source-mix is the standard head plus a synthetic
        // power-law long tail, giving corpus-like breadth + entropy.
        let m = SourceMixPrior::sap_default();
        let p = &m.probabilities;
        assert!(
            p.len() >= 300,
            "Lever-2 default should carry a long tail, got {} codes",
            p.len()
        );
        let total: f64 = p.values().sum();
        let ent: f64 = -p
            .values()
            .map(|&w| {
                let q = w / total;
                if q > 0.0 {
                    q * q.ln()
                } else {
                    0.0
                }
            })
            .sum::<f64>();
        assert!(
            ent > 3.0,
            "Lever-2 default entropy should exceed 3.0, got {ent:.3}"
        );
        assert!(p.contains_key("RV"), "standard head code present");
        assert!(p.contains_key("Z001"), "synthetic tail code present");
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        for _ in 0..50 {
            assert!(p.contains_key(&m.sample(&mut rng)));
        }
    }

    #[test]
    fn line_count_histogram_build_basic() {
        let values = vec![1, 1, 2, 3, 5, 5, 5, 32, 200];
        let (hist, dropped) = LineCountHistogram::build(&values, LINE_COUNT_BUCKETS);
        assert_eq!(dropped, 0);
        assert_eq!(hist.n, 9);
        assert!((hist.probabilities.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn line_count_histogram_drops_below_min() {
        let values = vec![0, 0, 1, 2];
        let (hist, dropped) = LineCountHistogram::build(&values, &[1, 2, 4]);
        assert_eq!(dropped, 2);
        assert_eq!(hist.n, 4);
        assert!((hist.probabilities[0] - 0.5).abs() < 1e-9);
    }

    #[test]
    fn sample_bucket_respects_probabilities() {
        let h = LineCountHistogram {
            buckets: vec![1, 2, 4, 8],
            probabilities: vec![0.0, 0.0, 1.0, 0.0],
            n: 100,
        };
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..50 {
            let s = h.sample_bucket(&mut rng);
            assert!((4..8).contains(&s), "expected sample in [4,8), got {s}");
        }
    }

    #[test]
    fn empirical_cdf_from_sorted_values() {
        let cdf = EmpiricalCdf::from_sorted_values("test", vec![1.0, 2.0, 3.0]);
        assert_eq!(cdf.values.len(), 3);
        assert!((cdf.probabilities[2] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn active_segments_prior_default_round_trips() {
        let p = ActiveSegmentsPrior::default();
        let json = serde_json::to_string(&p).expect("serialize");
        let back: ActiveSegmentsPrior = serde_json::from_str(&json).expect("deserialize");
        assert!(back.by_source.is_empty());
    }

    #[test]
    fn behavioral_priors_active_segments_optional_round_trip() {
        // Construct a minimal BehavioralPriors with active_segments=Some(...)
        // and round-trip.
        let bp = BehavioralPriors {
            schema_version: BehavioralPriors::SCHEMA_VERSION,
            generator_version: "test".to_string(),
            industry: "test".to_string(),
            n_client_inputs: 0,
            n_rows_aggregated: 0,
            source_mix: SourceMixPrior::default(),
            per_source_iet: PerSourceIetPrior::default(),
            lines_per_je: LinesPerJePrior::default(),
            active_lifetime: ActiveLifetimePrior::default(),
            fanout: FanoutPrior::default(),
            posting_lag: None,
            active_segments: Some(ActiveSegmentsPrior::default()),
            entity_clusters: None,
            per_source_attribute: None,
            tp_entity_clusters: None,
            coa_semantic: None,
            reference_formats: None,
            text_taxonomy: None,
            user_personas: None,
            source_amount_conditionals: None,
            source_role_gl_conditionals: None,
            tb_anchor: None,
        };
        let json = serde_json::to_string(&bp).expect("serialize");
        let back: BehavioralPriors = serde_json::from_str(&json).expect("deserialize");
        assert!(back.active_segments.is_some());
    }

    #[test]
    fn behavioral_priors_legacy_round_trips_without_active_segments() {
        // Old JSON (no active_segments field) should deserialise as None.
        let legacy = r#"{
            "schema_version": 1,
            "generator_version": "5.12.0",
            "industry": "health",
            "n_client_inputs": 1,
            "n_rows_aggregated": 100,
            "source_mix": {"probabilities": {}, "other_fraction": 0.0, "min_threshold": 0.005},
            "per_source_iet": {"by_source": {}},
            "lines_per_je": {"overall": {"buckets": [], "probabilities": [], "n": 0}, "by_source": {}, "min_jes_per_source": 500},
            "active_lifetime": {"by_source": {}, "overall": {"buckets": [], "probabilities": [], "n": 0}},
            "fanout": {"by_attribute": {}}
        }"#;
        let bp: BehavioralPriors = serde_json::from_str(legacy).expect("legacy parse");
        assert!(bp.active_segments.is_none());
        assert!(bp.posting_lag.is_none());
    }

    #[test]
    fn entity_clusters_prior_default_round_trips() {
        let p = EntityClustersPrior::default();
        let json = serde_json::to_string(&p).expect("serialize");
        let back: EntityClustersPrior = serde_json::from_str(&json).expect("deserialize");
        assert!(back.clusters.is_empty());
        assert!((back.clustering_rate).abs() < 1e-9);
    }

    #[test]
    fn categorical_distribution_samples_with_correct_weights() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        let mut counts = BTreeMap::new();
        counts.insert("A".to_string(), 700);
        counts.insert("B".to_string(), 200);
        counts.insert("C".to_string(), 100);
        let dist = CategoricalDistribution::from_counts(counts);

        assert_eq!(dist.n, 1000);
        assert!((dist.probabilities["A"] - 0.7).abs() < 1e-9);

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let mut buckets = BTreeMap::new();
        for _ in 0..10_000 {
            let v = dist.sample(&mut rng).expect("non-empty");
            *buckets.entry(v).or_insert(0) += 1;
        }
        // ~70% should be A, allow 2 std deviations of binomial.
        let a_count = buckets.get("A").copied().unwrap_or(0);
        assert!(
            (a_count as i64 - 7000).abs() < 200,
            "got {} A samples",
            a_count
        );
    }

    #[test]
    fn per_source_attribute_prior_conditional_lookup() {
        let mut inner = BTreeMap::new();
        let mut prob_map = BTreeMap::new();
        prob_map.insert("200001".to_string(), 0.9);
        prob_map.insert("200002".to_string(), 0.1);
        inner.insert(
            "gl_account".to_string(),
            CategoricalDistribution {
                probabilities: prob_map,
                n: 100,
            },
        );
        let mut by_source = BTreeMap::new();
        by_source.insert("KR".to_string(), inner);
        let prior = PerSourceAttributePrior {
            by_source,
            min_observations: 10,
        };
        assert!(prior.conditional("KR", "gl_account").is_some());
        assert!(prior.conditional("KR", "cost_center").is_none());
        assert!(prior.conditional("RV", "gl_account").is_none());
    }

    #[test]
    fn behavioral_priors_per_source_attribute_optional_round_trip() {
        let bp = BehavioralPriors {
            schema_version: BehavioralPriors::SCHEMA_VERSION,
            generator_version: "test".to_string(),
            industry: "test".to_string(),
            n_client_inputs: 0,
            n_rows_aggregated: 0,
            source_mix: SourceMixPrior::default(),
            per_source_iet: PerSourceIetPrior::default(),
            lines_per_je: LinesPerJePrior::default(),
            active_lifetime: ActiveLifetimePrior::default(),
            fanout: FanoutPrior::default(),
            posting_lag: None,
            active_segments: None,
            entity_clusters: None,
            per_source_attribute: Some(PerSourceAttributePrior::default()),
            tp_entity_clusters: None,
            coa_semantic: None,
            reference_formats: None,
            text_taxonomy: None,
            user_personas: None,
            source_amount_conditionals: None,
            source_role_gl_conditionals: None,
            tb_anchor: None,
        };
        let json = serde_json::to_string(&bp).expect("serialize");
        let back: BehavioralPriors = serde_json::from_str(&json).expect("deserialize");
        assert!(back.per_source_attribute.is_some());
    }

    #[test]
    fn entity_clusters_prior_with_members_round_trips() {
        let p = EntityClustersPrior {
            clusters: vec![EntityCluster {
                members: vec!["A".into(), "B".into(), "C".into()],
                avg_jaccard: 0.42,
            }],
            clustering_rate: 0.75,
        };
        let json = serde_json::to_string(&p).expect("serialize");
        let back: EntityClustersPrior = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.clusters.len(), 1);
        assert_eq!(back.clusters[0].members.len(), 3);
        assert!((back.clusters[0].avg_jaccard - 0.42).abs() < 1e-9);
        assert!((back.clustering_rate - 0.75).abs() < 1e-9);
    }

    // ---- SP4.3 tests -------------------------------------------------------

    /// `LognormalAmount::sample` returns positive values whose geometric mean
    /// is close to `exp(mu)` (within 20% for 1000 samples).
    #[test]
    fn lognormal_amount_sample_positive_values() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        let params = LognormalAmount {
            mu: 4.5, // exp(4.5) ≈ 90
            sigma: 0.8,
            n: 1000,
            median_abs: 90.0,
        };
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let samples: Vec<f64> = (0..1000).map(|_| params.sample(&mut rng)).collect();

        // All samples must be strictly positive.
        assert!(samples.iter().all(|&v| v > 0.0), "all samples must be > 0");

        // Geometric mean (mean of log-samples) should be close to mu.
        let log_mean: f64 = samples.iter().map(|v| v.ln()).sum::<f64>() / 1000.0;
        assert!(
            (log_mean - 4.5).abs() < 0.15,
            "log-mean {log_mean:.3} should be near mu=4.5"
        );
    }

    /// `LognormalAmount::sample` tolerates degenerate sigma (near-zero → clamped).
    #[test]
    fn lognormal_amount_sample_degenerate_sigma() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        let params = LognormalAmount {
            mu: 3.0,
            sigma: 0.0, // degenerate — should not panic
            n: 5,
            median_abs: 20.0,
        };
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        for _ in 0..10 {
            let v = params.sample(&mut rng);
            assert!(v > 0.0, "must be positive even with sigma=0");
        }
    }

    /// `PerSourceAmountPrior` round-trips through JSON.
    #[test]
    fn per_source_amount_prior_round_trip() {
        let mut by_source = BTreeMap::new();
        by_source.insert(
            "KR".to_string(),
            LognormalAmount {
                mu: 4.5,
                sigma: 2.158,
                n: 278939,
                median_abs: 100.0,
            },
        );
        let mut by_source_and_class = BTreeMap::new();
        let mut inner = BTreeMap::new();
        inner.insert(
            "0041".to_string(),
            LognormalAmount {
                mu: 5.394,
                sigma: 1.602,
                n: 61726,
                median_abs: 209.98,
            },
        );
        by_source_and_class.insert("KR".to_string(), inner);

        let prior = PerSourceAmountPrior {
            by_source_and_class,
            by_source,
        };
        let json = serde_json::to_string(&prior).expect("serialize");
        let back: PerSourceAmountPrior = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.by_source.len(), 1);
        assert_eq!(back.by_source_and_class.len(), 1);
        assert_eq!(back.by_source["KR"].n, 278939);
        assert_eq!(back.by_source_and_class["KR"]["0041"].n, 61726);
    }

    /// `BehavioralPriors` with `source_amount_conditionals: Some(...)` round-trips.
    #[test]
    fn behavioral_priors_source_amount_conditionals_optional_round_trip() {
        let prior = PerSourceAmountPrior {
            by_source_and_class: BTreeMap::new(),
            by_source: BTreeMap::new(),
        };
        let bp = BehavioralPriors {
            schema_version: BehavioralPriors::SCHEMA_VERSION,
            generator_version: "test".to_string(),
            industry: "test".to_string(),
            n_client_inputs: 0,
            n_rows_aggregated: 0,
            source_mix: SourceMixPrior::default(),
            per_source_iet: PerSourceIetPrior::default(),
            lines_per_je: LinesPerJePrior::default(),
            active_lifetime: ActiveLifetimePrior::default(),
            fanout: FanoutPrior::default(),
            posting_lag: None,
            active_segments: None,
            entity_clusters: None,
            per_source_attribute: None,
            tp_entity_clusters: None,
            coa_semantic: None,
            reference_formats: None,
            text_taxonomy: None,
            user_personas: None,
            source_amount_conditionals: Some(prior),
            source_role_gl_conditionals: None,
            tb_anchor: None,
        };
        let json = serde_json::to_string(&bp).expect("serialize");
        let back: BehavioralPriors = serde_json::from_str(&json).expect("deserialize");
        assert!(back.source_amount_conditionals.is_some());
    }

    /// Legacy `BehavioralPriors` JSON without `source_amount_conditionals`
    /// deserialises with `None` for that field (backwards compat).
    #[test]
    fn behavioral_priors_legacy_missing_source_amount_conditionals() {
        let legacy = r#"{
            "schema_version": 1,
            "generator_version": "5.21.0",
            "industry": "health",
            "n_client_inputs": 1,
            "n_rows_aggregated": 100,
            "source_mix": {"probabilities": {}, "other_fraction": 0.0, "min_threshold": 0.005},
            "per_source_iet": {"by_source": {}},
            "lines_per_je": {"overall": {"buckets": [], "probabilities": [], "n": 0}, "by_source": {}, "min_jes_per_source": 500},
            "active_lifetime": {"by_source": {}, "overall": {"buckets": [], "probabilities": [], "n": 0}},
            "fanout": {"by_attribute": {}}
        }"#;
        let bp: BehavioralPriors = serde_json::from_str(legacy).expect("legacy parse");
        assert!(
            bp.source_amount_conditionals.is_none(),
            "missing field should deserialise as None"
        );
    }

    // ---- SP4.6 tests -------------------------------------------------------

    /// `PerSourceRolePrior::conditional` returns the right distribution
    /// and `sample` returns only values from the matching role set.
    #[test]
    fn sp4_6_role_conditional_keeps_dr_in_expense_class() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        // Build a PerSourceRolePrior for KR:
        //   DR: expense accounts (6000, 6100)
        //   CR: AP account (2000)
        let mut dr_counts = BTreeMap::new();
        dr_counts.insert("6000".to_string(), 100usize);
        dr_counts.insert("6100".to_string(), 50usize);
        let mut cr_counts = BTreeMap::new();
        cr_counts.insert("2000".to_string(), 150usize);

        let mut role_map = BTreeMap::new();
        role_map.insert(
            "DR".to_string(),
            CategoricalDistribution::from_counts(dr_counts),
        );
        role_map.insert(
            "CR".to_string(),
            CategoricalDistribution::from_counts(cr_counts),
        );

        let mut by_source_and_role = BTreeMap::new();
        by_source_and_role.insert("KR".to_string(), role_map);

        let prior = PerSourceRolePrior { by_source_and_role };

        // DR draws should be in {6000, 6100}
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..100 {
            let v = prior
                .conditional("KR", "DR")
                .unwrap()
                .sample(&mut rng)
                .unwrap();
            assert!(
                v == "6000" || v == "6100",
                "DR draw must be expense account, got {v}"
            );
        }

        // CR draws should be {2000}
        for _ in 0..50 {
            let v = prior
                .conditional("KR", "CR")
                .unwrap()
                .sample(&mut rng)
                .unwrap();
            assert_eq!(v, "2000", "CR draw must be AP account");
        }
    }

    /// When a `(source, role)` pair is absent, `conditional` returns `None`.
    #[test]
    fn sp4_6_role_conditional_falls_back_when_pair_missing() {
        let prior = PerSourceRolePrior::default();
        assert!(
            prior.conditional("KR", "DR").is_none(),
            "empty prior must return None"
        );
        assert!(
            prior.conditional("KR", "CR").is_none(),
            "empty prior must return None for CR too"
        );
    }

    /// `PerSourceRolePrior` round-trips through JSON without loss.
    #[test]
    fn sp4_6_per_source_role_prior_json_round_trip() {
        let mut dr_counts = BTreeMap::new();
        dr_counts.insert("6000".to_string(), 200usize);
        let mut role_map = BTreeMap::new();
        role_map.insert(
            "DR".to_string(),
            CategoricalDistribution::from_counts(dr_counts),
        );
        let mut by_source_and_role = BTreeMap::new();
        by_source_and_role.insert("KR".to_string(), role_map);
        let prior = PerSourceRolePrior { by_source_and_role };

        let json = serde_json::to_string(&prior).expect("serialize");
        let back: PerSourceRolePrior = serde_json::from_str(&json).expect("deserialize");
        assert!(back.conditional("KR", "DR").is_some());
        assert!(back.conditional("KR", "CR").is_none());
    }

    /// `BehavioralPriors` with `source_role_gl_conditionals: Some(...)` round-trips.
    #[test]
    fn behavioral_priors_source_role_gl_conditionals_optional_round_trip() {
        let prior = PerSourceRolePrior::default();
        let bp = BehavioralPriors {
            schema_version: BehavioralPriors::SCHEMA_VERSION,
            generator_version: "test".to_string(),
            industry: "test".to_string(),
            n_client_inputs: 0,
            n_rows_aggregated: 0,
            source_mix: SourceMixPrior::default(),
            per_source_iet: PerSourceIetPrior::default(),
            lines_per_je: LinesPerJePrior::default(),
            active_lifetime: ActiveLifetimePrior::default(),
            fanout: FanoutPrior::default(),
            posting_lag: None,
            active_segments: None,
            entity_clusters: None,
            per_source_attribute: None,
            tp_entity_clusters: None,
            coa_semantic: None,
            reference_formats: None,
            text_taxonomy: None,
            user_personas: None,
            source_amount_conditionals: None,
            source_role_gl_conditionals: Some(prior),
            tb_anchor: None,
        };
        let json = serde_json::to_string(&bp).expect("serialize");
        let back: BehavioralPriors = serde_json::from_str(&json).expect("deserialize");
        assert!(back.source_role_gl_conditionals.is_some());
    }

    /// Legacy `BehavioralPriors` JSON without `source_role_gl_conditionals`
    /// deserialises with `None` for that field (backwards compat).
    #[test]
    fn behavioral_priors_legacy_missing_source_role_gl_conditionals() {
        let legacy = r#"{
            "schema_version": 1,
            "generator_version": "5.21.0",
            "industry": "health",
            "n_client_inputs": 1,
            "n_rows_aggregated": 100,
            "source_mix": {"probabilities": {}, "other_fraction": 0.0, "min_threshold": 0.005},
            "per_source_iet": {"by_source": {}},
            "lines_per_je": {"overall": {"buckets": [], "probabilities": [], "n": 0}, "by_source": {}, "min_jes_per_source": 500},
            "active_lifetime": {"by_source": {}, "overall": {"buckets": [], "probabilities": [], "n": 0}},
            "fanout": {"by_attribute": {}}
        }"#;
        let bp: BehavioralPriors = serde_json::from_str(legacy).expect("legacy parse");
        assert!(
            bp.source_role_gl_conditionals.is_none(),
            "missing field should deserialise as None"
        );
    }

    // ---- SP4.1 tests -------------------------------------------------------

    /// `TbAnchorPrior` round-trips through JSON without loss.
    #[test]
    fn tb_anchor_prior_json_round_trip() {
        let mut per_account = BTreeMap::new();
        per_account.insert(
            "1000".to_string(),
            TbTarget {
                opening_balance: 100_000.0,
                closing_balance: 120_000.0,
                period_net_activity: 20_000.0,
                opening_stdev: 5_000.0,
                closing_stdev: 6_000.0,
                n_clients: 3,
            },
        );
        per_account.insert(
            "2000".to_string(),
            TbTarget {
                opening_balance: -50_000.0,
                closing_balance: -60_000.0,
                period_net_activity: -10_000.0,
                opening_stdev: 2_000.0,
                closing_stdev: 3_000.0,
                n_clients: 3,
            },
        );
        let anchor = TbAnchorPrior {
            per_account,
            total_assets: 300_000.0,
            total_liabilities: 120_000.0,
            total_equity: 180_000.0,
            n_clients: 3,
        };
        let json = serde_json::to_string(&anchor).expect("serialize");
        let back: TbAnchorPrior = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.per_account.len(), 2);
        assert!((back.per_account["1000"].closing_balance - 120_000.0).abs() < 1e-6);
        assert!((back.total_assets - 300_000.0).abs() < 1e-6);
        assert_eq!(back.n_clients, 3);
    }

    /// `TbAnchorPrior::has_data()` returns `true` for non-zero balances.
    #[test]
    fn tb_anchor_prior_has_data() {
        let mut prior = TbAnchorPrior::default();
        assert!(!prior.has_data(), "empty prior must report no data");

        prior.per_account.insert(
            "1000".to_string(),
            TbTarget {
                closing_balance: 1.0,
                ..Default::default()
            },
        );
        assert!(
            prior.has_data(),
            "non-zero closing balance must report has_data"
        );
    }

    /// `BehavioralPriors` with `tb_anchor: Some(...)` round-trips through JSON.
    #[test]
    fn behavioral_priors_tb_anchor_optional_round_trip() {
        let mut per_account = BTreeMap::new();
        per_account.insert(
            "1000".to_string(),
            TbTarget {
                opening_balance: 50_000.0,
                closing_balance: 55_000.0,
                period_net_activity: 5_000.0,
                opening_stdev: 1_000.0,
                closing_stdev: 1_200.0,
                n_clients: 2,
            },
        );
        let tb_anchor = Some(TbAnchorPrior {
            per_account,
            total_assets: 55_000.0,
            total_liabilities: 0.0,
            total_equity: 55_000.0,
            n_clients: 2,
        });
        let bp = BehavioralPriors {
            schema_version: BehavioralPriors::SCHEMA_VERSION,
            generator_version: "test".to_string(),
            industry: "test".to_string(),
            n_client_inputs: 0,
            n_rows_aggregated: 0,
            source_mix: SourceMixPrior::default(),
            per_source_iet: PerSourceIetPrior::default(),
            lines_per_je: LinesPerJePrior::default(),
            active_lifetime: ActiveLifetimePrior::default(),
            fanout: FanoutPrior::default(),
            posting_lag: None,
            active_segments: None,
            entity_clusters: None,
            per_source_attribute: None,
            tp_entity_clusters: None,
            coa_semantic: None,
            reference_formats: None,
            text_taxonomy: None,
            user_personas: None,
            source_amount_conditionals: None,
            source_role_gl_conditionals: None,
            tb_anchor,
        };
        let json = serde_json::to_string(&bp).expect("serialize");
        let back: BehavioralPriors = serde_json::from_str(&json).expect("deserialize");
        let anchor = back.tb_anchor.expect("tb_anchor must be Some");
        assert_eq!(anchor.per_account.len(), 1);
        assert!((anchor.per_account["1000"].closing_balance - 55_000.0).abs() < 1e-6);
    }

    /// Legacy JSON without `tb_anchor` deserialises as `None` (backwards compat).
    #[test]
    fn behavioral_priors_legacy_missing_tb_anchor() {
        let legacy = r#"{
            "schema_version": 1,
            "generator_version": "5.22.0",
            "industry": "health",
            "n_client_inputs": 1,
            "n_rows_aggregated": 100,
            "source_mix": {"probabilities": {}, "other_fraction": 0.0, "min_threshold": 0.005},
            "per_source_iet": {"by_source": {}},
            "lines_per_je": {"overall": {"buckets": [], "probabilities": [], "n": 0}, "by_source": {}, "min_jes_per_source": 500},
            "active_lifetime": {"by_source": {}, "overall": {"buckets": [], "probabilities": [], "n": 0}},
            "fanout": {"by_attribute": {}}
        }"#;
        let bp: BehavioralPriors = serde_json::from_str(legacy).expect("legacy parse");
        assert!(
            bp.tb_anchor.is_none(),
            "missing tb_anchor field should deserialise as None"
        );
    }
}

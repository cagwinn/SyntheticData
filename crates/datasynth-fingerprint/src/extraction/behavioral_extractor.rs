//! Behavioral-prior extraction from corpus GL data.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use chrono::NaiveDate;
use datasynth_eval::behavioral_fidelity::loader::{load_csv_records, load_parquet_records};
use datasynth_eval::behavioral_fidelity::math::pearson_lag1_correlation;
use datasynth_eval::behavioral_fidelity::Record;

use crate::error::FingerprintError;
use crate::models::behavioral::BehavioralPriors;

use crate::models::behavioral::{
    ActiveLifetimePrior, ActiveSegmentsPrior, CategoricalDistribution, EntityCluster,
    EntityClustersPrior, FanoutPrior, IetSummary, LagSummary, LineCountHistogram, LinesPerJePrior,
    LognormalAmount, LognormalParams, PerSourceAmountPrior, PerSourceAttributePrior,
    PerSourceIetPrior, PerSourceRolePrior, PostingLagPrior, SourceMixPrior, SourceSegmentSummary,
    ACTIVE_LIFETIME_DAY_BUCKETS, FANOUT_BUCKETS, LINE_COUNT_BUCKETS, SEGMENT_COUNT_BUCKETS,
    SEGMENT_GAP_BUCKETS,
};
use crate::models::EmpiricalCdf;

use super::reference_extractor::extract_reference_formats;
use super::user_extractor::extract_user_personas;

/// SP4.5 — Default minimum row count for a user to be included in the
/// `UserPersonaPrior`.  Users appearing fewer than this many times are dropped
/// to avoid over-fitting one-off entries and to preserve privacy.
pub const DEFAULT_MIN_USER_RECORDS: usize = 100;

/// Default minimum occurrence count for a reference template to be retained
/// in the per-source reference-format prior.
pub const DEFAULT_MIN_REFERENCE_OCCURRENCES: usize = 10;

/// Default minimum row-share for a Source code to appear individually in the mix.
pub const DEFAULT_MIN_SOURCE_THRESHOLD: f64 = 0.005;

/// SP3.8b — Default minimum observation count per source.  Codes appearing
/// fewer than this many times in a single client's data are dropped from
/// the source_mix distribution to keep per-code event density high in
/// downstream generation.
pub const DEFAULT_MIN_SOURCE_OBSERVATIONS: usize = 1000;

/// Build a `SourceMixPrior` from a slice of records.
///
/// Two independent filters are applied:
/// 1. `min_threshold` — minimum row-share (probability).  Codes below this
///    fraction are rolled into `other_fraction`.
/// 2. `min_observations` — SP3.8b — minimum raw observation count.  Codes
///    that appear fewer than this many times are dropped regardless of their
///    fractional share.  Dropped mass is added to `other_fraction`.
///    After both filters probabilities are renormalised to sum to 1.0.
pub fn extract_source_mix(
    records: &[Record],
    min_threshold: f64,
    min_observations: usize,
) -> SourceMixPrior {
    if records.is_empty() {
        return SourceMixPrior {
            probabilities: BTreeMap::new(),
            other_fraction: 0.0,
            min_threshold,
        };
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for r in records {
        *counts.entry(r.source.clone()).or_insert(0) += 1;
    }

    // SP3.8b — drop the long tail of low-volume sources to concentrate the
    // distribution on dominant codes.  The probability-threshold filter still
    // applies separately below.
    counts.retain(|_, c| *c >= min_observations);

    let total = records.len() as f64;
    let mut probabilities = BTreeMap::new();
    let mut other = 0.0;
    for (src, c) in counts {
        let frac = c as f64 / total;
        if frac >= min_threshold {
            probabilities.insert(src, frac);
        } else {
            other += frac;
        }
    }

    // Renormalise retained probabilities so they sum to 1.0, rolling the
    // removed mass from dropped sources into other_fraction.
    let retained_sum: f64 = probabilities.values().sum();
    let dropped_mass = 1.0 - retained_sum - other;
    let other_fraction = other + dropped_mass;

    if retained_sum > 0.0 {
        for v in probabilities.values_mut() {
            *v /= retained_sum;
        }
    }

    SourceMixPrior {
        probabilities,
        other_fraction,
        min_threshold,
    }
}

/// Minimum sample count for a Source to receive its own IET summary.
pub const DEFAULT_MIN_IET_SAMPLES: usize = 100;

/// Extract per-Source inter-event-time distributions in days.
pub fn extract_per_source_iet(records: &[Record], min_samples: usize) -> PerSourceIetPrior {
    let mut by_source: BTreeMap<String, Vec<NaiveDate>> = BTreeMap::new();
    for r in records {
        by_source
            .entry(r.source.clone())
            .or_default()
            .push(r.entry_date);
    }
    let mut summaries: BTreeMap<String, IetSummary> = BTreeMap::new();
    for (source, mut dates) in by_source {
        if dates.len() < 2 {
            continue;
        }
        dates.sort();
        let iets: Vec<f64> = dates
            .windows(2)
            .map(|w| (w[1] - w[0]).num_days() as f64)
            .collect();
        if iets.len() < min_samples {
            continue;
        }
        let cdf = build_empirical_cdf(&format!("iet_{source}"), &iets);
        let lognormal = fit_lognormal(&iets);
        let auto = pearson_lag1_correlation(&iets).unwrap_or(0.0);
        summaries.insert(
            source,
            IetSummary {
                n: iets.len(),
                empirical_cdf_days: cdf,
                lognormal_fit: lognormal,
                lag1_autocorr: auto,
            },
        );
    }
    PerSourceIetPrior {
        by_source: summaries,
    }
}

fn build_empirical_cdf(column: &str, samples: &[f64]) -> EmpiricalCdf {
    let mut sorted: Vec<f64> = samples.iter().copied().filter(|x| x.is_finite()).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    EmpiricalCdf::from_sorted_values(column.to_string(), sorted)
}

/// Default minimum sample count for a Source to receive its own LagSummary.
pub const DEFAULT_MIN_LAG_SAMPLES: usize = 100;

/// Per-Source posting lag in days = EffectiveDate - EntryDate. Can be negative (backdating).
pub fn extract_posting_lag(records: &[Record], min_samples: usize) -> Option<PostingLagPrior> {
    if records.is_empty() {
        return None;
    }
    let mut by_source: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for r in records {
        let lag = (r.effective_date - r.entry_date).num_days() as f64;
        by_source.entry(r.source.clone()).or_default().push(lag);
    }
    let mut summaries: BTreeMap<String, LagSummary> = BTreeMap::new();
    for (source, samples) in by_source {
        if samples.len() < min_samples {
            continue;
        }
        let n = samples.len();
        let mean = samples.iter().sum::<f64>() / n as f64;
        let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
        let cdf = build_empirical_cdf(&format!("lag_{source}"), &samples);
        summaries.insert(
            source,
            LagSummary {
                empirical_cdf_days: cdf,
                mean,
                stddev: var.sqrt(),
                n,
            },
        );
    }
    if summaries.is_empty() {
        None
    } else {
        Some(PostingLagPrior {
            by_source: summaries,
        })
    }
}

fn fit_lognormal(samples: &[f64]) -> Option<LognormalParams> {
    let log_samples: Vec<f64> = samples
        .iter()
        .filter(|&&x| x.is_finite() && x > 0.0)
        .map(|&x| (x + 1.0).ln())
        .collect();
    if log_samples.len() < 3 {
        return None;
    }
    let n = log_samples.len() as f64;
    let mean: f64 = log_samples.iter().sum::<f64>() / n;
    let var: f64 = log_samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n.max(1.0);
    Some(LognormalParams {
        mu: mean,
        sigma: var.sqrt(),
    })
}

/// Per-Source active lifetime in days = max(EntryDate) - min(EntryDate).
pub fn extract_active_lifetime(records: &[Record]) -> ActiveLifetimePrior {
    let mut by_source: BTreeMap<String, (NaiveDate, NaiveDate)> = BTreeMap::new();
    for r in records {
        let d = r.entry_date;
        by_source
            .entry(r.source.clone())
            .and_modify(|(lo, hi)| {
                if d < *lo {
                    *lo = d;
                }
                if d > *hi {
                    *hi = d;
                }
            })
            .or_insert((d, d));
    }
    let lifetimes_by_source: Vec<u32> = by_source
        .values()
        .map(|(lo, hi)| hi.signed_duration_since(*lo).num_days().max(0) as u32)
        .collect();
    let (overall, _) = LineCountHistogram::build(&lifetimes_by_source, ACTIVE_LIFETIME_DAY_BUCKETS);

    let mut per_source_hists: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
    for (src, (lo, hi)) in &by_source {
        let life = hi.signed_duration_since(*lo).num_days().max(0) as u32;
        let (h, _) = LineCountHistogram::build(&[life], ACTIVE_LIFETIME_DAY_BUCKETS);
        per_source_hists.insert(src.clone(), h);
    }
    ActiveLifetimePrior {
        by_source: per_source_hists,
        overall,
    }
}

type AttributeProjector = fn(&Record) -> Option<String>;

/// Build the bipartite fan-out prior across {GLAccount, CostCenter, ProfitCenter, TradingPartner}.
pub fn extract_fanout(records: &[Record]) -> FanoutPrior {
    let attributes: [(&str, AttributeProjector); 4] = [
        ("GLAccount", |r| Some(r.gl_account.clone())),
        ("CostCenter", |r| r.cost_center.clone()),
        ("ProfitCenter", |r| r.profit_center.clone()),
        ("TradingPartner", |r| r.trading_partner.clone()),
    ];
    let mut by_attribute: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
    for (name, proj) in attributes {
        let mut sources_per_value: BTreeMap<String, HashSet<String>> = BTreeMap::new();
        for r in records {
            if let Some(v) = proj(r) {
                sources_per_value
                    .entry(v)
                    .or_default()
                    .insert(r.source.clone());
            }
        }
        let fanouts: Vec<u32> = sources_per_value.values().map(|s| s.len() as u32).collect();
        let (hist, _) = LineCountHistogram::build(&fanouts, FANOUT_BUCKETS);
        by_attribute.insert(name.to_string(), hist);
    }
    FanoutPrior { by_attribute }
}

/// Default minimum observations for a (source, attribute) pair to be retained
/// in the per-source attribute prior.
pub const DEFAULT_MIN_ATTRIBUTE_OBSERVATIONS: usize = 10;

/// SP3.7 — Extract per-source conditional distributions for downstream
/// attributes (GL account, cost center, profit center).  For each
/// (source, attribute) pair the function builds a categorical distribution
/// over the observed attribute values.  Pairs with fewer than
/// `min_observations` rows are dropped.
pub fn extract_per_source_attribute(
    records: &[Record],
    min_observations: usize,
) -> PerSourceAttributePrior {
    // Outer: source code. Middle: attribute name. Inner: value → count.
    let mut counts: BTreeMap<String, BTreeMap<String, BTreeMap<String, usize>>> = BTreeMap::new();

    for r in records {
        if r.source.is_empty() {
            continue;
        }
        let source_map = counts.entry(r.source.clone()).or_default();

        if !r.gl_account.is_empty() {
            *source_map
                .entry("gl_account".to_string())
                .or_default()
                .entry(r.gl_account.clone())
                .or_default() += 1;
        }
        if let Some(cc) = r.cost_center.as_ref().filter(|s| !s.is_empty()) {
            *source_map
                .entry("cost_center".to_string())
                .or_default()
                .entry(cc.clone())
                .or_default() += 1;
        }
        if let Some(pc) = r.profit_center.as_ref().filter(|s| !s.is_empty()) {
            *source_map
                .entry("profit_center".to_string())
                .or_default()
                .entry(pc.clone())
                .or_default() += 1;
        }
        // SP3.8a — trading_partner conditional per source.
        if let Some(tp) = r.trading_partner.as_ref().filter(|s| !s.is_empty()) {
            *source_map
                .entry("trading_partner".to_string())
                .or_default()
                .entry(tp.clone())
                .or_default() += 1;
        }
    }

    // Build the final prior, dropping (source, attribute) pairs below threshold.
    let by_source = counts
        .into_iter()
        .filter_map(|(source, attr_map)| {
            let kept: BTreeMap<String, CategoricalDistribution> = attr_map
                .into_iter()
                .filter_map(|(attr, value_counts)| {
                    let total: usize = value_counts.values().sum();
                    if total < min_observations {
                        None
                    } else {
                        Some((attr, CategoricalDistribution::from_counts(value_counts)))
                    }
                })
                .collect();
            if kept.is_empty() {
                None
            } else {
                Some((source, kept))
            }
        })
        .collect();

    PerSourceAttributePrior {
        by_source,
        min_observations,
    }
}

/// SP4.6 — Default minimum observation count per `(source, role, gl_account)` triple
/// before the value is retained in the per-source-role GL conditional.  Privacy gate.
pub const DEFAULT_MIN_SOURCE_ROLE_OBSERVATIONS: usize = 10;

/// SP4.6 — Extract per-(source, line_role) GL account categorical distributions.
///
/// `line_role` is derived from the sign of `functional_amount`:
/// - amount > 0  → "DR"
/// - amount < 0  → "CR"
/// - amount == 0 → skipped
///
/// Only `(source, role)` pairs with ≥ `min_observations` total rows and
/// individual `(source, role, gl_account)` values with ≥ `min_observations`
/// raw observations are retained.  This double-gates the output to avoid
/// PII leakage from low-volume entries.
pub fn extract_source_role_gl(records: &[Record], min_observations: usize) -> PerSourceRolePrior {
    // Outer: source. Middle: role ("DR"|"CR"). Inner: gl_account → count.
    let mut counts: BTreeMap<String, BTreeMap<String, BTreeMap<String, usize>>> = BTreeMap::new();

    for r in records {
        if r.source.is_empty() || r.gl_account.is_empty() {
            continue;
        }
        let role = if r.functional_amount > 0.0 {
            "DR"
        } else if r.functional_amount < 0.0 {
            "CR"
        } else {
            continue; // zero-amount lines contribute no signal
        };
        *counts
            .entry(r.source.clone())
            .or_default()
            .entry(role.to_string())
            .or_default()
            .entry(r.gl_account.clone())
            .or_default() += 1;
    }

    // Build the prior, applying min_observations per-value and per-(source,role).
    let mut by_source_and_role: BTreeMap<String, BTreeMap<String, CategoricalDistribution>> =
        BTreeMap::new();

    for (source, role_map) in counts {
        let mut roles: BTreeMap<String, CategoricalDistribution> = BTreeMap::new();
        for (role, value_counts) in role_map {
            // Filter individual values below the threshold.
            let filtered: BTreeMap<String, usize> = value_counts
                .into_iter()
                .filter(|(_, c)| *c >= min_observations)
                .collect();
            let total: usize = filtered.values().sum();
            if total < min_observations {
                continue;
            }
            roles.insert(role, CategoricalDistribution::from_counts(filtered));
        }
        if !roles.is_empty() {
            by_source_and_role.insert(source, roles);
        }
    }

    PerSourceRolePrior { by_source_and_role }
}

/// Default minimum observation count for a `(source, gl_prefix)` amount pair
/// to be retained in `PerSourceAmountPrior::by_source_and_class`.  Pairs with
/// fewer observations drop to the per-source marginal.  Privacy gate.
pub const DEFAULT_MIN_AMOUNT_OBSERVATIONS: usize = 10;

/// SP4.3 — Extract per-(source, gl_prefix) log-normal amount parameters from a
/// slice of records.
///
/// The "gl_prefix" key is the first 4 characters of the GL account number,
/// providing enough granularity to separate major balance-sheet categories
/// (e.g. "0041" vs "0022" vs "0047") without over-fitting.
///
/// Only absolute values > 0 are included in the fit; zero-amount lines are
/// dropped.  Groups below `min_observations` are silently excluded — their
/// source falls back to the source-marginal during sampling.
pub fn extract_source_amount_conditionals(
    records: &[Record],
    min_observations: usize,
) -> PerSourceAmountPrior {
    use std::collections::BTreeMap;

    // Accumulate raw absolute-amount values per (source, gl_prefix) and per source.
    let mut by_pair: BTreeMap<(String, String), Vec<f64>> = BTreeMap::new();
    let mut by_src: BTreeMap<String, Vec<f64>> = BTreeMap::new();

    for r in records {
        let abs_amt = r.functional_amount.abs();
        if abs_amt <= 0.0 || !abs_amt.is_finite() {
            continue;
        }
        if r.source.is_empty() {
            continue;
        }
        let gl_prefix: String = if r.gl_account.len() >= 4 {
            r.gl_account[..4].to_string()
        } else {
            r.gl_account.clone()
        };
        by_pair
            .entry((r.source.clone(), gl_prefix))
            .or_default()
            .push(abs_amt);
        by_src.entry(r.source.clone()).or_default().push(abs_amt);
    }

    // Fit log-normal to each group that meets the threshold.
    let mut by_source_and_class: BTreeMap<String, BTreeMap<String, LognormalAmount>> =
        BTreeMap::new();
    for ((source, gl_prefix), values) in by_pair {
        if values.len() < min_observations {
            continue;
        }
        if let Some(params) = fit_lognormal_amount(&values) {
            by_source_and_class
                .entry(source)
                .or_default()
                .insert(gl_prefix, params);
        }
    }

    // Source-marginal fallback.
    let mut by_source: BTreeMap<String, LognormalAmount> = BTreeMap::new();
    for (source, values) in by_src {
        if values.len() < min_observations {
            continue;
        }
        if let Some(params) = fit_lognormal_amount(&values) {
            by_source.insert(source, params);
        }
    }

    PerSourceAmountPrior {
        by_source_and_class,
        by_source,
    }
}

/// Fit log-normal parameters to a slice of positive absolute amounts.
///
/// Returns `None` when fewer than 2 finite values are present.
fn fit_lognormal_amount(values: &[f64]) -> Option<LognormalAmount> {
    let log_vals: Vec<f64> = values
        .iter()
        .filter(|&&v| v > 0.0 && v.is_finite())
        .map(|&v| v.ln())
        .collect();
    if log_vals.len() < 2 {
        return None;
    }
    let n = log_vals.len() as f64;
    let mu = log_vals.iter().sum::<f64>() / n;
    let var = log_vals.iter().map(|x| (x - mu).powi(2)).sum::<f64>() / n.max(1.0);
    let sigma = var.sqrt();

    // Compute median of the original absolute values.
    let mut sorted = values.to_vec();
    sorted.retain(|v| *v > 0.0 && v.is_finite());
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_abs = if sorted.is_empty() {
        0.0
    } else if sorted.len().is_multiple_of(2) {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
    } else {
        sorted[sorted.len() / 2]
    };

    Some(LognormalAmount {
        mu,
        sigma,
        n: log_vals.len(),
        median_abs,
    })
}

/// Default minimum JE count for a Source to receive its own histogram.
pub const DEFAULT_MIN_JES_PER_SOURCE: usize = 500;

/// Build LinesPerJePrior — overall + per-Source histogram of line counts.
pub fn extract_lines_per_je(records: &[Record], min_jes_per_source: usize) -> LinesPerJePrior {
    let mut lines_per_je: BTreeMap<String, u32> = BTreeMap::new();
    let mut source_of_je: BTreeMap<String, String> = BTreeMap::new();
    for r in records {
        *lines_per_je.entry(r.je_number.clone()).or_insert(0) += 1;
        source_of_je
            .entry(r.je_number.clone())
            .or_insert_with(|| r.source.clone());
    }
    let overall_values: Vec<u32> = lines_per_je.values().copied().collect();
    let (overall, _) = LineCountHistogram::build(&overall_values, LINE_COUNT_BUCKETS);

    let mut by_source_values: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for (je, n_lines) in &lines_per_je {
        if let Some(src) = source_of_je.get(je) {
            by_source_values
                .entry(src.clone())
                .or_default()
                .push(*n_lines);
        }
    }
    let mut by_source: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
    for (src, values) in by_source_values {
        if values.len() < min_jes_per_source {
            continue;
        }
        let (hist, _) = LineCountHistogram::build(&values, LINE_COUNT_BUCKETS);
        by_source.insert(src, hist);
    }

    LinesPerJePrior {
        overall,
        by_source,
        min_jes_per_source,
    }
}

pub type BehavioralResult<T> = Result<T, FingerprintError>;

/// Build a fully-populated `BehavioralPriors` for one client/data file.
pub fn extract_behavioral_priors(
    records: &[Record],
    industry: &str,
) -> BehavioralResult<BehavioralPriors> {
    Ok(BehavioralPriors {
        schema_version: BehavioralPriors::SCHEMA_VERSION,
        generator_version: env!("CARGO_PKG_VERSION").to_string(),
        industry: industry.to_string(),
        n_client_inputs: 1,
        n_rows_aggregated: records.len(),
        source_mix: extract_source_mix(
            records,
            DEFAULT_MIN_SOURCE_THRESHOLD,
            DEFAULT_MIN_SOURCE_OBSERVATIONS,
        ),
        per_source_iet: extract_per_source_iet(records, DEFAULT_MIN_IET_SAMPLES),
        lines_per_je: extract_lines_per_je(records, DEFAULT_MIN_JES_PER_SOURCE),
        active_lifetime: extract_active_lifetime(records),
        fanout: extract_fanout(records),
        posting_lag: extract_posting_lag(records, DEFAULT_MIN_LAG_SAMPLES),
        active_segments: Some(extract_active_segments(records)),
        entity_clusters: Some(extract_entity_clusters(records)),
        per_source_attribute: Some(extract_per_source_attribute(
            records,
            DEFAULT_MIN_ATTRIBUTE_OBSERVATIONS,
        )),
        tp_entity_clusters: Some(extract_tp_entity_clusters(records)),
        reference_formats: {
            let rf = extract_reference_formats(records, DEFAULT_MIN_REFERENCE_OCCURRENCES);
            if rf.by_source.is_empty() {
                None
            } else {
                Some(rf)
            }
        },
        coa_semantic: None,
        // SP4.5 — user_personas: the corpus GL files carry no user column, so
        // `extract_user_personas` returns an empty stub.  We always set `Some(stub)`
        // rather than `None` so that `LoadedPriors::user_personas` is present and the
        // `has_data()` guard on the generator side can be tested without real data.
        user_personas: {
            let up = extract_user_personas(records, DEFAULT_MIN_USER_RECORDS);
            Some(up)
        },
        // SP4.3 — per-(source, gl_prefix) amount conditionals.
        source_amount_conditionals: {
            let sac = extract_source_amount_conditionals(records, DEFAULT_MIN_AMOUNT_OBSERVATIONS);
            // Emit None when no (source, gl_prefix) pairs met the threshold —
            // avoids emitting an empty struct in bundles generated from tiny test data.
            if sac.by_source.is_empty() && sac.by_source_and_class.is_empty() {
                None
            } else {
                Some(sac)
            }
        },
        // SP4.6 — per-(source, line_role) GL account conditionals.
        source_role_gl_conditionals: {
            let srg = extract_source_role_gl(records, DEFAULT_MIN_SOURCE_ROLE_OBSERVATIONS);
            if srg.by_source_and_role.is_empty() {
                None
            } else {
                Some(srg)
            }
        },
        // SP6 — text_taxonomy: populated via extract_text_taxonomy (separate path).
        // The `extract_behavioral_priors` path (Record slice, JE-only) leaves this as None.
        text_taxonomy: None,
        // SP4.1 — TB anchoring is populated separately via `extract_tb_anchor_from_parquet`
        // (tb_extractor module) for callers that have access to real TB_XXX.parquet files.
        // The `extract_behavioral_priors` path (Record slice, JE-only) leaves this as None.
        tb_anchor: None,
    })
}

/// Convenience: load a parquet or CSV file and call `extract_behavioral_priors`.
pub fn extract_behavioral_priors_from_path(
    path: &Path,
    industry: &str,
) -> BehavioralResult<BehavioralPriors> {
    let records = match path.extension().and_then(|s| s.to_str()) {
        Some("parquet") => load_parquet_records(path)
            .map_err(|e| io_error_to_fp(format!("parquet load failed: {e}")))?,
        Some("csv") => {
            load_csv_records(path).map_err(|e| io_error_to_fp(format!("csv load failed: {e}")))?
        }
        _ => {
            return Err(io_error_to_fp(format!(
                "unsupported extension at {}",
                path.display()
            )));
        }
    };
    extract_behavioral_priors(&records, industry)
}

fn io_error_to_fp(msg: String) -> FingerprintError {
    FingerprintError::ExtractionError {
        extractor: "behavioral_priors".to_string(),
        message: msg,
    }
}

pub const SEGMENT_GAP_THRESHOLD_DAYS: i64 = 7;

/// Split a sorted+dedup'd date list into contiguous segments separated by
/// gaps > `gap_threshold` days. Returns (segments as (start, end), gap-day-values).
fn split_into_segments(
    dates: &[NaiveDate],
    gap_threshold: i64,
) -> (Vec<(NaiveDate, NaiveDate)>, Vec<u32>) {
    if dates.is_empty() {
        return (vec![], vec![]);
    }
    let mut segments = Vec::new();
    let mut gaps = Vec::new();
    let mut seg_start = dates[0];
    let mut seg_end = dates[0];
    for &d in &dates[1..] {
        let gap = (d - seg_end).num_days();
        if gap > gap_threshold {
            segments.push((seg_start, seg_end));
            gaps.push(gap as u32);
            seg_start = d;
            seg_end = d;
        } else {
            seg_end = d;
        }
    }
    segments.push((seg_start, seg_end));
    (segments, gaps)
}

/// Per-Source multi-segment active-pattern extractor.
pub fn extract_active_segments(records: &[Record]) -> ActiveSegmentsPrior {
    let mut by_source: BTreeMap<String, Vec<NaiveDate>> = BTreeMap::new();
    for r in records {
        by_source
            .entry(r.source.clone())
            .or_default()
            .push(r.entry_date);
    }
    let mut summaries: BTreeMap<String, SourceSegmentSummary> = BTreeMap::new();
    for (src, mut dates) in by_source {
        dates.sort();
        dates.dedup();
        if dates.len() < 2 {
            continue;
        }
        let (segments, gaps) = split_into_segments(&dates, SEGMENT_GAP_THRESHOLD_DAYS);
        let segment_count = segments.len() as u32;
        let segment_lengths: Vec<u32> = segments
            .iter()
            .map(|s| (s.1 - s.0).num_days().max(0) as u32)
            .collect();
        let (count_hist, _) = LineCountHistogram::build(&[segment_count], SEGMENT_COUNT_BUCKETS);
        let (length_hist, _) =
            LineCountHistogram::build(&segment_lengths, ACTIVE_LIFETIME_DAY_BUCKETS);
        let (gap_hist, _) = LineCountHistogram::build(&gaps, SEGMENT_GAP_BUCKETS);
        summaries.insert(
            src,
            SourceSegmentSummary {
                segment_count_histogram: count_hist,
                segment_length_histogram: length_hist,
                gap_length_histogram: gap_hist,
            },
        );
    }
    ActiveSegmentsPrior {
        by_source: summaries,
    }
}

/// Maximum number of Sources to cluster — bound on O(N²) Jaccard work.
const MAX_SOURCES_FOR_CLUSTERING: usize = 50;

/// Jaccard similarity threshold for a Source-pair to be considered "clustered".
const JACCARD_THRESHOLD: f64 = 0.3;

/// SP3.5a — Canonical SAP-style Source codes the synthetic generator emits.
/// Used by `normalise_source_code` to map heterogeneous corpus codes to
/// the synthetic vocabulary so the motif sampler can actually fire at lookup.
const CANONICAL_SAP_CODES: &[&str] = &[
    "KR", "RV", "DZ", "WE", "RE", "SA", "IM", "KZ", "AB", "AF", "DR", "KK", "K9", "KX", "PK", "RB",
    "RY", "SL", "ZP",
];

/// Map a corpus Source code to the synthetic generator's SAP vocabulary.
/// Unknown codes return `None` — they're excluded from clustering rather than
/// misclassified.
fn normalise_source_code(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if CANONICAL_SAP_CODES.contains(&trimmed) {
        return Some(trimmed.to_string());
    }
    // Best-effort numeric fallbacks observed in the corpus.
    match trimmed {
        "0" | "00" => Some("SA".to_string()),
        "1" | "01" => Some("RV".to_string()),
        "2" | "02" => Some("KR".to_string()),
        _ => None,
    }
}

/// SP3.3 — Discover clusters of Sources that share attribute pools (GL Account,
/// Cost Center, Profit Center, Trading Partner). Uses Jaccard-threshold
/// connected components on the top-K Sources by row count.
pub fn extract_entity_clusters(records: &[Record]) -> EntityClustersPrior {
    // 1. Count rows per Source so we can cap to the top K.
    let mut row_count_per_source: BTreeMap<String, usize> = BTreeMap::new();
    for r in records {
        *row_count_per_source.entry(r.source.clone()).or_insert(0) += 1;
    }
    let mut sorted_sources: Vec<(String, usize)> = row_count_per_source.into_iter().collect();
    sorted_sources.sort_by_key(|b| std::cmp::Reverse(b.1));
    let top_sources: Vec<String> = sorted_sources
        .into_iter()
        .take(MAX_SOURCES_FOR_CLUSTERING)
        .map(|(s, _)| s)
        .collect();
    let top_set: HashSet<&String> = top_sources.iter().collect();

    // 2. Build the per-Source attribute set across {GL, CC, PC, TP}.
    let mut attr_sets: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    for r in records {
        if !top_set.contains(&r.source) {
            continue;
        }
        let set = attr_sets.entry(r.source.clone()).or_default();
        set.insert(format!("GL:{}", r.gl_account));
        if let Some(cc) = &r.cost_center {
            set.insert(format!("CC:{cc}"));
        }
        if let Some(pc) = &r.profit_center {
            set.insert(format!("PC:{pc}"));
        }
        if let Some(tp) = &r.trading_partner {
            set.insert(format!("TP:{tp}"));
        }
    }

    // SP3.5a — Normalise Source codes so the resulting cluster members match
    // the synthetic generator's SAP vocabulary.
    let attr_sets: BTreeMap<String, HashSet<String>> = attr_sets
        .into_iter()
        .filter_map(|(raw, set)| normalise_source_code(&raw).map(|canonical| (canonical, set)))
        .fold(BTreeMap::new(), |mut acc, (canonical, set)| {
            acc.entry(canonical).or_default().extend(set);
            acc
        });

    // 3. Pairwise Jaccard, threshold, adjacency.
    let sources: Vec<String> = attr_sets.keys().cloned().collect();
    let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut edge_weights: BTreeMap<(String, String), f64> = BTreeMap::new();
    for i in 0..sources.len() {
        for j in (i + 1)..sources.len() {
            let a = &attr_sets[&sources[i]];
            let b = &attr_sets[&sources[j]];
            if a.is_empty() || b.is_empty() {
                continue;
            }
            let intersection = a.intersection(b).count() as f64;
            let union = a.union(b).count() as f64;
            if union == 0.0 {
                continue;
            }
            let jaccard = intersection / union;
            if jaccard >= JACCARD_THRESHOLD {
                adj.entry(sources[i].clone())
                    .or_default()
                    .push(sources[j].clone());
                adj.entry(sources[j].clone())
                    .or_default()
                    .push(sources[i].clone());
                let key = if sources[i] < sources[j] {
                    (sources[i].clone(), sources[j].clone())
                } else {
                    (sources[j].clone(), sources[i].clone())
                };
                edge_weights.insert(key, jaccard);
            }
        }
    }

    // 4. Connected components → clusters.
    let mut visited: HashSet<String> = HashSet::new();
    let mut clusters: Vec<EntityCluster> = Vec::new();
    for src in &sources {
        if visited.contains(src) {
            continue;
        }
        let mut members = Vec::new();
        let mut stack = vec![src.clone()];
        while let Some(s) = stack.pop() {
            if !visited.insert(s.clone()) {
                continue;
            }
            members.push(s.clone());
            if let Some(neighbors) = adj.get(&s) {
                for n in neighbors {
                    if !visited.contains(n) {
                        stack.push(n.clone());
                    }
                }
            }
        }
        if members.len() >= 2 {
            // Compute average Jaccard within the cluster.
            let mut sum = 0.0;
            let mut count = 0.0;
            for i in 0..members.len() {
                for j in (i + 1)..members.len() {
                    let key = if members[i] < members[j] {
                        (members[i].clone(), members[j].clone())
                    } else {
                        (members[j].clone(), members[i].clone())
                    };
                    if let Some(&w) = edge_weights.get(&key) {
                        sum += w;
                        count += 1.0;
                    }
                }
            }
            let avg_jaccard = if count > 0.0 { sum / count } else { 0.0 };
            clusters.push(EntityCluster {
                members,
                avg_jaccard,
            });
        }
    }

    let total_in_clusters: usize = clusters.iter().map(|c| c.members.len()).sum();
    let denom = sources.len().max(1);
    let clustering_rate = total_in_clusters as f64 / denom as f64;

    EntityClustersPrior {
        clusters,
        clustering_rate,
    }
}

/// Maximum number of TradingPartner values to cluster — bounds the O(N²) Jaccard work.
const MAX_TP_FOR_CLUSTERING: usize = 200;

/// SP3.12 — Discover clusters of TradingPartner values that share attribute pools
/// (GL Account, Cost Center, Profit Center, Source). Uses the same Jaccard-threshold
/// connected-components algorithm as `extract_entity_clusters` but keyed on TP.
/// These clusters drive the `TpMotifSampler` in the generator to emit TP values that
/// tend to share GL accounts, building triangle structure in the TP co-occurrence graph.
pub fn extract_tp_entity_clusters(records: &[Record]) -> EntityClustersPrior {
    // 1. Count rows per TP; cap to the top K.
    let mut row_count_per_tp: BTreeMap<String, usize> = BTreeMap::new();
    for r in records {
        if let Some(tp) = &r.trading_partner {
            if !tp.is_empty() {
                *row_count_per_tp.entry(tp.clone()).or_insert(0) += 1;
            }
        }
    }
    let mut sorted_tps: Vec<(String, usize)> = row_count_per_tp.into_iter().collect();
    sorted_tps.sort_by_key(|b| std::cmp::Reverse(b.1));
    let top_tps: Vec<String> = sorted_tps
        .into_iter()
        .take(MAX_TP_FOR_CLUSTERING)
        .map(|(tp, _)| tp)
        .collect();
    let top_set: HashSet<&String> = top_tps.iter().collect();

    // 2. Build the per-TP attribute set across {GL, CC, PC, Source}.
    let mut attr_sets: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    for r in records {
        let tp = match &r.trading_partner {
            Some(tp) if !tp.is_empty() && top_set.contains(tp) => tp.clone(),
            _ => continue,
        };
        let set = attr_sets.entry(tp).or_default();
        set.insert(format!("GL:{}", r.gl_account));
        if let Some(cc) = &r.cost_center {
            set.insert(format!("CC:{cc}"));
        }
        if let Some(pc) = &r.profit_center {
            set.insert(format!("PC:{pc}"));
        }
        // Include source code so TPs appearing on the same sources cluster.
        set.insert(format!("SRC:{}", r.source));
    }

    // 3. Pairwise Jaccard, threshold, adjacency.
    let tps: Vec<String> = attr_sets.keys().cloned().collect();
    let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut edge_weights: BTreeMap<(String, String), f64> = BTreeMap::new();
    for i in 0..tps.len() {
        for j in (i + 1)..tps.len() {
            let a = &attr_sets[&tps[i]];
            let b = &attr_sets[&tps[j]];
            if a.is_empty() || b.is_empty() {
                continue;
            }
            let intersection = a.intersection(b).count() as f64;
            let union = a.union(b).count() as f64;
            if union == 0.0 {
                continue;
            }
            let jaccard = intersection / union;
            if jaccard >= JACCARD_THRESHOLD {
                adj.entry(tps[i].clone()).or_default().push(tps[j].clone());
                adj.entry(tps[j].clone()).or_default().push(tps[i].clone());
                let key = if tps[i] < tps[j] {
                    (tps[i].clone(), tps[j].clone())
                } else {
                    (tps[j].clone(), tps[i].clone())
                };
                edge_weights.insert(key, jaccard);
            }
        }
    }

    // 4. Connected components → clusters.
    let mut visited: HashSet<String> = HashSet::new();
    let mut clusters: Vec<EntityCluster> = Vec::new();
    for tp in &tps {
        if visited.contains(tp) {
            continue;
        }
        let mut members = Vec::new();
        let mut stack = vec![tp.clone()];
        while let Some(t) = stack.pop() {
            if !visited.insert(t.clone()) {
                continue;
            }
            members.push(t.clone());
            if let Some(neighbors) = adj.get(&t) {
                for n in neighbors {
                    if !visited.contains(n) {
                        stack.push(n.clone());
                    }
                }
            }
        }
        if members.len() >= 2 {
            let mut sum = 0.0;
            let mut count = 0.0;
            for i in 0..members.len() {
                for j in (i + 1)..members.len() {
                    let key = if members[i] < members[j] {
                        (members[i].clone(), members[j].clone())
                    } else {
                        (members[j].clone(), members[i].clone())
                    };
                    if let Some(&w) = edge_weights.get(&key) {
                        sum += w;
                        count += 1.0;
                    }
                }
            }
            let avg_jaccard = if count > 0.0 { sum / count } else { 0.0 };
            clusters.push(EntityCluster {
                members,
                avg_jaccard,
            });
        }
    }

    let total_in_clusters: usize = clusters.iter().map(|c| c.members.len()).sum();
    let denom = tps.len().max(1);
    let clustering_rate = total_in_clusters as f64 / denom as f64;

    EntityClustersPrior {
        clusters,
        clustering_rate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, NaiveDate};
    use rand::{RngExt, SeedableRng};

    pub(crate) fn rec(src: &str) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, 1).expect("date");
        Record {
            source: src.into(),
            gl_account: "1".into(),
            cost_center: None,
            profit_center: None,
            trading_partner: None,
            je_number: "J1".into(),
            je_line_number: "001".into(),
            effective_date: d,
            entry_date: d,
            created_at: None,
            functional_amount: 1.0,
            header_text: String::new(),
            line_text: String::new(),
        }
    }

    #[test]
    fn source_mix_shares_match() {
        let mut recs: Vec<Record> = Vec::new();
        recs.extend(std::iter::repeat_with(|| rec("A")).take(60));
        recs.extend(std::iter::repeat_with(|| rec("B")).take(30));
        recs.extend(std::iter::repeat_with(|| rec("C")).take(10));
        // min_observations=0 so the obs-count filter does not fire here.
        let mix = extract_source_mix(&recs, DEFAULT_MIN_SOURCE_THRESHOLD, 0);
        // After renormalisation each source keeps its relative share.
        assert!((mix.probabilities["A"] - 0.6).abs() < 1e-9);
        assert!((mix.probabilities["B"] - 0.3).abs() < 1e-9);
        assert!((mix.probabilities["C"] - 0.1).abs() < 1e-9);
        assert!(mix.other_fraction.abs() < 1e-9);
    }

    #[test]
    fn source_mix_long_tail_rolls_into_other() {
        let mut recs: Vec<Record> = Vec::new();
        recs.extend(std::iter::repeat_with(|| rec("A")).take(995));
        for i in 1..=5 {
            recs.push(rec(&format!("X{i}")));
        }
        // min_observations=0 so only the probability threshold applies.
        let mix = extract_source_mix(&recs, 0.005, 0);
        // A has 99.5% share — after renormalisation over retained codes it is 1.0.
        // The X* codes are folded into other_fraction via the probability threshold.
        assert!(mix.probabilities.contains_key("A"));
        assert!(!mix.probabilities.contains_key("X1"));
        assert!(mix.other_fraction > 0.0);
    }

    #[test]
    fn source_mix_empty_input_returns_empty() {
        let mix = extract_source_mix(&[], DEFAULT_MIN_SOURCE_THRESHOLD, 0);
        assert!(mix.probabilities.is_empty());
        assert!(mix.other_fraction.abs() < 1e-9);
    }

    #[test]
    fn extract_source_mix_drops_low_volume_codes() {
        // KR has 1500 obs, RV has 100, DZ has 5 — only KR should survive
        // a min_observations=1000 threshold.
        let mut records = Vec::new();
        records.extend(std::iter::repeat_with(|| rec("KR")).take(1500));
        records.extend(std::iter::repeat_with(|| rec("RV")).take(100));
        records.extend(std::iter::repeat_with(|| rec("DZ")).take(5));

        // prob-threshold 0.0 so only the obs-count gate applies.
        let mix = extract_source_mix(&records, 0.0, 1000);

        assert!(mix.probabilities.contains_key("KR"), "KR should survive");
        assert!(
            !mix.probabilities.contains_key("RV"),
            "RV (100 obs) should be dropped"
        );
        assert!(
            !mix.probabilities.contains_key("DZ"),
            "DZ (5 obs) should be dropped"
        );
        // KR is the only survivor → renormalised probability must be 1.0.
        assert!(
            (mix.probabilities["KR"] - 1.0).abs() < 1e-9,
            "KR probability should be 1.0 after renormalisation"
        );
    }

    #[test]
    fn per_source_iet_basic() {
        let mut recs: Vec<Record> = Vec::new();
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        for i in 0..120 {
            let mut r = rec("A");
            r.entry_date = base + chrono::Duration::days(i);
            recs.push(r);
        }
        for i in 0..50 {
            let mut r = rec("B");
            r.entry_date = base + chrono::Duration::days(i);
            recs.push(r);
        }
        let p = extract_per_source_iet(&recs, 100);
        assert!(p.by_source.contains_key("A"));
        assert!(!p.by_source.contains_key("B"));
        let summ = &p.by_source["A"];
        assert_eq!(summ.n, 119);
        assert!(summ.lognormal_fit.is_some());
    }

    #[test]
    fn per_source_iet_constant_gap_zero_autocorr() {
        let mut recs: Vec<Record> = Vec::new();
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        for i in 0..200 {
            let mut r = rec("A");
            r.entry_date = base + chrono::Duration::days(3 * i);
            recs.push(r);
        }
        let p = extract_per_source_iet(&recs, 100);
        // Constant IET → zero variance → pearson returns None → falls back to 0.0.
        assert!((p.by_source["A"].lag1_autocorr).abs() < 1e-9);
    }

    #[test]
    fn lines_per_je_overall_known() {
        let mut recs: Vec<Record> = Vec::new();
        for _ in 0..3 {
            let mut r = rec("S");
            r.je_number = "JE-A".into();
            recs.push(r);
        }
        for _ in 0..2 {
            let mut r = rec("S");
            r.je_number = "JE-B".into();
            recs.push(r);
        }
        let mut r = rec("S");
        r.je_number = "JE-C".into();
        recs.push(r);

        let p = extract_lines_per_je(&recs, DEFAULT_MIN_JES_PER_SOURCE);
        let idx_1 = LINE_COUNT_BUCKETS.iter().position(|&b| b == 1).unwrap();
        let idx_2 = LINE_COUNT_BUCKETS.iter().position(|&b| b == 2).unwrap();
        let idx_3 = LINE_COUNT_BUCKETS.iter().position(|&b| b == 3).unwrap();
        assert!((p.overall.probabilities[idx_1] - 1.0 / 3.0).abs() < 1e-9);
        assert!((p.overall.probabilities[idx_2] - 1.0 / 3.0).abs() < 1e-9);
        assert!((p.overall.probabilities[idx_3] - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(p.overall.n, 3);
    }

    #[test]
    fn active_lifetime_basic() {
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        let mut recs: Vec<Record> = Vec::new();
        for i in 0..5 {
            let mut r = rec("A");
            r.entry_date = base + chrono::Duration::days(i * 6);
            recs.push(r);
        }
        for i in 0..5 {
            let mut r = rec("B");
            r.entry_date = base + chrono::Duration::days(i * 50);
            recs.push(r);
        }
        let p = extract_active_lifetime(&recs);
        let idx_7 = ACTIVE_LIFETIME_DAY_BUCKETS
            .iter()
            .position(|&b| b == 7)
            .unwrap();
        let idx_180 = ACTIVE_LIFETIME_DAY_BUCKETS
            .iter()
            .position(|&b| b == 180)
            .unwrap();
        // A: 4*6 = 24 days → bucket 7
        // B: 4*50 = 200 days → bucket 180
        assert!((p.overall.probabilities[idx_7] - 0.5).abs() < 1e-9);
        assert!((p.overall.probabilities[idx_180] - 0.5).abs() < 1e-9);
    }

    #[test]
    fn fanout_basic() {
        let mut recs: Vec<Record> = Vec::new();
        for &(src, gl) in &[("A", "X"), ("B", "X"), ("C", "X"), ("A", "Y")] {
            let mut r = rec(src);
            r.gl_account = gl.into();
            recs.push(r);
        }
        let p = extract_fanout(&recs);
        let hist = &p.by_attribute["GLAccount"];
        let idx_1 = FANOUT_BUCKETS.iter().position(|&b| b == 1).unwrap();
        let idx_3 = FANOUT_BUCKETS.iter().position(|&b| b == 3).unwrap();
        assert!((hist.probabilities[idx_1] - 0.5).abs() < 1e-9);
        assert!((hist.probabilities[idx_3] - 0.5).abs() < 1e-9);
    }

    #[test]
    fn posting_lag_known() {
        let mut recs: Vec<Record> = Vec::new();
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        for i in 0..120 {
            let mut r = rec("A");
            r.entry_date = base + chrono::Duration::days(i);
            r.effective_date = r.entry_date + chrono::Duration::days(5);
            recs.push(r);
        }
        for i in 0..120 {
            let mut r = rec("B");
            r.entry_date = base + chrono::Duration::days(i);
            r.effective_date = r.entry_date - chrono::Duration::days(2);
            recs.push(r);
        }
        let p = extract_posting_lag(&recs, 100).expect("non-empty");
        assert!((p.by_source["A"].mean - 5.0).abs() < 1e-9);
        assert!((p.by_source["B"].mean - (-2.0)).abs() < 1e-9);
        assert!((p.by_source["A"].stddev).abs() < 1e-9);
    }

    #[test]
    fn split_into_segments_known() {
        // Use day offsets from 2022-01-01 to avoid calendar overflow:
        // offsets 0,1,2 → segment [Jan1-Jan3]; offsets 14,15,16,17 → [Jan15-Jan18];
        // offset 49 → Feb19. Gaps: 12 days (Jan3→Jan15), 32 days (Jan18→Feb19).
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        let dates: Vec<NaiveDate> = [0i64, 1, 2, 14, 15, 16, 17, 49]
            .iter()
            .map(|&d| base + chrono::Duration::days(d))
            .collect();
        let (segs, gaps) = split_into_segments(&dates, 7);
        assert_eq!(segs.len(), 3); // [Jan1-Jan3], [Jan15-Jan18], [Feb19]
        assert_eq!(gaps.len(), 2); // gap of 12 days (Jan3→Jan15), gap of 32 days (Jan18→Feb19)
        assert_eq!(gaps[0], 12);
        assert_eq!(gaps[1], 32);
    }

    #[test]
    fn extract_active_segments_basic() {
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        let mut recs: Vec<Record> = Vec::new();
        for &day_off in &[0i64, 1, 2, 14, 15, 16, 17, 49] {
            let mut r = rec("A");
            r.entry_date = base + chrono::Duration::days(day_off);
            recs.push(r);
        }
        let p = extract_active_segments(&recs);
        assert!(p.by_source.contains_key("A"));
        let summary = &p.by_source["A"];
        // Expect 3 segments; bucket 3 should have full mass.
        let idx_3 = SEGMENT_COUNT_BUCKETS.iter().position(|&b| b == 3).unwrap();
        assert!((summary.segment_count_histogram.probabilities[idx_3] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn extract_entity_clusters_finds_shared_attrs() {
        // 4 sources using canonical SAP codes. KR,RV,DZ share GL accounts heavily; WE is isolated.
        let mut recs: Vec<Record> = Vec::new();
        // KR: GLs 1, 2, 3
        for gl in ["1", "2", "3"] {
            let mut r = rec("KR");
            r.gl_account = gl.into();
            recs.push(r);
        }
        // RV: GLs 1, 2, 3
        for gl in ["1", "2", "3"] {
            let mut r = rec("RV");
            r.gl_account = gl.into();
            recs.push(r);
        }
        // DZ: GLs 1, 2, 4
        for gl in ["1", "2", "4"] {
            let mut r = rec("DZ");
            r.gl_account = gl.into();
            recs.push(r);
        }
        // WE: GL 99 only
        let mut r = rec("WE");
        r.gl_account = "99".into();
        recs.push(r);

        let p = extract_entity_clusters(&recs);
        // Expect at least one cluster containing {KR, RV, DZ}; WE not clustered.
        let any_cluster_has_kr_rv_dz = p.clusters.iter().any(|c| {
            let members: HashSet<&String> = c.members.iter().collect();
            members.contains(&"KR".to_string())
                && members.contains(&"RV".to_string())
                && members.contains(&"DZ".to_string())
        });
        assert!(
            any_cluster_has_kr_rv_dz,
            "expected a cluster containing KR, RV, DZ"
        );
        let any_cluster_has_we = p
            .clusters
            .iter()
            .any(|c| c.members.iter().any(|m| m == "WE"));
        assert!(!any_cluster_has_we, "WE should be an isolate (no cluster)");
    }

    #[test]
    fn extract_entity_clusters_normalises_source_codes() {
        let base = chrono::NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        let mut recs: Vec<Record> = Vec::new();
        // Mix raw "0" (→ SA) and "KR" sources, all touching the same GL accounts.
        for source in ["0", "0", "0", "KR", "KR", "KR"] {
            for gl in ["1", "2", "3"] {
                let mut r = rec(source);
                r.entry_date = base;
                r.gl_account = gl.into();
                recs.push(r);
            }
        }
        let p = extract_entity_clusters(&recs);
        // No cluster member should be a raw numeric code — they should all be
        // canonical SAP-style codes.
        for cluster in &p.clusters {
            for member in &cluster.members {
                assert!(
                    !["0", "1", "2", "00", "01", "02"].contains(&member.as_str()),
                    "raw numeric code {member:?} should have been normalised"
                );
            }
        }
        // If any cluster formed, it must contain at least one canonical SAP code.
        if !p.clusters.is_empty() {
            let any_sap = p.clusters.iter().any(|c| {
                c.members
                    .iter()
                    .any(|m| ["KR", "RV", "DZ", "SA", "WE", "RE", "IM", "KZ"].contains(&m.as_str()))
            });
            assert!(
                any_sap,
                "expected at least one SAP-style canonical code in clusters"
            );
        }
    }

    #[test]
    fn normalise_source_code_known_mappings() {
        // Direct SAP codes pass through.
        assert_eq!(normalise_source_code("KR"), Some("KR".to_string()));
        assert_eq!(normalise_source_code("RV"), Some("RV".to_string()));
        // Numeric fallbacks.
        assert_eq!(normalise_source_code("0"), Some("SA".to_string()));
        assert_eq!(normalise_source_code("00"), Some("SA".to_string()));
        assert_eq!(normalise_source_code("1"), Some("RV".to_string()));
        assert_eq!(normalise_source_code("2"), Some("KR".to_string()));
        // Trimmed.
        assert_eq!(normalise_source_code("  KR  "), Some("KR".to_string()));
        // Unknown returns None.
        assert_eq!(normalise_source_code("XYZ"), None);
        assert_eq!(normalise_source_code(""), None);
    }

    fn make_record(
        src: &str,
        gl: &str,
        cost_center: Option<&str>,
        profit_center: Option<&str>,
    ) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, 1).expect("date");
        Record {
            source: src.into(),
            gl_account: gl.into(),
            cost_center: cost_center.map(|s| s.to_string()),
            profit_center: profit_center.map(|s| s.to_string()),
            trading_partner: None,
            je_number: "J1".into(),
            je_line_number: "001".into(),
            effective_date: d,
            entry_date: d,
            created_at: None,
            functional_amount: 1.0,
            header_text: String::new(),
            line_text: String::new(),
        }
    }

    #[test]
    fn extract_per_source_attribute_filters_low_observations() {
        // Build records: 15 KR rows all posting to "200001", 3 RV rows to "400001".
        let mut records: Vec<Record> = (0..15)
            .map(|_| make_record("KR", "200001", Some("CC1"), Some("PC1")))
            .collect();
        records.extend((0..3).map(|_| make_record("RV", "400001", Some("CC2"), Some("PC2"))));

        let prior = extract_per_source_attribute(&records, 10);

        // KR present (15 >= 10).
        assert!(prior.by_source.contains_key("KR"), "KR should be retained");
        let kr_gl = prior
            .conditional("KR", "gl_account")
            .expect("KR/gl_account");
        assert!(
            kr_gl.probabilities.contains_key("200001"),
            "200001 should appear"
        );
        assert_eq!(kr_gl.n, 15);
        assert!((kr_gl.probabilities["200001"] - 1.0).abs() < 1e-9);

        // RV dropped (3 < 10).
        assert!(
            !prior.by_source.contains_key("RV"),
            "RV should be filtered out"
        );
    }

    #[test]
    fn extract_per_source_attribute_skips_empty_source() {
        // Records with empty source should be silently skipped.
        let records: Vec<Record> = (0..20)
            .map(|_| make_record("", "100001", None, None))
            .collect();
        let prior = extract_per_source_attribute(&records, 5);
        assert!(
            prior.by_source.is_empty(),
            "empty source rows must be skipped"
        );
    }

    #[test]
    fn extract_per_source_attribute_multiple_values_normalise() {
        // Mix of two GL accounts for KR: 8 × "200001", 12 × "200002" (total 20 >= 10).
        let mut records: Vec<Record> = (0..8)
            .map(|_| make_record("KR", "200001", None, None))
            .collect();
        records.extend((0..12).map(|_| make_record("KR", "200002", None, None)));

        let prior = extract_per_source_attribute(&records, 10);
        let kr_gl = prior
            .conditional("KR", "gl_account")
            .expect("KR/gl_account");
        assert_eq!(kr_gl.n, 20);
        assert!((kr_gl.probabilities["200001"] - 0.4).abs() < 1e-9);
        assert!((kr_gl.probabilities["200002"] - 0.6).abs() < 1e-9);
        let total: f64 = kr_gl.probabilities.values().sum();
        assert!((total - 1.0).abs() < 1e-9, "probabilities must sum to 1.0");
    }

    #[test]
    fn extract_behavioral_priors_smoke() {
        // SP3.8b set DEFAULT_MIN_SOURCE_OBSERVATIONS = 1000 to drop the
        // long tail of low-volume sources from source_mix. The test must
        // give each source ≥ that threshold for source_mix to be populated.
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        let mut recs: Vec<Record> = Vec::new();
        for i in 0..1200i64 {
            let mut r = rec("A");
            r.je_number = format!("JE-A-{:04}", i / 3);
            r.entry_date = base + chrono::Duration::days(i);
            r.effective_date = r.entry_date + chrono::Duration::days(1);
            r.gl_account = format!("ACC-{}", i % 5);
            recs.push(r);
        }
        for i in 0..1200i64 {
            let mut r = rec("B");
            r.je_number = format!("JE-B-{:04}", i / 2);
            r.entry_date = base + chrono::Duration::days(i);
            r.effective_date = r.entry_date - chrono::Duration::days(1);
            r.gl_account = format!("ACC-{}", i % 7);
            recs.push(r);
        }
        let bp = extract_behavioral_priors(&recs, "test_industry").expect("ok");
        assert_eq!(bp.schema_version, BehavioralPriors::SCHEMA_VERSION);
        assert_eq!(bp.industry, "test_industry");
        assert_eq!(bp.n_client_inputs, 1);
        assert_eq!(bp.n_rows_aggregated, 2400);
        assert!(!bp.source_mix.probabilities.is_empty());
        assert!(bp.per_source_iet.by_source.contains_key("A"));
        assert!(bp.per_source_iet.by_source.contains_key("B"));
        assert!(bp.lines_per_je.overall.n > 0);
        assert!(bp.active_lifetime.overall.n > 0);
        assert_eq!(bp.fanout.by_attribute.len(), 4);
        assert!(bp.posting_lag.is_some());
        // SP3.7 — per_source_attribute should be populated (1200 rows per source ≥ threshold).
        assert!(
            bp.per_source_attribute.is_some(),
            "per_source_attribute should be extracted"
        );
        let psa = bp.per_source_attribute.as_ref().unwrap();
        // Both sources should appear; each has >= 10 GL accounts.
        assert!(psa.by_source.contains_key("A") || psa.by_source.contains_key("B"));
    }

    /// SP3.8a — trading_partner is extracted as a 4th attribute alongside
    /// gl_account, cost_center, profit_center.
    #[test]
    fn extract_per_source_attribute_includes_trading_partner() {
        // 15 KR records with trading_partner populated.
        let mut records: Vec<Record> = (0..15)
            .map(|_| {
                let mut r = make_record("KR", "200001", Some("CC1"), Some("PC1"));
                r.trading_partner = Some("V100".to_string());
                r
            })
            .collect();
        // 5 more KR records with a different TP value (still same source, total KR ≥ 10).
        records.extend((0..5).map(|_| {
            let mut r = make_record("KR", "200001", Some("CC1"), Some("PC1"));
            r.trading_partner = Some("V200".to_string());
            r
        }));
        // 3 RV records — below min_observations, should be dropped.
        records.extend((0..3).map(|_| {
            let mut r = make_record("RV", "400001", Some("CC2"), Some("PC2"));
            r.trading_partner = Some("V300".to_string());
            r
        }));

        let prior = extract_per_source_attribute(&records, 10);

        // KR/trading_partner must be present (20 observations ≥ 10).
        let kr_tp = prior
            .conditional("KR", "trading_partner")
            .expect("KR/trading_partner conditional must be present");
        assert!(
            kr_tp.probabilities.contains_key("V100"),
            "V100 should appear in KR trading_partner conditional"
        );
        assert!(
            kr_tp.probabilities.contains_key("V200"),
            "V200 should appear in KR trading_partner conditional"
        );
        assert_eq!(kr_tp.n, 20, "total observations should be 20");
        assert!(
            (kr_tp.probabilities["V100"] - 0.75).abs() < 1e-9,
            "V100 share should be 0.75"
        );
        assert!(
            (kr_tp.probabilities["V200"] - 0.25).abs() < 1e-9,
            "V200 share should be 0.25"
        );
        let total: f64 = kr_tp.probabilities.values().sum();
        assert!((total - 1.0).abs() < 1e-9, "probabilities must sum to 1.0");

        // RV was dropped (3 < 10).
        assert!(
            !prior.by_source.contains_key("RV"),
            "RV should be filtered out"
        );
    }

    /// SP3.12 W2 — TP entity cluster extraction test.
    /// Two TPs (T1, T2) share the same GL accounts; T3 is isolated.
    /// Expects a cluster {T1, T2} and T3 as an isolate.
    #[test]
    fn extract_tp_entity_clusters_finds_shared_attrs() {
        let d = NaiveDate::from_ymd_opt(2022, 1, 1).expect("date");
        let make_tp_rec = |tp: &str, gl: &str| Record {
            source: "KR".into(),
            gl_account: gl.into(),
            cost_center: None,
            profit_center: None,
            trading_partner: Some(tp.into()),
            je_number: "J1".into(),
            je_line_number: "001".into(),
            effective_date: d,
            entry_date: d,
            created_at: None,
            functional_amount: 1.0,
            header_text: String::new(),
            line_text: String::new(),
        };
        let mut recs = Vec::new();
        // T1 and T2 share GLs 10, 20, 30 — expect Jaccard ≥ 0.3.
        for gl in ["10", "20", "30"] {
            recs.push(make_tp_rec("T1", gl));
            recs.push(make_tp_rec("T2", gl));
        }
        // T3 uses a completely different GL — should be isolated.
        recs.push(make_tp_rec("T3", "99"));

        let p = extract_tp_entity_clusters(&recs);
        let cluster_has_t1_t2 = p.clusters.iter().any(|c| {
            let m: HashSet<&String> = c.members.iter().collect();
            m.contains(&"T1".to_string()) && m.contains(&"T2".to_string())
        });
        assert!(cluster_has_t1_t2, "expected a cluster containing T1 and T2");
        let cluster_has_t3 = p
            .clusters
            .iter()
            .any(|c| c.members.iter().any(|m| m == "T3"));
        assert!(!cluster_has_t3, "T3 should be an isolate (no cluster)");
        assert!(p.clustering_rate > 0.0, "clustering_rate must be > 0");
    }

    // ---- SP4.3 amount-conditional extractor tests --------------------------

    fn make_amount_rec(src: &str, gl: &str, amount: f64) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, 1).expect("date");
        Record {
            source: src.into(),
            gl_account: gl.into(),
            cost_center: None,
            profit_center: None,
            trading_partner: None,
            je_number: "J1".into(),
            je_line_number: "001".into(),
            effective_date: d,
            entry_date: d,
            created_at: None,
            functional_amount: amount,
            header_text: String::new(),
            line_text: String::new(),
        }
    }

    /// Extractor filters pairs below `min_observations` and keeps those above.
    #[test]
    fn extract_source_amount_conditionals_filters_low_count_pairs() {
        // KR/0041: 15 records — should be retained (≥10).
        // RV/0013: 3 records — should be dropped (<10).
        let mut records: Vec<Record> = (0..15)
            .map(|i| make_amount_rec("KR", "0041", 100.0 + i as f64))
            .collect();
        records.extend((0..3).map(|i| make_amount_rec("RV", "0013", 500.0 + i as f64)));

        let prior = extract_source_amount_conditionals(&records, 10);

        // KR marginal retained (15 ≥ 10).
        assert!(
            prior.by_source.contains_key("KR"),
            "KR marginal should be retained"
        );
        // KR/0041 retained (15 ≥ 10).
        assert!(
            prior
                .by_source_and_class
                .get("KR")
                .map(|m| m.contains_key("0041"))
                .unwrap_or(false),
            "KR/0041 pair should be retained"
        );
        // RV dropped (3 < 10).
        assert!(
            !prior.by_source.contains_key("RV"),
            "RV marginal should be dropped (only 3 observations)"
        );
    }

    /// Extractor produces sensible mu/sigma from known log-normal data.
    #[test]
    fn extract_source_amount_conditionals_lognormal_params_sensible() {
        // Generate 50 records from a known log-normal: mu=4.5, sigma=0.8.
        // Use fixed amounts close to the theoretical values.
        let base_amount = 4.5_f64.exp(); // ≈ 90
        let records: Vec<Record> = (0..50)
            .map(|i| {
                // Perturb slightly to avoid zero-variance.
                let amt = base_amount * (1.0 + 0.01 * ((i as f64) - 25.0));
                make_amount_rec("KR", "0041", amt)
            })
            .collect();

        let prior = extract_source_amount_conditionals(&records, 10);
        let params = prior
            .by_source_and_class
            .get("KR")
            .and_then(|m| m.get("0041"))
            .expect("KR/0041 params should be present");

        // mu should be close to ln(base_amount) = 4.5.
        assert!(
            (params.mu - 4.5).abs() < 0.1,
            "mu {:.3} should be close to 4.5",
            params.mu
        );
        assert_eq!(params.n, 50);
        assert!(params.median_abs > 0.0, "median_abs must be positive");
    }

    /// Extractor skips zero-amount and negative-amount lines (takes abs value, drops zero).
    #[test]
    fn extract_source_amount_conditionals_skips_zeros() {
        let mut records: Vec<Record> = (0..20)
            .map(|_| make_amount_rec("KR", "0041", 100.0))
            .collect();
        // Add some zero-amount records — should be ignored.
        records.extend((0..5).map(|_| make_amount_rec("KR", "0041", 0.0)));
        // Negative amounts are treated as absolute value.
        records.extend((0..5).map(|_| make_amount_rec("KR", "0041", -50.0)));

        let prior = extract_source_amount_conditionals(&records, 10);
        let params = prior
            .by_source_and_class
            .get("KR")
            .and_then(|m| m.get("0041"))
            .expect("KR/0041 should be present");
        // n should count only the 20 positive + 5 negative (abs) = 25 non-zero records.
        assert_eq!(params.n, 25, "zero-amount records must be excluded from n");
    }

    /// `extract_behavioral_priors` populates `source_amount_conditionals` when
    /// there are enough records.
    #[test]
    fn extract_behavioral_priors_populates_source_amount_conditionals() {
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        let mut recs: Vec<Record> = Vec::new();
        // 50 records for source "KR" with positive amounts.
        for i in 0..50 {
            let mut r = make_amount_rec("KR", "0041", 100.0 + i as f64);
            r.entry_date = base + chrono::Duration::days(i);
            r.effective_date = r.entry_date;
            r.je_number = format!("JE-{i}");
            recs.push(r);
        }
        let bp = extract_behavioral_priors(&recs, "test").expect("ok");
        assert!(
            bp.source_amount_conditionals.is_some(),
            "source_amount_conditionals should be populated"
        );
        let sac = bp.source_amount_conditionals.as_ref().unwrap();
        assert!(
            sac.by_source.contains_key("KR"),
            "KR marginal should be present"
        );
    }

    // ---- SP4.6 extractor tests ---------------------------------------------

    fn make_role_records(source: &str, role_sign: f64, gl: &str, n: usize) -> Vec<Record> {
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).expect("date");
        (0..n)
            .map(|i| Record {
                source: source.to_string(),
                gl_account: gl.to_string(),
                cost_center: None,
                profit_center: None,
                trading_partner: None,
                je_number: format!("J{i:06}"),
                je_line_number: "001".to_string(),
                effective_date: base,
                entry_date: base,
                created_at: None,
                functional_amount: role_sign * 100.0,
                header_text: String::new(),
                line_text: String::new(),
            })
            .collect()
    }

    /// DR records in expense class, CR records in AP class — role conditional
    /// should be populated for both (KR, DR) and (KR, CR).
    #[test]
    fn sp4_6_extract_source_role_gl_produces_role_conditionals() {
        let mut records = make_role_records("KR", 1.0, "6000", 15); // DR expense
        records.extend(make_role_records("KR", -1.0, "2000", 15)); // CR AP

        let prior = extract_source_role_gl(&records, 10);
        assert!(
            prior.conditional("KR", "DR").is_some(),
            "KR DR should be present"
        );
        assert!(
            prior.conditional("KR", "CR").is_some(),
            "KR CR should be present"
        );

        let dr_dist = prior.conditional("KR", "DR").unwrap();
        assert!(
            dr_dist.probabilities.contains_key("6000"),
            "DR should have 6000"
        );
        let cr_dist = prior.conditional("KR", "CR").unwrap();
        assert!(
            cr_dist.probabilities.contains_key("2000"),
            "CR should have 2000"
        );
    }

    /// Groups below min_observations are dropped.
    #[test]
    fn sp4_6_extract_source_role_gl_filters_low_counts() {
        // 15 KR-DR-6000 (passes), 3 KR-CR-2000 (below threshold of 10)
        let mut records = make_role_records("KR", 1.0, "6000", 15);
        records.extend(make_role_records("KR", -1.0, "2000", 3));

        let prior = extract_source_role_gl(&records, 10);
        assert!(
            prior.conditional("KR", "DR").is_some(),
            "KR DR should pass threshold"
        );
        // CR group has only 3 observations for 2000 → filtered at value level
        // total CR is also 3 → filtered at (source, role) level too
        assert!(
            prior.conditional("KR", "CR").is_none(),
            "KR CR with only 3 obs should be dropped"
        );
    }

    /// Zero-amount records are skipped.
    #[test]
    fn sp4_6_extract_source_role_gl_skips_zero_amounts() {
        let records: Vec<Record> = (0..20)
            .map(|i| Record {
                source: "SA".to_string(),
                gl_account: "4000".to_string(),
                cost_center: None,
                profit_center: None,
                trading_partner: None,
                je_number: format!("J{i:06}"),
                je_line_number: "001".to_string(),
                effective_date: NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(),
                entry_date: NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(),
                created_at: None,
                functional_amount: 0.0, // zero — should be skipped
                header_text: String::new(),
                line_text: String::new(),
            })
            .collect();
        let prior = extract_source_role_gl(&records, 10);
        assert!(
            prior.by_source_and_role.is_empty(),
            "zero-amount records should yield an empty prior"
        );
    }

    /// `extract_behavioral_priors` populates `source_role_gl_conditionals`
    /// when records contain sufficient DR/CR observations.
    #[test]
    fn sp4_6_extract_behavioral_priors_populates_source_role_gl_conditionals() {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        let mut recs: Vec<Record> = Vec::new();
        let base = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        let sources = ["KR", "RV", "SA"];
        let gl_dr = ["6000", "6100", "6200"];
        let gl_cr = ["2000", "2100", "1000"];
        for i in 0..1200usize {
            let src = sources[i % sources.len()];
            let (amt, gl) = if i % 2 == 0 {
                (100.0, gl_dr[i % gl_dr.len()])
            } else {
                (-100.0, gl_cr[i % gl_cr.len()])
            };
            recs.push(Record {
                source: src.to_string(),
                gl_account: gl.to_string(),
                cost_center: None,
                profit_center: None,
                trading_partner: None,
                je_number: format!("J{:06}", i / 3),
                je_line_number: format!("{:03}", (i % 3) + 1),
                effective_date: base + Duration::days(rng.random_range(0..365)),
                entry_date: base + Duration::days(rng.random_range(0..365)),
                created_at: None,
                functional_amount: amt,
                header_text: String::new(),
                line_text: String::new(),
            });
        }
        let bp = extract_behavioral_priors(&recs, "test").expect("ok");
        assert!(
            bp.source_role_gl_conditionals.is_some(),
            "source_role_gl_conditionals should be populated with sufficient data"
        );
    }
}

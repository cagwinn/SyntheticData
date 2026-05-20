//! Industry-level aggregation of behavioral priors.

use std::collections::BTreeMap;

use crate::error::FingerprintError;
use crate::models::behavioral::{
    AccountSemantic, ActiveLifetimePrior, ActiveSegmentsPrior, BehavioralPriors,
    CategoricalDistribution, CoaSemanticPrior, EntityCluster, EntityClustersPrior, FanoutPrior,
    IetSummary, LagSummary, LineCountHistogram, LinesPerJePrior, LognormalAmount,
    PerSourceAmountPrior, PerSourceAttributePrior, PerSourceIetPrior, PerSourceRolePrior,
    PostingLagPrior, ReferenceFormatPrior, ReferenceTemplate, SourceMixPrior, SourceSegmentSummary,
    TbAnchorPrior, TbTarget, UserBehavior, UserPersonaPrior,
};
use crate::models::EmpiricalCdf;
use datasynth_core::distributions::text_taxonomy::{
    TaxonomyMeta, TemplateEntry, TemplatePool, TextTaxonomyPrior,
};

pub type AggregationResult<T> = Result<T, FingerprintError>;

/// Number of quantile knots to retain in an aggregated EmpiricalCdf.
/// 256 is plenty for capturing distribution shape (W₁ error < 0.5% of range
/// on smooth distributions) while keeping `.dsf` bundles small enough to commit.
pub const AGGREGATED_CDF_KNOTS: usize = 256;

/// Resample an EmpiricalCdf to a fixed number of evenly-spaced quantile knots.
fn quantize_cdf(cdf: &EmpiricalCdf, n_knots: usize) -> EmpiricalCdf {
    if cdf.values.len() <= n_knots {
        return cdf.clone();
    }
    let probabilities: Vec<f64> = (1..=n_knots).map(|i| i as f64 / n_knots as f64).collect();
    let values: Vec<f64> = probabilities.iter().map(|&p| cdf.quantile(p)).collect();
    EmpiricalCdf {
        column: cdf.column.clone(),
        values,
        probabilities,
    }
}

pub fn aggregate_source_mix(inputs: &[&SourceMixPrior]) -> SourceMixPrior {
    if inputs.is_empty() {
        return SourceMixPrior::default();
    }
    let n = inputs.len() as f64;
    let mut probabilities: BTreeMap<String, f64> = BTreeMap::new();
    let mut other = 0.0;
    let min_threshold = inputs[0].min_threshold;
    for client in inputs {
        for (src, &p) in &client.probabilities {
            *probabilities.entry(src.clone()).or_insert(0.0) += p / n;
        }
        other += client.other_fraction / n;
    }
    let mut filtered = BTreeMap::new();
    for (src, p) in probabilities {
        if p >= min_threshold {
            filtered.insert(src, p);
        } else {
            other += p;
        }
    }
    SourceMixPrior {
        probabilities: filtered,
        other_fraction: other,
        min_threshold,
    }
}
pub fn aggregate_lines_per_je(inputs: &[&LinesPerJePrior]) -> LinesPerJePrior {
    if inputs.is_empty() {
        return LinesPerJePrior::default();
    }
    let mut overall = inputs[0].overall.clone();
    for &client in &inputs[1..] {
        if let Some(pooled) = overall.pool(&client.overall) {
            overall = pooled;
        }
    }
    let mut by_source: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
    for &client in inputs {
        for (src, hist) in &client.by_source {
            let merged = match by_source.get(src) {
                Some(existing) => existing.pool(hist).unwrap_or_else(|| hist.clone()),
                None => hist.clone(),
            };
            by_source.insert(src.clone(), merged);
        }
    }
    LinesPerJePrior {
        overall,
        by_source,
        min_jes_per_source: inputs[0].min_jes_per_source,
    }
}
pub fn aggregate_per_source_iet(inputs: &[&PerSourceIetPrior]) -> PerSourceIetPrior {
    if inputs.is_empty() {
        return PerSourceIetPrior::default();
    }
    // pooled_knots: source -> (concatenated values, sum(autocorr * n), sum(n))
    let mut pooled_knots: BTreeMap<String, (Vec<f64>, f64, usize)> = BTreeMap::new();
    for &client in inputs {
        for (src, summ) in &client.by_source {
            let entry = pooled_knots
                .entry(src.clone())
                .or_insert((Vec::new(), 0.0, 0));
            entry
                .0
                .extend(summ.empirical_cdf_days.values.iter().copied());
            entry.1 += summ.lag1_autocorr * summ.n as f64;
            entry.2 += summ.n;
        }
    }
    let mut by_source: BTreeMap<String, IetSummary> = BTreeMap::new();
    for (src, (mut values, auto_sum, n)) in pooled_knots {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if values.is_empty() {
            continue;
        }
        let full_cdf = EmpiricalCdf::from_sorted_values(format!("iet_{src}_agg"), values);
        let cdf = quantize_cdf(&full_cdf, AGGREGATED_CDF_KNOTS);
        let lag1_autocorr = if n > 0 { auto_sum / n as f64 } else { 0.0 };
        by_source.insert(
            src,
            IetSummary {
                n,
                empirical_cdf_days: cdf,
                lognormal_fit: None, // recompute on pooled samples requires raw IETs; skip during aggregation
                lag1_autocorr,
            },
        );
    }
    PerSourceIetPrior { by_source }
}
pub fn aggregate_active_lifetime(inputs: &[&ActiveLifetimePrior]) -> ActiveLifetimePrior {
    if inputs.is_empty() {
        return ActiveLifetimePrior::default();
    }
    let mut overall = inputs[0].overall.clone();
    for &client in &inputs[1..] {
        if let Some(pooled) = overall.pool(&client.overall) {
            overall = pooled;
        }
    }
    let mut by_source: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
    for &client in inputs {
        for (src, hist) in &client.by_source {
            let merged = match by_source.get(src) {
                Some(existing) => existing.pool(hist).unwrap_or_else(|| hist.clone()),
                None => hist.clone(),
            };
            by_source.insert(src.clone(), merged);
        }
    }
    ActiveLifetimePrior { by_source, overall }
}

pub fn aggregate_active_segments(inputs: &[&ActiveSegmentsPrior]) -> ActiveSegmentsPrior {
    if inputs.is_empty() {
        return ActiveSegmentsPrior::default();
    }
    let mut by_source: BTreeMap<String, SourceSegmentSummary> = BTreeMap::new();
    for &client in inputs {
        for (src, summ) in &client.by_source {
            let merged = match by_source.get(src) {
                Some(existing) => SourceSegmentSummary {
                    segment_count_histogram: existing
                        .segment_count_histogram
                        .pool(&summ.segment_count_histogram)
                        .unwrap_or_else(|| summ.segment_count_histogram.clone()),
                    segment_length_histogram: existing
                        .segment_length_histogram
                        .pool(&summ.segment_length_histogram)
                        .unwrap_or_else(|| summ.segment_length_histogram.clone()),
                    gap_length_histogram: existing
                        .gap_length_histogram
                        .pool(&summ.gap_length_histogram)
                        .unwrap_or_else(|| summ.gap_length_histogram.clone()),
                },
                None => summ.clone(),
            };
            by_source.insert(src.clone(), merged);
        }
    }
    ActiveSegmentsPrior { by_source }
}

pub fn aggregate_fanout(inputs: &[&FanoutPrior]) -> FanoutPrior {
    if inputs.is_empty() {
        return FanoutPrior::default();
    }
    let mut by_attribute: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
    for &client in inputs {
        for (attr, hist) in &client.by_attribute {
            let merged = match by_attribute.get(attr) {
                Some(existing) => existing.pool(hist).unwrap_or_else(|| hist.clone()),
                None => hist.clone(),
            };
            by_attribute.insert(attr.clone(), merged);
        }
    }
    FanoutPrior { by_attribute }
}

pub fn aggregate_posting_lag(inputs: &[&PostingLagPrior]) -> Option<PostingLagPrior> {
    if inputs.is_empty() {
        return None;
    }
    let mut pooled: BTreeMap<String, (Vec<f64>, usize)> = BTreeMap::new();
    for &client in inputs {
        for (src, summ) in &client.by_source {
            let entry = pooled.entry(src.clone()).or_insert((Vec::new(), 0));
            entry
                .0
                .extend(summ.empirical_cdf_days.values.iter().copied());
            entry.1 += summ.n;
        }
    }
    if pooled.is_empty() {
        return None;
    }
    let mut by_source: BTreeMap<String, LagSummary> = BTreeMap::new();
    for (src, (mut samples, n)) in pooled {
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let total = samples.len();
        let mean = if total > 0 {
            samples.iter().sum::<f64>() / total as f64
        } else {
            0.0
        };
        let var = if total > 0 {
            samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / total as f64
        } else {
            0.0
        };
        let full_cdf = EmpiricalCdf::from_sorted_values(format!("lag_{src}_agg"), samples);
        let cdf = quantize_cdf(&full_cdf, AGGREGATED_CDF_KNOTS);
        by_source.insert(
            src,
            LagSummary {
                empirical_cdf_days: cdf,
                mean,
                stddev: var.sqrt(),
                n,
            },
        );
    }
    Some(PostingLagPrior { by_source })
}

/// SP3.3 — Aggregate per-client entity-cluster priors by greedily merging
/// clusters that share at least one member. Averages clustering_rate across
/// clients.
pub fn aggregate_entity_clusters(inputs: &[&EntityClustersPrior]) -> EntityClustersPrior {
    if inputs.is_empty() {
        return EntityClustersPrior::default();
    }
    let mut all_clusters: Vec<EntityCluster> = inputs
        .iter()
        .flat_map(|p| p.clusters.iter().cloned())
        .collect();

    // Greedy merge: any two clusters sharing ≥1 member fuse into one.
    loop {
        let mut merged_pair: Option<(usize, usize)> = None;
        'outer: for i in 0..all_clusters.len() {
            for j in (i + 1)..all_clusters.len() {
                let i_set: std::collections::HashSet<&String> =
                    all_clusters[i].members.iter().collect();
                if all_clusters[j].members.iter().any(|m| i_set.contains(m)) {
                    merged_pair = Some((i, j));
                    break 'outer;
                }
            }
        }
        match merged_pair {
            Some((i, j)) => {
                let other = all_clusters.swap_remove(j);
                let mut merged_members: std::collections::HashSet<String> =
                    all_clusters[i].members.iter().cloned().collect();
                merged_members.extend(other.members);
                let avg = (all_clusters[i].avg_jaccard + other.avg_jaccard) / 2.0;
                let mut members: Vec<String> = merged_members.into_iter().collect();
                members.sort();
                all_clusters[i] = EntityCluster {
                    members,
                    avg_jaccard: avg,
                };
            }
            None => break,
        }
    }

    let avg_rate = inputs.iter().map(|p| p.clustering_rate).sum::<f64>() / inputs.len() as f64;
    EntityClustersPrior {
        clusters: all_clusters,
        clustering_rate: avg_rate,
    }
}

// ---------------------------------------------------------------------------
// SP3.9 W2 — GL-account namespace detection helpers
// ---------------------------------------------------------------------------

/// SP3.9 — Classifies a GL account / cost center / profit center value
/// into a coarse format family.  Values whose family doesn't match the
/// client's dominant family are stripped during aggregation to prevent
/// cross-client namespace clashes.
fn classify_account_format(value: &str) -> AccountFormat {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return AccountFormat::Empty;
    }
    // 0000xxxxxx — 10-digit zero-padded format (corpus enterprise client)
    if trimmed.len() == 10
        && trimmed.starts_with("0000")
        && trimmed.chars().all(|c| c.is_ascii_digit())
    {
        return AccountFormat::ZeroPadded10;
    }
    // xx.xxxxx — dotted notation (one observed corpus account-number family)
    if trimmed.contains('.')
        && trimmed
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    {
        return AccountFormat::Dotted;
    }
    // pure short numeric (3-6 digits) — another observed account-number family
    if trimmed.len() <= 6 && !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        return AccountFormat::ShortNumeric;
    }
    // Synthetic-default fallback (shouldn't be in real priors, but defensive)
    if trimmed.starts_with("GLAccount-")
        || trimmed.starts_with("CostCenter-")
        || trimmed.starts_with("ProfitCenter-")
    {
        return AccountFormat::SyntheticDefault;
    }
    AccountFormat::Other
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum AccountFormat {
    Empty,
    ZeroPadded10,
    Dotted,
    ShortNumeric,
    SyntheticDefault,
    Other,
}

/// SP3.9 — For a single client prior, determine the dominant AccountFormat
/// for a given attribute name (e.g. "gl_account", "cost_center", …).
/// Returns `None` if the attribute doesn't appear or has no non-empty
/// non-synthetic values.
fn detect_dominant_format(
    prior: &PerSourceAttributePrior,
    attribute: &str,
) -> Option<AccountFormat> {
    use std::collections::HashMap;
    let mut tally: HashMap<AccountFormat, usize> = HashMap::new();
    for attrs in prior.by_source.values() {
        if let Some(dist) = attrs.get(attribute) {
            for (value, &prob) in &dist.probabilities {
                let format = classify_account_format(value);
                if matches!(
                    format,
                    AccountFormat::Empty | AccountFormat::SyntheticDefault
                ) {
                    continue;
                }
                let count = (prob * dist.n as f64).round() as usize;
                *tally.entry(format).or_insert(0) += count;
            }
        }
    }
    tally.into_iter().max_by_key(|(_, n)| *n).map(|(f, _)| f)
}

// ---------------------------------------------------------------------------
// SP3.11 W1 — Cross-client GL namespace dominant-format detection & filtering
// ---------------------------------------------------------------------------

/// SP3.11 W1 — Determine the GL-namespace dominant format **across all clients**.
///
/// The aggregator was previously cleaning within-client format
/// contamination (SP3.9 W2), but preserved cross-client format
/// diversity in the aggregated bundle.  That diversity produced a
/// fragmented Source-Source adjacency graph in synthetic output
/// (different sources from different clients had no overlapping GL
/// accounts) — and the eval's P3 ClusteringGap / TriangleLogRatio
/// metrics stayed at ~36 / ~17 on the Source entity.
///
/// This function picks the single dominant format across all client
/// inputs by total `gl_account` observation count.  Client inputs
/// whose dominant GL format doesn't match the aggregated dominant
/// are subsequently filtered out by `is_client_in_dominant_namespace`.
fn detect_cross_client_dominant_gl_format(
    inputs: &[&PerSourceAttributePrior],
) -> Option<AccountFormat> {
    use std::collections::HashMap;
    let mut tally: HashMap<AccountFormat, usize> = HashMap::new();
    for &input in inputs {
        if let Some(client_dominant) = detect_dominant_format(input, "gl_account") {
            // Weight each client's vote by their total gl_account observations.
            let client_obs: usize = input
                .by_source
                .values()
                .filter_map(|attrs| attrs.get("gl_account"))
                .map(|gl_dist| gl_dist.n)
                .sum();
            *tally.entry(client_dominant).or_insert(0) += client_obs;
        }
    }
    tally.into_iter().max_by_key(|(_, n)| *n).map(|(f, _)| f)
}

/// SP3.11 W1 — Returns `true` when this client's dominant GL format
/// matches the cross-client-dominant `target`. Clients in other namespaces
/// are excluded from the aggregated bundle.
fn is_client_in_dominant_namespace(input: &PerSourceAttributePrior, target: AccountFormat) -> bool {
    detect_dominant_format(input, "gl_account") == Some(target)
}

// ---------------------------------------------------------------------------

/// SP3.7 — Aggregate per-source attribute priors across N clients.
///
/// For each (source, attribute) pair seen in any input, pool the raw
/// observation counts across all clients (by multiplying each
/// probability by the source's reported `n`), then renormalise.
///
/// `min_observations` is carried through from the inputs — if they
/// disagree, the aggregator picks the max (most conservative) value.
///
/// SP3.9 W2 — Before pooling, each client's contributions are filtered
/// so that only values matching that client's dominant GL-account format
/// (per attribute) are included.  This prevents cross-client namespace
/// clashes (e.g. SAP 0000xxxxxx vs. French 40.xxxxx) from fragmenting
/// the Source-Source adjacency graph and inflating ClusteringGap / TriangleLogRatio.
pub fn aggregate_per_source_attribute(
    inputs: &[&PerSourceAttributePrior],
) -> PerSourceAttributePrior {
    use std::collections::HashMap;

    if inputs.is_empty() {
        return PerSourceAttributePrior::default();
    }

    // Outer: source. Middle: attribute. Inner: value → pooled count.
    let mut pooled: BTreeMap<String, BTreeMap<String, BTreeMap<String, usize>>> = BTreeMap::new();

    // Attributes subject to namespace-format filtering.
    const FORMAT_FILTERED_ATTRS: &[&str] = &[
        "gl_account",
        "cost_center",
        "profit_center",
        "trading_partner",
    ];

    for input in inputs {
        // SP3.9 W2 — detect this client's dominant format per filtered attribute.
        let dominant: HashMap<&str, Option<AccountFormat>> = FORMAT_FILTERED_ATTRS
            .iter()
            .map(|&attr| (attr, detect_dominant_format(input, attr)))
            .collect();

        for (source, attr_map) in &input.by_source {
            let s_entry = pooled.entry(source.clone()).or_default();
            for (attr, dist) in attr_map {
                let a_entry = s_entry.entry(attr.clone()).or_default();
                // Reconstruct integer counts from (prob × n) and add.
                let dominant_format = dominant.get(attr.as_str()).copied().flatten();
                for (value, &prob) in &dist.probabilities {
                    // SP3.9 W2 — skip values not matching this client's dominant
                    // format for this attribute.  Empty and SyntheticDefault always
                    // pass through: Empty is semantically meaningful (real missing
                    // values), SyntheticDefault is the marginal-fallback generator
                    // output and must not be stripped.
                    if let Some(target) = dominant_format {
                        let value_format = classify_account_format(value);
                        if !matches!(
                            value_format,
                            AccountFormat::Empty | AccountFormat::SyntheticDefault
                        ) && value_format != target
                        {
                            continue;
                        }
                    }
                    let count = (prob * dist.n as f64).round() as usize;
                    *a_entry.entry(value.clone()).or_insert(0) += count;
                }
            }
        }
    }

    // Build the final prior. The aggregator does NOT re-filter by
    // min_observations — the per-client extractor already filtered at
    // ingest time, and post-aggregation a (source, attribute) that
    // accumulated cross-client may legitimately have plenty of data.
    //
    // SP3.9 W2 — additionally drop (source, attribute) entries that ended up
    // completely empty after format filtering (format clash in every client).
    let by_source = pooled
        .into_iter()
        .filter_map(|(source, attr_map)| {
            let attr_dists: BTreeMap<String, CategoricalDistribution> = attr_map
                .into_iter()
                .filter_map(|(attr, value_counts)| {
                    let total: usize = value_counts.values().sum();
                    if total == 0 {
                        None
                    } else {
                        Some((attr, CategoricalDistribution::from_counts(value_counts)))
                    }
                })
                .collect();
            if attr_dists.is_empty() {
                None
            } else {
                Some((source, attr_dists))
            }
        })
        .collect();

    // Carry through min_observations as the max across inputs (most
    // conservative — disclaimers the threshold to the consumer).
    let min_observations = inputs.iter().map(|p| p.min_observations).max().unwrap_or(0);

    PerSourceAttributePrior {
        by_source,
        min_observations,
    }
}

// ---------------------------------------------------------------------------
// SP4.2 — CoA semantic aggregation
// ---------------------------------------------------------------------------

/// SP4.2 — Aggregate per-client `CoaSemanticPrior` values by union.
///
/// Each account that appears in ANY client is included.  Conflict resolution:
/// when the same account number appears across clients with different
/// descriptions, the most-frequent description (mode) is used, or the first
/// non-empty one as fallback.  Optional fields (`account_type`, `account_class`,
/// etc.) use the first non-`None` value encountered.
pub fn aggregate_coa_semantic(inputs: &[&CoaSemanticPrior]) -> CoaSemanticPrior {
    if inputs.is_empty() {
        return CoaSemanticPrior::default();
    }

    // Collect all descriptions per account number for mode selection.
    let mut desc_counts: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    // First non-None optional fields per account.
    let mut first_optional: BTreeMap<String, AccountSemantic> = BTreeMap::new();

    for &client in inputs {
        for (account_number, sem) in &client.accounts {
            // Count description occurrences.
            *desc_counts
                .entry(account_number.clone())
                .or_default()
                .entry(sem.description.clone())
                .or_insert(0) += 1;

            // Merge optional fields: keep first non-None value seen.
            let entry = first_optional.entry(account_number.clone()).or_default();
            if entry.account_type.is_none() {
                entry.account_type.clone_from(&sem.account_type);
            }
            if entry.account_class.is_none() {
                entry.account_class.clone_from(&sem.account_class);
            }
            if entry.account_class_name.is_none() {
                entry.account_class_name.clone_from(&sem.account_class_name);
            }
            if entry.account_sub_class.is_none() {
                entry.account_sub_class.clone_from(&sem.account_sub_class);
            }
            if entry.account_sub_class_name.is_none() {
                entry
                    .account_sub_class_name
                    .clone_from(&sem.account_sub_class_name);
            }
            if entry.parent_account.is_none() {
                entry.parent_account.clone_from(&sem.parent_account);
            }
        }
    }

    let mut accounts = BTreeMap::new();
    for (account_number, desc_map) in desc_counts {
        // Pick the most frequent description (mode).
        let description = desc_map
            .into_iter()
            .filter(|(d, _)| !d.is_empty())
            .max_by_key(|(_, count)| *count)
            .map(|(d, _)| d)
            .unwrap_or_default();

        let optional = first_optional
            .get(&account_number)
            .cloned()
            .unwrap_or_default();

        accounts.insert(
            account_number,
            AccountSemantic {
                description,
                account_type: optional.account_type,
                account_class: optional.account_class,
                account_class_name: optional.account_class_name,
                account_sub_class: optional.account_sub_class,
                account_sub_class_name: optional.account_sub_class_name,
                parent_account: optional.parent_account,
            },
        );
    }

    CoaSemanticPrior { accounts }
}

// ---------------------------------------------------------------------------
// SP4.7 — Reference format aggregation
// ---------------------------------------------------------------------------

/// SP4.7 — Aggregate per-client `ReferenceFormatPrior` values by union.
///
/// Templates for the same source are merged across clients: templates are
/// combined, probabilities renormalized so they sum to 1 per source.
/// When the same template string appears in multiple clients it is deduplicated
/// (keeping the first example encountered) and its probabilities are averaged.
pub fn aggregate_reference_formats(inputs: &[&ReferenceFormatPrior]) -> ReferenceFormatPrior {
    if inputs.is_empty() {
        return ReferenceFormatPrior::default();
    }

    // src -> template_string -> (sum_probability, count, first_example)
    let mut accum: BTreeMap<String, BTreeMap<String, (f64, usize, String)>> = BTreeMap::new();

    for &client in inputs {
        for (src, templates) in &client.by_source {
            let src_entry = accum.entry(src.clone()).or_default();
            for tmpl in templates {
                let entry = src_entry.entry(tmpl.template.clone()).or_insert((
                    0.0,
                    0,
                    tmpl.example.clone(),
                ));
                entry.0 += tmpl.probability;
                entry.1 += 1;
            }
        }
    }

    let mut by_source: BTreeMap<String, Vec<ReferenceTemplate>> = BTreeMap::new();
    for (src, template_map) in accum {
        let mut templates: Vec<ReferenceTemplate> = template_map
            .into_iter()
            .map(|(template, (sum_prob, count, example))| ReferenceTemplate {
                template,
                probability: sum_prob / count as f64,
                example,
            })
            .collect();
        // Renormalize so probabilities sum to 1.0 within each source.
        let total: f64 = templates.iter().map(|t| t.probability).sum();
        if total > 0.0 {
            for t in &mut templates {
                t.probability /= total;
            }
        }
        // Sort by descending probability for determinism.
        templates.sort_by(|a, b| {
            b.probability
                .partial_cmp(&a.probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        by_source.insert(src, templates);
    }

    ReferenceFormatPrior { by_source }
}

// ---------------------------------------------------------------------------
// SP6 — Text-taxonomy aggregation
// ---------------------------------------------------------------------------

/// SP6 — Aggregate per-client `TextTaxonomyPrior` values into an industry prior.
/// Pools sharing a key are unioned; templates sharing a `template` string have
/// their probability mass pooled (weighted by pool `n`) and renormalised.
/// CoA pools are unioned by account number, last-writer-wins on conflict.
pub fn aggregate_text_taxonomy(inputs: &[&TextTaxonomyPrior]) -> TextTaxonomyPrior {
    if inputs.is_empty() {
        return TextTaxonomyPrior::default();
    }
    let line_pools = aggregate_pool_side(inputs.iter().map(|p| &p.line_pools));
    let header_pools = aggregate_pool_side(inputs.iter().map(|p| &p.header_pools));

    let mut coa_pools = BTreeMap::new();
    for p in inputs {
        for (acct, entry) in &p.coa_pools {
            coa_pools.insert(acct.clone(), entry.clone());
        }
    }

    // meta: take the first non-default, override n_client_inputs.
    let mut meta: TaxonomyMeta = inputs[0].meta.clone();
    meta.n_client_inputs = inputs.len();

    TextTaxonomyPrior {
        line_pools,
        header_pools,
        coa_pools,
        meta,
    }
}

/// Aggregate one pool map (line or header) across clients.
fn aggregate_pool_side<'a>(
    sides: impl Iterator<Item = &'a BTreeMap<String, TemplatePool>>,
) -> BTreeMap<String, TemplatePool> {
    // key -> (template -> weighted_prob_sum, synthetic_example), key -> total_n
    let mut acc: BTreeMap<String, BTreeMap<String, (f64, String)>> = BTreeMap::new();
    let mut totals: BTreeMap<String, usize> = BTreeMap::new();
    for side in sides {
        for (key, pool) in side {
            let entry = acc.entry(key.clone()).or_default();
            *totals.entry(key.clone()).or_insert(0) += pool.n;
            let weight = pool.n.max(1) as f64;
            for t in &pool.templates {
                let slot = entry
                    .entry(t.template.clone())
                    .or_insert((0.0, t.synthetic_example.clone()));
                slot.0 += t.probability * weight;
            }
        }
    }
    let mut result = BTreeMap::new();
    for (key, tmpl_map) in acc {
        let mass: f64 = tmpl_map.values().map(|(p, _)| *p).sum();
        if mass <= 0.0 {
            continue;
        }
        let mut templates: Vec<TemplateEntry> = tmpl_map
            .into_iter()
            .map(|(template, (p, synthetic_example))| TemplateEntry {
                template,
                probability: p / mass,
                synthetic_example,
            })
            .collect();
        templates.sort_by(|a, b| {
            b.probability
                .partial_cmp(&a.probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let n = *totals.get(&key).unwrap_or(&0);
        result.insert(key, TemplatePool { templates, n });
    }
    result
}

// ---------------------------------------------------------------------------
// SP4.5 — User-persona aggregation
// ---------------------------------------------------------------------------

/// SP4.5 — Aggregate per-client `UserPersonaPrior` values.
///
/// Users with the same ID across clients have their distributions weighted-
/// averaged by row count (`n` = 1 for each client since we only have the
/// probability vectors, not raw counts; so we treat each client equally).
/// `volume_share` is re-normalised after merging so it sums to 1.0 across
/// the merged user pool.
///
/// When all inputs are empty stubs (no user-column data), the aggregated result
/// is also empty — preserving the "no data available" signal.
// Per-user accumulator element: (sum_source_mix, sum_hourly_density, sum_weekday_density, count)
type UserAccEntry = (BTreeMap<String, f64>, [f64; 24], [f64; 7], usize);

pub fn aggregate_user_personas(inputs: &[&UserPersonaPrior]) -> UserPersonaPrior {
    if inputs.is_empty() {
        return UserPersonaPrior::default();
    }

    // Per-user accumulator: user_id → UserAccEntry
    let mut acc: BTreeMap<String, UserAccEntry> = BTreeMap::new();

    for &input in inputs {
        for (uid, beh) in &input.users {
            let entry = acc
                .entry(uid.clone())
                .or_insert_with(|| (BTreeMap::new(), [0.0; 24], [0.0; 7], 0));
            // Add source mix
            for (src, &p) in &beh.source_mix {
                *entry.0.entry(src.clone()).or_insert(0.0) += p;
            }
            // Add hourly density
            for (h, &p) in beh.hourly_density.iter().enumerate() {
                entry.1[h] += p;
            }
            // Add weekday density
            for (d, &p) in beh.weekday_density.iter().enumerate() {
                entry.2[d] += p;
            }
            entry.3 += 1;
        }
    }

    if acc.is_empty() {
        return UserPersonaPrior::default();
    }

    // Build merged users and raw volume shares (equal weight per client appearance).
    let n_clients = inputs.len() as f64;
    let total_users = acc.len() as f64;
    let mut users: BTreeMap<String, UserBehavior> = BTreeMap::new();

    for (uid, (src_sum, hour_sum, weekday_sum, count)) in acc {
        let n = count as f64;

        // Average source mix
        let total_src: f64 = src_sum.values().sum();
        let source_mix: BTreeMap<String, f64> = if total_src > 0.0 {
            src_sum.into_iter().map(|(s, v)| (s, v / n)).collect()
        } else {
            BTreeMap::new()
        };
        // Renormalise source_mix
        let src_total: f64 = source_mix.values().sum();
        let source_mix: BTreeMap<String, f64> = if src_total > 0.0 {
            source_mix
                .into_iter()
                .map(|(s, v)| (s, v / src_total))
                .collect()
        } else {
            source_mix
        };

        // Average hourly density
        let mut hourly_density = [0.0f64; 24];
        for (h, &v) in hour_sum.iter().enumerate() {
            hourly_density[h] = v / n;
        }
        // Renormalise
        let h_total: f64 = hourly_density.iter().sum();
        if h_total > 0.0 {
            for v in hourly_density.iter_mut() {
                *v /= h_total;
            }
        }

        // Average weekday density
        let mut weekday_density = [0.0f64; 7];
        for (d, &v) in weekday_sum.iter().enumerate() {
            weekday_density[d] = v / n;
        }
        // Renormalise
        let w_total: f64 = weekday_density.iter().sum();
        if w_total > 0.0 {
            for v in weekday_density.iter_mut() {
                *v /= w_total;
            }
        }

        // Volume share: proportion of clients that have this user, divided by
        // total distinct users → equal weight across the pool.
        let volume_share = (n / n_clients) / total_users.max(1.0);

        users.insert(
            uid,
            UserBehavior {
                source_mix,
                hourly_density,
                weekday_density,
                volume_share,
            },
        );
    }

    // Normalise volume_share so it sums to 1.0.
    let vs_total: f64 = users.values().map(|u| u.volume_share).sum();
    if vs_total > 0.0 {
        for u in users.values_mut() {
            u.volume_share /= vs_total;
        }
    }

    // Pool the user_count_distribution histograms.
    let count_hists: Vec<&LineCountHistogram> =
        inputs.iter().map(|p| &p.user_count_distribution).collect();
    let user_count_distribution = count_hists
        .iter()
        .skip(1)
        .fold(count_hists[0].clone(), |acc, h| {
            acc.pool(h).unwrap_or_else(|| acc.clone())
        });

    UserPersonaPrior {
        users,
        user_count_distribution,
    }
}

// ---------------------------------------------------------------------------
// aggregate_source_amount_conditionals (SP4.3)
// ---------------------------------------------------------------------------

/// SP4.3 — Aggregate per-(source, gl_prefix) log-normal amount parameters
/// across N client priors.
///
/// Weighted average by sample count (`n`): `mu_agg = Σ(n_i * mu_i) / Σ n_i`.
/// The aggregated sigma uses the same n-weighted average.  This preserves the
/// log-scale centroid across corpora of different sizes.
pub fn aggregate_source_amount_conditionals(
    inputs: &[&PerSourceAmountPrior],
) -> PerSourceAmountPrior {
    if inputs.is_empty() {
        return PerSourceAmountPrior::default();
    }

    // ---- Aggregate by_source_and_class ----
    // Accumulate (weighted-mu-sum, weighted-sigma-sum, total-n, all-values-for-median)
    // per (source, gl_prefix) key.
    // We compute a weighted arithmetic mean of mu and sigma, weighted by n.
    type WAccum = (f64, f64, usize, Vec<f64>); // (mu_sum*n, sigma_sum*n, n_total, medians)
    let mut pair_acc: BTreeMap<(String, String), WAccum> = BTreeMap::new();
    let mut src_acc: BTreeMap<String, WAccum> = BTreeMap::new();

    for &client in inputs {
        for (source, class_map) in &client.by_source_and_class {
            for (gl_prefix, params) in class_map {
                let key = (source.clone(), gl_prefix.clone());
                let entry = pair_acc.entry(key).or_insert((0.0, 0.0, 0, Vec::new()));
                entry.0 += params.mu * params.n as f64;
                entry.1 += params.sigma * params.n as f64;
                entry.2 += params.n;
                entry.3.push(params.median_abs);
            }
        }
        for (source, params) in &client.by_source {
            let entry = src_acc
                .entry(source.clone())
                .or_insert((0.0, 0.0, 0, Vec::new()));
            entry.0 += params.mu * params.n as f64;
            entry.1 += params.sigma * params.n as f64;
            entry.2 += params.n;
            entry.3.push(params.median_abs);
        }
    }

    let to_lognormal = |(mu_wsum, sigma_wsum, n_total, medians): WAccum| -> LognormalAmount {
        if n_total == 0 {
            return LognormalAmount::default();
        }
        let n_f = n_total as f64;
        let mu = mu_wsum / n_f;
        let sigma = (sigma_wsum / n_f).max(1e-6);
        // Median of medians as a robust central estimate.
        let mut meds = medians;
        meds.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median_abs = if meds.is_empty() {
            mu.exp()
        } else if meds.len().is_multiple_of(2) {
            (meds[meds.len() / 2 - 1] + meds[meds.len() / 2]) / 2.0
        } else {
            meds[meds.len() / 2]
        };
        LognormalAmount {
            mu,
            sigma,
            n: n_total,
            median_abs,
        }
    };

    let mut by_source_and_class: BTreeMap<String, BTreeMap<String, LognormalAmount>> =
        BTreeMap::new();
    for ((source, gl_prefix), accum) in pair_acc {
        by_source_and_class
            .entry(source)
            .or_default()
            .insert(gl_prefix, to_lognormal(accum));
    }

    let by_source: BTreeMap<String, LognormalAmount> = src_acc
        .into_iter()
        .map(|(src, accum)| (src, to_lognormal(accum)))
        .collect();

    PerSourceAmountPrior {
        by_source_and_class,
        by_source,
    }
}

// ---------------------------------------------------------------------------
// aggregate_source_role_gl (SP4.6)
// ---------------------------------------------------------------------------

/// SP4.6 — Aggregate per-(source, role) GL account distributions across N client
/// priors.  Counts are pooled cross-client (scaled by `n` before merging), then
/// renormalised into probabilities.
///
/// The SP3.9/SP3.11 namespace filter is inherited: this function is called only
/// on the `inputs_filtered` slice after the dominant-GL-format filter has already
/// been applied in `aggregate_industry_priors`.
pub fn aggregate_source_role_gl(inputs: &[&PerSourceRolePrior]) -> PerSourceRolePrior {
    if inputs.is_empty() {
        return PerSourceRolePrior::default();
    }

    // Outer: source. Middle: role. Inner: gl_account → pooled count.
    let mut pooled: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, usize>>,
    > = std::collections::BTreeMap::new();

    for input in inputs {
        for (source, role_map) in &input.by_source_and_role {
            let s_entry = pooled.entry(source.clone()).or_default();
            for (role, dist) in role_map {
                let r_entry = s_entry.entry(role.clone()).or_default();
                for (value, &prob) in &dist.probabilities {
                    let count = (prob * dist.n as f64).round() as usize;
                    *r_entry.entry(value.clone()).or_insert(0) += count;
                }
            }
        }
    }

    let by_source_and_role = pooled
        .into_iter()
        .filter_map(|(source, role_map)| {
            let roles: std::collections::BTreeMap<
                String,
                crate::models::behavioral::CategoricalDistribution,
            > = role_map
                .into_iter()
                .filter_map(|(role, value_counts)| {
                    let total: usize = value_counts.values().sum();
                    if total == 0 {
                        None
                    } else {
                        Some((role, CategoricalDistribution::from_counts(value_counts)))
                    }
                })
                .collect();
            if roles.is_empty() {
                None
            } else {
                Some((source, roles))
            }
        })
        .collect();

    PerSourceRolePrior { by_source_and_role }
}

// ---------------------------------------------------------------------------
// aggregate_tb_anchor (SP4.1)
// ---------------------------------------------------------------------------

/// SP4.1 — Aggregate per-client `TbAnchorPrior` values into an industry-level
/// TB anchor.
///
/// **Strategy per account:**
/// - `opening_balance` / `closing_balance`: median across clients that have the
///   account.  Median is more robust than mean against outlier client sizes.
/// - `period_net_activity`: computed as `closing_balance − opening_balance` on
///   the aggregated medians.
/// - `opening_stdev` / `closing_stdev`: sample standard deviation of the raw
///   per-client values (after normalisation by each client's total assets, scaled
///   back to an absolute magnitude for interpretability).
/// - `n_clients`: number of clients in which the account appeared.
///
/// **Balance-sheet totals**: mean of per-client totals.
///
/// Accounts that appear in only one client are included but carry `opening_stdev
/// = closing_stdev = 0.0` — the generator should treat single-client accounts as
/// soft targets.
pub fn aggregate_tb_anchor(inputs: &[&TbAnchorPrior]) -> TbAnchorPrior {
    if inputs.is_empty() {
        return TbAnchorPrior::default();
    }

    // Collect per-account values across clients.
    // account → (opening_values, closing_values)
    let mut acc: BTreeMap<String, (Vec<f64>, Vec<f64>)> = BTreeMap::new();

    for &client in inputs {
        for (account, target) in &client.per_account {
            let entry = acc.entry(account.clone()).or_default();
            entry.0.push(target.opening_balance);
            entry.1.push(target.closing_balance);
        }
    }

    let mut per_account: BTreeMap<String, TbTarget> = BTreeMap::new();

    for (account, (openings, closings)) in &acc {
        let n_clients = openings.len();

        let opening_balance = median(openings);
        let closing_balance = median(closings);
        let period_net_activity = closing_balance - opening_balance;

        let opening_stdev = if n_clients > 1 {
            sample_stdev(openings)
        } else {
            0.0
        };
        let closing_stdev = if n_clients > 1 {
            sample_stdev(closings)
        } else {
            0.0
        };

        per_account.insert(
            account.clone(),
            TbTarget {
                opening_balance,
                closing_balance,
                period_net_activity,
                opening_stdev,
                closing_stdev,
                n_clients,
            },
        );
    }

    let n = inputs.len() as f64;
    let total_assets = inputs.iter().map(|c| c.total_assets).sum::<f64>() / n;
    let total_liabilities = inputs.iter().map(|c| c.total_liabilities).sum::<f64>() / n;
    let total_equity = inputs.iter().map(|c| c.total_equity).sum::<f64>() / n;

    TbAnchorPrior {
        per_account,
        total_assets,
        total_liabilities,
        total_equity,
        n_clients: inputs.len(),
    }
}

/// Compute the median of a non-empty slice.  Panics if the slice is empty.
fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    if n.is_multiple_of(2) {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    } else {
        sorted[n / 2]
    }
}

/// Sample standard deviation (ddof=1).  Returns 0.0 for slices with ≤1 element.
fn sample_stdev(values: &[f64]) -> f64 {
    let n = values.len();
    if n <= 1 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / n as f64;
    let var = values.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
    var.sqrt()
}

// ---------------------------------------------------------------------------
// Main industry-level aggregation
// ---------------------------------------------------------------------------

/// Aggregate N per-client behavioral priors into a single industry-level prior.
pub fn aggregate_industry_priors(
    inputs: &[&BehavioralPriors],
    industry: &str,
) -> AggregationResult<BehavioralPriors> {
    if inputs.is_empty() {
        return Err(FingerprintError::ExtractionError {
            extractor: "industry_aggregator".to_string(),
            message: "aggregate_industry_priors: no inputs".to_string(),
        });
    }

    // SP3.11 W1 — detect the cross-client dominant GL namespace and filter
    // clients whose namespace doesn't match. This prevents fragmented
    // Source-Source adjacency graphs caused by incompatible GL formats
    // (e.g. Swiss `0000xxxxxx` vs French `40.xxxxx`) from different clients.
    // When no dominant format can be detected, all clients are retained.
    let psa_inputs_all: Vec<&PerSourceAttributePrior> = inputs
        .iter()
        .filter_map(|bp| bp.per_source_attribute.as_ref())
        .collect();

    let dominant_gl_format = detect_cross_client_dominant_gl_format(&psa_inputs_all);

    let client_in_dominant: Vec<bool> = inputs
        .iter()
        .map(|bp| match (&bp.per_source_attribute, dominant_gl_format) {
            (Some(psa), Some(target)) => is_client_in_dominant_namespace(psa, target),
            // If a client lacks per_source_attribute or no dominant exists, keep them.
            _ => true,
        })
        .collect();

    let inputs_filtered: Vec<&BehavioralPriors> = inputs
        .iter()
        .zip(client_in_dominant.iter())
        .filter_map(|(bp, &keep)| if keep { Some(*bp) } else { None })
        .collect();

    let n_dropped = inputs.len() - inputs_filtered.len();
    if n_dropped > 0 {
        tracing::info!(
            target: "datasynth_fingerprint::aggregation",
            "SP3.11 W1 — dropped {n_dropped} of {} client priors due to GL namespace mismatch \
             (dominant format: {:?})",
            inputs.len(),
            dominant_gl_format
        );
    }

    let n_rows_aggregated: usize = inputs_filtered.iter().map(|bp| bp.n_rows_aggregated).sum();
    let source_mixes: Vec<&_> = inputs_filtered.iter().map(|bp| &bp.source_mix).collect();
    let lpj: Vec<&_> = inputs_filtered.iter().map(|bp| &bp.lines_per_je).collect();
    let iets: Vec<&_> = inputs_filtered
        .iter()
        .map(|bp| &bp.per_source_iet)
        .collect();
    let lifetimes: Vec<&_> = inputs_filtered
        .iter()
        .map(|bp| &bp.active_lifetime)
        .collect();
    let fanouts: Vec<&_> = inputs_filtered.iter().map(|bp| &bp.fanout).collect();
    let lags: Vec<&_> = inputs_filtered
        .iter()
        .filter_map(|bp| bp.posting_lag.as_ref())
        .collect();

    let posting_lag = if lags.is_empty() {
        None
    } else {
        aggregate_posting_lag(&lags)
    };

    let active_segments_inputs: Vec<&_> = inputs_filtered
        .iter()
        .filter_map(|bp| bp.active_segments.as_ref())
        .collect();
    let active_segments = if active_segments_inputs.is_empty() {
        None
    } else {
        Some(aggregate_active_segments(&active_segments_inputs))
    };

    let entity_clusters_inputs: Vec<&_> = inputs_filtered
        .iter()
        .filter_map(|bp| bp.entity_clusters.as_ref())
        .collect();
    let entity_clusters = if entity_clusters_inputs.is_empty() {
        None
    } else {
        Some(aggregate_entity_clusters(&entity_clusters_inputs))
    };

    Ok(BehavioralPriors {
        schema_version: BehavioralPriors::SCHEMA_VERSION,
        generator_version: env!("CARGO_PKG_VERSION").to_string(),
        industry: industry.to_string(),
        n_client_inputs: inputs_filtered.len(),
        n_rows_aggregated,
        source_mix: aggregate_source_mix(&source_mixes),
        per_source_iet: aggregate_per_source_iet(&iets),
        lines_per_je: aggregate_lines_per_je(&lpj),
        active_lifetime: aggregate_active_lifetime(&lifetimes),
        fanout: aggregate_fanout(&fanouts),
        posting_lag,
        active_segments,
        entity_clusters,
        per_source_attribute: {
            let psa_filtered: Vec<&_> = inputs_filtered
                .iter()
                .filter_map(|bp| bp.per_source_attribute.as_ref())
                .collect();
            if psa_filtered.is_empty() {
                None
            } else {
                Some(aggregate_per_source_attribute(&psa_filtered))
            }
        },
        tp_entity_clusters: {
            let tp_ec_inputs: Vec<&_> = inputs_filtered
                .iter()
                .filter_map(|bp| bp.tp_entity_clusters.as_ref())
                .collect();
            if tp_ec_inputs.is_empty() {
                None
            } else {
                Some(aggregate_entity_clusters(&tp_ec_inputs))
            }
        },
        coa_semantic: {
            let coa_inputs: Vec<&_> = inputs_filtered
                .iter()
                .filter_map(|bp| bp.coa_semantic.as_ref())
                .collect();
            if coa_inputs.is_empty() {
                None
            } else {
                Some(aggregate_coa_semantic(&coa_inputs))
            }
        },
        reference_formats: {
            let rf_inputs: Vec<&_> = inputs_filtered
                .iter()
                .filter_map(|bp| bp.reference_formats.as_ref())
                .collect();
            if rf_inputs.is_empty() {
                None
            } else {
                Some(aggregate_reference_formats(&rf_inputs))
            }
        },
        // SP6 — aggregate per-client text_taxonomy priors into the industry bundle.
        text_taxonomy: {
            let tx_inputs: Vec<&_> = inputs_filtered
                .iter()
                .filter_map(|bp| bp.text_taxonomy.as_ref())
                .collect();
            if tx_inputs.is_empty() {
                None
            } else {
                Some(aggregate_text_taxonomy(&tx_inputs))
            }
        },
        // SP4.5 — user_personas: aggregate stub priors from all clients.
        // When all inputs are empty stubs (no user-column data in the corpus),
        // the aggregated result is also an empty stub (None).
        user_personas: {
            let up_inputs: Vec<&_> = inputs_filtered
                .iter()
                .filter_map(|bp| bp.user_personas.as_ref())
                .collect();
            if up_inputs.is_empty() {
                None
            } else {
                let agg = aggregate_user_personas(&up_inputs);
                // Suppress Some(empty-stub) — treat it the same as None.
                if agg.has_data() {
                    Some(agg)
                } else {
                    None
                }
            }
        },
        // SP4.3 — per-(source, gl_prefix) amount conditionals.
        source_amount_conditionals: {
            let sac_inputs: Vec<&_> = inputs_filtered
                .iter()
                .filter_map(|bp| bp.source_amount_conditionals.as_ref())
                .collect();
            if sac_inputs.is_empty() {
                None
            } else {
                let agg = aggregate_source_amount_conditionals(&sac_inputs);
                if agg.by_source.is_empty() && agg.by_source_and_class.is_empty() {
                    None
                } else {
                    Some(agg)
                }
            }
        },
        // SP4.6 — per-(source, line_role) GL account conditionals.
        source_role_gl_conditionals: {
            let srg_inputs: Vec<&_> = inputs_filtered
                .iter()
                .filter_map(|bp| bp.source_role_gl_conditionals.as_ref())
                .collect();
            if srg_inputs.is_empty() {
                None
            } else {
                let agg = aggregate_source_role_gl(&srg_inputs);
                if agg.by_source_and_role.is_empty() {
                    None
                } else {
                    Some(agg)
                }
            }
        },
        // SP4.1 — TB anchor prior.  Aggregate per-client TB targets into an
        // industry-level anchor.  Old bundles and clients without TB data produce None.
        tb_anchor: {
            let tb_inputs: Vec<&_> = inputs_filtered
                .iter()
                .filter_map(|bp| bp.tb_anchor.as_ref())
                .filter(|tb| tb.has_data())
                .collect();
            if tb_inputs.is_empty() {
                None
            } else {
                let agg = aggregate_tb_anchor(&tb_inputs);
                if agg.has_data() {
                    Some(agg)
                } else {
                    None
                }
            }
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::behavioral::{LineCountHistogram, SourceSegmentSummary, LINE_COUNT_BUCKETS};

    fn smix(probs: &[(&str, f64)], other: f64, thresh: f64) -> SourceMixPrior {
        SourceMixPrior {
            probabilities: probs.iter().map(|(s, p)| (s.to_string(), *p)).collect(),
            other_fraction: other,
            min_threshold: thresh,
        }
    }

    fn hist(values: &[u32]) -> LineCountHistogram {
        LineCountHistogram::build(values, LINE_COUNT_BUCKETS).0
    }

    #[test]
    fn aggregate_source_mix_equal_clients() {
        let a = smix(&[("KR", 0.5), ("RE", 0.5)], 0.0, 0.005);
        let b = smix(&[("KR", 0.3), ("KZ", 0.7)], 0.0, 0.005);
        let agg = aggregate_source_mix(&[&a, &b]);
        assert!((agg.probabilities["KR"] - 0.4).abs() < 1e-9);
        assert!((agg.probabilities["RE"] - 0.25).abs() < 1e-9);
        assert!((agg.probabilities["KZ"] - 0.35).abs() < 1e-9);
        let total: f64 = agg.probabilities.values().sum::<f64>() + agg.other_fraction;
        assert!((total - 1.0).abs() < 1e-9);
    }

    fn iet_summary(samples: &[f64], autocorr: f64, n: usize) -> IetSummary {
        IetSummary {
            n,
            empirical_cdf_days: EmpiricalCdf::from_sorted_values(
                "iet".to_string(),
                samples.to_vec(),
            ),
            lognormal_fit: None,
            lag1_autocorr: autocorr,
        }
    }

    #[test]
    fn aggregate_per_source_iet_pools_knots_and_weights_autocorr() {
        let mut a_by = BTreeMap::new();
        a_by.insert("KR".to_string(), iet_summary(&[1.0, 2.0, 3.0], 0.5, 3));
        let a = PerSourceIetPrior { by_source: a_by };
        let mut b_by = BTreeMap::new();
        b_by.insert("KR".to_string(), iet_summary(&[4.0, 5.0, 6.0, 7.0], 0.3, 4));
        let b = PerSourceIetPrior { by_source: b_by };
        let agg = aggregate_per_source_iet(&[&a, &b]);
        let summ = &agg.by_source["KR"];
        assert_eq!(summ.empirical_cdf_days.values.len(), 7);
        // Weighted autocorr: (0.5 * 3 + 0.3 * 4) / 7 = 2.7/7 ≈ 0.3857142857
        assert!((summ.lag1_autocorr - 0.3857142857).abs() < 1e-6);
    }

    #[test]
    fn aggregate_lines_per_je_pools_overall() {
        let a = LinesPerJePrior {
            overall: hist(&[2, 2, 3]),
            by_source: BTreeMap::new(),
            min_jes_per_source: 500,
        };
        let b = LinesPerJePrior {
            overall: hist(&[4, 5, 8]),
            by_source: BTreeMap::new(),
            min_jes_per_source: 500,
        };
        let agg = aggregate_lines_per_je(&[&a, &b]);
        assert_eq!(agg.overall.n, 6);
    }

    #[test]
    fn aggregate_active_lifetime_pools_overall() {
        let a = ActiveLifetimePrior {
            by_source: BTreeMap::new(),
            overall: hist(&[10, 20]),
        };
        let b = ActiveLifetimePrior {
            by_source: BTreeMap::new(),
            overall: hist(&[30, 40, 50]),
        };
        let agg = aggregate_active_lifetime(&[&a, &b]);
        assert_eq!(agg.overall.n, 5);
    }

    #[test]
    fn aggregate_fanout_merges_attributes() {
        let mut m1: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
        m1.insert("GLAccount".into(), hist(&[1, 2, 3]));
        let mut m2: BTreeMap<String, LineCountHistogram> = BTreeMap::new();
        m2.insert("GLAccount".into(), hist(&[4, 5]));
        let a = FanoutPrior { by_attribute: m1 };
        let b = FanoutPrior { by_attribute: m2 };
        let agg = aggregate_fanout(&[&a, &b]);
        assert_eq!(agg.by_attribute["GLAccount"].n, 5);
    }

    #[test]
    fn aggregate_posting_lag_pools_means() {
        let cdf = |xs: &[f64]| EmpiricalCdf::from_sorted_values("lag".to_string(), xs.to_vec());
        let lag = |xs: &[f64]| LagSummary {
            empirical_cdf_days: cdf(xs),
            mean: xs.iter().sum::<f64>() / xs.len() as f64,
            stddev: 0.0,
            n: xs.len(),
        };
        let mut m1 = BTreeMap::new();
        m1.insert("KR".into(), lag(&[1.0, 2.0]));
        let mut m2 = BTreeMap::new();
        m2.insert("KR".into(), lag(&[3.0, 4.0, 5.0]));
        let a = PostingLagPrior { by_source: m1 };
        let b = PostingLagPrior { by_source: m2 };
        let agg = aggregate_posting_lag(&[&a, &b]).unwrap();
        assert!((agg.by_source["KR"].mean - 3.0).abs() < 1e-9);
        assert_eq!(agg.by_source["KR"].n, 5);
    }

    #[test]
    fn aggregate_active_segments_pools_histograms() {
        use crate::models::behavioral::SEGMENT_COUNT_BUCKETS;
        let count_hist_a = LineCountHistogram::build(&[2], SEGMENT_COUNT_BUCKETS).0;
        let count_hist_b = LineCountHistogram::build(&[3], SEGMENT_COUNT_BUCKETS).0;
        let mut a_by = BTreeMap::new();
        a_by.insert(
            "KR".to_string(),
            SourceSegmentSummary {
                segment_count_histogram: count_hist_a,
                segment_length_histogram: LineCountHistogram::default(),
                gap_length_histogram: LineCountHistogram::default(),
            },
        );
        let a = ActiveSegmentsPrior { by_source: a_by };
        let mut b_by = BTreeMap::new();
        b_by.insert(
            "KR".to_string(),
            SourceSegmentSummary {
                segment_count_histogram: count_hist_b,
                segment_length_histogram: LineCountHistogram::default(),
                gap_length_histogram: LineCountHistogram::default(),
            },
        );
        let b = ActiveSegmentsPrior { by_source: b_by };
        let agg = aggregate_active_segments(&[&a, &b]);
        let summ = &agg.by_source["KR"];
        // Pooled count: both observations present.
        assert_eq!(summ.segment_count_histogram.n, 2);
    }

    #[test]
    fn aggregate_entity_clusters_merges_overlapping() {
        let a = EntityClustersPrior {
            clusters: vec![EntityCluster {
                members: vec!["A".into(), "B".into()],
                avg_jaccard: 0.4,
            }],
            clustering_rate: 0.5,
        };
        let b = EntityClustersPrior {
            clusters: vec![EntityCluster {
                members: vec!["B".into(), "C".into()],
                avg_jaccard: 0.6,
            }],
            clustering_rate: 0.7,
        };
        let agg = aggregate_entity_clusters(&[&a, &b]);
        // Overlap on "B" should fuse the two clusters into {A, B, C}.
        assert_eq!(agg.clusters.len(), 1);
        let members: std::collections::HashSet<_> = agg.clusters[0].members.iter().collect();
        assert!(members.contains(&"A".to_string()));
        assert!(members.contains(&"B".to_string()));
        assert!(members.contains(&"C".to_string()));
        // Avg jaccard ≈ (0.4 + 0.6) / 2 = 0.5
        assert!((agg.clusters[0].avg_jaccard - 0.5).abs() < 1e-9);
        // Clustering rate avg = (0.5 + 0.7) / 2 = 0.6
        assert!((agg.clustering_rate - 0.6).abs() < 1e-9);
    }

    #[test]
    fn aggregate_entity_clusters_keeps_disjoint() {
        let a = EntityClustersPrior {
            clusters: vec![EntityCluster {
                members: vec!["A".into(), "B".into()],
                avg_jaccard: 0.4,
            }],
            clustering_rate: 0.4,
        };
        let b = EntityClustersPrior {
            clusters: vec![EntityCluster {
                members: vec!["X".into(), "Y".into()],
                avg_jaccard: 0.5,
            }],
            clustering_rate: 0.5,
        };
        let agg = aggregate_entity_clusters(&[&a, &b]);
        assert_eq!(
            agg.clusters.len(),
            2,
            "disjoint clusters should stay separate"
        );
    }

    fn single_source_prior(
        source: &str,
        attribute: &str,
        values: &[(&str, usize)],
    ) -> PerSourceAttributePrior {
        let mut by_source = BTreeMap::new();
        let mut attrs = BTreeMap::new();
        let counts: BTreeMap<String, usize> =
            values.iter().map(|(v, c)| (v.to_string(), *c)).collect();
        attrs.insert(
            attribute.to_string(),
            CategoricalDistribution::from_counts(counts),
        );
        by_source.insert(source.to_string(), attrs);
        PerSourceAttributePrior {
            by_source,
            min_observations: 10,
        }
    }

    #[test]
    fn aggregate_per_source_attribute_pools_counts_across_clients() {
        // Client A: 100 KR obs, all posting to "200001"
        let client_a = {
            let mut by_source = BTreeMap::new();
            let mut attrs = BTreeMap::new();
            let mut counts = BTreeMap::new();
            counts.insert("200001".to_string(), 100);
            attrs.insert(
                "gl_account".to_string(),
                CategoricalDistribution::from_counts(counts),
            );
            by_source.insert("KR".to_string(), attrs);
            PerSourceAttributePrior {
                by_source,
                min_observations: 10,
            }
        };
        // Client B: 50 KR obs, 30 to "200001" and 20 to "200002"
        let client_b = {
            let mut by_source = BTreeMap::new();
            let mut attrs = BTreeMap::new();
            let mut counts = BTreeMap::new();
            counts.insert("200001".to_string(), 30);
            counts.insert("200002".to_string(), 20);
            attrs.insert(
                "gl_account".to_string(),
                CategoricalDistribution::from_counts(counts),
            );
            by_source.insert("KR".to_string(), attrs);
            PerSourceAttributePrior {
                by_source,
                min_observations: 10,
            }
        };

        let merged = aggregate_per_source_attribute(&[&client_a, &client_b]);
        let kr_gl = merged
            .conditional("KR", "gl_account")
            .expect("KR gl_account");
        assert_eq!(kr_gl.n, 150);
        // 130 / 150 ≈ 0.8667
        assert!((kr_gl.probabilities["200001"] - 0.8666666_f64).abs() < 1e-4);
        assert!((kr_gl.probabilities["200002"] - 0.1333333_f64).abs() < 1e-4);
    }

    #[test]
    fn aggregate_per_source_attribute_handles_disjoint_sources() {
        // Client A has only KR; client B has only RV.
        let client_a = single_source_prior("KR", "gl_account", &[("200001", 50)]);
        let client_b = single_source_prior("RV", "gl_account", &[("400001", 80)]);
        let merged = aggregate_per_source_attribute(&[&client_a, &client_b]);
        assert!(merged.conditional("KR", "gl_account").is_some());
        assert!(merged.conditional("RV", "gl_account").is_some());
    }

    #[test]
    fn aggregate_industry_priors_smoke() {
        let bp = |industry: &str, n_rows: usize| BehavioralPriors {
            schema_version: BehavioralPriors::SCHEMA_VERSION,
            generator_version: "test".into(),
            industry: industry.into(),
            n_client_inputs: 1,
            n_rows_aggregated: n_rows,
            source_mix: smix(&[("KR", 0.5)], 0.5, 0.005),
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
            tb_anchor: None,
        };
        let a = bp("health", 1000);
        let b = bp("health", 2000);
        let agg = aggregate_industry_priors(&[&a, &b], "health").expect("ok");
        assert_eq!(agg.industry, "health");
        assert_eq!(agg.n_client_inputs, 2);
        assert_eq!(agg.n_rows_aggregated, 3000);
        assert!(agg.posting_lag.is_none());
        assert!(agg.per_source_attribute.is_none());
    }

    #[test]
    fn aggregate_industry_priors_propagates_per_source_attribute() {
        // When at least one input carries per_source_attribute, the
        // aggregated result must be Some(…).
        let psa = Some(single_source_prior("KR", "gl_account", &[("200001", 40)]));
        let bp = |psa_val: Option<PerSourceAttributePrior>| BehavioralPriors {
            schema_version: BehavioralPriors::SCHEMA_VERSION,
            generator_version: "test".into(),
            industry: "health".into(),
            n_client_inputs: 1,
            n_rows_aggregated: 500,
            source_mix: smix(&[("KR", 1.0)], 0.0, 0.005),
            per_source_iet: PerSourceIetPrior::default(),
            lines_per_je: LinesPerJePrior::default(),
            active_lifetime: ActiveLifetimePrior::default(),
            fanout: FanoutPrior::default(),
            posting_lag: None,
            active_segments: None,
            entity_clusters: None,
            per_source_attribute: psa_val,
            tp_entity_clusters: None,
            coa_semantic: None,
            reference_formats: None,
            text_taxonomy: None,
            user_personas: None,
            source_amount_conditionals: None,
            source_role_gl_conditionals: None,
            tb_anchor: None,
        };
        let a = bp(psa.clone());
        let b = bp(None); // second client has no attribute priors
        let agg = aggregate_industry_priors(&[&a, &b], "health").expect("ok");
        // non-None because client A contributed
        let psa_agg = agg
            .per_source_attribute
            .expect("per_source_attribute should be Some when any input has it");
        assert!(
            psa_agg.conditional("KR", "gl_account").is_some(),
            "KR gl_account should survive aggregation"
        );
    }

    // SP3.9 W2 tests --------------------------------------------------------

    #[test]
    fn classify_account_format_examples() {
        assert_eq!(
            classify_account_format("0000200001"),
            AccountFormat::ZeroPadded10
        );
        assert_eq!(classify_account_format("40.20001"), AccountFormat::Dotted);
        assert_eq!(
            classify_account_format("11000"),
            AccountFormat::ShortNumeric
        );
        assert_eq!(
            classify_account_format("GLAccount-0123"),
            AccountFormat::SyntheticDefault
        );
        assert_eq!(classify_account_format(""), AccountFormat::Empty);
        assert_eq!(
            classify_account_format("SomeAccountName"),
            AccountFormat::Other
        );
    }

    #[test]
    fn aggregate_per_source_attribute_strips_cross_format_clash() {
        // Client A: KR posts to 0000200001 (10-digit zero-padded — ZeroPadded10).
        let client_a = single_source_prior("KR", "gl_account", &[("0000200001", 1000)]);
        // Client B: KR posts to 40.20001 (dotted — Dotted).  Different namespace.
        let client_b = single_source_prior("KR", "gl_account", &[("40.20001", 500)]);

        let merged = aggregate_per_source_attribute(&[&client_a, &client_b]);
        let kr_gl = merged
            .conditional("KR", "gl_account")
            .expect("KR gl_account");

        // Client A's dominant format is ZeroPadded10: its own value passes.
        // Client B's dominant format is Dotted: its own value passes.
        // But when viewed together in the merged distribution, each client
        // contributed only values matching *its own* dominant format —
        // so both values should appear (from their respective clients).
        assert!(
            kr_gl.probabilities.contains_key("0000200001"),
            "ZeroPadded10 value from client A must survive"
        );
        // 40.20001 passes client B's own-format filter, so it also appears.
        assert!(
            kr_gl.probabilities.contains_key("40.20001"),
            "Dotted value from client B must survive its own-format filter"
        );

        // The key invariant: neither value is *stripped by the wrong client's
        // filter*.  Client A must not have contributed any Dotted values, and
        // client B must not have contributed any ZeroPadded10 values.
        // This is verified by total count:
        // client A contributed 1000 counts (ZeroPadded10), client B contributed 500 (Dotted).
        assert_eq!(
            kr_gl.n, 1500,
            "total count must be 1000 + 500 (each client's values pass its own filter)"
        );
    }

    #[test]
    fn aggregate_per_source_attribute_strips_foreign_format_from_mixed_client() {
        // A single client that has KR posting to both ZeroPadded10 AND Dotted.
        // This simulates a real-world data-quality issue where one client's
        // extract accidentally contains values from another client's namespace.
        // The dominant format for this client is ZeroPadded10 (higher count).
        // The Dotted values should be stripped.
        let mut by_source = BTreeMap::new();
        let mut attrs = BTreeMap::new();
        let mut counts = BTreeMap::new();
        counts.insert("0000200001".to_string(), 800); // ZeroPadded10 — dominant
        counts.insert("0000200002".to_string(), 400); // ZeroPadded10
        counts.insert("40.20001".to_string(), 50); // Dotted — minority, should be stripped
        attrs.insert(
            "gl_account".to_string(),
            CategoricalDistribution::from_counts(counts),
        );
        by_source.insert("KR".to_string(), attrs);
        let mixed_client = PerSourceAttributePrior {
            by_source,
            min_observations: 10,
        };

        let merged = aggregate_per_source_attribute(&[&mixed_client]);
        let kr_gl = merged
            .conditional("KR", "gl_account")
            .expect("KR gl_account");

        assert!(
            kr_gl.probabilities.contains_key("0000200001"),
            "dominant-format value must be retained"
        );
        assert!(
            kr_gl.probabilities.contains_key("0000200002"),
            "second dominant-format value must be retained"
        );
        assert!(
            !kr_gl.probabilities.contains_key("40.20001"),
            "foreign-format (Dotted) value must be stripped from ZeroPadded10-dominant client"
        );
        assert_eq!(
            kr_gl.n, 1200,
            "only the 800+400 ZeroPadded10 counts survive"
        );
    }

    // SP3.11 W1 tests -------------------------------------------------------

    /// Build a minimal `BehavioralPriors` with the given `PerSourceAttributePrior`.
    fn bp_with_psa(
        industry: &str,
        n_rows: usize,
        psa: PerSourceAttributePrior,
    ) -> BehavioralPriors {
        BehavioralPriors {
            schema_version: BehavioralPriors::SCHEMA_VERSION,
            generator_version: "test".into(),
            industry: industry.into(),
            n_client_inputs: 1,
            n_rows_aggregated: n_rows,
            source_mix: smix(&[("KR", 1.0)], 0.0, 0.005),
            per_source_iet: PerSourceIetPrior::default(),
            lines_per_je: LinesPerJePrior::default(),
            active_lifetime: ActiveLifetimePrior::default(),
            fanout: FanoutPrior::default(),
            posting_lag: None,
            active_segments: None,
            entity_clusters: None,
            per_source_attribute: Some(psa),
            tp_entity_clusters: None,
            coa_semantic: None,
            reference_formats: None,
            text_taxonomy: None,
            user_personas: None,
            source_amount_conditionals: None,
            source_role_gl_conditionals: None,
            tb_anchor: None,
        }
    }

    #[test]
    fn aggregate_industry_priors_drops_minority_namespace_clients() {
        // 3 clients in 0000xxxxxx format (ZeroPadded10, dominant)
        // 1 client in 40.xxxxx format (Dotted, minority — should be dropped)
        let majority_a = bp_with_psa(
            "health",
            1000,
            single_source_prior("KR", "gl_account", &[("0000200001", 1000)]),
        );
        let majority_b = bp_with_psa(
            "health",
            800,
            single_source_prior("SA", "gl_account", &[("0000300001", 800)]),
        );
        let majority_c = bp_with_psa(
            "health",
            600,
            single_source_prior("DR", "gl_account", &[("0000400001", 600)]),
        );
        let minority = bp_with_psa(
            "health",
            500,
            single_source_prior("Debitor", "gl_account", &[("40.20001", 500)]),
        );

        let aggregated = aggregate_industry_priors(
            &[&majority_a, &majority_b, &majority_c, &minority],
            "health",
        )
        .expect("aggregate");

        let psa = aggregated.per_source_attribute.expect("psa");
        // KR / SA / DR should be present (from majority clients)
        assert!(
            psa.conditional("KR", "gl_account").is_some(),
            "KR from majority client A must be present"
        );
        assert!(
            psa.conditional("SA", "gl_account").is_some(),
            "SA from majority client B must be present"
        );
        assert!(
            psa.conditional("DR", "gl_account").is_some(),
            "DR from majority client C must be present"
        );
        // Debitor should be ABSENT — client D (minority Dotted namespace) was filtered out
        assert!(
            psa.conditional("Debitor", "gl_account").is_none(),
            "Debitor from minority client D must be absent after namespace filtering"
        );

        // n_client_inputs should reflect only the 3 dominant-namespace clients
        assert_eq!(
            aggregated.n_client_inputs, 3,
            "n_client_inputs must reflect filtered client count"
        );
        // n_rows_aggregated should not include the dropped client's rows
        assert_eq!(
            aggregated.n_rows_aggregated,
            1000 + 800 + 600,
            "n_rows_aggregated must exclude dropped client's rows"
        );
    }

    #[test]
    fn aggregate_industry_priors_keeps_all_when_no_dominant_detectable() {
        // When no client has per_source_attribute, dominant_gl_format is None
        // and all clients must be retained (default-keep semantics).
        let bp = |n: usize| BehavioralPriors {
            schema_version: BehavioralPriors::SCHEMA_VERSION,
            generator_version: "test".into(),
            industry: "health".into(),
            n_client_inputs: 1,
            n_rows_aggregated: n,
            source_mix: smix(&[("KR", 1.0)], 0.0, 0.005),
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
            tb_anchor: None,
        };
        let a = bp(100);
        let b = bp(200);
        let agg = aggregate_industry_priors(&[&a, &b], "health").expect("ok");
        assert_eq!(
            agg.n_client_inputs, 2,
            "both clients kept when no dominant detectable"
        );
        assert_eq!(agg.n_rows_aggregated, 300);
    }

    // ---- SP4.3 aggregator tests -------------------------------------------

    fn make_sac(
        source: &str,
        gl_prefix: &str,
        mu: f64,
        sigma: f64,
        n: usize,
    ) -> PerSourceAmountPrior {
        use datasynth_core::distributions::behavioral_priors::{
            LognormalAmount, PerSourceAmountPrior,
        };
        use std::collections::BTreeMap;
        let mut inner = BTreeMap::new();
        inner.insert(
            gl_prefix.to_string(),
            LognormalAmount {
                mu,
                sigma,
                n,
                median_abs: mu.exp(),
            },
        );
        let mut by_source_and_class = BTreeMap::new();
        by_source_and_class.insert(source.to_string(), inner);
        let mut by_source = BTreeMap::new();
        by_source.insert(
            source.to_string(),
            LognormalAmount {
                mu,
                sigma,
                n,
                median_abs: mu.exp(),
            },
        );
        PerSourceAmountPrior {
            by_source_and_class,
            by_source,
        }
    }

    /// Aggregated mu is the n-weighted average of input mus.
    #[test]
    fn aggregate_source_amount_conditionals_weighted_mu() {
        let a = make_sac("KR", "0041", 4.0, 1.0, 100);
        let b = make_sac("KR", "0041", 6.0, 1.0, 300);
        let agg = aggregate_source_amount_conditionals(&[&a, &b]);
        // Weighted average: (4.0*100 + 6.0*300) / 400 = 5.5
        let params = agg
            .by_source_and_class
            .get("KR")
            .and_then(|m| m.get("0041"))
            .expect("KR/0041 should be present");
        assert!(
            (params.mu - 5.5).abs() < 1e-9,
            "aggregated mu should be 5.5, got {:.4}",
            params.mu
        );
        assert_eq!(params.n, 400);
    }

    /// Aggregator handles disjoint (source, gl_prefix) pairs from different clients.
    #[test]
    fn aggregate_source_amount_conditionals_disjoint_pairs() {
        let a = make_sac("KR", "0041", 5.4, 1.6, 200);
        let b = make_sac("RV", "0013", 6.6, 1.7, 150);
        let agg = aggregate_source_amount_conditionals(&[&a, &b]);
        assert!(
            agg.by_source_and_class.contains_key("KR"),
            "KR should be present"
        );
        assert!(
            agg.by_source_and_class.contains_key("RV"),
            "RV should be present"
        );
        assert!(agg.by_source.contains_key("KR"), "KR marginal");
        assert!(agg.by_source.contains_key("RV"), "RV marginal");
    }

    /// Empty input returns empty PerSourceAmountPrior.
    #[test]
    fn aggregate_source_amount_conditionals_empty_input() {
        let agg = aggregate_source_amount_conditionals(&[]);
        assert!(agg.by_source.is_empty());
        assert!(agg.by_source_and_class.is_empty());
    }

    // -----------------------------------------------------------------------
    // SP6 — aggregate_text_taxonomy tests
    // -----------------------------------------------------------------------

    use datasynth_core::distributions::text_taxonomy::{
        TemplateEntry, TemplatePool, TextTaxonomyPrior,
    };

    fn taxonomy_with_line(key: &str, template: &str, prob: f64, n: usize) -> TextTaxonomyPrior {
        let mut p = TextTaxonomyPrior::default();
        p.line_pools.insert(
            key.to_string(),
            TemplatePool {
                templates: vec![TemplateEntry {
                    template: template.to_string(),
                    probability: prob,
                    synthetic_example: format!("ex-{template}"),
                }],
                n,
            },
        );
        p
    }

    #[test]
    fn aggregate_text_taxonomy_unions_pools_and_renormalises() {
        let a = taxonomy_with_line("KR|A.B", "Rechnung", 1.0, 30);
        let b = taxonomy_with_line("KR|A.B", "Gutschrift", 1.0, 10);
        let agg = aggregate_text_taxonomy(&[&a, &b]);
        let pool = &agg.line_pools["KR|A.B"];
        assert_eq!(pool.templates.len(), 2);
        let sum: f64 = pool.templates.iter().map(|t| t.probability).sum();
        assert!(
            (sum - 1.0).abs() < 1e-9,
            "probabilities must renormalise to 1.0"
        );
        assert_eq!(pool.n, 40);
        assert_eq!(agg.meta.n_client_inputs, 2);
    }

    #[test]
    fn aggregate_text_taxonomy_empty_input() {
        let agg = aggregate_text_taxonomy(&[]);
        assert!(agg.line_pools.is_empty());
        assert!(agg.header_pools.is_empty());
        assert!(agg.coa_pools.is_empty());
    }

    // -----------------------------------------------------------------------
    // SP6 T8 — aggregate_industry_priors text_taxonomy wiring
    // -----------------------------------------------------------------------

    /// Minimal `BehavioralPriors` fixture for aggregation tests (text_taxonomy: None).
    fn minimal_behavioral_priors_fixture(industry: &str) -> BehavioralPriors {
        BehavioralPriors {
            schema_version: BehavioralPriors::SCHEMA_VERSION,
            generator_version: "test".into(),
            industry: industry.into(),
            n_client_inputs: 1,
            n_rows_aggregated: 1000,
            source_mix: smix(&[("KR", 1.0)], 0.0, 0.005),
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
            tb_anchor: None,
        }
    }

    #[test]
    fn aggregate_industry_priors_populates_text_taxonomy() {
        // Build two BehavioralPriors each carrying a text_taxonomy with one line pool;
        // call aggregate_industry_priors; assert the aggregated bundle's text_taxonomy
        // unions them and reports n_client_inputs from the contributing taxonomies.
        let mut bp_a = minimal_behavioral_priors_fixture("test");
        bp_a.text_taxonomy = Some(taxonomy_with_line("KR|A.B", "Rechnung", 1.0, 20));
        let mut bp_b = minimal_behavioral_priors_fixture("test");
        bp_b.text_taxonomy = Some(taxonomy_with_line("KR|A.B", "Gutschrift", 1.0, 5));

        let agg = aggregate_industry_priors(&[&bp_a, &bp_b], "test").expect("ok");
        let tx = agg.text_taxonomy.expect("text_taxonomy must be populated");
        assert!(tx.line_pools.contains_key("KR|A.B"));
        assert_eq!(tx.meta.n_client_inputs, 2);
    }
}

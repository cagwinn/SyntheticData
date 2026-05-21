//! P1–P4 behavioral-fidelity evaluation for GL data.
//!
//! Implements the Sajja (2026) framework adapted for GL semantics:
//! `Source` as the primary entity, `TradingPartner` as secondary, `EntryDate`
//! at day resolution, with a structural JE-line-burst metric and a canonical
//! R1..R10 velocity rule set. Anchors every metric to a 50/50-split noise
//! floor via the degradation-ratio normaliser.
//!
//! Spec: `docs/superpowers/specs/2026-05-11-sp1-behavioral-fidelity-design.md`.

pub mod burst;
pub mod degradation;
pub mod entity_profile;
pub mod error;
pub mod fanout;
pub mod ietd;
pub mod intraday;
pub mod loader;
pub mod math;
pub mod report;
pub mod types;
pub mod velocity_rules;

pub use entity_profile::{gl_source_tp, real_corpus_aliases, synthetic_aliases};
pub use error::{BehavioralFidelityError, BehavioralFidelityResult};
pub use report::BehavioralFidelityReport;
pub use types::{BehavioralFidelityConfig, EntityProfile, GateThresholds, Record, RuleSet};

use std::collections::BTreeMap;
use std::path::Path;

use chrono::Utc;

use crate::behavioral_fidelity::report::{
    BaselineValues, CorpusSummary, EntityMetrics, GateResult, PerMetric,
};

const SELF_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Construct a [`PerMetric`] with `is_degenerate_baseline` and `is_volume_bounded`
/// populated from the baseline value and canonical metric name respectively.
/// SP3.13 W3: `name` is matched against [`degradation::VOLUME_BOUNDED_METRICS`].
fn per_metric(name: &str, raw: f64, baseline: f64, dr: f64) -> PerMetric {
    PerMetric {
        raw,
        baseline,
        dr,
        is_degenerate_baseline: degradation::is_degenerate_baseline(baseline),
        is_volume_bounded: degradation::is_volume_bounded(name),
    }
}

pub fn compute_report(
    cfg: &BehavioralFidelityConfig,
    real: &[Record],
    syn: &[Record],
) -> BehavioralFidelityResult<BehavioralFidelityReport> {
    // Bound the corpus to a fixed effective scale so the 50/50 noise-floor
    // split stays non-degenerate — a full multi-million-JE corpus splits into
    // statistically identical halves and every DR saturates. No-op at or below
    // the cap (existing baselines unaffected); applied to the raw comparison
    // too, so raw + baseline share the same scale.
    let real_capped =
        degradation::subsample_to_je_cap(real, degradation::NOISE_FLOOR_JE_CAP, cfg.seed);
    let real: &[Record] = &real_capped;
    let (real_a, real_b) = degradation::split_5050(real, cfg.seed);

    let mut per_entity = BTreeMap::new();

    // Primary entity
    let em_primary = compute_entity_metrics(
        &cfg.profile,
        real,
        syn,
        &real_a,
        &real_b,
        &cfg.profile.primary_entity,
    )?;
    per_entity.insert(cfg.profile.primary_entity.clone(), em_primary);

    // Secondary entity (optional)
    if let Some(sec) = &cfg.profile.secondary_entity {
        let em_sec = compute_entity_metrics(&cfg.profile, real, syn, &real_a, &real_b, sec)?;
        per_entity.insert(sec.clone(), em_sec);
    }

    // P4 attached to primary entity only.
    let (rule_results, mean_gap) =
        velocity_rules::evaluate_rule_set(&cfg.rule_set, real, syn, |r| {
            project_entity(r, &cfg.profile.primary_entity)
        });
    let (_, mean_gap_baseline) =
        velocity_rules::evaluate_rule_set(&cfg.rule_set, &real_a, &real_b, |r| {
            project_entity(r, &cfg.profile.primary_entity)
        });
    if let Some(em) = per_entity.get_mut(&cfg.profile.primary_entity) {
        em.p4_rule_results = rule_results;
        em.p4_mean_gap = per_metric(
            "P4_MeanGap",
            mean_gap,
            mean_gap_baseline,
            degradation::degradation_ratio(mean_gap, mean_gap_baseline),
        );
    }

    let intraday =
        intraday::compute_intraday(syn, |r| project_entity(r, &cfg.profile.primary_entity));

    let noise_floor = collect_baseline_values(&per_entity, &cfg.profile);
    let (
        composite_bf_score,
        composite_bf_median,
        composite_bf_volume_corrected,
        n_metrics_aggregated,
        n_metrics_excluded_degenerate,
        n_metrics_excluded_volume,
    ) = compute_composite_bf(&per_entity);

    let gates = build_gate_result(&cfg.fail_thresholds, &per_entity, composite_bf_score);

    Ok(BehavioralFidelityReport {
        profile: cfg.profile.name.clone(),
        generator_id: "datasynth".to_string(),
        generator_version: SELF_VERSION.to_string(),
        seed: cfg.seed,
        generated_at: Utc::now(),
        real_corpus: summary(real, &cfg.profile),
        synthetic: summary(syn, &cfg.profile),
        noise_floor,
        per_entity,
        composite_bf_score,
        composite_bf_median,
        composite_bf_volume_corrected,
        n_metrics_aggregated,
        n_metrics_excluded_degenerate,
        n_metrics_excluded_volume,
        intraday_structural: intraday,
        gates,
    })
}

pub fn compute_report_from_paths(
    cfg: &BehavioralFidelityConfig,
    real_path: &Path,
    syn_path: &Path,
) -> BehavioralFidelityResult<BehavioralFidelityReport> {
    let real = load_any(real_path)?;
    let syn = load_any(syn_path)?;
    compute_report(cfg, &real, &syn)
}

fn load_any(p: &Path) -> BehavioralFidelityResult<Vec<Record>> {
    if p.is_dir() {
        for entry in std::fs::read_dir(p)? {
            let path = entry?.path();
            if let Some(ext) = path.extension() {
                if ext.eq_ignore_ascii_case("parquet") {
                    return loader::load_parquet_records(&path);
                }
                if ext.eq_ignore_ascii_case("csv") {
                    return loader::load_csv_records(&path);
                }
            }
        }
        return Err(BehavioralFidelityError::Io(std::io::Error::other(
            "no .parquet or .csv in dir",
        )));
    }
    match p.extension().and_then(|s| s.to_str()) {
        Some("parquet") => loader::load_parquet_records(p),
        Some("csv") => loader::load_csv_records(p),
        _ => Err(BehavioralFidelityError::Io(std::io::Error::other(
            "unknown extension",
        ))),
    }
}

fn compute_entity_metrics(
    profile: &EntityProfile,
    real: &[Record],
    syn: &[Record],
    real_a: &[Record],
    real_b: &[Record],
    entity_col: &str,
) -> BehavioralFidelityResult<EntityMetrics> {
    let project = |r: &Record| project_entity(r, entity_col);

    // P1
    let p1 = ietd::compute_p1(real, syn, project, |r| r.entry_date);
    let p1_bl = ietd::compute_p1(real_a, real_b, project, |r| r.entry_date);
    let p1_ietd = per_metric(
        "P1_IETD_W1_days",
        p1.ietd_w1_days,
        p1_bl.ietd_w1_days,
        degradation::degradation_ratio(p1.ietd_w1_days, p1_bl.ietd_w1_days),
    );
    let p1_autocorr = per_metric(
        "P1_AutocorrGap",
        p1.autocorr_gap,
        p1_bl.autocorr_gap,
        degradation::degradation_ratio(p1.autocorr_gap, p1_bl.autocorr_gap),
    );

    // P2 active lifetime
    let p2_al_raw = burst::active_lifetime_w1(real, syn, project, |r| r.entry_date);
    let p2_al_bl = burst::active_lifetime_w1(real_a, real_b, project, |r| r.entry_date);
    let p2_active_lifetime = per_metric(
        "P2_ActiveLifetime_W1",
        p2_al_raw,
        p2_al_bl,
        degradation::degradation_ratio(p2_al_raw, p2_al_bl),
    );

    // P2 burst length per threshold
    let mut p2_burst_len_by_threshold = BTreeMap::new();
    for t in &profile.burst_thresholds {
        let raw = burst::burst_length_w1(real, syn, project, |r| r.entry_date, *t);
        let bl = burst::burst_length_w1(real_a, real_b, project, |r| r.entry_date, *t);
        let name = format!("P2_BurstLen_W1_{}d", t);
        p2_burst_len_by_threshold.insert(
            *t,
            per_metric(&name, raw, bl, degradation::degradation_ratio(raw, bl)),
        );
    }

    // P2 JE-line-burst (structural)
    let p2_jl_raw = burst::je_line_burst_w1(real, syn);
    let p2_jl_bl = burst::je_line_burst_w1(real_a, real_b);
    let p2_je_line_burst = per_metric(
        "P2_JELineBurst_W1",
        p2_jl_raw,
        p2_jl_bl,
        degradation::degradation_ratio(p2_jl_raw, p2_jl_bl),
    );

    // P3 fanout per attribute
    let mut p3_fanout_by_attr = BTreeMap::new();
    for attr in &profile.attributes_for_p3 {
        let attr_proj = make_attr_projector(attr);
        let raw = fanout::fanout_w1(real, syn, project, attr_proj);
        let bl = fanout::fanout_w1(real_a, real_b, project, attr_proj);
        let name = format!("P3_Fanout_W1_{}", attr);
        p3_fanout_by_attr.insert(
            attr.clone(),
            per_metric(&name, raw, bl, degradation::degradation_ratio(raw, bl)),
        );
    }

    // P3 clustering & triangles — pick the first attribute as canonical for the projection
    let canonical_attr = profile
        .attributes_for_p3
        .first()
        .map(|a| make_attr_projector(a))
        .unwrap_or(fanout::gl_account_of);
    let cc_real = fanout::clustering_coefficient(real, project, canonical_attr);
    let cc_syn = fanout::clustering_coefficient(syn, project, canonical_attr);
    let cc_a = fanout::clustering_coefficient(real_a, project, canonical_attr);
    let cc_b = fanout::clustering_coefficient(real_b, project, canonical_attr);
    let cc_gap_real_syn = (cc_real - cc_syn).abs();
    let cc_gap_bl = (cc_a - cc_b).abs();
    let p3_clustering = per_metric(
        "P3_ClusteringGap",
        cc_gap_real_syn,
        cc_gap_bl,
        degradation::degradation_ratio(cc_gap_real_syn, cc_gap_bl),
    );

    let t_real = fanout::triangle_count(real, project, canonical_attr);
    let t_syn = fanout::triangle_count(syn, project, canonical_attr);
    let t_a = fanout::triangle_count(real_a, project, canonical_attr);
    let t_b = fanout::triangle_count(real_b, project, canonical_attr);
    let tr_raw = fanout::triangle_log_ratio_gap(t_real, t_syn);
    let tr_bl = fanout::triangle_log_ratio_gap(t_a, t_b);
    let p3_triangle_log_ratio = per_metric(
        "P3_TriangleLogRatio",
        tr_raw,
        tr_bl,
        degradation::degradation_ratio(tr_raw, tr_bl),
    );

    Ok(EntityMetrics {
        entity_column: entity_col.to_string(),
        p1_ietd,
        p1_autocorr,
        p2_active_lifetime,
        p2_burst_len_by_threshold,
        p2_je_line_burst,
        p3_fanout_by_attr,
        p3_clustering,
        p3_triangle_log_ratio,
        p4_rule_results: Vec::new(),
        p4_mean_gap: per_metric("P4_MeanGap", 0.0, 0.0, 0.0),
    })
}

fn project_entity(r: &Record, col: &str) -> Option<String> {
    match col {
        "Source" => Some(r.source.clone()),
        "TradingPartner" => r.trading_partner.clone(),
        "GLAccount" => Some(r.gl_account.clone()),
        "CostCenter" => r.cost_center.clone(),
        "ProfitCenter" => r.profit_center.clone(),
        _ => None,
    }
}

fn make_attr_projector(attr: &str) -> fn(&Record) -> Option<String> {
    match attr {
        "GLAccount" => fanout::gl_account_of,
        "CostCenter" => fanout::cost_center_of,
        "ProfitCenter" => fanout::profit_center_of,
        "TradingPartner" => fanout::trading_partner_attr_of,
        _ => fanout::gl_account_of,
    }
}

fn summary(records: &[Record], profile: &EntityProfile) -> CorpusSummary {
    let entities_p: std::collections::HashSet<String> = records
        .iter()
        .filter_map(|r| project_entity(r, &profile.primary_entity))
        .collect();
    let entities_s: std::collections::HashSet<String> = profile
        .secondary_entity
        .as_ref()
        .map(|c| {
            records
                .iter()
                .filter_map(|r| project_entity(r, c))
                .collect()
        })
        .unwrap_or_default();
    let mut period_start = None;
    let mut period_end = None;
    for r in records {
        period_start =
            Some(period_start.map_or(r.entry_date, |d: chrono::NaiveDate| d.min(r.entry_date)));
        period_end =
            Some(period_end.map_or(r.entry_date, |d: chrono::NaiveDate| d.max(r.entry_date)));
    }
    CorpusSummary {
        path: "(in-memory)".to_string(),
        n_rows: records.len(),
        n_entities_primary: entities_p.len(),
        n_entities_secondary: entities_s.len(),
        period_start,
        period_end,
    }
}

fn collect_baseline_values(
    per_entity: &BTreeMap<String, EntityMetrics>,
    profile: &EntityProfile,
) -> BaselineValues {
    let primary = per_entity.get(&profile.primary_entity).cloned();
    let mut p2_burst_len = BTreeMap::new();
    let mut p3_fanout = BTreeMap::new();
    let mut bv = BaselineValues {
        p1_ietd_w1_days: 0.0,
        p1_autocorr_gap: 0.0,
        p2_active_lifetime_w1: 0.0,
        p2_burst_len_by_threshold: BTreeMap::new(),
        p2_je_line_burst_w1: 0.0,
        p3_fanout_by_attr: BTreeMap::new(),
        p3_clustering_gap: 0.0,
        p3_triangle_log_ratio: 0.0,
        p4_mean_gap: 0.0,
    };
    if let Some(p) = primary {
        bv.p1_ietd_w1_days = p.p1_ietd.baseline;
        bv.p1_autocorr_gap = p.p1_autocorr.baseline;
        bv.p2_active_lifetime_w1 = p.p2_active_lifetime.baseline;
        for (t, pm) in &p.p2_burst_len_by_threshold {
            p2_burst_len.insert(*t, pm.baseline);
        }
        bv.p2_burst_len_by_threshold = p2_burst_len;
        bv.p2_je_line_burst_w1 = p.p2_je_line_burst.baseline;
        for (a, pm) in &p.p3_fanout_by_attr {
            p3_fanout.insert(a.clone(), pm.baseline);
        }
        bv.p3_fanout_by_attr = p3_fanout;
        bv.p3_clustering_gap = p.p3_clustering.baseline;
        bv.p3_triangle_log_ratio = p.p3_triangle_log_ratio.baseline;
        bv.p4_mean_gap = p.p4_mean_gap.baseline;
    }
    bv
}

/// Collect all per-entity per-metric [`PerMetric`] records into a flat list,
/// then compute the arithmetic mean, median, and a volume-corrected mean over
/// non-degenerate metrics only.
///
/// Returns `(mean, median, vol_corrected_mean, n_aggregated, n_excluded_degenerate, n_excluded_volume)`.
///
/// - **Degenerate-baseline** metrics (`is_degenerate_baseline = true`) are excluded
///   from all three aggregates.
/// - **Volume-bounded** metrics (`is_volume_bounded = true`) are included in the
///   headline `mean` and `median` but excluded from `vol_corrected_mean`.  They are
///   still present in `per_entity` for drill-down inspection.
///
/// The median is robust to a small number of extremely high-DR outlier metrics
/// (e.g. Source P1_IETD = 398) and should be reported alongside the mean so
/// users can gauge how much skew the distribution contains.
fn compute_composite_bf(
    per_entity: &BTreeMap<String, EntityMetrics>,
) -> (f64, f64, f64, usize, usize, usize) {
    let mut included: Vec<f64> = Vec::new();
    let mut vol_corrected: Vec<f64> = Vec::new();
    let mut n_excluded_degen: usize = 0;
    let mut n_excluded_volume: usize = 0;

    let mut push = |pm: &PerMetric| {
        if pm.is_degenerate_baseline {
            n_excluded_degen += 1;
        } else {
            included.push(pm.dr);
            if pm.is_volume_bounded {
                n_excluded_volume += 1;
            } else {
                vol_corrected.push(pm.dr);
            }
        }
    };

    for em in per_entity.values() {
        push(&em.p1_ietd);
        push(&em.p1_autocorr);
        push(&em.p2_active_lifetime);
        for pm in em.p2_burst_len_by_threshold.values() {
            push(pm);
        }
        push(&em.p2_je_line_burst);
        for pm in em.p3_fanout_by_attr.values() {
            push(pm);
        }
        push(&em.p3_clustering);
        push(&em.p3_triangle_log_ratio);
        push(&em.p4_mean_gap);
    }

    let n_aggregated = included.len();
    if included.is_empty() {
        return (0.0, 0.0, 0.0, 0, n_excluded_degen, n_excluded_volume);
    }

    let mean = included.iter().sum::<f64>() / included.len() as f64;

    let median = {
        let mut sorted = included.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = sorted.len() / 2;
        if sorted.len().is_multiple_of(2) {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        } else {
            sorted[mid]
        }
    };

    let vol_corrected_mean = if vol_corrected.is_empty() {
        0.0
    } else {
        vol_corrected.iter().sum::<f64>() / vol_corrected.len() as f64
    };

    (
        mean,
        median,
        vol_corrected_mean,
        n_aggregated,
        n_excluded_degen,
        n_excluded_volume,
    )
}

fn build_gate_result(
    thresholds: &GateThresholds,
    per_entity: &BTreeMap<String, EntityMetrics>,
    composite: f64,
) -> GateResult {
    let mut failures = Vec::new();
    for (name, em) in per_entity {
        let metric_checks: Vec<(&str, f64)> = vec![
            ("P1_IETD", em.p1_ietd.dr),
            ("P1_Autocorr", em.p1_autocorr.dr),
            ("P2_ActiveLifetime", em.p2_active_lifetime.dr),
            ("P2_JELineBurst", em.p2_je_line_burst.dr),
            ("P3_Clustering", em.p3_clustering.dr),
            ("P3_TriangleLogRatio", em.p3_triangle_log_ratio.dr),
            ("P4_MeanGap", em.p4_mean_gap.dr),
        ];
        for (mname, dr) in metric_checks {
            if dr > thresholds.fail_if_dr_above {
                failures.push(format!(
                    "{}/{} DR={:.3} > {:.2}",
                    name, mname, dr, thresholds.fail_if_dr_above
                ));
            }
        }
        for (t, pm) in &em.p2_burst_len_by_threshold {
            if pm.dr > thresholds.fail_if_dr_above {
                failures.push(format!(
                    "{}/P2_BurstLen_{}d DR={:.3} > {:.2}",
                    name, t, pm.dr, thresholds.fail_if_dr_above
                ));
            }
        }
        for (attr, pm) in &em.p3_fanout_by_attr {
            if pm.dr > thresholds.fail_if_dr_above {
                failures.push(format!(
                    "{}/P3_Fanout_{} DR={:.3} > {:.2}",
                    name, attr, pm.dr, thresholds.fail_if_dr_above
                ));
            }
        }
    }
    if composite > thresholds.fail_if_composite_above {
        failures.push(format!(
            "Composite BF={:.3} > {:.2}",
            composite, thresholds.fail_if_composite_above
        ));
    }
    GateResult {
        fail_if_dr_above: thresholds.fail_if_dr_above,
        fail_if_composite_above: thresholds.fail_if_composite_above,
        passed: failures.is_empty(),
        failures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn make_records(source: &str, days: &[u32], je_prefix: &str) -> Vec<Record> {
        days.iter()
            .enumerate()
            .map(|(i, &d)| Record {
                source: source.into(),
                gl_account: "1100".into(),
                cost_center: Some("CC1".into()),
                profit_center: Some("PC1".into()),
                trading_partner: Some("TP1".into()),
                je_number: format!("{je_prefix}-{i:03}"),
                je_line_number: "001".into(),
                effective_date: NaiveDate::from_ymd_opt(2022, 1, d).unwrap(),
                entry_date: NaiveDate::from_ymd_opt(2022, 1, d).unwrap(),
                created_at: None,
                functional_amount: 100.0,
                header_text: String::new(),
                line_text: String::new(),
            })
            .collect()
    }

    #[test]
    fn compute_report_identical_produces_low_composite() {
        let mut real = make_records("SRC_A", &[3, 4, 5, 6, 7, 10, 11, 12, 13, 14], "JA");
        real.extend(make_records(
            "SRC_B",
            &[3, 5, 7, 10, 12, 14, 17, 19, 21, 24],
            "JB",
        ));

        let cfg = BehavioralFidelityConfig::gl_default();
        let report = compute_report(&cfg, &real, &real)
            .expect("compute_report should succeed on identical inputs");

        // Identical real vs syn should have near-zero raw distances => near-zero DR
        assert!(
            report.composite_bf_score < 1.0,
            "identical data composite should be well below 1.0, got {}",
            report.composite_bf_score
        );
        assert!(
            report.per_entity.contains_key("Source"),
            "primary entity 'Source' must be present"
        );
        assert!(
            report.per_entity.contains_key("TradingPartner"),
            "secondary entity 'TradingPartner' must be present"
        );
    }

    #[test]
    fn compute_report_gates_pass_on_identical() {
        let real = make_records("SRC_A", &[3, 4, 5, 6, 7, 10, 11, 12, 13, 14], "JA");
        let cfg = BehavioralFidelityConfig::gl_default();
        let report = compute_report(&cfg, &real, &real).expect("compute_report should succeed");
        assert!(
            report.gates.passed,
            "gates should pass on identical data; failures: {:?}",
            report.gates.failures
        );
    }

    #[test]
    fn compute_report_summary_counts_entities() {
        let mut real = make_records("SRC_A", &[3, 4, 5], "JA");
        real.extend(make_records("SRC_B", &[6, 7, 8], "JB"));
        let cfg = BehavioralFidelityConfig::gl_default();
        let report = compute_report(&cfg, &real, &real).expect("compute_report");
        assert_eq!(report.real_corpus.n_rows, 6);
        assert_eq!(report.real_corpus.n_entities_primary, 2); // SRC_A, SRC_B
        assert_eq!(report.synthetic.n_rows, 6);
    }

    #[test]
    fn noise_floor_baseline_populated_from_primary() {
        let real = make_records("SRC_A", &[3, 4, 5, 6, 7, 10, 11, 12], "JA");
        let cfg = BehavioralFidelityConfig::gl_default();
        let report = compute_report(&cfg, &real, &real).expect("compute_report");
        // noise_floor should be populated with baseline values from the primary entity
        let em_primary = report
            .per_entity
            .get("Source")
            .expect("Source entity present");
        assert!(
            (report.noise_floor.p1_ietd_w1_days - em_primary.p1_ietd.baseline).abs() < 1e-9,
            "noise_floor.p1_ietd_w1_days must match primary baseline"
        );
    }

    #[test]
    fn compute_report_version_and_seed_set() {
        let real = make_records("SRC_A", &[3, 4, 5], "JA");
        let cfg = BehavioralFidelityConfig::gl_default();
        let report = compute_report(&cfg, &real, &real).expect("compute_report");
        assert_eq!(report.generator_id, "datasynth");
        assert!(!report.generator_version.is_empty());
        assert_eq!(report.seed, 42);
    }

    #[test]
    fn per_entity_has_p4_rule_results_for_primary() {
        let real = make_records("SRC_A", &[3, 4, 5, 6, 7, 10, 11, 12], "JA");
        let cfg = BehavioralFidelityConfig::gl_default();
        let report = compute_report(&cfg, &real, &real).expect("compute_report");
        let em = report
            .per_entity
            .get("Source")
            .expect("Source entity present");
        assert_eq!(
            em.p4_rule_results.len(),
            10,
            "canonical rule set has 10 rules"
        );
    }

    // SP3.10 — degenerate-baseline exclusion tests

    /// Build a synthetic EntityMetrics with controllable per-metric baselines.
    /// Returns a BTreeMap ready to pass to compute_composite_bf.
    fn make_per_entity_with_metrics(
        healthy_drs: &[f64],
        degenerate_count: usize,
    ) -> BTreeMap<String, EntityMetrics> {
        use crate::behavioral_fidelity::report::PerMetric;

        // Build a PerMetric with the given DR and baseline.
        let healthy_pm = |dr: f64| PerMetric {
            raw: dr,
            baseline: 1.0, // non-degenerate
            dr,
            is_degenerate_baseline: false,
            is_volume_bounded: false,
        };
        let degenerate_pm = || PerMetric {
            raw: 1.0,
            baseline: 0.0, // degenerate
            dr: degradation::DEGENERATE_BASELINE_CAP,
            is_degenerate_baseline: true,
            is_volume_bounded: false,
        };

        // Construct a minimal EntityMetrics.  We put some healthy metrics in
        // p3_fanout_by_attr so the slice length is flexible.
        let mut p3_fanout = BTreeMap::new();
        for (i, &dr) in healthy_drs.iter().enumerate() {
            p3_fanout.insert(format!("attr_{i}"), healthy_pm(dr));
        }
        // Put degenerate metrics in p2_burst_len_by_threshold (arbitrary slot).
        let mut p2_burst = BTreeMap::new();
        for i in 0..degenerate_count {
            p2_burst.insert(i as i64, degenerate_pm());
        }

        let em = EntityMetrics {
            entity_column: "Source".into(),
            p1_ietd: healthy_pm(0.0),
            p1_autocorr: healthy_pm(0.0),
            p2_active_lifetime: healthy_pm(0.0),
            p2_burst_len_by_threshold: p2_burst,
            p2_je_line_burst: healthy_pm(0.0),
            p3_fanout_by_attr: p3_fanout,
            p3_clustering: healthy_pm(0.0),
            p3_triangle_log_ratio: healthy_pm(0.0),
            p4_rule_results: vec![],
            p4_mean_gap: healthy_pm(0.0),
        };
        let mut map = BTreeMap::new();
        map.insert("Source".to_string(), em);
        map
    }

    #[test]
    fn composite_excludes_degenerate_baseline_metrics() {
        // 5 healthy metrics each with DR=10.0, 1 degenerate capped at 100.
        // Under old formula: (5×10 + 100) / 6 ≈ 25.0
        // Under new formula: 5×10 / 5 = 10.0 (degenerate excluded)
        let per_entity = make_per_entity_with_metrics(&[10.0, 10.0, 10.0, 10.0, 10.0], 1);
        let (composite, _median, _vol, n_agg, n_excl, _n_vol) = compute_composite_bf(&per_entity);

        // The 5 healthy P3_fanout metrics each contribute DR=10; the 6 fixed
        // metrics (P1_ietd, P1_autocorr, P2_active_lifetime, P2_je_line_burst,
        // P3_clustering, P3_triangle_log_ratio, P4_mean_gap) all have DR=0.
        // So included = [0,0,0,0,10,10,10,10,10,0,0], sum=50, len=11, mean≈4.55.
        // The key assertion is that the 1 degenerate metric is excluded.
        assert_eq!(n_excl, 1, "exactly 1 degenerate metric should be excluded");
        assert!(n_agg >= 1, "at least one healthy metric must be aggregated");
        // Composite must be strictly less than 100/n (what the old formula gave
        // for a single degenerate with cap=100).
        assert!(
            composite < 100.0 / (n_agg + n_excl) as f64 + 1e-6,
            "composite {composite} should be far below the old degenerate-dominated value"
        );
    }

    #[test]
    fn composite_returns_zero_when_all_metrics_degenerate() {
        // All baselines are 0 → every metric is degenerate → excluded → empty mean → 0.0
        // We build an EntityMetrics where every fixed slot is also degenerate.
        use crate::behavioral_fidelity::report::PerMetric;
        let degen = PerMetric {
            raw: 1.0,
            baseline: 0.0,
            dr: degradation::DEGENERATE_BASELINE_CAP,
            is_degenerate_baseline: true,
            is_volume_bounded: false,
        };
        let em = EntityMetrics {
            entity_column: "Source".into(),
            p1_ietd: degen.clone(),
            p1_autocorr: degen.clone(),
            p2_active_lifetime: degen.clone(),
            p2_burst_len_by_threshold: BTreeMap::new(),
            p2_je_line_burst: degen.clone(),
            p3_fanout_by_attr: BTreeMap::new(),
            p3_clustering: degen.clone(),
            p3_triangle_log_ratio: degen.clone(),
            p4_rule_results: vec![],
            p4_mean_gap: degen,
        };
        let mut per_entity = BTreeMap::new();
        per_entity.insert("Source".to_string(), em);

        let (composite, _median, _vol, n_agg, n_excl, _n_vol) = compute_composite_bf(&per_entity);
        assert_eq!(composite, 0.0, "all-degenerate composite should be 0.0");
        assert_eq!(n_agg, 0);
        assert_eq!(n_excl, 7, "7 fixed metrics, all degenerate");
    }

    // SP3.11 W2 — median composite tests

    #[test]
    fn compute_composite_bf_returns_mean_and_median() {
        // 5 p3_fanout metrics with DRs [1, 5, 10, 20, 100]; 7 fixed metrics each 0.0.
        // Included DRs (12 total): [0,0,0,0,0,0,0, 1,5,10,20,100]
        // Mean = (0*7 + 1+5+10+20+100) / 12 = 136/12 ≈ 11.333
        // Sorted: [0,0,0,0,0,0,0,1,5,10,20,100], mid=6, even → (sorted[5]+sorted[6])/2 = (0+0)/2 = 0.0
        // We verify via known properties: mean > median (skewed right).
        let per_entity = make_per_entity_with_metrics(&[1.0, 5.0, 10.0, 20.0, 100.0], 0);
        let (mean, median, _vol, n, excl, _n_vol) = compute_composite_bf(&per_entity);
        assert_eq!(excl, 0);
        assert_eq!(n, 12, "7 fixed + 5 fanout metrics");
        assert!(
            mean > median,
            "mean ({mean:.3}) should exceed median ({median:.3}) for right-skewed distribution"
        );
        // 100 is the outlier; mean should be noticeably pulled up
        assert!(mean > 5.0, "mean dragged up by outlier 100");
        // Median is robust — most DRs are 0 or small so median stays low
        assert!(median < mean, "median robust to outlier");
    }

    #[test]
    fn compute_composite_bf_median_robust_to_outlier() {
        // 4 fanout metrics around 5-15, 1 extreme outlier at 1000.
        // Fixed slots (7) all 0.0. Total 12 metrics.
        // Mean is dragged up by 1000; median stays near the bulk.
        let per_entity = make_per_entity_with_metrics(&[5.0, 10.0, 12.0, 15.0, 1000.0], 0);
        let (mean, median, _vol, n, excl, _n_vol) = compute_composite_bf(&per_entity);
        assert_eq!(excl, 0);
        assert_eq!(n, 12);
        // Outlier at 1000 raises the mean far above the typical 0-15 range
        assert!(
            mean > 50.0,
            "mean should be pulled up by outlier 1000; got {mean:.3}"
        );
        // Median of [0,0,0,0,0,0,0,5,10,12,15,1000] is (0+5)/2 = 2.5 — well below mean
        assert!(
            median < 10.0,
            "median should be robust to outlier; got {median:.3}"
        );
    }

    #[test]
    fn n_metrics_aggregated_and_excluded_on_report() {
        // Healthy identical data — no degenerate metrics expected.
        let real = make_records("SRC_A", &[3, 4, 5, 6, 7, 10, 11, 12, 13, 14], "JA");
        let cfg = BehavioralFidelityConfig::gl_default();
        let report = compute_report(&cfg, &real, &real).expect("compute_report");
        // On identical data every baseline > 0 (split A ≠ split B implies non-zero
        // IETD gap eventually; but even if some are zero the field must be set).
        assert!(
            report.n_metrics_aggregated + report.n_metrics_excluded_degenerate > 0,
            "total metric count must be positive"
        );
        // Aggregated + excluded must sum to total metric count (sanity).
        let total = report.n_metrics_aggregated + report.n_metrics_excluded_degenerate;
        assert!(total >= 7, "at least 7 fixed metrics per entity");
    }

    // SP3.13 W3 — volume-bounded annotation tests

    #[test]
    fn is_volume_bounded_flags_p1_ietd() {
        assert!(
            degradation::is_volume_bounded("P1_IETD_W1_days"),
            "P1_IETD_W1_days must be flagged as volume-bounded"
        );
        assert!(
            degradation::is_volume_bounded("P3_Fanout_W1_GLAccount"),
            "P3_Fanout_W1_GLAccount must be flagged as volume-bounded"
        );
        assert!(
            degradation::is_volume_bounded("P3_Fanout_W1_CostCenter"),
            "P3_Fanout_W1_CostCenter must be flagged as volume-bounded"
        );
        assert!(
            degradation::is_volume_bounded("P2_BurstLen_W1_7d"),
            "P2_BurstLen_W1_7d must be flagged as volume-bounded"
        );
        assert!(
            !degradation::is_volume_bounded("P3_ClusteringGap"),
            "P3_ClusteringGap must NOT be volume-bounded"
        );
        assert!(
            !degradation::is_volume_bounded("P4_MeanGap"),
            "P4_MeanGap must NOT be volume-bounded"
        );
        assert!(
            !degradation::is_volume_bounded("P1_AutocorrGap"),
            "P1_AutocorrGap must NOT be volume-bounded"
        );
    }

    #[test]
    fn compute_composite_bf_volume_corrected_excludes_volume_bounded() {
        use crate::behavioral_fidelity::report::PerMetric;

        // Build an entity with two categories of non-degenerate metrics:
        //   - p1_ietd: is_volume_bounded=true, dr=50.0
        //   - all other fixed metrics: dr=10.0, not volume-bounded
        // Volume-corrected mean must exclude p1_ietd (and any other vb).
        let vol_bounded_pm = PerMetric {
            raw: 50.0,
            baseline: 1.0,
            dr: 50.0,
            is_degenerate_baseline: false,
            is_volume_bounded: true,
        };
        let healthy_pm = PerMetric {
            raw: 10.0,
            baseline: 1.0,
            dr: 10.0,
            is_degenerate_baseline: false,
            is_volume_bounded: false,
        };

        let em = EntityMetrics {
            entity_column: "Source".into(),
            p1_ietd: vol_bounded_pm.clone(), // volume-bounded, dr=50
            p1_autocorr: healthy_pm.clone(),
            p2_active_lifetime: healthy_pm.clone(),
            p2_burst_len_by_threshold: BTreeMap::new(),
            p2_je_line_burst: healthy_pm.clone(),
            p3_fanout_by_attr: BTreeMap::new(),
            p3_clustering: healthy_pm.clone(),
            p3_triangle_log_ratio: healthy_pm.clone(),
            p4_rule_results: vec![],
            p4_mean_gap: healthy_pm.clone(),
        };
        let mut per_entity = BTreeMap::new();
        per_entity.insert("Source".to_string(), em);

        let (mean, _median, vol_corrected, n_agg, n_excl_degen, n_excl_vol) =
            compute_composite_bf(&per_entity);

        // 7 fixed metrics: p1_ietd(vb,50) + 6 healthy(10).
        assert_eq!(n_agg, 7, "all 7 metrics are non-degenerate");
        assert_eq!(n_excl_degen, 0, "no degenerate metrics");
        assert_eq!(n_excl_vol, 1, "exactly 1 volume-bounded metric (p1_ietd)");

        // Headline mean includes volume-bounded: (50 + 6×10) / 7 = 110/7 ≈ 15.71
        let expected_mean = (50.0 + 6.0 * 10.0) / 7.0;
        assert!(
            (mean - expected_mean).abs() < 1e-9,
            "mean={mean:.6} expected={expected_mean:.6}"
        );

        // Volume-corrected excludes p1_ietd: 6×10 / 6 = 10.0
        assert!(
            (vol_corrected - 10.0).abs() < 1e-9,
            "vol_corrected={vol_corrected:.6} expected=10.0"
        );
        assert!(
            vol_corrected < mean,
            "volume-corrected ({vol_corrected:.3}) must be below headline mean ({mean:.3}) when vb metric has high DR"
        );
    }
}

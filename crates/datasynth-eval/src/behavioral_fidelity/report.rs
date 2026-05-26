//! BehavioralFidelityReport struct + JSON / Markdown / CSV serialisation.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::error::{BehavioralFidelityError, BehavioralFidelityResult};
use super::intraday::IntradayMetrics;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorpusSummary {
    pub path: String,
    pub n_rows: usize,
    pub n_entities_primary: usize,
    pub n_entities_secondary: usize,
    pub period_start: Option<NaiveDate>,
    pub period_end: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineValues {
    pub p1_ietd_w1_days: f64,
    pub p1_autocorr_gap: f64,
    pub p2_active_lifetime_w1: f64,
    pub p2_burst_len_by_threshold: BTreeMap<i64, f64>,
    pub p2_je_line_burst_w1: f64,
    pub p3_fanout_by_attr: BTreeMap<String, f64>,
    pub p3_clustering_gap: f64,
    pub p3_triangle_log_ratio: f64,
    pub p4_mean_gap: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerMetric {
    pub raw: f64,
    pub baseline: f64,
    pub dr: f64,
    /// True when the real-split baseline was below EPS (≈ 0), meaning this
    /// metric has no measurable noise floor on the corpus.  Such metrics are
    /// excluded from the composite BF aggregation (Fix A, SP3.10).
    #[serde(default)]
    pub is_degenerate_baseline: bool,
    /// SP3.13 W3 — True for metrics whose DR scales inversely with synthetic
    /// event volume (P1 IETD, P2 ActiveLifetime, P2 BurstLen *).  A high DR
    /// on these metrics may reflect the synthetic config generating fewer
    /// events than the corpus rather than a generator fidelity gap.
    /// Dashboards should contextualise: compare same-volume runs for fidelity
    /// assessment.  This is annotation-only — these metrics are still included
    /// in the composite.
    #[serde(default)]
    pub is_volume_bounded: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityMetrics {
    pub entity_column: String,
    pub p1_ietd: PerMetric,
    pub p1_autocorr: PerMetric,
    pub p2_active_lifetime: PerMetric,
    pub p2_burst_len_by_threshold: BTreeMap<i64, PerMetric>,
    pub p2_je_line_burst: PerMetric,
    pub p3_fanout_by_attr: BTreeMap<String, PerMetric>,
    pub p3_clustering: PerMetric,
    pub p3_triangle_log_ratio: PerMetric,
    pub p4_rule_results: Vec<super::velocity_rules::RuleResult>,
    pub p4_mean_gap: PerMetric,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateResult {
    pub fail_if_dr_above: f64,
    pub fail_if_composite_above: f64,
    pub passed: bool,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BehavioralFidelityReport {
    pub profile: String,
    pub generator_id: String,
    pub generator_version: String,
    pub seed: u64,
    pub generated_at: DateTime<Utc>,
    pub reference_corpus: CorpusSummary,
    pub synthetic: CorpusSummary,
    pub noise_floor: BaselineValues,
    pub per_entity: BTreeMap<String, EntityMetrics>,
    pub composite_bf_score: f64,
    /// Median DR over the same non-degenerate metrics used for `composite_bf_score`.
    /// Robust to a small number of very high-DR outlier metrics — compare with the
    /// mean to gauge how much skew the distribution contains.
    #[serde(default)]
    pub composite_bf_median: f64,
    /// Number of per-entity per-metric records included in the composite mean.
    /// Excludes degenerate-baseline metrics (see `n_metrics_excluded_degenerate`).
    #[serde(default)]
    pub n_metrics_aggregated: usize,
    /// Number of per-entity per-metric records excluded from the composite mean
    /// because their real-split baseline was degenerate (≈ 0).  These are still
    /// present in `per_entity` with `is_degenerate_baseline = true`.
    #[serde(default)]
    pub n_metrics_excluded_degenerate: usize,
    /// SP3.13 W3 — Composite mean excluding both degenerate-baseline metrics AND
    /// volume-bounded metrics (`is_volume_bounded = true`).  This is a
    /// supplementary figure intended for volume-attribution-corrected analysis;
    /// the headline composite remains `composite_bf_score`.
    #[serde(default)]
    pub composite_bf_volume_corrected: f64,
    /// Number of non-degenerate metrics excluded from `composite_bf_volume_corrected`
    /// because they are flagged `is_volume_bounded = true`.
    #[serde(default)]
    pub n_metrics_excluded_volume: usize,
    pub intraday_structural: Option<IntradayMetrics>,
    pub gates: GateResult,
}

impl BehavioralFidelityReport {
    pub fn write_json(&self, path: &Path) -> BehavioralFidelityResult<()> {
        let f = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(f, self)?;
        Ok(())
    }
}

impl BehavioralFidelityReport {
    pub fn write_markdown(&self, path: &Path) -> BehavioralFidelityResult<()> {
        use std::fmt::Write;
        let mut buf = String::new();

        writeln!(buf, "# Behavioral-Fidelity Report").ok();
        writeln!(buf).ok();
        writeln!(buf, "- **Profile:** `{}`", self.profile).ok();
        writeln!(
            buf,
            "- **Generator:** `{}` ({})",
            self.generator_id, self.generator_version
        )
        .ok();
        writeln!(buf, "- **Seed:** {}", self.seed).ok();
        writeln!(
            buf,
            "- **Generated at:** {}",
            self.generated_at.to_rfc3339()
        )
        .ok();
        writeln!(
            buf,
            "- **Composite BF score (mean):** **{:.3}** (over {} metrics; {} excluded for degenerate baseline; 1.0 = noise floor; lower is better)",
            self.composite_bf_score,
            self.n_metrics_aggregated,
            self.n_metrics_excluded_degenerate,
        )
        .ok();
        writeln!(
            buf,
            "- **Composite BF score (median):** **{:.3}** (robust to outliers; compare with mean to gauge skew)",
            self.composite_bf_median,
        )
        .ok();
        writeln!(
            buf,
            "- **Composite BF score (volume-corrected, exc. is_volume_bounded):** **{:.3}** (over {} metrics; {} excluded as volume-bounded)",
            self.composite_bf_volume_corrected,
            self.n_metrics_aggregated.saturating_sub(self.n_metrics_excluded_volume),
            self.n_metrics_excluded_volume,
        )
        .ok();
        writeln!(buf).ok();
        writeln!(buf, "## Per-entity DR table").ok();
        writeln!(buf).ok();
        writeln!(buf, "| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |").ok();
        writeln!(buf, "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|").ok();
        for (name, m) in &self.per_entity {
            let p2_burst_avg = avg_dr(&m.p2_burst_len_by_threshold);
            let p3_fanout_avg = avg_dr_str(&m.p3_fanout_by_attr);
            writeln!(
                buf,
                "| `{}` | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |",
                name,
                m.p1_ietd.dr,
                m.p1_autocorr.dr,
                m.p2_active_lifetime.dr,
                p2_burst_avg,
                m.p2_je_line_burst.dr,
                p3_fanout_avg,
                m.p3_clustering.dr,
                m.p3_triangle_log_ratio.dr,
                m.p4_mean_gap.dr
            )
            .ok();
        }
        writeln!(buf).ok();
        writeln!(buf, "## Gate result").ok();
        writeln!(buf).ok();
        writeln!(
            buf,
            "- **Passed:** {}",
            if self.gates.passed { "yes" } else { "no" }
        )
        .ok();
        writeln!(
            buf,
            "- **Threshold (any DR):** {:.2}",
            self.gates.fail_if_dr_above
        )
        .ok();
        writeln!(
            buf,
            "- **Threshold (composite):** {:.2}",
            self.gates.fail_if_composite_above
        )
        .ok();
        if !self.gates.failures.is_empty() {
            writeln!(buf, "- **Failures:**").ok();
            for f in &self.gates.failures {
                writeln!(buf, "  - {}", f).ok();
            }
        }
        if let Some(intra) = &self.intraday_structural {
            writeln!(buf).ok();
            writeln!(buf, "## Synthetic-only intraday metrics (info)").ok();
            writeln!(buf).ok();
            writeln!(buf, "- IETD median (s): {:.2}", intra.p1_intra_w1_seconds).ok();
            writeln!(buf, "- Lag-1 autocorr (s): {:.3}", intra.p1_intra_autocorr).ok();
            writeln!(buf, "- Off-hours rate: {:.3}", intra.off_hours_rate).ok();
        }
        std::fs::write(path, buf)?;
        Ok(())
    }

    pub fn write_csv(&self, path: &Path) -> BehavioralFidelityResult<()> {
        let mut wtr = csv::Writer::from_path(path)
            .map_err(|e| BehavioralFidelityError::Io(std::io::Error::other(e.to_string())))?;
        wtr.write_record([
            "entity_column",
            "metric",
            "raw",
            "baseline",
            "dr",
            "is_degenerate_baseline",
            "is_volume_bounded",
        ])
        .map_err(|e| BehavioralFidelityError::Io(std::io::Error::other(e.to_string())))?;
        for (name, m) in &self.per_entity {
            write_metric_row(&mut wtr, name, "P1_IETD_W1_days", &m.p1_ietd)?;
            write_metric_row(&mut wtr, name, "P1_AutocorrGap", &m.p1_autocorr)?;
            write_metric_row(
                &mut wtr,
                name,
                "P2_ActiveLifetime_W1",
                &m.p2_active_lifetime,
            )?;
            for (t, v) in &m.p2_burst_len_by_threshold {
                write_metric_row(&mut wtr, name, &format!("P2_BurstLen_W1_{}d", t), v)?;
            }
            write_metric_row(&mut wtr, name, "P2_JELineBurst_W1", &m.p2_je_line_burst)?;
            for (attr, v) in &m.p3_fanout_by_attr {
                write_metric_row(&mut wtr, name, &format!("P3_Fanout_W1_{}", attr), v)?;
            }
            write_metric_row(&mut wtr, name, "P3_ClusteringGap", &m.p3_clustering)?;
            write_metric_row(
                &mut wtr,
                name,
                "P3_TriangleLogRatio",
                &m.p3_triangle_log_ratio,
            )?;
            write_metric_row(&mut wtr, name, "P4_MeanGap", &m.p4_mean_gap)?;
        }
        wtr.flush()
            .map_err(|e| BehavioralFidelityError::Io(std::io::Error::other(e.to_string())))?;
        Ok(())
    }
}

fn write_metric_row(
    wtr: &mut csv::Writer<std::fs::File>,
    entity: &str,
    metric: &str,
    pm: &PerMetric,
) -> BehavioralFidelityResult<()> {
    wtr.write_record([
        entity,
        metric,
        &format!("{:.6}", pm.raw),
        &format!("{:.6}", pm.baseline),
        &format!("{:.6}", pm.dr),
        if pm.is_degenerate_baseline {
            "true"
        } else {
            "false"
        },
        if pm.is_volume_bounded {
            "true"
        } else {
            "false"
        },
    ])
    .map_err(|e| BehavioralFidelityError::Io(std::io::Error::other(e.to_string())))?;
    Ok(())
}

fn avg_dr(map: &BTreeMap<i64, PerMetric>) -> f64 {
    if map.is_empty() {
        return 0.0;
    }
    map.values().map(|p| p.dr).sum::<f64>() / map.len() as f64
}

fn avg_dr_str(map: &BTreeMap<String, PerMetric>) -> f64 {
    if map.is_empty() {
        return 0.0;
    }
    map.values().map(|p| p.dr).sum::<f64>() / map.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_preserves_btreemap_ordering() {
        let mut by_attr = BTreeMap::new();
        by_attr.insert("CostCenter".to_string(), 1.0);
        by_attr.insert("GLAccount".to_string(), 2.0);
        let baseline = BaselineValues {
            p1_ietd_w1_days: 1.0,
            p1_autocorr_gap: 0.0,
            p2_active_lifetime_w1: 1.0,
            p2_burst_len_by_threshold: BTreeMap::new(),
            p2_je_line_burst_w1: 1.0,
            p3_fanout_by_attr: by_attr,
            p3_clustering_gap: 0.0,
            p3_triangle_log_ratio: 0.0,
            p4_mean_gap: 0.0,
        };
        let json = serde_json::to_string(&baseline).expect("serialize");
        let key_a = json.find("CostCenter").expect("CostCenter present");
        let key_g = json.find("GLAccount").expect("GLAccount present");
        assert!(key_a < key_g, "BTreeMap should produce ordered JSON keys");
        let _: BaselineValues = serde_json::from_str(&json).expect("roundtrip");
    }
}

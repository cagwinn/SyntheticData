//! Anomaly pattern extractor.
//!
//! Reads common anomaly-label columns (`is_anomaly`, `is_fraud`,
//! `anomaly_type`, `fraud_type`, `posting_date`) from the source data and
//! builds an [`AnomalyFingerprint`] describing the overall rate, category
//! distribution, per-type profiles, and temporal clustering.
//!
//! When no anomaly-label columns are present in the source (unlabelled data),
//! the extractor still emits a fingerprint with `total_anomalies = 0` and
//! `has_labels = false`, so downstream synthesize paths can cleanly detect
//! "no ground truth available" and fall back to a configured injection rate.

use std::collections::HashMap;

use crate::error::FingerprintResult;
use crate::models::{
    AnomalyCategory, AnomalyFingerprint, AnomalyOverview, AnomalyProfile, MonthlyRate,
    TemporalAnomalyPatterns,
};
use crate::privacy::PrivacyEngine;

use super::{CsvDataSource, DataSource, ExtractedComponent, ExtractionConfig, Extractor};

/// Extractor for anomaly patterns.
pub struct AnomalyExtractor;

impl Extractor for AnomalyExtractor {
    fn name(&self) -> &'static str {
        "anomalies"
    }

    fn extract(
        &self,
        data: &DataSource,
        _config: &ExtractionConfig,
        _privacy: &mut PrivacyEngine,
    ) -> FingerprintResult<ExtractedComponent> {
        let fingerprint = match data {
            DataSource::Csv(csv) => extract_from_csv(csv)?,
            // Non-CSV sources fall back to an empty fingerprint for now —
            // Parquet / JSON label extraction would require per-source
            // column-typed readers, which would expand scope substantially.
            _ => AnomalyFingerprint::new(AnomalyOverview::new(0, 0)),
        };
        Ok(ExtractedComponent::Anomalies(fingerprint))
    }
}

/// Common column names that carry anomaly labels (case-insensitive match).
const IS_ANOMALY_COLS: &[&str] = &["is_anomaly", "anomaly", "isanomaly"];
const IS_FRAUD_COLS: &[&str] = &["is_fraud", "fraud", "isfraud"];
const ANOMALY_TYPE_COLS: &[&str] = &["anomaly_type", "anomalytype"];
const FRAUD_TYPE_COLS: &[&str] = &["fraud_type", "fraudtype"];
const POSTING_DATE_COLS: &[&str] = &["posting_date", "postingdate", "date", "transaction_date"];

fn find_column(headers: &[String], candidates: &[&str]) -> Option<usize> {
    headers.iter().position(|h| {
        let lower = h.to_ascii_lowercase();
        candidates.iter().any(|c| lower == *c)
    })
}

/// Extract an anomaly fingerprint from a CSV file.
fn extract_from_csv(csv: &CsvDataSource) -> FingerprintResult<AnomalyFingerprint> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(csv.has_headers)
        .delimiter(csv.delimiter)
        .from_path(&csv.path)?;

    let headers: Vec<String> = reader
        .headers()?
        .iter()
        .map(std::string::ToString::to_string)
        .collect();

    let is_anomaly_idx = find_column(&headers, IS_ANOMALY_COLS);
    let is_fraud_idx = find_column(&headers, IS_FRAUD_COLS);
    let anomaly_type_idx = find_column(&headers, ANOMALY_TYPE_COLS);
    let fraud_type_idx = find_column(&headers, FRAUD_TYPE_COLS);
    let posting_date_idx = find_column(&headers, POSTING_DATE_COLS);

    let has_labels = is_anomaly_idx.is_some() || is_fraud_idx.is_some();
    if !has_labels {
        return Ok(AnomalyFingerprint::new(AnomalyOverview::new(0, 0)));
    }

    let mut total_records: u64 = 0;
    let mut total_anomalies: u64 = 0;
    let mut total_fraud: u64 = 0;
    // count per (category, type_name)
    let mut by_type: HashMap<(AnomalyCategory, String), u64> = HashMap::new();
    // YYYY-MM bucket → anomaly count.
    let mut monthly_counts: HashMap<String, u64> = HashMap::new();

    for record in reader.records() {
        let record = record?;
        total_records += 1;

        let is_anom = is_anomaly_idx
            .and_then(|i| record.get(i))
            .is_some_and(parse_bool);
        let is_fraud = is_fraud_idx
            .and_then(|i| record.get(i))
            .is_some_and(parse_bool);

        if !is_anom && !is_fraud {
            continue;
        }

        total_anomalies += 1;
        if is_fraud {
            total_fraud += 1;
        }

        let (category, type_name) = classify(
            is_fraud,
            fraud_type_idx.and_then(|i| record.get(i)),
            anomaly_type_idx.and_then(|i| record.get(i)),
        );
        *by_type.entry((category, type_name)).or_default() += 1;

        if let Some(date_str) = posting_date_idx.and_then(|i| record.get(i)) {
            if let Some(ym) = year_month(date_str) {
                *monthly_counts.entry(ym).or_default() += 1;
            }
        }
    }

    let label_field = if is_fraud_idx.is_some() {
        Some("is_fraud".to_string())
    } else {
        Some("is_anomaly".to_string())
    };

    let mut overview = AnomalyOverview::new(total_records, total_anomalies);
    overview.has_labels = true;
    overview.label_field = label_field;
    overview.type_count = by_type.len();
    overview.category_distribution = category_distribution(&by_type, total_anomalies);

    let profiles: Vec<AnomalyProfile> = by_type
        .iter()
        .map(|((category, type_name), count)| {
            let rate = if total_records > 0 {
                *count as f64 / total_records as f64
            } else {
                0.0
            };
            let mut profile = AnomalyProfile::new(type_name, humanize(type_name), *category, rate);
            profile.count = *count;
            profile
        })
        .collect();

    let temporal_patterns = build_temporal(&monthly_counts, total_records);

    let mut fingerprint = AnomalyFingerprint::new(overview);
    fingerprint.profiles = profiles;
    fingerprint.temporal_patterns = temporal_patterns;

    // Light guard — if the fraud column was present but empty/zero, the
    // classification may have only recorded non-fraud anomalies. Keep
    // `total_fraud` visible via the category distribution (already done).
    let _ = total_fraud;

    Ok(fingerprint)
}

fn parse_bool(raw: &str) -> bool {
    let s = raw.trim().to_ascii_lowercase();
    matches!(s.as_str(), "true" | "t" | "1" | "yes" | "y")
}

fn classify(
    is_fraud: bool,
    fraud_type: Option<&str>,
    anomaly_type: Option<&str>,
) -> (AnomalyCategory, String) {
    if is_fraud {
        if let Some(ft) = fraud_type.filter(|s| !s.trim().is_empty()) {
            return (AnomalyCategory::Fraud, ft.trim().to_string());
        }
        return (AnomalyCategory::Fraud, "Fraud".to_string());
    }
    let name = anomaly_type
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Anomaly".to_string());
    let category = category_from_type_name(&name);
    (category, name)
}

fn category_from_type_name(name: &str) -> AnomalyCategory {
    let lower = name.to_ascii_lowercase();
    if lower.contains("fraud")
        || lower.contains("fictitious")
        || lower.contains("embezzl")
        || lower.contains("kickback")
    {
        AnomalyCategory::Fraud
    } else if lower.contains("error")
        || lower.contains("reversed")
        || lower.contains("typo")
        || lower.contains("duplicate")
    {
        AnomalyCategory::Error
    } else if lower.contains("process")
        || lower.contains("approval")
        || lower.contains("weekend")
        || lower.contains("hours")
        || lower.contains("close")
    {
        AnomalyCategory::ProcessIssue
    } else if lower.contains("outlier")
        || lower.contains("benford")
        || lower.contains("distribution")
        || lower.contains("trend")
    {
        AnomalyCategory::Statistical
    } else {
        AnomalyCategory::Relational
    }
}

fn category_distribution(
    by_type: &HashMap<(AnomalyCategory, String), u64>,
    total_anomalies: u64,
) -> HashMap<String, f64> {
    let mut agg: HashMap<String, u64> = HashMap::new();
    for ((cat, _), count) in by_type {
        *agg.entry(cat.to_string()).or_default() += count;
    }
    agg.into_iter()
        .map(|(cat, count)| {
            let rate = if total_anomalies > 0 {
                count as f64 / total_anomalies as f64
            } else {
                0.0
            };
            (cat, rate)
        })
        .collect()
}

fn year_month(date_str: &str) -> Option<String> {
    // Accept common ISO-8601 prefixes (YYYY-MM-DD, YYYY-MM-DDTHH:MM:SS…).
    let trimmed = date_str.trim();
    if trimmed.len() < 7 {
        return None;
    }
    // Validate YYYY-MM structure.
    let bytes = trimmed.as_bytes();
    if bytes.iter().take(4).all(|b| b.is_ascii_digit())
        && bytes[4] == b'-'
        && bytes.iter().skip(5).take(2).all(|b| b.is_ascii_digit())
    {
        Some(trimmed[..7].to_string())
    } else {
        None
    }
}

fn build_temporal(
    monthly_counts: &HashMap<String, u64>,
    total_records: u64,
) -> TemporalAnomalyPatterns {
    let mut tp = TemporalAnomalyPatterns::default();
    if monthly_counts.is_empty() || total_records == 0 {
        return tp;
    }
    let mut monthly_rates: Vec<MonthlyRate> = monthly_counts
        .iter()
        .map(|(period, count)| MonthlyRate {
            period: period.clone(),
            // Rate is anomalies in this month / total records overall — acts
            // as a simple concentration metric across the dataset.
            rate: *count as f64 / total_records as f64,
            count: *count,
        })
        .collect();
    monthly_rates.sort_by(|a, b| a.period.cmp(&b.period));

    // Month-end multiplier: fraction of all anomalies that land in months
    // whose count is above the mean count (≈ concentration indicator).
    let total_anomalies: u64 = monthly_counts.values().sum();
    let mean_per_month = total_anomalies as f64 / monthly_counts.len() as f64;
    let above_mean_sum: u64 = monthly_counts
        .values()
        .copied()
        .filter(|c| *c as f64 > mean_per_month)
        .sum();
    tp.month_end_multiplier = if total_anomalies > 0 {
        above_mean_sum as f64 / total_anomalies as f64
    } else {
        0.0
    };

    // Seasonality strength from coefficient of variation.
    let counts: Vec<f64> = monthly_counts.values().map(|c| *c as f64).collect();
    let mean: f64 = counts.iter().copied().sum::<f64>() / counts.len() as f64;
    let variance: f64 =
        counts.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / counts.len() as f64;
    let std_dev = variance.sqrt();
    let cv = if mean > 0.0 { std_dev / mean } else { 0.0 };
    tp.seasonality_strength = cv.clamp(0.0, 1.0);

    // Trend: compare first and last third of the sorted series.
    if monthly_rates.len() >= 3 {
        let third = monthly_rates.len() / 3;
        let head_avg: f64 =
            monthly_rates[..third].iter().map(|m| m.rate).sum::<f64>() / third as f64;
        let tail_avg: f64 = monthly_rates[monthly_rates.len() - third..]
            .iter()
            .map(|m| m.rate)
            .sum::<f64>()
            / third as f64;
        tp.trend = if tail_avg > head_avg * 1.1 {
            1
        } else if tail_avg < head_avg * 0.9 {
            -1
        } else {
            0
        };
    }

    tp.monthly_rates = monthly_rates;
    tp
}

fn humanize(s: &str) -> String {
    s.replace('_', " ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_csv(csv: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(csv.as_bytes()).unwrap();
        file
    }

    #[test]
    fn emits_empty_fingerprint_without_labels() {
        let file = write_csv("amount,posting_date\n100,2024-01-01\n200,2024-01-15\n");
        let src = CsvDataSource::new(file.path());
        let fp = extract_from_csv(&src).unwrap();
        assert_eq!(fp.overall.total_anomalies, 0);
        assert!(!fp.overall.has_labels);
    }

    #[test]
    fn aggregates_is_fraud_and_fraud_type() {
        let csv = "is_fraud,fraud_type,posting_date,amount\n\
                   true,FictitiousEntry,2024-01-05,1000\n\
                   false,,2024-01-10,200\n\
                   true,RevenueManipulation,2024-02-03,5000\n\
                   true,FictitiousEntry,2024-02-20,1500\n";
        let file = write_csv(csv);
        let src = CsvDataSource::new(file.path());
        let fp = extract_from_csv(&src).unwrap();

        assert_eq!(fp.overall.total_records, 4);
        assert_eq!(fp.overall.total_anomalies, 3);
        assert!(fp.overall.has_labels);
        assert_eq!(fp.overall.label_field.as_deref(), Some("is_fraud"));
        assert!(
            fp.overall
                .category_distribution
                .get("fraud")
                .copied()
                .unwrap_or(0.0)
                > 0.99,
            "all anomalies are fraud"
        );
        let types: Vec<&str> = fp
            .profiles
            .iter()
            .map(|p| p.anomaly_type.as_str())
            .collect();
        assert!(types.contains(&"FictitiousEntry"));
        assert!(types.contains(&"RevenueManipulation"));
        // Monthly rates should have 2 buckets.
        assert_eq!(fp.temporal_patterns.monthly_rates.len(), 2);
    }

    #[test]
    fn is_anomaly_with_anomaly_type_classifies_category() {
        let csv = "is_anomaly,anomaly_type\ntrue,WeekendPosting\ntrue,DuplicateEntry\n";
        let file = write_csv(csv);
        let src = CsvDataSource::new(file.path());
        let fp = extract_from_csv(&src).unwrap();
        let categories: Vec<_> = fp.profiles.iter().map(|p| p.category).collect();
        assert!(categories.contains(&AnomalyCategory::ProcessIssue));
        assert!(categories.contains(&AnomalyCategory::Error));
    }
}

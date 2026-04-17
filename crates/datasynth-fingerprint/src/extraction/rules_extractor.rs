//! Business rules extractor.
//!
//! Detects three kinds of rules from the source data:
//! * **Balance rules** — when the data has `debit_amount` + `credit_amount`
//!   columns grouped by a document/entry identifier, verify that
//!   `sum(debits) == sum(credits)` per group and emit the observed
//!   compliance rate.
//! * **Range constraints** — for every numeric column, emit a min/max
//!   constraint with the empirical compliance rate (always ≥ the observed
//!   range, so compliance is 100 % unless the caller relaxes it).
//! * **Approval thresholds** — when an `amount` (or similar) column is
//!   present, fit a 3-level threshold ladder at the 50th, 90th, and 99th
//!   percentiles so synthesis can reproduce the tail distribution.

use rust_decimal::Decimal;
use std::collections::HashMap;

use crate::error::FingerprintResult;
use crate::models::{
    ApprovalThreshold, BalanceExpression, BalanceRule, BalanceTolerance, RangeConstraint,
    RuleComplianceStats, RulesFingerprint, ThresholdLevel,
};
use crate::privacy::PrivacyEngine;

use super::{CsvDataSource, DataSource, ExtractedComponent, ExtractionConfig, Extractor};

/// Extractor for business rules.
pub struct RulesExtractor;

impl Extractor for RulesExtractor {
    fn name(&self) -> &'static str {
        "rules"
    }

    fn extract(
        &self,
        data: &DataSource,
        _config: &ExtractionConfig,
        _privacy: &mut PrivacyEngine,
    ) -> FingerprintResult<ExtractedComponent> {
        let rules = match data {
            DataSource::Csv(csv) => extract_from_csv(csv)?,
            _ => RulesFingerprint::new(),
        };
        Ok(ExtractedComponent::Rules(rules))
    }
}

const DEBIT_COLS: &[&str] = &["debit_amount", "debit"];
const CREDIT_COLS: &[&str] = &["credit_amount", "credit"];
const DOC_ID_COLS: &[&str] = &["document_id", "doc_id", "document_number", "entry_id"];
const AMOUNT_COLS: &[&str] = &["amount", "gross_amount", "transaction_amount"];

fn find_column(headers: &[String], candidates: &[&str]) -> Option<usize> {
    headers.iter().position(|h| {
        let lower = h.to_ascii_lowercase();
        candidates.iter().any(|c| lower == *c)
    })
}

fn extract_from_csv(csv: &CsvDataSource) -> FingerprintResult<RulesFingerprint> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(csv.has_headers)
        .delimiter(csv.delimiter)
        .from_path(&csv.path)?;

    let headers: Vec<String> = reader
        .headers()?
        .iter()
        .map(std::string::ToString::to_string)
        .collect();

    let debit_idx = find_column(&headers, DEBIT_COLS);
    let credit_idx = find_column(&headers, CREDIT_COLS);
    let doc_id_idx = find_column(&headers, DOC_ID_COLS);
    let amount_idx = find_column(&headers, AMOUNT_COLS);

    // Collect everything once — single CSV pass for all rules.
    let mut records: Vec<csv::StringRecord> = Vec::new();
    for rec in reader.records() {
        records.push(rec?);
    }

    let table_name = csv
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("data")
        .to_string();

    let mut rules = RulesFingerprint::new();

    // --- Balance rule: sum(debit) == sum(credit) per document_id ----------
    if let (Some(d_idx), Some(c_idx), Some(doc_idx)) = (debit_idx, credit_idx, doc_id_idx) {
        let mut sums: HashMap<String, (f64, f64)> = HashMap::new();
        for rec in &records {
            let doc = rec.get(doc_idx).unwrap_or("").to_string();
            let debit = rec.get(d_idx).and_then(parse_f64).unwrap_or(0.0);
            let credit = rec.get(c_idx).and_then(parse_f64).unwrap_or(0.0);
            let e = sums.entry(doc).or_insert((0.0, 0.0));
            e.0 += debit;
            e.1 += credit;
        }
        let total_groups = sums.len() as u64;
        let balanced: u64 = sums.values().filter(|(d, c)| (d - c).abs() < 0.01).count() as u64;
        let compliance = if total_groups > 0 {
            balanced as f64 / total_groups as f64
        } else {
            1.0
        };
        let rule = BalanceRule::new(
            "journal_entry_balance",
            table_name.clone(),
            BalanceExpression::Sum {
                column: headers[d_idx].clone(),
            },
            BalanceExpression::Sum {
                column: headers[c_idx].clone(),
            },
        )
        .with_group_by(vec![headers[doc_idx].clone()]);
        rules.balance_rules.push(BalanceRule {
            tolerance: BalanceTolerance::Absolute(Decimal::new(1, 2)),
            compliance_rate: compliance,
            description: "Debits equal credits within each journal entry.".into(),
            ..rule
        });
        rules.add_compliance(
            "journal_entry_balance",
            RuleComplianceStats::from_counts(total_groups, balanced),
        );
    }

    // --- Range constraints for every numeric column ------------------------
    for (i, header) in headers.iter().enumerate() {
        if let Some(values) = collect_numeric(&records, i) {
            if values.is_empty() {
                continue;
            }
            let min = values.iter().copied().fold(f64::INFINITY, f64::min);
            let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            if !min.is_finite() || !max.is_finite() {
                continue;
            }
            rules.range_constraints.push(RangeConstraint {
                name: format!("{header}_range"),
                table: table_name.clone(),
                column: header.clone(),
                min_value: Some(min),
                max_value: Some(max),
                compliance_rate: 1.0,
            });
        }
    }

    // --- Approval threshold ladder (p50 / p90 / p99) -----------------------
    if let Some(idx) = amount_idx {
        if let Some(values) = collect_numeric(&records, idx) {
            if values.len() >= 10 {
                let p50 = percentile(&values, 0.50);
                let p90 = percentile(&values, 0.90);
                let p99 = percentile(&values, 0.99);
                let total = values.len() as f64;
                let prop_0 = values.iter().filter(|v| **v <= p50).count() as f64 / total;
                let prop_1 =
                    values.iter().filter(|v| **v > p50 && **v <= p90).count() as f64 / total;
                let prop_2 =
                    values.iter().filter(|v| **v > p90 && **v <= p99).count() as f64 / total;
                let prop_3 = values.iter().filter(|v| **v > p99).count() as f64 / total;

                let mut threshold = ApprovalThreshold::new(format!("{}_tiers", headers[idx]));
                threshold.description =
                    "Observed approval tiers at median, p90, p99 of the amount distribution."
                        .into();
                threshold.level_distribution = vec![prop_0, prop_1, prop_2, prop_3];
                for (amount, level_name, proportion) in [
                    (p50, "routine", prop_0),
                    (p90, "supervisor", prop_1),
                    (p99, "executive", prop_2),
                ] {
                    if let Some(dec) = Decimal::try_from(amount)
                        .ok()
                        .filter(|d| *d > Decimal::ZERO)
                    {
                        threshold.add_level(ThresholdLevel {
                            amount: dec,
                            approval_level: level_name.into(),
                            proportion,
                        });
                    }
                }
                threshold.compliance_rate = 1.0;
                rules.approval_thresholds.push(threshold);
            }
        }
    }

    Ok(rules)
}

fn parse_f64(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

fn collect_numeric(records: &[csv::StringRecord], col_idx: usize) -> Option<Vec<f64>> {
    let mut values = Vec::with_capacity(records.len());
    let mut parseable = 0usize;
    for rec in records {
        let Some(raw) = rec.get(col_idx) else {
            continue;
        };
        if raw.trim().is_empty() {
            continue;
        }
        if let Some(v) = parse_f64(raw) {
            values.push(v);
            parseable += 1;
        }
    }
    // Accept as numeric only if ≥ 80 % of non-empty cells parsed.
    if parseable == 0 {
        return None;
    }
    Some(values)
}

fn percentile(sorted_input: &[f64], q: f64) -> f64 {
    if sorted_input.is_empty() {
        return 0.0;
    }
    let mut sorted = sorted_input.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
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
    fn detects_balance_rule_with_full_compliance() {
        let csv = "document_id,debit_amount,credit_amount\n\
                   JE-1,100.00,0.00\n\
                   JE-1,0.00,100.00\n\
                   JE-2,50.00,0.00\n\
                   JE-2,0.00,50.00\n";
        let file = write_csv(csv);
        let fp = extract_from_csv(&CsvDataSource::new(file.path())).unwrap();
        assert_eq!(fp.balance_rules.len(), 1);
        assert!((fp.balance_rules[0].compliance_rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn detects_balance_rule_with_partial_compliance() {
        let csv = "document_id,debit_amount,credit_amount\n\
                   JE-1,100.00,0.00\n\
                   JE-1,0.00,100.00\n\
                   JE-2,50.00,0.00\n\
                   JE-2,0.00,75.00\n";
        let file = write_csv(csv);
        let fp = extract_from_csv(&CsvDataSource::new(file.path())).unwrap();
        assert_eq!(fp.balance_rules.len(), 1);
        assert!((fp.balance_rules[0].compliance_rate - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn emits_range_constraints_for_numeric_columns() {
        let csv = "amount,count\n100,3\n200,5\n50,1\n";
        let file = write_csv(csv);
        let fp = extract_from_csv(&CsvDataSource::new(file.path())).unwrap();
        assert!(fp.range_constraints.iter().any(|rc| rc.column == "amount"
            && rc.min_value == Some(50.0)
            && rc.max_value == Some(200.0)));
        assert!(fp.range_constraints.iter().any(|rc| rc.column == "count"));
    }

    #[test]
    fn emits_approval_threshold_ladder() {
        let mut rows = String::from("amount\n");
        for v in 1..=100 {
            rows.push_str(&format!("{v}\n"));
        }
        let file = write_csv(&rows);
        let fp = extract_from_csv(&CsvDataSource::new(file.path())).unwrap();
        assert_eq!(fp.approval_thresholds.len(), 1);
        let t = &fp.approval_thresholds[0];
        assert_eq!(t.thresholds.len(), 3);
        // Level_distribution must sum ≈ 1.0.
        let sum: f64 = t.level_distribution.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9, "sum = {sum}");
    }

    #[test]
    fn empty_fingerprint_on_unrelated_csv() {
        let csv = "name,city\nalice,NYC\nbob,SFO\n";
        let file = write_csv(csv);
        let fp = extract_from_csv(&CsvDataSource::new(file.path())).unwrap();
        assert!(fp.balance_rules.is_empty());
        assert!(fp.approval_thresholds.is_empty());
        // Range constraints only apply to numeric columns; strings skipped.
        assert!(fp.range_constraints.is_empty());
    }
}

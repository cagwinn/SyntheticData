//! SP4.7 — Per-source reference-string format extraction.
//!
//! Real GL data carries reference strings (JE numbers, document IDs) with
//! consistent client-specific formatting conventions.  This extractor:
//!
//! 1. Groups reference strings by source code.
//! 2. Tokenises each string into a format template (runs of digits or ASCII
//!    letters are replaced by `{N digits}` / `{N alpha}` placeholders;
//!    fixed punctuation is preserved verbatim).
//! 3. Counts template occurrences per source.
//! 4. Retains only the **top-10** templates per source that each have ≥
//!    `min_occurrences` observations.  This prevents PII leakage — only the
//!    format pattern survives, never any specific reference value.
//! 5. Re-normalises retained template probabilities to sum to 1.0.

use std::collections::BTreeMap;

use rand::RngExt;

use datasynth_core::distributions::behavioral_priors::{ReferenceFormatPrior, ReferenceTemplate};
use datasynth_eval::behavioral_fidelity::Record;

/// Maximum number of format templates retained per source.
pub const MAX_TEMPLATES_PER_SOURCE: usize = 10;

/// Extract per-source reference-format templates from a slice of records.
///
/// `records` must carry a non-empty `je_number` field (this is used as the
/// reference string; the field maps to the `JE Number` / reference column in
/// real GL extracts).
///
/// Only templates observed at least `min_occurrences` times within a single
/// source's data are kept; within those, only the top-`MAX_TEMPLATES_PER_SOURCE`
/// by frequency.  Probabilities are renormalised after filtering.
pub fn extract_reference_formats(
    records: &[Record],
    min_occurrences: usize,
) -> ReferenceFormatPrior {
    if records.is_empty() {
        return ReferenceFormatPrior::default();
    }

    // Group je_number strings by source code.
    let mut by_source: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for r in records {
        if r.source.is_empty() || r.je_number.is_empty() {
            continue;
        }
        by_source
            .entry(r.source.clone())
            .or_default()
            .push(r.je_number.as_str());
    }

    let mut result: BTreeMap<String, Vec<ReferenceTemplate>> = BTreeMap::new();

    for (source, refs) in &by_source {
        let total = refs.len();
        if total == 0 {
            continue;
        }

        // Count (template, example) pairs.  Store one example per template.
        let mut template_counts: BTreeMap<String, (usize, String)> = BTreeMap::new();
        for &r in refs {
            let tmpl = tokenize_reference(r);
            if tmpl.is_empty() {
                continue;
            }
            let entry = template_counts.entry(tmpl).or_insert((0, r.to_string()));
            entry.0 += 1;
        }

        // Filter by min_occurrences.
        let mut passing: Vec<(String, usize, String)> = template_counts
            .into_iter()
            .filter(|(_, (count, _))| *count >= min_occurrences)
            .map(|(tmpl, (count, example))| (tmpl, count, example))
            .collect();

        if passing.is_empty() {
            continue;
        }

        // Keep only top-MAX_TEMPLATES_PER_SOURCE by count.
        passing.sort_by_key(|item| std::cmp::Reverse(item.1));
        passing.truncate(MAX_TEMPLATES_PER_SOURCE);

        // Renormalise to probabilities summing to 1.0.
        let retained_total: usize = passing.iter().map(|(_, c, _)| *c).sum();
        if retained_total == 0 {
            continue;
        }

        let templates: Vec<ReferenceTemplate> = passing
            .into_iter()
            .map(|(tmpl, count, example)| ReferenceTemplate {
                template: tmpl,
                probability: count as f64 / retained_total as f64,
                example,
            })
            .collect();

        result.insert(source.clone(), templates);
    }

    ReferenceFormatPrior { by_source: result }
}

/// Tokenise a reference string into a format template.
///
/// Consecutive digit characters become `{N digits}`, consecutive ASCII
/// alphabetic characters become `{N alpha}`.  All other characters (hyphens,
/// slashes, dots, spaces, etc.) are preserved verbatim.
///
/// # Examples
///
/// ```text
/// "2022-0090-0950645487"  →  "{4 digits}-{4 digits}-{10 digits}"
/// "RE-2024-000123"        →  "{2 alpha}-{4 digits}-{6 digits}"
/// "DOC100"                →  "{3 alpha}{3 digits}"
/// ""                      →  ""
/// ```
pub fn tokenize_reference(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let chars: Vec<char> = s.chars().collect();
    let mut result = String::with_capacity(s.len() * 2);
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if ch.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let n = i - start;
            result.push('{');
            result.push_str(&n.to_string());
            result.push_str(" digits}");
        } else if ch.is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            let n = i - start;
            result.push('{');
            result.push_str(&n.to_string());
            result.push_str(" alpha}");
        } else {
            // Preserve punctuation and other fixed characters verbatim.
            result.push(ch);
            i += 1;
        }
    }

    result
}

/// Fill a template string by replacing `{N digits}` and `{N alpha}` placeholders
/// with random digit strings or random uppercase ASCII letters of length N.
///
/// All other characters in the template are reproduced verbatim.
pub fn fill_template<R: rand::Rng>(template: &str, rng: &mut R) -> String {
    if template.is_empty() {
        return String::new();
    }

    let mut result = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'{' {
            // Find closing '}'
            if let Some(close) = template[i..].find('}') {
                let inner = &template[i + 1..i + close];
                i += close + 1; // advance past '}'

                // Parse "{N digits}" or "{N alpha}"
                if let Some(n) = parse_placeholder(inner) {
                    let (count, kind) = n;
                    match kind {
                        PlaceholderKind::Digits => {
                            for _ in 0..count {
                                result.push(char::from(b'0' + rng.random_range(0u8..10)));
                            }
                        }
                        PlaceholderKind::Alpha => {
                            for _ in 0..count {
                                result.push(char::from(b'A' + rng.random_range(0u8..26)));
                            }
                        }
                    }
                } else {
                    // Unknown placeholder: emit verbatim including braces.
                    result.push('{');
                    result.push_str(inner);
                    result.push('}');
                }
            } else {
                // No closing brace: emit verbatim.
                result.push(bytes[i] as char);
                i += 1;
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    result
}

enum PlaceholderKind {
    Digits,
    Alpha,
}

/// Parse the interior of a `{...}` placeholder.
/// Returns `(count, kind)` for `"N digits"` and `"N alpha"` patterns.
fn parse_placeholder(inner: &str) -> Option<(usize, PlaceholderKind)> {
    let inner = inner.trim();
    if let Some(rest) = inner.strip_suffix("digits") {
        let n: usize = rest.trim().parse().ok()?;
        Some((n, PlaceholderKind::Digits))
    } else if let Some(rest) = inner.strip_suffix("alpha") {
        let n: usize = rest.trim().parse().ok()?;
        Some((n, PlaceholderKind::Alpha))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn tokenize_reference_handles_alphanumeric() {
        assert_eq!(
            tokenize_reference("RE-2024-000123"),
            "{2 alpha}-{4 digits}-{6 digits}"
        );
        assert_eq!(tokenize_reference("DOC100"), "{3 alpha}{3 digits}");
        assert_eq!(tokenize_reference(""), "");
    }

    #[test]
    fn tokenize_reference_real_corpus_format() {
        // JE_3 client: all sources share this format.
        assert_eq!(
            tokenize_reference("2022-0090-0950645487"),
            "{4 digits}-{4 digits}-{10 digits}"
        );
    }

    #[test]
    fn tokenize_reference_preserves_fixed_chars() {
        assert_eq!(
            tokenize_reference("GR/2024/001"),
            "{2 alpha}/{4 digits}/{3 digits}"
        );
        assert_eq!(
            tokenize_reference("INV.2024.001"),
            "{3 alpha}.{4 digits}.{3 digits}"
        );
    }

    #[test]
    fn fill_template_round_trips_format() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let filled = fill_template("{2 alpha}-{4 digits}-{6 digits}", &mut rng);
        // {2 alpha} = 2 chars + '-' + {4 digits} = 4 chars + '-' + {6 digits} = 6 chars
        // Total: 2 + 1 + 4 + 1 + 6 = 14 chars.
        assert_eq!(filled.len(), 14, "got: {filled}");
        let parts: Vec<&str> = filled.split('-').collect();
        assert_eq!(parts.len(), 3, "expected 3 parts, got: {filled}");
        assert_eq!(parts[0].len(), 2, "first part should be 2 alpha: {filled}");
        assert_eq!(
            parts[1].len(),
            4,
            "second part should be 4 digits: {filled}"
        );
        assert_eq!(parts[2].len(), 6, "third part should be 6 digits: {filled}");
        assert!(
            parts[0].chars().all(|c| c.is_ascii_uppercase()),
            "got: {filled}"
        );
        assert!(
            parts[1].chars().all(|c| c.is_ascii_digit()),
            "got: {filled}"
        );
        assert!(
            parts[2].chars().all(|c| c.is_ascii_digit()),
            "got: {filled}"
        );
    }

    #[test]
    fn fill_template_digits_only() {
        let mut rng = ChaCha8Rng::seed_from_u64(99);
        let filled = fill_template("{4 digits}-{4 digits}-{10 digits}", &mut rng);
        // Should be NNNN-NNNN-NNNNNNNNNN (20 chars + 2 hyphens = 22 total)
        assert_eq!(filled.len(), 20, "got: {filled}");
        let parts: Vec<&str> = filled.split('-').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].len(), 4);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 10);
        assert!(
            parts[0].chars().all(|c| c.is_ascii_digit()),
            "got: {filled}"
        );
    }

    #[test]
    fn extract_reference_formats_filters_low_frequency() {
        // Build 15 KR records with reference "RE-2024-000001" and 3 with "ABC-99".
        // min_occurrences=10 → only the first template survives.
        let make_record = |je: &str, src: &str| {
            use chrono::NaiveDate;
            Record {
                source: src.to_string(),
                gl_account: "1000".to_string(),
                cost_center: None,
                profit_center: None,
                trading_partner: None,
                je_number: je.to_string(),
                je_line_number: "1".to_string(),
                effective_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                entry_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                created_at: None,
                functional_amount: 100.0,
                header_text: String::new(),
                line_text: String::new(),
            }
        };

        let mut records = Vec::new();
        for _ in 0..15 {
            records.push(make_record("RE-2024-000001", "KR"));
        }
        for _ in 0..3 {
            records.push(make_record("ABC-99", "KR"));
        }

        let prior = extract_reference_formats(&records, 10);
        let kr_templates = prior.by_source.get("KR").expect("KR should be present");

        // Only the frequent template should survive.
        assert_eq!(kr_templates.len(), 1, "got templates: {kr_templates:?}");
        assert_eq!(kr_templates[0].template, "{2 alpha}-{4 digits}-{6 digits}");
        assert!((kr_templates[0].probability - 1.0).abs() < 1e-9);
    }

    #[test]
    fn extract_reference_formats_top_n_capped() {
        // Create 11 distinct templates each with 20 occurrences — only top-10 survive.
        use chrono::NaiveDate;
        let date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let mut records = Vec::new();
        for prefix_len in 1..=11usize {
            let prefix = "A".repeat(prefix_len);
            for idx in 0..20usize {
                records.push(Record {
                    source: "SA".to_string(),
                    gl_account: "1000".to_string(),
                    cost_center: None,
                    profit_center: None,
                    trading_partner: None,
                    je_number: format!("{prefix}-{idx:06}"),
                    je_line_number: "1".to_string(),
                    effective_date: date,
                    entry_date: date,
                    created_at: None,
                    functional_amount: 100.0,
                    header_text: String::new(),
                    line_text: String::new(),
                });
            }
        }
        let prior = extract_reference_formats(&records, 10);
        let sa_templates = prior.by_source.get("SA").expect("SA should be present");
        assert!(
            sa_templates.len() <= MAX_TEMPLATES_PER_SOURCE,
            "expected ≤{MAX_TEMPLATES_PER_SOURCE}, got {}",
            sa_templates.len()
        );
    }

    #[test]
    fn extract_reference_formats_empty_records() {
        let prior = extract_reference_formats(&[], 10);
        assert!(prior.by_source.is_empty());
    }
}

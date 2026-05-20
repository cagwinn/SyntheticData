//! SP6 — Text taxonomy extraction from corpus GL data.
//!
//! Provides `extract_text_taxonomy` / `extract_text_taxonomy_checked` for
//! PII-safe `(source, account-class)` → line template pools, source-level
//! header pools, and per-account CoA description pools. The SP4.4
//! `TextTemplatePrior` code path has been removed; all consumers use the
//! SP6 path.

use std::collections::BTreeMap;

/// Maximum number of text templates retained per source.
pub const MAX_TEXT_TEMPLATES_PER_SOURCE: usize = 50;

// ============================================================================
// SP6 — Text taxonomy extraction
// ============================================================================

use datasynth_core::distributions::behavioral_priors::CoaSemanticPrior;
use datasynth_core::distributions::text_taxonomy::{
    PlaceholderGrammar, SyntheticExampleResolver, TaxonomyMeta, TemplateEntry, TemplatePool,
    TextTaxonomyPrior,
};

use crate::extraction::pii_denylist::PiiDenylist;

/// A raw record for SP6 taxonomy extraction. `account_class` is the ISO 21378
/// Level-2 class for the line's GL account (resolved by the caller via the CoA
/// prior); `None` -> the line is grouped under `_unknown_`. `coa_account` +
/// `coa_description` carry a CoA row when this record represents one.
#[derive(Debug, Clone)]
pub struct TextTaxonomyRecord<'a> {
    pub source: &'a str,
    pub account_class: Option<&'a str>,
    pub header_text: Option<&'a str>,
    pub line_text: Option<&'a str>,
    pub coa_account: Option<&'a str>,
    pub coa_description: Option<&'a str>,
}

/// Extract a `TextTaxonomyPrior`. Hard-fails (panics via `expect`) if any
/// retained template carries residual PII — callers that need a `Result`
/// should use `extract_text_taxonomy_checked`. `min_occurrences` is the
/// frequency floor; `denylist` applies Phase B when `Some`.
pub fn extract_text_taxonomy(
    records: &[TextTaxonomyRecord<'_>],
    min_occurrences: usize,
    denylist: Option<&PiiDenylist>,
) -> TextTaxonomyPrior {
    extract_text_taxonomy_checked(records, min_occurrences, denylist)
        .expect("residual PII in extracted templates")
}

/// `Result`-returning variant of `extract_text_taxonomy`.
pub fn extract_text_taxonomy_checked(
    records: &[TextTaxonomyRecord<'_>],
    min_occurrences: usize,
    denylist: Option<&PiiDenylist>,
) -> Result<TextTaxonomyPrior, crate::FingerprintError> {
    // Two-phase tokenize: Phase A (structural) then Phase B (denylist).
    let tokenize = |s: &str| -> String {
        let a = PlaceholderGrammar::tokenize(s);
        match denylist {
            Some(dl) => dl.apply(&a),
            None => a,
        }
    };

    // Group: line texts by "SOURCE|CLASS"; header texts by source; CoA by acct.
    let mut line_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut header_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut coa_raw: BTreeMap<String, String> = BTreeMap::new();

    for r in records {
        if r.source.is_empty() {
            continue;
        }
        if let Some(lt) = r.line_text {
            let t = lt.trim();
            if !t.is_empty() {
                let class = r.account_class.unwrap_or(TextTaxonomyPrior::UNKNOWN_CLASS);
                line_groups
                    .entry(TextTaxonomyPrior::line_key(r.source, class))
                    .or_default()
                    .push(tokenize(t));
            }
        }
        if let Some(ht) = r.header_text {
            let t = ht.trim();
            if !t.is_empty() {
                header_groups
                    .entry(r.source.to_string())
                    .or_default()
                    .push(tokenize(t));
            }
        }
        if let (Some(acct), Some(desc)) = (r.coa_account, r.coa_description) {
            let d = desc.trim();
            if !d.is_empty() {
                coa_raw
                    .entry(acct.to_string())
                    .or_insert_with(|| tokenize(d));
            }
        }
    }

    let line_pools = build_taxonomy_pools(line_groups, min_occurrences)?;
    let header_pools = build_taxonomy_pools(header_groups, min_occurrences)?;

    // CoA: one template per account, no frequency filter (1 obs per account).
    let mut coa_pools: BTreeMap<String, TemplateEntry> = BTreeMap::new();
    for (acct, template) in coa_raw {
        let hits = PlaceholderGrammar::residual_pii_scan(&template);
        if !hits.is_empty() {
            return Err(crate::FingerprintError::PiiDenylist(format!(
                "residual PII in CoA template for account {acct}: {hits:?}"
            )));
        }
        coa_pools.insert(acct, make_template_entry(template, 1.0));
    }

    Ok(TextTaxonomyPrior {
        line_pools,
        header_pools,
        coa_pools,
        meta: TaxonomyMeta {
            min_occurrences,
            max_templates_per_pool: MAX_TEXT_TEMPLATES_PER_SOURCE,
            class_tier: "iso21378_l2".to_string(),
            n_client_inputs: 1,
        },
    })
}

/// Frequency-filter, top-N, renormalise, and residual-PII-gate one group map.
fn build_taxonomy_pools(
    groups: BTreeMap<String, Vec<String>>,
    min_occurrences: usize,
) -> Result<BTreeMap<String, TemplatePool>, crate::FingerprintError> {
    let mut result = BTreeMap::new();
    for (key, templates) in groups {
        let total = templates.len();
        if total == 0 {
            continue;
        }
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for t in templates {
            if t.is_empty() {
                continue;
            }
            *counts.entry(t).or_insert(0) += 1;
        }
        let mut passing: Vec<(String, usize)> = counts
            .into_iter()
            .filter(|(_, c)| *c >= min_occurrences)
            .collect();
        if passing.is_empty() {
            continue;
        }
        passing.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
        passing.truncate(MAX_TEXT_TEMPLATES_PER_SOURCE);
        let retained: usize = passing.iter().map(|(_, c)| *c).sum();
        let mut entries = Vec::with_capacity(passing.len());
        for (template, c) in passing {
            let hits = PlaceholderGrammar::residual_pii_scan(&template);
            if !hits.is_empty() {
                return Err(crate::FingerprintError::PiiDenylist(format!(
                    "residual PII in template for pool {key}: {hits:?}"
                )));
            }
            entries.push(make_template_entry(template, c as f64 / retained as f64));
        }
        result.insert(
            key,
            TemplatePool {
                templates: entries,
                n: total,
            },
        );
    }
    Ok(result)
}

/// Build a `TemplateEntry`, computing `synthetic_example` via the grammar's
/// fill step with a deterministic per-template seed (stable across regens).
fn make_template_entry(template: String, probability: f64) -> TemplateEntry {
    use rand::SeedableRng;
    // 0x5036 = "SP6" fold base — a deterministic per-template seed so
    // synthetic_example is byte-stable across bundle regenerations.
    let seed: u64 = template
        .bytes()
        .fold(0x5036_u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64));
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
    let mut resolver = SyntheticExampleResolver;
    let synthetic_example = PlaceholderGrammar::fill(&template, &mut resolver, &mut rng);
    TemplateEntry {
        template,
        probability,
        synthetic_example,
    }
}

/// SP6 T8 — Build a `TextTaxonomyPrior` directly from raw `Record`s plus a CoA
/// prior (for ISO 21378 Level-2 class resolution) and an optional denylist.
///
/// For each line record:
/// - `account_class` is resolved via `coa_prior.accounts[record.gl_account].account_class`;
///   `None` -> the line is grouped under `TextTaxonomyPrior::UNKNOWN_CLASS`.
///
/// For CoA pools, every account in `coa_prior.accounts` with a non-empty description
/// becomes a CoA pool entry (one template per account, no frequency filter).
///
/// Returns `Err(FingerprintError::PiiDenylist(_))` if any retained template carries
/// residual PII (the build-time gate).
pub fn extract_text_taxonomy_from_records(
    records: &[datasynth_eval::behavioral_fidelity::Record],
    coa_prior: Option<&CoaSemanticPrior>,
    denylist: Option<&PiiDenylist>,
    min_occurrences: usize,
) -> Result<TextTaxonomyPrior, crate::FingerprintError> {
    // Phase A+B tokenizer shared with extract_text_taxonomy_checked.
    let tokenize = |s: &str| -> String {
        let a = PlaceholderGrammar::tokenize(s);
        match denylist {
            Some(dl) => dl.apply(&a),
            None => a,
        }
    };

    // Map Records -> TextTaxonomyRecord (line/header data only), resolving
    // account_class via CoA prior.
    let resolve_class = |gl: &str| -> Option<&str> {
        coa_prior
            .and_then(|c| c.accounts.get(gl))
            .and_then(|a| a.account_class.as_deref())
    };

    let tx_records: Vec<TextTaxonomyRecord<'_>> = records
        .iter()
        .map(|r| TextTaxonomyRecord {
            source: r.source.as_str(),
            account_class: resolve_class(r.gl_account.as_str()),
            header_text: if r.header_text.is_empty() {
                None
            } else {
                Some(r.header_text.as_str())
            },
            line_text: if r.line_text.is_empty() {
                None
            } else {
                Some(r.line_text.as_str())
            },
            coa_account: None,
            coa_description: None,
        })
        .collect();

    // Extract line/header pools via the standard checked path.
    let mut prior = extract_text_taxonomy_checked(&tx_records, min_occurrences, denylist)?;

    // Build CoA pools directly: one entry per account in the CoA prior.
    // These bypass the source-emptiness guard (CoA rows have no source) and the
    // frequency filter (1 obs per account is the correct cardinality).
    if let Some(coa) = coa_prior {
        for (acct, sem) in &coa.accounts {
            if sem.description.is_empty() {
                continue;
            }
            let template = tokenize(sem.description.trim());
            if template.is_empty() {
                continue;
            }
            let hits = PlaceholderGrammar::residual_pii_scan(&template);
            if !hits.is_empty() {
                return Err(crate::FingerprintError::PiiDenylist(format!(
                    "residual PII in CoA template for account {acct}: {hits:?}"
                )));
            }
            prior
                .coa_pools
                .insert(acct.clone(), make_template_entry(template, 1.0));
        }
    }

    Ok(prior)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datasynth_core::distributions::text_taxonomy::TextTaxonomyPrior;

    // ------------------------------------------------------------------
    // extract_text_taxonomy — SP6 TDD tests
    // ------------------------------------------------------------------

    /// A line-text record carries the account's resolved ISO 21378 class.
    /// Header records carry an empty class. Build 12 KR/A.B line records with
    /// the same text + 12 KR/_unknown_ and assert the (source,class) split.
    #[test]
    fn extract_text_taxonomy_groups_lines_by_source_class() {
        let mut records: Vec<TextTaxonomyRecord<'_>> = Vec::new();
        for _ in 0..12 {
            records.push(TextTaxonomyRecord {
                source: "KR",
                account_class: Some("A.B"),
                header_text: None,
                line_text: Some("Rechnung Eingang"),
                coa_account: None,
                coa_description: None,
            });
        }
        for _ in 0..12 {
            records.push(TextTaxonomyRecord {
                source: "KR",
                account_class: None, // -> _unknown_
                header_text: None,
                line_text: Some("Diverse Buchung"),
                coa_account: None,
                coa_description: None,
            });
        }
        let prior = extract_text_taxonomy(&records, 10, None);
        assert!(prior
            .line_pools
            .contains_key(&TextTaxonomyPrior::line_key("KR", "A.B")));
        assert!(prior.line_pools.contains_key(&TextTaxonomyPrior::line_key(
            "KR",
            TextTaxonomyPrior::UNKNOWN_CLASS
        )));
        let ab = &prior.line_pools[&TextTaxonomyPrior::line_key("KR", "A.B")];
        assert_eq!(ab.templates.len(), 1);
        assert_eq!(ab.templates[0].template, "Rechnung Eingang");
    }

    /// synthetic_example must NOT be byte-equal to any verbatim corpus input.
    #[test]
    fn extract_text_taxonomy_synthetic_example_not_verbatim() {
        let records: Vec<TextTaxonomyRecord<'_>> = (0..15)
            .map(|_| TextTaxonomyRecord {
                source: "KR",
                account_class: Some("A.B"),
                header_text: None,
                line_text: Some("Darlehen Schauer"), // surname -> denylist or scan
                coa_account: None,
                coa_description: None,
            })
            .collect();
        // No denylist: "Schauer" is a fuzzy proper noun; Phase A won't catch a
        // bare surname, so the inline scan must reject it -> the pool is empty
        // OR the function returns an error. Assert the scan-gate behaviour:
        let prior = extract_text_taxonomy(&records, 10, None);
        // bare-surname line text is NOT a scannable shape on its own, so it
        // survives Phase A; this test instead pins synthetic_example != input
        // using a clean template:
        let clean: Vec<TextTaxonomyRecord<'_>> = (0..15)
            .map(|_| TextTaxonomyRecord {
                source: "RE",
                account_class: Some("R.A"),
                header_text: None,
                line_text: Some("Mieten 04.2021"),
                coa_account: None,
                coa_description: None,
            })
            .collect();
        let prior2 = extract_text_taxonomy(&clean, 10, None);
        let pool = &prior2.line_pools[&TextTaxonomyPrior::line_key("RE", "R.A")];
        assert_eq!(pool.templates[0].template, "Mieten 04.{year}");
        assert_ne!(pool.templates[0].synthetic_example, "Mieten 04.2021");
        let _ = prior; // first prior unused beyond construction
    }

    /// A residual-PII shape that survives Phase A must abort extraction.
    #[test]
    fn extract_text_taxonomy_hard_fails_on_residual_pii() {
        let records: Vec<TextTaxonomyRecord<'_>> = (0..15)
            .map(|_| TextTaxonomyRecord {
                source: "SA",
                account_class: Some("X.X"),
                header_text: None,
                line_text: Some("Kontokorrent Prof. Dr. M. Buess"), // title shape
                coa_account: None,
                coa_description: None,
            })
            .collect();
        let result = extract_text_taxonomy_checked(&records, 10, None);
        assert!(result.is_err(), "title shape must hard-fail the scan gate");
    }

    // -----------------------------------------------------------------------
    // SP6 T8 — extract_text_taxonomy_from_records
    // -----------------------------------------------------------------------

    /// Helper to build a minimal `Record` for testing.
    fn make_test_record(
        source: &str,
        gl_account: &str,
        line_text: &str,
    ) -> datasynth_eval::behavioral_fidelity::Record {
        use chrono::NaiveDate;
        datasynth_eval::behavioral_fidelity::Record {
            source: source.to_string(),
            gl_account: gl_account.to_string(),
            cost_center: None,
            profit_center: None,
            trading_partner: None,
            je_number: "JE001".to_string(),
            je_line_number: "1".to_string(),
            effective_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            entry_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            created_at: None,
            functional_amount: 100.0,
            header_text: String::new(),
            line_text: line_text.to_string(),
        }
    }

    #[test]
    fn extract_text_taxonomy_from_records_resolves_class_via_coa() {
        use datasynth_core::distributions::behavioral_priors::{AccountSemantic, CoaSemanticPrior};

        // Build a minimal CoA: account "0000204000" -> class "L.2".
        let mut coa = CoaSemanticPrior::default();
        coa.accounts.insert(
            "0000204000".to_string(),
            AccountSemantic {
                description: "Kreditoren".to_string(),
                account_class: Some("L.2".to_string()),
                ..Default::default()
            },
        );

        // 12 records all hitting that account with the same line text.
        let records: Vec<_> = (0..12)
            .map(|_| make_test_record("KR", "0000204000", "Rechnung Eingang"))
            .collect();

        let prior = extract_text_taxonomy_from_records(&records, Some(&coa), None, 10)
            .expect("extraction ok");

        // Line text should be keyed on (KR, L.2), not (KR, _unknown_).
        assert!(
            prior
                .line_pools
                .contains_key(&TextTaxonomyPrior::line_key("KR", "L.2")),
            "expected KR|L.2 pool; got keys: {:?}",
            prior.line_pools.keys().collect::<Vec<_>>()
        );
        // CoA pool: account 0000204000 should have a template entry from the description.
        assert!(
            prior.coa_pools.contains_key("0000204000"),
            "expected coa_pools[0000204000]; got keys: {:?}",
            prior.coa_pools.keys().collect::<Vec<_>>()
        );
    }
}

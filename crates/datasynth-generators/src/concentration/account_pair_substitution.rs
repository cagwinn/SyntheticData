//! `AccountPairSubstitutionPass` — Phase 2 of the central concentration
//! abstraction (#143). Reshapes the per-JE `(debit_account, credit_account)`
//! edge distribution toward a corpus-empirical PMF.
//!
//! Algorithm (per JE):
//!   1. Look up PMF for the JE's `sap_source_code`; skip if no coverage.
//!   2. Identify the dominant debit line (largest `debit_amount`) and the
//!      dominant credit line (largest `credit_amount`).
//!   3. If either touches a [`STRUCTURAL_BRIDGE_ACCOUNTS`](super) entry → skip
//!      (would break AR/AP aging or subledger reconciliation).
//!   4. Compute current pair's probability under the source PMF. If it's
//!      already ≥ `rarity_threshold` (default 0.005) → skip (plausible).
//!   5. Draw a candidate `(d_new, c_new)` from the source PMF's top-K pairs,
//!      weighted by probability.
//!   6. Apply substitution to `gl_account` + `account_code` (alias) on the
//!      two dominant lines. Clear `account_description` so downstream
//!      lookups regenerate. Leave amounts untouched.
//!   7. `is_balanced()` belt-and-suspenders revert — should never fire since
//!      we don't touch amounts.
//!
//! Privacy: the PMF JSON is aggregate per-source (`debit, credit, p`); no row
//! content or client identifiers. Path supplied at runtime, never committed.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use datasynth_config::schema::AccountPairSubstitutionPassConfig;
use datasynth_core::models::JournalEntry;
use rand::distr::weighted::WeightedIndex;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use serde::Deserialize;

use super::{is_structural_bridge, ConcentrationPass, ConcentrationStats};

const PASS_NAME: &str = "account_pair_substitution";
const DEFAULT_RARITY_THRESHOLD: f64 = 0.005;
const DEFAULT_TOP_K: usize = 10;

// --- PMF on-disk schema ---------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct PmfFile {
    /// Retained on parse for forward-compat / observability; not read at runtime.
    #[serde(default)]
    #[allow(dead_code)]
    pub schema_version: u32,
    pub pmfs: Vec<PmfPerSource>,
}

#[derive(Debug, Clone, Deserialize)]
struct PmfPerSource {
    pub source: String,
    /// Sample size used to build this PMF — informational, retained on parse
    /// for downstream tooling that consults the file directly.
    #[allow(dead_code)]
    pub n_jes: u64,
    /// `[[debit_acct, credit_acct, probability], ...]` sorted desc by p.
    pub pmf: Vec<(String, String, f64)>,
}

// --- Compiled in-memory PMF -----------------------------------------------

#[derive(Debug, Clone)]
struct SourcePmf {
    /// Reverse-lookup of current-pair probability.
    pair_to_p: HashMap<(String, String), f64>,
    /// Top-K candidate pairs and their probabilities (renormalised across
    /// the top-K subset for the weighted draw).
    top_k: Vec<((String, String), f64)>,
    top_k_weights: Vec<f64>,
}

// --- Pass ------------------------------------------------------------------

pub struct AccountPairSubstitutionPass {
    pmfs_by_source: HashMap<String, SourcePmf>,
    rarity_threshold: f64,
    #[allow(dead_code)] // retained for stats reporting
    top_k: usize,
}

impl AccountPairSubstitutionPass {
    /// Build from a PMF JSON file on disk. The path is a runtime argument;
    /// the file holds aggregate stats only (per-source `(debit, credit, p)`
    /// triples).
    pub fn from_pmf_file(
        cfg: AccountPairSubstitutionPassConfig,
    ) -> Result<Self, AccountPairSubstitutionError> {
        let body = fs::read_to_string(Path::new(&cfg.pmf_path))
            .map_err(|e| AccountPairSubstitutionError::Io(e.to_string()))?;
        let file: PmfFile = serde_json::from_str(&body)
            .map_err(|e| AccountPairSubstitutionError::Parse(e.to_string()))?;
        Self::from_pmf_file_inner(file, cfg)
    }

    /// Construct from an already-parsed `PmfFile`. Used by tests + the
    /// public `from_pmf_file` constructor.
    fn from_pmf_file_inner(
        file: PmfFile,
        cfg: AccountPairSubstitutionPassConfig,
    ) -> Result<Self, AccountPairSubstitutionError> {
        let top_k = cfg.top_k.unwrap_or(DEFAULT_TOP_K).max(1);
        let rarity_threshold = cfg.rarity_threshold.unwrap_or(DEFAULT_RARITY_THRESHOLD);

        let mut pmfs_by_source: HashMap<String, SourcePmf> = HashMap::new();
        for entry in file.pmfs {
            if entry.pmf.is_empty() {
                continue;
            }
            let mut pair_to_p: HashMap<(String, String), f64> = HashMap::new();
            for (d, c, p) in entry.pmf.iter().cloned() {
                pair_to_p.insert((d, c), p);
            }
            // PMF is documented as sorted desc by p; take the first K.
            let raw_top: Vec<((String, String), f64)> = entry
                .pmf
                .iter()
                .take(top_k)
                .map(|(d, c, p)| ((d.clone(), c.clone()), *p))
                .collect();
            // Renormalise weights across the K-subset so they sum to 1.
            let total_w: f64 = raw_top.iter().map(|(_, p)| *p).sum();
            let weights: Vec<f64> = if total_w > 0.0 {
                raw_top.iter().map(|(_, p)| *p / total_w).collect()
            } else {
                // Pathological — uniform fallback.
                vec![1.0 / raw_top.len() as f64; raw_top.len()]
            };
            pmfs_by_source.insert(
                entry.source,
                SourcePmf {
                    pair_to_p,
                    top_k: raw_top,
                    top_k_weights: weights,
                },
            );
        }

        Ok(Self {
            pmfs_by_source,
            rarity_threshold,
            top_k,
        })
    }

    /// Number of sources for which a PMF is loaded.
    pub fn source_count(&self) -> usize {
        self.pmfs_by_source.len()
    }

    /// Identify the line indices of the dominant debit + dominant credit
    /// lines of a JE, or `None` if either side has no positive amount.
    fn dominant_pair_indices(je: &JournalEntry) -> Option<(usize, usize)> {
        let mut best_debit: Option<(usize, rust_decimal::Decimal)> = None;
        let mut best_credit: Option<(usize, rust_decimal::Decimal)> = None;
        for (idx, line) in je.lines.iter().enumerate() {
            if line.debit_amount > rust_decimal::Decimal::ZERO
                && best_debit.is_none_or(|(_, m)| line.debit_amount > m)
            {
                best_debit = Some((idx, line.debit_amount));
            }
            if line.credit_amount > rust_decimal::Decimal::ZERO
                && best_credit.is_none_or(|(_, m)| line.credit_amount > m)
            {
                best_credit = Some((idx, line.credit_amount));
            }
        }
        match (best_debit, best_credit) {
            (Some((di, _)), Some((ci, _))) if di != ci => Some((di, ci)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum AccountPairSubstitutionError {
    Io(String),
    Parse(String),
}

impl std::fmt::Display for AccountPairSubstitutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(s) => write!(f, "PMF file I/O error: {s}"),
            Self::Parse(s) => write!(f, "PMF file parse error: {s}"),
        }
    }
}

impl std::error::Error for AccountPairSubstitutionError {}

impl ConcentrationPass for AccountPairSubstitutionPass {
    fn name(&self) -> &'static str {
        PASS_NAME
    }

    fn apply(
        &self,
        entries: &mut [JournalEntry],
        rng: &mut ChaCha8Rng,
    ) -> ConcentrationStats {
        let mut substitutions_applied: u64 = 0;
        let mut entries_modified: usize = 0;
        let mut skipped_no_source: u64 = 0;
        let mut skipped_no_pmf: u64 = 0;
        let mut skipped_bridge: u64 = 0;
        let mut skipped_plausible: u64 = 0;
        let mut skipped_rebalance: u64 = 0;

        for je in entries.iter_mut() {
            let source = match je.header.sap_source_code.as_deref() {
                Some(s) if !s.is_empty() => s,
                _ => {
                    skipped_no_source += 1;
                    continue;
                }
            };
            let pmf = match self.pmfs_by_source.get(source) {
                Some(p) => p,
                None => {
                    skipped_no_pmf += 1;
                    continue;
                }
            };
            let (di, ci) = match Self::dominant_pair_indices(je) {
                Some(idx) => idx,
                None => continue, // JE with no dominant pair (single-sided?)
            };
            let orig_debit = je.lines[di].gl_account.clone();
            let orig_credit = je.lines[ci].gl_account.clone();
            if is_structural_bridge(&orig_debit) || is_structural_bridge(&orig_credit) {
                skipped_bridge += 1;
                continue;
            }
            let p_orig = pmf
                .pair_to_p
                .get(&(orig_debit.clone(), orig_credit.clone()))
                .copied()
                .unwrap_or(0.0);
            if p_orig >= self.rarity_threshold {
                skipped_plausible += 1;
                continue;
            }

            // Draw a candidate from top-K. Constructing WeightedIndex per JE
            // is acceptable here — top-K is small (default 10) and the alloc
            // is bounded. Could be cached per source as a follow-up.
            let chosen_idx = match WeightedIndex::new(&pmf.top_k_weights) {
                Ok(dist) => dist.sample(rng),
                Err(_) => continue, // degenerate weights — skip silently
            };
            let (ref new_pair, _new_p) = pmf.top_k[chosen_idx];

            // Skip no-op substitutions (e.g. when corpus PMF still names the
            // same pair as the synthetic — unlikely but possible).
            if new_pair.0 == orig_debit && new_pair.1 == orig_credit {
                skipped_plausible += 1;
                continue;
            }
            // Also skip if substitute would hit a bridge account (the corpus
            // PMF *could* legitimately contain bridge pairs — we still skip
            // because we treat bridges as immutable in either direction).
            if is_structural_bridge(&new_pair.0) || is_structural_bridge(&new_pair.1) {
                skipped_bridge += 1;
                continue;
            }

            // Apply substitution. Amounts untouched ⇒ balance preserved
            // by construction; the is_balanced() below is belt-and-suspenders.
            je.lines[di].gl_account = new_pair.0.clone();
            je.lines[di].account_code = new_pair.0.clone();
            je.lines[di].account_description = None;
            je.lines[ci].gl_account = new_pair.1.clone();
            je.lines[ci].account_code = new_pair.1.clone();
            je.lines[ci].account_description = None;

            if !je.is_balanced() {
                // Revert — should be unreachable since we don't touch amounts.
                je.lines[di].gl_account = orig_debit.clone();
                je.lines[di].account_code = orig_debit;
                je.lines[ci].gl_account = orig_credit.clone();
                je.lines[ci].account_code = orig_credit;
                skipped_rebalance += 1;
                continue;
            }

            substitutions_applied += 1;
            entries_modified += 1;
        }

        let mut extra: BTreeMap<&'static str, u64> = BTreeMap::new();
        extra.insert("substitutions_applied", substitutions_applied);
        extra.insert("skipped_no_source", skipped_no_source);
        extra.insert("skipped_no_pmf", skipped_no_pmf);
        extra.insert("skipped_bridge", skipped_bridge);
        extra.insert("skipped_plausible", skipped_plausible);
        extra.insert("skipped_rebalance", skipped_rebalance);
        extra.insert("sources_loaded", self.pmfs_by_source.len() as u64);

        ConcentrationStats {
            pass: PASS_NAME,
            entries_examined: entries.len(),
            entries_modified,
            extra,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use datasynth_core::models::{JournalEntry, JournalEntryLine};
    use rand::SeedableRng;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn make_je(idx: usize, source: &str, debit_acct: &str, credit_acct: &str) -> JournalEntry {
        let mut je = JournalEntry::new_simple(
            format!("JE{idx}"),
            "C1".to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            format!("test {idx}"),
        );
        je.header.sap_source_code = Some(source.to_string());
        let debit_line = JournalEntryLine {
            gl_account: debit_acct.to_string(),
            account_code: debit_acct.to_string(),
            debit_amount: Decimal::from_str("100.00").unwrap(),
            credit_amount: Decimal::ZERO,
            local_amount: Decimal::from_str("100.00").unwrap(),
            ..JournalEntryLine::default()
        };
        let credit_line = JournalEntryLine {
            gl_account: credit_acct.to_string(),
            account_code: credit_acct.to_string(),
            debit_amount: Decimal::ZERO,
            credit_amount: Decimal::from_str("100.00").unwrap(),
            local_amount: Decimal::from_str("100.00").unwrap(),
            ..JournalEntryLine::default()
        };
        je.lines.push(debit_line);
        je.lines.push(credit_line);
        je
    }

    fn corpus_pmf(source: &str, pairs: Vec<(&str, &str, f64)>) -> PmfFile {
        PmfFile {
            schema_version: 1,
            pmfs: vec![PmfPerSource {
                source: source.to_string(),
                n_jes: 1000,
                pmf: pairs.into_iter().map(|(d, c, p)| (d.to_string(), c.to_string(), p)).collect(),
            }],
        }
    }

    #[test]
    fn substitutes_rare_pair_with_corpus_pair() {
        // Synthetic emits source S1 with rare (6000, 5000) pair. Corpus says
        // S1 normally posts (7000, 4000) at 95% / (7100, 4100) at 5%.
        let mut entries = vec![make_je(0, "S1", "6000", "5000")];

        let file = corpus_pmf("S1", vec![("7000", "4000", 0.95), ("7100", "4100", 0.05)]);
        let cfg = AccountPairSubstitutionPassConfig {
            pmf_path: "".to_string(),
            rarity_threshold: Some(0.005),
            top_k: Some(2),
        };
        let pass = AccountPairSubstitutionPass::from_pmf_file_inner(file, cfg).unwrap();

        let mut rng = ChaCha8Rng::seed_from_u64(11);
        let stats = pass.apply(&mut entries, &mut rng);

        assert_eq!(stats.entries_modified, 1);
        assert_eq!(stats.extra["substitutions_applied"], 1);
        let je = &entries[0];
        // The dominant-debit line should now be 7000 or 7100, dominant-credit 4000 or 4100.
        assert!(
            je.lines[0].gl_account == "7000" || je.lines[0].gl_account == "7100",
            "got: {}",
            je.lines[0].gl_account
        );
        assert!(
            je.lines[1].gl_account == "4000" || je.lines[1].gl_account == "4100",
            "got: {}",
            je.lines[1].gl_account
        );
        // Substitution must preserve balance.
        assert!(je.is_balanced());
    }

    #[test]
    fn skips_bridge_account_pairs() {
        // JE touches GR_IR_CLEARING (2900) — must skip.
        let mut entries = vec![make_je(0, "S1", "6000", "2900")];

        let file = corpus_pmf("S1", vec![("7000", "4000", 1.0)]);
        let cfg = AccountPairSubstitutionPassConfig {
            pmf_path: "".to_string(),
            rarity_threshold: Some(0.5),
            top_k: Some(1),
        };
        let pass = AccountPairSubstitutionPass::from_pmf_file_inner(file, cfg).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let stats = pass.apply(&mut entries, &mut rng);

        assert_eq!(stats.entries_modified, 0);
        assert_eq!(stats.extra["skipped_bridge"], 1);
        // Bridge account preserved verbatim.
        assert_eq!(entries[0].lines[1].gl_account, "2900");
    }

    #[test]
    fn skips_plausible_pairs() {
        // Synthetic already emits (7000, 4000) — PMF says that's the top pair
        // at p=0.95 > rarity_threshold=0.005, so it's plausible and skipped.
        let mut entries = vec![make_je(0, "S1", "7000", "4000")];

        let file = corpus_pmf("S1", vec![("7000", "4000", 0.95), ("7100", "4100", 0.05)]);
        let cfg = AccountPairSubstitutionPassConfig {
            pmf_path: "".to_string(),
            rarity_threshold: Some(0.005),
            top_k: Some(2),
        };
        let pass = AccountPairSubstitutionPass::from_pmf_file_inner(file, cfg).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let stats = pass.apply(&mut entries, &mut rng);

        assert_eq!(stats.entries_modified, 0);
        assert_eq!(stats.extra["skipped_plausible"], 1);
        assert_eq!(entries[0].lines[0].gl_account, "7000"); // unchanged
        assert_eq!(entries[0].lines[1].gl_account, "4000");
    }

    #[test]
    fn deterministic_under_same_seed() {
        let make_batch = || -> Vec<JournalEntry> {
            (0..20).map(|i| make_je(i, "S1", "6000", "5000")).collect()
        };
        let file = corpus_pmf("S1", vec![
            ("7000", "4000", 0.4),
            ("7100", "4100", 0.3),
            ("7200", "4200", 0.2),
            ("7300", "4300", 0.1),
        ]);
        let cfg = AccountPairSubstitutionPassConfig {
            pmf_path: "".to_string(),
            rarity_threshold: Some(0.005),
            top_k: Some(4),
        };
        let pass_a = AccountPairSubstitutionPass::from_pmf_file_inner(file.clone(), cfg.clone()).unwrap();
        let pass_b = AccountPairSubstitutionPass::from_pmf_file_inner(file, cfg).unwrap();

        let mut batch_a = make_batch();
        let mut batch_b = make_batch();
        let mut rng_a = ChaCha8Rng::seed_from_u64(42);
        let mut rng_b = ChaCha8Rng::seed_from_u64(42);
        pass_a.apply(&mut batch_a, &mut rng_a);
        pass_b.apply(&mut batch_b, &mut rng_b);

        for (a, b) in batch_a.iter().zip(batch_b.iter()) {
            assert_eq!(a.lines[0].gl_account, b.lines[0].gl_account);
            assert_eq!(a.lines[1].gl_account, b.lines[1].gl_account);
        }
    }

    #[test]
    fn preserves_balance_across_batch() {
        let mut entries: Vec<JournalEntry> =
            (0..30).map(|i| make_je(i, "S1", "6000", "5000")).collect();
        let file = corpus_pmf("S1", vec![("7000", "4000", 0.95), ("7100", "4100", 0.05)]);
        let cfg = AccountPairSubstitutionPassConfig {
            pmf_path: "".to_string(),
            rarity_threshold: Some(0.005),
            top_k: Some(2),
        };
        let pass = AccountPairSubstitutionPass::from_pmf_file_inner(file, cfg).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(99);
        let _ = pass.apply(&mut entries, &mut rng);

        for je in &entries {
            assert!(je.is_balanced(), "JE {} unbalanced after pass", je.header.document_id);
        }
    }

    #[test]
    fn skips_when_no_source_pmf_loaded() {
        // JE has source S2 but PMF only knows S1.
        let mut entries = vec![make_je(0, "S2", "6000", "5000")];
        let file = corpus_pmf("S1", vec![("7000", "4000", 1.0)]);
        let cfg = AccountPairSubstitutionPassConfig {
            pmf_path: "".to_string(),
            rarity_threshold: Some(0.005),
            top_k: Some(1),
        };
        let pass = AccountPairSubstitutionPass::from_pmf_file_inner(file, cfg).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let stats = pass.apply(&mut entries, &mut rng);

        assert_eq!(stats.entries_modified, 0);
        assert_eq!(stats.extra["skipped_no_pmf"], 1);
        assert_eq!(entries[0].lines[0].gl_account, "6000"); // unchanged
    }
}

//! `SourceBlankingPass` — Phase 1.5 of the central concentration abstraction
//! (#143). Closes SOTA-7 (#132) by nulling `sap_source_code` on a configurable
//! fraction of JEs to match the corpus's ~21% blank-source rate.
//!
//! Synthetic currently emits an SAP source code on ~100% of JEs (default-on
//! after SOTA-3 / T2(D) Lever 1b). The corpus shows ~21% blank-source / no-
//! doc-type postings — see `experiments/ml/FINDINGS.md §9`. Closing the gap
//! requires a post-process drop, not a generator change: the source codes
//! themselves are realistic per-process; what's missing is the *absence
//! pattern* the corpus exhibits.
//!
//! Why post-process: the same multi-generator coverage problem that motivated
//! the central abstraction — every generator path emits a source code, and a
//! per-generator opt-out would require ~7 wiring points. One trait + one pass
//! covers them all.
//!
//! Safety: `sap_source_code` is an observable attribute; no downstream
//! invariant (balance, document-chain refs, subledger reconciliation,
//! `is_balanced()`) reads it. Other passes that DO read it
//! (`SourceConditionalRarityPass`, `AccountPairSubstitutionPass`) handle
//! `None` correctly — they just skip those JEs. Pipeline ordering matters:
//! `SourceBlankingPass` must run AFTER those passes, otherwise their PMF
//! coverage drops by the blanking rate. Pipeline registration order (in
//! `mod.rs::from_config`) enforces this.

use std::collections::BTreeMap;

use datasynth_config::schema::SourceBlankingPassConfig;
use datasynth_core::models::JournalEntry;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

use super::{ConcentrationPass, ConcentrationStats};

const PASS_NAME: &str = "source_blanking";

pub struct SourceBlankingPass {
    rate: f64,
}

impl SourceBlankingPass {
    pub fn new(cfg: SourceBlankingPassConfig) -> Self {
        Self {
            // Clamp to [0.0, 1.0] so a misconfigured value can't break the pass.
            rate: cfg.rate.clamp(0.0, 1.0),
        }
    }
}

impl ConcentrationPass for SourceBlankingPass {
    fn name(&self) -> &'static str {
        PASS_NAME
    }

    fn apply(&self, entries: &mut [JournalEntry], rng: &mut ChaCha8Rng) -> ConcentrationStats {
        if self.rate == 0.0 {
            // Fast path — pass is opted out; emit stats but make no changes.
            return ConcentrationStats {
                pass: PASS_NAME,
                entries_examined: entries.len(),
                entries_modified: 0,
                extra: BTreeMap::new(),
            };
        }

        let mut blanked: usize = 0;
        let mut already_blank: u64 = 0;
        for je in entries.iter_mut() {
            // "Already blank" = sap_source_code missing or empty string.
            // Both states are treated the same downstream by the CSV writer
            // (it back-fills the TransactionSource Display label only when
            // sap_source_code is None, NOT when it's an empty string — so we
            // express "intentionally blanked" as `Some("")` to override the
            // writer's fallback and match the corpus's literal-empty Source
            // column semantic).
            match je.header.sap_source_code.as_deref() {
                None | Some("") => {
                    already_blank += 1;
                    continue;
                }
                _ => {}
            }
            let draw: f64 = rng.random();
            if draw < self.rate {
                // See above: `Some("")` (not `None`) so the output writer
                // honors the blank intent. None still falls back to the
                // TransactionSource Display label for the priors-disabled
                // legacy path; that fallback is preserved.
                je.header.sap_source_code = Some(String::new());
                blanked += 1;
            }
        }

        let mut extra = BTreeMap::new();
        extra.insert("blanked", blanked as u64);
        extra.insert("already_blank", already_blank);
        // Effective rate in basis points (1bp = 0.01%) — cheap integer encoding so
        // observability doesn't need a float field.
        let total = entries.len() as u64;
        let target_bp = (self.rate * 10_000.0) as u64;
        extra.insert("target_rate_bp", target_bp);
        if let Some(eff_bp) = (blanked as u64 * 10_000).checked_div(total) {
            extra.insert("effective_rate_bp", eff_bp);
        }

        ConcentrationStats {
            pass: PASS_NAME,
            entries_examined: entries.len(),
            entries_modified: blanked,
            extra,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use datasynth_core::models::{JournalEntry, JournalEntryLine};
    use rand::SeedableRng;

    fn make_je(idx: usize, source: Option<&str>) -> JournalEntry {
        let mut je = JournalEntry::new_simple(
            format!("JE{idx}"),
            "C1".to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            format!("test {idx}"),
        );
        je.header.sap_source_code = source.map(String::from);
        let line = JournalEntryLine {
            gl_account: "6000".to_string(),
            ..JournalEntryLine::default()
        };
        je.lines.push(line);
        je
    }

    #[test]
    fn rate_zero_leaves_all_sources_intact() {
        let mut entries: Vec<JournalEntry> = (0..100).map(|i| make_je(i, Some("BKPF"))).collect();
        let pass = SourceBlankingPass::new(SourceBlankingPassConfig { rate: 0.0 });
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let stats = pass.apply(&mut entries, &mut rng);
        assert_eq!(stats.entries_modified, 0);
        for je in &entries {
            assert_eq!(je.header.sap_source_code.as_deref(), Some("BKPF"));
        }
    }

    #[test]
    fn rate_one_blanks_every_source() {
        let mut entries: Vec<JournalEntry> = (0..100).map(|i| make_je(i, Some("BKPF"))).collect();
        let pass = SourceBlankingPass::new(SourceBlankingPassConfig { rate: 1.0 });
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let stats = pass.apply(&mut entries, &mut rng);
        assert_eq!(stats.entries_modified, 100);
        for je in &entries {
            // Post-pass: sap_source_code is `Some("")` (intentionally blanked
            // marker); never `None` for previously-set rows. See the pass-body
            // comment for why `Some("")` not `None`.
            assert_eq!(je.header.sap_source_code.as_deref(), Some(""));
        }
    }

    #[test]
    fn rate_021_lands_in_corpus_band() {
        // n=2000 with rate=0.21: ~420 blanked. Binomial std ≈ √(npq) ≈ 18, so
        // ±100 is ~5σ — assertion comfortably bounds CI noise without being lax.
        let mut entries: Vec<JournalEntry> = (0..2000).map(|i| make_je(i, Some("BKPF"))).collect();
        let pass = SourceBlankingPass::new(SourceBlankingPassConfig { rate: 0.21 });
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let stats = pass.apply(&mut entries, &mut rng);
        let blanked = stats.entries_modified;
        assert!(
            (320..=520).contains(&blanked),
            "rate=0.21 blanked={} (expected ~420)",
            blanked
        );
        assert_eq!(stats.extra["target_rate_bp"], 2100);
    }

    #[test]
    fn already_blank_jes_pass_through_uncounted() {
        // Half the batch already has None — those should be skipped before the RNG roll.
        let mut entries: Vec<JournalEntry> = (0..100)
            .map(|i| make_je(i, if i % 2 == 0 { Some("BKPF") } else { None }))
            .collect();
        let pass = SourceBlankingPass::new(SourceBlankingPassConfig { rate: 1.0 });
        let mut rng = ChaCha8Rng::seed_from_u64(2);
        let stats = pass.apply(&mut entries, &mut rng);
        assert_eq!(stats.entries_modified, 50); // only the 50 originally-set
        assert_eq!(stats.extra["already_blank"], 50);
    }

    #[test]
    fn deterministic_under_same_seed() {
        let make_batch =
            || -> Vec<JournalEntry> { (0..100).map(|i| make_je(i, Some("BKPF"))).collect() };
        let cfg = SourceBlankingPassConfig { rate: 0.3 };
        let pass_a = SourceBlankingPass::new(cfg.clone());
        let pass_b = SourceBlankingPass::new(cfg);

        let mut batch_a = make_batch();
        let mut batch_b = make_batch();
        let mut rng_a = ChaCha8Rng::seed_from_u64(42);
        let mut rng_b = ChaCha8Rng::seed_from_u64(42);
        pass_a.apply(&mut batch_a, &mut rng_a);
        pass_b.apply(&mut batch_b, &mut rng_b);

        for (a, b) in batch_a.iter().zip(batch_b.iter()) {
            assert_eq!(a.header.sap_source_code, b.header.sap_source_code);
        }
    }
}

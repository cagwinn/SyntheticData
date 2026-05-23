//! `SourceConditionalRarityPass` — Phase 1 trait wrapper around the already-
//! shipped SOTA-12 tagger (`crate::anomaly::source_conditional_rarity`).
//!
//! Pure interface adapter — no algorithm change. The existing 4 unit tests in
//! `anomaly/source_conditional_rarity.rs` continue to cover the algorithm;
//! this module's test verifies the wrapper plumbing.

use std::collections::BTreeMap;

use datasynth_config::schema::SourceConditionalRarityPassConfig;
use datasynth_core::models::JournalEntry;
use rand_chacha::ChaCha8Rng;

use super::{ConcentrationPass, ConcentrationStats};
use crate::anomaly::source_conditional_rarity::{
    tag_source_conditional_rarity, SourceConditionalRarityConfig,
};

const PASS_NAME: &str = "source_conditional_rarity";

pub struct SourceConditionalRarityPass {
    cfg: SourceConditionalRarityPassConfig,
}

impl SourceConditionalRarityPass {
    pub fn new(cfg: SourceConditionalRarityPassConfig) -> Self {
        Self { cfg }
    }

    fn build_inner_config(&self) -> SourceConditionalRarityConfig {
        let defaults = SourceConditionalRarityConfig::default();
        SourceConditionalRarityConfig {
            rate: self.cfg.rate,
            min_surprise: self.cfg.min_surprise.unwrap_or(defaults.min_surprise),
            min_per_source_lines: self
                .cfg
                .min_per_source_lines
                .unwrap_or(defaults.min_per_source_lines),
        }
    }
}

impl ConcentrationPass for SourceConditionalRarityPass {
    fn name(&self) -> &'static str {
        PASS_NAME
    }

    fn apply(
        &self,
        entries: &mut [JournalEntry],
        _rng: &mut ChaCha8Rng,
    ) -> ConcentrationStats {
        let inner = self.build_inner_config();
        let tagged = tag_source_conditional_rarity(entries, &inner);
        let mut extra = BTreeMap::new();
        extra.insert("jes_tagged", tagged as u64);
        ConcentrationStats {
            pass: PASS_NAME,
            entries_examined: entries.len(),
            entries_modified: tagged,
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

    fn make_je_with_source(idx: usize, source: &str, account: &str) -> JournalEntry {
        let mut je = JournalEntry::new_simple(
            format!("JE{idx}"),
            "C1".to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            format!("test je {idx}"),
        );
        je.header.sap_source_code = Some(source.to_string());
        let line = JournalEntryLine {
            gl_account: account.to_string(),
            ..JournalEntryLine::default()
        };
        je.lines.push(line);
        je
    }

    #[test]
    fn wrapper_delegates_to_inner_tagger() {
        // 100 JEs, 99 share the common (S1, 6000) edge, 1 is rare (S1, 9999).
        // With rate=0.02 the wrapper should tag at most 2.
        let mut entries: Vec<JournalEntry> = (0..99)
            .map(|i| make_je_with_source(i, "S1", "6000"))
            .collect();
        entries.push(make_je_with_source(99, "S1", "9999"));

        let pass = SourceConditionalRarityPass::new(SourceConditionalRarityPassConfig {
            rate: 0.02,
            min_surprise: Some(0.0), // disable floor to confirm count from rate alone
            min_per_source_lines: Some(1),
        });
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let stats = pass.apply(&mut entries, &mut rng);

        assert_eq!(stats.pass, PASS_NAME);
        assert_eq!(stats.entries_examined, 100);
        // rate=0.02 * 100 = 2 JEs eligible to tag; the rare one (9999) must be in.
        assert!(stats.entries_modified <= 2, "{}", stats.entries_modified);
        assert!(stats.entries_modified >= 1, "rare JE should be tagged");
        // The rare JE must be one of the tagged ones.
        let tagged_rare = entries.iter().any(|je| {
            je.lines[0].gl_account == "9999" && je.header.is_anomaly
        });
        assert!(tagged_rare, "the rare (S1, 9999) JE must be tagged");
    }
}

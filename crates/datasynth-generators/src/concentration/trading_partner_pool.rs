//! `TradingPartnerPoolPass` — closes the SOTA-11.1 / #142 coverage gap by
//! rewriting JE-line `trading_partner` strings to a target pool size.
//!
//! Why post-process: the SOTA-11 round's `vendor.count` knob only affected
//! `je_generator`'s direct path; document-flow / allocation / period-close
//! generators use their own hard-coded V-000001..V-000040 pool. This pass
//! sees every JE regardless of which generator emitted it.
//!
//! Safety: `trading_partner` is an informational field — no downstream
//! invariant (balance, document-chain refs, subledger reconciliation) reads
//! it. Rewriting in post-process is correct by construction.
//!
//! Determinism: FNV-1a 64-bit hash of the original TP string indexes a pool
//! of size `target_size`. Same input → same output across runs.

use std::collections::BTreeMap;

use datasynth_config::schema::TradingPartnerPoolPassConfig;
use datasynth_core::models::JournalEntry;
use rand_chacha::ChaCha8Rng;

use super::{ConcentrationPass, ConcentrationStats};

const PASS_NAME: &str = "trading_partner_pool";

pub struct TradingPartnerPoolPass {
    target_size: u64,
}

impl TradingPartnerPoolPass {
    pub fn new(cfg: TradingPartnerPoolPassConfig) -> Self {
        Self {
            // Clamp to >= 1 so the modulus is well-defined.
            target_size: cfg.target_size.max(1) as u64,
        }
    }

    /// FNV-1a 64-bit hash → pool index. Same TP string always maps to the
    /// same canonical pool TP across runs.
    #[inline]
    fn pool_index(&self, tp: &str) -> u64 {
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;
        let mut h = FNV_OFFSET;
        for b in tp.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
        h % self.target_size
    }

    #[inline]
    fn canonical_tp(&self, original: &str) -> String {
        format!("TP-{:06}", self.pool_index(original))
    }
}

impl ConcentrationPass for TradingPartnerPoolPass {
    fn name(&self) -> &'static str {
        PASS_NAME
    }

    fn apply(
        &self,
        entries: &mut [JournalEntry],
        _rng: &mut ChaCha8Rng,
    ) -> ConcentrationStats {
        let mut lines_modified: u64 = 0;
        let mut entries_modified: usize = 0;
        for je in entries.iter_mut() {
            let mut je_touched = false;
            for line in je.lines.iter_mut() {
                if let Some(tp) = line.trading_partner.as_ref() {
                    let new_tp = self.canonical_tp(tp);
                    if &new_tp != tp {
                        line.trading_partner = Some(new_tp);
                        lines_modified += 1;
                        je_touched = true;
                    }
                }
            }
            if je_touched {
                entries_modified += 1;
            }
        }
        let mut extra = BTreeMap::new();
        extra.insert("lines_modified", lines_modified);
        extra.insert("target_pool_size", self.target_size);
        ConcentrationStats {
            pass: PASS_NAME,
            entries_examined: entries.len(),
            entries_modified,
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
    use std::collections::HashSet;

    fn make_je(idx: usize, tp: Option<&str>) -> JournalEntry {
        let mut je = JournalEntry::new_simple(
            format!("JE{idx}"),
            "C1".to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            format!("test {idx}"),
        );
        let line = JournalEntryLine {
            gl_account: "6000".to_string(),
            trading_partner: tp.map(String::from),
            ..JournalEntryLine::default()
        };
        je.lines.push(line);
        je
    }

    #[test]
    fn converges_to_target_pool_size() {
        // 200 distinct TP strings, target 25 — distinct count must collapse to ≤ 25.
        let mut entries: Vec<JournalEntry> = (0..200)
            .map(|i| make_je(i, Some(&format!("V-{:06}", i))))
            .collect();

        let pass = TradingPartnerPoolPass::new(TradingPartnerPoolPassConfig {
            target_size: 25,
        });
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let stats = pass.apply(&mut entries, &mut rng);

        let distinct: HashSet<&String> = entries
            .iter()
            .filter_map(|je| je.lines[0].trading_partner.as_ref())
            .collect();
        assert!(distinct.len() <= 25, "pool exceeded target: {}", distinct.len());
        // Hash quality lower bound — FNV-1a on sequential inputs is uniform
        // asymptotically; with 200 → 25 bins we expect >= half the pool to fill.
        assert!(
            distinct.len() >= 12,
            "pool under-filled below half: {}",
            distinct.len()
        );
        assert_eq!(stats.entries_examined, 200);
        assert_eq!(stats.extra["target_pool_size"], 25);
    }

    #[test]
    fn deterministic_under_same_seed() {
        let make_batch = || -> Vec<JournalEntry> {
            (0..50)
                .map(|i| make_je(i, Some(&format!("V-orig-{i:04}"))))
                .collect()
        };
        let cfg = TradingPartnerPoolPassConfig { target_size: 8 };
        let pass_a = TradingPartnerPoolPass::new(cfg.clone());
        let pass_b = TradingPartnerPoolPass::new(cfg);

        let mut batch_a = make_batch();
        let mut batch_b = make_batch();
        let mut rng_a = ChaCha8Rng::seed_from_u64(123);
        let mut rng_b = ChaCha8Rng::seed_from_u64(123);
        pass_a.apply(&mut batch_a, &mut rng_a);
        pass_b.apply(&mut batch_b, &mut rng_b);

        for (a, b) in batch_a.iter().zip(batch_b.iter()) {
            assert_eq!(a.lines[0].trading_partner, b.lines[0].trading_partner);
        }
    }

    #[test]
    fn preserves_lines_without_trading_partner() {
        let mut entries: Vec<JournalEntry> = (0..10).map(|i| make_je(i, None)).collect();
        let pass = TradingPartnerPoolPass::new(TradingPartnerPoolPassConfig {
            target_size: 5,
        });
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let stats = pass.apply(&mut entries, &mut rng);
        assert_eq!(stats.entries_modified, 0);
        assert_eq!(stats.extra["lines_modified"], 0);
        for je in &entries {
            assert!(je.lines[0].trading_partner.is_none());
        }
    }

    #[test]
    fn zero_target_size_is_clamped_to_one() {
        let pass = TradingPartnerPoolPass::new(TradingPartnerPoolPassConfig {
            target_size: 0,
        });
        let mut entries = vec![make_je(0, Some("V-000001"))];
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let _ = pass.apply(&mut entries, &mut rng);
        // All TPs must collapse to a single string (since pool size = 1).
        assert_eq!(
            entries[0].lines[0].trading_partner.as_deref(),
            Some("TP-000000")
        );
    }
}

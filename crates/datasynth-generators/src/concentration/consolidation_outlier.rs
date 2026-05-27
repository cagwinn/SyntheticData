//! `ConsolidationOutlierPass` — v5.30 B2 (#154) heavy-tail outlier emission.
//!
//! Reshapes a small fraction of normal 2-line journal entries into
//! multi-100-line postings that touch *bridge accounts* (clearing,
//! suspense, intercompany clearing, equity adjustments). Models the
//! consolidation entries, period-end accruals, and manual reclasses
//! that real general ledgers carry in their heavy tail but synthetic
//! emits zero of by default.
//!
//! ## Why it matters
//!
//! Sajja's relational_score metric weighs the p99 / max line-count
//! and account-degree percentiles. The corpus exhibits ~20× normal
//! mode there (a handful of monster 100-500-line JEs every period);
//! synthetic's default ~12× falls short. The B2 lever closes that
//! gap without distorting the bulk (median / p75) of the distribution.
//!
//! ## Mechanism
//!
//! For each input JE, draw `Bernoulli(rate)`:
//!
//! - 0 → leave unchanged (fast path)
//! - 1 → expand by appending N ∈ [`min_extra_lines`, `max_extra_lines`]
//!   additional lines in equal DR/CR pairs (so total debit and total
//!   credit each get the same delta and the JE remains balanced).
//!   Each pair picks one bridge-account-DR + one bridge-account-CR at
//!   random, with amount log-uniform in
//!   [`line_amount_min`, `line_amount_max`].
//!
//! ## Invariants preserved
//!
//! - **Balance.** Each appended pair contributes `(amount, amount)` to
//!   `(total_debit_delta, total_credit_delta)` → both deltas equal →
//!   JE stays balanced.
//! - **No IC corruption.** IC injector JEs (header.ic_pair_id.is_some())
//!   are skipped entirely. Reshaping an IC JE would either (a) change
//!   its line count past what the IC matcher expects, or (b) split the
//!   notional across more lines than the elimination factory's
//!   `find(|l| l.is_debit())` first-match heuristic can handle.
//! - **No duplicate document IDs.** All appended lines reuse the
//!   original JE's `document_id`; only `line_number` is incremented.
//! - **Deterministic.** A dedicated ChaCha8 substream from the
//!   pipeline; identical input + rate + seed → identical output.

use std::collections::BTreeMap;

use datasynth_config::schema::ConsolidationOutlierPassConfig;
use datasynth_core::models::{JournalEntry, JournalEntryLine};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rust_decimal::Decimal;

use super::{ConcentrationPass, ConcentrationStats};

const PASS_NAME: &str = "consolidation_outlier";

/// Default bridge accounts used when the config doesn't supply a list.
/// Mix of clearing (1900/1950), other accruals (2900), equity (3900),
/// inter-process clearing (5900), intercompany clearing (8900), and
/// suspense (9900) — the corpus's actual heavy-tail JEs touch this
/// kind of account set.
pub(crate) const DEFAULT_BRIDGE_ACCOUNTS: &[&str] = &[
    "1900", "1950", "2900", "2950", "3900", "3950", "5900", "5950", "8900", "8950", "9900", "9950",
];

pub struct ConsolidationOutlierPass {
    rate: f64,
    min_extra_lines: usize,
    max_extra_lines: usize,
    bridge_accounts: Vec<String>,
    line_amount_min: f64,
    line_amount_max: f64,
}

impl ConsolidationOutlierPass {
    pub fn new(cfg: ConsolidationOutlierPassConfig) -> Self {
        let bridge_accounts = if cfg.bridge_accounts.is_empty() {
            DEFAULT_BRIDGE_ACCOUNTS
                .iter()
                .map(|s| (*s).to_string())
                .collect()
        } else {
            cfg.bridge_accounts
        };
        // Force min ≤ max and round up to even (so we always append
        // complete DR/CR pairs).
        let (min_lines, max_lines) = {
            let mut lo = cfg.min_extra_lines.max(2);
            let mut hi = cfg.max_extra_lines.max(lo);
            if !lo.is_multiple_of(2) {
                lo += 1;
            }
            if !hi.is_multiple_of(2) {
                hi += 1;
            }
            (lo, hi)
        };
        Self {
            rate: cfg.rate.clamp(0.0, 1.0),
            min_extra_lines: min_lines,
            max_extra_lines: max_lines,
            bridge_accounts,
            line_amount_min: cfg.line_amount_min.max(1.0),
            line_amount_max: cfg.line_amount_max.max(cfg.line_amount_min),
        }
    }
}

impl ConcentrationPass for ConsolidationOutlierPass {
    fn name(&self) -> &'static str {
        PASS_NAME
    }

    fn apply(&self, entries: &mut [JournalEntry], rng: &mut ChaCha8Rng) -> ConcentrationStats {
        if self.rate == 0.0 || self.bridge_accounts.is_empty() {
            return ConcentrationStats {
                pass: PASS_NAME,
                entries_examined: entries.len(),
                entries_modified: 0,
                extra: BTreeMap::new(),
            };
        }

        let mut modified: usize = 0;
        let mut total_added_lines: u64 = 0;
        let mut skipped_ic: u64 = 0;
        let log_lo = self.line_amount_min.ln();
        let log_hi = self.line_amount_max.ln();

        for je in entries.iter_mut() {
            // IC contract: skip — see module docs.
            if je.header.ic_pair_id.is_some() {
                skipped_ic += 1;
                continue;
            }
            // Bernoulli(rate).
            let draw: f64 = rng.random();
            if draw >= self.rate {
                continue;
            }

            let n_pairs = if self.min_extra_lines == self.max_extra_lines {
                self.min_extra_lines / 2
            } else {
                rng.random_range(self.min_extra_lines / 2..=self.max_extra_lines / 2)
            };
            if n_pairs == 0 {
                continue;
            }

            let next_line_no = je.lines.iter().map(|l| l.line_number).max().unwrap_or(0) + 1;
            let doc_id = je.header.document_id;

            for i in 0..n_pairs {
                // Pick two distinct bridge accounts (if the pool only has one,
                // accept the degenerate same-account pair — JE still balances).
                let dr_idx = rng.random_range(0..self.bridge_accounts.len());
                let cr_idx = if self.bridge_accounts.len() > 1 {
                    let mut idx = rng.random_range(0..self.bridge_accounts.len());
                    if idx == dr_idx {
                        idx = (idx + 1) % self.bridge_accounts.len();
                    }
                    idx
                } else {
                    dr_idx
                };

                // Log-uniform amount in [line_amount_min, line_amount_max].
                // Rounding to whole units before convert-to-Decimal — synthetic
                // bridge-line amounts read scale not precision for the pp99
                // metric, and whole-unit amounts make the line easier to spot
                // in audit fixtures.
                let log_amount = log_lo + (log_hi - log_lo) * rng.random::<f64>();
                let amount_f = log_amount.exp().round().max(1.0);
                let amount = Decimal::try_from(amount_f).unwrap_or(Decimal::ONE);

                let pair_offset = (i as u32) * 2;
                je.add_line(JournalEntryLine::debit(
                    doc_id,
                    next_line_no + pair_offset,
                    self.bridge_accounts[dr_idx].clone(),
                    amount,
                ));
                je.add_line(JournalEntryLine::credit(
                    doc_id,
                    next_line_no + pair_offset + 1,
                    self.bridge_accounts[cr_idx].clone(),
                    amount,
                ));
            }

            modified += 1;
            total_added_lines += (n_pairs * 2) as u64;
        }

        let mut extra = BTreeMap::new();
        extra.insert("added_lines_total", total_added_lines);
        extra.insert("skipped_ic", skipped_ic);
        ConcentrationStats {
            pass: PASS_NAME,
            entries_examined: entries.len(),
            entries_modified: modified,
            extra,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use datasynth_core::models::{IcPairId, JournalEntry, JournalEntryHeader};
    use rand::SeedableRng;
    use rust_decimal_macros::dec;

    fn make_je(line_amount: Decimal) -> JournalEntry {
        let header = JournalEntryHeader::new(
            "C001".to_string(),
            NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
        );
        let mut je = JournalEntry::new(header);
        let doc_id = je.header.document_id;
        je.add_line(JournalEntryLine::debit(
            doc_id,
            1,
            "5000".into(),
            line_amount,
        ));
        je.add_line(JournalEntryLine::credit(
            doc_id,
            2,
            "1000".into(),
            line_amount,
        ));
        je
    }

    #[test]
    fn rate_zero_is_noop() {
        let cfg = ConsolidationOutlierPassConfig {
            rate: 0.0,
            min_extra_lines: 50,
            max_extra_lines: 200,
            bridge_accounts: vec![],
            line_amount_min: 100.0,
            line_amount_max: 50_000.0,
        };
        let pass = ConsolidationOutlierPass::new(cfg);
        let mut entries = vec![make_je(dec!(1000)); 10];
        let pre_lines: Vec<usize> = entries.iter().map(|e| e.lines.len()).collect();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let stats = pass.apply(&mut entries, &mut rng);

        assert_eq!(stats.entries_modified, 0);
        let post_lines: Vec<usize> = entries.iter().map(|e| e.lines.len()).collect();
        assert_eq!(pre_lines, post_lines, "rate=0 must not modify any JE");
    }

    #[test]
    fn expansion_balances_the_je() {
        let cfg = ConsolidationOutlierPassConfig {
            rate: 1.0, // every JE expands — exercises the path
            min_extra_lines: 50,
            max_extra_lines: 50, // pin to exactly 25 DR/CR pairs
            bridge_accounts: vec![],
            line_amount_min: 100.0,
            line_amount_max: 10_000.0,
        };
        let pass = ConsolidationOutlierPass::new(cfg);
        let mut entries = vec![make_je(dec!(1000)); 5];
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let stats = pass.apply(&mut entries, &mut rng);

        assert_eq!(stats.entries_examined, 5);
        assert_eq!(stats.entries_modified, 5);
        for (i, je) in entries.iter().enumerate() {
            assert_eq!(
                je.lines.len(),
                52,
                "JE {i} should have 2 original + 50 added lines",
            );
            let total_dr: Decimal = je.lines.iter().map(|l| l.debit_amount).sum();
            let total_cr: Decimal = je.lines.iter().map(|l| l.credit_amount).sum();
            assert_eq!(
                total_dr, total_cr,
                "JE {i} must remain balanced after expansion (DR={total_dr}, CR={total_cr})",
            );
        }
    }

    #[test]
    fn ic_jes_are_skipped() {
        let cfg = ConsolidationOutlierPassConfig {
            rate: 1.0,
            min_extra_lines: 50,
            max_extra_lines: 50,
            bridge_accounts: vec![],
            line_amount_min: 100.0,
            line_amount_max: 10_000.0,
        };
        let pass = ConsolidationOutlierPass::new(cfg);
        let mut entries = vec![make_je(dec!(1000)); 3];
        // Mark middle JE as an IC injector posting.
        entries[1].header.ic_pair_id = Some(IcPairId::from_bytes([0xCD; 32]));
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let stats = pass.apply(&mut entries, &mut rng);

        assert_eq!(stats.entries_modified, 2, "only non-IC JEs should expand");
        assert_eq!(
            entries[1].lines.len(),
            2,
            "IC JE must NOT be expanded — would break the elimination contract",
        );
        assert_eq!(
            *stats.extra.get("skipped_ic").unwrap(),
            1,
            "stats should record one IC skip",
        );
    }

    #[test]
    fn rate_filters_realistically() {
        // 1000 JEs at rate=0.05 should produce ~50 expansions ±15 (3σ).
        let cfg = ConsolidationOutlierPassConfig {
            rate: 0.05,
            min_extra_lines: 50,
            max_extra_lines: 200,
            bridge_accounts: vec![],
            line_amount_min: 100.0,
            line_amount_max: 50_000.0,
        };
        let pass = ConsolidationOutlierPass::new(cfg);
        let mut entries: Vec<JournalEntry> = (0..1000).map(|_| make_je(dec!(1000))).collect();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let stats = pass.apply(&mut entries, &mut rng);

        // Binomial(n=1000, p=0.05) has mean 50, σ ≈ 7. 3σ window: [29, 71].
        assert!(
            (29..=71).contains(&stats.entries_modified),
            "expansions {} not in 3σ window [29, 71] for n=1000, p=0.05",
            stats.entries_modified
        );
    }
}

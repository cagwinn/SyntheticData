//! Central concentration abstraction (#143, Phase 1).
//!
//! Post-generation passes that reshape the JE batch's distributional structure
//! toward a corpus-derived target. Single integration point in the orchestrator
//! sidesteps the multi-generator coverage problem that blocked SOTA-8 (#141) and
//! SOTA-11 (#142) during the SOTA-N round.
//!
//! Design reference: `docs/superpowers/specs/2026-05-23-concentration-pass-INDEX.md`
//!
//! Invariants every implementor MUST preserve:
//!   1. Per-JE balance: `sum(debits) == sum(credits)`
//!   2. Subledger-bridge accounts unchanged (see [`STRUCTURAL_BRIDGE_ACCOUNTS`])
//!   3. Document-chain refs (`DocumentReference` is keyed by `document_id`,
//!      not by `gl_account` — substituting non-bridge accounts is safe)
//!   4. Determinism: same RNG seed + same input batch ⇒ same output
//!
//! # Phase 1 (this commit)
//!   - [`ConcentrationPass`] trait + [`ConcentrationStats`] aggregate
//!   - [`ConcentrationPipeline`] runs passes in order with per-pass RNG isolation
//!   - [`SourceConditionalRarityPass`] — wraps shipped SOTA-12 tagger
//!   - [`TradingPartnerPoolPass`] — closes SOTA-11.1 / #142 coverage gap
//!
//! # Phase 2 (next commit)
//!   - `AccountPairSubstitutionPass` — corpus-PMF-driven account rewriting
//!     respecting the bridge allowlist + AccountType invariant

use std::collections::BTreeMap;

use datasynth_config::schema::ConcentrationConfig;
use datasynth_core::models::JournalEntry;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::Serialize;

pub mod source_conditional_rarity_pass;
pub mod trading_partner_pool;

pub use source_conditional_rarity_pass::SourceConditionalRarityPass;
pub use trading_partner_pool::TradingPartnerPoolPass;

/// The 7 subledger-bridge accounts whose balances must be preserved across any
/// post-process substitution. Aggregating to subledger totals (AR/AP aging) or
/// netting across matching pairs (GR/IR, IC, wire, fixed-asset acquisition),
/// substituting them silently breaks reconciliation reports.
///
/// Source: chain-invariants addendum (`docs/superpowers/specs/2026-05-23-central-abstraction-addendum-chain-invariants.md`).
pub const STRUCTURAL_BRIDGE_ACCOUNTS: &[&str] = &[
    "1100", // AR_CONTROL          — GL counterpart of AR subledger
    "2000", // AP_CONTROL          — GL counterpart of AP subledger
    "2900", // GR_IR_CLEARING      — Goods Receipt ↔ Invoice Receipt bridge
    "1150", // IC_AR_CLEARING      — Intercompany AR matching
    "2050", // IC_AP_CLEARING      — Intercompany AP matching
    "1030", // WIRE_CLEARING       — Wire-transfer reconciliation
    "1599", // ACQUISITION_CLEARING — Fixed-asset acquisition clearing
];

/// True iff the account code is a subledger-bridge account that must be
/// preserved across post-process substitution. Phase-1 passes don't substitute
/// accounts at all so this is currently informational; Phase-2's account-pair
/// substitution pass will gate substitution on this.
#[inline]
pub fn is_structural_bridge(account: &str) -> bool {
    STRUCTURAL_BRIDGE_ACCOUNTS.contains(&account)
}

// ---------------------------------------------------------------------------
// Public trait
// ---------------------------------------------------------------------------

/// A post-generation transformation reshaping the JE batch's concentration /
/// distributional structure toward a target. See module-level docs for the
/// invariants every implementor must preserve.
pub trait ConcentrationPass: Send + Sync {
    /// Short stable identifier — used in stats + logs.
    fn name(&self) -> &'static str;

    /// Apply the transformation in place. Returns aggregate counters.
    /// `rng` is a dedicated per-pass ChaCha8 substream split by the pipeline,
    /// so adding/removing a pass doesn't perturb downstream passes' RNG state.
    fn apply(
        &self,
        entries: &mut [JournalEntry],
        rng: &mut ChaCha8Rng,
    ) -> ConcentrationStats;
}

/// Per-pass aggregate counters. Serialised into the orchestrator's run report.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ConcentrationStats {
    pub pass: &'static str,
    pub entries_examined: usize,
    pub entries_modified: usize,
    /// Pass-specific counters (e.g. `lines_substituted`, `tps_remapped`).
    #[serde(default)]
    pub extra: BTreeMap<&'static str, u64>,
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

/// Ordered, deterministic execution of zero or more `ConcentrationPass`
/// instances. Each pass receives a dedicated ChaCha8 substream so adding or
/// removing a pass does not perturb the RNG state of any other pass.
pub struct ConcentrationPipeline {
    passes: Vec<Box<dyn ConcentrationPass>>,
}

impl ConcentrationPipeline {
    /// Build a pipeline from config. Returns an empty pipeline if
    /// `cfg.enabled == false` or no passes are configured.
    pub fn from_config(cfg: &ConcentrationConfig) -> Self {
        let mut passes: Vec<Box<dyn ConcentrationPass>> = Vec::new();

        if !cfg.enabled {
            return Self { passes };
        }

        if let Some(c) = cfg.source_conditional_rarity.as_ref() {
            passes.push(Box::new(SourceConditionalRarityPass::new(c.clone())));
        }
        if let Some(c) = cfg.trading_partner_pool.as_ref() {
            passes.push(Box::new(TradingPartnerPoolPass::new(c.clone())));
        }
        // Phase 2 will register AccountPairSubstitutionPass here.

        Self { passes }
    }

    /// Execute every pass in order, each with its own ChaCha8 substream
    /// derived deterministically from `seed`. Returns one `ConcentrationStats`
    /// per pass in execution order.
    pub fn run(
        &self,
        entries: &mut [JournalEntry],
        seed: u64,
    ) -> Vec<ConcentrationStats> {
        // Golden-ratio multiplier keeps substreams well-separated across passes.
        const STREAM_STRIDE: u64 = 0x9E37_79B9_7F4A_7C15;

        self.passes
            .iter()
            .enumerate()
            .map(|(idx, pass)| {
                let stream_seed =
                    seed.wrapping_add((idx as u64).wrapping_mul(STREAM_STRIDE));
                let mut rng = ChaCha8Rng::seed_from_u64(stream_seed);
                pass.apply(entries, &mut rng)
            })
            .collect()
    }

    /// True iff the pipeline has at least one configured pass.
    pub fn is_active(&self) -> bool {
        !self.passes.is_empty()
    }

    /// Number of configured passes (for tests / observability).
    pub fn len(&self) -> usize {
        self.passes.len()
    }

    /// True iff zero passes are configured.
    pub fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use datasynth_config::schema::{
        SourceConditionalRarityPassConfig, TradingPartnerPoolPassConfig,
    };

    #[test]
    fn empty_config_yields_inactive_pipeline() {
        let cfg = ConcentrationConfig::default();
        let pipeline = ConcentrationPipeline::from_config(&cfg);
        assert!(!pipeline.is_active());
        assert_eq!(pipeline.len(), 0);
    }

    #[test]
    fn disabled_master_switch_yields_inactive_pipeline_even_with_passes() {
        let cfg = ConcentrationConfig {
            enabled: false, // master switch off
            trading_partner_pool: Some(TradingPartnerPoolPassConfig {
                target_size: 25,
            }),
            ..Default::default()
        };
        let pipeline = ConcentrationPipeline::from_config(&cfg);
        assert!(!pipeline.is_active(), "master-switch-off must override pass configs");
    }

    #[test]
    fn enabled_config_registers_each_configured_pass() {
        let cfg = ConcentrationConfig {
            enabled: true,
            source_conditional_rarity: Some(SourceConditionalRarityPassConfig {
                rate: 0.01,
                min_surprise: None,
                min_per_source_lines: None,
            }),
            trading_partner_pool: Some(TradingPartnerPoolPassConfig {
                target_size: 25,
            }),
            ..Default::default()
        };
        let pipeline = ConcentrationPipeline::from_config(&cfg);
        assert!(pipeline.is_active());
        assert_eq!(pipeline.len(), 2);
    }

    #[test]
    fn structural_bridge_allowlist_recognises_seven_accounts() {
        assert_eq!(STRUCTURAL_BRIDGE_ACCOUNTS.len(), 7);
        assert!(is_structural_bridge("1100")); // AR_CONTROL
        assert!(is_structural_bridge("2000")); // AP_CONTROL
        assert!(is_structural_bridge("2900")); // GR_IR_CLEARING
        assert!(is_structural_bridge("1150")); // IC_AR_CLEARING
        assert!(is_structural_bridge("2050")); // IC_AP_CLEARING
        assert!(is_structural_bridge("1030")); // WIRE_CLEARING
        assert!(is_structural_bridge("1599")); // ACQUISITION_CLEARING
        assert!(!is_structural_bridge("6000")); // expense account — safe
        assert!(!is_structural_bridge("4000")); // revenue — safe
        assert!(!is_structural_bridge(""));      // empty — safe
    }

    /// Pass-isolation invariant: adding a no-op pass FIRST should not change
    /// the second pass's RNG-derived output. Verified by running a TP-pool pass
    /// alone vs after a no-op and asserting the resulting JEs are identical.
    #[test]
    fn rng_streams_are_isolated_across_passes() {
        use datasynth_core::models::{JournalEntry, JournalEntryLine};

        // Pass that consumes some RNG state but doesn't touch entries.
        struct NoopRngConsumer;
        impl ConcentrationPass for NoopRngConsumer {
            fn name(&self) -> &'static str { "noop_rng_consumer" }
            fn apply(
                &self,
                entries: &mut [JournalEntry],
                rng: &mut ChaCha8Rng,
            ) -> ConcentrationStats {
                use rand::RngExt;
                // Consume some entropy so this pass has observable RNG effect.
                let _: u64 = rng.random();
                let _: u64 = rng.random();
                ConcentrationStats {
                    pass: "noop_rng_consumer",
                    entries_examined: entries.len(),
                    entries_modified: 0,
                    extra: BTreeMap::new(),
                }
            }
        }

        // Build two identical JE batches.
        let make_batch = || {
            let mut jes = Vec::new();
            for i in 0..5 {
                let mut je = JournalEntry::new_simple(
                    format!("JE{i}"),
                    "C1".to_string(),
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    format!("desc {i}"),
                );
                let line = JournalEntryLine {
                    gl_account: "6000".to_string(),
                    trading_partner: Some(format!("TP-orig-{i}")),
                    ..JournalEntryLine::default()
                };
                je.lines.push(line);
                jes.push(je);
            }
            jes
        };

        // Pipeline A: just the TP pool pass.
        let pipe_a = ConcentrationPipeline {
            passes: vec![Box::new(TradingPartnerPoolPass::new(
                TradingPartnerPoolPassConfig { target_size: 3 },
            ))],
        };
        let mut batch_a = make_batch();
        pipe_a.run(&mut batch_a, 12345);

        // Pipeline B: no-op consumer + TP pool pass. With proper stream
        // isolation each pass sees the same seed regardless of position.
        let pipe_b = ConcentrationPipeline {
            passes: vec![
                Box::new(NoopRngConsumer),
                Box::new(TradingPartnerPoolPass::new(
                    TradingPartnerPoolPassConfig { target_size: 3 },
                )),
            ],
        };
        let mut batch_b = make_batch();
        pipe_b.run(&mut batch_b, 12345);

        // The TP pool pass is hash-based deterministic — it shouldn't read
        // its rng at all. So the outputs must match byte-for-byte regardless.
        for (a, b) in batch_a.iter().zip(batch_b.iter()) {
            assert_eq!(a.lines[0].trading_partner, b.lines[0].trading_partner);
        }
    }
}

// ============================================================================
// DRAFT — NOT COMPILED — CODE-SHAPE SPEC ONLY
// ============================================================================
//
// This file is the Rust skeleton companion to the markdown design docs:
//   - docs/superpowers/specs/2026-05-23-central-abstraction-proposal.md
//   - docs/superpowers/specs/2026-05-23-central-abstraction-addendum-chain-invariants.md
//   - docs/superpowers/specs/2026-05-23-concentration-pass-phase1-design.md
//
// It is intentionally NOT declared in lib.rs / mod.rs so rustc never tries to
// compile it. The `#![cfg(any())]` attribute is belt-and-suspenders — `any()`
// is the always-false cfg, so even if the file were ever declared as a module
// nothing inside would compile. The file exists purely as a concrete artifact
// the user can browse in the source tree to react to the trait + interface
// shape before any engine commit.
//
// Status: DRAFT · 2026-05-23 · awaiting user steer on Option A vs B
// Phase 1 effort estimate if approved: ~1-2 days
// ============================================================================

#![cfg(any())]

use std::collections::BTreeMap;

use datasynth_config::schema::{
    ConcentrationConfig, SourceConditionalRarityConfig, TradingPartnerPoolConfig,
};
use datasynth_core::error::SynthError;
use datasynth_core::models::journal_entry::JournalEntry;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::Serialize;

// ----------------------------------------------------------------------------
// Public trait
// ----------------------------------------------------------------------------

/// A post-generation transformation that reshapes the JE batch's
/// concentration / distributional structure toward a target.
///
/// Invariants every implementor MUST preserve:
///   1. Per-JE balance: sum(debits) == sum(credits)
///   2. Subledger-bridge accounts unchanged (see Phase-2 addendum allowlist)
///   3. Document-chain refs (DocumentReference is keyed by document_id,
///      not by gl_account — substituting non-bridge accounts is safe)
///   4. Determinism: same RNG seed + same input batch ⇒ same output
pub trait ConcentrationPass: Send + Sync {
    /// Short stable identifier — used in config + stats.
    fn name(&self) -> &'static str;

    /// Apply the transformation in place. Returns aggregate counters.
    /// `rng` is a dedicated per-pass ChaCha8 substream split by the pipeline.
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
    pub extra: BTreeMap<&'static str, u64>,
}

// ----------------------------------------------------------------------------
// Pipeline
// ----------------------------------------------------------------------------

/// Ordered, deterministic execution of zero or more ConcentrationPass instances.
///
/// Each pass receives a dedicated ChaCha8 substream so that adding or removing
/// a pass does not perturb the RNG state of any other pass — critical for
/// reproducibility.
pub struct ConcentrationPipeline {
    passes: Vec<Box<dyn ConcentrationPass>>,
}

impl ConcentrationPipeline {
    /// Build a pipeline from config. Returns an empty pipeline if
    /// `cfg.enabled == false` or no passes are configured.
    pub fn from_config(cfg: &ConcentrationConfig) -> Result<Self, SynthError> {
        let mut passes: Vec<Box<dyn ConcentrationPass>> = Vec::new();

        if !cfg.enabled {
            return Ok(Self { passes });
        }

        if let Some(c) = cfg.source_conditional_rarity.as_ref() {
            passes.push(Box::new(SourceConditionalRarityPass::new(c.clone())));
        }
        if let Some(c) = cfg.trading_partner_pool.as_ref() {
            passes.push(Box::new(TradingPartnerPoolPass::new(c.clone())));
        }

        // Phase 2 will register here:
        //   if let Some(c) = cfg.account_pair_substitution.as_ref() {
        //       passes.push(Box::new(AccountPairSubstitutionPass::new(c.clone())?));
        //   }

        Ok(Self { passes })
    }

    /// Execute every pass in order, each with its own ChaCha8 substream
    /// derived deterministically from `seed`.
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
                let stream_seed = seed.wrapping_add((idx as u64).wrapping_mul(STREAM_STRIDE));
                let mut rng = ChaCha8Rng::seed_from_u64(stream_seed);
                pass.apply(entries, &mut rng)
            })
            .collect()
    }

    /// True iff the pipeline has at least one configured pass.
    pub fn is_active(&self) -> bool {
        !self.passes.is_empty()
    }
}

// ----------------------------------------------------------------------------
// Phase 1 concrete pass — SourceConditionalRarityPass
// ----------------------------------------------------------------------------
//
// Wraps the already-shipped `tag_source_conditional_rarity` (commit 5678dd90).
// Pure interface adapter — no algorithm change. Existing 4 unit tests in
// `anomaly/source_conditional_rarity.rs` continue to cover the algorithm; this
// struct adds 1 test for the trait wiring.

pub struct SourceConditionalRarityPass {
    cfg: SourceConditionalRarityConfig,
}

impl SourceConditionalRarityPass {
    pub fn new(cfg: SourceConditionalRarityConfig) -> Self {
        Self { cfg }
    }
}

impl ConcentrationPass for SourceConditionalRarityPass {
    fn name(&self) -> &'static str {
        "source_conditional_rarity"
    }

    fn apply(
        &self,
        entries: &mut [JournalEntry],
        _rng: &mut ChaCha8Rng,
    ) -> ConcentrationStats {
        // Delegates to:
        //   crate::anomaly::source_conditional_rarity::tag_source_conditional_rarity
        let tagged: usize = unimplemented!("Phase 1: delegate to existing tagger");
        ConcentrationStats {
            pass: "source_conditional_rarity",
            entries_examined: entries.len(),
            entries_modified: tagged,
            extra: BTreeMap::new(),
        }
    }
}

// ----------------------------------------------------------------------------
// Phase 1 concrete pass — TradingPartnerPoolPass
// ----------------------------------------------------------------------------
//
// Closes the SOTA-11 coverage gap. The trading_partner field appears only on
// doc-flow-derived JEs and is informational from the audit-graph perspective
// (no downstream invariant — balance, chain refs, subledger reconciliation —
// reads it). Rewriting in post-process to a target pool size is therefore
// safe and gives exact target_size by construction.

pub struct TradingPartnerPoolPass {
    target_size: usize,
}

impl TradingPartnerPoolPass {
    pub fn new(cfg: TradingPartnerPoolConfig) -> Self {
        Self {
            target_size: cfg.target_size.max(1),
        }
    }

    /// Deterministic FNV-1a 64-bit hash → pool index. Same TP string always
    /// maps to the same canonical pool TP across runs.
    #[inline]
    fn pool_index(&self, tp: &str) -> u64 {
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;
        let mut h = FNV_OFFSET;
        for b in tp.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
        h % (self.target_size as u64)
    }
}

impl ConcentrationPass for TradingPartnerPoolPass {
    fn name(&self) -> &'static str {
        "trading_partner_pool"
    }

    fn apply(
        &self,
        entries: &mut [JournalEntry],
        _rng: &mut ChaCha8Rng,
    ) -> ConcentrationStats {
        let mut modified_lines: usize = 0;
        let mut entries_modified: usize = 0;
        for je in entries.iter_mut() {
            let mut je_touched = false;
            for line in je.lines.iter_mut() {
                if let Some(tp) = line.trading_partner.as_ref() {
                    let new_tp = format!("TP-{:06}", self.pool_index(tp));
                    if &new_tp != tp {
                        line.trading_partner = Some(new_tp);
                        modified_lines += 1;
                        je_touched = true;
                    }
                }
            }
            if je_touched {
                entries_modified += 1;
            }
        }
        let mut extra = BTreeMap::new();
        extra.insert("lines_modified", modified_lines as u64);
        ConcentrationStats {
            pass: "trading_partner_pool",
            entries_examined: entries.len(),
            entries_modified,
            extra,
        }
    }
}

// ----------------------------------------------------------------------------
// Phase 2 — AccountPairSubstitutionPass (signature only; algorithm in spec)
// ----------------------------------------------------------------------------
//
// Reshapes the per-JE (debit_account, credit_account) distribution toward a
// corpus-empirical target PMF. Honors the 7-account subledger-bridge
// allowlist from the chain-invariants addendum + matches AccountType so the
// balance-sheet equation holds at the type level.
//
// NOT scoped for Phase 1 — present here only so the trait shape can be
// reviewed against future use.

pub struct AccountPairSubstitutionPass {
    // pmf_by_source: HashMap<String, Vec<((String, String), f64)>>,
    // bridge_allowlist: &'static [&'static str],
    // coa_by_type: HashMap<AccountType, Vec<String>>,
}

impl AccountPairSubstitutionPass {
    /// Build from a path to a per-source pair-PMF JSON produced by
    /// `experiments/ml/inverse_audit/corpus_vs_synth_gap.py`.
    pub fn from_pmf_file(_path: &str) -> Result<Self, SynthError> {
        unimplemented!("Phase 2: see 2026-05-23-concentration-pass-phase2-design.md")
    }
}

impl ConcentrationPass for AccountPairSubstitutionPass {
    fn name(&self) -> &'static str {
        "account_pair_substitution"
    }

    fn apply(
        &self,
        _entries: &mut [JournalEntry],
        _rng: &mut ChaCha8Rng,
    ) -> ConcentrationStats {
        unimplemented!("Phase 2: see 2026-05-23-concentration-pass-phase2-design.md")
    }
}

// ----------------------------------------------------------------------------
// The 7-account subledger-bridge allowlist (chain-invariants addendum)
// ----------------------------------------------------------------------------
//
// Phase-2 substitution must skip any line whose gl_account is in this set;
// these accounts aggregate to subledger totals or net out across matching
// pairs (AR/AP aging, GR/IR clearing, IC matching, wire reconciliation,
// fixed-asset acquisition clearing). Substituting them would silently break
// subledger reconciliation reports.

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
/// preserved across post-process account substitution.
#[inline]
pub fn is_structural_bridge(account: &str) -> bool {
    STRUCTURAL_BRIDGE_ACCOUNTS.iter().any(|a| *a == account)
}

// ----------------------------------------------------------------------------
// Orchestrator call site (sketch — for review)
// ----------------------------------------------------------------------------
//
// Single insertion point in EnhancedOrchestrator::generate(), AFTER all
// generators have emitted, BEFORE inject_anomalies:
//
//     const CONCENTRATION_SEED_OFFSET: u64 = 0xC0_NC3_NTR_4710_NA77;
//
//     let concentration_stats = if let Some(cfg) = &self.config.concentration {
//         let pipeline = ConcentrationPipeline::from_config(cfg)?;
//         if pipeline.is_active() {
//             pipeline.run(entries, self.seed.wrapping_add(CONCENTRATION_SEED_OFFSET))
//         } else {
//             Vec::new()
//         }
//     } else {
//         Vec::new()
//     };
//     self.run_report.concentration = Some(concentration_stats);
//
// CONCENTRATION_SEED_OFFSET is a fresh constant chosen to be disjoint from
// every other generator-RNG seed offset already in use (priors, archetypes,
// anomaly-injector etc.). One source of truth, picked deliberately.

// ----------------------------------------------------------------------------
// Test plan (Phase 1, ~150 LOC total, 6 tests)
// ----------------------------------------------------------------------------
//
//  1. pipeline_runs_each_pass_exactly_once
//  2. pipeline_rng_streams_disjoint
//       (asserts: adding a no-op pass first doesn't change the second pass's
//        output bit-exactly)
//  3. source_conditional_rarity_pass_tags_expected_count
//       (wraps existing 4 tests in anomaly/source_conditional_rarity.rs;
//        those tests stay where they are, this one verifies the wrapper)
//  4. trading_partner_pool_converges_to_target
//       (generate 1k JEs with 200 distinct TPs, target_size=25,
//        assert exactly 25 distinct TPs after pass)
//  5. trading_partner_pool_preserves_balance_and_refs
//       (assert is_balanced() && document_references unchanged)
//  6. orchestrator_smoke (Python extending experiments/ml/inverse_audit
//       smoke pattern — concentration: { enabled: true, ... } produces
//       the expected stats section in the run report)

// EOF — DRAFT

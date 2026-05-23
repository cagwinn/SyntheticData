# ConcentrationPass — Phase 1 design (Option B scaffolding)

**Status:** DRAFT SPEC · 2026-05-23 · NOT engine code
**Companion to:** `2026-05-23-central-abstraction-proposal.md` +
                  `2026-05-23-central-abstraction-addendum-chain-invariants.md`
**Closes:** parent proposal's Phase 1 sketch question
**Effort estimate (Phase 1 alone):** ~1–2 days

This document is the code-shape spec the user can react to before any engine
commit. It defines the `ConcentrationPass` trait, the orchestrator call site,
two initial concrete passes, and the test plan. No code outside this file
moves until the user steers.

## Why a trait, not a function

The SOTA-N round produced three candidate concentration mechanisms — source-
conditional rarity tagging, TP-pool resizing, and (Phase 2) account-pair
substitution. Two of them are already implemented as ad-hoc post-process
functions called from the orchestrator. The trait formalises that pattern so:

1. **Each lever lands as one new struct + impl** — no orchestrator surgery,
   no generator-by-generator wiring (the failure mode of SOTA-8/-11).
2. **Pipeline composition is explicit** — the orchestrator iterates a `Vec`
   of passes, each with a deterministic RNG stream.
3. **Stats are collected uniformly** — every pass returns a `ConcentrationStats`
   so the orchestrator's run report has one consistent section.

## The trait

```rust
// crates/datasynth-generators/src/concentration/mod.rs   (Phase 1 — new file)

use datasynth_core::models::journal_entry::JournalEntry;
use rand_chacha::ChaCha8Rng;

/// A post-generation transformation that reshapes the JE batch's
/// concentration / distributional structure toward a target.
///
/// Invariants every implementor MUST preserve:
///   1. Per-JE balance: sum(debits) == sum(credits)
///   2. Subledger-bridge accounts unchanged (see Phase-2 addendum)
///   3. Document-chain refs (DocumentReference is keyed by document_id,
///      not by gl_account — substituting non-bridge accounts is safe)
///   4. Determinism: same RNG seed + same input batch ⇒ same output
pub trait ConcentrationPass: Send + Sync {
    /// Short stable identifier — used in config + stats.
    fn name(&self) -> &'static str;

    /// Apply the transformation in place. Returns aggregate counters.
    /// `rng` is a dedicated per-pass ChaCha8 substream (orchestrator splits).
    fn apply(
        &self,
        entries: &mut [JournalEntry],
        rng: &mut ChaCha8Rng,
    ) -> ConcentrationStats;
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ConcentrationStats {
    pub pass: &'static str,
    pub entries_examined: usize,
    pub entries_modified: usize,
    /// Optional pass-specific counters (e.g. lines_substituted).
    pub extra: std::collections::BTreeMap<&'static str, u64>,
}
```

## The pipeline

```rust
// crates/datasynth-generators/src/concentration/pipeline.rs   (Phase 1)

pub struct ConcentrationPipeline {
    passes: Vec<Box<dyn ConcentrationPass>>,
}

impl ConcentrationPipeline {
    pub fn from_config(
        cfg: &datasynth_config::schema::ConcentrationConfig,
    ) -> Result<Self, datasynth_core::error::SynthError> {
        let mut passes: Vec<Box<dyn ConcentrationPass>> = Vec::new();

        // Phase 1: two concrete passes.
        if let Some(c) = &cfg.source_conditional_rarity {
            passes.push(Box::new(SourceConditionalRarityPass::new(c.clone())));
        }
        if let Some(c) = &cfg.trading_partner_pool {
            passes.push(Box::new(TradingPartnerPoolPass::new(c.clone())));
        }

        // Phase 2 will register AccountPairSubstitutionPass here.
        Ok(Self { passes })
    }

    pub fn run(
        &self,
        entries: &mut [JournalEntry],
        seed: u64,
    ) -> Vec<ConcentrationStats> {
        self.passes
            .iter()
            .enumerate()
            .map(|(idx, pass)| {
                // Each pass gets its own ChaCha8 substream — adding a pass
                // doesn't perturb downstream passes' RNG state.
                let mut rng = ChaCha8Rng::seed_from_u64(seed.wrapping_add(idx as u64 * 0x9E37_79B9));
                pass.apply(entries, &mut rng)
            })
            .collect()
    }
}
```

## The orchestrator call site

Single insertion point, after all generators have emitted, before
`inject_anomalies`:

```rust
// crates/datasynth-runtime/src/enhanced_orchestrator.rs
// (Phase 1 insertion — REPLACES the current SOTA-12-only wire-in)

// ... existing generation phases ...

// Run concentration passes (post-process distribution reshaping).
let concentration_stats = if let Some(cfg) = &self.config.concentration {
    if cfg.enabled {
        use datasynth_generators::concentration::ConcentrationPipeline;
        let pipeline = ConcentrationPipeline::from_config(cfg)?;
        pipeline.run(entries, self.seed.wrapping_add(CONCENTRATION_SEED_OFFSET))
    } else {
        Vec::new()
    }
} else {
    Vec::new()
};
self.run_report.concentration = Some(concentration_stats);

// ... existing anomaly_injection phase ...
```

`CONCENTRATION_SEED_OFFSET` is a fresh constant (e.g. `0x_C0NC_3NTR_4710N_u64`)
that keeps the pass RNG streams disjoint from every existing generator-RNG
seed (priors, archetypes, anomaly-injector etc.).

## Phase 1 concrete passes

### SourceConditionalRarityPass

Lifts the already-shipped `tag_source_conditional_rarity` (commit 5678dd90)
into a pass. Pure wrapper — no algorithm change.

```rust
pub struct SourceConditionalRarityPass {
    cfg: SourceConditionalRarityConfig, // existing type
}

impl ConcentrationPass for SourceConditionalRarityPass {
    fn name(&self) -> &'static str { "source_conditional_rarity" }

    fn apply(&self, entries: &mut [JournalEntry], _rng: &mut ChaCha8Rng) -> ConcentrationStats {
        let tagged = tag_source_conditional_rarity(entries, &self.cfg);
        ConcentrationStats {
            pass: "source_conditional_rarity",
            entries_examined: entries.len(),
            entries_modified: tagged,
            extra: Default::default(),
        }
    }
}
```

Config migration path: `anomaly_injection.source_conditional_rarity_rate`
stays as the source of truth in Phase 1 (back-compat); Phase 3 introduces the
unified `concentration:` config DSL and the old key becomes a deprecated alias.

### TradingPartnerPoolPass

Closes the SOTA-11 coverage blocker — the `tp_set_size` gap (corpus ~12 vs
synth ~40) is purely a count of distinct strings in the `trading_partner`
column. Rewriting in post-process is safe because no downstream invariant
(balance, chain refs, subledger reconciliation) reads `trading_partner`.

```rust
pub struct TradingPartnerPoolPass {
    target_size: usize,
    // 64-bit hash of the original TP string indexes into a synthetic pool
    // of size `target_size` — deterministic, balance-preserving.
}

impl ConcentrationPass for TradingPartnerPoolPass {
    fn name(&self) -> &'static str { "trading_partner_pool" }

    fn apply(&self, entries: &mut [JournalEntry], _rng: &mut ChaCha8Rng) -> ConcentrationStats {
        let mut modified = 0usize;
        for je in entries.iter_mut() {
            for line in &mut je.lines {
                if let Some(tp) = &line.trading_partner {
                    let h = fnv1a_64(tp.as_bytes()) % (self.target_size as u64);
                    let new_tp = format!("TP-{:06}", h);
                    if new_tp != *tp {
                        line.trading_partner = Some(new_tp);
                        modified += 1;
                    }
                }
            }
        }
        ConcentrationStats {
            pass: "trading_partner_pool",
            entries_examined: entries.len(),
            entries_modified: modified, // counts LINES modified
            extra: Default::default(),
        }
    }
}
```

Effect on Round 0: `tp_set_size` drops from ~40 to exactly `target_size` by
construction; no other metric moves (TP appears only on doc-flow-derived JEs
and is informational from the audit-graph perspective).

## Config schema (Phase 1)

Additive — no breaking change:

```yaml
concentration:
  enabled: true                    # default false
  source_conditional_rarity:       # Phase 1 wrapper, mirrors existing config
    rate: 0.01
  trading_partner_pool:            # Phase 1 new
    target_size: 25
  # account_pair_substitution:     # Phase 2 — not yet wired
  #   target_pmf_path: ./corpus_pmfs.json
```

Rust schema additions (4 fields, all `Option<_>`):

```rust
// crates/datasynth-config/src/schema.rs
pub struct GenerationConfig {
    // ... existing ...
    pub concentration: Option<ConcentrationConfig>,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct ConcentrationConfig {
    #[serde(default)]
    pub enabled: bool,
    pub source_conditional_rarity: Option<SourceConditionalRarityConfig>,
    pub trading_partner_pool: Option<TradingPartnerPoolConfig>,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct TradingPartnerPoolConfig {
    pub target_size: usize,
}
```

## Test plan (Phase 1)

| test                                                | lives in                                                    |
|-----------------------------------------------------|-------------------------------------------------------------|
| pipeline runs each pass exactly once                | `datasynth-generators/src/concentration/pipeline.rs` (#[cfg(test)]) |
| RNG isolation — adding a pass doesn't perturb downstream | same                                                       |
| SourceConditionalRarityPass tags expected count     | wraps existing 4 tests in `anomaly/source_conditional_rarity.rs` |
| TradingPartnerPoolPass converges to target_size     | new — generate 1k JEs with 200 distinct TPs, target_size=25, assert distinct count = 25 |
| TradingPartnerPoolPass preserves balance + refs     | new — assert is_balanced() + document_references unchanged  |
| Orchestrator smoke (Python)                         | extends `experiments/ml/inverse_audit/smoke_*.py` pattern   |

Six tests total, ~150 LOC. Composes with the existing 4 SOTA-12 tests (which
move from `anomaly/` to `concentration/` as the pass owns them).

## Migration / back-compat

| key                                                      | Phase 1                                                  | Phase 3                                              |
|----------------------------------------------------------|----------------------------------------------------------|------------------------------------------------------|
| `anomaly_injection.source_conditional_rarity_rate`       | still honored — populates `concentration.source_conditional_rarity.rate` if `concentration:` absent | deprecation warning; both still honored             |
| `concentration.source_conditional_rarity.rate`           | preferred when present                                   | preferred; logs warning if old key also set         |
| `concentration.trading_partner_pool.target_size`         | new                                                      | unchanged                                            |

## Open question for the user

This document **completes** the parent proposal's Phase 1 sketch. The
remaining decision is binary:

1. **Approve Phase 1 implementation** — I land the trait + pipeline + 2
   passes + 6 tests as one engine commit (~1-2 days), following the
   CLAUDE.md engine-commit discipline (lib tests + clippy before push).
2. **Steer differently** — Option A (threaded sampler), or stop the round.

Once Phase 1 is in main, Phase 2 (account-pair substitution with the
chain-invariants addendum's 7-account allowlist) and Phase 3 (config DSL +
follow-on levers) can ship independently as separate engine commits.

# SOTA-8 — source-conditional Dirichlet account-pair sampler

**Author:** capstone overnight loop (2026-05-23)
**Tasks:** #136 (in_progress) · informed by #135 Round 0 baseline
**Status:** spec + skeleton this round; full wiring next round

## Problem (from FINDINGS §14)

Round 0 quantified the corpus-vs-synth gap on source-conditional structure:

| metric                       | corpus med | synth | gap        |
|------------------------------|-----------:|------:|------------|
| src-cond entropy median      |       0.68 |  0.97 | **0.71×** — synth too uniform |
| src-cond entropy p10 (tight) |       0.47 |  0.79 | 0.60× — corpus has tight sources synth never reaches |
| accts / source median        |       23.5 |  5.0  | 4.70× — synth has too *few* accounts per source |

The corpus pattern is *many accounts per source, but concentrated on a few*. Synth
today does the opposite — few accounts per source, used uniformly.

This matters for the inverse-audit capstone because `source_cond_edge_surprise` was the
single dominant explainer for 100% of top-50 JEs in the prod-scale audit packet (§13).
Closing the source-conditional gap shrinks the OOD distance between synth and corpus,
sharpens the residual signal, and makes the capstone's relational arm trainable on
synthetic data that resembles auditor-relevant production patterns.

## Design

Per source string `s`, fit a Dirichlet-multinomial over (debit_account, credit_account)
pairs. Two hyperparameters:

- **`accts_per_source_target`** (default 25): expected number of distinct accounts in
  the source's pool. Drawn around this with mild jitter at init.
- **`concentration` α** (default 0.5): symmetric Dirichlet concentration. Lower → more
  concentrated (tighter, lower entropy); higher → more uniform.
  - α = 0.5 with `N_s` = 25 gives expected normalised entropy ≈ 0.65, matching corpus 0.68.

Init time, for each source `s`:
1. Pool: draw `N_s ~ max(2, round(accts_per_source_target × Lognormal(0, 0.3)))` distinct
   accounts from the global CoA, weighted by the existing account-Pareto so hot
   accounts are over-represented.
2. PMF: draw `p_s ~ Dirichlet(α · 1_{N_s})`.

Generation time, when a JE's source is `s`:
1. Draw debit account from `p_s` (account-set, sampled by `p_s`).
2. Draw credit account from `p_s`, conditional on debit (with simple "must differ"
   filter; richer joint pair distribution is a follow-on if Round 0 re-run shows the
   edges/je gap remains).

## Config schema

`crates/datasynth-config/src/schema.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceConditionalAccountPairConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_source_cond_concentration")]
    pub concentration: f64,            // Dirichlet α
    #[serde(default = "default_accts_per_source_target")]
    pub accts_per_source_target: usize,
}

fn default_source_cond_concentration() -> f64 { 0.5 }
fn default_accts_per_source_target() -> usize { 25 }

impl Default for SourceConditionalAccountPairConfig {
    fn default() -> Self { Self { enabled: false, ...defaults } }
}
```

Hung under `TransactionsConfig`. Default OFF (opt-in) so it doesn't move the synthetic
manifold under existing users.

## Sampler

`crates/datasynth-core/src/distributions/source_conditional_pair.rs`:

```rust
pub struct SourceConditionalPairSampler {
    per_source: HashMap<String, SourcePool>,
}

struct SourcePool {
    accounts: Vec<String>,         // N_s accounts in this source's pool
    cumulative_pmf: Vec<f64>,      // cumulative PMF for O(log N) sampling
}

impl SourceConditionalPairSampler {
    pub fn new(sources: &[String], all_accounts: &[String],
               account_weights: &[f64], cfg: &SourceConditionalAccountPairConfig,
               rng: &mut impl rand::RngCore) -> Self { ... }

    /// Draws (debit_account, credit_account) conditional on source.
    /// Returns None if the sampler hasn't been initialised for this source.
    pub fn sample_pair(&self, source: &str, rng: &mut impl rand::RngCore)
        -> Option<(String, String)> { ... }
}
```

Drift / reproducibility:
- Each source's PMF is initialised once at sampler construction with the engine's
  ChaCha8 RNG (same seed → same per-source pools).
- The sampler does **not** modify the existing global account-Pareto; it overlays on
  top via the je_generator integration point.

## Integration point

`crates/datasynth-generators/src/je_generator.rs`: account picking happens in the JE
construction loop. The sampler is consulted IFF
`cfg.transactions.source_conditional_account_pair.enabled` AND the JE has a known
source. If the sampler doesn't have the source, fall back to the existing picker.

A single `cond_pair_rng` stream (ChaCha8 seeded with `seed + 90_017` similar to other
sub-streams) so the main RNG sequence is byte-identical when the feature is OFF.

## Validation

1. **Unit tests in `source_conditional_pair.rs`**:
   - α=0.5, N=25 → expected normalised entropy in [0.55, 0.75] (matches corpus).
   - α=10, N=25 → entropy > 0.95 (diffuse).
   - Same seed → identical PMFs.
   - Sample distribution converges to the PMF as `n → ∞`.

2. **`cargo test -p datasynth-core --lib`** + **`cargo clippy -p datasynth-core
   -p datasynth-config --lib --tests`** before pushing the engine commit.

3. **Re-run Round 0** (`corpus_vs_synth_gap.py`) on a synthetic generation that has the
   feature enabled. Expected movements:
   - src-cond entropy median: 0.97 → ~0.68 (matches corpus).
   - accts/source median: 5 → ~25 (within target ±20%).
   - src-cond entropy p10 (tight): 0.79 → ~0.45 (matches corpus).
   - Other metrics should be near-unchanged (this lever only touches source-cond).

4. **Capstone synthetic regression**: re-run `python -m inverse_audit.run_capstone`
   with the feature OFF and ON. With OFF, §12 numbers (PR-AUC density 0.78, unified
   0.40) should be unchanged. With ON, the relational arm's per-feature ROC on
   `source_cond_edge_surprise` should *increase* (the feature is now meaningfully
   discriminative on synthetic data, not just uniform noise).

## Out of scope (separate follow-ons)

- Coupled debit-credit pair sampling (joint distribution): only if the simple
  per-leg sampling leaves the edges/je gap (SOTA-9 territory).
- Per-industry α defaults: the corpus shows industries with different tightness; if a
  single α default is good enough for the gap, we keep it as one knob.

# Central concentration abstraction — proposal (FINDINGS §15 follow-on)

**Status:** PROPOSAL · awaiting user decision · 2026-05-23
**Context:** §14 baseline + §15 scorecard. Three planned SOTA levers (#8, #11, plus
the `edges/je` long tail in #9) blocked by the same multi-generator coverage problem.

## Problem statement

The synthetic engine has ~7 independent generator modules (je_generator,
document_flow_p2p, document_flow_o2c, allocation/balance, period_close, subledger,
intercompany). Each does its own account / trading-partner / line-count selection.
A config knob added in one is silently bypassed by the others.

Round 0 confirmed empirically:
- `master_data.vendor.count = 20` ⇒ tp_set_size stays at ~35 (doc-flow generators
  use their own V-000001..V-000040 pool).
- SOTA-8 sampler in je_generator ⇒ `src-cond entropy` unchanged (allocation /
  doc-flow / period-close paths dominate).
- SOTA-9 archetype reuse @ 0.97 ⇒ `edges/je` only −15% (other generators emit fresh
  edges).

Every distributional-concentration lever we will want next is the *same shape* as
these — a global target distribution that needs to be enforced across all paths.

## Two competing designs

### Option A — Shared sampler threaded through all generators

A new `datasynth-core::concentration::ConcentrationContext` that every generator
holds a reference to. Each generator's "pick account" / "pick TP" / "pick line
count" call site consults the context first, falling back to its current logic.

Implementation: ~7 generator modules × ~3 call sites each ≈ 20 call-site changes,
each with their own `&mut` borrow shape. Tests + clippy + downstream regression
risk per touch.

Pros:
- The lever takes effect during generation; downstream consumers see the corrected
  distribution end-to-end.
- Composes cleanly with the generator's existing structure (priors, archetypes,
  Pareto) at the call site.

Cons:
- Large blast radius. Every generator's main code path changes. Per CLAUDE.md
  guidance on engine commits, each touch needs lib tests + clippy.
- The shared context complicates the orchestrator's RNG / config wiring.
- Long-tail of similar refactors as new levers arrive — each one adds a method
  to the context and call-site touches downstream.

### Option B — Post-process pass over the generated batch (recommended)

After every generator has emitted its JEs, a single module reshapes the batch's
distribution to match a target. For SOTA-8 + SOTA-11 + edges/je this looks like:

```
                 ┌─ generation (unchanged) ─┐    ┌─ concentration pass ──┐
  config ──────► │  je_generator            │ ─► │  for each JE:         │
                 │  document_flow_p2p       │    │    pick a (source,    │
                 │  document_flow_o2c       │    │    debit_acct, credit_│
                 │  allocation_batch        │    │    acct) substitution │
                 │  period_close            │    │    from a target PMF  │
                 │  subledger ...           │    │  validate balance     │
                 └──────────────────────────┘    └───────────────────────┘
```

Implementation: ONE new module + ONE call site (in the orchestrator's post-
generation pipeline, between generation and anomaly injection). Each "lever" is a
configurable transformation in that module.

Pros:
- **Single integration point.** Adding a new concentration lever is a function in
  one file — no generator-by-generator wiring.
- **Covers every JE by construction**, regardless of which generator emitted it.
- **Empirically validated approach** — SOTA-12 was designed this way and shipped
  cleanly with 4 tests in one round.
- **Preserves balance / data integrity** as long as substitutions preserve
  per-JE sum invariants (debit = credit). For account substitution, the line's
  amount stays — only the gl_account string changes.
- **Reversible / opt-in.** A user who wants the raw generator output disables the
  pass.

Cons:
- The pass is a "rewriter" — it doesn't capture the generation logic, just the
  output distribution. Some realism (e.g. allocation batches having specific cost-
  center patterns) might need pre-pass + post-pass coordination.
- Account substitution must preserve cross-process linkage (a payment's account must
  still match the linked invoice's). The pass needs to respect document-chain
  invariants the generators built up.

## Recommended path: B with a phased rollout

**Phase 1 — `ConcentrationPass` module + the orchestrator call site.** Module skeleton
that takes the JE batch + a `ConcentrationConfig` and routes per-lever transformations.
Initial transformations: (a) the SOTA-12 source-conditional-rarity tagger (already
shipped, just wire in), (b) tp_set_size enforcement (rewrites trading_partner strings
to a target pool size).

**Phase 2 — `edges/je` concentration.** Account-pair substitution per JE so the
aggregate edge frequency tracks a target distribution. Must respect document-chain
linkage (read off `document_references` if present; only substitute within a
chain-equivalence class).

**Phase 3 — extensible config DSL.** A `concentration: [ { type: ..., target: ... }, ... ]`
config list so future levers (source-conditional, hot-account Pareto follow-up,
period-end mass) drop in as one new variant per lever.

## Effort estimate

- Phase 1: ~1-2 days. Module + call site + 2 transformations + tests.
- Phase 2: ~2-3 days. The edge substitution + chain-linkage preservation are the
  hardest part. Round 0 re-validation included.
- Phase 3: ~1 day. Config DSL + 2-3 follow-on transformations.

Total ~1 week of focused work, lands SOTA-8.1 + SOTA-11.1 + future similar levers in
a single shared mechanism instead of N parallel coverage refactors.

## Why this NOW

The pattern is unambiguous after the SOTA-N round:
1. Levers that ship cleanly (SOTA-9, SOTA-10, SOTA-12) have **single integration
   points** by construction.
2. Levers that hit the wall (SOTA-8, SOTA-11) need to be threaded through all
   generators.
3. The remaining big gaps (edges/je 7×, tp_set_size 3.3×) are *exactly the same
   shape* as the blocked levers.

Option B's incremental cost is bounded; the per-lever pattern's cost compounds.

## Open question for the user

**Decision required:** approve Option B as the next round? (Or pick Option A if there's
a reason post-process can't preserve a specific generator invariant I haven't seen.
The biggest risk in Option B is the document-chain linkage in Phase 2 — if the
analytical answer is "we can't substitute accounts without breaking PO → GR → IR →
Payment refs," then Option A becomes the forced path.)

# ConcentrationPass — design landing page

**Status:** DESIGN PHASE COMPLETE · 2026-05-23 · awaiting user steer
**Decision required:** approve Phase 1+2 / approve Phase 1 only /
                       pick Option A / stop the round

This is the navigation entry point for the central-concentration-abstraction
roadmap (task #143). The design phase produced 5 specs over a single
overnight loop iteration; this page lists them in reading order so the user
can decide without hunting.

## Reading order

| # | doc | purpose | LOC |
|---|-----|---------|----:|
| 1 | [parent proposal](2026-05-23-central-abstraction-proposal.md) | problem statement; Option A (threaded sampler) vs Option B (post-process pass); recommendation = B | ~130 |
| 2 | [chain-invariants addendum](2026-05-23-central-abstraction-addendum-chain-invariants.md) | closes Option B's one decision-changing question: post-process account substitution preserves PO/GR/IR/Payment refs (the chain is keyed by `document_id`, NOT `gl_account`); the only invariant is a 7-account subledger-bridge allowlist | ~115 |
| 3 | [Phase 1 markdown design](2026-05-23-concentration-pass-phase1-design.md) | the `ConcentrationPass` trait + `ConcentrationPipeline` + orchestrator call site + 2 concrete passes (`SourceConditionalRarityPass` wrapping shipped SOTA-12, `TradingPartnerPoolPass` closing SOTA-11) + 6-test plan + back-compat migration | ~230 |
| 4 | [Phase 1 .rs code-shape draft](../../../crates/datasynth-generators/src/concentration_pass_draft.rs) | concrete Rust skeleton companion — interface-only, `#![cfg(any())]` guards compilation, unlinked from module tree; gives the user a tangible file to browse alongside the markdown | ~260 |
| 5 | [Phase 2 design](2026-05-23-concentration-pass-phase2-design.md) | `AccountPairSubstitutionPass` algorithm: corpus-PMF input format, 7-account allowlist guard, AccountType-matched substitution, balance-revalidation fallback, 5-test plan, trait integration | ~225 |

## What lands per chunk

### Phase 1 (~1 day, ~330 LOC, 5 tests)
- `ConcentrationPass` trait + `ConcentrationStats`
- `ConcentrationPipeline` with per-pass ChaCha8 substream isolation
- Orchestrator call site (single insertion point, after generation, before
  anomaly injection)
- `SourceConditionalRarityPass` — pure wrapper of the already-shipped
  `tag_source_conditional_rarity`
- `TradingPartnerPoolPass` — closes SOTA-11 `tp_set_size` coverage gap by
  rewriting `trading_partner` strings to a hash-indexed pool
- Additive config schema (4 `Option<_>` fields)
- Back-compat alias: `anomaly_injection.source_conditional_rarity_rate`
  remains honored as Phase-1 source of truth

### Phase 2 (~3 days, ~610 LOC, 5 tests)
- `corpus_vs_synth_gap.py --emit-pair-pmf` flag producing
  `corpus_pair_pmf.json` (per-source `(debit, credit)` PMFs; aggregate only)
- `AccountPairSubstitutionPass` honoring the 7-account allowlist + the
  AccountType invariant + `is_balanced()` belt-and-suspenders revert
- `round0_validation.py` smoke harness — regression guard that closes the
  loop with the Round-0 baseline

### Phase 3 (~1 day, ~150 LOC, 1 test)
- Unified `concentration: [{ type: ..., target: ... }, ...]` config DSL
- Deprecation warning on the old back-compat alias key
- Single-point registration for future levers

**Combined Phase 1+2+3:** ~5 days, ~1100 LOC, 11 tests. Matches the
parent proposal's "~1 week" estimate.

## What each lever closes

| lever                              | gap addressed                                     | source |
|------------------------------------|---------------------------------------------------|--------|
| `SourceConditionalRarityPass`      | already shipped (composes via trait)              | SOTA-12 |
| `TradingPartnerPoolPass`           | `tp_set_size` (synth ~40 → target, e.g. 12)       | SOTA-11.1 |
| `AccountPairSubstitutionPass`      | `edges/je` (synth ~7× corpus) + `src_cond_entropy` (synth too uniform) | SOTA-8.1 |

Closes tasks **#141, #142, #143** under a single mechanism instead of N
parallel coverage refactors per generator path.

## Why Option B (post-process) beat Option A (threaded sampler)

The SOTA-N round empirically proved the pattern:

| outcome | levers | why |
|---------|--------|-----|
| **ship cleanly** | SOTA-9, SOTA-10, SOTA-12 | single integration point by construction |
| **hit the wall** | SOTA-8, SOTA-11 | needed threading through all 7 generators |

The remaining big gaps (`edges/je 7×`, `tp_set_size 3.3×`) are *the same
shape* as the blocked levers. Option B's incremental cost is bounded; the
per-lever pattern's cost compounds. Option A is the forced path only if
post-process can't preserve some invariant — and the chain-invariants
addendum demonstrated it can (with the 7-account guard).

## Open decision for the user

Spec preparation is complete. The next action requires a steer:

1. **Approve Phase 1+2 combined** — ~4 days engine work, 10 tests, lands
   SOTA-8.1 + SOTA-11.1 + SOTA-12 unification + the `edges/je` gap closure
   in one round. Recommended.
2. **Approve Phase 1 only** — ~1 day, lower risk, defers Phase 2 until the
   PMF infrastructure proves out.
3. **Pick Option A** — threaded sampler instead; viable but more
   compounding cost per future lever.
4. **Stop the round** — hold off; the spec stays on the shelf until needed.

No engine commit happens until the user steers. The autonomous loop's
output is bounded to docs at this point — adding more design variants
would be makework.

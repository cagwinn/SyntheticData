# Central abstraction — addendum: document-chain invariants under Option B

**Companion to:** `2026-05-23-central-abstraction-proposal.md`
**Status:** ANALYSIS — closes the proposal's open question
**Date:** 2026-05-23

## The open question

The central-abstraction proposal raised one decision-changing question: can Phase-2
of Option B (post-process account substitution to close `edges/je`) preserve the
document chain refs (PO → GR → IR → Payment), or does that invariant force Option A
(threaded sampler)?

## What the chain actually depends on

Walking the codebase shows the chain link is **structurally decoupled from gl_account**:

`DocumentReference` (`datasynth-core/src/models/documents/document_chain.rs:140`)
is keyed by:

```rust
pub struct DocumentReference {
    pub source_doc_type: DocumentType,
    pub source_doc_id: String,         // document_id only — NOT gl_account
    pub target_doc_type: DocumentType,
    pub target_doc_id: String,         // document_id only — NOT gl_account
    pub reference_type: ReferenceType, // FollowOn / Payment / Reversal / ...
    // ...
}
```

So the **chain itself** is safe under account substitution: `references.json` is
written by document_id and never reads `gl_account`. Substituting an account on
a JE line does not break any `document_references` entry.

## Where account substitution *would* break things

The real invariant lives one level down — at the **subledger-to-GL bridge accounts**
(`datasynth-core/src/accounts.rs:30+`):

| account constant      | code   | role                                                |
|-----------------------|-------:|-----------------------------------------------------|
| `AR_CONTROL`          | `1100` | GL counterpart of AR subledger; AR aging totals     |
| `AP_CONTROL`          | `2000` | GL counterpart of AP subledger; AP aging totals     |
| `GR_IR_CLEARING`      | `2900` | Bridge between Goods Receipt and Invoice Receipt    |
| `IC_AR_CLEARING`      | `1150` | Bridge for intercompany AR matching                  |
| `IC_AP_CLEARING`      | `2050` | Bridge for intercompany AP matching                  |
| `WIRE_CLEARING`       | `1030` | Bridge for wire-transfer reconciliation              |
| `ACQUISITION_CLEARING`| `1599` | Bridge for fixed-asset acquisition / clearing        |

These are *aggregate* accounts whose balances must match a subledger total or net
out across a matching pair. If we substitute a `GR_IR_CLEARING` line with some
other account:
- The GR-side `2900` debit and the IR-side `2900` credit no longer net to zero.
- AP aging stops matching `AP_CONTROL` (line 2000) totals.
- Subledger reconciliation reports (the `subledger_reconciliation.json` / AR/AP
  aging output) report broken balances.

The rest of the accounts (revenue, expense, regular assets/liabilities, individual
COA detail accounts) carry **no cross-process dependency** — substituting them
only changes which expense/revenue line a particular cost-center hit. The chain
references and subledger reconciliation are entirely unaffected.

## Implication for Phase-2 ConcentrationPass

The substitution rule is one line:

```rust
const STRUCTURAL_BRIDGE_ACCOUNTS: &[&str] = &[
    "1100", "2000", "2900", "1150", "2050", "1030", "1599",
];

fn substitute_safe(line: &mut JournalEntryLine, target_acct: &str) {
    if STRUCTURAL_BRIDGE_ACCOUNTS.contains(&line.gl_account.as_str()) {
        return; // never substitute a bridge account — would break aging / matching
    }
    line.gl_account = target_acct.to_string();
}
```

That's the whole guard. Inside any JE, the bridge line stays untouched; the
offsetting line can be rewritten to match the target edge-distribution.

**Conservative additional guard** for safety: substitute only within the same
`AccountType` (Asset → Asset, Expense → Expense, etc.) so balance-sheet equation
holds at the type level. This is cheap — `coa.get_accounts_by_type(t)` already
exists.

## Verdict

**Option B (post-process ConcentrationPass) is structurally feasible** without
breaking the document chain — the chain doesn't depend on `gl_account`, only on
`document_id`. The subledger-bridge constraint is a 7-account allowlist guard;
the central abstraction's Phase-2 design just needs to honor it.

This unblocks the decision in the proposal. Recommendation stands: **Option B,
3-phase rollout, ~1 week total**.

## Open follow-on (Phase 2 detail)

For the actual substitution algorithm:
1. Aggregate target distribution: per JE source, the corpus-empirical
   `P(debit_account, credit_account | source)` (computed from a held-out sample —
   the `corpus_vs_synth_gap.py` we already have produces the per-source PMFs as a
   side-product).
2. For each generated JE, compute the current `(debit, credit)` pair's likelihood
   under that target; if low, propose a substitute pair (uniformly from the top-k
   target pairs).
3. Apply `substitute_safe` per line (which honors the bridge allowlist + the
   AccountType invariant).
4. Re-validate balance (existing `JournalEntry::is_balanced()` test). Reject
   substitutions that don't balance and fall back to original.

Effort estimate for Phase 2 alone: ~2 days (algorithm + the bridge allowlist guard
+ Round 0 re-validation). Phase 1 + Phase 3 unchanged from the parent proposal.

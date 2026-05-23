# ConcentrationPass — Phase 2 design (account-pair substitution)

**Status:** DRAFT SPEC · 2026-05-23 · NOT engine code
**Companion to:**
  - `2026-05-23-central-abstraction-proposal.md`
  - `2026-05-23-central-abstraction-addendum-chain-invariants.md`
  - `2026-05-23-concentration-pass-phase1-design.md`
**Effort estimate (Phase 2 alone):** ~2–3 days

This is the algorithm spec for the Phase 2 `AccountPairSubstitutionPass` — the
last big lever in the central-abstraction roadmap, and the one with non-trivial
correctness questions (chain refs, balance, type discipline). All decisions
that were "open" in the parent proposal are now closed by the chain-invariants
addendum + Phase 1 design + this document.

## What this pass does

For each generated JE, look at its dominant `(debit_account, credit_account)`
edge. If that pair is rare under the corpus-empirical PMF for the JE's
`source`, propose a substitute pair drawn from the target PMF and rewrite
**non-bridge** lines accordingly. Result: the synthetic batch's per-source
edge distribution converges toward the corpus distribution without
re-running any generator.

This is the central post-process answer to FINDINGS §15's `edges/je 7×` and
`src_cond_entropy` gaps — the two metrics SOTA-8 and SOTA-9 couldn't close
because they're each gated by one generator path of seven.

## (1) Corpus-PMF input format

### Source of truth

`experiments/ml/inverse_audit/corpus_vs_synth_gap.py` already computes the
ingredients (per-source `gl_account` distributions in `_source_cond_tightness`;
per-JE dominant `(debit_acct, credit_acct)` pairs in
`_edges_per_je_and_concentration`). Phase 2 adds a sibling output: per-source
joint PMF over those pairs.

### Extraction extension (Python)

Sketched as a follow-on to `corpus_vs_synth_gap.py` — does NOT modify the
existing gap-measurement output, lives in a new `--emit-pair-pmf` flag:

```python
def _source_pair_pmf(df: pd.DataFrame) -> dict[str, list]:
    """Per source: list of [debit_acct, credit_acct, probability], sorted desc by p.
    Only emits sources with >= MIN_JES_PER_SOURCE (default 50) — below that the
    PMF is too sparse to be useful and would overfit."""
    rows = []
    for src, grp in df.groupby("source"):
        pairs = _dominant_pairs(grp)        # reuse from gap.py
        if len(pairs) < MIN_JES_PER_SOURCE:
            continue
        counts = Counter(pairs)
        total = sum(counts.values())
        triples = [(d, c, n / total) for (d, c), n in counts.most_common()]
        rows.append({"source": str(src), "pmf": triples, "n_jes": total})
    return {"pmfs": rows, "min_jes_per_source": MIN_JES_PER_SOURCE}
```

### On-disk format (`corpus_pair_pmf.json`)

```json
{
  "schema_version": 1,
  "min_jes_per_source": 50,
  "produced_by": "corpus_vs_synth_gap.py --emit-pair-pmf",
  "pmfs": [
    {
      "source":  "<source-code>",
      "n_jes":   12345,
      "pmf": [
        ["<debit-acct>",  "<credit-acct>", 0.18],
        ["<debit-acct>",  "<credit-acct>", 0.11],
        ...
      ]
    },
    ...
  ]
}
```

**Privacy:** the file holds only aggregate account codes + probabilities. No
row content, no document IDs, no client identifiers. The Rust pass loads this
JSON at startup; the corpus path is a runtime-only argument and never reaches
a commit (same discipline as the existing gap script).

## (2) 7-account subledger-bridge allowlist guard

From the chain-invariants addendum — these accounts carry semantic load (they
aggregate to subledger totals or net across matching pairs). Substituting them
silently breaks AR/AP aging and reconciliation.

```rust
const STRUCTURAL_BRIDGE_ACCOUNTS: &[&str] = &[
    "1100", // AR_CONTROL
    "2000", // AP_CONTROL
    "2900", // GR_IR_CLEARING
    "1150", // IC_AR_CLEARING
    "2050", // IC_AP_CLEARING
    "1030", // WIRE_CLEARING
    "1599", // ACQUISITION_CLEARING
];

fn is_structural_bridge(acct: &str) -> bool {
    STRUCTURAL_BRIDGE_ACCOUNTS.iter().any(|a| *a == acct)
}
```

Hard rule: **if either side of the JE's dominant pair is a bridge account,
the pass skips the JE entirely.** This is the conservative cut — it leaves
the bridge-coupling JEs alone (which is what we want for subledger
reconciliation) at the cost of slightly reduced coverage. Round 0 data shows
bridge-touching JEs are ~12% of total in the canonical synthetic batch, so
~88% remain eligible — plenty of substrate.

## (3) AccountType-matched substitution invariant

Beyond the bridge allowlist, balance-sheet integrity at the type level
requires substitutions to swap like-for-like:

| current account type | substitute account type |
|----------------------|-------------------------|
| Asset                | Asset                   |
| Liability            | Liability               |
| Equity               | Equity                  |
| Revenue              | Revenue                 |
| Expense              | Expense                 |

Why: an Expense → Asset substitution would silently move money out of the
P&L into the balance sheet — the JE still balances per-row, but the
financial statements no longer reconcile.

Implementation:

```rust
// At pass construction, partition the CoA by AccountType.
let accts_by_type: HashMap<AccountType, Vec<String>> = coa.accounts_by_type();

// At substitution time, only propose a swap if the candidate's type matches
// the original line's account type.
fn type_match(orig_acct: &str, candidate: &str, coa: &ChartOfAccounts) -> bool {
    coa.lookup(orig_acct).map(|a| a.account_type)
        == coa.lookup(candidate).map(|a| a.account_type)
}
```

The corpus-PMF's pair preserves the original (debit, credit) typing by
construction (it's drawn from corpus JEs that themselves balance), so this
check is rarely violated — but the safety net catches CoA mismatches across
corpus and synthetic naming conventions.

## (4) Substitution algorithm + balance-revalidation fallback

```text
For each JE in entries:
    if dominant pair (d_orig, c_orig) touches any bridge account:
        skip                                            # rule (2)
    let pmf = self.pmf_for_source(je.source)?
        else skip                                       # no PMF coverage
    let p_orig = pmf.lookup((d_orig, c_orig)).unwrap_or(0.0)
    if p_orig >= self.rarity_threshold:                 # default 0.005
        skip                                            # already plausible
    draw a candidate pair (d_new, c_new) from pmf top-K (default K=10)
    if not type_match(d_orig → d_new) or not type_match(c_orig → c_new):
        retry up to RETRY_BUDGET; fall through to skip
    apply substitution:
        line[dominant_debit].gl_account  = d_new
        line[dominant_credit].gl_account = c_new
    if not je.is_balanced():                            # belt-and-suspenders
        revert; skip                                    # rule (4)
    counters.entries_modified += 1
    counters.extra["substitutions_applied"] += 1
```

Notes:
- Only the dominant debit + dominant credit lines are touched; other lines
  on the JE (additional debits/credits, tax lines, FX rounding) keep their
  accounts. This preserves both per-JE balance and any line-specific
  semantics generators may have attached.
- `is_balanced()` is the model's existing invariant check. We re-run it as
  belt-and-suspenders, even though the algorithm above can't unbalance a JE
  (substituting an account string changes neither debit_amount nor
  credit_amount).
- `rarity_threshold` and `top_k` are config — both are tunable knobs to
  trade off "how much we reshape" vs "how much we leave alone".

### Determinism

`AccountPairSubstitutionPass::apply` receives its own ChaCha8 substream
from the pipeline (see Phase 1 design — `STREAM_STRIDE` separation). The
top-K candidate draw is `WeightedIndex`-based, fully deterministic under
the seed. Same seed + same input batch + same PMF = bit-identical output.

## (5) Test plan (5 tests, ~200 LOC)

| test                                                              | lives in                                          |
|-------------------------------------------------------------------|---------------------------------------------------|
| `account_pair_substitution_skips_bridge_accounts`                 | `concentration/account_pair.rs` (#[cfg(test)])     |
| `account_pair_substitution_respects_account_type`                 | same — assert no Asset↔Expense leaks               |
| `account_pair_substitution_preserves_balance`                     | same — sweep 1000 random JEs, all `is_balanced()` post-pass |
| `account_pair_substitution_round_trip_determinism`                | same — same seed twice ⇒ bit-identical output     |
| `account_pair_substitution_closes_edges_per_je_gap` (Python smoke)| `experiments/ml/inverse_audit/round0_validation.py` (new) — runs corpus_vs_synth_gap.py before/after Phase 2 and asserts `edges_per_je` shifts toward corpus value |

One Python smoke alongside the four Rust unit tests. The smoke test
*regenerates* the Round 0 gap report on the same synthetic batch with and
without the pass, then asserts the delta moves in the expected direction
(directional assertion, not exact-value — gives us a CI-stable regression
guard for the lever's effectiveness).

## (6) Integration with the Phase 1 trait

`AccountPairSubstitutionPass` implements `ConcentrationPass` exactly as
sketched in `concentration_pass_draft.rs:225+`:

```rust
pub struct AccountPairSubstitutionPass {
    pmf_by_source: HashMap<String, SourcePairPmf>,  // loaded from JSON at construction
    bridge_allowlist: &'static [&'static str],       // = STRUCTURAL_BRIDGE_ACCOUNTS
    coa: Arc<ChartOfAccounts>,                       // for type lookup
    rarity_threshold: f64,                           // default 0.005
    top_k: usize,                                    // default 10
}

impl ConcentrationPass for AccountPairSubstitutionPass {
    fn name(&self) -> &'static str { "account_pair_substitution" }
    fn apply(&self, entries: &mut [JournalEntry], rng: &mut ChaCha8Rng) -> ConcentrationStats {
        // see algorithm in section (4)
    }
}
```

Pipeline registration (extending the Phase 1 `from_config`):

```rust
if let Some(c) = cfg.account_pair_substitution.as_ref() {
    let pass = AccountPairSubstitutionPass::from_pmf_file(&c.pmf_path, coa.clone(), c)?;
    passes.push(Box::new(pass));
}
```

Config schema addition (additive, Option):

```yaml
concentration:
  enabled: true
  account_pair_substitution:
    pmf_path:          ./corpus_pair_pmf.json   # produced by gap.py --emit-pair-pmf
    rarity_threshold:  0.005                    # default
    top_k:             10                       # default
```

```rust
pub struct AccountPairSubstitutionConfig {
    pub pmf_path: String,
    #[serde(default = "default_rarity_threshold")]
    pub rarity_threshold: f64,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}
```

## Phase 1 + Phase 2 effort if landed together

| chunk           | rough LOC | tests | time   |
|-----------------|----------:|------:|--------|
| Phase 1 trait + pipeline + orchestrator call site | ~150 | 2 | ~3-4h |
| SourceConditionalRarityPass (wrapper)             |  ~60 | 1 | ~1h   |
| TradingPartnerPoolPass                            | ~120 | 2 | ~3h   |
| Phase 1 subtotal                                  | **~330** | **5** | **~1d** |
| AccountPairSubstitutionPass + PMF loader          | ~400 | 4 | ~2d   |
| `corpus_vs_synth_gap.py --emit-pair-pmf` flag     |  ~60 | -- | ~1h   |
| `round0_validation.py` smoke harness              | ~150 | 1 | ~3h   |
| Phase 2 subtotal                                  | **~610** | **5** | **~3d** |
| **Combined**                                      | **~940 LOC** | **10** | **~4d** |

Phase 3 (config DSL unification + back-compat alias for the existing
`anomaly_injection.source_conditional_rarity_rate` key) is ~1 day on top —
the parent proposal's ~1-week total estimate still holds.

## Open decision for the user (same one)

This document closes the algorithmic open questions. The decision the user
needs to make is still binary:

1. **Approve combined Phase 1 + Phase 2 implementation** (~4 days engine
   work, lands SOTA-8.1 + SOTA-11.1 + SOTA-12 unification + edges/je gap
   closure in one round).
2. **Approve Phase 1 only** (~1 day), keep Phase 2 in spec until needed.
3. **Steer differently** — Option A (threaded sampler), or stop the round.

Recommendation unchanged: combined Phase 1 + Phase 2, Option B. The
spec preparation is complete; awaiting steer.

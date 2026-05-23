# SOTA-12 — source-conditional anomaly injection

**Tasks:** #140 (in_progress) · informed by Round 0 (#135) + FINDINGS §13 audit-packet finding
**Status:** spec + enum variant this round; injector strategy + config next round

## Motivation (from FINDINGS §13)

The corpus audit packet (174k-JE production-scale Health client) showed
`source_cond_edge_surprise_max` is the **#1 contributor for 100 % of top-50 JEs**.
The synthetic engine's existing anomaly types are global-rarity-based (`UnusualAccountPair`,
`NewCounterparty`) — none target the *source-conditional* rarity pattern auditors
actually flag. Closing this means ML models trained on synthetic data under-fit the
single most important real-world audit signal.

## What this lever does

Add a new `RelationalAnomalyType::SourceConditionalRarity` variant. An injected JE
of this type is one whose `(source, debit_account, credit_account)` triple has very
low frequency under the per-source marginal — i.e. the JE *would* be flagged as
high `source_cond_edge_surprise` by `inverse_audit/relational/graph_scorer.py` even
though its globally-marginal edge rarity is unremarkable.

## Design

Implementation is a **post-process over generated JEs** (decoupled from JE generation):

1. After generation, scan the JE set to build per-source `(source, gl_account)`
   frequencies (this is the empirical p(account | source)).
2. For each JE, compute its expected `-log P(account | source)` summed across its lines.
3. Sort JEs by that surprise score; pick the top X% as anomaly candidates.
4. Tag them with `AnomalyType::Relational(RelationalAnomalyType::SourceConditionalRarity)`.

This sidesteps the SOTA-8/11 architectural coverage problem because the post-process
runs **after** every generator has emitted its JEs — single integration point in the
orchestrator's anomaly-injection step.

## Config

`anomaly_injection.types.source_conditional_rarity`:
- `enabled`: bool (default false)
- `rate`: fraction of JEs to tag (default 0.01 — matches the audit packet's 1% hot list)
- `surprise_threshold`: minimum `-log P(edge | source)` to consider (default 5.0)

## Why this is the right level for the audit-realism feedback loop

Most other SOTA realism levers concentrate the synth's *normal* distribution toward
the corpus. SOTA-12 adds a class of *labelled anomalies* that match what auditors
actually flag — closing the synthetic-vs-real-audit gap from the other direction.
Combined with the capstone's relational graph residual (Stage 1 closeout, §12),
this lets the unified routed detector be trained + validated on synthetic data
whose labelled-anomaly *distribution* matches the corpus's residual fingerprint.

## Out of scope

- Per-source PMF *generation* during synthesis (that's SOTA-8 / SOTA-8.1).
- Other source-conditional anomaly classes (e.g. source-conditional duplicate-motif).
  Add as RelationalAnomalyType variants only if Round 0 measurement shows a need.

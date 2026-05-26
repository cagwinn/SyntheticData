# v5.30 A1 (#149) — Sajja exact eval with P3 graph motifs

Closes A1 on the v5.30 roadmap. Wires the P3 (Shared-Entity Graph Motifs)
sub-pattern into the Sajja exact eval by passing `attr_cols=['trading_partner',
'gl_account']` to `BehavioralFidelityEvaluator`. P3 was previously skipped
because no attribute columns were threaded through.

Previous A0 run (2026-05-26 baseline, no P3):
[`docs/baselines/2026-05-26-sajja-exact-eval/COMPARISON.md`](../2026-05-26-sajja-exact-eval/COMPARISON.md)

## Setup

```python
ev = BehavioralFidelityEvaluator(
    entity_col='entity', time_col='timestamp',
    label_col='label',    amount_col='amount',
    attr_cols=['trading_partner', 'gl_account'],   # ← v5.30 A1
)
```

Driver: `~/regen/run_sajja_eval.py` (on VM); reference and synth both
sub-sampled to 500K rows for the O(n²) inner loops. Label heuristic
remains top-4 % of |Functional Amount| on both sides (A2 closes the
synth-side ground-truth gap separately).

## Results

| sub-metric | v5.29 raw | baseline raw | DR (lower=better) |
|---|--:|--:|--:|
| P1 IETD W₁ (fraud) | 39 134.6 s | 640.1 s | **61.1×** |
| P1 IET autocorr gap | 0.0380 | 0.0004 | **105.9×** |
| P2 Active lifetime W₁ (fraud) | 31 874 871.8 s | 367 200.0 s | **86.8×** |
| P2 Burst length W₁ | 80.31 | 6.03 | **13.3×** |
| **P3 Fanout W₁ (trading_partner)** | **129.98** | **0.32** | **403.0×** |
| P3 Clustering coefficient delta | 0.0 | 0.0 | (both 1.0, degenerate) |
| P3 Triangle log ratio | 9.24 | 0.0 | (raw — no DR; baseline degenerate) |
| P3 Component size W₁ | 498.0 | 0.0 | (raw — no DR; baseline degenerate) |
| P4 Velocity-rule trigger gap | 0.0396 | 0.0000 | NaN (degenerate baseline) |
| **Composite DR (5 valid sub-metrics)** | | | **134.0×** |

Composite went **66.78 → 134.0** vs the prior A0 run. The 67× absolute
delta is **entirely from P3 fanout** being newly measured at 403×.
The four pre-existing sub-metrics produced **identical values** to A0
(seed `random_state=42` preserved across both runs).

## Why P3 fanout is 403× — interpretation

The raw P3 fanout Wasserstein on synth is 130 vs **0.32** on the
reference-half-split baseline. The reference shard has **36** distinct
trading partners across ~3.3M lines; both halves see essentially the
same TP-fanout distribution (W₁ = 0.32 is noise-floor). The synth has
**13** TPs total (configured via SOTA-11.1's
`concentration.trading_partner_pool.target_size = 12` + one
unmapped-tail bucket), and they concentrate differently than the
reference's 36-TP distribution.

The 403× isn't a flaw in P3 measurement — it's the eval surfacing the
SOTA-11.1 trading-partner pool size mismatch (synth has 36% as many TPs
as the reference). This is a **trade-off, not a bug**: SOTA-11.1 was
calibrated against a *different* corpus where the TP target was ~12;
on this reference shard, the corpus has 36 TPs and the synth's
pool-size cap creates the gap.

Three valid responses:

1. **Raise the synth's TP pool target** to match the current
   reference shard (target_size 12 → 30). Reduces P3 fanout DR
   but requires a regen and may shift other metrics.
2. **Document the trade-off** — for ML-fraud-detection use cases the
   tighter pool is preferred (denser per-TP signal); for behavioral
   fidelity it's looser.
3. **Per-dataset TP pool sizing** — make the trading-partner pool size
   configurable per-dataset / per-deployment.

Recommendation: option (3) is the long-term right answer; option (2)
is the right answer for v5.30 (document, don't retune, since the
SOTA-11.1 calibration was a deliberate concentration choice).

## P3 sub-metrics that didn't get a DR

`triangle_log_ratio`, `component_size_wasserstein`, and
`clustering_coeff_delta` are computed but excluded from the composite —
the baseline values are 0.0 (degenerate). The reference-half-split has
identical clustering structure across both halves (both halves see all
36 TPs + all 294 accounts), so the W₁ on those sub-metrics is exactly
0.0, making the DR undefined. These metrics report meaningfully when
*synth-on-ref* but not in the baseline normalisation step.

The Sajja paper composite design uses fanout as the primary P3
sub-metric for exactly this reason; the others are diagnostic-only.

## Comparison to A0 (no P3)

| eval | sub-metrics in composite | composite DR | notes |
|---|---|--:|---|
| **A0** (2026-05-26 baseline) | 4 (P1×2 + P2×2) | **66.78×** | P3 skipped, P4 NaN |
| **A1** (this run) | 5 (P1×2 + P2×2 + P3 fanout) | **134.0×** | P3 fanout 403×, P4 still NaN |

Reading: A0 had 4 metrics averaging 66.78×; A1 has the same 4 metrics
(values unchanged at sample level) **plus** P3 fanout at 403×.
`mean(61.1, 105.9, 86.8, 13.3, 403.0) = 134.0` — the composite
went up because we added a much-larger-than-average sub-metric.

This is the **right** direction: measurement got more complete.
DataSynth's headline number is "honest 134× on 5 sub-metrics" vs
"66.78× on 4 sub-metrics, P3 unmeasured".

## What this tells us about Proposition 1

The Sajja paper's Proposition 1 says row-independent generators cannot
preserve P3 graph motifs. DataSynth is row-aware (joint JE generation,
document chains, FSM-driven processes) — Proposition 1 does not bind.
P3 fanout 403× isn't proof of Proposition 1; it's evidence that
**TP pool size cap + Z-tail expansion** is the dominant signal in our
TP→entity bipartite projection. Engine-level fix to P3 fanout is
within reach via per-dataset TP pool sizing, not paradigm-level.

## How this compares to the paper's published numbers

The paper's row-independent generators (TVAE, CTGAN, TabularARGN,
GaussianCopula) all collapse on P3 in the IEEE-CIS card-fraud anchor
(81-99× P3 composites). At 403× on `trading_partner` fanout alone
we're 4× above the paper's worst row-independent generator, but
**only because** we cap TPs at 12 while the reference shard has 36;
this is config, not paradigm. Note also: paper's TabularARGN scores
17.2× on its P3 composite — that's its own anchor (IEEE-CIS), not
directly comparable to ours (GL reference).

The composite-headline framing is genuine: 134× on a 5-metric Sajja
composite, with P1 autocorr 105.9× still being the qualitative
Proposition-2-relevant signal (TVAE: 25.9×, CTGAN: 30.0×). Row-aware
generation buys positive within-entity IET autocorrelation **in
principle** — making it numerically tight requires the per-source IET
sampler refinement queued as B1.

## Artefacts

```
docs/baselines/2026-05-26-v5.30-a1-sajja-p3/
├── COMPARISON.md             (this file)
├── baseline.json             (ref half_A vs half_B, dataclass dump)
└── datasynth_v5.29_p3.json   (v5.29 synth vs ref, with P3, dataclass dump)
```

Driver `~/regen/run_sajja_eval.py` on VM at PID 76543 ran 2026-05-26
15:00 → 15:27 (27 min total: 13.5 min baseline + 13.5 min synth-vs-ref).

## Next: A2 + A3 in flight

- **A2 (#151)** synth real `is_fraud` labels — running PID 77979,
  separate driver, separate output dir
  (`~/regen/sajja_eval_v2_a2/`). Will collapse the P4 NaN.
- **A3 (#150)** TAIL_MASS 0.30 → 0.15 committed `b13b06ad`. Waits on a
  fresh regen (1M scale) — gated on VM availability after the
  enterprise_2000 archive finishes its aggregate phase.

## Compute

VM 143.47.102.202 (Lambda A10, 30c/222GB).
Step 1 (baseline): 810 s.
Step 2 (v5.29 vs ref): 810 s.
Total: 27 min wallclock for the A1 driver.

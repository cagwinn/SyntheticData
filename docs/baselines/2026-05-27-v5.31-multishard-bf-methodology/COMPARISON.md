# v5.31 T3 — multi-seed Sajja-eval methodology stability test

Three Sajja exact evals run in parallel against the same v5.30 A3 synth
output, varying only the half-split RNG seed (42, 123, 7) for the
reference-shard subsample + the ref-vs-ref baseline computation. **The
DRs are far less stable across seeds than the single-shard A3 result
(`docs/baselines/2026-05-26-v5.30-a3-tail-mass/COMPARISON.md`) implied.**

## Per-seed results

| sub-metric | seed=42 | seed=123 | seed=7 | mean | std | CV% |
|---|--:|--:|--:|--:|--:|--:|
| P1 IETD W₁ (fraud) | 57.95 | 38.35 | 15.76 | **37.35** | 21.11 | **56.5%** |
| P1 IET autocorr | 62.84 | 1.96 | 24.66 | **29.82** | 30.77 | **103.2%** ⚠️ |
| P2 Active lifetime | 89.48 | 80.66 | 101.97 | 90.70 | 10.71 | 11.8% |
| P2 Burst length | 12.40 | 12.09 | 11.95 | **12.15** | 0.23 | **1.9%** ✓ |
| P3 Fanout (TP) | 382.80 | 276.85 | 237.11 | 298.92 | 75.31 | 25.2% |
| P4 vrtrigger | NaN | NaN | NaN | (structural — see A2) |
| **Composite** | **121.10** | **81.98** | **78.29** | **93.79** | **23.72** | **25.3%** |

Range: 78 - 121, **delta 43 points**.

## Three research-grade findings

### 1. The headline composite is a high-side draw

The A3 COMPARISON.md reported `Composite Score: 121.10` as the
v5.30 baseline-to-beat. **That was seed=42, the highest of three
draws.** Mean across seeds is 93.79×; CV is 25.3 %. A more honest
single-number summary is **93.8× ± 23.7× (n=3)**, with a 95 %
half-width of ~27 points around the mean.

Implication: future v5.32+ improvements must beat the **mean** at
multiple seeds, not the single high-side seed=42 reading.

### 2. P1 IET autocorr is wildly unstable (CV 103 %)

| seed | P1 autocorr DR |
|---|--:|
| 42 | 62.84 |
| 123 | **1.96** |
| 7 | 24.66 |

seed=123 produces **DR = 1.96** — essentially at the noise floor.
seed=42 produces **62.84** — same metric, same synth, same reference.
The DR varies 32× across half-splits.

**B-tier consequences:**
- B1 (per-source IET refinement) targeted P1 autocorr's 62.84× DR.
  At seed=123 the metric is already at 1.96×; no improvement available.
- B3 (per-process fraud rates) achieved 20.7× on P1 autocorr at
  seed=42 — a celebrated drop. At seed=123 the BASELINE A3 was
  already 1.96×, so B3 might *raise* DR on that seed.

The P1 autocorr drop attributable to engine changes vs the noise
floor's natural variance is hard to disentangle from a single-shard
measurement. **Future B-tier work targeting P1 autocorr needs a
multi-seed evaluation to claim meaningful improvement.**

### 3. P2 burst length is a rock-solid methodology anchor (CV 1.9 %)

`p2_burstlen` lands at 12.40, 12.09, 11.95 across seeds — variance
under 5 %. This metric is genuinely measuring engine behavior, not
sampling noise. Engine work that moves P2 burst length is reliably
attributable.

P3 fanout (CV 25 %) and P2 active lifetime (CV 12 %) are
intermediate — directionally meaningful, but a single seed isn't
enough to claim a >25 % move.

## Implications for v5.30 Tier-B retrospective

| metric | A3→B-tier delta | T3 CV | claim status |
|---|--:|--:|---|
| P1 autocorr (B1 + B3 target) | 62.84→20.7 (-67 %) | 103 % | **NOT methodologically sound from 1 seed** |
| P3 fanout (A3 trade-off) | 382.80→same | 25 % | **borderline; multi-seed needed** |
| Composite (A3 vs A1) | 134→121 (-10 %) | 25 % | **borderline; could be seed noise** |

Reading: **v5.30's A1→A3 composite drop (134→121, −10 %) is within
the multi-seed noise band of A3 alone (78-121).** The structural
improvements from A3 may be smaller than the headline COMPARISON.md
suggested, OR they may be unmeasurable from a single-shard Sajja
eval at this sample size.

This **does NOT invalidate** the v5.30 engine changes — what landed
in A3 (TAIL_MASS reduction redistributing Z-tail mass) and B3
(per-process fraud rates) are real, deterministic engine changes
with clear mechanism. What T3 invalidates is the **methodology** of
treating single-shard half-split DRs as point estimates.

## Recommendation — eval methodology going forward

1. **Report multi-seed mean + CV** as the headline. Single-shard DRs
   are now flagged as "high-variance reference points only".
2. **Drop P1 autocorr from primary metrics** for engine-change
   attribution (CV too high). Keep it as a sub-metric in the
   composite, but don't optimise against it from one seed.
3. **Make P2 burst length the primary fidelity anchor** — its low CV
   makes it the most informative metric for engine work.
4. **B3 follow-up**: the P1 autocorr drop to 20.7× we celebrated may
   not be a real engine improvement. Re-run B3 at all three seeds
   to confirm.

## Compute

VM 143.47.102.202 (Lambda A10, 30c/222GB).
3 parallel Sajja evals (subsample 500K rows each side, attr_cols =
[trading_partner, gl_account]).
Wallclock: 21 min (parallel) — would be 78 min if serialised.

## Artefacts

```
docs/baselines/2026-05-27-v5.31-multishard-bf-methodology/
├── COMPARISON.md                  (this file)
├── aggregate.json                 (per-metric mean/std/CV across seeds)
├── seed_42/{baseline,synth}.json
├── seed_123/{baseline,synth}.json
└── seed_7/{baseline,synth}.json
```

Closes task #145 (multi-shard BF eval scaling).

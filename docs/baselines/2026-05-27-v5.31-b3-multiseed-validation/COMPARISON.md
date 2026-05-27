# B3 multi-seed validation — does the P1 autocorr drop hold?

Re-running the **B3 per-process fraud rates** Sajja eval at three RNG
seeds [42, 123, 7] to test whether the celebrated single-shard claim
("P1 autocorr 62.84 → 20.68, −67 %, TVAE-paper level") holds across
seeds — or whether it was a single-shard artefact per the T3
methodology finding (`docs/baselines/2026-05-27-v5.31-multishard-bf-methodology/`).

**Result: the B3 P1 autocorr drop is REAL but the single-shard claim
overstated its magnitude 2.7×. Mean drop is −16 (−53 %), not −42
(−67 %). P3 fanout regression confirms at all 3 seeds. Composite
regression confirms at all 3 seeds.**

## Side-by-side (A3 vs B3 across seeds)

| metric | A3 seed=42 | A3 seed=123 | A3 seed=7 | A3 mean | B3 seed=42 | B3 seed=123 | B3 seed=7 | B3 mean | Δ mean |
|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| P1 IETD W₁ | 57.95 | 38.35 | 15.76 | 37.35 | 59.81 | 41.15 | 14.13 | 38.36 | **+1.01 (+3 %)** |
| **P1 autocorr** | **62.84** | **1.96** | **24.66** | **29.82** | **20.68** | **0.28** | **21.02** | **13.99** | **−15.83 (−53 %)** ⭐ |
| P2 Active lifetime | 89.48 | 80.66 | 101.97 | 90.70 | 89.61 | 80.88 | 102.64 | 91.04 | +0.34 (+0.4 %) |
| P2 Burst length | 12.40 | 12.09 | 11.95 | 12.15 | 12.44 | 12.11 | 11.96 | 12.17 | +0.02 (+0.2 %) |
| **P3 Fanout** | **382.80** | **276.85** | **237.11** | **298.92** | **771.43** | **812.25** | **469.89** | **684.53** | **+385.61 (+129 %)** ⚠️ |
| **Composite** | **121.10** | **81.98** | **78.29** | **93.79** | **190.79** | **189.33** | **123.93** | **168.02** | **+74.23 (+79 %)** ⚠️ |

## The headline correction

What v5.30 B3 documented (commit `8c7dbc6d` COMPARISON.md):

> P1 autocorr DR drops 62.84× → 20.68× (-67%, TVAE-paper level)

What the multi-seed evidence shows:

> P1 autocorr DR mean drops 29.82 → 13.99 (-53%)
> - seed=42: 62.84 → 20.68 (-67%) ✓ matches v5.30 claim
> - seed=123: 1.96 → 0.28 (-86%, but absolute movement <2)
> - seed=7: 24.66 → 21.02 (-15%)

The **direction is correct** (B3 reduces P1 autocorr DR monotonically
at all 3 seeds) but the **single-shard cited magnitude (-42 absolute)
was 2.6× the multi-seed mean (-16 absolute)**. The v5.30 claim
celebrated the seed where the absolute reduction was largest.

## Trade-off direction confirmed

The B3 P3 fanout regression is **stable across seeds**:

| seed | A3 P3 fanout | B3 P3 fanout | factor |
|---|--:|--:|--:|
| 42 | 382.80 | 771.43 | 2.01× |
| 123 | 276.85 | 812.25 | 2.94× |
| 7 | 237.11 | 469.89 | 1.98× |

B3 reliably doubles+ the P3 fanout DR. The trade-off the v5.30 B3
COMPARISON.md flagged ("P1 autocorr win at cost of P3 fanout
regression") **is real and consistent**.

## Composite verdict

| seed | A3 composite | B3 composite | Δ |
|---|--:|--:|--:|
| 42 | 121.10 | 190.79 | +57.6 % |
| 123 | 81.98 | 189.33 | +131 % |
| 7 | 78.29 | 123.93 | +58.3 % |

B3 increases composite at all 3 seeds. **The decision to ship B3 as
opt-in only (config block commented out by default) was correct.**

## Implications for v5.30 dataset cards

The 1m and 10m HF dataset cards cite the v5.30 A3 composite as
121.1× (the seed=42 high-side draw). Per T3's methodology finding,
the honest multi-seed mean is **94 ± 24** (A3) and **168 ± 38** (B3
opt-in).

The cards don't need to advertise B3 (it's opt-in, default-off), but
the A3 headline should be updated to the multi-seed mean.

## Methodological note

This is the first DataSynth result that's been validated across
multiple RNG seeds on the Sajja eval. The protocol established here:

1. Run Sajja exact eval at 3 seeds (42, 123, 7)
2. Compute mean + std + CV per sub-metric
3. Compute delta-of-means vs baseline (A3, A2, etc.)
4. Only credit engine-change wins where the delta-of-means exceeds
   the CV-derived noise band

T3's CV finding (P1 autocorr CV 103 %, P2 burst length CV 1.9 %)
makes P2 burst length the cleanest engine-attribution metric.
B3's effect on P2 burst length is +0.02 (+0.2 %) — within noise.
So B3's behavioral-fidelity engine effect, if any, is invisible on
the most reliable metric.

## What B3 DOES affect reliably

- **P1 autocorr drop** (−16 mean, real direction)
- **P3 fanout regression** (+386 mean, real direction)
- **Composite worsening** (+74 mean, real direction)

None of these have CV-tight enough to be credit-worthy as "engine
fidelity improvements" — the P3 + composite directions are
regressions. **B3 IS a useful research lever for understanding the
P1/P3 trade-off, but it's not a v5.30 fidelity win.**

## Artefacts

```
docs/baselines/2026-05-27-v5.31-b3-multiseed-validation/
├── COMPARISON.md                  (this file)
├── aggregate.json                 (per-metric mean/std/delta-A3 across seeds)
├── seed_42/{baseline,synth}.json
├── seed_123/{baseline,synth}.json
└── seed_7/{baseline,synth}.json
```

## Compute

VM 143.47.102.202 (Lambda A10, 30c/222GB).
3 parallel Sajja evals (subsample 500K, attr_cols = [trading_partner,
gl_account]).
Wallclock: 21 min (parallel).

## Recommended next steps

1. **Update v5.30 dataset cards** with multi-seed mean (94 ± 24) instead of
   the single-shard 121.1×.
2. **B3 retrospective in CHANGELOG** — note that the engine effect is
   real but smaller than the single-shard claim suggested.
3. **Adopt the multi-seed protocol** as the default for future Sajja
   eval reports.

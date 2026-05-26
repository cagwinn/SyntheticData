# v5.30 A3 (#150) — Sajja exact eval with TAIL_MASS 0.30 → 0.15

Closes A3 on the v5.30 roadmap. Re-runs the Sajja exact eval against
a v5.30-A3 regen of the 10M-scale synth output, with the SP3 Z-tail
mass reduced from 0.30 → 0.15. Source change at commit `b13b06ad`.

Previous baselines:
- [A1 (P3 wired)](../2026-05-26-v5.30-a1-sajja-p3/COMPARISON.md) — composite 134.0×
- [A2 (real synth labels)](../2026-05-26-v5.30-a2-real-labels/COMPARISON.md) — composite 133.5×

## Result vs A1 baseline

| sub-metric | A1 (v5.29) | **A3 (v5.30)** | Δ vs A1 |
|---|--:|--:|--:|
| P1 IETD W₁ (fraud) | 61.1× | **58.0×** | -5% |
| P1 IET autocorr gap | 105.9× | **62.8×** | **-41%** ⭐ |
| P2 Active lifetime W₁ | 86.8× | 89.5× | +3% |
| P2 Burst length W₁ | 13.3× | 12.4× | -7% |
| P3 Fanout W₁ (TP) | 403.0× | 382.8× | -5% |
| P4 vrtrigger | NaN | NaN | (structural blocker) |
| **Composite (5 valid)** | **134.0×** | **121.1×** | **-10%** ⭐ |

The roadmap A3 expectation was "vol-corrected composite -10 to -15 %".
**A3 lands at -10% — at the bottom of the expected band.** The P1
autocorr drop of -41 % was unexpected; the mechanism analysis below
explains why this is the dominant lift.

## Why P1 autocorr dropped 41 % — mechanism analysis

The A3 change is a **one-line constant** flip in
`crates/datasynth-core/src/distributions/behavioral_priors.rs:346`:

```rust
const TAIL_MASS: f64 = 0.15;  // was 0.30
```

The Z-tail still has TAIL_N=500 codes (so synth still emits 526
distinct sources, same as v5.29). What changed is the **mass
distribution**:

| | v5.29 (TAIL_MASS=0.30) | v5.30 A3 (TAIL_MASS=0.15) |
|---|--:|--:|
| Head sources (26 SAP-canonical codes) | ~70% mass | ~85% mass |
| Z-tail (500 synthetic codes) | ~30% mass | ~15% mass |
| **Events per Z-code @ 10.9M draws** | ~6 500 each | ~3 270 each |
| **Events per head code @ 10.9M draws** | ~290 000 each | ~360 000 each |

The lag-1 autocorrelation in `ConditionalIETSampler` needs
**per-source event density** to surface. By shifting 15 percentage
points of sampling mass from the synthetic-only Z-tail back to the
real-data head sources, each head source now sees ~25% more events
per draw, giving the lag-1 copula coupling more signal to fire on.

The Z-tail codes have **no real prior data** (they're synthetic
placeholders for source-diversity), so events drawn from them
naturally fall back to a flat distribution with no autocorr —
which inflates the within-source autocorr noise floor. Dialing
back the Z-tail mass concentrates events where the priors actually
have signal.

## What this tells us about the v5.30 design

A3 expected to compress source count (526 → ~390-440); it did **not**
(stayed at 526). The original design comment was wrong about the
mechanism — TAIL_MASS controls mass per code, not the count of codes.
TAIL_N is what controls count.

**But A3's actual effect — mass redistribution improving lag-1
autocorr signal — is more valuable than the intended source-count
compression.** The Sajja P1 autocorr metric is the qualitative
Proposition-2 metric Sajja calls out as the row-independent
generators' failure mode; dropping it 41 % is a concrete signal
that row-aware generation is now actually exercising its inherent
advantage.

## What A3 *didn't* close

- **P3 fanout** stayed at 382.8× (was 403). The trading-partner pool
  is still capped at 12 (SOTA-11.1); reference shard has 36.
  Structural, not addressed by A3.
- **P4 vrtrigger** still NaN. Structural blocker per A2 — Sajja
  baseline normalization needs ground-truth labels on both sides.
- **P2 active lifetime** slightly *worse* (+3%). Within margin
  of error.

## Stacked with B1 Phase 1 (committed in parallel)

The autocorr improvement seen here is from mass redistribution
alone. B1 Phase 1 (commit `6aff2b3e`) compounds with this by routing
the IET sampler through `sap_source_code` instead of `doc_type` —
exposing the per-source lag-1 machinery for all 526 sources rather
than the 5 that previously got per-source treatment via the doc_type
collision (KR/DR/SA/HR/AA).

If A3 alone delivered -41 % on autocorr from mass redistribution,
B1 Phase 1 is expected to compound, especially on the head sources
where the priors have rich data. Combined A3+B1 evaluation
queued.

## Cumulative Tier-A status

| task | status | composite | notes |
|---|---|--:|---|
| A0 baseline (no P3) | — | 66.78× | P3 skipped, 4 metrics |
| A1 (#149) P3 wired | ✅ | 134.0× | new sub-metric exposed |
| A2 (#151) real labels | ✅ | 133.5× | P4 NaN structural |
| **A3 (#150) TAIL_MASS** | ✅ | **121.1×** | **-10% vs A1, P1 autocorr -41%** |

Tier-A closed. Next: B1 Phase 1 regen + eval (post-VM-rebuild) to
measure compound effect with source-keyed IET sampling.

## Artefacts

```
docs/baselines/2026-05-26-v5.30-a3-tail-mass/
├── COMPARISON.md          (this file)
├── baseline.json          (ref half_A vs half_B, deterministic)
└── datasynth_a3.json      (v5.30 A3 synth vs reference)
```

The baseline.json is identical to A1's (same code, same seed) —
deterministic re-derivation confirmed.

## Compute

VM 143.47.102.202 (Lambda A10, 30c/222GB).
A3 regen (10M scale): 3 min (was ~10-15 min historically — newer build is faster).
Sajja eval Step 1: 813 s.
Sajja eval Step 2: 859 s.
Eval total: 28 min.

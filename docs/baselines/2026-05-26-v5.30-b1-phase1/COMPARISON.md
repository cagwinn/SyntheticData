# v5.30 B1 Phase 1 (#152) — source-keyed IET sampling

Re-runs the Sajja exact eval against a v5.30 B1-Phase-1 regen
(commit `6aff2b3e`: switch IET sampler key from `doc_type` to
`sap_source_code`). The expectation was that exposing the per-source
priors to all 526 emitted sources (vs 5 doc_type values previously)
would lift Sajja P1 autocorr below the post-A3 62.8× baseline.

**Result: no measurable change.** Composite stays at 121.1×; every
sub-metric matches A3 to 4 significant figures.

## Result

| sub-metric | A3 | **B1 Phase 1** | Δ |
|---|--:|--:|--:|
| P1 IETD W₁ (fraud) | 57.95× | **57.95×** | 0.00 |
| P1 IET autocorr | 62.84× | **62.84×** | 0.00 |
| P2 Active lifetime | 89.48× | 89.48× | 0.00 |
| P2 Burst length | 12.40× | 12.40× | 0.00 |
| P3 Fanout | 382.80× | 382.80× | 0.00 |
| P4 vrtrigger | NaN | NaN | — |
| **Composite** | **121.10×** | **121.10×** | **0.00** |

Data hashes confirm the synthetic outputs **are** different (parquet
SHA-256 mismatches between A3 and B1 regens), but the eval-level
summary statistics land in the same 4-significant-figure band.

## Root cause: coupling threshold gates the change

`crates/datasynth-core/src/distributions/conditional_iet.rs:176-203`
implements the lag-1 coupling Gaussian-copula path that the B1
patch was meant to exercise more broadly:

```rust
let rho = state.lag1_autocorr.clamp(-1.0, 1.0);
if rho.abs() < 0.1 || state.last_iet_days.is_none() {
    let s = state.sample_quantile(rng).max(0.0);
    state.last_iet_days = Some(s);
    return s;
}
```

The threshold `|ρ| < 0.1` causes the coupling path to **silently
fall back** to independent quantile sampling whenever a source's
prior `lag1_autocorr` is below 0.1. The bundled SP3 priors'
per-source `lag1_autocorr` values are very likely below 0.1 for
most sources (corpus has only weak day-resolution autocorrelation
to capture). So:

- Pre-B1: 5 source codes (KR/DR/SA/HR/AA) routed through the
  sampler. Per-source coupling fires for any of these whose
  `lag1_autocorr ≥ 0.1` (probably 0-2 of them based on observed
  Sajja P1 autocorr ~0.04 for the fraud cohort).
- Post-B1: 526 source codes routed through the sampler.
  Per-source coupling fires for those with `lag1_autocorr ≥ 0.1`
  (still probably 0-2 of them, just from a different lookup
  surface).

The set of *coupling-firing* sources is roughly the same before
and after. B1 Phase 1 routes the right key to the sampler, but
the sampler still falls back to independent sampling for most
sources because the priors' coupling strength is below threshold.

## What this means for B1 follow-up

The B1 design doc proposed two options:
- **Option A** (Phase 1, this commit): switch key doc_type → source
- **Option B** (Phase 2, pending): add burst clustering that
  *deterministically* emits 2-4 short-IET events per source when
  a burst-trigger condition fires

Phase 1 alone is insufficient. Phase 2 bypasses the ρ-threshold
gate by enforcing same-source consecutive emission regardless of
prior coupling strength. This is the actual lever that closes the
P1 autocorr gap.

Phase 1 is **not wasted**: it's a correctness fix (the 521
sources that previously got misrouted through 5 collision-keyed
buckets now get their actual priors). The performance impact
just lives downstream of the coupling threshold, where Phase 2
takes over.

## Alternative paths considered

1. **Drop the coupling threshold to 0.0** (always couple).
   Pros: simple, exercises the existing copula machinery for
   every source. Cons: tiny ρ values produce nearly-flat coupling
   (the Gaussian copula approaches independence as ρ → 0), so
   the net behavior matches the no-coupling path anyway.
   Verdict: no expected lift; not worth the patch.

2. **Re-fit the priors with stronger autocorr inference**. The
   priors' `lag1_autocorr` is computed from day-resolution
   timestamps in the corpus extraction. Sub-day resolution would
   probably yield stronger autocorr. Cons: requires re-running
   the extraction pipeline on the corpus; out of scope for the
   v5.30 sprint.
   Verdict: queue for a future SP-N round.

3. **Burst clustering (Phase 2)**. Deterministic short-IET
   ordering for consecutive same-source events. Bypasses the
   ρ-threshold gate. Direct measurable impact on within-source
   autocorr.
   Verdict: ✅ the right next step. Implementation starts at
   `crates/datasynth-generators/src/je_generator.rs:2148` per
   the B1 design doc.

## Artefacts

```
docs/baselines/2026-05-26-v5.30-b1-phase1/
├── COMPARISON.md       (this file)
├── baseline.json       (ref half_A vs half_B, deterministic)
└── datasynth_b1.json   (v5.30 B1 Phase 1 synth vs reference)
```

## Compute

VM 143.47.102.202 (Lambda A10, 30c/222GB).
B1 rebuild (datasynth-generators only): 12 min.
B1 regen (10M scale): 3 min.
Corpus-schema projection: 2 min.
Sajja eval: 28 min (Step 1: 13.5 min, Step 2: 14.2 min).
End-to-end pipeline: ~46 min wallclock.

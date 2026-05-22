# Track 4 — Learned eval surrogate + CMA-ES tuning loop

## Objective

Make the **calibration loop fast**. Today, tuning a generator knob means:
edit config → full generate → full BF eval (the hours-long cycle the project
history keeps deferring). Replace most of those full evals with a learned
**surrogate** `f(knobs) → predicted BF composite`, and search the knob space
with **CMA-ES** against the surrogate — validating only the promising points
with the real eval.

This is the only track that touches **performance, not realism**, and it has
**zero coherence risk** — it never changes generation, only *which config* we
pick. The generator + constraints are untouched.

## Why a surrogate (vs grid / manual tuning)

The knob space (bypass share, drift thresholds, motif-bias weights, per-source
IET scales, …) is ~10-20 dimensional with expensive, noisy evaluations —
exactly the regime where Bayesian / surrogate-assisted optimization wins.
A cheap surrogate turns "10 full evals/day" into "thousands of surrogate
queries + a handful of confirmatory evals."

## Data

Bootstrapped from the baseline history: every `docs/baselines/*/metrics.csv`
is a `(knobs, composite)` sample. Plus an active-learning loop: each
confirmatory full eval adds a labeled point and retrains the surrogate.
Knob vector schema lives in `surrogate/knobs.py` (TODO: enumerate from
`GeneratorConfig` + the SP-series tuning params).

## Architecture

* **Surrogate**: small MLP (or GP for calibrated uncertainty at low data) —
  `knobs (d) → [P1, P2, P3, P4 DRs]`, composite = aggregation. Predict the
  *vector* of DRs, not just the scalar, so the optimizer can target specific
  gaps.
* **Optimizer**: `cma.CMAEvolutionStrategy` over normalized knobs; acquisition
  = surrogate-predicted composite + uncertainty bonus (UCB) to keep exploring.
* **Active loop**: every N surrogate-proposed optima → 1 real
  `bf_bridge.score_canonical` → append → retrain surrogate.

## Success criteria

* Reach the current best composite (v5.26 ≈ 42 mean / 18 median) in **≤ ⅓ the
  full evals** a manual sweep needed.
* Surrogate rank-correlation (Spearman) with the real eval > 0.8 on held-out
  configs before trusting its proposals.

## Handoff

No generator change — output is a **tuned config patch** (same format the
existing `AutoTuner` emits). Drops straight into the regen pipeline. Runs CPU
or A100; the A100 just makes the surrogate retrain + CMA-ES batches instant.

## Privacy

None — operates on knob vectors + aggregate composite scores, never corpus
data or row-level output.

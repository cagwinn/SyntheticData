# C3 (#158) — adversarial calibration loop design

**Status:** designed. Ready to implement.

**Scope:** Closed-loop generator parameter calibration. Drive the
synthetic engine's tunable knobs so a chosen gap metric (versus a
reference corpus, or versus a target) converges, with built-in
multi-seed variance averaging and safety rails.

## What "adversarial" means here

Two players in opposition:

- **Generator** — the synthetic engine, controlled by a parameter
  vector θ ∈ ℝᵏ (rates, distribution scales, pool sizes).
- **Discriminator** — the gap evaluator. Returns scalar loss L(θ) =
  D(synth(θ), reference) where D is a known divergence (BF composite,
  Sajja sub-metrics, KS-distance, etc.).

The loop iteratively chooses θ_{t+1} = θ_t + Δ to reduce E_seed[L(θ)].
The "adversarial" framing matches the GAN literature where a
discriminator's score steers a generator's parameters — except here
the discriminator is a fixed analytic function (no co-trained model),
which makes the dynamic system far easier to reason about + debug.

## Existing surface to build on

Don't rebuild — wire together:

- **[`AutoTuner::analyze`]** (`datasynth-eval/src/enhancement/auto_tuner.rs`)
  → produces a `Vec<ConfigPatch>` from a single eval result.
  Already rule-based, already enumerates the engine's tunable knobs.
- **[`behavioral_fidelity::compute_report`]**
  (`datasynth-eval/src/behavioral_fidelity/mod.rs`) → scalar BF
  composite + sub-metric breakdown.
- **[`ConfigPatch`]** (path + suggested value + confidence + impact
  estimate) — already the right shape for an iteration step.
- **[`ComprehensiveEvaluation`]** — the eval-side data structure
  AutoTuner reads.
- **[`AiTuner`]** — `AutoTuner` wrapped with LLM interpretation; reuse
  as an optional secondary patch source.
- **Multi-shard variance methodology** (v5.31 T3, memory:
  [[project_v5_31_overnight_loop]]) — single-shard Sajja-composite CV
  ≈ 25 %, P1 autocorr CV ≈ 103 %, P2 burst length CV ≈ 1.9 %.
  C3 MUST average across ≥ 3 seeds before crediting any patch as an
  improvement.

## What's missing — the closed loop

Five pieces.

### Piece 1 — Calibration objective + parameter space

New module: `crates/datasynth-eval/src/calibration/objective.rs`

```rust
/// One iterable target — what we want to minimise.
pub struct CalibrationObjective {
    /// Which metric drives the loop.
    pub metric: ObjectiveMetric,
    /// Weight on each sub-metric when `metric == BfComposite`.
    /// Defaults match `BfReport` defaults; override to focus on
    /// e.g. P1 autocorr exclusively.
    pub weights: BTreeMap<String, f64>,
    /// Optional convergence target — stop when E_seed[L] ≤ this.
    /// Default: open (loop runs to max_iterations).
    pub target: Option<f64>,
}

pub enum ObjectiveMetric {
    /// Sajja BF composite (default).
    BfComposite,
    /// A specific sub-metric path (e.g. "p1.iet_dr_ratio").
    Submetric(String),
    /// Custom: caller-supplied closure for advanced cases.
    Custom(Arc<dyn Fn(&BfReport) -> f64 + Send + Sync>),
}
```

Parameter space — enumerable, bounded, named:

```rust
/// One tunable knob.
pub struct CalibrationKnob {
    /// Config path the knob writes (e.g. "fraud.fraud_rate").
    pub path: String,
    /// Current value (Decimal string form for round-trip via YAML).
    pub current: String,
    /// Allowable bounds — patches outside these are clipped.
    pub bounds: KnobBounds,
    /// Step size — Δ proposed per iteration is bounded by this.
    pub max_step: f64,
}

pub enum KnobBounds {
    /// f64 in [min, max].
    F64Range { min: f64, max: f64 },
    /// usize in [min, max].
    UsizeRange { min: usize, max: usize },
    /// Discrete enum string values.
    Discrete(Vec<String>),
}
```

The starting knob set inventories `AutoTuner`'s known patch targets —
no new knob enumeration logic; we just reuse the rule-based gap
analysis as the "patch proposal" stream.

### Piece 2 — Iteration controller

```rust
pub struct CalibrationLoop {
    objective: CalibrationObjective,
    knobs: Vec<CalibrationKnob>,
    config: CalibrationConfig,
    history: CalibrationHistory,
}

pub struct CalibrationConfig {
    /// Maximum iterations before giving up (default 20).
    pub max_iterations: usize,
    /// Number of seeds per iteration (default 3 — per T3 CV finding).
    pub seeds_per_iteration: usize,
    /// Convergence patience — stop when E[L] hasn't improved by
    /// > `min_improvement` across `patience` iterations (default 3).
    pub patience: usize,
    pub min_improvement: f64,
    /// Damping factor — apply Δ × damping each step so the loop
    /// doesn't oscillate (default 0.5).
    pub damping: f64,
    /// Rollback policy when a step makes E[L] worse.
    pub rollback: RollbackPolicy,
}

pub enum RollbackPolicy {
    /// Revert and try a different patch from the proposal list.
    Revert,
    /// Keep the worse θ (gradient-descent style — accept noise).
    Keep,
    /// Halve `damping` and retry the same patch (annealing).
    HalveDamping,
}
```

Step function:

```rust
impl CalibrationLoop {
    /// One step: generate ×seeds, eval, propose, accept/reject, persist.
    pub fn step<G>(&mut self, generator: &G) -> CalibrationResult<StepReport>
    where G: GenerateFn;
}

pub trait GenerateFn {
    /// Drive the engine end-to-end with the given config + seed.
    /// Returns the synthetic outputs needed by the BF eval.
    fn generate(&self, config: &GroupConfig, seed: u64) -> Result<SyntheticOutput, GenError>;
}
```

The loop:
1. Apply current θ to a fresh `GroupConfig` clone.
2. For each seed in 0..seeds_per_iteration: generate + eval.
3. Compute E_seed[L] (and the per-seed variance for noise floor).
4. Call AutoTuner on the average eval to get candidate patches.
5. Apply one patch (highest-confidence, weighted by impact);
   damp Δ by `damping`; clip to bounds.
6. Update knob.current; persist to history.
7. If E[L] worse than best-seen by > min_improvement × s.d., apply
   `rollback`.
8. Check convergence: target met? patience exhausted? max_iter?

### Piece 3 — History + persistence

```rust
pub struct CalibrationHistory {
    /// One entry per step.
    pub steps: Vec<StepReport>,
    /// Best θ seen so far.
    pub best_theta: BTreeMap<String, String>,
    /// E[L] at best_theta.
    pub best_loss: f64,
    /// Per-knob trajectory — Vec<(iter, value)> for plotting.
    pub trajectories: BTreeMap<String, Vec<(usize, String)>>,
}
```

Persisted to `<out>/calibration_history.json` after every step so a
long-running loop can resume after interruption (CLAUDE.md memory
note: existing `GenerationSession::save/resume` provides the same
shape for engine state).

### Piece 4 — CLI surface

```
datasynth-data calibrate \
  --config base.yaml \
  --reference ./reference_shard/ \
  --out ./calibration_out/ \
  [--objective {bf_composite | <submetric_path>}] \
  [--max-iter 20] \
  [--seeds 3] \
  [--target 25.0] \
  [--resume ./calibration_out/]
```

Per-step output:
```
./calibration_out/
  iter_00/  config.yaml  synth_seed_0/  synth_seed_1/  synth_seed_2/
            bf_report_seed_0.json  ... bf_report_seed_2.json
            eval_summary.json (E[L], std, knob values)
  iter_01/  ...
  calibration_history.json
  best/  config.yaml  (the calibrated config)
```

### Piece 5 — Safety rails

Each rail is a CalibrationLoop check that aborts the iteration with
a clear error:

- **Knob clip**: patches outside `bounds` are clipped, not applied
  verbatim. Loss is recomputed from the clipped value.
- **Monotonicity drift detection**: if a knob bounces ±2 σ over 5
  iterations, the optimizer is oscillating — fall back to halving
  damping and emit a warning.
- **Overfitting guard**: if E[L] on the calibration seed set drops
  while a held-out validation seed's E[L] doesn't, abort.
  (Adversarial calibration on a single shard's metrics is the classic
  way to overfit — the T3 CV finding is exactly this signal.)
- **Wall-clock budget**: per-iteration generate + eval should fit a
  10-minute budget by default. Configurable via
  `--max-iteration-seconds`.

## Implementation order

1. **Piece 1** (objective + knob types, ~300 LOC + 5 unit tests) —
   self-contained. Land first.
2. **Piece 2** (iteration controller, ~400 LOC + 3 tests using mock
   generator) — the loop body.
3. **Piece 3** (history persistence, ~150 LOC + 2 tests) — the
   resume hook.
4. **Piece 4** (CLI wiring, ~150 LOC + 1 integration test against
   a small fixture) — user-facing surface.
5. **Piece 5** (safety rails, ~200 LOC + 4 tests) — guards.

Total estimate: ~1 200 LOC + 15 tests. **2-3 days focused work**.

## Validation plan

Single-shard small-N validation (1M-row engagement):
1. Set `fraud.fraud_rate` and `anomaly_injection.rates.consolidation_outlier_rate`
   to demonstrably-wrong values (5× corpus + 0.001 ÷ 10).
2. Run `datasynth-data calibrate` with the v5.29 corpus as reference.
3. Confirm the loop drives both knobs toward the corpus-matching
   values within 10 iterations.

Multi-shard cross-validation:
4. Calibrate on shard A, evaluate the calibrated config on shard B
   (held out). The loss on B must not be > 1.5× the loss on A —
   anything more is overfitting and should fail the validation gate.

## Why this matters

The engine has ~25 named tunable knobs (fraud rate, anomaly rates,
distribution scales, pool sizes, IET parameters, etc.). v5.30 SOTA
hand-tuned ~12 of them via 4 manual iteration rounds; the v5.31 work
added 4 more knobs. Each hand-tune round takes a person ~1-2 hours
including evaluation. C3 collapses that to an overnight automated
run.

The harder claim: when SOTA tuning hits diminishing returns at ~24×
composite gap, the remaining gap is partly because we're tuning
one knob at a time. C3 can in principle find interaction effects
(e.g. "raise fraud rate AND lower consolidation outlier rate
together") that single-knob rounds miss. That's the genuine
adversarial-loop benefit over manual SOTA.

## Out of scope (deferred)

- **Neural surrogate for the loss landscape** — using a small NN to
  predict L(θ) from θ without running the full generator. Useful for
  speed but adds a new training loop.
- **Multi-objective Pareto front** — when objectives conflict (e.g.
  closing the autocorr gap regresses the burst-length gap), the
  current design picks one scalar objective via the `weights` map.
  Pareto sampling is a follow-up if the weighted-sum approach
  empirically fails.
- **Black-box optimizers beyond rule-based patches** — Bayesian
  optimization, simulated annealing on the parameter vector
  directly. The current design defers to AutoTuner's rule-based
  patch proposal; a CMA-ES / scipy.optimize-style alternative is
  a follow-up.
- **Calibration against multiple reference shards simultaneously**
  — current design takes one `--reference`. Multi-reference (e.g.
  "match the average of these 3 corpora") would require a different
  loss aggregator and is deferred.

## Dependencies

- **C1 #156** (streaming aggregate) — needed if the loop calibrates
  at 2k-entity scale. For the initial 1M-row single-shard validation
  plan above, C1 isn't required. **Soft prerequisite.**
- **C2 #157** (multi-period) — independent; C2 is about period
  chaining, C3 is about parameter tuning. **No dependency.**

## Open follow-up tasks

- C3 implementation (this design's deliverables).
- Neural surrogate (above).
- Multi-objective Pareto (above).
- Multi-reference calibration (above).

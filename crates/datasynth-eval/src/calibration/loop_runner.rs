//! C3 Piece 2 — iteration controller for the adversarial calibration
//! loop.
//!
//! Drives the generate → eval → propose → accept/reject cycle.
//! Wires:
//!   - [`super::CalibrationObjective`] (what we minimise)
//!   - [`super::CalibrationKnob`] (parameter space)
//!   - [`Proposer`] (who suggests the next step)
//!   - [`Evaluator`] (who runs generation + BF eval; abstract so
//!     unit tests can inject a deterministic mock without the
//!     orchestrator).
//!
//! See `docs/design/2026-05-27-c3-adversarial-calibration-design.md`
//! for the broader plan. This module ships the iteration body +
//! convergence + rollback logic; the AutoTuner-driven proposer and
//! the real generator-backed evaluator are Pieces 2.5 / 4.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::behavioral_fidelity::report::BehavioralFidelityReport;

use super::knob::{CalibrationKnob, KnobValue};
use super::objective::CalibrationObjective;

// ── Public types ──────────────────────────────────────────────────────────────

/// What to do when a step makes the multi-seed mean loss WORSE than
/// the best-seen value (by more than the noise floor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RollbackPolicy {
    /// Revert the knob to its pre-step value and try a different
    /// patch on the next iteration.
    #[default]
    Revert,
    /// Keep the worse θ — the optimizer accepts noise; useful for
    /// pure gradient-descent-style runs that should walk uphill
    /// occasionally to escape plateaus.
    Keep,
    /// Halve the loop's effective damping and re-propose the same
    /// patch direction on the next iteration. Simulated-annealing
    /// flavour.
    HalveDamping,
}

/// Loop-level config.
#[derive(Debug, Clone)]
pub struct CalibrationConfig {
    /// Maximum iterations before the loop gives up. Default 20.
    pub max_iterations: usize,
    /// Seeds evaluated per iteration to amortise the T3 single-shard
    /// composite CV ≈ 25 %. Default 3 — matches the v5.31 T3
    /// methodology finding (memory note `project_v5_31_overnight_loop`).
    pub seeds_per_iteration: usize,
    /// Convergence patience — stop when the best mean loss hasn't
    /// improved by > `min_improvement` × σ across `patience`
    /// consecutive iterations. Default 3.
    pub patience: usize,
    /// Minimum relative improvement (in units of σ_loss) needed to
    /// credit a step as "better". Default 1.0 — i.e. > 1 standard
    /// deviation of the aggregate.
    pub min_improvement: f64,
    /// Damping applied to proposed Δ before clipping. Default 0.5
    /// (take half the proposed step to avoid oscillation).
    pub damping: f64,
    /// How to handle a step that worsens the loss.
    pub rollback: RollbackPolicy,
}

impl Default for CalibrationConfig {
    fn default() -> Self {
        Self {
            max_iterations: 20,
            seeds_per_iteration: 3,
            patience: 3,
            min_improvement: 1.0,
            damping: 0.5,
            rollback: RollbackPolicy::default(),
        }
    }
}

/// One step's outcome. Persisted to the history so a long-running
/// loop can resume after interruption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepReport {
    /// 0-indexed step number.
    pub iter: usize,
    /// Multi-seed mean loss BEFORE this step's patch was applied.
    pub loss_before_mean: f64,
    /// Multi-seed standard deviation BEFORE (the noise floor used
    /// for accept/reject).
    pub loss_before_std: f64,
    /// Patch the proposer suggested (`None` → proposer gave up,
    /// loop stops).
    pub proposed_patch: Option<ProposedPatch>,
    /// Multi-seed mean loss AFTER applying the patch (and any
    /// clipping). `None` when `proposed_patch == None`.
    pub loss_after_mean: Option<f64>,
    pub loss_after_std: Option<f64>,
    /// Knob's value AFTER apply (= current state at end of step).
    pub knob_values: BTreeMap<String, KnobValue>,
    /// Whether the step was accepted, rolled back, or otherwise
    /// resolved. See [`StepOutcome`].
    pub outcome: StepOutcome,
}

/// What the loop did with the proposed step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepOutcome {
    /// Patch reduced loss enough to credit as improvement.
    Improved,
    /// Patch was applied but didn't beat the noise floor. Kept anyway
    /// (RollbackPolicy::Keep) OR damping was halved (HalveDamping).
    AcceptedNoNoiseFloorBeat,
    /// Patch worsened loss; reverted per RollbackPolicy::Revert.
    Reverted,
    /// Proposer returned None — nothing left to try.
    ProposerExhausted,
    /// Convergence target met before this step ran.
    TargetMet,
    /// Patience exhausted (best loss flat for `patience` iterations).
    PatienceExhausted,
}

/// What a proposer suggests: a knob to change and what value to try.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedPatch {
    /// Index into the knobs slice.
    pub knob_index: usize,
    /// Proposed new value (before clipping).
    pub proposed_value: KnobValue,
    /// Optional rationale for the logs.
    pub rationale: String,
}

/// Strategy for choosing the next knob + value to try.
///
/// Implementations receive the current knob state, the multi-seed
/// loss observation, and the history. They return `None` when they
/// have nothing more to propose (the loop then stops). The default
/// [`BoundsScanProposer`] cycles through knobs proposing
/// `current ± max_step` in the direction that improved the loss
/// most recently; an AutoTuner-driven proposer is a Piece 2.5 follow-up.
pub trait Proposer {
    fn propose(
        &mut self,
        knobs: &[CalibrationKnob],
        current_loss: (f64, f64),
        history: &[StepReport],
    ) -> Option<ProposedPatch>;
}

/// What runs a generation + eval for a given knob state + seed.
///
/// Real implementations call into the orchestrator + emit a
/// `BehavioralFidelityReport`. Unit tests inject a deterministic
/// mock that maps `(knobs, seed)` to a hand-crafted report so the
/// loop's accept/reject logic can be exercised without running the
/// engine.
pub trait Evaluator {
    fn evaluate(
        &self,
        knobs: &[CalibrationKnob],
        seed: u64,
    ) -> Result<BehavioralFidelityReport, EvaluatorError>;
}

/// Generic error wrapper so the iteration loop can propagate
/// underlying engine + IO failures without the loop being tied to
/// any one error type.
#[derive(Debug)]
pub struct EvaluatorError(pub String);

impl std::fmt::Display for EvaluatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "evaluator error: {}", self.0)
    }
}

impl std::error::Error for EvaluatorError {}

// ── The loop ─────────────────────────────────────────────────────────────────

/// The iteration controller. Owns the knobs, objective, config, and
/// the accumulated history.
pub struct CalibrationLoop {
    pub objective: CalibrationObjective,
    pub knobs: Vec<CalibrationKnob>,
    pub config: CalibrationConfig,
    pub history: Vec<StepReport>,
    /// Best (mean, std) seen so far.
    pub best_loss: Option<(f64, f64)>,
    /// Knob values when `best_loss` was observed.
    pub best_knob_values: BTreeMap<String, KnobValue>,
    /// Effective damping (may shrink under HalveDamping policy).
    effective_damping: f64,
    /// Steps since last improvement — counts toward patience.
    steps_since_improvement: usize,
}

impl CalibrationLoop {
    pub fn new(
        objective: CalibrationObjective,
        knobs: Vec<CalibrationKnob>,
        config: CalibrationConfig,
    ) -> Self {
        let damping = config.damping;
        Self {
            objective,
            knobs,
            config,
            history: Vec::new(),
            best_loss: None,
            best_knob_values: BTreeMap::new(),
            effective_damping: damping,
            steps_since_improvement: 0,
        }
    }

    /// Run one step. Returns the resulting [`StepReport`] which is
    /// also appended to `self.history`.
    pub fn step<E: Evaluator, P: Proposer>(
        &mut self,
        evaluator: &E,
        proposer: &mut P,
    ) -> Result<&StepReport, EvaluatorError> {
        let iter = self.history.len();

        // 1. Measure current loss at the existing knob state (multi-seed).
        let (mean_before, std_before) = self.measure_loss(evaluator)?;

        // 2. Check convergence target.
        if let Some(target) = self.objective.target {
            if mean_before <= target {
                return Ok(self.record(StepReport {
                    iter,
                    loss_before_mean: mean_before,
                    loss_before_std: std_before,
                    proposed_patch: None,
                    loss_after_mean: None,
                    loss_after_std: None,
                    knob_values: self.snapshot_knobs(),
                    outcome: StepOutcome::TargetMet,
                }));
            }
        }

        // 3. Update best-seen tracker BEFORE proposing a new step
        //    (matters on iteration 0 when history is empty).
        if self.best_loss.map(|(m, _)| mean_before < m).unwrap_or(true) {
            self.best_loss = Some((mean_before, std_before));
            self.best_knob_values = self.snapshot_knobs();
            self.steps_since_improvement = 0;
        }

        // 4. Ask the proposer for a patch.
        let raw_patch = proposer.propose(&self.knobs, (mean_before, std_before), &self.history);
        let Some(patch) = raw_patch else {
            return Ok(self.record(StepReport {
                iter,
                loss_before_mean: mean_before,
                loss_before_std: std_before,
                proposed_patch: None,
                loss_after_mean: None,
                loss_after_std: None,
                knob_values: self.snapshot_knobs(),
                outcome: StepOutcome::ProposerExhausted,
            }));
        };

        // 5. Apply damping to Δ before letting the knob clip.
        let damped_value = damp_value(
            self.knobs[patch.knob_index].current,
            patch.proposed_value,
            self.effective_damping,
        );

        // 6. Save the pre-step value for rollback.
        let pre_value = self.knobs[patch.knob_index].current;
        let _clip_result = self.knobs[patch.knob_index].apply(damped_value);

        // 7. Re-measure loss at the new state.
        let (mean_after, std_after) = self.measure_loss(evaluator)?;

        // 8. Decide outcome.
        let outcome = self.decide_outcome(
            mean_before,
            std_before,
            mean_after,
            patch.knob_index,
            pre_value,
        );

        // 9. Patience accounting.
        match outcome {
            StepOutcome::Improved => {
                self.steps_since_improvement = 0;
            }
            _ => {
                self.steps_since_improvement += 1;
            }
        }

        // 10. Persist + return.
        Ok(self.record(StepReport {
            iter,
            loss_before_mean: mean_before,
            loss_before_std: std_before,
            proposed_patch: Some(ProposedPatch {
                knob_index: patch.knob_index,
                proposed_value: damped_value,
                rationale: patch.rationale,
            }),
            loss_after_mean: Some(mean_after),
            loss_after_std: Some(std_after),
            knob_values: self.snapshot_knobs(),
            outcome,
        }))
    }

    /// Run until convergence, patience exhaustion, max-iter, or
    /// proposer exhaustion. Returns the final history.
    pub fn run<E: Evaluator, P: Proposer>(
        &mut self,
        evaluator: &E,
        proposer: &mut P,
    ) -> Result<&[StepReport], EvaluatorError> {
        for _ in 0..self.config.max_iterations {
            let outcome = self.step(evaluator, proposer)?.outcome;
            if matches!(
                outcome,
                StepOutcome::TargetMet
                    | StepOutcome::ProposerExhausted
                    | StepOutcome::PatienceExhausted
            ) {
                break;
            }
            if self.steps_since_improvement >= self.config.patience {
                // Synthesise a terminal entry so the caller sees why we stopped.
                let last = self.history.last().expect("history non-empty after step");
                let mut term = last.clone();
                term.iter = self.history.len();
                term.outcome = StepOutcome::PatienceExhausted;
                term.proposed_patch = None;
                term.loss_after_mean = None;
                term.loss_after_std = None;
                self.history.push(term);
                break;
            }
        }
        Ok(&self.history)
    }

    // ── Internals ────────────────────────────────────────────────────────────

    /// Measure (mean, std) loss across `seeds_per_iteration` seeds.
    fn measure_loss<E: Evaluator>(&self, evaluator: &E) -> Result<(f64, f64), EvaluatorError> {
        let mut reports = Vec::with_capacity(self.config.seeds_per_iteration);
        for seed in 0..self.config.seeds_per_iteration as u64 {
            reports.push(evaluator.evaluate(&self.knobs, seed)?);
        }
        self.objective.aggregate(&reports).ok_or_else(|| {
            EvaluatorError("objective returned None from non-empty report set".into())
        })
    }

    fn snapshot_knobs(&self) -> BTreeMap<String, KnobValue> {
        self.knobs
            .iter()
            .map(|k| (k.path.clone(), k.current))
            .collect()
    }

    fn decide_outcome(
        &mut self,
        mean_before: f64,
        std_before: f64,
        mean_after: f64,
        knob_idx: usize,
        pre_value: KnobValue,
    ) -> StepOutcome {
        let beat_noise_floor = std_before > 0.0
            && (mean_before - mean_after) > self.config.min_improvement * std_before;
        if mean_after < mean_before && beat_noise_floor {
            // Real improvement.
            self.best_loss = Some((mean_after, 0.0));
            self.best_knob_values = self.snapshot_knobs();
            return StepOutcome::Improved;
        }

        if mean_after >= mean_before {
            // Worse than baseline.
            match self.config.rollback {
                RollbackPolicy::Revert => {
                    self.knobs[knob_idx].current = pre_value;
                    StepOutcome::Reverted
                }
                RollbackPolicy::Keep => StepOutcome::AcceptedNoNoiseFloorBeat,
                RollbackPolicy::HalveDamping => {
                    self.effective_damping *= 0.5;
                    self.knobs[knob_idx].current = pre_value;
                    StepOutcome::Reverted
                }
            }
        } else {
            // Better numerically but didn't beat noise floor.
            StepOutcome::AcceptedNoNoiseFloorBeat
        }
    }

    fn record(&mut self, report: StepReport) -> &StepReport {
        self.history.push(report);
        self.history.last().expect("just pushed")
    }
}

/// Apply `damping ∈ [0, 1]` to the (proposed - current) delta and
/// return the damped value. Numeric only — type taken from
/// `proposed`'s variant.
fn damp_value(current: KnobValue, proposed: KnobValue, damping: f64) -> KnobValue {
    let cur = current.as_f64();
    let prop = proposed.as_f64();
    let damped = cur + (prop - cur) * damping;
    match proposed {
        KnobValue::F64(_) => KnobValue::F64(damped),
        KnobValue::Usize(_) => KnobValue::Usize(damped.round().max(0.0) as usize),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavioral_fidelity::report::{
        BaselineValues, BehavioralFidelityReport, CorpusSummary, EntityMetrics, GateResult,
        PerMetric,
    };
    use chrono::Utc;

    fn empty_per_metric() -> PerMetric {
        PerMetric {
            raw: 0.0,
            baseline: 0.0,
            dr: 0.0,
            is_degenerate_baseline: false,
            is_volume_bounded: false,
        }
    }

    fn empty_em() -> EntityMetrics {
        EntityMetrics {
            entity_column: "t".into(),
            p1_ietd: empty_per_metric(),
            p1_autocorr: empty_per_metric(),
            p2_active_lifetime: empty_per_metric(),
            p2_burst_len_by_threshold: BTreeMap::new(),
            p2_je_line_burst: empty_per_metric(),
            p3_fanout_by_attr: BTreeMap::new(),
            p3_clustering: empty_per_metric(),
            p3_triangle_log_ratio: empty_per_metric(),
            p4_rule_results: vec![],
            p4_mean_gap: empty_per_metric(),
        }
    }

    fn make_report(composite: f64) -> BehavioralFidelityReport {
        BehavioralFidelityReport {
            profile: "t".into(),
            generator_id: "t".into(),
            generator_version: "v5.x".into(),
            seed: 0,
            generated_at: Utc::now(),
            reference_corpus: CorpusSummary {
                path: "/dev/null".into(),
                n_rows: 0,
                n_entities_primary: 0,
                n_entities_secondary: 0,
                period_start: None,
                period_end: None,
            },
            synthetic: CorpusSummary {
                path: "/dev/null".into(),
                n_rows: 0,
                n_entities_primary: 0,
                n_entities_secondary: 0,
                period_start: None,
                period_end: None,
            },
            noise_floor: BaselineValues {
                p1_ietd_w1_days: 0.0,
                p1_autocorr_gap: 0.0,
                p2_active_lifetime_w1: 0.0,
                p2_burst_len_by_threshold: BTreeMap::new(),
                p2_je_line_burst_w1: 0.0,
                p3_fanout_by_attr: BTreeMap::new(),
                p3_clustering_gap: 0.0,
                p3_triangle_log_ratio: 0.0,
                p4_mean_gap: 0.0,
            },
            per_entity: {
                let mut m = BTreeMap::new();
                m.insert("t".into(), empty_em());
                m
            },
            composite_bf_score: composite,
            composite_bf_median: composite,
            n_metrics_aggregated: 1,
            n_metrics_excluded_degenerate: 0,
            composite_bf_volume_corrected: composite,
            n_metrics_excluded_volume: 0,
            intraday_structural: None,
            gates: GateResult {
                fail_if_dr_above: 100.0,
                fail_if_composite_above: 100.0,
                passed: true,
                failures: vec![],
            },
        }
    }

    /// Mock evaluator: maps the first knob's f64 value linearly to a
    /// composite. Optimum at the supplied `optimum`. `noise` is
    /// added deterministically as a function of seed so we can
    /// exercise the multi-seed aggregator.
    struct LinearMockEvaluator {
        optimum: f64,
        noise: f64,
    }
    impl Evaluator for LinearMockEvaluator {
        fn evaluate(
            &self,
            knobs: &[CalibrationKnob],
            seed: u64,
        ) -> Result<BehavioralFidelityReport, EvaluatorError> {
            let v = knobs[0].current.as_f64();
            // Loss = |v - optimum| + per-seed deterministic noise.
            let noise = self.noise * (seed as f64 - 1.0); // -1, 0, +1 for seeds 0,1,2
            let composite = (v - self.optimum).abs() + noise;
            Ok(make_report(composite))
        }
    }

    /// Trivial proposer: always pushes knob 0 toward `target`,
    /// bounded by max_step.
    struct StepTowardProposer {
        target: f64,
    }
    impl Proposer for StepTowardProposer {
        fn propose(
            &mut self,
            knobs: &[CalibrationKnob],
            _current_loss: (f64, f64),
            _history: &[StepReport],
        ) -> Option<ProposedPatch> {
            let cur = knobs[0].current.as_f64();
            if (cur - self.target).abs() < 1e-9 {
                return None;
            }
            let direction = (self.target - cur).signum();
            let step = direction * knobs[0].max_step;
            Some(ProposedPatch {
                knob_index: 0,
                proposed_value: KnobValue::F64(cur + step),
                rationale: format!("step toward {target}", target = self.target),
            })
        }
    }

    #[test]
    fn step_reduces_loss_when_moving_toward_optimum() {
        let knobs = vec![CalibrationKnob::new_f64("test.rate", 0.10, 0.0, 1.0, 0.05)];
        let mut loop_ = CalibrationLoop::new(
            CalibrationObjective::bf_composite(),
            knobs,
            CalibrationConfig {
                seeds_per_iteration: 1,
                max_iterations: 1,
                min_improvement: 0.0, // disable noise-floor gate for deterministic loss
                damping: 1.0,         // no damping so the step lands at proposed
                ..CalibrationConfig::default()
            },
        );
        let eval = LinearMockEvaluator {
            optimum: 0.02,
            noise: 0.0,
        };
        let mut prop = StepTowardProposer { target: 0.02 };

        let report = loop_.step(&eval, &mut prop).expect("step ok").clone();

        assert!(report.loss_before_mean > 0.0);
        let after = report.loss_after_mean.unwrap();
        assert!(
            after < report.loss_before_mean,
            "step should reduce loss (before={}, after={})",
            report.loss_before_mean,
            after
        );
    }

    #[test]
    fn run_converges_to_optimum_within_max_iter() {
        // Start at 0.10, optimum at 0.02, max_step 0.02 — needs 4 steps.
        let knobs = vec![CalibrationKnob::new_f64("test.rate", 0.10, 0.0, 1.0, 0.02)];
        let mut loop_ = CalibrationLoop::new(
            CalibrationObjective::bf_composite().with_target(0.001),
            knobs,
            CalibrationConfig {
                seeds_per_iteration: 1,
                max_iterations: 10,
                min_improvement: 0.0,
                damping: 1.0,
                patience: 20, // disable patience so the loop runs to convergence
                ..CalibrationConfig::default()
            },
        );
        let eval = LinearMockEvaluator {
            optimum: 0.02,
            noise: 0.0,
        };
        let mut prop = StepTowardProposer { target: 0.02 };

        let history = loop_.run(&eval, &mut prop).unwrap().to_vec();

        assert!(!history.is_empty());
        let final_value = loop_.knobs[0].current.as_f64();
        assert!(
            (final_value - 0.02).abs() < 1e-6,
            "final knob value should converge to optimum: got {final_value}"
        );
        assert!(
            history
                .iter()
                .any(|s| matches!(s.outcome, StepOutcome::TargetMet)),
            "convergence target should have been met before max_iter"
        );
    }

    #[test]
    fn proposer_exhaustion_stops_the_loop() {
        // Proposer returns None immediately (knob is already at target).
        let knobs = vec![CalibrationKnob::new_f64("test.rate", 0.02, 0.0, 1.0, 0.02)];
        let mut loop_ = CalibrationLoop::new(
            CalibrationObjective::bf_composite(),
            knobs,
            CalibrationConfig {
                seeds_per_iteration: 1,
                max_iterations: 10,
                ..CalibrationConfig::default()
            },
        );
        let eval = LinearMockEvaluator {
            optimum: 0.02,
            noise: 0.0,
        };
        let mut prop = StepTowardProposer { target: 0.02 };

        let history = loop_.run(&eval, &mut prop).unwrap().to_vec();
        assert_eq!(history.len(), 1);
        assert!(matches!(history[0].outcome, StepOutcome::ProposerExhausted));
    }

    #[test]
    fn rollback_revert_restores_pre_step_value() {
        // Optimum at 0.02 but proposer steps AWAY (toward 0.5). The
        // loop should rollback every step under Revert policy.
        let knobs = vec![CalibrationKnob::new_f64("test.rate", 0.02, 0.0, 1.0, 0.05)];
        let mut loop_ = CalibrationLoop::new(
            CalibrationObjective::bf_composite(),
            knobs,
            CalibrationConfig {
                seeds_per_iteration: 1,
                max_iterations: 3,
                min_improvement: 0.0,
                damping: 1.0,
                rollback: RollbackPolicy::Revert,
                patience: 20,
            },
        );
        let eval = LinearMockEvaluator {
            optimum: 0.02,
            noise: 0.0,
        };
        let mut prop = StepTowardProposer { target: 0.5 };

        loop_.run(&eval, &mut prop).unwrap();

        let final_value = loop_.knobs[0].current.as_f64();
        assert!(
            (final_value - 0.02).abs() < 1e-9,
            "Revert policy should restore the starting value; got {final_value}"
        );
        assert!(
            loop_
                .history
                .iter()
                .any(|s| matches!(s.outcome, StepOutcome::Reverted)),
            "at least one step should have been Reverted"
        );
    }

    #[test]
    fn multi_seed_aggregate_produces_std() {
        // Three seeds with noise = 0.01 → composites {-0.01, 0.0, +0.01}
        // relative to base (0.10 - 0.02) = 0.08. So actual composites:
        // {0.07, 0.08, 0.09}. Mean 0.08, std sqrt(((-0.01)² + 0 +
        // (0.01)²)/3) = sqrt(0.0002/3) ≈ 0.00816.
        let knobs = vec![CalibrationKnob::new_f64("test.rate", 0.10, 0.0, 1.0, 0.05)];
        let loop_ = CalibrationLoop::new(
            CalibrationObjective::bf_composite(),
            knobs,
            CalibrationConfig {
                seeds_per_iteration: 3,
                ..CalibrationConfig::default()
            },
        );
        let eval = LinearMockEvaluator {
            optimum: 0.02,
            noise: 0.01,
        };
        let (mean, std) = loop_.measure_loss(&eval).unwrap();
        assert!((mean - 0.08).abs() < 1e-9, "expected mean 0.08, got {mean}");
        assert!(
            (std - (2.0_f64 / 30000.0).sqrt()).abs() < 1e-9,
            "expected std ≈ 0.00816, got {std}"
        );
    }
}

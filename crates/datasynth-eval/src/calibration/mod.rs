//! C3 (#158) — adversarial calibration loop.
//!
//! Closed-loop calibration that drives the synthetic engine's tunable
//! knobs so a chosen gap metric (versus a reference corpus, or
//! versus a target value) converges. Builds on
//! [`crate::enhancement::AutoTuner`] for patch proposals and
//! [`crate::behavioral_fidelity::compute_report`] for the loss
//! signal. The "adversarial" framing matches a GAN-style dynamic
//! (generator config θ vs discriminator gap-metric L) but with a
//! fixed analytic discriminator instead of a co-trained model,
//! making the loop trivial to reason about + debug.
//!
//! See `docs/design/2026-05-27-c3-adversarial-calibration-design.md`
//! for the broader plan and the validation strategy.
//!
//! This module ships Piece 1 of the C3 plan:
//! - [`CalibrationObjective`] — what we're minimising.
//! - [`CalibrationKnob`] + [`KnobBounds`] — the parameter space.
//!
//! Pieces 2 (iteration controller), 3 (history persistence), 4
//! (`datasynth-data calibrate` CLI), 5 (safety rails) land in
//! follow-up commits.

mod history;
mod knob;
mod loop_runner;
mod objective;
mod safety;

pub use history::{CalibrationHistory, HistoryError, HISTORY_SCHEMA_VERSION};
pub use knob::{CalibrationKnob, KnobBounds, KnobClipResult, KnobValue};
pub use loop_runner::{
    CalibrationConfig, CalibrationLoop, Evaluator, EvaluatorError, ProposedPatch, Proposer,
    RollbackPolicy, StepOutcome, StepReport,
};
pub use objective::{CalibrationObjective, ObjectiveMetric};
pub use safety::{ClipCounts, KnobClipDiagnostics, OscillationDetector, WallClockBudget};

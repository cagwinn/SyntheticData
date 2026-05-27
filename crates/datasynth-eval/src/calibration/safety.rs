//! C3 Piece 5 — safety rails for the calibration loop.
//!
//! Stateless inspectors that read the loop's step history and
//! return diagnostic signals. The loop calls these between steps;
//! a positive signal can drive a damping reduction, an emit-warn,
//! or an outright abort.
//!
//! Rails shipped in this piece:
//!
//! - [`OscillationDetector`] — flags when the same knob has its Δ
//!   sign-alternate across `window` recent steps, signalling the
//!   optimizer is bouncing between two values.
//! - [`KnobClipDiagnostics`] — tracks per-knob clip counts so the
//!   loop's reporter can surface knobs that hit their bounds
//!   often (= likely the bounds need widening, OR the optimizer
//!   wants to escape its current basin).
//! - [`WallClockBudget`] — wraps a clock so the loop can stop
//!   after a configured elapsed time. Useful for overnight runs
//!   that need a hard deadline.
//!
//! Overfit detection (calibration vs held-out validation seeds) is
//! a follow-up — it requires a second evaluator stream and is
//! tracked as a separate task in the design doc.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::knob::KnobValue;
use super::loop_runner::StepReport;

/// Detect oscillation on a specific knob across recent steps.
///
/// "Oscillation" = the knob's `(after - before)` delta has been
/// sign-alternating across the configured `window` of the most
/// recent steps that actually touched this knob. When that
/// happens, the optimizer is bouncing between two values and the
/// loop should reduce damping (or warn) rather than keep
/// proposing the same step direction.
pub struct OscillationDetector {
    /// How many recent same-knob steps to scan. Default 5.
    pub window: usize,
    /// Minimum alternations needed within the window to flag
    /// oscillation. Default 3 (≥ 3 sign-changes in 5 steps).
    pub min_alternations: usize,
}

impl Default for OscillationDetector {
    fn default() -> Self {
        Self {
            window: 5,
            min_alternations: 3,
        }
    }
}

impl OscillationDetector {
    /// Return true if `knob_path` is oscillating per the
    /// configured window + alternation threshold. Skips steps
    /// whose patch didn't touch `knob_path` and steps that didn't
    /// apply a patch at all. Returns false when fewer than
    /// `window` qualifying steps exist.
    pub fn check(&self, history: &[StepReport], knob_path: &str) -> bool {
        // Collect signs of Δ for this knob's recent steps.
        let mut signs: Vec<i32> = Vec::new();
        for step in history.iter().rev() {
            if step.proposed_patch.is_none() {
                continue;
            }
            // Did this step modify our knob? `knob_values` records
            // the AFTER state; we don't have a direct "before"
            // pointer, but the proposed_value tells us the
            // direction the proposer wanted to go, so we use that
            // as the oscillation signal (sign of proposed - prior
            // step's value for the same knob).
            let path = step.knob_values.keys().find(|p| *p == knob_path).cloned();
            if path.is_none() {
                continue;
            }

            // Look at the previous step's recorded value for this
            // knob to compute Δ direction.
            let cur = step.knob_values.get(knob_path).and_then(value_as_f64);
            let prev = find_prior_value(history, step.iter, knob_path);
            if let (Some(c), Some(p)) = (cur, prev) {
                let delta = c - p;
                if delta.abs() > f64::EPSILON {
                    signs.push(if delta > 0.0 { 1 } else { -1 });
                }
            }
            // Bail once we've reached the configured window.
            if signs.len() >= self.window {
                break;
            }
        }

        if signs.len() < self.window {
            return false;
        }
        // Count sign alternations.
        let alternations = signs.windows(2).filter(|w| w[0] != w[1]).count();
        alternations >= self.min_alternations
    }
}

/// Look up the most recent value of `knob_path` recorded BEFORE
/// the step at `iter`. Returns `None` if no prior step has a
/// value for this knob (= we're at the start of the history).
fn find_prior_value(history: &[StepReport], iter: usize, knob_path: &str) -> Option<f64> {
    for step in history.iter().rev() {
        if step.iter >= iter {
            continue;
        }
        if let Some(v) = step.knob_values.get(knob_path) {
            return value_as_f64(v);
        }
    }
    None
}

fn value_as_f64(v: &KnobValue) -> Option<f64> {
    Some(v.as_f64())
}

/// Per-knob diagnostics — currently just clip counts.
///
/// Updated by the loop after each `knob.apply()` call; surfaced in
/// the final run report so a user can see which knobs hit their
/// bounds often (= bounds probably too tight, OR the optimizer
/// found a knob it can't move further).
#[derive(Debug, Clone, Default)]
pub struct KnobClipDiagnostics {
    /// `knob_path → (n_clipped_low, n_clipped_high, n_in_range)`.
    pub counts: BTreeMap<String, ClipCounts>,
}

#[derive(Debug, Clone, Default)]
pub struct ClipCounts {
    pub low: usize,
    pub high: usize,
    pub in_range: usize,
    pub type_mismatch: usize,
}

impl KnobClipDiagnostics {
    /// Record one `apply` outcome.
    pub fn record(&mut self, knob_path: &str, result: super::knob::KnobClipResult) {
        let entry = self.counts.entry(knob_path.to_string()).or_default();
        use super::knob::KnobClipResult::*;
        match result {
            InRange => entry.in_range += 1,
            ClippedLow => entry.low += 1,
            ClippedHigh => entry.high += 1,
            TypeMismatch => entry.type_mismatch += 1,
        }
    }

    /// Knobs whose clip count exceeds `threshold` (low + high
    /// combined). Useful in the run-report summary as a flag for
    /// the human reviewer.
    pub fn frequently_clipped(&self, threshold: usize) -> Vec<&String> {
        self.counts
            .iter()
            .filter(|(_, c)| c.low + c.high >= threshold)
            .map(|(k, _)| k)
            .collect()
    }
}

/// Wall-clock budget — wraps an `Instant` start time + a budget
/// duration. Loop calls `.expired()` between steps and breaks
/// when true.
#[derive(Debug, Clone)]
pub struct WallClockBudget {
    start: Instant,
    budget: Duration,
}

impl WallClockBudget {
    /// Start the clock with `budget` remaining.
    pub fn new(budget: Duration) -> Self {
        Self {
            start: Instant::now(),
            budget,
        }
    }

    /// Has the budget elapsed?
    pub fn expired(&self) -> bool {
        self.start.elapsed() >= self.budget
    }

    /// Time remaining (saturating at zero).
    pub fn remaining(&self) -> Duration {
        self.budget.saturating_sub(self.start.elapsed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::knob::{KnobClipResult, KnobValue};
    use crate::calibration::loop_runner::{ProposedPatch, StepOutcome, StepReport};

    fn make_step_with_knob(iter: usize, knob_path: &str, value: f64) -> StepReport {
        let mut kv = BTreeMap::new();
        kv.insert(knob_path.to_string(), KnobValue::F64(value));
        StepReport {
            iter,
            loss_before_mean: 0.0,
            loss_before_std: 0.0,
            proposed_patch: Some(ProposedPatch {
                knob_index: 0,
                proposed_value: KnobValue::F64(value),
                rationale: "test".into(),
            }),
            loss_after_mean: Some(0.0),
            loss_after_std: Some(0.0),
            knob_values: kv,
            outcome: StepOutcome::Improved,
        }
    }

    #[test]
    fn oscillation_detector_flags_alternating_deltas() {
        // Knob bouncing 0.05 → 0.07 → 0.05 → 0.07 → 0.05 → 0.07 (deltas: +,-,+,-,+).
        let history = vec![
            make_step_with_knob(0, "k", 0.05),
            make_step_with_knob(1, "k", 0.07),
            make_step_with_knob(2, "k", 0.05),
            make_step_with_knob(3, "k", 0.07),
            make_step_with_knob(4, "k", 0.05),
            make_step_with_knob(5, "k", 0.07),
        ];
        let detector = OscillationDetector::default();
        assert!(
            detector.check(&history, "k"),
            "alternating deltas across 5 same-knob steps should flag"
        );
    }

    #[test]
    fn oscillation_detector_quiet_on_monotonic_progress() {
        // Knob walking monotonically: 0.05 → 0.06 → 0.07 → 0.08 → 0.09 → 0.10.
        let history: Vec<_> = (0..6)
            .map(|i| make_step_with_knob(i, "k", 0.05 + 0.01 * (i as f64)))
            .collect();
        let detector = OscillationDetector::default();
        assert!(
            !detector.check(&history, "k"),
            "monotonic walk should not flag oscillation"
        );
    }

    #[test]
    fn oscillation_detector_needs_full_window() {
        // Only 3 steps — below default window=5 — should not fire.
        let history = vec![
            make_step_with_knob(0, "k", 0.05),
            make_step_with_knob(1, "k", 0.07),
            make_step_with_knob(2, "k", 0.05),
        ];
        let detector = OscillationDetector::default();
        assert!(!detector.check(&history, "k"));
    }

    #[test]
    fn knob_clip_diagnostics_count_each_result() {
        let mut diag = KnobClipDiagnostics::default();
        diag.record("fraud.fraud_rate", KnobClipResult::InRange);
        diag.record("fraud.fraud_rate", KnobClipResult::InRange);
        diag.record("fraud.fraud_rate", KnobClipResult::ClippedHigh);
        diag.record("fraud.fraud_rate", KnobClipResult::ClippedLow);
        diag.record("pool.size", KnobClipResult::TypeMismatch);

        let fraud = diag.counts.get("fraud.fraud_rate").unwrap();
        assert_eq!(fraud.in_range, 2);
        assert_eq!(fraud.high, 1);
        assert_eq!(fraud.low, 1);
        let pool = diag.counts.get("pool.size").unwrap();
        assert_eq!(pool.type_mismatch, 1);
    }

    #[test]
    fn frequently_clipped_filters_by_threshold() {
        let mut diag = KnobClipDiagnostics::default();
        for _ in 0..5 {
            diag.record("often.clipped", KnobClipResult::ClippedHigh);
        }
        for _ in 0..2 {
            diag.record("rarely.clipped", KnobClipResult::ClippedLow);
        }
        diag.record("never.clipped", KnobClipResult::InRange);

        let flagged = diag.frequently_clipped(3);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0], "often.clipped");
    }

    #[test]
    fn wall_clock_budget_expires() {
        let budget = WallClockBudget::new(Duration::from_millis(50));
        assert!(!budget.expired());
        std::thread::sleep(Duration::from_millis(80));
        assert!(budget.expired());
        assert_eq!(budget.remaining(), Duration::ZERO);
    }
}

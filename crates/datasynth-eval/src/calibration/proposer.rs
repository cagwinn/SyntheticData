//! C3 Piece 2.5 — stock proposer implementations.
//!
//! Two ship in this piece:
//!
//! - [`GreedyKnobProposer`] — cycles through knobs and remembers
//!   the direction (positive or negative Δ) that most recently
//!   improved the loss for each knob. Re-proposes the same
//!   direction until it stops working, then flips. Self-contained
//!   coordinate-descent baseline that needs no external eval
//!   surface beyond the loop's own [`StepReport`] history.
//! - [`RoundRobinProposer`] — simpler still: always proposes
//!   `current ± max_step` alternating per call. Useful as a sanity
//!   baseline + as the proposer used in the C3 Piece 2 mock
//!   evaluator tests.
//!
//! An AutoTuner-backed proposer that consults
//! [`crate::enhancement::AutoTuner`] is a follow-up — that path
//! requires bridging the BF-report objective stream to the
//! AutoTuner's `ComprehensiveEvaluation` shape, which is more
//! work than fits this commit.

use std::collections::BTreeMap;

use super::knob::{CalibrationKnob, KnobValue};
use super::loop_runner::{ProposedPatch, Proposer, StepOutcome, StepReport};

/// Per-knob bookkeeping for [`GreedyKnobProposer`].
#[derive(Debug, Clone, Default)]
struct KnobState {
    /// `+1` or `-1` — the direction of the last proposed step.
    /// `0` means "no direction tried yet".
    last_direction: i32,
    /// True if the last step in that direction was credited as
    /// Improved by the loop; signals the proposer to keep going.
    last_improved: bool,
}

/// Coordinate-descent proposer: pick the next knob round-robin,
/// propose its current ± max_step in the direction that last
/// improved (default +). After a step that didn't improve, flip
/// the direction for that knob. Returns `None` only when every
/// knob has tried both directions without improvement, which
/// makes the loop stop with `ProposerExhausted`.
pub struct GreedyKnobProposer {
    /// Per-knob state.
    state: BTreeMap<String, KnobState>,
    /// Round-robin cursor — next knob index to try.
    next_knob: usize,
    /// Each knob's count of consecutive failed directions. When
    /// this hits 2 (both ± tried, neither worked), the knob is
    /// marked exhausted.
    fails_per_knob: BTreeMap<String, usize>,
}

impl Default for GreedyKnobProposer {
    fn default() -> Self {
        Self::new()
    }
}

impl GreedyKnobProposer {
    pub fn new() -> Self {
        Self {
            state: BTreeMap::new(),
            next_knob: 0,
            fails_per_knob: BTreeMap::new(),
        }
    }

    /// Per-knob exhaustion threshold. `fails ≥ MAX_FAILS` →
    /// proposer skips this knob entirely.
    const MAX_FAILS: usize = 2;
}

impl Proposer for GreedyKnobProposer {
    fn propose(
        &mut self,
        knobs: &[CalibrationKnob],
        _current_loss: (f64, f64),
        history: &[StepReport],
    ) -> Option<ProposedPatch> {
        if knobs.is_empty() {
            return None;
        }

        // Update state from the most recent step (if any).
        if let Some(last) = history.last() {
            if let Some(patch) = &last.proposed_patch {
                let path = knobs[patch.knob_index].path.clone();
                let improved = matches!(last.outcome, StepOutcome::Improved);
                let entry = self.state.entry(path.clone()).or_default();
                entry.last_improved = improved;
                if !improved {
                    *self.fails_per_knob.entry(path).or_default() += 1;
                }
            }
        }

        // Round-robin sweep starting at next_knob, looking for a
        // knob that hasn't exhausted both directions yet.
        for offset in 0..knobs.len() {
            let idx = (self.next_knob + offset) % knobs.len();
            let path = &knobs[idx].path;
            let fails = self.fails_per_knob.get(path).copied().unwrap_or(0);
            if fails >= Self::MAX_FAILS {
                continue;
            }

            let entry = self.state.entry(path.clone()).or_default();
            // Decide direction:
            //   - First time: try +1.
            //   - Last improved: keep same direction.
            //   - Last failed: flip direction.
            let dir = match (entry.last_direction, entry.last_improved) {
                (0, _) => 1,
                (d, true) => d,
                (d, false) => -d,
            };
            entry.last_direction = dir;
            entry.last_improved = false; // Will be updated next call.

            let cur = knobs[idx].current.as_f64();
            let step = knobs[idx].max_step * dir as f64;
            let proposed_f = cur + step;

            // Convert proposed value to the same KnobValue variant.
            let proposed = match knobs[idx].current {
                KnobValue::F64(_) => KnobValue::F64(proposed_f),
                KnobValue::Usize(_) => KnobValue::Usize(proposed_f.round().max(0.0) as usize),
            };

            self.next_knob = (idx + 1) % knobs.len();
            return Some(ProposedPatch {
                knob_index: idx,
                proposed_value: proposed,
                rationale: format!(
                    "greedy: knob `{}` direction {dir} step {step}",
                    knobs[idx].path
                ),
            });
        }

        // All knobs exhausted.
        None
    }
}

/// Stateless proposer that just steps the next knob in round-robin
/// order by `+max_step` each time. Mostly useful as a smoke test
/// fixture (or for the mock-evaluator unit tests).
pub struct RoundRobinProposer {
    pub next_knob: usize,
}

impl RoundRobinProposer {
    pub fn new() -> Self {
        Self { next_knob: 0 }
    }
}

impl Default for RoundRobinProposer {
    fn default() -> Self {
        Self::new()
    }
}

impl Proposer for RoundRobinProposer {
    fn propose(
        &mut self,
        knobs: &[CalibrationKnob],
        _current_loss: (f64, f64),
        _history: &[StepReport],
    ) -> Option<ProposedPatch> {
        if knobs.is_empty() {
            return None;
        }
        let idx = self.next_knob % knobs.len();
        self.next_knob = (self.next_knob + 1) % knobs.len();
        let cur = knobs[idx].current.as_f64();
        let proposed = match knobs[idx].current {
            KnobValue::F64(_) => KnobValue::F64(cur + knobs[idx].max_step),
            KnobValue::Usize(_) => {
                KnobValue::Usize((cur + knobs[idx].max_step).round().max(0.0) as usize)
            }
        };
        Some(ProposedPatch {
            knob_index: idx,
            proposed_value: proposed,
            rationale: format!("round-robin step on `{}`", knobs[idx].path),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::loop_runner::{ProposedPatch, StepReport};
    use std::collections::BTreeMap;

    fn knobs() -> Vec<CalibrationKnob> {
        vec![
            CalibrationKnob::new_f64("k.a", 0.05, 0.0, 1.0, 0.01),
            CalibrationKnob::new_f64("k.b", 0.10, 0.0, 1.0, 0.02),
        ]
    }

    fn step_with_outcome(
        iter: usize,
        knob_index: usize,
        proposed_value: KnobValue,
        outcome: StepOutcome,
    ) -> StepReport {
        StepReport {
            iter,
            loss_before_mean: 1.0,
            loss_before_std: 0.0,
            proposed_patch: Some(ProposedPatch {
                knob_index,
                proposed_value,
                rationale: "test".into(),
            }),
            loss_after_mean: Some(1.0),
            loss_after_std: Some(0.0),
            knob_values: BTreeMap::new(),
            outcome,
        }
    }

    #[test]
    fn round_robin_proposer_cycles_through_knobs() {
        let mut prop = RoundRobinProposer::new();
        let ks = knobs();
        let p1 = prop.propose(&ks, (0.0, 0.0), &[]).unwrap();
        assert_eq!(p1.knob_index, 0);
        let p2 = prop.propose(&ks, (0.0, 0.0), &[]).unwrap();
        assert_eq!(p2.knob_index, 1);
        let p3 = prop.propose(&ks, (0.0, 0.0), &[]).unwrap();
        assert_eq!(p3.knob_index, 0);
    }

    #[test]
    fn round_robin_proposer_steps_by_max_step() {
        let mut prop = RoundRobinProposer::new();
        let ks = knobs();
        let p = prop.propose(&ks, (0.0, 0.0), &[]).unwrap();
        // k.a starts at 0.05 + 0.01 max_step = 0.06.
        assert!(
            (p.proposed_value.as_f64() - 0.06).abs() < 1e-12,
            "expected 0.06, got {}",
            p.proposed_value
        );
    }

    #[test]
    fn round_robin_proposer_empty_knobs_returns_none() {
        let mut prop = RoundRobinProposer::new();
        assert!(prop.propose(&[], (0.0, 0.0), &[]).is_none());
    }

    #[test]
    fn greedy_proposer_first_call_picks_positive_direction() {
        let mut prop = GreedyKnobProposer::new();
        let ks = knobs();
        let p = prop.propose(&ks, (0.0, 0.0), &[]).unwrap();
        // First time on k.a → +max_step → 0.05 + 0.01 = 0.06.
        assert_eq!(p.knob_index, 0);
        assert!((p.proposed_value.as_f64() - 0.06).abs() < 1e-12);
    }

    #[test]
    fn greedy_proposer_continues_same_direction_after_improvement() {
        let mut prop = GreedyKnobProposer::new();
        let ks = knobs();

        // First call: propose +max_step on knob 0.
        let _p1 = prop.propose(&ks, (0.0, 0.0), &[]).unwrap();
        // Simulate a history where that step Improved.
        let h1 = vec![step_with_outcome(
            0,
            0,
            KnobValue::F64(0.06),
            StepOutcome::Improved,
        )];

        // Second call: should propose on knob 1 (round-robin), then
        // when we come back to knob 0 it should still go +.
        let p2 = prop.propose(&ks, (0.0, 0.0), &h1).unwrap();
        assert_eq!(p2.knob_index, 1);

        // Tell the proposer knob 1 also improved.
        let h2 = vec![
            step_with_outcome(0, 0, KnobValue::F64(0.06), StepOutcome::Improved),
            step_with_outcome(1, 1, KnobValue::F64(0.12), StepOutcome::Improved),
        ];

        // Third call: back to knob 0, direction should still be +.
        let p3 = prop.propose(&ks, (0.0, 0.0), &h2).unwrap();
        assert_eq!(p3.knob_index, 0);
        // Knob 0 is still at 0.05 in `ks` (we didn't mutate the
        // input slice in this test). Direction should be +1, so
        // proposed = 0.05 + 0.01.
        assert!((p3.proposed_value.as_f64() - 0.06).abs() < 1e-12);
    }

    #[test]
    fn greedy_proposer_flips_direction_after_failure() {
        let mut prop = GreedyKnobProposer::new();
        let ks = knobs();

        // First call on knob 0.
        let _p1 = prop.propose(&ks, (0.0, 0.0), &[]).unwrap();
        // Simulate that step being Reverted (no improvement).
        let h1 = vec![step_with_outcome(
            0,
            0,
            KnobValue::F64(0.06),
            StepOutcome::Reverted,
        )];

        // Round-robin advances to knob 1 first.
        let _p2 = prop.propose(&ks, (0.0, 0.0), &h1).unwrap();
        let h2 = vec![
            step_with_outcome(0, 0, KnobValue::F64(0.06), StepOutcome::Reverted),
            step_with_outcome(1, 1, KnobValue::F64(0.12), StepOutcome::Improved),
        ];

        // Back to knob 0: direction should now be -1.
        let p3 = prop.propose(&ks, (0.0, 0.0), &h2).unwrap();
        assert_eq!(p3.knob_index, 0);
        // Proposed should now be 0.05 - 0.01 = 0.04.
        assert!(
            (p3.proposed_value.as_f64() - 0.04).abs() < 1e-12,
            "expected 0.04 (flipped direction), got {}",
            p3.proposed_value
        );
    }

    #[test]
    fn greedy_proposer_exhausts_after_both_directions_fail() {
        let mut prop = GreedyKnobProposer::new();
        // Single-knob set so we can deterministically test exhaustion.
        let ks = vec![CalibrationKnob::new_f64("k", 0.05, 0.0, 1.0, 0.01)];

        // Step 1: + direction, Reverted.
        let _p1 = prop.propose(&ks, (0.0, 0.0), &[]).unwrap();
        let h1 = vec![step_with_outcome(
            0,
            0,
            KnobValue::F64(0.06),
            StepOutcome::Reverted,
        )];
        // Step 2: - direction, Reverted.
        let _p2 = prop.propose(&ks, (0.0, 0.0), &h1).unwrap();
        let h2 = vec![
            step_with_outcome(0, 0, KnobValue::F64(0.06), StepOutcome::Reverted),
            step_with_outcome(1, 0, KnobValue::F64(0.04), StepOutcome::Reverted),
        ];

        // Step 3: both directions failed → proposer should give up.
        let p3 = prop.propose(&ks, (0.0, 0.0), &h2);
        assert!(
            p3.is_none(),
            "proposer should exhaust after both directions fail; got {:?}",
            p3.map(|p| p.proposed_value)
        );
    }
}

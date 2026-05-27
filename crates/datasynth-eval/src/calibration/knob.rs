//! C3 Piece 1 — calibration knob (parameter space element).
//!
//! Each [`CalibrationKnob`] names one tunable engine parameter, its
//! current value, its allowable range, and the maximum step size
//! the calibration loop is permitted to propose per iteration.
//! Knobs are typed via [`KnobValue`] — initial cut covers `f64`
//! (rates, ratios) and `usize` (pool sizes); discrete-string knobs
//! are deferred.
//!
//! Values round-trip through YAML config patches as strings so a
//! `--patch fraud.fraud_rate=0.03` flag works the same way the
//! existing `AutoTuner::ConfigPatch.suggested_value` shape does.

use std::fmt;

use serde::{Deserialize, Serialize};

/// One value a knob can hold.
///
/// Strings round-trip via YAML; numbers are parsed/formatted when
/// the caller writes a config patch.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum KnobValue {
    /// f64 — e.g. `fraud.fraud_rate = 0.02`.
    F64(f64),
    /// usize — e.g. `concentration.trading_partner_pool.target_size = 12`.
    Usize(usize),
}

impl KnobValue {
    /// YAML-ready string form: float as `"0.02"`, int as `"12"`.
    pub fn to_yaml_string(&self) -> String {
        match self {
            Self::F64(v) => format!("{v}"),
            Self::Usize(v) => v.to_string(),
        }
    }

    /// Numeric magnitude (for comparing two values or computing a
    /// step size). Both variants project to `f64`.
    pub fn as_f64(&self) -> f64 {
        match self {
            Self::F64(v) => *v,
            Self::Usize(v) => *v as f64,
        }
    }
}

impl fmt::Display for KnobValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_yaml_string())
    }
}

/// Allowable range for a knob's value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KnobBounds {
    /// `f64 ∈ [min, max]`.
    F64Range { min: f64, max: f64 },
    /// `usize ∈ [min, max]`.
    UsizeRange { min: usize, max: usize },
}

impl KnobBounds {
    /// Range width — `max - min`. Used as a default step-size scale.
    pub fn width(&self) -> f64 {
        match self {
            Self::F64Range { min, max } => max - min,
            Self::UsizeRange { min, max } => (max - min) as f64,
        }
    }

    /// Check whether the bounds variant matches a [`KnobValue`].
    /// Returns false on type mismatch (e.g. F64Range for a Usize
    /// value) — a constructor + setter should never produce such a
    /// pair, but the loop's safety rail tests for it defensively.
    pub fn matches_value(&self, value: &KnobValue) -> bool {
        matches!(
            (self, value),
            (Self::F64Range { .. }, KnobValue::F64(_))
                | (Self::UsizeRange { .. }, KnobValue::Usize(_))
        )
    }
}

/// Outcome of [`CalibrationKnob::clip`]: whether the proposed value
/// was inside bounds, or was clamped to a bound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KnobClipResult {
    /// Proposed value was inside bounds; final value == proposed.
    InRange,
    /// Proposed value was below the lower bound; clamped to `min`.
    ClippedLow,
    /// Proposed value was above the upper bound; clamped to `max`.
    ClippedHigh,
    /// Type mismatch between bounds and proposed value — final
    /// value == current (no change). Should never occur in
    /// well-constructed knobs; the loop logs this as a bug.
    TypeMismatch,
}

/// One tunable engine parameter.
#[derive(Debug, Clone)]
pub struct CalibrationKnob {
    /// Config-tree path the knob writes (e.g. `"fraud.fraud_rate"`).
    /// Must match the path the orchestrator's config-patcher
    /// understands — same format as
    /// [`crate::enhancement::ConfigPatch::path`].
    pub path: String,
    /// Current value.
    pub current: KnobValue,
    /// Allowable bounds.
    pub bounds: KnobBounds,
    /// Maximum step size (numeric magnitude) the loop may propose
    /// per iteration. Patches with `|Δ| > max_step` are clipped to
    /// `±max_step`. Use this to prevent the optimizer from making
    /// a giant jump on a single noisy gradient.
    pub max_step: f64,
}

impl CalibrationKnob {
    /// Convenience constructor for an f64 knob.
    pub fn new_f64(
        path: impl Into<String>,
        current: f64,
        min: f64,
        max: f64,
        max_step: f64,
    ) -> Self {
        Self {
            path: path.into(),
            current: KnobValue::F64(current),
            bounds: KnobBounds::F64Range { min, max },
            max_step,
        }
    }

    /// Convenience constructor for a usize knob.
    pub fn new_usize(
        path: impl Into<String>,
        current: usize,
        min: usize,
        max: usize,
        max_step: f64,
    ) -> Self {
        Self {
            path: path.into(),
            current: KnobValue::Usize(current),
            bounds: KnobBounds::UsizeRange { min, max },
            max_step,
        }
    }

    /// Clip `proposed` to the knob's bounds AND step budget.
    /// Returns the final value + a [`KnobClipResult`] describing
    /// whether any clipping happened. Does NOT mutate `self`.
    pub fn clip(&self, proposed: KnobValue) -> (KnobValue, KnobClipResult) {
        if !self.bounds.matches_value(&proposed) {
            return (self.current, KnobClipResult::TypeMismatch);
        }

        let cur_f = self.current.as_f64();
        let prop_f = proposed.as_f64();

        // Step-size clip.
        let delta = prop_f - cur_f;
        let stepped_f = if delta.abs() > self.max_step {
            cur_f + delta.signum() * self.max_step
        } else {
            prop_f
        };

        // Bounds clip.
        let (clipped_f, bound_result) = match self.bounds {
            KnobBounds::F64Range { min, max } => {
                if stepped_f < min {
                    (min, KnobClipResult::ClippedLow)
                } else if stepped_f > max {
                    (max, KnobClipResult::ClippedHigh)
                } else {
                    (stepped_f, KnobClipResult::InRange)
                }
            }
            KnobBounds::UsizeRange { min, max } => {
                // Round to nearest non-negative integer first.
                let i = stepped_f.round().max(0.0) as usize;
                if i < min {
                    (min as f64, KnobClipResult::ClippedLow)
                } else if i > max {
                    (max as f64, KnobClipResult::ClippedHigh)
                } else {
                    (i as f64, KnobClipResult::InRange)
                }
            }
        };

        let final_v = match proposed {
            KnobValue::F64(_) => KnobValue::F64(clipped_f),
            KnobValue::Usize(_) => KnobValue::Usize(clipped_f as usize),
        };
        (final_v, bound_result)
    }

    /// Apply a clipped value to the knob (mutates `self.current`).
    /// Returns the same [`KnobClipResult`] [`Self::clip`] returns.
    pub fn apply(&mut self, proposed: KnobValue) -> KnobClipResult {
        let (final_v, result) = self.clip(proposed);
        self.current = final_v;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f64_clip_in_range_is_noop() {
        let mut k = CalibrationKnob::new_f64("fraud.fraud_rate", 0.02, 0.0, 0.1, 0.01);
        let r = k.apply(KnobValue::F64(0.025));
        assert_eq!(r, KnobClipResult::InRange);
        assert_eq!(k.current, KnobValue::F64(0.025));
    }

    #[test]
    fn f64_clip_low_clamps_to_min() {
        let mut k = CalibrationKnob::new_f64("fraud.fraud_rate", 0.02, 0.01, 0.1, 1.0);
        // Step budget is wide so the step doesn't clip first;
        // the bounds clip dominates.
        let r = k.apply(KnobValue::F64(0.001));
        assert_eq!(r, KnobClipResult::ClippedLow);
        assert_eq!(k.current, KnobValue::F64(0.01));
    }

    #[test]
    fn f64_clip_high_clamps_to_max() {
        let mut k = CalibrationKnob::new_f64("fraud.fraud_rate", 0.02, 0.0, 0.05, 1.0);
        let r = k.apply(KnobValue::F64(0.1));
        assert_eq!(r, KnobClipResult::ClippedHigh);
        assert_eq!(k.current, KnobValue::F64(0.05));
    }

    #[test]
    fn f64_step_size_clamps_large_jumps() {
        // Bounds wide, step tight: a jump from 0.02 → 0.05 (Δ=0.03)
        // exceeds max_step=0.01, so the value lands at 0.02+0.01.
        let mut k = CalibrationKnob::new_f64("fraud.fraud_rate", 0.02, 0.0, 0.1, 0.01);
        let r = k.apply(KnobValue::F64(0.05));
        assert_eq!(r, KnobClipResult::InRange);
        assert!(
            (k.current.as_f64() - 0.03).abs() < 1e-12,
            "value should land at 0.02 + 0.01 step = 0.03, got {}",
            k.current
        );
    }

    #[test]
    fn f64_step_size_clamps_negative_jumps() {
        let mut k = CalibrationKnob::new_f64("rate", 0.10, 0.0, 1.0, 0.01);
        // Δ = -0.08, step budget 0.01 → final = 0.10 - 0.01 = 0.09.
        let r = k.apply(KnobValue::F64(0.02));
        assert_eq!(r, KnobClipResult::InRange);
        assert!((k.current.as_f64() - 0.09).abs() < 1e-12);
    }

    #[test]
    fn usize_clip_rounds_and_clamps() {
        let mut k = CalibrationKnob::new_usize("pool.target_size", 12, 5, 20, 4.0);
        // Δ = 5, exceeds step budget 4, so step lands at 16.
        let r = k.apply(KnobValue::Usize(17));
        assert_eq!(r, KnobClipResult::InRange);
        assert_eq!(k.current, KnobValue::Usize(16));
    }

    #[test]
    fn usize_clip_low_clamps_to_min() {
        let mut k = CalibrationKnob::new_usize("pool.target_size", 12, 5, 20, 100.0);
        let r = k.apply(KnobValue::Usize(2));
        assert_eq!(r, KnobClipResult::ClippedLow);
        assert_eq!(k.current, KnobValue::Usize(5));
    }

    #[test]
    fn type_mismatch_is_noop() {
        let mut k = CalibrationKnob::new_f64("fraud.fraud_rate", 0.02, 0.0, 0.1, 0.01);
        let r = k.apply(KnobValue::Usize(12));
        assert_eq!(r, KnobClipResult::TypeMismatch);
        // Knob unchanged.
        assert_eq!(k.current, KnobValue::F64(0.02));
    }

    #[test]
    fn yaml_string_round_trips() {
        assert_eq!(KnobValue::F64(0.025).to_yaml_string(), "0.025");
        assert_eq!(KnobValue::Usize(12).to_yaml_string(), "12");
    }

    #[test]
    fn bounds_width_is_max_minus_min() {
        assert!((KnobBounds::F64Range { min: 0.0, max: 0.1 }.width() - 0.1).abs() < 1e-12);
        assert_eq!(KnobBounds::UsizeRange { min: 5, max: 20 }.width(), 15.0);
    }
}

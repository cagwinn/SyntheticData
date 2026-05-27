//! C3 Piece 3 — calibration history persistence.
//!
//! Wraps the loop's `Vec<StepReport>` + best-tracker in a
//! serialisable container so a long-running calibration can be
//! interrupted and resumed without losing trajectory or knob state.
//!
//! Persistence shape (JSON):
//!
//! ```json
//! {
//!   "schema_version": "1.0",
//!   "objective_metric": "bf_composite",
//!   "steps": [ ... StepReport ... ],
//!   "best_loss_mean": 38.7,
//!   "best_loss_std": 4.2,
//!   "best_knob_values": { "fraud.fraud_rate": "0.02", ... }
//! }
//! ```
//!
//! The file is rewritten after every step so an interrupted loop
//! never loses more than the in-flight step. The schema version
//! lets future loop changes detect and reject incompatible files
//! rather than silently mis-resume.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::knob::KnobValue;
use super::loop_runner::{CalibrationLoop, StepReport};

/// Current persistence schema version. Bump on any breaking change
/// to `StepReport` / `KnobValue` / `CalibrationObjective` serde
/// shape. `load` rejects mismatched versions.
pub const HISTORY_SCHEMA_VERSION: &str = "1.0";

/// Errors from loading a saved history.
#[derive(Debug)]
pub enum HistoryError {
    Io(std::io::Error),
    Parse(serde_json::Error),
    SchemaMismatch {
        found: String,
        expected: &'static str,
    },
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "history IO: {e}"),
            Self::Parse(e) => write!(f, "history JSON parse: {e}"),
            Self::SchemaMismatch { found, expected } => write!(
                f,
                "history schema mismatch: file declares {found}, runtime expects {expected}"
            ),
        }
    }
}

impl std::error::Error for HistoryError {}

impl From<std::io::Error> for HistoryError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for HistoryError {
    fn from(e: serde_json::Error) -> Self {
        Self::Parse(e)
    }
}

/// Persistable snapshot of a [`CalibrationLoop`]'s trajectory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationHistory {
    /// Schema version of the file format. See
    /// [`HISTORY_SCHEMA_VERSION`].
    pub schema_version: String,
    /// Objective the loop was minimising — recorded so resume can
    /// refuse to load history whose objective differs from the
    /// runtime's (would yield meaningless comparisons).
    pub objective_metric: String,
    /// Per-step trajectory in order. Empty until the first
    /// `loop.step()` returns.
    pub steps: Vec<StepReport>,
    /// Best loss mean seen so far. `None` if no step has produced
    /// a measurement yet.
    pub best_loss_mean: Option<f64>,
    /// Best loss std at the same time `best_loss_mean` was
    /// recorded.
    pub best_loss_std: Option<f64>,
    /// Knob values at the best-loss point. Maps config-tree path
    /// → value (stringified via `KnobValue::to_yaml_string`).
    pub best_knob_values: BTreeMap<String, KnobValue>,
}

impl CalibrationHistory {
    /// Build a history snapshot from the current state of a
    /// running [`CalibrationLoop`]. Cheap — clones the steps Vec
    /// + best-tracker maps.
    pub fn from_loop(loop_: &CalibrationLoop) -> Self {
        Self {
            schema_version: HISTORY_SCHEMA_VERSION.to_string(),
            objective_metric: loop_.objective.metric.name().to_string(),
            steps: loop_.history.clone(),
            best_loss_mean: loop_.best_loss.map(|(m, _)| m),
            best_loss_std: loop_.best_loss.map(|(_, s)| s),
            best_knob_values: loop_.best_knob_values.clone(),
        }
    }

    /// Serialise to `path` as pretty-printed JSON. Atomic: writes
    /// to `path.tmp` first then renames, so an interruption mid-
    /// write doesn't corrupt the existing file.
    pub fn save(&self, path: &Path) -> Result<(), HistoryError> {
        let tmp = path.with_extension("tmp");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Load from `path`. Rejects mismatched schema versions so a
    /// resume against an incompatible file fails loud rather than
    /// silent.
    pub fn load(path: &Path) -> Result<Self, HistoryError> {
        let bytes = std::fs::read(path)?;
        let parsed: Self = serde_json::from_slice(&bytes)?;
        if parsed.schema_version != HISTORY_SCHEMA_VERSION {
            return Err(HistoryError::SchemaMismatch {
                found: parsed.schema_version,
                expected: HISTORY_SCHEMA_VERSION,
            });
        }
        Ok(parsed)
    }

    /// Apply this history to a fresh [`CalibrationLoop`] so the
    /// loop resumes from where it stopped.
    ///
    /// Caller is responsible for ensuring the loop's objective +
    /// knob set + config match what was used when the history was
    /// produced; we check the objective metric name as a soft
    /// guard but full structural equality on the knob vector is
    /// out of scope for the first cut (the design doc deferred
    /// strict resume safety to Piece 5).
    pub fn apply_to(&self, loop_: &mut CalibrationLoop) -> Result<(), HistoryError> {
        // Soft objective check.
        if self.objective_metric != loop_.objective.metric.name() {
            return Err(HistoryError::SchemaMismatch {
                found: self.objective_metric.clone(),
                expected: loop_.objective.metric.name(),
            });
        }
        loop_.history = self.steps.clone();
        loop_.best_loss = match (self.best_loss_mean, self.best_loss_std) {
            (Some(m), Some(s)) => Some((m, s)),
            (Some(m), None) => Some((m, 0.0)),
            _ => None,
        };
        loop_.best_knob_values = self.best_knob_values.clone();
        // Restore each knob's current value from the most recent
        // step's `knob_values` (if any). This is what makes the
        // resume actually continue from the last applied state.
        if let Some(last) = self.steps.last() {
            for knob in &mut loop_.knobs {
                if let Some(v) = last.knob_values.get(&knob.path) {
                    knob.current = *v;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::knob::CalibrationKnob;
    use crate::calibration::loop_runner::{CalibrationConfig, StepOutcome};
    use crate::calibration::objective::CalibrationObjective;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn empty_loop() -> CalibrationLoop {
        CalibrationLoop::new(
            CalibrationObjective::bf_composite(),
            vec![CalibrationKnob::new_f64("test.rate", 0.10, 0.0, 1.0, 0.02)],
            CalibrationConfig::default(),
        )
    }

    fn fake_step(iter: usize, before: f64, after: f64, knob_value: f64) -> StepReport {
        let mut kv = BTreeMap::new();
        kv.insert("test.rate".to_string(), KnobValue::F64(knob_value));
        StepReport {
            iter,
            loss_before_mean: before,
            loss_before_std: 1.0,
            proposed_patch: None,
            loss_after_mean: Some(after),
            loss_after_std: Some(1.0),
            knob_values: kv,
            outcome: StepOutcome::Improved,
        }
    }

    #[test]
    fn save_and_load_round_trips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("calibration_history.json");

        let mut loop_ = empty_loop();
        loop_.history.push(fake_step(0, 50.0, 45.0, 0.08));
        loop_.history.push(fake_step(1, 45.0, 40.0, 0.06));
        loop_.best_loss = Some((40.0, 1.0));
        loop_
            .best_knob_values
            .insert("test.rate".into(), KnobValue::F64(0.06));

        let history = CalibrationHistory::from_loop(&loop_);
        history.save(&path).unwrap();

        let loaded = CalibrationHistory::load(&path).unwrap();
        assert_eq!(loaded.schema_version, HISTORY_SCHEMA_VERSION);
        assert_eq!(loaded.objective_metric, "bf_composite");
        assert_eq!(loaded.steps.len(), 2);
        assert_eq!(loaded.best_loss_mean, Some(40.0));
        assert_eq!(
            loaded.best_knob_values.get("test.rate"),
            Some(&KnobValue::F64(0.06))
        );
    }

    #[test]
    fn schema_mismatch_rejected_on_load() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("history.json");
        let bad = r#"{
            "schema_version": "99.99",
            "objective_metric": "bf_composite",
            "steps": [],
            "best_loss_mean": null,
            "best_loss_std": null,
            "best_knob_values": {}
        }"#;
        std::fs::write(&path, bad).unwrap();
        let err = CalibrationHistory::load(&path).expect_err("schema must mismatch");
        assert!(
            matches!(err, HistoryError::SchemaMismatch { .. }),
            "expected SchemaMismatch, got {err:?}"
        );
    }

    #[test]
    fn apply_to_restores_knob_state_and_history() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("h.json");

        // Source loop: 1 step, knob at 0.06.
        let mut src = empty_loop();
        src.history.push(fake_step(0, 50.0, 45.0, 0.06));
        src.best_loss = Some((45.0, 1.0));
        src.best_knob_values
            .insert("test.rate".into(), KnobValue::F64(0.06));
        src.knobs[0].current = KnobValue::F64(0.06);

        CalibrationHistory::from_loop(&src).save(&path).unwrap();

        // Fresh loop: knob at 0.10 (the default), no history.
        let mut dst = empty_loop();
        assert_eq!(dst.knobs[0].current.as_f64(), 0.10);
        assert!(dst.history.is_empty());

        CalibrationHistory::load(&path)
            .unwrap()
            .apply_to(&mut dst)
            .unwrap();

        assert_eq!(dst.history.len(), 1);
        assert_eq!(dst.best_loss, Some((45.0, 1.0)));
        assert!(
            (dst.knobs[0].current.as_f64() - 0.06).abs() < 1e-9,
            "knob should resume to last-step value: got {}",
            dst.knobs[0].current
        );
    }

    #[test]
    fn apply_to_rejects_objective_mismatch() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("h.json");

        // Save a history with bf_composite objective.
        let src = empty_loop();
        CalibrationHistory::from_loop(&src).save(&path).unwrap();

        // Build a destination loop with a DIFFERENT objective.
        let mut dst = CalibrationLoop::new(
            CalibrationObjective::default()
                .with_metric(crate::calibration::ObjectiveMetric::BfCompositeMedian),
            vec![CalibrationKnob::new_f64("test.rate", 0.10, 0.0, 1.0, 0.02)],
            CalibrationConfig::default(),
        );

        let err = CalibrationHistory::load(&path)
            .unwrap()
            .apply_to(&mut dst)
            .expect_err("objective mismatch must reject");
        assert!(matches!(err, HistoryError::SchemaMismatch { .. }));
    }

    #[test]
    fn save_uses_atomic_rename() {
        // Smoke test that the .tmp file doesn't linger after a
        // successful save. We can't easily test the actual atomic-
        // ness without process injection, but we can verify the
        // observable contract.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("history.json");
        let tmp_path = path.with_extension("tmp");

        let loop_ = empty_loop();
        CalibrationHistory::from_loop(&loop_).save(&path).unwrap();

        assert!(path.exists(), "target file should exist after save");
        assert!(
            !tmp_path.exists(),
            "tmp staging file should be renamed away after save"
        );
    }
}

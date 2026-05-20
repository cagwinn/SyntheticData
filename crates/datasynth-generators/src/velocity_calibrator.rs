//! SP3.4 — online velocity-rule calibrator.
//!
//! Periodically (every N=10,000 emitted lines), compares current rule-trigger
//! rates against the target rates derived from a `BehavioralPriors`. For the
//! most-off rule, proposes a bounded parameter nudge. Hysteresis prevents
//! direction-flip oscillation.

use std::collections::HashMap;

use chrono::{Datelike, Weekday};

use datasynth_core::models::journal_entry::JournalEntryLine;

/// One adjustment proposed by the calibrator.
#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationStep {
    pub rule_id: String,
    pub parameter: String,
    pub delta: f64,
    pub new_value: f64,
}

/// Online calibrator. Lives on the generator state.
pub struct VelocityCalibrator {
    pub n_lines_between_calibrations: usize,
    pub target_trigger_rates: HashMap<String, f64>,
    current_window_counts: HashMap<String, u64>,
    current_window_total: u64,
    last_direction: HashMap<String, i32>,
    windows_since_direction_change: HashMap<String, u32>,
    pub adjustments: Vec<CalibrationStep>,
    bounds: HashMap<String, (f64, f64)>,
    pub current_values: HashMap<String, f64>,
    step_sizes: HashMap<String, f64>,
}

impl VelocityCalibrator {
    pub fn new(target_trigger_rates: HashMap<String, f64>, n_lines: usize) -> Self {
        let mut bounds: HashMap<String, (f64, f64)> = HashMap::new();
        bounds.insert("R6".into(), (1.0, 4.0));
        bounds.insert("R7".into(), (0.0, 0.5));
        bounds.insert("R8".into(), (0.0, 0.3));
        bounds.insert("R9".into(), (0.0, 0.5));
        bounds.insert("R10".into(), (0.0, 0.3));
        let mut step_sizes: HashMap<String, f64> = HashMap::new();
        step_sizes.insert("R6".into(), 0.01);
        step_sizes.insert("R7".into(), 0.005);
        step_sizes.insert("R8".into(), 0.005);
        step_sizes.insert("R9".into(), 0.005);
        step_sizes.insert("R10".into(), 0.005);
        Self {
            n_lines_between_calibrations: n_lines.max(1),
            target_trigger_rates,
            current_window_counts: HashMap::new(),
            current_window_total: 0,
            last_direction: HashMap::new(),
            windows_since_direction_change: HashMap::new(),
            adjustments: Vec::new(),
            bounds,
            current_values: HashMap::new(),
            step_sizes,
        }
    }

    /// Called per emitted line. Counts trigger events; when window full, may
    /// return a `CalibrationStep` for the most-off rule.
    pub fn observe_line(&mut self, line: &JournalEntryLine) -> Option<CalibrationStep> {
        self.current_window_total += 1;
        if Self::is_off_hours(line) {
            *self.current_window_counts.entry("R7".into()).or_insert(0) += 1;
        }
        if Self::is_round_dollar(line) {
            *self.current_window_counts.entry("R9".into()).or_insert(0) += 1;
        }
        if self.current_window_total < self.n_lines_between_calibrations as u64 {
            return None;
        }
        let step = self.propose_step();
        self.current_window_counts.clear();
        self.current_window_total = 0;
        step
    }

    fn propose_step(&mut self) -> Option<CalibrationStep> {
        let mut worst: Option<(String, f64)> = None;
        for (rule_id, &target) in &self.target_trigger_rates {
            let observed = *self.current_window_counts.get(rule_id).unwrap_or(&0) as f64
                / self.current_window_total.max(1) as f64;
            let signed = observed - target;
            let abs = signed.abs();
            if worst
                .as_ref()
                .is_none_or(|(_, prev_abs)| abs > prev_abs.abs())
            {
                worst = Some((rule_id.clone(), signed));
            }
        }
        let (rule_id, signed_delta) = worst?;
        if signed_delta.abs() < 1e-9 {
            return None;
        }
        let step_size = *self.step_sizes.get(&rule_id)?;
        let (lo, hi) = *self.bounds.get(&rule_id)?;
        let parameter = match rule_id.as_str() {
            "R6" => "amounts.lognormal_sigma",
            "R7" => "posting.off_hours_share",
            "R8" => "period_close.post_close_share",
            "R9" => "amounts.round_dollar_share",
            "R10" => "posting.backdating_share",
            _ => return None,
        };
        let direction = if signed_delta > 0.0 { -1 } else { 1 };
        let last_dir = *self.last_direction.get(&rule_id).unwrap_or(&0);
        let windows_since = *self
            .windows_since_direction_change
            .get(&rule_id)
            .unwrap_or(&999);
        if last_dir != 0 && direction != last_dir && windows_since < 5 {
            self.windows_since_direction_change
                .insert(rule_id, windows_since + 1);
            return None;
        }
        let current = *self
            .current_values
            .get(parameter)
            .unwrap_or(&((lo + hi) / 2.0));
        let new_value = (current + direction as f64 * step_size).clamp(lo, hi);
        if (new_value - current).abs() < 1e-9 {
            return None;
        }
        self.current_values.insert(parameter.to_string(), new_value);
        self.last_direction.insert(rule_id.clone(), direction);
        if last_dir != direction {
            self.windows_since_direction_change
                .insert(rule_id.clone(), 0);
        }
        let step = CalibrationStep {
            rule_id,
            parameter: parameter.to_string(),
            delta: direction as f64 * step_size,
            new_value,
        };
        self.adjustments.push(step.clone());
        Some(step)
    }

    fn is_off_hours(line: &JournalEntryLine) -> bool {
        if let Some(d) = line.value_date {
            matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
        } else {
            false
        }
    }

    fn is_round_dollar(line: &JournalEntryLine) -> bool {
        let amt = line.local_amount.abs();
        let amt_f64 = decimal_to_f64(amt);
        let amt_i = amt_f64.round() as i64;
        amt_i > 0 && amt_i % 1000 == 0
    }
}

fn decimal_to_f64(d: rust_decimal::Decimal) -> f64 {
    use rust_decimal::prelude::ToPrimitive;
    d.to_f64().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    fn line_with_amount(amount: f64) -> JournalEntryLine {
        use rust_decimal::prelude::FromPrimitive;
        JournalEntryLine {
            local_amount: Decimal::from_f64(amount).unwrap_or(Decimal::ZERO),
            value_date: Some(NaiveDate::from_ymd_opt(2026, 1, 5).unwrap()), // Mon
            ..JournalEntryLine::default()
        }
    }

    #[test]
    fn calibrator_proposes_step_when_off_target() {
        let mut targets = HashMap::new();
        targets.insert("R9".into(), 0.10);
        let mut cal = VelocityCalibrator::new(targets, 10);
        cal.current_values
            .insert("amounts.round_dollar_share".into(), 0.30);

        let mut step: Option<CalibrationStep> = None;
        // 10 lines, 5 round-dollar (1000) → observed 0.5 vs target 0.10 → propose down.
        for i in 0..10 {
            let amt = if i % 2 == 0 { 1000.0 } else { 1500.0 };
            let s = cal.observe_line(&line_with_amount(amt));
            if s.is_some() {
                step = s;
            }
        }
        let s = step.expect("step proposed");
        assert_eq!(s.rule_id, "R9");
        assert!(s.delta < 0.0, "expected downward delta, got {}", s.delta);
        assert!(s.new_value < 0.30);
    }

    #[test]
    fn calibrator_no_step_when_at_target() {
        let mut targets = HashMap::new();
        targets.insert("R9".into(), 0.5); // We'll feed 5/10 = 0.5 round-dollar.
        let mut cal = VelocityCalibrator::new(targets, 10);
        let mut step: Option<CalibrationStep> = None;
        for i in 0..10 {
            let amt = if i % 2 == 0 { 1000.0 } else { 1500.0 };
            let s = cal.observe_line(&line_with_amount(amt));
            if s.is_some() {
                step = s;
            }
        }
        // At target → step delta should be tiny / None.
        assert!(step.is_none() || step.unwrap().delta.abs() < 1e-9);
    }
}

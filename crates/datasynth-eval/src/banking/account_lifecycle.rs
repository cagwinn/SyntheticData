//! Account lifecycle quality evaluator.
//!
//! Validates that account lifecycle phases are distributed realistically:
//! - Multiple phases represented (not all stuck in one)
//! - Transitions occur at reasonable rates
//! - Life events contribute to transitions

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::EvalResult;

/// Per-account lifecycle end-state observation.
#[derive(Debug, Clone)]
pub struct LifecycleEndState {
    pub lifecycle_phase: String, // "new" | "ramp_up" | "steady" | "decline" | "dormant"
    pub days_since_opening: u32,
}

/// Transition record.
#[derive(Debug, Clone)]
pub struct TransitionRecord {
    pub from_phase: String,
    pub to_phase: String,
    pub triggered_by_event: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleThresholds {
    /// Minimum number of distinct phases that should be represented
    pub min_phase_diversity: usize,
    /// Minimum fraction of accounts that moved beyond New
    pub min_progression_rate: f64,
    /// Minimum fraction of transitions that are event-driven
    pub min_event_driven_rate: f64,
    /// Maximum fraction of accounts stuck in New despite age > 180 days
    pub max_stuck_new_rate: f64,
}

impl Default for LifecycleThresholds {
    fn default() -> Self {
        Self {
            min_phase_diversity: 3,
            min_progression_rate: 0.70,
            min_event_driven_rate: 0.10,
            max_stuck_new_rate: 0.05,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleAnalysis {
    pub total_accounts: usize,
    pub phase_distribution: HashMap<String, f64>,
    pub progression_rate: f64,
    pub event_driven_rate: f64,
    pub stuck_new_rate: f64,
    pub phases_observed: usize,
    pub total_transitions: usize,
    pub passes: bool,
    pub issues: Vec<String>,
}

pub struct LifecycleAnalyzer {
    pub thresholds: LifecycleThresholds,
}

impl LifecycleAnalyzer {
    pub fn new() -> Self {
        Self {
            thresholds: LifecycleThresholds::default(),
        }
    }

    pub fn analyze(
        &self,
        end_states: &[LifecycleEndState],
        transitions: &[TransitionRecord],
    ) -> EvalResult<LifecycleAnalysis> {
        let total = end_states.len();
        if total == 0 {
            return Ok(LifecycleAnalysis {
                total_accounts: 0,
                phase_distribution: HashMap::new(),
                progression_rate: 0.0,
                event_driven_rate: 0.0,
                stuck_new_rate: 0.0,
                phases_observed: 0,
                total_transitions: 0,
                passes: true,
                issues: Vec::new(),
            });
        }

        let mut phase_counts: HashMap<String, usize> = HashMap::new();
        for s in end_states {
            *phase_counts.entry(s.lifecycle_phase.clone()).or_insert(0) += 1;
        }
        let phase_distribution: HashMap<String, f64> = phase_counts
            .iter()
            .map(|(k, v)| (k.clone(), *v as f64 / total as f64))
            .collect();
        let phases_observed = phase_distribution.len();

        let progressed = end_states
            .iter()
            .filter(|s| s.lifecycle_phase != "new")
            .count();
        let progression_rate = progressed as f64 / total as f64;

        // Accounts older than 180d still stuck in New
        let stuck_new = end_states
            .iter()
            .filter(|s| s.lifecycle_phase == "new" && s.days_since_opening > 180)
            .count();
        let stuck_new_rate = stuck_new as f64 / total as f64;

        let event_driven_rate = if !transitions.is_empty() {
            let event_count = transitions.iter().filter(|t| t.triggered_by_event).count();
            event_count as f64 / transitions.len() as f64
        } else {
            1.0
        };

        let mut issues = Vec::new();
        if phases_observed < self.thresholds.min_phase_diversity {
            issues.push(format!(
                "Only {} phases observed — expected at least {}",
                phases_observed, self.thresholds.min_phase_diversity,
            ));
        }
        if progression_rate < self.thresholds.min_progression_rate {
            issues.push(format!(
                "Progression rate {:.1}% below minimum {:.1}%",
                progression_rate * 100.0,
                self.thresholds.min_progression_rate * 100.0,
            ));
        }
        if stuck_new_rate > self.thresholds.max_stuck_new_rate {
            issues.push(format!(
                "{:.1}% of accounts stuck in New phase despite >180d age",
                stuck_new_rate * 100.0,
            ));
        }
        if !transitions.is_empty() && event_driven_rate < self.thresholds.min_event_driven_rate {
            issues.push(format!(
                "Event-driven transition rate {:.1}% below minimum {:.1}%",
                event_driven_rate * 100.0,
                self.thresholds.min_event_driven_rate * 100.0,
            ));
        }

        Ok(LifecycleAnalysis {
            total_accounts: total,
            phase_distribution,
            progression_rate,
            event_driven_rate,
            stuck_new_rate,
            phases_observed,
            total_transitions: transitions.len(),
            passes: issues.is_empty(),
            issues,
        })
    }
}

impl Default for LifecycleAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_realistic_lifecycle_passes() {
        let states = vec![
            // 20% new (young accounts)
            LifecycleEndState {
                lifecycle_phase: "new".into(),
                days_since_opening: 20,
            },
            LifecycleEndState {
                lifecycle_phase: "new".into(),
                days_since_opening: 25,
            },
            // 30% ramp_up
            LifecycleEndState {
                lifecycle_phase: "ramp_up".into(),
                days_since_opening: 50,
            },
            LifecycleEndState {
                lifecycle_phase: "ramp_up".into(),
                days_since_opening: 80,
            },
            LifecycleEndState {
                lifecycle_phase: "ramp_up".into(),
                days_since_opening: 70,
            },
            // 40% steady
            LifecycleEndState {
                lifecycle_phase: "steady".into(),
                days_since_opening: 200,
            },
            LifecycleEndState {
                lifecycle_phase: "steady".into(),
                days_since_opening: 300,
            },
            LifecycleEndState {
                lifecycle_phase: "steady".into(),
                days_since_opening: 350,
            },
            LifecycleEndState {
                lifecycle_phase: "steady".into(),
                days_since_opening: 250,
            },
            // 10% decline/dormant
            LifecycleEndState {
                lifecycle_phase: "dormant".into(),
                days_since_opening: 400,
            },
        ];
        let transitions = vec![
            TransitionRecord {
                from_phase: "new".into(),
                to_phase: "ramp_up".into(),
                triggered_by_event: false,
            },
            TransitionRecord {
                from_phase: "ramp_up".into(),
                to_phase: "steady".into(),
                triggered_by_event: false,
            },
            TransitionRecord {
                from_phase: "steady".into(),
                to_phase: "decline".into(),
                triggered_by_event: true,
            },
            TransitionRecord {
                from_phase: "decline".into(),
                to_phase: "dormant".into(),
                triggered_by_event: false,
            },
        ];
        let a = LifecycleAnalyzer::new();
        let r = a.analyze(&states, &transitions).unwrap();
        assert!(r.passes, "Issues: {:?}", r.issues);
    }

    #[test]
    fn test_all_stuck_in_new_flagged() {
        let states: Vec<_> = (0..100)
            .map(|_| LifecycleEndState {
                lifecycle_phase: "new".into(),
                days_since_opening: 300,
            })
            .collect();
        let a = LifecycleAnalyzer::new();
        let r = a.analyze(&states, &[]).unwrap();
        assert!(!r.passes);
        assert!(r
            .issues
            .iter()
            .any(|i| i.contains("stuck") || i.contains("Progression") || i.contains("phases")));
    }
}

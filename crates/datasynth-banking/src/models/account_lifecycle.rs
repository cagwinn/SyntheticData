//! Account lifecycle phase model.
//!
//! Tracks the maturity phase of a bank account, which modulates transaction
//! frequency and amount patterns. New accounts naturally have lower activity,
//! making sudden high-volume patterns more detectable.

use serde::{Deserialize, Serialize};

/// Lifecycle phase of a bank account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccountLifecyclePhase {
    /// First 30 days after opening. Very low activity, small amounts.
    New,
    /// Days 31-90. Activity ramps up linearly toward steady state.
    RampUp,
    /// Normal operating phase. Full persona-based transaction patterns.
    #[default]
    Steady,
    /// Declining activity (customer disengaging, pre-dormancy).
    Decline,
    /// No activity for 90+ days.
    Dormant,
}

impl AccountLifecyclePhase {
    /// Typical duration of each phase in days.
    pub fn typical_duration_days(&self) -> u32 {
        match self {
            Self::New => 30,
            Self::RampUp => 60,
            Self::Steady => 365, // open-ended, but 1 year as typical
            Self::Decline => 60,
            Self::Dormant => 0, // indefinite
        }
    }

    /// Transaction activity multiplier for this phase.
    ///
    /// Applied to the persona-based daily transaction count.
    /// - New: 0.2 (20% of normal)
    /// - RampUp: linearly interpolates from 0.3 to 1.0 over 60 days
    /// - Steady: 1.0
    /// - Decline: 0.3-0.5
    /// - Dormant: 0.0
    pub fn activity_multiplier(&self, days_in_phase: u32) -> f64 {
        match self {
            Self::New => 0.2,
            Self::RampUp => {
                let progress = (days_in_phase as f64 / 60.0).min(1.0);
                0.3 + 0.7 * progress
            }
            Self::Steady => 1.0,
            Self::Decline => {
                let progress = (days_in_phase as f64 / 60.0).min(1.0);
                0.5 - 0.2 * progress // 0.5 → 0.3
            }
            Self::Dormant => 0.0,
        }
    }

    /// Amount multiplier for this phase.
    ///
    /// New accounts tend to have smaller transactions (testing the account).
    pub fn amount_multiplier(&self, days_in_phase: u32) -> f64 {
        match self {
            Self::New => 0.3,
            Self::RampUp => {
                let progress = (days_in_phase as f64 / 60.0).min(1.0);
                0.4 + 0.6 * progress
            }
            Self::Steady => 1.0,
            Self::Decline => 0.6,
            Self::Dormant => 0.0,
        }
    }

    /// Determine the lifecycle phase from account age in days.
    pub fn from_account_age(age_days: u32, days_since_last_activity: u32) -> Self {
        if days_since_last_activity >= 90 {
            Self::Dormant
        } else if age_days <= 30 {
            Self::New
        } else if age_days <= 90 {
            Self::RampUp
        } else {
            Self::Steady
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_phase_low_activity() {
        assert!(AccountLifecyclePhase::New.activity_multiplier(0) < 0.3);
    }

    #[test]
    fn test_rampup_increases() {
        let early = AccountLifecyclePhase::RampUp.activity_multiplier(5);
        let late = AccountLifecyclePhase::RampUp.activity_multiplier(55);
        assert!(late > early);
        assert!(late > 0.9);
    }

    #[test]
    fn test_steady_is_full() {
        assert!(
            (AccountLifecyclePhase::Steady.activity_multiplier(100) - 1.0).abs() < f64::EPSILON
        );
    }

    #[test]
    fn test_dormant_is_zero() {
        assert!((AccountLifecyclePhase::Dormant.activity_multiplier(0)).abs() < f64::EPSILON);
    }

    #[test]
    fn test_from_account_age() {
        assert_eq!(
            AccountLifecyclePhase::from_account_age(5, 0),
            AccountLifecyclePhase::New
        );
        assert_eq!(
            AccountLifecyclePhase::from_account_age(50, 0),
            AccountLifecyclePhase::RampUp
        );
        assert_eq!(
            AccountLifecyclePhase::from_account_age(200, 0),
            AccountLifecyclePhase::Steady
        );
        assert_eq!(
            AccountLifecyclePhase::from_account_age(200, 100),
            AccountLifecyclePhase::Dormant
        );
    }
}

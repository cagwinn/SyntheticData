//! Stochastic account lifecycle engine.
//!
//! Replaces the purely deterministic phase progression with a transition
//! probability matrix + life event triggers. Accounts can:
//! - Linger longer in one phase (some stay New → RampUp for 120+ days)
//! - Skip phases (a Steady account abandoned → Dormant directly)
//! - Reactivate from Dormant → Steady
//! - Regress on life events (relocation, job loss → Decline even from Steady)

use chrono::{Duration, NaiveDate};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

use crate::models::{AccountLifecyclePhase, BankAccount};

/// Seed offset for the stochastic lifecycle engine.
pub const STOCHASTIC_LIFECYCLE_SEED_OFFSET: u64 = 8500;

/// Types of life events that trigger phase transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifeEvent {
    /// Job change — temporary decline, recovery likely
    JobChange,
    /// Relocation — temporary activity change
    Relocation,
    /// Major purchase (home, car) — spike then decline
    MajorPurchase,
    /// Retirement — shift to lower activity
    Retirement,
    /// Account abandonment (account holder disengagement)
    Abandonment,
    /// Reactivation from dormancy (return after long absence)
    Reactivation,
}

impl LifeEvent {
    /// Relative frequency of this event per account per year.
    pub fn annual_probability(&self) -> f64 {
        match self {
            Self::JobChange => 0.08,
            Self::Relocation => 0.05,
            Self::MajorPurchase => 0.10,
            Self::Retirement => 0.02,
            Self::Abandonment => 0.03,
            Self::Reactivation => 0.20, // of dormant accounts
        }
    }

    /// Which phase this event pushes the account toward.
    pub fn target_phase(&self) -> AccountLifecyclePhase {
        match self {
            Self::JobChange | Self::Relocation => AccountLifecyclePhase::Decline,
            Self::MajorPurchase => AccountLifecyclePhase::Steady, // spike absorbed
            Self::Retirement => AccountLifecyclePhase::Decline,
            Self::Abandonment => AccountLifecyclePhase::Dormant,
            Self::Reactivation => AccountLifecyclePhase::RampUp,
        }
    }
}

/// Record of a lifecycle transition that occurred.
#[derive(Debug, Clone)]
pub struct LifecycleTransition {
    pub account_id: uuid::Uuid,
    pub date: NaiveDate,
    pub from_phase: AccountLifecyclePhase,
    pub to_phase: AccountLifecyclePhase,
    pub triggered_by: Option<LifeEvent>,
}

/// Stochastic lifecycle simulator.
pub struct StochasticLifecycleEngine {
    rng: ChaCha8Rng,
}

impl StochasticLifecycleEngine {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(STOCHASTIC_LIFECYCLE_SEED_OFFSET)),
        }
    }

    /// Simulate lifecycle transitions for accounts over a time window.
    ///
    /// Updates each account's `lifecycle_phase` and `phase_start_date` to reflect
    /// the most recent phase as of `reference_date`. Returns the full transition
    /// history for analytics/eval.
    pub fn simulate(
        &mut self,
        accounts: &mut [BankAccount],
        reference_date: NaiveDate,
    ) -> Vec<LifecycleTransition> {
        let mut transitions = Vec::new();
        for account in accounts.iter_mut() {
            let history = self.simulate_account(account, reference_date);
            transitions.extend(history);
        }
        transitions
    }

    /// Simulate a single account's lifecycle journey from opening to reference date.
    fn simulate_account(
        &mut self,
        account: &mut BankAccount,
        reference_date: NaiveDate,
    ) -> Vec<LifecycleTransition> {
        let mut transitions = Vec::new();
        let mut current_phase = AccountLifecyclePhase::New;
        let mut phase_start = account.opening_date;
        let mut current_date = account.opening_date;

        while current_date < reference_date {
            // Check if any life event triggers today
            let event = self.sample_event(current_phase);

            // Also check natural phase progression (timer-based)
            let natural_transition = self.natural_progression(
                current_phase,
                (current_date - phase_start).num_days() as u32,
            );

            if let Some(ev) = event {
                let next_phase = ev.target_phase();
                if next_phase != current_phase {
                    transitions.push(LifecycleTransition {
                        account_id: account.account_id,
                        date: current_date,
                        from_phase: current_phase,
                        to_phase: next_phase,
                        triggered_by: Some(ev),
                    });
                    current_phase = next_phase;
                    phase_start = current_date;
                }
            } else if let Some(next) = natural_transition {
                transitions.push(LifecycleTransition {
                    account_id: account.account_id,
                    date: current_date,
                    from_phase: current_phase,
                    to_phase: next,
                    triggered_by: None,
                });
                current_phase = next;
                phase_start = current_date;
            }

            // Advance by a variable time step (1-7 days)
            let step = self.rng.random_range(1..=7);
            current_date += Duration::days(step as i64);
        }

        account.lifecycle_phase = current_phase;
        account.phase_start_date = Some(phase_start);
        transitions
    }

    /// Sample a life event occurring on the current day.
    /// Daily probability = annual / 365.
    fn sample_event(&mut self, current_phase: AccountLifecyclePhase) -> Option<LifeEvent> {
        // Reactivation only applies to Dormant
        if current_phase == AccountLifecyclePhase::Dormant {
            if self.rng.random::<f64>() < LifeEvent::Reactivation.annual_probability() / 365.0 {
                return Some(LifeEvent::Reactivation);
            }
            return None;
        }

        let events = [
            LifeEvent::JobChange,
            LifeEvent::Relocation,
            LifeEvent::MajorPurchase,
            LifeEvent::Retirement,
            LifeEvent::Abandonment,
        ];
        for ev in &events {
            if self.rng.random::<f64>() < ev.annual_probability() / 365.0 {
                return Some(*ev);
            }
        }
        None
    }

    /// Natural (non-event) phase progression based on time in current phase.
    fn natural_progression(
        &mut self,
        current: AccountLifecyclePhase,
        days_in_phase: u32,
    ) -> Option<AccountLifecyclePhase> {
        let typical = current.typical_duration_days();
        if typical == 0 {
            // Open-ended (Steady, Dormant) — no timer-based progression
            return None;
        }
        // Soft transition: probability grows as we exceed typical duration
        let overrun_ratio = days_in_phase as f64 / typical as f64;
        if overrun_ratio < 0.5 {
            return None;
        }
        // Sigmoid-like probability: ~0 at 0.5x, ~0.5 at 1x, ~0.95 at 2x
        let p = (overrun_ratio - 0.5) * 0.3;
        if self.rng.random::<f64>() < p {
            Some(match current {
                AccountLifecyclePhase::New => AccountLifecyclePhase::RampUp,
                AccountLifecyclePhase::RampUp => AccountLifecyclePhase::Steady,
                AccountLifecyclePhase::Decline => AccountLifecyclePhase::Dormant,
                _ => return None,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_account(opening: NaiveDate) -> BankAccount {
        BankAccount::new(
            Uuid::new_v4(),
            "ACC".into(),
            datasynth_core::models::banking::BankAccountType::Checking,
            Uuid::new_v4(),
            "USD",
            opening,
        )
    }

    #[test]
    fn test_simulate_produces_transitions() {
        let mut engine = StochasticLifecycleEngine::new(42);
        let mut accounts: Vec<_> = (0..50)
            .map(|_| make_account(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()))
            .collect();
        let transitions = engine.simulate(
            &mut accounts,
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
        );
        assert!(!transitions.is_empty(), "Should produce some transitions");

        // Most accounts should progress from New (at opening) to some later phase
        let moved = accounts
            .iter()
            .filter(|a| a.lifecycle_phase != AccountLifecyclePhase::New)
            .count();
        assert!(moved > accounts.len() / 2, "Most accounts should progress beyond New");
    }

    #[test]
    fn test_distribution_across_phases() {
        let mut engine = StochasticLifecycleEngine::new(42);
        let mut accounts: Vec<_> = (0..500)
            .map(|_| make_account(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()))
            .collect();
        engine.simulate(
            &mut accounts,
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
        );

        let mut counts = std::collections::HashMap::new();
        for a in &accounts {
            *counts.entry(a.lifecycle_phase).or_insert(0u32) += 1;
        }
        // Should have at least 2 distinct phases at end of year
        assert!(counts.len() >= 2, "Expect multiple phase endpoints: {counts:?}");
    }

    #[test]
    fn test_life_event_transitions_recorded() {
        let mut engine = StochasticLifecycleEngine::new(42);
        let mut accounts: Vec<_> = (0..200)
            .map(|_| make_account(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()))
            .collect();
        let transitions = engine.simulate(
            &mut accounts,
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
        );

        // At least some transitions should be event-triggered
        let event_driven = transitions
            .iter()
            .filter(|t| t.triggered_by.is_some())
            .count();
        assert!(event_driven > 0, "Should have some event-triggered transitions");
    }
}

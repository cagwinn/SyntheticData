//! Account lifecycle phase engine.
//!
//! Assigns and updates lifecycle phases on accounts based on their age
//! and activity patterns. The lifecycle multiplier is used by the
//! transaction generator to modulate daily transaction counts.

use chrono::NaiveDate;

use crate::models::{AccountLifecyclePhase, BankAccount};

/// Assigns lifecycle phases to accounts based on age relative to a reference date.
pub fn assign_lifecycle_phases(accounts: &mut [BankAccount], reference_date: NaiveDate) {
    for account in accounts.iter_mut() {
        let age_days = (reference_date - account.opening_date).num_days().max(0) as u32;
        let days_dormant = account.days_dormant;
        let phase = AccountLifecyclePhase::from_account_age(age_days, days_dormant);
        account.lifecycle_phase = phase;
    }
}

/// Compute the activity multiplier for an account on a given date.
///
/// This is called by the transaction generator on each day to scale
/// the persona-based transaction count.
pub fn activity_multiplier_for_date(account: &BankAccount, current_date: NaiveDate) -> f64 {
    let phase_start = account.phase_start_date.unwrap_or(account.opening_date);
    let days_in_phase = (current_date - phase_start).num_days().max(0) as u32;
    account.lifecycle_phase.activity_multiplier(days_in_phase)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_account(opening_date: NaiveDate) -> BankAccount {
        BankAccount::new(
            Uuid::new_v4(),
            "1234567890".to_string(),
            datasynth_core::models::banking::BankAccountType::Checking,
            Uuid::new_v4(),
            "USD",
            opening_date,
        )
    }

    #[test]
    fn test_new_account_gets_new_phase() {
        let today = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
        let mut accounts = vec![make_account(NaiveDate::from_ymd_opt(2024, 1, 20).unwrap())];
        assign_lifecycle_phases(&mut accounts, today);
        assert_eq!(accounts[0].lifecycle_phase, AccountLifecyclePhase::New);
    }

    #[test]
    fn test_old_account_gets_steady_phase() {
        let today = NaiveDate::from_ymd_opt(2024, 6, 1).unwrap();
        let mut accounts = vec![make_account(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())];
        assign_lifecycle_phases(&mut accounts, today);
        assert_eq!(accounts[0].lifecycle_phase, AccountLifecyclePhase::Steady);
    }

    #[test]
    fn test_activity_multiplier_new_is_low() {
        let account = make_account(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
        let mult =
            activity_multiplier_for_date(&account, NaiveDate::from_ymd_opt(2024, 1, 10).unwrap());
        assert!(
            mult < 0.3,
            "New account should have low multiplier, got {mult}"
        );
    }
}

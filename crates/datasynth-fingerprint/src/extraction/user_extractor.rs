//! SP4.5 — User-persona prior extraction from corpus GL data.
//!
//! # Corpus note
//!
//! All 45 JE parquet files in the project's reference dataset were examined.
//! None carries a user column (Created-By, Posted-By, Entered-By, etc.).
//! Consequently, `extract_user_personas` returns an **empty** `UserPersonaPrior`
//! with zero users.  The downstream generator gates on
//! `UserPersonaPrior::has_data()` and falls back to the internal user pool
//! when the prior is empty — so the stub causes no regression.
//!
//! When a future data delivery includes a per-user column, implement the body
//! of `extract_user_personas` here.  The required interface is:
//!
//! - Group records by user_id.
//! - For each user with ≥ `min_user_records` rows:
//!   - Compute `source_mix` (source-code count → normalised probabilities).
//!   - Compute `hourly_density` from the `created_at` field (hour-of-day
//!     distribution across 24 buckets).  Skip when `created_at` is `None`.
//!   - Compute `weekday_density` from `entry_date` (Mon=0 … Sun=6, 7 buckets).
//!   - Compute `volume_share` = this user's row count / total row count.
//! - Populate `user_count_distribution` from the total distinct qualifying user count.
//!
//! No other files need to change: `behavioral_extractor.rs` already calls this
//! function and passes its result to `extract_behavioral_priors`.

use datasynth_core::distributions::behavioral_priors::UserPersonaPrior;
use datasynth_eval::behavioral_fidelity::Record;

/// SP4.5 — Extract per-user behavioral patterns from `records`.
///
/// **Currently stubbed**: returns an empty `UserPersonaPrior` because the
/// corpus GL files do not carry a user column.  See the module-level
/// documentation for the full extraction algorithm to implement when user
/// data becomes available.
///
/// # Arguments
/// * `records` — normalised GL records (may or may not carry `created_at`).
/// * `min_user_records` — minimum row count for a user to be included
///   (privacy threshold; typically 100).
pub fn extract_user_personas(records: &[Record], min_user_records: usize) -> UserPersonaPrior {
    // Suppress unused-variable warnings while the body is stubbed.
    let _ = (records, min_user_records);

    // DEFERRED: corpus GL files carry no user column.  Return empty prior
    // so `LoadedPriors::user_personas` is `Some(empty)` rather than `None`,
    // making the wiring testable without real data.
    UserPersonaPrior::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use datasynth_eval::behavioral_fidelity::Record;

    fn rec(source: &str) -> Record {
        let d = NaiveDate::from_ymd_opt(2022, 1, 1).expect("date");
        Record {
            source: source.to_string(),
            gl_account: "1000".to_string(),
            cost_center: None,
            profit_center: None,
            trading_partner: None,
            je_number: "J1".to_string(),
            je_line_number: "001".to_string(),
            effective_date: d,
            entry_date: d,
            created_at: None,
            functional_amount: 100.0,
            header_text: String::new(),
            line_text: String::new(),
        }
    }

    #[test]
    fn stub_returns_empty_prior_on_any_input() {
        // Even with many records, the stub returns empty.
        let recs: Vec<Record> = (0..500).map(|_| rec("KR")).collect();
        let prior = extract_user_personas(&recs, 100);
        assert!(prior.users.is_empty(), "stub must return empty users map");
        assert!(!prior.has_data(), "has_data() must be false on empty prior");
    }

    #[test]
    fn stub_returns_empty_prior_on_empty_input() {
        let prior = extract_user_personas(&[], 100);
        assert!(prior.users.is_empty());
    }

    #[test]
    fn has_data_false_on_default() {
        let prior = UserPersonaPrior::default();
        assert!(!prior.has_data());
    }

    #[test]
    fn sample_user_for_source_returns_none_on_empty_prior() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;
        let prior = UserPersonaPrior::default();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        assert!(prior.sample_user_for_source("KR", &mut rng).is_none());
    }

    #[test]
    fn sample_timestamp_for_user_returns_none_on_unknown_user() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;
        let prior = UserPersonaPrior::default();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        assert!(prior
            .sample_timestamp_for_user("USER0001", &mut rng)
            .is_none());
    }

    /// Verify that a manually constructed UserBehavior with real data
    /// is correctly sampled by `sample_user_for_source`.
    #[test]
    fn sample_user_for_source_with_real_data() {
        use datasynth_core::distributions::behavioral_priors::UserBehavior;
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;
        use std::collections::BTreeMap;

        let mut users = BTreeMap::new();

        // AP clerk: heavily KR-weighted
        let mut ap_mix = BTreeMap::new();
        ap_mix.insert("KR".to_string(), 0.7);
        ap_mix.insert("KZ".to_string(), 0.3);
        users.insert(
            "USER0010".to_string(),
            UserBehavior {
                source_mix: ap_mix,
                hourly_density: {
                    let mut h = [0.0; 24];
                    h[9] = 0.5; // morning peak
                    h[14] = 0.5; // afternoon
                    h
                },
                weekday_density: {
                    let mut w = [0.0; 7];
                    w[0] = 0.25; // Mon
                    w[1] = 0.25; // Tue
                    w[2] = 0.25; // Wed
                    w[3] = 0.25; // Thu
                    w
                },
                volume_share: 0.6,
            },
        );

        // AR clerk: only RV
        let mut ar_mix = BTreeMap::new();
        ar_mix.insert("RV".to_string(), 1.0);
        users.insert(
            "USER0020".to_string(),
            UserBehavior {
                source_mix: ar_mix,
                hourly_density: {
                    let mut h = [0.0; 24];
                    h[10] = 1.0;
                    h
                },
                weekday_density: {
                    let mut w = [0.0; 7];
                    w[4] = 1.0; // Fri only
                    w
                },
                volume_share: 0.4,
            },
        );

        let prior = UserPersonaPrior {
            users,
            user_count_distribution: Default::default(),
        };

        assert!(prior.has_data());

        let mut rng = ChaCha8Rng::seed_from_u64(99);

        // Sampling for "KR" should always return USER0010 (only one user has KR).
        for _ in 0..20 {
            let uid = prior
                .sample_user_for_source("KR", &mut rng)
                .expect("must return a user");
            assert_eq!(uid, "USER0010", "KR must map to the AP clerk");
        }

        // Sampling for "RV" should always return USER0020.
        for _ in 0..20 {
            let uid = prior
                .sample_user_for_source("RV", &mut rng)
                .expect("must return a user");
            assert_eq!(uid, "USER0020", "RV must map to the AR clerk");
        }

        // Sampling for an unknown source returns None.
        assert!(prior.sample_user_for_source("XX", &mut rng).is_none());

        // Timestamp sampling for USER0010 returns hour in {9, 14} and weekday in {0..=3}.
        let (hour, weekday) = prior
            .sample_timestamp_for_user("USER0010", &mut rng)
            .expect("must return timestamp");
        assert!(hour == 9 || hour == 14, "expected hour 9 or 14, got {hour}");
        assert!(weekday <= 3, "expected weekday 0..=3, got {weekday}");

        // Timestamp sampling for USER0020 returns hour=10 and weekday=4.
        let (hour, weekday) = prior
            .sample_timestamp_for_user("USER0020", &mut rng)
            .expect("must return timestamp");
        assert_eq!(hour, 10);
        assert_eq!(weekday, 4);
    }
}

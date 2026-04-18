//! Behavioral-bias injection for fraud-labeled journal entries.
//!
//! Without bias, fraud entries inherit the same temporal and amount
//! distributions as legitimate data, so a classifier sees ~0 lift on the
//! canonical forensic signals (weekend posting, round-dollar amounts,
//! off-hours timestamps, post-close adjustments). This module applies
//! those biases in a single place so every fraud-marking path gets them.
//!
//! The function is intentionally idempotent-ish: weekend and off-hours
//! always fire probabilistically (they just re-shift an already-shifted
//! timestamp), post-close guards against re-setting, and round-dollar is
//! skipped on entries whose amounts are already exact round targets.

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc, Weekday};
use rand::{Rng, RngExt};
use rust_decimal::Decimal;

use crate::models::{JournalEntry, ProcessIssueType};

/// Probabilities for the four canonical fraud behavioral biases.
///
/// Defaults yield ≥2× lift on each signal when applied to every fraud
/// entry, assuming the non-fraud baseline follows the generator's normal
/// temporal / amount distributions.
#[derive(Debug, Clone, Copy)]
pub struct FraudBehavioralBiasConfig {
    /// Master switch — when `false`, no bias is applied.
    pub enabled: bool,
    /// Probability that a fraud entry's posting date is shifted to a
    /// weekend. Baseline ≈ 0 % (business-day-only posting), 0.30 yields
    /// ~30 % absolute fraud rate.
    pub weekend_bias: f64,
    /// Probability that a fraud entry's amount is snapped to a
    /// "suspicious" round-dollar value ($1 K, $5 K, $10 K, $25 K, $50 K,
    /// $100 K). Multi-line entries are rescaled proportionally so the
    /// entry remains balanced.
    pub round_dollar_bias: f64,
    /// Probability that a fraud entry's `created_at` timestamp is shifted
    /// to off-hours (22:00–05:59 UTC).
    pub off_hours_bias: f64,
    /// Probability that a fraud entry is marked `is_post_close = true`.
    pub post_close_bias: f64,
}

impl Default for FraudBehavioralBiasConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            weekend_bias: 0.30,
            round_dollar_bias: 0.40,
            off_hours_bias: 0.35,
            post_close_bias: 0.25,
        }
    }
}

impl FraudBehavioralBiasConfig {
    /// Construct a disabled config (all biases skipped). Useful for
    /// fraud datasets where raw distributions must be preserved.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            weekend_bias: 0.0,
            round_dollar_bias: 0.0,
            off_hours_bias: 0.0,
            post_close_bias: 0.0,
        }
    }
}

/// Apply behavioral biases to a fraud-labeled entry. Callers are
/// responsible for ensuring `entry.header.is_fraud == true` before
/// calling — the function does not check, so biases can also be applied
/// to not-yet-labeled entries if desired.
///
/// Returns the list of [`ProcessIssueType`] variants corresponding to
/// biases that fired. Callers emit these as secondary labels so auditors
/// can filter for specific forensic patterns.
///
/// Round targets include every $1 K increment we consider materially
/// "suspicious"; the closest to the entry's current amount is chosen so
/// the rescaling is small enough to not distort aggregates by orders of
/// magnitude. Multi-line entries are rescaled by a common factor so
/// debits still equal credits after rounding.
pub fn apply_fraud_behavioral_bias<R: Rng>(
    entry: &mut JournalEntry,
    cfg: &FraudBehavioralBiasConfig,
    rng: &mut R,
) -> Vec<ProcessIssueType> {
    let mut fired: Vec<ProcessIssueType> = Vec::new();

    if !cfg.enabled {
        return fired;
    }

    // --- Weekend bias ---
    if cfg.weekend_bias > 0.0 && rng.random::<f64>() < cfg.weekend_bias {
        let original = entry.header.posting_date;
        let days_to_weekend = match original.weekday() {
            Weekday::Mon => 5,
            Weekday::Tue => 4,
            Weekday::Wed => 3,
            Weekday::Thu => 2,
            Weekday::Fri => 1,
            Weekday::Sat | Weekday::Sun => 0,
        };
        let extra = if rng.random_bool(0.5) { 0 } else { 1 };
        entry.header.posting_date = original + Duration::days(days_to_weekend + extra);
        fired.push(ProcessIssueType::WeekendPosting);
    }

    // --- Round-dollar bias ---
    //
    // The full entry is rescaled by a single factor so balances are
    // preserved across arbitrary numbers of debit/credit lines. We
    // snap the entry's max absolute amount to the nearest round target
    // and apply the implied ratio to every other line.
    if cfg.round_dollar_bias > 0.0 && rng.random::<f64>() < cfg.round_dollar_bias {
        const ROUND_TARGETS: &[i64] = &[1_000, 5_000, 10_000, 25_000, 50_000, 100_000];
        // Find the largest magnitude on any line.
        let max_abs: Decimal = entry
            .lines
            .iter()
            .map(|l| l.debit_amount.max(l.credit_amount))
            .max()
            .unwrap_or(Decimal::ZERO);
        if max_abs > Decimal::ZERO {
            let max_f64: f64 = max_abs.try_into().unwrap_or(0.0);
            let target = ROUND_TARGETS
                .iter()
                .min_by(|a, b| {
                    let da = (**a as f64 - max_f64).abs();
                    let db = (**b as f64 - max_f64).abs();
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied()
                .unwrap_or(1_000);
            let target_dec = Decimal::from(target);
            // Ratio to scale every line by.
            if let Some(ratio) = target_dec.checked_div(max_abs) {
                for line in entry.lines.iter_mut() {
                    if line.debit_amount > Decimal::ZERO {
                        line.debit_amount = (line.debit_amount * ratio).round_dp(2);
                    }
                    if line.credit_amount > Decimal::ZERO {
                        line.credit_amount = (line.credit_amount * ratio).round_dp(2);
                    }
                    // Local amount mirrors the scaling; if generators set
                    // local_amount from debit - credit they stay consistent.
                    line.local_amount = (line.local_amount * ratio).round_dp(2);
                }
                // Correct residual imbalance from rounding: bump the first
                // non-zero credit line to absorb any pennies.
                let debit_total: Decimal = entry.lines.iter().map(|l| l.debit_amount).sum();
                let credit_total: Decimal = entry.lines.iter().map(|l| l.credit_amount).sum();
                let diff = debit_total - credit_total;
                if diff != Decimal::ZERO {
                    if diff > Decimal::ZERO {
                        // Debits exceed credits — add to the first credit line.
                        if let Some(line) = entry
                            .lines
                            .iter_mut()
                            .find(|l| l.credit_amount > Decimal::ZERO)
                        {
                            line.credit_amount += diff;
                        }
                    } else {
                        // Credits exceed debits — add to the first debit line.
                        if let Some(line) = entry
                            .lines
                            .iter_mut()
                            .find(|l| l.debit_amount > Decimal::ZERO)
                        {
                            line.debit_amount += -diff;
                        }
                    }
                }
            }
        }
    }

    // --- Off-hours bias (22:00–05:59 UTC) ---
    if cfg.off_hours_bias > 0.0 && rng.random::<f64>() < cfg.off_hours_bias {
        let hour: u32 = if rng.random_bool(0.5) {
            rng.random_range(22..24)
        } else {
            rng.random_range(0..6)
        };
        let minute: u32 = rng.random_range(0..60);
        let second: u32 = rng.random_range(0..60);
        if let chrono::LocalResult::Single(new_ts) = Utc.with_ymd_and_hms(
            entry.header.posting_date.year(),
            entry.header.posting_date.month(),
            entry.header.posting_date.day(),
            hour,
            minute,
            second,
        ) {
            entry.header.created_at = new_ts;
            fired.push(ProcessIssueType::AfterHoursPosting);
        }
    }

    // --- Post-close bias ---
    if cfg.post_close_bias > 0.0
        && rng.random::<f64>() < cfg.post_close_bias
        && !entry.header.is_post_close
    {
        entry.header.is_post_close = true;
        fired.push(ProcessIssueType::PostClosePosting);
    }

    fired
}

/// Apply biases to every fraud-labeled entry in a slice. Entries where
/// `is_fraud == false` are skipped.
///
/// Returns the number of entries touched (bias attempted — at least one
/// bias may not have fired due to its own probability check).
pub fn apply_biases_to_fraud_entries<R: Rng>(
    entries: &mut [JournalEntry],
    cfg: &FraudBehavioralBiasConfig,
    rng: &mut R,
) -> usize {
    if !cfg.enabled {
        return 0;
    }
    let mut touched = 0usize;
    for entry in entries.iter_mut() {
        if entry.header.is_fraud {
            apply_fraud_behavioral_bias(entry, cfg, rng);
            touched += 1;
        }
    }
    touched
}

/// Utility: clamp a timestamp's hour to off-hours regardless of date.
/// Not called by the bias function itself — exposed for callers that
/// want to force off-hours on a specific record.
pub fn clamp_to_off_hours<R: Rng>(ts: DateTime<Utc>, rng: &mut R) -> DateTime<Utc> {
    let hour: u32 = if rng.random_bool(0.5) {
        rng.random_range(22..24)
    } else {
        rng.random_range(0..6)
    };
    let minute: u32 = rng.random_range(0..60);
    let second: u32 = rng.random_range(0..60);
    match Utc.with_ymd_and_hms(ts.year(), ts.month(), ts.day(), hour, minute, second) {
        chrono::LocalResult::Single(new_ts) => new_ts,
        _ => ts,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::models::{FraudType, JournalEntryLine};
    use chrono::{NaiveDate, Timelike};
    use rand_chacha::rand_core::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn make_fraud_entry(posting_date: NaiveDate) -> JournalEntry {
        let mut je =
            JournalEntry::new_simple("DOC1".into(), "C001".into(), posting_date, "tester".into());
        je.header.is_fraud = true;
        je.header.fraud_type = Some(FraudType::FictitiousEntry);
        // Give it a known amount for round-dollar testing
        je.add_line(JournalEntryLine::debit(
            je.header.document_id,
            1,
            "1000".into(),
            Decimal::new(98765, 2), // 987.65
        ));
        je.add_line(JournalEntryLine::credit(
            je.header.document_id,
            2,
            "2000".into(),
            Decimal::new(98765, 2), // 987.65
        ));
        je
    }

    #[test]
    fn applies_all_biases_at_rate_one() {
        let cfg = FraudBehavioralBiasConfig {
            enabled: true,
            weekend_bias: 1.0,
            round_dollar_bias: 1.0,
            off_hours_bias: 1.0,
            post_close_bias: 1.0,
        };
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let mut je = make_fraud_entry(NaiveDate::from_ymd_opt(2024, 6, 12).unwrap()); // Wednesday

        let fired = apply_fraud_behavioral_bias(&mut je, &cfg, &mut rng);

        assert_eq!(fired.len(), 3, "three process-issue labels expected");
        assert!(
            matches!(
                je.header.posting_date.weekday(),
                Weekday::Sat | Weekday::Sun
            ),
            "posting_date should shift to a weekend"
        );
        let hour = je.header.created_at.hour();
        assert!(
            !(6..22).contains(&hour),
            "created_at should be off-hours, got hour={hour}"
        );
        assert!(je.header.is_post_close, "is_post_close should be true");
        // Round target — 1000 is closest to 987.65
        assert_eq!(je.lines[0].debit_amount, Decimal::from(1000));
        assert_eq!(je.lines[1].credit_amount, Decimal::from(1000));
    }

    #[test]
    fn rate_zero_applies_nothing() {
        let cfg = FraudBehavioralBiasConfig {
            enabled: true,
            weekend_bias: 0.0,
            round_dollar_bias: 0.0,
            off_hours_bias: 0.0,
            post_close_bias: 0.0,
        };
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let original = NaiveDate::from_ymd_opt(2024, 6, 12).unwrap();
        let mut je = make_fraud_entry(original);
        let created_at_before = je.header.created_at;
        let fired = apply_fraud_behavioral_bias(&mut je, &cfg, &mut rng);
        assert!(fired.is_empty());
        assert_eq!(je.header.posting_date, original);
        assert_eq!(je.header.created_at, created_at_before);
        assert!(!je.header.is_post_close);
    }

    #[test]
    fn multi_line_round_dollar_preserves_balance() {
        // Entry with 3 lines: DR, DR, CR (asymmetric)
        let mut je = JournalEntry::new_simple(
            "DOC".into(),
            "C001".into(),
            NaiveDate::from_ymd_opt(2024, 6, 12).unwrap(),
            "tester".into(),
        );
        je.header.is_fraud = true;
        je.add_line(JournalEntryLine::debit(
            je.header.document_id,
            1,
            "1000".into(),
            Decimal::new(60000, 2), // 600.00
        ));
        je.add_line(JournalEntryLine::debit(
            je.header.document_id,
            2,
            "1160".into(),
            Decimal::new(40000, 2), // 400.00
        ));
        je.add_line(JournalEntryLine::credit(
            je.header.document_id,
            3,
            "2000".into(),
            Decimal::new(100000, 2), // 1000.00
        ));
        let cfg = FraudBehavioralBiasConfig {
            enabled: true,
            weekend_bias: 0.0,
            round_dollar_bias: 1.0,
            off_hours_bias: 0.0,
            post_close_bias: 0.0,
        };
        let mut rng = ChaCha8Rng::seed_from_u64(3);
        apply_fraud_behavioral_bias(&mut je, &cfg, &mut rng);
        let debit: Decimal = je.lines.iter().map(|l| l.debit_amount).sum();
        let credit: Decimal = je.lines.iter().map(|l| l.credit_amount).sum();
        assert_eq!(debit, credit, "debits must equal credits after rounding");
    }

    #[test]
    fn skips_non_fraud_in_slice() {
        let cfg = FraudBehavioralBiasConfig::default();
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let fraud = make_fraud_entry(NaiveDate::from_ymd_opt(2024, 6, 12).unwrap());
        let mut clean = make_fraud_entry(NaiveDate::from_ymd_opt(2024, 6, 12).unwrap());
        clean.header.is_fraud = false;
        let mut slice = [fraud, clean];
        let touched = apply_biases_to_fraud_entries(&mut slice, &cfg, &mut rng);
        assert_eq!(touched, 1);
    }
}

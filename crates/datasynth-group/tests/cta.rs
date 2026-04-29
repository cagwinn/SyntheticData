//! Integration tests for the CTA computation + rollforward (Task 6.3).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_group::{
    compute_cta, cta_rollforward, write_cta_rollforward, CtaRollforward, DrCr, RateBasis,
    TranslatedLine, TranslatedTb, TranslationAccountType, CONSOLIDATED_SUBDIR,
    CTA_ROLLFORWARD_FILENAME,
};

// ── Fixture builders ─────────────────────────────────────────────────

fn period_end() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
}

fn make_translated(entity_code: &str, debits: Decimal, credits: Decimal) -> TranslatedTb {
    TranslatedTb {
        entity_code: entity_code.to_string(),
        functional_currency: "USD".to_string(),
        presentation_currency: "CHF".to_string(),
        as_of_date: period_end(),
        lines: vec![TranslatedLine {
            account_code: "1000".to_string(),
            local_amount: Decimal::ZERO,
            local_dr_cr: DrCr::Debit,
            fx_rate: dec!(1.10),
            rate_basis: RateBasis::Closing,
            translated_amount: debits,
            account_type: TranslationAccountType::BsMonetary,
        }],
        total_translated_debits: debits,
        total_translated_credits: credits,
        cta: debits - credits,
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[test]
fn compute_cta_returns_dr_minus_cr() {
    let t = make_translated("E1", dec!(15000), dec!(13500));
    assert_eq!(compute_cta(&t), dec!(1500));

    // Negative CTA — credits exceed debits.
    let t_neg = make_translated("E2", dec!(10000), dec!(12000));
    assert_eq!(compute_cta(&t_neg), dec!(-2000));

    // Zero CTA — perfectly balanced (e.g., identity translation).
    let t_zero = make_translated("E3", dec!(20000), dec!(20000));
    assert_eq!(compute_cta(&t_zero), Decimal::ZERO);
}

#[test]
fn rollforward_math_closing_equals_opening_plus_period() {
    // Standard rollforward: opening 500 + period 1500 = closing 2000.
    let r = cta_rollforward("NESTLE_USA", "USD", "CHF", dec!(500), dec!(1500));

    assert_eq!(r.entity_code, "NESTLE_USA");
    assert_eq!(r.functional_currency, "USD");
    assert_eq!(r.presentation_currency, "CHF");
    assert_eq!(r.opening_cta, dec!(500));
    assert_eq!(r.period_cta, dec!(1500));
    assert_eq!(r.closing_cta, dec!(2000));
    assert_eq!(r.closing_cta, r.opening_cta + r.period_cta);

    // Negative period CTA.
    let r_neg = cta_rollforward("NESTLE_DE", "EUR", "CHF", dec!(0), dec!(-300));
    assert_eq!(r_neg.closing_cta, dec!(-300));

    // Zero opening (first period).
    let r_first = cta_rollforward("NESTLE_BR", "BRL", "CHF", Decimal::ZERO, dec!(750));
    assert_eq!(r_first.closing_cta, dec!(750));
}

#[test]
fn write_cta_rollforward_creates_file_and_round_trips() {
    let tmp = tempfile::tempdir().expect("tmp dir");
    let out_dir = tmp.path();

    let rollforwards = vec![
        cta_rollforward("NESTLE_USA", "USD", "CHF", dec!(100), dec!(900)),
        cta_rollforward("NESTLE_DE", "EUR", "CHF", dec!(50), dec!(-150)),
    ];

    let path = write_cta_rollforward(&rollforwards, out_dir).expect("write must succeed");

    // Path is `{out_dir}/consolidated/cta_rollforward.json`.
    assert!(path.is_absolute() || path.starts_with(out_dir));
    assert!(path.ends_with(CTA_ROLLFORWARD_FILENAME));
    assert!(path.parent().unwrap().ends_with(CONSOLIDATED_SUBDIR));

    // File exists and round-trips through serde.
    let bytes = std::fs::read(&path).expect("read");
    let round_trip: Vec<CtaRollforward> = serde_json::from_slice(&bytes).expect("parse");
    assert_eq!(round_trip, rollforwards);

    // File content ends with a trailing newline (human-friendly).
    let s = String::from_utf8(bytes).expect("utf8");
    assert!(s.ends_with('\n'));
}

#[test]
fn empty_input_writes_empty_array() {
    let tmp = tempfile::tempdir().expect("tmp dir");
    let out_dir = tmp.path();

    let path = write_cta_rollforward(&[], out_dir).expect("write must succeed");

    let bytes = std::fs::read(&path).expect("read");
    let round_trip: Vec<CtaRollforward> = serde_json::from_slice(&bytes).expect("parse");
    assert!(round_trip.is_empty());

    // Whitespace-tolerant check that the file holds an empty JSON array.
    let s = String::from_utf8(bytes).expect("utf8");
    let trimmed = s.trim();
    assert_eq!(trimmed, "[]");
}

#[test]
fn determinism_two_calls_produce_byte_identical_files() {
    let tmp_a = tempfile::tempdir().expect("tmp a");
    let tmp_b = tempfile::tempdir().expect("tmp b");

    let rollforwards = vec![
        cta_rollforward("NESTLE_USA", "USD", "CHF", dec!(100), dec!(900)),
        cta_rollforward("NESTLE_DE", "EUR", "CHF", dec!(50), dec!(-150)),
        cta_rollforward("NESTLE_BR", "BRL", "CHF", Decimal::ZERO, dec!(220)),
    ];

    let p_a = write_cta_rollforward(&rollforwards, tmp_a.path()).expect("write a");
    let p_b = write_cta_rollforward(&rollforwards, tmp_b.path()).expect("write b");

    let bytes_a = std::fs::read(&p_a).expect("read a");
    let bytes_b = std::fs::read(&p_b).expect("read b");

    assert_eq!(bytes_a, bytes_b, "two calls must produce identical bytes");
}

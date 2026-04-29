//! Task 8.4 — statement of changes in equity integration tests.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_group::{build_statement_of_changes_in_equity, EquityChangesInputs};

fn period_start() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
}

fn period_end() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
}

#[test]
fn happy_path_math() {
    let inputs = EquityChangesInputs {
        opening_owners_equity: dec!(1_000_000),
        opening_nci: dec!(200_000),
        net_income_to_owners: dec!(150_000),
        net_income_to_nci: dec!(40_000),
        oci_to_owners: dec!(20_000),
        oci_to_nci: dec!(5_000),
        dividends_to_owners: dec!(50_000),
        dividends_to_nci: dec!(10_000),
        other_owners: dec!(0),
        other_nci: dec!(0),
    };

    let s =
        build_statement_of_changes_in_equity(&inputs, "TEST_GROUP", period_start(), period_end(), "CHF");

    // Owners: 1_000_000 + 150_000 + 20_000 - 50_000 + 0 = 1_120_000.
    assert_eq!(s.owners_equity.opening, dec!(1_000_000));
    assert_eq!(s.owners_equity.net_income, dec!(150_000));
    assert_eq!(s.owners_equity.oci, dec!(20_000));
    assert_eq!(s.owners_equity.dividends, dec!(50_000));
    assert_eq!(s.owners_equity.closing, dec!(1_120_000));

    // NCI: 200_000 + 40_000 + 5_000 - 10_000 + 0 = 235_000.
    assert_eq!(s.nci.opening, dec!(200_000));
    assert_eq!(s.nci.closing, dec!(235_000));

    assert_eq!(s.group_id, "TEST_GROUP");
    assert_eq!(s.currency, "CHF");
    assert_eq!(s.period_start, period_start());
    assert_eq!(s.period_end, period_end());
}

#[test]
fn total_is_per_line_sum_of_owners_and_nci() {
    let inputs = EquityChangesInputs {
        opening_owners_equity: dec!(1_000_000),
        opening_nci: dec!(200_000),
        net_income_to_owners: dec!(150_000),
        net_income_to_nci: dec!(40_000),
        oci_to_owners: dec!(20_000),
        oci_to_nci: dec!(5_000),
        dividends_to_owners: dec!(50_000),
        dividends_to_nci: dec!(10_000),
        other_owners: dec!(7_000),
        other_nci: dec!(3_000),
    };
    let s =
        build_statement_of_changes_in_equity(&inputs, "G", period_start(), period_end(), "CHF");

    // Per-line sum check.
    assert_eq!(
        s.total_equity.opening,
        s.owners_equity.opening + s.nci.opening
    );
    assert_eq!(
        s.total_equity.net_income,
        s.owners_equity.net_income + s.nci.net_income
    );
    assert_eq!(s.total_equity.oci, s.owners_equity.oci + s.nci.oci);
    assert_eq!(
        s.total_equity.dividends,
        s.owners_equity.dividends + s.nci.dividends
    );
    assert_eq!(s.total_equity.other, s.owners_equity.other + s.nci.other);
    assert_eq!(
        s.total_equity.closing,
        s.owners_equity.closing + s.nci.closing
    );
}

#[test]
fn empty_inputs_all_zeros() {
    let inputs = EquityChangesInputs {
        opening_owners_equity: Decimal::ZERO,
        opening_nci: Decimal::ZERO,
        net_income_to_owners: Decimal::ZERO,
        net_income_to_nci: Decimal::ZERO,
        oci_to_owners: Decimal::ZERO,
        oci_to_nci: Decimal::ZERO,
        dividends_to_owners: Decimal::ZERO,
        dividends_to_nci: Decimal::ZERO,
        other_owners: Decimal::ZERO,
        other_nci: Decimal::ZERO,
    };
    let s = build_statement_of_changes_in_equity(
        &inputs,
        "EMPTY",
        period_start(),
        period_end(),
        "EUR",
    );

    for col in [&s.owners_equity, &s.nci, &s.total_equity] {
        assert_eq!(col.opening, Decimal::ZERO);
        assert_eq!(col.net_income, Decimal::ZERO);
        assert_eq!(col.oci, Decimal::ZERO);
        assert_eq!(col.dividends, Decimal::ZERO);
        assert_eq!(col.other, Decimal::ZERO);
        assert_eq!(col.closing, Decimal::ZERO);
    }
}

use chrono::NaiveDate;
use datasynth_core::accounts::{cash_accounts, dividend_accounts, equity_accounts};
use datasynth_generators::period_close::DividendGenerator;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

#[test]
fn test_dividend_produces_two_jes() {
    let mut gen = DividendGenerator::new(42);
    let result = gen.generate(
        "C001",
        NaiveDate::from_ymd_opt(2025, 3, 31).unwrap(),
        dec!(2.50),
        dec!(100_000),
        "USD",
    );
    assert_eq!(result.declarations.len(), 1);
    assert_eq!(result.journal_entries.len(), 2);
    for je in &result.journal_entries {
        assert!(je.is_balanced());
    }
}

#[test]
fn test_declaration_je_accounts() {
    let mut gen = DividendGenerator::new(42);
    let result = gen.generate(
        "C001",
        NaiveDate::from_ymd_opt(2025, 6, 15).unwrap(),
        dec!(1.00),
        dec!(50_000),
        "USD",
    );
    let decl_je = &result.journal_entries[0];
    assert!(decl_je.lines.iter().any(|l| l.gl_account == equity_accounts::RETAINED_EARNINGS
        && l.debit_amount > Decimal::ZERO));
    assert!(decl_je.lines.iter().any(|l| l.gl_account == dividend_accounts::DIVIDENDS_PAYABLE
        && l.credit_amount > Decimal::ZERO));
}

#[test]
fn test_payment_je_accounts() {
    let mut gen = DividendGenerator::new(42);
    let result = gen.generate(
        "C001",
        NaiveDate::from_ymd_opt(2025, 6, 15).unwrap(),
        dec!(1.00),
        dec!(50_000),
        "USD",
    );
    let pay_je = &result.journal_entries[1];
    assert!(pay_je.lines.iter().any(|l| l.gl_account == dividend_accounts::DIVIDENDS_PAYABLE
        && l.debit_amount > Decimal::ZERO));
    assert!(pay_je.lines.iter().any(|l| l.gl_account == cash_accounts::OPERATING_CASH
        && l.credit_amount > Decimal::ZERO));
}

#[test]
fn test_dividend_dates_ordering() {
    let mut gen = DividendGenerator::new(42);
    let result = gen.generate(
        "C001",
        NaiveDate::from_ymd_opt(2025, 3, 31).unwrap(),
        dec!(2.50),
        dec!(100_000),
        "USD",
    );
    let d = &result.declarations[0];
    assert!(d.declaration_date <= d.record_date);
    assert!(d.record_date <= d.payment_date);
}

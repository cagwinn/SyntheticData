//! C2 (#157) Piece 1 — project a closing trial balance onto next-period
//! opening balances.
//!
//! Multi-period generation needs Year N+1's opening positions to come
//! from Year N's closing TB. Three carry rules apply:
//!
//! - **Balance-sheet accounts** (`Asset`, `ContraAsset`, `Liability`,
//!   `ContraLiability`, `Equity`, `ContraEquity`) carry their closing
//!   `debit_balance` / `credit_balance` straight into Year N+1's opening.
//! - **P&L accounts** (`Revenue`, `Expense`) **do not** carry — they
//!   are zeroed in Year N+1 (every period starts with zero earnings).
//!   Their net effect is absorbed into Retained Earnings instead.
//! - **Retained Earnings** absorbs the period's net income. Concretely,
//!   the closing RE balance plus `Σ(P&L credit - P&L debit)` becomes
//!   Year N+1's opening RE balance.
//!
//! This is exactly the year-end "close" entry every real general
//! ledger posts as the last move of each year. We synthesize it
//! here from the closing TB rather than emitting an explicit JE,
//! because the consumer (`ShardContext.opening_balances` in
//! `datasynth-runtime`) wants the projected positions, not the
//! posting that produced them.
//!
//! See `docs/design/2026-05-27-c2-multi-period-design.md` for the
//! broader C2 plan.

use datasynth_core::framework_accounts::FrameworkAccounts;
use datasynth_core::models::balance::{AccountType, EntityOpeningBalance, TrialBalance};
use rust_decimal::Decimal;

/// Project a closing trial balance onto next-period opening balances.
///
/// `framework` selects the Retained-Earnings account code (US GAAP
/// uses `"3200"`; SKR03/04 uses `"2970"`; IFRS uses different
/// numbering still). Pass the same framework string the orchestrator
/// uses for the prior period — mismatches mean net income lands in
/// the wrong account.
///
/// The output:
/// - Contains one row per BS account from the closing TB.
/// - Excludes all Revenue / Expense lines (they zero out).
/// - The Retained Earnings row carries closing RE plus the period's
///   net income (positive RE → credit, negative → debit; one-sided).
/// - If the closing TB has no Retained Earnings row but a non-zero
///   net income, inserts one with the framework's RE account code.
pub fn project_closing_to_opening(
    closing_tb: &TrialBalance,
    framework: &str,
) -> Vec<EntityOpeningBalance> {
    let fa = FrameworkAccounts::for_framework(framework);
    let re_account = fa.retained_earnings.clone();

    let mut net_income = Decimal::ZERO;
    let mut openings: Vec<EntityOpeningBalance> = Vec::with_capacity(closing_tb.lines.len());
    let mut re_idx: Option<usize> = None;

    for line in &closing_tb.lines {
        match line.account_type {
            // Balance-sheet → carry as-is.
            AccountType::Asset
            | AccountType::ContraAsset
            | AccountType::Liability
            | AccountType::ContraLiability
            | AccountType::Equity
            | AccountType::ContraEquity => {
                if line.account_code == re_account {
                    re_idx = Some(openings.len());
                }
                openings.push(EntityOpeningBalance {
                    account_code: line.account_code.clone(),
                    account_type: line.account_type,
                    debit: line.debit_balance,
                    credit: line.credit_balance,
                });
            }
            // P&L → accumulate signed contribution to net income.
            // For credit-normal Revenue: (credit - debit) is positive
            // when revenue exceeded reversals. For debit-normal
            // Expense: (credit - debit) is *negative* by the same
            // sign, so summing both yields net income directly.
            AccountType::Revenue | AccountType::Expense => {
                net_income += line.credit_balance - line.debit_balance;
            }
        }
    }

    // Absorb net income into Retained Earnings.
    if !net_income.is_zero() {
        match re_idx {
            Some(i) => {
                // Combine prior credit-side balance with net income, then
                // normalise to one-sided so the EntityOpeningBalance row
                // stays well-formed (we never want debit > 0 AND credit > 0
                // on the same opening row).
                let net = openings[i].credit - openings[i].debit + net_income;
                if net >= Decimal::ZERO {
                    openings[i].credit = net;
                    openings[i].debit = Decimal::ZERO;
                } else {
                    openings[i].debit = -net;
                    openings[i].credit = Decimal::ZERO;
                }
            }
            None => {
                // No RE row in the closing TB but the period generated
                // earnings — synthesize the row.
                let (debit, credit) = if net_income >= Decimal::ZERO {
                    (Decimal::ZERO, net_income)
                } else {
                    (-net_income, Decimal::ZERO)
                };
                openings.push(EntityOpeningBalance {
                    account_code: re_account,
                    account_type: AccountType::Equity,
                    debit,
                    credit,
                });
            }
        }
    }

    openings
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveDateTime};
    use datasynth_core::models::balance::{
        AccountCategory, TrialBalanceLine, TrialBalanceStatus, TrialBalanceType,
    };
    use rust_decimal_macros::dec;
    use std::collections::HashMap;

    /// Pin the US-GAAP retained-earnings account this module's tests
    /// reference. Sourced from
    /// `datasynth_core::accounts::equity_accounts::RETAINED_EARNINGS`
    /// so a future re-numbering can't silently break the tests.
    const RE_US_GAAP: &str = datasynth_core::accounts::equity_accounts::RETAINED_EARNINGS;

    fn tb_line(
        code: &str,
        account_type: AccountType,
        category: AccountCategory,
        debit: Decimal,
        credit: Decimal,
    ) -> TrialBalanceLine {
        TrialBalanceLine {
            account_code: code.to_string(),
            account_description: format!("Test {code}"),
            category,
            account_type,
            opening_balance: Decimal::ZERO,
            period_debits: Decimal::ZERO,
            period_credits: Decimal::ZERO,
            closing_balance: debit - credit,
            debit_balance: debit,
            credit_balance: credit,
            cost_center: None,
            profit_center: None,
        }
    }

    fn make_tb(lines: Vec<TrialBalanceLine>) -> TrialBalance {
        let total_debits: Decimal = lines.iter().map(|l| l.debit_balance).sum();
        let total_credits: Decimal = lines.iter().map(|l| l.credit_balance).sum();
        TrialBalance {
            trial_balance_id: "TB001".into(),
            company_code: "C001".into(),
            company_name: None,
            as_of_date: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            fiscal_year: 2026,
            fiscal_period: 12,
            currency: "USD".into(),
            balance_type: TrialBalanceType::PostClosing,
            lines,
            total_debits,
            total_credits,
            is_balanced: total_debits == total_credits,
            out_of_balance: total_debits - total_credits,
            is_equation_valid: true,
            equation_difference: Decimal::ZERO,
            category_summary: HashMap::new(),
            created_at: NaiveDateTime::default(),
            created_by: "test".into(),
            approved_by: None,
            approved_at: None,
            status: TrialBalanceStatus::Final,
        }
    }

    #[test]
    fn bs_accounts_carry_pl_accounts_do_not() {
        // Mini TB: cash + AP + revenue + cogs (no RE — net income
        // should synthesize one).
        let lines = vec![
            tb_line(
                "1000",
                AccountType::Asset,
                AccountCategory::CurrentAssets,
                dec!(10_000),
                dec!(0),
            ),
            tb_line(
                "2000",
                AccountType::Liability,
                AccountCategory::CurrentLiabilities,
                dec!(0),
                dec!(4_000),
            ),
            tb_line(
                "4000",
                AccountType::Revenue,
                AccountCategory::Revenue,
                dec!(0),
                dec!(8_000),
            ),
            tb_line(
                "5000",
                AccountType::Expense,
                AccountCategory::OperatingExpenses,
                dec!(2_000),
                dec!(0),
            ),
        ];
        let tb = make_tb(lines);

        let openings = project_closing_to_opening(&tb, "us_gaap");

        // Should have 3 rows: 1000 (asset), 2000 (liability), 3300
        // (synthesized RE) — Revenue + Expense excluded.
        assert_eq!(openings.len(), 3);
        let codes: Vec<&str> = openings.iter().map(|o| o.account_code.as_str()).collect();
        assert!(codes.contains(&"1000"));
        assert!(codes.contains(&"2000"));
        assert!(codes.contains(&RE_US_GAAP));
        assert!(!codes.contains(&"4000"), "revenue must not carry");
        assert!(!codes.contains(&"5000"), "expense must not carry");
    }

    #[test]
    fn retained_earnings_absorbs_net_income() {
        // Net income = (8000 revenue credit) + (-2000 expense debit
        // contribution) = (0 - 8000) wait, net_income = Σ(credit -
        // debit) over P&L. For 4000 (credit 8000): credit - debit =
        // 8000. For 5000 (debit 2000): credit - debit = -2000. Sum =
        // 6000 net income.
        let lines = vec![
            tb_line(
                "1000",
                AccountType::Asset,
                AccountCategory::CurrentAssets,
                dec!(20_000),
                dec!(0),
            ),
            tb_line(
                "2000",
                AccountType::Liability,
                AccountCategory::CurrentLiabilities,
                dec!(0),
                dec!(4_000),
            ),
            tb_line(
                "3000",
                AccountType::Equity,
                AccountCategory::Equity,
                dec!(0),
                dec!(10_000),
            ),
            tb_line(
                RE_US_GAAP,
                AccountType::Equity,
                AccountCategory::Equity,
                dec!(0),
                dec!(0),
            ),
            tb_line(
                "4000",
                AccountType::Revenue,
                AccountCategory::Revenue,
                dec!(0),
                dec!(8_000),
            ),
            tb_line(
                "5000",
                AccountType::Expense,
                AccountCategory::OperatingExpenses,
                dec!(2_000),
                dec!(0),
            ),
        ];
        let tb = make_tb(lines);

        let openings = project_closing_to_opening(&tb, "us_gaap");

        let re = openings
            .iter()
            .find(|o| o.account_code == RE_US_GAAP)
            .expect("RE row missing");
        assert_eq!(
            re.credit,
            dec!(6000),
            "RE should absorb +6000 net income (8000 revenue - 2000 expense)"
        );
        assert_eq!(re.debit, dec!(0));
    }

    #[test]
    fn opening_balance_is_balanced_after_close() {
        // A balanced closing TB must yield a balanced opening
        // balance after the close. Build a balanced TB:
        //   Assets    = 20_000
        //   Liab      =  4_000
        //   Common Eq = 10_000
        //   RE        =      0 (will absorb net income)
        //   Revenue   =  8_000 (credit)
        //   Expense   =  2_000 (debit)
        // Total DR = 20_000 + 2_000 = 22_000
        // Total CR = 4_000 + 10_000 + 8_000 = 22_000. ✓
        let lines = vec![
            tb_line(
                "1000",
                AccountType::Asset,
                AccountCategory::CurrentAssets,
                dec!(20_000),
                dec!(0),
            ),
            tb_line(
                "2000",
                AccountType::Liability,
                AccountCategory::CurrentLiabilities,
                dec!(0),
                dec!(4_000),
            ),
            tb_line(
                "3000",
                AccountType::Equity,
                AccountCategory::Equity,
                dec!(0),
                dec!(10_000),
            ),
            tb_line(
                RE_US_GAAP,
                AccountType::Equity,
                AccountCategory::Equity,
                dec!(0),
                dec!(0),
            ),
            tb_line(
                "4000",
                AccountType::Revenue,
                AccountCategory::Revenue,
                dec!(0),
                dec!(8_000),
            ),
            tb_line(
                "5000",
                AccountType::Expense,
                AccountCategory::OperatingExpenses,
                dec!(2_000),
                dec!(0),
            ),
        ];
        let tb = make_tb(lines);
        assert!(tb.is_balanced, "test fixture must be balanced");

        let openings = project_closing_to_opening(&tb, "us_gaap");

        // After close: Σ(debit) and Σ(credit) over openings must be equal.
        let total_dr: Decimal = openings.iter().map(|o| o.debit).sum();
        let total_cr: Decimal = openings.iter().map(|o| o.credit).sum();
        assert_eq!(
            total_dr, total_cr,
            "openings unbalanced: DR={total_dr} CR={total_cr} (Σ(BS lines + RE-with-net-income) must net to zero)"
        );

        // And the accounting equation Assets = Liabilities + Equity
        // (using net_balance which sign-aware sums each account type).
        let assets: Decimal = openings
            .iter()
            .filter(|o| {
                matches!(
                    o.account_type,
                    AccountType::Asset | AccountType::ContraAsset
                )
            })
            .map(|o| o.net_balance())
            .sum();
        let liab: Decimal = openings
            .iter()
            .filter(|o| {
                matches!(
                    o.account_type,
                    AccountType::Liability | AccountType::ContraLiability
                )
            })
            .map(|o| o.net_balance())
            .sum();
        let equity: Decimal = openings
            .iter()
            .filter(|o| {
                matches!(
                    o.account_type,
                    AccountType::Equity | AccountType::ContraEquity
                )
            })
            .map(|o| o.net_balance())
            .sum();
        assert_eq!(
            assets,
            liab + equity,
            "accounting equation broken: Assets={assets} Liabilities={liab} Equity={equity}"
        );
    }

    #[test]
    fn loss_year_pushes_retained_earnings_to_debit() {
        // Loss = expense exceeds revenue.
        let lines = vec![
            tb_line(
                "1000",
                AccountType::Asset,
                AccountCategory::CurrentAssets,
                dec!(5_000),
                dec!(0),
            ),
            tb_line(
                "2000",
                AccountType::Liability,
                AccountCategory::CurrentLiabilities,
                dec!(0),
                dec!(2_000),
            ),
            tb_line(
                "3000",
                AccountType::Equity,
                AccountCategory::Equity,
                dec!(0),
                dec!(3_000),
            ),
            tb_line(
                RE_US_GAAP,
                AccountType::Equity,
                AccountCategory::Equity,
                dec!(0),
                dec!(500),
            ), // small positive RE entering
            tb_line(
                "4000",
                AccountType::Revenue,
                AccountCategory::Revenue,
                dec!(0),
                dec!(1_000),
            ),
            tb_line(
                "5000",
                AccountType::Expense,
                AccountCategory::OperatingExpenses,
                dec!(2_500),
                dec!(0),
            ),
        ];
        let tb = make_tb(lines);

        let openings = project_closing_to_opening(&tb, "us_gaap");

        // Net income = (1000) + (-2500) = -1500 (a 1 500 loss).
        // Closing RE = 500 credit; after absorbing loss = 500 - 1500
        // = -1000 → goes to debit side.
        let re = openings
            .iter()
            .find(|o| o.account_code == RE_US_GAAP)
            .expect("RE missing");
        assert_eq!(
            re.debit,
            dec!(1000),
            "RE should land on debit side after loss exceeds prior credit"
        );
        assert_eq!(re.credit, dec!(0));
    }

    #[test]
    fn zero_net_income_leaves_re_untouched() {
        // Revenue = expense → net income 0. RE row stays exactly as-is.
        let lines = vec![
            tb_line(
                "1000",
                AccountType::Asset,
                AccountCategory::CurrentAssets,
                dec!(10_000),
                dec!(0),
            ),
            tb_line(
                RE_US_GAAP,
                AccountType::Equity,
                AccountCategory::Equity,
                dec!(0),
                dec!(10_000),
            ),
            tb_line(
                "4000",
                AccountType::Revenue,
                AccountCategory::Revenue,
                dec!(0),
                dec!(3_000),
            ),
            tb_line(
                "5000",
                AccountType::Expense,
                AccountCategory::OperatingExpenses,
                dec!(3_000),
                dec!(0),
            ),
        ];
        let tb = make_tb(lines);

        let openings = project_closing_to_opening(&tb, "us_gaap");

        let re = openings
            .iter()
            .find(|o| o.account_code == RE_US_GAAP)
            .expect("RE missing");
        assert_eq!(
            re.credit,
            dec!(10_000),
            "RE should be unchanged when net income is zero"
        );
        assert_eq!(re.debit, dec!(0));
    }
}

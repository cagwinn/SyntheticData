//! Mini-Nestlé translation round-trip e2e (Task 6.5).
//!
//! Hand-builds per-entity TBs for the four Mini-Nestlé fixtures
//! (NESTLE_SA / parent CHF, NESTLE_USA / USD, NESTLE_DE / EUR,
//! NESTLE_BR / BRL) and exercises the IAS 21 translation pipeline
//! end-to-end without invoking the orchestrator (the orchestrator
//! would OOM the host — see Task 4.5 and the XXL VM plan).
//!
//! Acceptance:
//! - All four functional currencies produce a `TranslatedTb`.
//! - CTA rollforward has exactly 3 non-parent entities (USA / DE / BR;
//!   the JV is equity-method, deferred to Chunk 7).
//! - Each non-parent CTA is non-zero (rate bases differ).
//! - NESTLE_SA lines are identity-translated (CHF == CHF).

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_core::accounts::{
    cash_accounts, control_accounts, equity_accounts, expense_accounts, revenue_accounts,
};
use datasynth_core::models::balance::{
    AccountCategory, AccountType, TrialBalance, TrialBalanceLine, TrialBalanceType,
};
use datasynth_group::config::{FxPolicyConfig, FxRateBasis};
use datasynth_group::manifest::FxRateMaster;
use datasynth_group::{
    cta_rollforward, translate_entity_tb, write_cta_rollforward, write_translation_worksheet,
    CtaRollforward, TranslatedTb,
};
use datasynth_standards::framework::AccountingFramework;

const PRESENTATION_CCY: &str = "CHF";

fn period_end() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
}

fn default_policy() -> FxPolicyConfig {
    FxPolicyConfig {
        balance_sheet: FxRateBasis::Closing,
        income_statement: FxRateBasis::Average,
        equity: FxRateBasis::Historical,
    }
}

/// Build the Mini-Nestlé `FxRateMaster` covering USD/CHF, EUR/CHF, BRL/CHF.
///
/// Mirrors the inline rates in `fixtures/mini_nestle.yaml` after
/// auto-inversion to canonical FUNCTIONAL/PRESENTATION direction.
fn nestle_fx_master() -> FxRateMaster {
    let dates = [
        NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    ];

    // CHF/foreign rates from the fixture (presentation/functional).
    // We invert each to get USD/CHF, EUR/CHF, BRL/CHF for translation.
    let chf_usd = [dec!(0.8870), dec!(0.8812), dec!(0.9012)];
    let chf_eur = [dec!(0.9520), dec!(0.9488), dec!(0.9611)];
    let chf_brl = [dec!(5.5210), dec!(5.4880), dec!(5.6700)];

    let mk_pair = |label: &str, raw: &[Decimal; 3]|
        -> (String, BTreeMap<NaiveDate, Decimal>, Decimal, Decimal)
    {
        let mut table: BTreeMap<NaiveDate, Decimal> = BTreeMap::new();
        let mut sum = Decimal::ZERO;
        let mut closing = Decimal::ZERO;
        for (i, r) in raw.iter().enumerate() {
            let inv = Decimal::ONE / r;
            table.insert(dates[i], inv);
            sum += inv;
            closing = inv;
        }
        let avg = sum / Decimal::from(raw.len() as u32);
        (label.to_string(), table, closing, avg)
    };

    let pairs = vec![
        mk_pair("USD/CHF", &chf_usd),
        mk_pair("EUR/CHF", &chf_eur),
        mk_pair("BRL/CHF", &chf_brl),
    ];

    let mut rates = BTreeMap::new();
    let mut closing_by_pair = BTreeMap::new();
    let mut average_by_pair = BTreeMap::new();
    for (label, table, closing, avg) in pairs {
        rates.insert(label.clone(), table);
        closing_by_pair.insert(label.clone(), closing);
        average_by_pair.insert(label, avg);
    }

    FxRateMaster {
        base_currency: PRESENTATION_CCY.to_string(),
        policy: default_policy(),
        rates,
        closing_by_pair,
        average_by_pair,
    }
}

/// Hand-roll a small balanced TB for the given entity / currency.
///
/// Layout — six lines, balanced:
///   DR cash 10,000
///   DR AR 5,000
///   DR COGS 4,000
///   CR revenue 7,000
///   CR AP 3,000
///   CR retained earnings 9,000
fn make_tb(company_code: &str, currency: &str) -> TrialBalance {
    let mut tb = TrialBalance::new(
        format!("TB_{company_code}"),
        company_code.to_string(),
        period_end(),
        2024,
        3,
        currency.to_string(),
        TrialBalanceType::Adjusted,
    );

    let push_dr = |tb: &mut TrialBalance, code: &str, ty: AccountType, amt: Decimal| {
        tb.add_line(TrialBalanceLine {
            account_code: code.to_string(),
            account_description: code.to_string(),
            category: AccountCategory::from_account_type(ty),
            account_type: ty,
            opening_balance: Decimal::ZERO,
            period_debits: amt,
            period_credits: Decimal::ZERO,
            closing_balance: amt,
            debit_balance: amt,
            credit_balance: Decimal::ZERO,
            cost_center: None,
            profit_center: None,
        });
    };
    let push_cr = |tb: &mut TrialBalance, code: &str, ty: AccountType, amt: Decimal| {
        tb.add_line(TrialBalanceLine {
            account_code: code.to_string(),
            account_description: code.to_string(),
            category: AccountCategory::from_account_type(ty),
            account_type: ty,
            opening_balance: Decimal::ZERO,
            period_debits: Decimal::ZERO,
            period_credits: amt,
            closing_balance: amt,
            debit_balance: Decimal::ZERO,
            credit_balance: amt,
            cost_center: None,
            profit_center: None,
        });
    };

    push_dr(
        &mut tb,
        cash_accounts::OPERATING_CASH,
        AccountType::Asset,
        dec!(10000),
    );
    push_dr(
        &mut tb,
        control_accounts::AR_CONTROL,
        AccountType::Asset,
        dec!(5000),
    );
    push_dr(
        &mut tb,
        expense_accounts::COGS,
        AccountType::Expense,
        dec!(4000),
    );
    push_cr(
        &mut tb,
        revenue_accounts::PRODUCT_REVENUE,
        AccountType::Revenue,
        dec!(7000),
    );
    push_cr(
        &mut tb,
        control_accounts::AP_CONTROL,
        AccountType::Liability,
        dec!(3000),
    );
    push_cr(
        &mut tb,
        equity_accounts::RETAINED_EARNINGS,
        AccountType::Equity,
        dec!(9000),
    );

    tb
}

#[test]
fn mini_nestle_translation_e2e() {
    // ── 1. Build per-entity TBs for the four functional currencies. ──
    let entities = [
        ("NESTLE_SA", "CHF"), // parent — identity translation
        ("NESTLE_USA", "USD"),
        ("NESTLE_DE", "EUR"),
        ("NESTLE_BR", "BRL"),
    ];

    let master = nestle_fx_master();
    let framework = AccountingFramework::default();

    // ── 2. Translate each entity's TB. ───────────────────────────────
    let translated: Vec<TranslatedTb> = entities
        .iter()
        .map(|(code, ccy)| {
            let tb = make_tb(code, ccy);
            translate_entity_tb(&tb, ccy, &master, period_end(), PRESENTATION_CCY, framework)
                .unwrap_or_else(|e| panic!("translate {code} ({ccy} → CHF) failed: {e}"))
        })
        .collect();

    assert_eq!(translated.len(), 4, "all 4 entities must produce a TranslatedTb");

    // All 4 functional currencies appear in the output.
    let mut functional_ccys: Vec<&str> = translated
        .iter()
        .map(|t| t.functional_currency.as_str())
        .collect();
    functional_ccys.sort();
    assert_eq!(functional_ccys, vec!["BRL", "CHF", "EUR", "USD"]);

    // Every output names CHF as the presentation currency.
    for t in &translated {
        assert_eq!(t.presentation_currency, PRESENTATION_CCY);
        assert_eq!(t.as_of_date, period_end());
    }

    // ── 3. NESTLE_SA must be identity-translated. ───────────────────
    let parent = translated
        .iter()
        .find(|t| t.entity_code == "NESTLE_SA")
        .expect("NESTLE_SA must be in output");
    assert_eq!(parent.functional_currency, "CHF");
    assert_eq!(parent.presentation_currency, "CHF");
    for line in &parent.lines {
        assert_eq!(
            line.fx_rate,
            Decimal::ONE,
            "parent line {} must have rate 1.0",
            line.account_code
        );
        assert_eq!(line.translated_amount, line.local_amount);
    }
    assert_eq!(parent.cta, Decimal::ZERO, "parent CTA must be 0");

    // ── 4. CTA rollforward for the 3 non-parent foreign entities. ───
    //
    // The JV (equity-method) is deferred to Chunk 7 — there is no JV
    // in this fixture set, so we expect exactly 3 rollforwards.
    let rollforwards: Vec<CtaRollforward> = translated
        .iter()
        .filter(|t| t.entity_code != "NESTLE_SA")
        .map(|t| {
            cta_rollforward(
                &t.entity_code,
                &t.functional_currency,
                &t.presentation_currency,
                Decimal::ZERO, // first period: no opening CTA
                t.cta,
            )
        })
        .collect();

    assert_eq!(
        rollforwards.len(),
        3,
        "expected 3 non-parent CTA rollforwards (USA, DE, BR); JV is equity-method, deferred",
    );

    // Each non-parent CTA must be non-zero (different closing vs.
    // average rates ⇒ residual ≠ 0).
    for rf in &rollforwards {
        assert_ne!(
            rf.period_cta,
            Decimal::ZERO,
            "non-parent {} ({}) period CTA must be non-zero",
            rf.entity_code,
            rf.functional_currency,
        );
        // Identity: closing = opening + period; opening here is 0.
        assert_eq!(rf.closing_cta, rf.period_cta);
    }

    // Every non-parent entity (sorted) is represented.
    let mut codes: Vec<&str> = rollforwards.iter().map(|r| r.entity_code.as_str()).collect();
    codes.sort();
    assert_eq!(codes, vec!["NESTLE_BR", "NESTLE_DE", "NESTLE_USA"]);

    // ── 5. Round-trip the rollforward and worksheet through disk. ───
    let tmp = tempfile::tempdir().expect("tmp dir");
    let cta_path =
        write_cta_rollforward(&rollforwards, tmp.path()).expect("write rollforward");
    assert!(cta_path.exists());
    let worksheet_path =
        write_translation_worksheet(&translated, tmp.path()).expect("write worksheet");
    assert!(worksheet_path.exists());

    // Worksheet must include all 4 entities.
    let bytes = std::fs::read(&worksheet_path).expect("read worksheet");
    let s = String::from_utf8(bytes).expect("utf8");
    for (code, _) in &entities {
        assert!(
            s.contains(&format!("\"entity_code\": \"{code}\"")),
            "worksheet must mention entity {code}"
        );
    }
}

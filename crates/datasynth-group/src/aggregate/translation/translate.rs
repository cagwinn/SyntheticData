//! Per-entity TB translation — Task 6.2.
//!
//! Walks each [`TrialBalance`] line, classifies its GL account via
//! [`crate::aggregate::translation::classify_account`], picks the
//! IAS 21 rate basis (closing / historical / average), looks up the
//! rate in the [`FxRateMaster`], and produces a [`TranslatedTb`] with
//! all lines translated to the presentation currency. The DR/CR
//! residual after translation is the **CTA** (currency translation
//! adjustment) the group will accumulate in OCI per IAS 21.39.
//!
//! # Identity case
//!
//! When `entity_functional_ccy == presentation_currency`, every line
//! translates at rate 1.0 and the residual CTA is exactly zero. The
//! function still produces a [`TranslatedTb`] with all lines (rate
//! basis preserved) so that downstream consumers can rely on the
//! shape being uniform across entities.
//!
//! # IAS 21 rate-basis mapping
//!
//! | Account type      | Rate basis        | Source                          |
//! |-------------------|-------------------|---------------------------------|
//! | `BsMonetary`      | `Closing`         | `closing_by_pair`               |
//! | `BsNonMonetary`   | `Historical`      | `average_by_pair` (proxy*)      |
//! | `Equity`          | `Historical`      | `average_by_pair` (proxy*)      |
//! | `PlRevenue`       | `Average`         | `average_by_pair`               |
//! | `PlExpense`       | `Average`         | `average_by_pair`               |
//! | `PlOci`           | `Average`         | `average_by_pair`               |
//!
//! \* IAS 21 strictly requires the rate at the date of the original
//! transaction (or initial recognition for equity). For v5.0 we use the
//! period's average as a proxy because the orchestrator does not yet
//! track per-line transaction dates. This approximation means that the
//! resulting CTA is slightly understated when intra-period FX volatility
//! is high; the gap closes as soon as the orchestrator emits dated
//! lines. See spec §"Aggregate phase — Chunk 6" for the planned
//! refinement path.
//!
//! # Determinism
//!
//! Lines in the output preserve the input ordering of
//! [`TrialBalance::lines`] verbatim. Two calls with identical inputs
//! produce byte-identical [`TranslatedTb`] structures.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use datasynth_core::models::balance::{AccountType, TrialBalance};
use datasynth_standards::framework::AccountingFramework;

use crate::aggregate::translation::classify::{classify_account, TranslationAccountType};
use crate::errors::{GroupError, GroupResult};
use crate::manifest::FxRateMaster;

// ── Public types ──────────────────────────────────────────────────────────────

/// Direction (debit or credit) of a translated TB line.
///
/// `TranslatedLine::translated_amount` is always **non-negative**; the
/// direction is carried separately in `local_dr_cr`. This mirrors the
/// underlying [`datasynth_core::models::balance::TrialBalanceLine`]
/// where `debit_balance` and `credit_balance` are both non-negative
/// and at most one is non-zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrCr {
    /// Debit-side line.
    Debit,
    /// Credit-side line.
    Credit,
}

/// IAS 21 rate basis applied to a translated line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateBasis {
    /// Closing rate at `period_end` (BS monetary items).
    Closing,
    /// Historical rate at the date of recognition (BS non-monetary,
    /// equity). Proxied by the period average for v5.0.
    Historical,
    /// Period average rate (P&L items, OCI).
    Average,
}

/// One TB line after translation to the presentation currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslatedLine {
    /// GL account code (unchanged from the source TB).
    pub account_code: String,
    /// Original amount in functional currency (always non-negative).
    pub local_amount: Decimal,
    /// Side of the original line (debit or credit).
    pub local_dr_cr: DrCr,
    /// Rate applied to translate the line.
    pub fx_rate: Decimal,
    /// Which IAS 21 basis the rate came from.
    pub rate_basis: RateBasis,
    /// Amount in presentation currency (always non-negative; direction
    /// in `local_dr_cr`).
    pub translated_amount: Decimal,
    /// Translation classification of the account.
    pub account_type: TranslationAccountType,
}

/// One entity's TB after IAS 21 translation to the presentation
/// currency.
///
/// `cta` is the residual `total_translated_debits -
/// total_translated_credits`. For functional == presentation entities
/// this is exactly zero; for foreign entities it reflects the
/// difference in rate bases applied across the BS / P&L / equity lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslatedTb {
    /// Entity code (mirrors [`TrialBalance::company_code`]).
    pub entity_code: String,
    /// Functional currency the source TB was denominated in.
    pub functional_currency: String,
    /// Group presentation currency the lines are translated to.
    pub presentation_currency: String,
    /// As-of date the TB was struck (mirrors
    /// [`TrialBalance::as_of_date`]).
    pub as_of_date: NaiveDate,
    /// One translated line per source TB line (input order preserved).
    pub lines: Vec<TranslatedLine>,
    /// Sum of `translated_amount` across all `Debit` lines.
    pub total_translated_debits: Decimal,
    /// Sum of `translated_amount` across all `Credit` lines.
    pub total_translated_credits: Decimal,
    /// Currency translation adjustment — the residual after
    /// translation. Positive means net debit, negative means net
    /// credit. Booked to OCI on aggregation.
    pub cta: Decimal,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Translate one entity's standalone TB from `entity_functional_ccy`
/// to `presentation_currency`, producing a [`TranslatedTb`] and the
/// implied CTA residual.
///
/// # Behaviour
///
/// 1. **Identity short-circuit.** If `entity_functional_ccy ==
///    presentation_currency`, every line is translated at rate 1.0
///    with `rate_basis` mirroring its IAS 21 classification (so the
///    output shape is uniform across entities). `cta` is exactly zero.
/// 2. **Foreign translation.** For each input line:
///    - classify the account via [`classify_account`],
///    - select the rate basis (closing / historical / average),
///    - look up the rate in the [`FxRateMaster`] under the canonical
///      pair key `"{functional}/{presentation}"`,
///    - multiply the local amount by the rate and emit a
///      [`TranslatedLine`] preserving the input direction.
/// 3. **Totals & CTA.** Sum the translated debits and credits. The
///    residual is the CTA.
///
/// # Errors
///
/// - [`GroupError::Aggregate`] if a required FX rate is missing from
///   the [`FxRateMaster`]. The error message names the missing pair,
///   the rate basis, and the period date so the operator can fix the
///   manifest input.
pub fn translate_entity_tb(
    entity_tb: &TrialBalance,
    entity_functional_ccy: &str,
    fx_rate_master: &FxRateMaster,
    period_end: NaiveDate,
    presentation_currency: &str,
    framework: AccountingFramework,
) -> GroupResult<TranslatedTb> {
    let identity = entity_functional_ccy == presentation_currency;
    let pair_key = format!("{entity_functional_ccy}/{presentation_currency}");

    let mut lines = Vec::with_capacity(entity_tb.lines.len());
    let mut total_dr = Decimal::ZERO;
    let mut total_cr = Decimal::ZERO;

    for tbl in &entity_tb.lines {
        let account_type = classify_account(&tbl.account_code, framework);
        let rate_basis = rate_basis_for(account_type);

        let (local_amount, local_dr_cr) = direction_and_amount(tbl);

        let fx_rate = if identity {
            Decimal::ONE
        } else {
            lookup_rate(fx_rate_master, &pair_key, rate_basis, period_end)?
        };

        let translated_amount = (local_amount * fx_rate).round_dp(2);

        match local_dr_cr {
            DrCr::Debit => total_dr += translated_amount,
            DrCr::Credit => total_cr += translated_amount,
        }

        lines.push(TranslatedLine {
            account_code: tbl.account_code.clone(),
            local_amount,
            local_dr_cr,
            fx_rate,
            rate_basis,
            translated_amount,
            account_type,
        });
    }

    let cta = total_dr - total_cr;

    Ok(TranslatedTb {
        entity_code: entity_tb.company_code.clone(),
        functional_currency: entity_functional_ccy.to_string(),
        presentation_currency: presentation_currency.to_string(),
        as_of_date: entity_tb.as_of_date,
        lines,
        total_translated_debits: total_dr,
        total_translated_credits: total_cr,
        cta,
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Map a [`TranslationAccountType`] to the IAS 21 [`RateBasis`].
fn rate_basis_for(ty: TranslationAccountType) -> RateBasis {
    match ty {
        TranslationAccountType::BsMonetary => RateBasis::Closing,
        TranslationAccountType::BsNonMonetary | TranslationAccountType::Equity => {
            RateBasis::Historical
        }
        TranslationAccountType::PlRevenue
        | TranslationAccountType::PlExpense
        | TranslationAccountType::PlOci => RateBasis::Average,
    }
}

/// Pull the non-negative amount and direction out of a [`TrialBalanceLine`].
///
/// At most one of `debit_balance` / `credit_balance` is non-zero.
/// When both are zero (a zero-balance line), we tag the line as
/// `Debit` with amount 0 — this is a no-op for the totals and matches
/// the convention used by the underlying TB tooling
/// (`AccountType::Asset` with zero balance → debit-side row).
fn direction_and_amount(
    tbl: &datasynth_core::models::balance::TrialBalanceLine,
) -> (Decimal, DrCr) {
    if tbl.debit_balance > Decimal::ZERO {
        (tbl.debit_balance, DrCr::Debit)
    } else if tbl.credit_balance > Decimal::ZERO {
        (tbl.credit_balance, DrCr::Credit)
    } else {
        // Zero-balance line — choose direction by underlying account
        // type so the TranslatedLine stays internally consistent.
        let dir = match tbl.account_type {
            AccountType::Liability
            | AccountType::Equity
            | AccountType::Revenue
            | AccountType::ContraAsset => DrCr::Credit,
            _ => DrCr::Debit,
        };
        (Decimal::ZERO, dir)
    }
}

/// Look up the rate for a pair under the requested basis. Returns a
/// [`GroupError::Aggregate`] when the rate is missing.
fn lookup_rate(
    master: &FxRateMaster,
    pair_key: &str,
    basis: RateBasis,
    period_end: NaiveDate,
) -> GroupResult<Decimal> {
    let rate = match basis {
        RateBasis::Closing => master.closing_by_pair.get(pair_key).copied(),
        // v5.0 proxy: historical rate ≈ period average. See module
        // rustdoc for the rationale and refinement plan.
        RateBasis::Historical | RateBasis::Average => master.average_by_pair.get(pair_key).copied(),
    };

    rate.ok_or_else(|| {
        GroupError::Aggregate(format!(
            "translate_entity_tb: missing FX rate {pair_key} at {period_end} (basis={basis:?})"
        ))
    })
}

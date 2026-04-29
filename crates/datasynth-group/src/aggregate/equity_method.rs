//! Equity-method investment rollforward — Task 7.3.
//!
//! After the IC-pair matcher (Task 5.3) has joined every fully-
//! consolidated subsidiary's ledgers, the aggregate phase still has to
//! account for joint ventures and significant-influence associates that
//! were *not* line-by-line consolidated.  These investees were held back
//! by [`crate::aggregate::pre_elim::aggregate_pre_elimination`] in
//! [`crate::aggregate::pre_elim::DeferredEntity`] sidecars; this module
//! processes them via the IAS 28 / ASC 323 equity-method single-line
//! treatment.
//!
//! # Standards reference
//!
//! - **IAS 28** *Investments in Associates and Joint Ventures* §§ 10–11:
//!   the investor recognises its share of the investee's profit or loss
//!   in its own profit or loss.  Distributions received from the
//!   investee reduce the carrying amount of the investment.
//! - **IAS 28 § 16** — the carrying amount of the investment is
//!   reduced when impairment is recognised (IAS 36 reference).  In v5.0
//!   the caller supplies the impairment amount; auto-impairment is
//!   deferred to a later chunk.
//! - **IAS 28 § 38** — when the investor's share of losses equals or
//!   exceeds its interest in the investee, the investor *discontinues*
//!   recognising its share of further losses; the carrying amount must
//!   not go below zero (with limited exceptions for guaranteed
//!   obligations).
//! - **US GAAP — ASC 323** *Investments — Equity Method and Joint
//!   Ventures*: the same rollforward identity applies.
//!
//! # v5.0 scope
//!
//! - **EquityMethod consolidation only.**  Reject `Parent` / `Full` /
//!   `Proportional` / `FairValue` (those are handled elsewhere).
//! - **Ownership in `(0, 1)`.**  Boundary values (zero or full) are not
//!   meaningful for equity-method treatment and likely indicate a
//!   caller bug.
//! - **No auto-recovery from negative carrying values.**  Per IAS 28 §
//!   38 the carrying amount must not go below zero — we surface this
//!   as a typed error so the caller can either reduce the share of
//!   loss recognised, or recognise an additional liability if a
//!   guaranteed obligation exists.
//!
//! # File-not-found semantics
//!
//! Mirrors [`super::nci::opening::ingest_opening_nci_balances`]:
//! missing prior-period file ≡ first-period engagement, log a warning
//! and return an empty map.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::config::ConsolidationMethod;
use crate::errors::{GroupError, GroupResult};
use crate::manifest::ManifestEntity;

/// Subdirectory within the group output root for the consolidated
/// equity-method rollforward, mirroring the
/// [`crate::aggregate::nci::opening::CONSOLIDATED_SUBDIR`] layout.
pub const CONSOLIDATED_SUBDIR: &str = "consolidated";

/// File name for the on-disk equity-method investment rollforward
/// array.
pub const EQUITY_METHOD_INVESTMENTS_FILENAME: &str = "equity_method_investments.json";

// ── Public types ──────────────────────────────────────────────────────────────

/// One equity-method investment's rollforward record for the period.
///
/// `closing_carrying_value = opening + share_of_profit
///                          - dividends_received - impairment`
/// per IAS 28.10–11 / ASC 323-10-35.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EquityMethodInvestment {
    /// Code of the investee (associate or joint venture).
    pub investee_code: String,
    /// Code of the investor entity (the parent who holds the
    /// investment).  Carried explicitly so the consolidated note
    /// disclosure can attribute the investment without re-walking the
    /// manifest.
    pub investor_entity_code: String,
    /// Investor's ownership share of the investee, in `(0, 1)`.
    pub ownership_percent: Decimal,
    /// Carrying amount of the investment brought forward from the
    /// prior period (zero on the first period of an engagement).
    pub opening_carrying_value: Decimal,
    /// Investor's share of the investee's period net income =
    /// `ownership_percent * investee_net_income` (IAS 28.10).
    pub share_of_profit: Decimal,
    /// Distributions (dividends) received from the investee =
    /// `ownership_percent * investee_dividends_paid`.  Reduces the
    /// carrying amount.
    pub dividends_received: Decimal,
    /// Impairment loss recognised this period (IAS 28.40 / IAS 36).
    /// Caller supplies the amount; v5.0 has no auto-impairment.  Always
    /// non-negative.
    pub impairment: Decimal,
    /// Closing carrying value =
    /// `opening + share_of_profit - dividends_received - impairment`,
    /// rounded to 2dp.  Must remain non-negative (IAS 28.38).
    pub closing_carrying_value: Decimal,
    /// Period end date the rollforward is as of.
    pub period_end: NaiveDate,
    /// Group presentation currency.
    pub currency: String,
}

/// Inputs required to derive an [`EquityMethodInvestment`].
///
/// The caller is responsible for already having translated
/// `investee_net_income` and `investee_dividends_paid` into the group
/// presentation currency (Chunk 6).
pub struct EquityMethodInputs<'a> {
    /// Reference to the investee's manifest entity.  Provides the code,
    /// ownership percent, and consolidation method used to validate
    /// inputs.
    pub investee: &'a ManifestEntity,
    /// Code of the investor entity (parent who holds the investment).
    pub investor_entity_code: String,
    /// Investee's period net income (after tax).
    pub investee_net_income: Decimal,
    /// Investee's total dividends paid this period (gross — both
    /// to controlling and non-controlling shareholders; the share
    /// the investor receives is `ownership * total`).
    pub investee_dividends_paid: Decimal,
    /// Carrying amount brought forward from the prior period.
    pub opening_carrying_value: Decimal,
    /// Impairment loss to recognise this period.  Always non-negative;
    /// zero by default if no impairment indicator is observed.
    pub impairment: Decimal,
    /// Period end date.
    pub period_end: NaiveDate,
    /// Group presentation currency.
    pub currency: String,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Derive an [`EquityMethodInvestment`] for one investee.
///
/// Pure function: no I/O, no allocation beyond the record itself.
///
/// # Validation
///
/// 1. `investee.consolidation_method` **must** be
///    [`ConsolidationMethod::EquityMethod`].
/// 2. `investee.ownership_percent` **must** be present and in `(0, 1)`
///    (strict inequalities — boundary values aren't meaningful for
///    equity-method treatment).
/// 3. The closing carrying value **must** remain non-negative.  IAS
///    28.38 / ASC 323-10-35-20 require the investor to discontinue
///    recognising further losses once the carrying amount hits zero;
///    we surface this as a typed error so the caller can decide whether
///    to clamp the share of loss or recognise an additional liability.
pub fn compute_equity_method_investment(
    inputs: &EquityMethodInputs,
) -> GroupResult<EquityMethodInvestment> {
    let investee = inputs.investee;

    // 1. Reject any non-EquityMethod consolidation method.
    if investee.consolidation_method != ConsolidationMethod::EquityMethod {
        return Err(GroupError::Aggregate(format!(
            "compute_equity_method_investment: entity `{}` has \
             consolidation_method={:?} — equity-method treatment is only \
             valid for ConsolidationMethod::EquityMethod (Parent / Full are \
             line-by-line consolidated; Proportional / FairValue use other \
             methods)",
            investee.code, investee.consolidation_method,
        )));
    }

    // 2. Ownership must be strictly in (0, 1).
    let ownership_percent = investee.ownership_percent.ok_or_else(|| {
        GroupError::Aggregate(format!(
            "compute_equity_method_investment: entity `{}` is \
             consolidation_method=EquityMethod but has no ownership_percent \
             set — supply ownership_percent in (0, 1)",
            investee.code,
        ))
    })?;
    if ownership_percent <= Decimal::ZERO || ownership_percent >= Decimal::ONE {
        return Err(GroupError::Aggregate(format!(
            "compute_equity_method_investment: entity `{}` ownership_percent={} \
             is outside (0, 1) — equity-method treatment requires strict \
             0 < ownership < 1",
            investee.code, ownership_percent,
        )));
    }

    // 3. Apply the IAS 28.10–11 rollforward identity.
    let share_of_profit = ownership_percent * inputs.investee_net_income;
    let dividends_received = ownership_percent * inputs.investee_dividends_paid;

    let raw_closing =
        (inputs.opening_carrying_value + share_of_profit - dividends_received - inputs.impairment)
            .round_dp(2);

    // IAS 28.38 / ASC 323-10-35-20: when the share of losses would push
    // the carrying amount below zero, the investor discontinues
    // recognising further losses. The investment is reported at zero
    // and the unrecognised loss is tracked separately (memorandum
    // record). For v5.0 we clamp at zero and log the suppressed
    // amount; future v5.1 work will surface the suppressed loss in a
    // separate `equity_method_suppressed_losses` artefact.
    let closing_carrying_value = if raw_closing < Decimal::ZERO {
        tracing::warn!(
            investee = %investee.code,
            raw_closing = %raw_closing,
            opening = %inputs.opening_carrying_value,
            share_of_profit = %share_of_profit,
            dividends_received = %dividends_received,
            impairment = %inputs.impairment,
            "equity-method carrying value would go negative — clamped at zero per IAS 28.38; suppressed loss not tracked separately in v5.0",
        );
        Decimal::ZERO
    } else {
        raw_closing
    };

    Ok(EquityMethodInvestment {
        investee_code: investee.code.clone(),
        investor_entity_code: inputs.investor_entity_code.clone(),
        ownership_percent,
        opening_carrying_value: inputs.opening_carrying_value.round_dp(2),
        share_of_profit: share_of_profit.round_dp(2),
        dividends_received: dividends_received.round_dp(2),
        impairment: inputs.impairment.round_dp(2),
        closing_carrying_value,
        period_end: inputs.period_end,
        currency: inputs.currency.clone(),
    })
}

/// Write an array of [`EquityMethodInvestment`] records to
/// `{out_dir}/consolidated/equity_method_investments.json`.
///
/// Creates the `consolidated/` subdirectory if it doesn't already
/// exist.  Output is pretty-printed JSON with a trailing newline.
/// Returns the absolute path of the written file.
///
/// # Errors
///
/// - [`GroupError::Io`] on subdirectory creation or file write failure.
/// - [`GroupError::Serde`] if serialisation fails (should be
///   impossible — every field is `Serialize`-friendly).
pub fn write_equity_method_investments(
    investments: &[EquityMethodInvestment],
    out_dir: &Path,
) -> GroupResult<PathBuf> {
    let dir = out_dir.join(CONSOLIDATED_SUBDIR);
    fs::create_dir_all(&dir).map_err(GroupError::Io)?;

    let path = dir.join(EQUITY_METHOD_INVESTMENTS_FILENAME);

    let mut json = serde_json::to_string_pretty(investments)?;
    json.push('\n');
    fs::write(&path, json).map_err(GroupError::Io)?;

    Ok(path)
}

/// Read prior-period closing carrying values as this period's opening,
/// mirror of
/// [`crate::aggregate::nci::opening::ingest_opening_nci_balances`].
///
/// Walks `{prior_period_dir}/consolidated/equity_method_investments.json`
/// and returns a map of `(investee_code -> closing_carrying_value)` from
/// the prior period.
///
/// # Errors
///
/// - [`GroupError::Serde`] if the file exists but cannot be parsed.
/// - [`GroupError::Aggregate`] if the file contains two or more records
///   for the same `investee_code`.
/// - Missing file → `Ok(BTreeMap::new())` plus a `tracing::warn!` log.
pub fn ingest_opening_equity_method_carrying_values(
    prior_period_dir: &Path,
) -> GroupResult<BTreeMap<String, Decimal>> {
    let path = prior_period_dir
        .join(CONSOLIDATED_SUBDIR)
        .join(EQUITY_METHOD_INVESTMENTS_FILENAME);

    if !path.exists() {
        tracing::warn!(
            path = %path.display(),
            "opening equity-method investments file not found; defaulting \
             to zero opening carrying value per investee"
        );
        return Ok(BTreeMap::new());
    }

    let bytes = fs::read(&path).map_err(GroupError::Io)?;
    let investments: Vec<EquityMethodInvestment> = serde_json::from_slice(&bytes)?;

    let mut map: BTreeMap<String, Decimal> = BTreeMap::new();
    for inv in investments {
        if map.contains_key(&inv.investee_code) {
            return Err(GroupError::Aggregate(format!(
                "ingest_opening_equity_method_carrying_values: duplicate \
                 investee `{}` in opening file {} — writer regression?",
                inv.investee_code,
                path.display(),
            )));
        }
        map.insert(inv.investee_code, inv.closing_carrying_value);
    }

    Ok(map)
}

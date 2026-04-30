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
//! # v5.1 scope (extended from v5.0)
//!
//! - **EquityMethod consolidation only.**  Reject `Parent` / `Full` /
//!   `Proportional` / `FairValue` (those are handled elsewhere).
//! - **Ownership in `(0, 1)`.**  Boundary values (zero or full) are not
//!   meaningful for equity-method treatment and likely indicate a
//!   caller bug.
//! - **IAS 28 § 38 first paragraph (clamp).**  When the rollforward
//!   would push the carrying value below zero, the carrying value is
//!   clamped at zero and the unrecognised amount accumulates in the
//!   suppressed-loss memorandum (`suppressed_loss_this_period` +
//!   `closing_suppressed_loss`).  v5.0 logged the suppressed amount
//!   but did not surface it to consumers; v5.1 emits the per-investee
//!   memorandum on every record plus a filtered `*_suppressed_losses`
//!   side-artefact.
//! - **IAS 28 § 38 second paragraph (recovery against future profits).**
//!   When the investee subsequently reports profits, the entity resumes
//!   recognising its share only after its share of profits equals the
//!   share of losses not recognised.  The caller passes
//!   `opening_suppressed_loss` (typically read from the prior period
//!   via [`ingest_opening_suppressed_losses`]); positive
//!   `share_of_profit` is applied against this opening balance first,
//!   and only the residual flows to `share_of_profit_recognised` and
//!   the carrying-value rollforward.
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

/// File name for the v5.1+ IAS 28.38 suppressed-loss memorandum
/// artefact.  Contains only the records where
/// `closing_suppressed_loss > 0`, surfaced separately so consumers
/// can disclose suppressed-loss balances without scanning the full
/// equity-method file.
pub const EQUITY_METHOD_SUPPRESSED_LOSSES_FILENAME: &str = "equity_method_suppressed_losses.json";

// ── Public types ──────────────────────────────────────────────────────────────

/// One equity-method investment's rollforward record for the period.
///
/// `closing_carrying_value = opening + share_of_profit_recognised
///                          - dividends_received - impairment`
/// per IAS 28.10–11 / ASC 323-10-35, **clamped at zero** per IAS 28.38.
///
/// When the rollforward would push the carrying value below zero, the
/// investor discontinues recognising further losses; the unrecognised
/// amount is tracked as `suppressed_loss_this_period` and accumulated
/// in `closing_suppressed_loss` for use against future profits.
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
    /// Cumulative losses suppressed (not recognised in profit or loss)
    /// brought forward from prior periods per IAS 28.38.  Zero on the
    /// first period of an engagement.  v5.1+: applied against current
    /// period profits before any further share is recognised.
    #[serde(default)]
    pub opening_suppressed_loss: Decimal,
    /// Investor's "natural" share of the investee's period net
    /// income = `ownership_percent * investee_net_income` (IAS 28.10),
    /// before any clawback against opening suppressed losses.
    pub share_of_profit: Decimal,
    /// Amount of `share_of_profit` actually recognised in profit or
    /// loss this period.  Equals `share_of_profit` when there is no
    /// opening suppressed loss; otherwise reduced by the portion
    /// applied against the opening cumulative suppressed loss
    /// (IAS 28.38 second paragraph).
    #[serde(default)]
    pub share_of_profit_recognised: Decimal,
    /// Distributions (dividends) received from the investee =
    /// `ownership_percent * investee_dividends_paid`.  Reduces the
    /// carrying amount.
    pub dividends_received: Decimal,
    /// Impairment loss recognised this period (IAS 28.40 / IAS 36).
    /// Caller supplies the amount; v5.0 has no auto-impairment.  Always
    /// non-negative.
    pub impairment: Decimal,
    /// Loss not recognised this period because the rollforward would
    /// have pushed the carrying value below zero (IAS 28.38).  Always
    /// non-negative.  Zero when the carrying value rollforward stays
    /// at or above zero.
    #[serde(default)]
    pub suppressed_loss_this_period: Decimal,
    /// Cumulative suppressed losses carried forward to next period =
    /// `(opening_suppressed_loss − recovered) + suppressed_loss_this_period`,
    /// where `recovered` is the portion of opening suppressed losses
    /// applied against current-period share of profit.  Used as next
    /// period's `opening_suppressed_loss`.
    #[serde(default)]
    pub closing_suppressed_loss: Decimal,
    /// Closing carrying value =
    /// `opening + share_of_profit_recognised - dividends_received - impairment`,
    /// rounded to 2dp and clamped at zero (IAS 28.38).
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
    /// Cumulative losses brought forward from prior periods that the
    /// investor previously suppressed under IAS 28.38.  Zero on the
    /// first period of an engagement.  Applied against the current
    /// period's `share_of_profit` before any is recognised.
    pub opening_suppressed_loss: Decimal,
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

    // IAS 28.38 second paragraph: if the investee subsequently reports
    // profits, the entity resumes recognising its share only after
    // its share of profits equals the share of losses not recognised.
    // Apply opening suppressed losses against any positive share of
    // profit BEFORE the rollforward, so the carrying value only
    // increases for the residual.
    let opening_suppressed = inputs.opening_suppressed_loss.max(Decimal::ZERO);
    let (share_of_profit_recognised, suppressed_after_recovery) =
        if share_of_profit > Decimal::ZERO && opening_suppressed > Decimal::ZERO {
            let recovered = share_of_profit.min(opening_suppressed);
            (share_of_profit - recovered, opening_suppressed - recovered)
        } else {
            (share_of_profit, opening_suppressed)
        };

    let raw_closing = (inputs.opening_carrying_value + share_of_profit_recognised
        - dividends_received
        - inputs.impairment)
        .round_dp(2);

    // IAS 28.38 / ASC 323-10-35-20: when the rollforward would push
    // the carrying amount below zero, discontinue recognising further
    // losses.  The investment is reported at zero and the unrecognised
    // amount accumulates in the suppressed-loss memorandum.  v5.1+:
    // the gap is tracked explicitly via `suppressed_loss_this_period`
    // + `closing_suppressed_loss`, restoring the IAS 28.38 second
    // paragraph (recovery against future profits).
    let (closing_carrying_value, suppressed_loss_this_period) = if raw_closing < Decimal::ZERO {
        tracing::debug!(
            investee = %investee.code,
            raw_closing = %raw_closing,
            opening = %inputs.opening_carrying_value,
            share_of_profit_recognised = %share_of_profit_recognised,
            dividends_received = %dividends_received,
            impairment = %inputs.impairment,
            "equity-method carrying value clamped at zero per IAS 28.38; suppressed loss tracked",
        );
        (Decimal::ZERO, (-raw_closing).round_dp(2))
    } else {
        (raw_closing, Decimal::ZERO)
    };

    let closing_suppressed_loss =
        (suppressed_after_recovery + suppressed_loss_this_period).round_dp(2);

    Ok(EquityMethodInvestment {
        investee_code: investee.code.clone(),
        investor_entity_code: inputs.investor_entity_code.clone(),
        ownership_percent,
        opening_carrying_value: inputs.opening_carrying_value.round_dp(2),
        opening_suppressed_loss: opening_suppressed.round_dp(2),
        share_of_profit: share_of_profit.round_dp(2),
        share_of_profit_recognised: share_of_profit_recognised.round_dp(2),
        dividends_received: dividends_received.round_dp(2),
        impairment: inputs.impairment.round_dp(2),
        suppressed_loss_this_period,
        closing_suppressed_loss,
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

/// Read prior-period closing suppressed-loss balances as this period's
/// opening suppressed losses (IAS 28.38 second paragraph).  Returns
/// `(investee_code -> closing_suppressed_loss)` for every investee in
/// the prior-period file, even when the value is zero (callers can
/// `unwrap_or(Decimal::ZERO)` if a code is absent).
///
/// Mirrors the I/O contract of
/// [`ingest_opening_equity_method_carrying_values`] — same file, just
/// reading a different field.
///
/// # Errors
///
/// - [`GroupError::Serde`] if the file exists but cannot be parsed.
/// - [`GroupError::Aggregate`] if the file contains two or more records
///   for the same `investee_code`.
/// - Missing file → `Ok(BTreeMap::new())` plus a `tracing::warn!` log.
pub fn ingest_opening_suppressed_losses(
    prior_period_dir: &Path,
) -> GroupResult<BTreeMap<String, Decimal>> {
    let path = prior_period_dir
        .join(CONSOLIDATED_SUBDIR)
        .join(EQUITY_METHOD_INVESTMENTS_FILENAME);

    if !path.exists() {
        // Mirror the ingest_opening_carrying_values behaviour: warn
        // once on first-period engagements and return empty.  No
        // separate warn here — the carrying-value ingest already
        // logs.
        return Ok(BTreeMap::new());
    }

    let bytes = fs::read(&path).map_err(GroupError::Io)?;
    let investments: Vec<EquityMethodInvestment> = serde_json::from_slice(&bytes)?;

    let mut map: BTreeMap<String, Decimal> = BTreeMap::new();
    for inv in investments {
        if map.contains_key(&inv.investee_code) {
            return Err(GroupError::Aggregate(format!(
                "ingest_opening_suppressed_losses: duplicate \
                 investee `{}` in opening file {} — writer regression?",
                inv.investee_code,
                path.display(),
            )));
        }
        map.insert(inv.investee_code, inv.closing_suppressed_loss);
    }

    Ok(map)
}

/// Write the v5.1+ IAS 28.38 suppressed-loss memorandum artefact.
///
/// Filters the input to only the records with
/// `closing_suppressed_loss > 0` so the artefact stays small and
/// readers can map it directly to the disclosure note.  Records
/// without suppressed losses don't need a disclosure entry.
///
/// Output path:
/// `{out_dir}/consolidated/equity_method_suppressed_losses.json`
///
/// # Errors
///
/// - [`GroupError::Io`] on subdirectory creation or file write failure.
/// - [`GroupError::Serde`] if serialisation fails.
pub fn write_suppressed_losses(
    investments: &[EquityMethodInvestment],
    out_dir: &Path,
) -> GroupResult<PathBuf> {
    let dir = out_dir.join(CONSOLIDATED_SUBDIR);
    fs::create_dir_all(&dir).map_err(GroupError::Io)?;

    let path = dir.join(EQUITY_METHOD_SUPPRESSED_LOSSES_FILENAME);

    let filtered: Vec<&EquityMethodInvestment> = investments
        .iter()
        .filter(|i| i.closing_suppressed_loss > Decimal::ZERO)
        .collect();

    let mut json = serde_json::to_string_pretty(&filtered)?;
    json.push('\n');
    fs::write(&path, json).map_err(GroupError::Io)?;

    Ok(path)
}

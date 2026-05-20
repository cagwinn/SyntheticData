//! Hyperinflationary-economy accounting under IAS 29 / ASC 830.
//!
//! When an entity's functional currency is the currency of a
//! hyperinflationary economy, IAS 29 requires the entity to restate
//! its non-monetary items using a general price index so the
//! financial statements are stated in terms of the **measuring unit
//! current at the end of the reporting period**.  v5.0 / v5.1 did not
//! handle this — every entity went through the standard IAS 21
//! translation path which produces nonsense results once cumulative
//! inflation crosses ~100 %.
//!
//! v5.2 ships the typed model + arithmetic helpers; the integration
//! with the IAS 21 translation pipeline (`translate_entity_tb` /
//! `cta_rollforward`) is a follow-up.
//!
//! # Standards reference
//!
//! - **IAS 29 § 3** — characteristics of a hyperinflationary economy
//!   (cumulative 3-year inflation approaching or exceeding 100 %; the
//!   general population prefers a stable foreign currency to keep its
//!   wealth; etc.).  IAS 29 does not establish an absolute rate at
//!   which hyperinflation is deemed to arise — it's a matter of
//!   judgement.
//! - **IAS 29 § 8** — the financial statements **shall be stated in
//!   terms of the measuring unit current at the end of the reporting
//!   period**.  Comparative figures are also restated.
//! - **IAS 29 § 12** — non-monetary items carried at historical cost
//!   are restated by applying the change in the general price index
//!   between the date of acquisition (or revaluation) and the
//!   reporting date.
//! - **IAS 29 § 13** — non-monetary items at current value (e.g.
//!   inventories at NRV) are NOT restated (already at current
//!   measuring unit).
//! - **IAS 29 § 27** — the **net gain or loss on the net monetary
//!   position** is included in profit or loss for the period.  It
//!   represents the loss of purchasing power on monetary items
//!   (cash, receivables, payables).
//! - **IAS 21 § 39 / IAS 29 § 33** — when restated financial
//!   statements of a hyperinflationary subsidiary are translated
//!   into the group's (non-hyperinflationary) presentation
//!   currency, the **closing rate** is used for ALL items (not the
//!   spot/average split that IAS 21 normally prescribes).
//!
//! # Scope
//!
//! v5.2 ships:
//!
//! - [`HyperinflationStatus`] entity-level flag
//! - [`GeneralPriceIndex`] CPI series + index lookup
//! - [`IndexedRestatement`] line-item restatement record
//! - [`NetMonetaryPositionGainLoss`] helper for the IAS 29 § 27
//!   purchasing-power gain/loss calc
//!
//! The wiring to actually drive these through `translate_entity_tb`
//! is a follow-up tracked in the README.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Hyperinflation status of an entity's functional currency.
/// Captured per-entity per-period because a country can transition
/// in or out of hyperinflation across reporting cycles (e.g.
/// Argentina entered hyperinflationary status in 2018 per IAS 29
/// criteria; Türkiye did so in 2022).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HyperinflationStatus {
    /// The functional currency is **not** hyperinflationary.
    /// Standard IAS 21 translation applies.
    #[default]
    NotHyperinflationary,
    /// The functional currency **is** hyperinflationary; IAS 29
    /// restatement applies before IAS 21 translation per IAS 21 § 43.
    /// The closing rate is used for all items per IAS 21 § 42(b).
    Hyperinflationary,
}

impl HyperinflationStatus {
    /// Returns `true` when this status requires IAS 29 restatement
    /// before IAS 21 translation.
    pub fn requires_restatement(&self) -> bool {
        matches!(self, Self::Hyperinflationary)
    }
}

/// Time series of general-price-index (CPI) observations for a
/// hyperinflationary economy.  The index is monotonically
/// non-decreasing in normal use; the lookup helpers tolerate
/// out-of-order dates by sorting on access.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GeneralPriceIndex {
    /// ISO 4217 currency code the index applies to (e.g. "ARS",
    /// "TRY").  Joins to entity functional currency for lookup.
    pub currency: String,

    /// Source / methodology label (e.g. "INDEC IPC General",
    /// "TÜİK CPI").  Carried for audit-trail purposes.
    pub source: String,

    /// Observed (date, index level) pairs.  Index level convention:
    /// any positive [`Decimal`] — the helpers compute relative
    /// indexation factors (`new / old`), so absolute scale is free.
    pub observations: Vec<(NaiveDate, Decimal)>,
}

impl GeneralPriceIndex {
    /// Construct an empty index for `currency` from a labelled
    /// source.
    pub fn new(currency: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            currency: currency.into(),
            source: source.into(),
            observations: Vec::new(),
        }
    }

    /// Append an observation.  Caller is responsible for ordering;
    /// [`Self::lookup`] sorts on access.
    pub fn observe(&mut self, date: NaiveDate, level: Decimal) -> &mut Self {
        self.observations.push((date, level));
        self
    }

    /// Look up the index level for a date.  Returns the level on the
    /// **most recent observation at or before** `date`, mirroring
    /// the conservative IAS 29 convention of using the latest
    /// available CPI for each measurement date.  Returns `None` when
    /// no observation exists at or before the date.
    pub fn lookup(&self, date: NaiveDate) -> Option<Decimal> {
        let mut sorted: Vec<&(NaiveDate, Decimal)> = self.observations.iter().collect();
        sorted.sort_by_key(|(d, _)| *d);
        sorted
            .iter()
            .rev()
            .find(|(d, _)| *d <= date)
            .map(|(_, level)| *level)
    }

    /// Compute the IAS 29 § 12 indexation factor for restating a
    /// historical-cost amount from `from_date` to `to_date`:
    ///
    /// `factor = index(to_date) / index(from_date)`
    ///
    /// Returns `None` when either lookup misses, or when the
    /// `from_date` index is zero (would otherwise divide-by-zero).
    pub fn indexation_factor(&self, from_date: NaiveDate, to_date: NaiveDate) -> Option<Decimal> {
        let from = self.lookup(from_date)?;
        let to = self.lookup(to_date)?;
        if from.is_zero() {
            None
        } else {
            Some(to / from)
        }
    }
}

/// One line-item restatement under IAS 29 § 12.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IndexedRestatement {
    /// Account code being restated.
    pub account_code: String,

    /// The historical date on which the underlying item was
    /// recognised (acquisition date for non-monetary items).
    pub historical_date: NaiveDate,

    /// Reporting period end the restatement is being made for.
    pub reporting_date: NaiveDate,

    /// Pre-restatement (historical-cost) carrying amount, in the
    /// functional currency.
    #[serde(with = "crate::serde_decimal")]
    pub historical_amount: Decimal,

    /// Indexation factor applied =
    /// `index(reporting_date) / index(historical_date)`.
    #[serde(with = "crate::serde_decimal")]
    pub indexation_factor: Decimal,

    /// Post-restatement amount =
    /// `historical_amount * indexation_factor`.
    #[serde(with = "crate::serde_decimal")]
    pub restated_amount: Decimal,

    /// Functional currency code.
    pub currency: String,
}

impl IndexedRestatement {
    /// Restate `historical_amount` from `historical_date` to
    /// `reporting_date` using the supplied [`GeneralPriceIndex`].
    /// Returns `None` when the index can't yield a factor for either
    /// date.  Pure projection — no I/O.
    pub fn restate(
        account_code: impl Into<String>,
        historical_date: NaiveDate,
        reporting_date: NaiveDate,
        historical_amount: Decimal,
        index: &GeneralPriceIndex,
    ) -> Option<Self> {
        let factor = index.indexation_factor(historical_date, reporting_date)?;
        Some(Self {
            account_code: account_code.into(),
            historical_date,
            reporting_date,
            historical_amount,
            indexation_factor: factor,
            restated_amount: (historical_amount * factor).round_dp(2),
            currency: index.currency.clone(),
        })
    }

    /// Restatement adjustment amount =
    /// `restated_amount − historical_amount`.  Positive when the
    /// asset's measured value increased (general inflation outpaces
    /// historical book value); negative when the index has fallen
    /// (rare — typically only happens with a base-period reset).
    pub fn adjustment(&self) -> Decimal {
        self.restated_amount - self.historical_amount
    }
}

/// IAS 29 § 27 net-monetary-position gain or loss for a period.
///
/// Monetary items (cash, receivables, payables) lose purchasing
/// power as the general price level rises.  The gain or loss is
/// recognised in P&L for the period.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetMonetaryPositionGainLoss {
    /// Reporting period end.
    pub reporting_date: NaiveDate,

    /// Opening net monetary position (monetary assets less
    /// monetary liabilities) at the start of the period, in the
    /// functional currency.
    #[serde(with = "crate::serde_decimal")]
    pub opening_net_monetary_position: Decimal,

    /// Closing net monetary position at `reporting_date`.
    #[serde(with = "crate::serde_decimal")]
    pub closing_net_monetary_position: Decimal,

    /// Indexation factor for the period =
    /// `index(reporting_date) / index(opening_date)`.
    #[serde(with = "crate::serde_decimal")]
    pub period_indexation_factor: Decimal,

    /// Computed gain or loss on the net monetary position.  Sign
    /// convention: a **loss** (negative number) when the entity is a
    /// net holder of monetary assets in a rising-price environment
    /// (the typical case in hyperinflation).  A **gain** (positive)
    /// arises when the entity is a net debtor — its monetary
    /// liabilities lose purchasing power.
    #[serde(with = "crate::serde_decimal")]
    pub gain_or_loss: Decimal,

    /// Functional currency code.
    pub currency: String,
}

impl NetMonetaryPositionGainLoss {
    /// Compute the IAS 29 § 27 gain/loss using the simplified
    /// **opening-balance restatement** approach:
    ///
    /// `gain_or_loss = closing_net_monetary − (opening_net_monetary × factor)`
    ///
    /// A more rigorous calculation would index every monetary
    /// transaction during the period — that's out of scope for v5.2's
    /// model layer; the wiring layer can refine the input
    /// aggregation later.
    pub fn compute(
        reporting_date: NaiveDate,
        opening_net_monetary_position: Decimal,
        closing_net_monetary_position: Decimal,
        period_indexation_factor: Decimal,
        currency: impl Into<String>,
    ) -> Self {
        let restated_opening = opening_net_monetary_position * period_indexation_factor;
        let gain_or_loss = (closing_net_monetary_position - restated_opening).round_dp(2);
        Self {
            reporting_date,
            opening_net_monetary_position: opening_net_monetary_position.round_dp(2),
            closing_net_monetary_position: closing_net_monetary_position.round_dp(2),
            period_indexation_factor,
            gain_or_loss,
            currency: currency.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn open_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
    }
    fn mid_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 6, 30).unwrap()
    }
    fn close_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()
    }

    fn ars_index() -> GeneralPriceIndex {
        let mut idx = GeneralPriceIndex::new("ARS", "INDEC IPC General");
        idx.observe(open_date(), dec!(100));
        idx.observe(mid_date(), dec!(160));
        idx.observe(close_date(), dec!(220));
        idx
    }

    #[test]
    fn status_requires_restatement_only_when_hyperinflationary() {
        assert!(!HyperinflationStatus::NotHyperinflationary.requires_restatement());
        assert!(HyperinflationStatus::Hyperinflationary.requires_restatement());
    }

    #[test]
    fn index_lookup_returns_most_recent_at_or_before_date() {
        let idx = ars_index();
        // Exact match.
        assert_eq!(idx.lookup(open_date()), Some(dec!(100)));
        assert_eq!(idx.lookup(close_date()), Some(dec!(220)));
        // Between observations: returns the prior observation.
        let between = NaiveDate::from_ymd_opt(2024, 9, 15).unwrap();
        assert_eq!(idx.lookup(between), Some(dec!(160)));
        // Before the first observation: None.
        let earlier = NaiveDate::from_ymd_opt(2023, 12, 31).unwrap();
        assert_eq!(idx.lookup(earlier), None);
    }

    #[test]
    fn index_lookup_handles_unsorted_observations() {
        // Insert in reverse order; lookup must still pick the
        // chronologically most recent ≤ target.
        let mut idx = GeneralPriceIndex::new("ARS", "INDEC IPC General");
        idx.observe(close_date(), dec!(220));
        idx.observe(open_date(), dec!(100));
        idx.observe(mid_date(), dec!(160));
        assert_eq!(idx.lookup(mid_date()), Some(dec!(160)));
    }

    #[test]
    fn indexation_factor_is_ratio_of_indices() {
        let idx = ars_index();
        // open → close: 220 / 100 = 2.2.
        let factor = idx.indexation_factor(open_date(), close_date()).unwrap();
        assert_eq!(factor, dec!(2.2));
        // mid → close: 220 / 160 = 1.375.
        let factor = idx.indexation_factor(mid_date(), close_date()).unwrap();
        assert_eq!(factor, dec!(1.375));
    }

    #[test]
    fn indexation_factor_returns_none_on_missing_data() {
        let idx = ars_index();
        let pre_index = NaiveDate::from_ymd_opt(2023, 1, 1).unwrap();
        // Pre-index date returns None for the from-side.
        assert_eq!(idx.indexation_factor(pre_index, close_date()), None);
    }

    #[test]
    fn restate_applies_factor_to_historical_amount() {
        let idx = ars_index();
        let r = IndexedRestatement::restate(
            "1500", // PP&E
            open_date(),
            close_date(),
            dec!(1_000_000),
            &idx,
        )
        .unwrap();
        assert_eq!(r.indexation_factor, dec!(2.2));
        assert_eq!(r.restated_amount, dec!(2_200_000.00));
        assert_eq!(r.adjustment(), dec!(1_200_000.00));
        assert_eq!(r.currency, "ARS");
    }

    #[test]
    fn restate_returns_none_when_factor_unavailable() {
        let idx = ars_index();
        let pre_index = NaiveDate::from_ymd_opt(2023, 1, 1).unwrap();
        assert!(IndexedRestatement::restate(
            "1500",
            pre_index,
            close_date(),
            dec!(1_000_000),
            &idx
        )
        .is_none());
    }

    #[test]
    fn net_monetary_loss_for_a_net_holder_of_cash() {
        // Entity is a net holder of monetary assets.  Opening net =
        // 100k, closing net = 180k after a year of 120% inflation
        // (factor 2.2).  Restated opening = 100k × 2.2 = 220k.
        // Loss = closing 180k − restated 220k = −40k (loss in P&L).
        let result = NetMonetaryPositionGainLoss::compute(
            close_date(),
            dec!(100_000),
            dec!(180_000),
            dec!(2.2),
            "ARS",
        );
        assert_eq!(result.gain_or_loss, dec!(-40_000.00));
    }

    #[test]
    fn net_monetary_gain_for_a_net_debtor() {
        // Entity is a net debtor (negative net monetary position).
        // Opening = −500k, closing = −540k, factor 2.2.  Restated
        // opening = −500k × 2.2 = −1.1M.  Gain = −540k − (−1.1M) =
        // +560k (purchasing power gain on debt).
        let result = NetMonetaryPositionGainLoss::compute(
            close_date(),
            dec!(-500_000),
            dec!(-540_000),
            dec!(2.2),
            "ARS",
        );
        assert_eq!(result.gain_or_loss, dec!(560_000.00));
    }

    #[test]
    fn round_trips_serialise_for_audit_evidence() {
        let idx = ars_index();
        let json = serde_json::to_string(&idx).unwrap();
        let back: GeneralPriceIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(back, idx);

        let r =
            IndexedRestatement::restate("1500", open_date(), close_date(), dec!(1_000_000), &idx)
                .unwrap();
        let json = serde_json::to_string(&r).unwrap();
        let back: IndexedRestatement = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);

        let gl = NetMonetaryPositionGainLoss::compute(
            close_date(),
            dec!(100_000),
            dec!(180_000),
            dec!(2.2),
            "ARS",
        );
        let json = serde_json::to_string(&gl).unwrap();
        let back: NetMonetaryPositionGainLoss = serde_json::from_str(&json).unwrap();
        assert_eq!(back, gl);
    }
}

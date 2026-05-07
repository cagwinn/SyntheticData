//! IAS 29 § 12 indexed restatement of trial-balance amounts.
//!
//! When an entity's functional currency is the currency of a
//! hyperinflationary economy, IAS 29 § 8 requires the financial
//! statements to be **stated in terms of the measuring unit current
//! at the end of the reporting period** before any IAS 21 translation
//! is performed.  In practice this means each amount on the
//! pre-translation TB is multiplied by a *general price index factor*
//! reflecting the change in purchasing power between the date the
//! amount was originally recorded and the period-end date.
//!
//! # Three classes of amounts (IAS 29 § 12, § 13, § 26)
//!
//! | Account class                          | Restatement factor                              |
//! |----------------------------------------|------------------------------------------------|
//! | Monetary BS items (cash, AR, AP, debt) | **1.0** — already at period-end measuring unit |
//! | Non-monetary BS items + equity         | `closing_index / opening_index`                 |
//! | Income statement items (P&L + OCI)     | `closing_index / average_index`                 |
//!
//! A precise IAS 29 implementation would restate each non-monetary
//! item by `closing_index / index_at_acquisition_date`, requiring an
//! acquisition-date stamp on every PPE / inventory / goodwill record.
//! The synthetic-data engine instead applies a single *period-level*
//! factor — `closing_index / opening_index` — which captures the
//! systemic effect of restatement (purchasing-power loss across the
//! period) without per-item provenance.  This is the same
//! simplification the spec calls out: see
//! `docs/superpowers/specs/2026-04-23-group-audit-simulation-design.md`
//! §"IAS 29 restatement" for the design discussion.
//!
//! # Composition with IAS 21 § 42(b)
//!
//! IAS 21 § 42(b) further requires the restated amounts to be
//! translated at the **closing rate**.  The two steps compose:
//!
//! ```text
//! presentation_amount = local_amount
//!                     × restatement_factor    (IAS 29 § 12)
//!                     × closing_rate          (IAS 21 § 42(b))
//! ```
//!
//! [`crate::aggregate::translation::translate::translate_entity_tb_with_indexed_restatement`]
//! applies both steps; the simpler
//! [`crate::aggregate::translation::translate::translate_entity_tb_with_hyperinflation`]
//! delegates with `restatement = None` to skip § 12 while still
//! honouring § 42(b).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::aggregate::translation::classify::TranslationAccountType;
use datasynth_core::models::hyperinflation::{GeneralPriceIndex, HyperinflationStatus};

/// Period-level general price indices used for IAS 29 § 12 restatement.
///
/// All three values are positive Decimals.  The opening / closing /
/// average semantics match the IAS 29 § 12 / § 26 contract:
///
/// - `opening_index` — value of the general price index at the start
///   of the reporting period (proxy for the index on dates when
///   non-monetary items were originally recognised; the simplification
///   discussed in the module docs).
/// - `closing_index` — value at the end of the reporting period; the
///   measuring unit current at the balance sheet date that all amounts
///   are restated *to*.
/// - `average_index` — average value over the reporting period; the
///   measuring unit appropriate for income-statement items per IAS 29
///   § 26.
///
/// All factors are derived; the type carries the raw indices so callers
/// can audit the inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedRestatement {
    /// General price index at the start of the period.
    #[serde(with = "datasynth_core::serde_decimal")]
    pub opening_index: Decimal,
    /// General price index at the end of the period — the measuring
    /// unit all amounts are restated *to*.
    #[serde(with = "datasynth_core::serde_decimal")]
    pub closing_index: Decimal,
    /// Average general price index over the period — used for income
    /// statement / OCI restatement per IAS 29 § 26.
    #[serde(with = "datasynth_core::serde_decimal")]
    pub average_index: Decimal,
}

impl IndexedRestatement {
    /// Construct a restatement record after validating that all three
    /// indices are strictly positive.  Returns the validation message
    /// as a `String` so the caller can attribute it to a specific
    /// entity / period.
    pub fn new(
        opening_index: Decimal,
        closing_index: Decimal,
        average_index: Decimal,
    ) -> Result<Self, String> {
        if opening_index <= Decimal::ZERO {
            return Err(format!(
                "IndexedRestatement::new: opening_index must be > 0, got {opening_index}"
            ));
        }
        if closing_index <= Decimal::ZERO {
            return Err(format!(
                "IndexedRestatement::new: closing_index must be > 0, got {closing_index}"
            ));
        }
        if average_index <= Decimal::ZERO {
            return Err(format!(
                "IndexedRestatement::new: average_index must be > 0, got {average_index}"
            ));
        }
        Ok(Self {
            opening_index,
            closing_index,
            average_index,
        })
    }

    /// Restatement factor for non-monetary balance-sheet items + equity:
    /// `closing_index / opening_index`.
    ///
    /// Greater than 1.0 in a hyperinflationary economy (rising prices →
    /// historical-cost amounts must be scaled up to remain comparable).
    pub fn non_monetary_factor(&self) -> Decimal {
        self.closing_index / self.opening_index
    }

    /// Restatement factor for income-statement items (revenue, expense,
    /// OCI): `closing_index / average_index`.  Smaller than the
    /// non-monetary factor when prices rise monotonically through the
    /// period.
    pub fn pl_factor(&self) -> Decimal {
        self.closing_index / self.average_index
    }

    /// Pick the appropriate factor for an account class.  Monetary BS
    /// items pass through unchanged (factor = 1).
    pub fn factor_for(&self, account_type: TranslationAccountType) -> Decimal {
        match account_type {
            TranslationAccountType::BsMonetary => Decimal::ONE,
            TranslationAccountType::BsNonMonetary | TranslationAccountType::Equity => {
                self.non_monetary_factor()
            }
            TranslationAccountType::PlRevenue
            | TranslationAccountType::PlExpense
            | TranslationAccountType::PlOci => self.pl_factor(),
        }
    }

    /// **v5.5.2** — Build an [`IndexedRestatement`] from a
    /// [`GeneralPriceIndex`] CPI series for a given engagement period.
    ///
    /// Derivation rules:
    /// - `opening_index` = `series.lookup(period_start)`
    /// - `closing_index` = `series.lookup(period_end)`
    /// - `average_index` = arithmetic mean of every observation whose
    ///   date falls within `[period_start, period_end]`. When no
    ///   in-period observations exist, falls back to the midpoint
    ///   `(opening + closing) / 2` so the restatement record is always
    ///   well-defined when both endpoints are looked up successfully.
    ///
    /// Returns `None` (rather than `Err`) when either endpoint lookup
    /// misses — the calling driver treats this as "no CPI data for this
    /// entity in this period" and falls back to the closing-rate path
    /// per IAS 21 § 42(b) only.
    pub fn from_cpi_series(
        series: &GeneralPriceIndex,
        period_start: NaiveDate,
        period_end: NaiveDate,
    ) -> Option<Self> {
        let opening_index = series.lookup(period_start)?;
        let closing_index = series.lookup(period_end)?;

        // Derive the period average from every observation that
        // falls inside [start, end] (inclusive on both ends).  When
        // the series has no observations in the period, we fall back
        // to the midpoint of opening / closing so the record always
        // has a valid average.
        let in_period: Vec<Decimal> = series
            .observations
            .iter()
            .filter(|(d, _)| *d >= period_start && *d <= period_end)
            .map(|(_, level)| *level)
            .collect();
        let average_index = if in_period.is_empty() {
            (opening_index + closing_index) / Decimal::from(2u32)
        } else {
            let sum: Decimal = in_period.iter().copied().sum();
            sum / Decimal::from(in_period.len() as u64)
        };

        // Reuse the constructor for index validation (positive only).
        Self::new(opening_index, closing_index, average_index).ok()
    }
}

/// **v5.5.2** — Which IAS 21 / IAS 29 translation path the aggregate
/// driver should take for a given entity.  Returned by
/// [`select_restatement_path`] so the driver routing is testable
/// without hitting the full translation pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestatementPath {
    /// Standard IAS 21 multi-rate translation — used for every
    /// non-hyperinflationary entity.
    Standard,
    /// IAS 21 § 42(b) closing-rate-for-all-items — used when the
    /// entity is hyperinflationary but no CPI series matched (the
    /// pre-v5.5.2 behaviour).
    ClosingRate,
    /// Full IAS 29 § 12 indexed restatement composed with IAS 21 §
    /// 42(b) closing-rate translation — used when the entity is
    /// hyperinflationary **and** a matching CPI series produced a
    /// well-defined `IndexedRestatement`.
    Indexed(IndexedRestatement),
}

/// **v5.5.2** — Pick the IAS 21 / IAS 29 translation path for one
/// entity given its hyperinflation status, functional currency, and
/// the optional CPI series map.
///
/// Routing logic:
/// - `NotHyperinflationary` → [`RestatementPath::Standard`]
/// - `Hyperinflationary` with no map provided **or** no matching
///   currency in the map → [`RestatementPath::ClosingRate`] (caller
///   should log a warning at the driver level so the operator can
///   notice missing data).
/// - `Hyperinflationary` with a matching series whose lookups
///   succeed → [`RestatementPath::Indexed`].
/// - `Hyperinflationary` with a matching series whose lookups miss
///   (period start/end falls before the earliest observation) →
///   [`RestatementPath::ClosingRate`] (degenerate series — caller
///   should warn).
pub fn select_restatement_path(
    hyperinflation: HyperinflationStatus,
    functional_currency: &str,
    cpi_series_by_currency: Option<&std::collections::BTreeMap<String, GeneralPriceIndex>>,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> RestatementPath {
    if !hyperinflation.requires_restatement() {
        return RestatementPath::Standard;
    }
    let Some(map) = cpi_series_by_currency else {
        return RestatementPath::ClosingRate;
    };
    let Some(series) = map.get(functional_currency) else {
        return RestatementPath::ClosingRate;
    };
    match IndexedRestatement::from_cpi_series(series, period_start, period_end) {
        Some(ir) => RestatementPath::Indexed(ir),
        None => RestatementPath::ClosingRate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn new_rejects_non_positive_indices() {
        assert!(IndexedRestatement::new(dec!(0), dec!(110), dec!(105)).is_err());
        assert!(IndexedRestatement::new(dec!(100), dec!(-1), dec!(105)).is_err());
        assert!(IndexedRestatement::new(dec!(100), dec!(110), dec!(0)).is_err());
    }

    #[test]
    fn factors_compute_from_indices() {
        // Period-end index 200 vs opening 100 → 2.0× restatement for
        // non-monetary; average 150 → 200/150 ≈ 1.333× for P&L.
        let ir = IndexedRestatement::new(dec!(100), dec!(200), dec!(150)).unwrap();
        assert_eq!(ir.non_monetary_factor(), dec!(2));
        assert_eq!(ir.pl_factor(), dec!(200) / dec!(150));
    }

    #[test]
    fn factor_for_dispatches_by_account_type() {
        let ir = IndexedRestatement::new(dec!(100), dec!(200), dec!(150)).unwrap();
        assert_eq!(ir.factor_for(TranslationAccountType::BsMonetary), dec!(1));
        assert_eq!(
            ir.factor_for(TranslationAccountType::BsNonMonetary),
            dec!(2)
        );
        assert_eq!(ir.factor_for(TranslationAccountType::Equity), dec!(2));
        assert_eq!(
            ir.factor_for(TranslationAccountType::PlRevenue),
            dec!(200) / dec!(150)
        );
        assert_eq!(
            ir.factor_for(TranslationAccountType::PlExpense),
            dec!(200) / dec!(150)
        );
        assert_eq!(
            ir.factor_for(TranslationAccountType::PlOci),
            dec!(200) / dec!(150)
        );
    }

    #[test]
    fn unit_factors_when_all_indices_equal() {
        // Stable economy: all factors = 1 → restatement is a no-op.
        let ir = IndexedRestatement::new(dec!(100), dec!(100), dec!(100)).unwrap();
        for ty in [
            TranslationAccountType::BsMonetary,
            TranslationAccountType::BsNonMonetary,
            TranslationAccountType::Equity,
            TranslationAccountType::PlRevenue,
            TranslationAccountType::PlExpense,
            TranslationAccountType::PlOci,
        ] {
            assert_eq!(ir.factor_for(ty), dec!(1), "{ty:?}");
        }
    }

    #[test]
    fn json_round_trip() {
        let ir = IndexedRestatement::new(dec!(123.45), dec!(678.90), dec!(401.17)).unwrap();
        let json = serde_json::to_string(&ir).unwrap();
        let back: IndexedRestatement = serde_json::from_str(&json).unwrap();
        assert_eq!(ir, back);
    }

    // ── v5.5.2: CPI-driven IndexedRestatement + select_restatement_path ────

    fn ars_index_q1() -> GeneralPriceIndex {
        // Argentina-style series spanning Q1 2024.  Strictly increasing
        // — the period average should land between opening and closing.
        let mut idx = GeneralPriceIndex::new("ARS", "INDEC IPC General (test)");
        idx.observe(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(), dec!(100));
        idx.observe(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(), dec!(120));
        idx.observe(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(), dec!(150));
        idx.observe(NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(), dec!(180));
        idx
    }

    #[test]
    fn from_cpi_series_uses_endpoints_and_in_period_average() {
        let idx = ars_index_q1();
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 3, 31).unwrap();
        let ir = IndexedRestatement::from_cpi_series(&idx, start, end).unwrap();

        assert_eq!(ir.opening_index, dec!(100));
        assert_eq!(ir.closing_index, dec!(180));
        // (100 + 120 + 150 + 180) / 4 = 137.5
        assert_eq!(ir.average_index, dec!(137.5));
    }

    #[test]
    fn from_cpi_series_falls_back_to_midpoint_when_no_in_period_obs() {
        // Series with observations only outside the period — average
        // should fall back to (opening + closing) / 2.  Lookup uses
        // the most recent obs at or before the date, so we still have
        // valid endpoints.
        let mut idx = GeneralPriceIndex::new("TRY", "test");
        idx.observe(NaiveDate::from_ymd_opt(2023, 12, 1).unwrap(), dec!(100));
        idx.observe(NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(), dec!(200));

        // Period entirely between the two observations: lookup at
        // both endpoints returns the prior observation (100).
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 3, 31).unwrap();
        let ir = IndexedRestatement::from_cpi_series(&idx, start, end).unwrap();
        assert_eq!(ir.opening_index, dec!(100));
        assert_eq!(ir.closing_index, dec!(100));
        // No in-period obs → midpoint = (100 + 100) / 2 = 100
        assert_eq!(ir.average_index, dec!(100));
    }

    #[test]
    fn from_cpi_series_returns_none_when_no_obs_at_or_before_start() {
        let mut idx = GeneralPriceIndex::new("TRY", "test");
        idx.observe(NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(), dec!(200));
        // Period start is before any observation → opening lookup
        // misses → None.
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 3, 31).unwrap();
        assert!(IndexedRestatement::from_cpi_series(&idx, start, end).is_none());
    }

    #[test]
    fn select_restatement_path_non_hyperinflationary_is_standard() {
        let path = select_restatement_path(
            HyperinflationStatus::NotHyperinflationary,
            "USD",
            None,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
        );
        assert_eq!(path, RestatementPath::Standard);
    }

    #[test]
    fn select_restatement_path_hyperinflationary_no_map_is_closing_rate() {
        let path = select_restatement_path(
            HyperinflationStatus::Hyperinflationary,
            "ARS",
            None,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
        );
        assert_eq!(path, RestatementPath::ClosingRate);
    }

    #[test]
    fn select_restatement_path_hyperinflationary_no_match_is_closing_rate() {
        let mut map: std::collections::BTreeMap<String, GeneralPriceIndex> =
            std::collections::BTreeMap::new();
        map.insert("TRY".to_string(), ars_index_q1());
        let path = select_restatement_path(
            HyperinflationStatus::Hyperinflationary,
            "ARS",
            Some(&map),
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
        );
        assert_eq!(path, RestatementPath::ClosingRate);
    }

    #[test]
    fn select_restatement_path_hyperinflationary_match_is_indexed() {
        let mut map: std::collections::BTreeMap<String, GeneralPriceIndex> =
            std::collections::BTreeMap::new();
        map.insert("ARS".to_string(), ars_index_q1());
        let path = select_restatement_path(
            HyperinflationStatus::Hyperinflationary,
            "ARS",
            Some(&map),
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
        );
        match path {
            RestatementPath::Indexed(ir) => {
                assert_eq!(ir.opening_index, dec!(100));
                assert_eq!(ir.closing_index, dec!(180));
            }
            other => panic!("expected Indexed; got {other:?}"),
        }
    }

    #[test]
    fn select_restatement_path_hyperinflationary_match_but_lookup_miss_is_closing_rate() {
        // Series only has observations in the future → from_cpi_series
        // returns None → path should fall back to ClosingRate.
        let mut idx = GeneralPriceIndex::new("ARS", "test");
        idx.observe(NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(), dec!(200));
        let mut map: std::collections::BTreeMap<String, GeneralPriceIndex> =
            std::collections::BTreeMap::new();
        map.insert("ARS".to_string(), idx);
        let path = select_restatement_path(
            HyperinflationStatus::Hyperinflationary,
            "ARS",
            Some(&map),
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
        );
        assert_eq!(path, RestatementPath::ClosingRate);
    }

    #[test]
    fn cpi_series_json_round_trip() {
        // Verify GeneralPriceIndex (the type the CLI deserialises
        // from JSON) round-trips cleanly through serde.  This is the
        // shape consumed by `--cpi-series <path>`.
        let idx = ars_index_q1();
        let series = vec![idx];
        let json = serde_json::to_string(&series).unwrap();
        let back: Vec<GeneralPriceIndex> = serde_json::from_str(&json).unwrap();
        assert_eq!(series, back);
        assert_eq!(back[0].currency, "ARS");
        assert_eq!(back[0].observations.len(), 4);
    }
}

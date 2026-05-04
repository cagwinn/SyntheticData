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

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::aggregate::translation::classify::TranslationAccountType;

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
}

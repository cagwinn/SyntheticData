//! Business Combination models (IFRS 3 / ASC 805).
//!
//! Provides data structures for:
//! - Acquisition consideration (cash, shares, contingent)
//! - Purchase price allocation (PPA) with fair value adjustments
//! - Goodwill computation
//! - Day 1 journal entries and subsequent amortization of acquired intangibles

use crate::models::intercompany::NciMeasurementMethod;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A business combination transaction representing an acquisition accounted for
/// under IFRS 3 (acquisition method) or ASC 805.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusinessCombination {
    /// Unique identifier for this acquisition
    pub id: String,

    /// Company code of the acquirer entity
    pub acquirer_entity: String,

    /// Name of the acquiree (target company)
    pub acquiree_name: String,

    /// Entity code of the acquiree (the subsidiary in the group's
    /// consolidation graph after the acquisition).  When set, the
    /// acquisition can be matched to the corresponding `ManifestEntity`
    /// in the v5.x group manifest so the NCI rollforward can pick up
    /// the IFRS 3.19 measurement choice automatically.  Optional —
    /// `None` means the acquisition has no consolidated counterpart
    /// (asset purchase, fully owned, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acquiree_entity_code: Option<String>,

    /// Date control was obtained (acquisition date)
    pub acquisition_date: NaiveDate,

    /// Total consideration paid or transferred
    pub consideration: AcquisitionConsideration,

    /// Purchase price allocation at acquisition date
    pub purchase_price_allocation: AcquisitionPpa,

    /// Goodwill recognised (consideration minus net identifiable assets at FV).
    /// Zero when consideration < net identifiable assets (bargain purchase).
    #[serde(with = "crate::serde_decimal")]
    pub goodwill: Decimal,

    /// IFRS 3 § 19 / ASC 805-30-30-1 acquisition-date NCI measurement
    /// method.  Defaults to `Proportionate` (matches the v5.0–v5.1
    /// behaviour where every acquisition was effectively measured on
    /// the proportionate basis).  US GAAP requires `FullGoodwill`;
    /// IFRS allows either on a per-acquisition basis.
    #[serde(default)]
    pub nci_measurement_method: NciMeasurementMethod,

    /// IFRS 3 § 19(a) acquisition-date NCI fair value.  Required when
    /// `nci_measurement_method == FullGoodwill`; optional otherwise
    /// (carried for disclosure purposes under either method).  When
    /// supplied this becomes the period-1 opening NCI for the
    /// acquired subsidiary in the group consolidation rollforward.
    #[serde(default, with = "crate::serde_decimal::option")]
    pub acquisition_date_nci_fair_value: Option<Decimal>,

    /// Accounting framework applied: "IFRS" or "US_GAAP"
    pub framework: String,
}

impl BusinessCombination {
    /// Resolve the acquisition-date NCI fair value to feed into the
    /// group consolidation rollforward.  Returns `Some(fv)` when the
    /// caller has supplied one; `None` otherwise — in which case the
    /// rollforward falls back to the v5.0–v5.1 proportionate-basis
    /// behaviour (opening NCI = 0, grows via share of profit).
    ///
    /// Use from `run_aggregate` when wiring per-acquisition NCI
    /// measurement choices into the period-1 opening of the
    /// `compute_nci_rollforward` call.
    pub fn nci_opening_fair_value(&self) -> Option<Decimal> {
        self.acquisition_date_nci_fair_value
    }
}

/// Consideration transferred in a business combination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcquisitionConsideration {
    /// Cash and cash equivalents paid
    #[serde(with = "crate::serde_decimal")]
    pub cash: Decimal,

    /// Fair value of equity instruments issued by the acquirer
    #[serde(default, with = "crate::serde_decimal::option")]
    pub shares_issued_value: Option<Decimal>,

    /// Fair value of contingent consideration (earn-out) at acquisition date
    #[serde(default, with = "crate::serde_decimal::option")]
    pub contingent_consideration: Option<Decimal>,

    /// Total consideration (sum of cash + shares + contingent)
    #[serde(with = "crate::serde_decimal")]
    pub total: Decimal,
}

/// Purchase price allocation mapping the consideration to identifiable
/// net assets at fair value, with the residual as goodwill.
///
/// Named `AcquisitionPpa` (not `PurchasePriceAllocation`) to avoid
/// a name collision with the same-named struct in `organizational_event`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcquisitionPpa {
    /// Identifiable assets acquired at fair value
    pub identifiable_assets: Vec<AcquisitionFvAdjustment>,

    /// Identifiable liabilities assumed at fair value
    pub identifiable_liabilities: Vec<AcquisitionFvAdjustment>,

    /// Net identifiable assets at fair value
    /// = sum(asset FVs) - sum(liability FVs)
    #[serde(with = "crate::serde_decimal")]
    pub net_identifiable_assets_fv: Decimal,
}

/// A single asset or liability line within the purchase price allocation,
/// showing book value, fair value step-up, and useful life for intangibles.
///
/// Named `AcquisitionFvAdjustment` to avoid a name collision with
/// `FairValueAdjustment` in `organizational_event`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcquisitionFvAdjustment {
    /// Description of the asset or liability (e.g. "Customer Relationships")
    pub asset_or_liability: String,

    /// Carrying amount in the acquiree's books at acquisition date
    #[serde(with = "crate::serde_decimal")]
    pub book_value: Decimal,

    /// Fair value assigned in the PPA
    #[serde(with = "crate::serde_decimal")]
    pub fair_value: Decimal,

    /// Step-up amount (fair_value - book_value; may be negative for liabilities)
    #[serde(with = "crate::serde_decimal")]
    pub step_up: Decimal,

    /// Useful life in years for finite-lived intangibles; None for PP&E and indefinite-lived assets
    #[serde(default)]
    pub useful_life_years: Option<u32>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn sample_bc() -> BusinessCombination {
        BusinessCombination {
            id: "BC-001".to_string(),
            acquirer_entity: "PARENT".to_string(),
            acquiree_name: "Acme Sub".to_string(),
            acquiree_entity_code: Some("SUB1".to_string()),
            acquisition_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            consideration: AcquisitionConsideration {
                cash: dec!(1_000_000),
                shares_issued_value: None,
                contingent_consideration: None,
                total: dec!(1_000_000),
            },
            purchase_price_allocation: AcquisitionPpa {
                identifiable_assets: Vec::new(),
                identifiable_liabilities: Vec::new(),
                net_identifiable_assets_fv: dec!(800_000),
            },
            goodwill: dec!(200_000),
            nci_measurement_method: NciMeasurementMethod::Proportionate,
            acquisition_date_nci_fair_value: None,
            framework: "IFRS".to_string(),
        }
    }

    #[test]
    fn nci_opening_fair_value_returns_none_under_proportionate() {
        let bc = sample_bc();
        assert!(bc.nci_opening_fair_value().is_none());
    }

    #[test]
    fn nci_opening_fair_value_returns_supplied_amount() {
        let mut bc = sample_bc();
        bc.nci_measurement_method = NciMeasurementMethod::FullGoodwill;
        bc.acquisition_date_nci_fair_value = Some(dec!(280_000));

        assert_eq!(bc.nci_opening_fair_value(), Some(dec!(280_000)));
    }

    #[test]
    fn business_combination_round_trips_with_new_fields() {
        let mut bc = sample_bc();
        bc.nci_measurement_method = NciMeasurementMethod::FullGoodwill;
        bc.acquisition_date_nci_fair_value = Some(dec!(280_000));

        let json = serde_json::to_string(&bc).unwrap();
        let back: BusinessCombination = serde_json::from_str(&json).unwrap();

        assert_eq!(back.acquiree_entity_code.as_deref(), Some("SUB1"));
        assert_eq!(
            back.nci_measurement_method,
            NciMeasurementMethod::FullGoodwill
        );
        assert_eq!(back.acquisition_date_nci_fair_value, Some(dec!(280_000)));
    }

    #[test]
    fn legacy_archives_default_to_proportionate_and_no_fair_value() {
        // v5.0 / v5.1 archives lack the three new fields entirely.
        // `#[serde(default)]` must let them deserialise cleanly to
        // the Proportionate default + None fair value + no acquiree
        // entity code.
        let legacy_json = r#"{
            "id": "BC-LEGACY",
            "acquirer_entity": "PARENT",
            "acquiree_name": "Old Sub",
            "acquisition_date": "2024-01-01",
            "consideration": {
                "cash": "1000000",
                "shares_issued_value": null,
                "contingent_consideration": null,
                "total": "1000000"
            },
            "purchase_price_allocation": {
                "identifiable_assets": [],
                "identifiable_liabilities": [],
                "net_identifiable_assets_fv": "800000"
            },
            "goodwill": "200000",
            "framework": "IFRS"
        }"#;
        let bc: BusinessCombination = serde_json::from_str(legacy_json).unwrap();
        assert!(bc.acquiree_entity_code.is_none());
        assert_eq!(
            bc.nci_measurement_method,
            NciMeasurementMethod::Proportionate
        );
        assert!(bc.acquisition_date_nci_fair_value.is_none());
    }
}

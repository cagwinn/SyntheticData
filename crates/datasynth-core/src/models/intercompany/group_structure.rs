//! Group structure ownership models for consolidated financial reporting.
//!
//! This module provides models for capturing parent-subsidiary relationships,
//! ownership percentages, and consolidation methods. It feeds into ISA 600
//! (component auditor scope), consolidated financial statements, and NCI
//! (non-controlling interest) calculations.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Complete ownership/consolidation structure for a corporate group.
///
/// Captures the parent entity and all subsidiaries and associates, with their
/// respective ownership percentages and consolidation methods.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupStructure {
    /// Code of the ultimate parent entity in the group.
    pub parent_entity: String,
    /// Subsidiary relationships (>50% owned or otherwise controlled entities).
    pub subsidiaries: Vec<SubsidiaryRelationship>,
    /// Associate relationships (20–50% owned entities, significant influence).
    pub associates: Vec<AssociateRelationship>,
}

impl GroupStructure {
    /// Create a new group structure with the given parent entity.
    pub fn new(parent_entity: String) -> Self {
        Self {
            parent_entity,
            subsidiaries: Vec::new(),
            associates: Vec::new(),
        }
    }

    /// Add a subsidiary relationship.
    pub fn add_subsidiary(&mut self, subsidiary: SubsidiaryRelationship) {
        self.subsidiaries.push(subsidiary);
    }

    /// Add an associate relationship.
    pub fn add_associate(&mut self, associate: AssociateRelationship) {
        self.associates.push(associate);
    }

    /// Return the total number of entities in the group (parent + subs + associates).
    pub fn entity_count(&self) -> usize {
        1 + self.subsidiaries.len() + self.associates.len()
    }
}

/// Relationship between the group parent and a subsidiary entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubsidiaryRelationship {
    /// Entity code of the subsidiary.
    pub entity_code: String,
    /// Percentage of shares held by the parent (0–100).
    pub ownership_percentage: Decimal,
    /// Percentage of voting rights held by the parent (0–100).
    pub voting_rights_percentage: Decimal,
    /// Accounting consolidation method applied to this subsidiary.
    pub consolidation_method: GroupConsolidationMethod,
    /// Date the parent acquired control of this subsidiary.
    pub acquisition_date: Option<NaiveDate>,
    /// Non-controlling interest percentage (= 100 − ownership_percentage).
    pub nci_percentage: Decimal,
    /// Functional currency code of the subsidiary (e.g. "USD", "EUR").
    pub functional_currency: String,
}

impl SubsidiaryRelationship {
    /// Create a fully-owned (100 %) subsidiary with full consolidation.
    pub fn new_full(entity_code: String, functional_currency: String) -> Self {
        Self {
            entity_code,
            ownership_percentage: Decimal::from(100),
            voting_rights_percentage: Decimal::from(100),
            consolidation_method: GroupConsolidationMethod::FullConsolidation,
            acquisition_date: None,
            nci_percentage: Decimal::ZERO,
            functional_currency,
        }
    }

    /// Create a subsidiary with a specified ownership percentage.
    ///
    /// The consolidation method and NCI are derived automatically from the
    /// ownership percentage using IFRS 10 / IAS 28 thresholds.
    pub fn new_with_ownership(
        entity_code: String,
        ownership_percentage: Decimal,
        functional_currency: String,
        acquisition_date: Option<NaiveDate>,
    ) -> Self {
        let consolidation_method = GroupConsolidationMethod::from_ownership(ownership_percentage);
        let nci_percentage = Decimal::from(100) - ownership_percentage;
        Self {
            entity_code,
            ownership_percentage,
            voting_rights_percentage: ownership_percentage,
            consolidation_method,
            acquisition_date,
            nci_percentage,
            functional_currency,
        }
    }
}

/// Consolidation method applied to a subsidiary or investee.
///
/// Distinct from the existing [`super::ConsolidationMethod`] in that it uses
/// IFRS-aligned terminology and adds a `FairValue` option for FVTPL investments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupConsolidationMethod {
    /// Full line-by-line consolidation (IFRS 10, >50 % ownership / control).
    FullConsolidation,
    /// Equity method (IAS 28, 20–50 % ownership, significant influence).
    EquityMethod,
    /// Fair value through profit or loss (<20 % ownership, no influence).
    FairValue,
}

impl GroupConsolidationMethod {
    /// Derive the consolidation method from the ownership percentage.
    ///
    /// Uses standard IFRS 10 / IAS 28 thresholds:
    /// - > 50 % → FullConsolidation
    /// - 20–50 % → EquityMethod
    /// - < 20 % → FairValue
    pub fn from_ownership(ownership_pct: Decimal) -> Self {
        if ownership_pct > Decimal::from(50) {
            Self::FullConsolidation
        } else if ownership_pct >= Decimal::from(20) {
            Self::EquityMethod
        } else {
            Self::FairValue
        }
    }
}

/// Relationship between the group parent and an associate entity.
///
/// Associates are accounted for under the equity method (IAS 28).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssociateRelationship {
    /// Entity code of the associate.
    pub entity_code: String,
    /// Percentage of shares held by the investor (typically 20–50 %).
    pub ownership_percentage: Decimal,
    /// Share of the associate's profit/(loss) recognised in the period.
    pub equity_pickup: Decimal,
}

impl AssociateRelationship {
    /// Create a new associate relationship with zero equity pickup.
    pub fn new(entity_code: String, ownership_percentage: Decimal) -> Self {
        Self {
            entity_code,
            ownership_percentage,
            equity_pickup: Decimal::ZERO,
        }
    }
}

// ---------------------------------------------------------------------------
// NCI Measurement
// ---------------------------------------------------------------------------

/// IFRS 3 § 19 / ASC 805-30-30-1 acquisition-date NCI measurement
/// methods.  Determines how the non-controlling interest is initially
/// measured at the date of a business combination and, by extension,
/// how goodwill is measured (full vs partial / proportionate).
///
/// **The choice is per-acquisition** — IFRS 3.19 lets the acquirer
/// elect on a transaction-by-transaction basis.  US GAAP (ASC 805) is
/// stricter: full-goodwill (fair value) is mandatory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NciMeasurementMethod {
    /// **Proportionate share method** (partial goodwill) — IFRS 3.19(b).
    /// NCI is measured at its proportionate share of the acquiree's
    /// **identifiable net assets**.  Goodwill recognised on
    /// consolidation reflects only the parent's share.  v5.0 default
    /// — the v5.0 stub effectively applied this method by computing
    /// `total_nci = nci_share_net_assets`.
    #[default]
    Proportionate,
    /// **Fair value method** (full goodwill) — IFRS 3.19(a) / ASC
    /// 805-30-30-1.  NCI is measured at its acquisition-date fair
    /// value (typically determined by reference to the quoted price
    /// per share or a valuation technique).  Goodwill recognised on
    /// consolidation includes both the parent's and the NCI's share.
    /// Required under US GAAP; optional under IFRS.
    FullGoodwill,
}

/// Non-controlling interest measurement for a subsidiary.
///
/// Captures the NCI share of net assets and current-period profit/loss,
/// computed from the subsidiary's `nci_percentage` in [`SubsidiaryRelationship`].
///
/// v5.2: extended with an explicit `method` field plus an optional
/// `acquisition_date_fair_value` carrying the IFRS 3 § 19 acquisition-
/// date measurement.  When `method == FullGoodwill` the
/// `acquisition_date_fair_value` is the IFRS 3.19(a) NCI fair-value
/// figure used for total NCI; when `Proportionate` it can still be
/// recorded for disclosure purposes but does not flow into `total_nci`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NciMeasurement {
    /// Entity code of the subsidiary carrying an NCI.
    pub entity_code: String,
    /// NCI percentage (= 100 − parent ownership percentage).
    #[serde(with = "crate::serde_decimal")]
    pub nci_percentage: Decimal,
    /// IFRS 3 § 19 acquisition-date NCI measurement method.  Defaults
    /// to `Proportionate` (matches the v5.0–v5.1 behaviour of
    /// `NciMeasurement::compute`).
    #[serde(default)]
    pub method: NciMeasurementMethod,
    /// Acquisition-date NCI fair value per IFRS 3.19(a).  Required to
    /// be `Some(fv)` when `method == FullGoodwill`; optional otherwise
    /// (carried for disclosure even under the Proportionate method).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "crate::serde_decimal::option")]
    pub acquisition_date_fair_value: Option<Decimal>,
    /// NCI share of the subsidiary's net assets at period-end.
    #[serde(with = "crate::serde_decimal")]
    pub nci_share_net_assets: Decimal,
    /// NCI share of the subsidiary's net income/(loss) for the period.
    #[serde(with = "crate::serde_decimal")]
    pub nci_share_profit: Decimal,
    /// Total NCI recognised in the consolidated balance sheet.  Under
    /// `Proportionate` this equals `nci_share_net_assets`; under
    /// `FullGoodwill` this equals
    /// `acquisition_date_fair_value + Σ(nci_share_profit − nci_dividends)`
    /// — the acquisition-date fair value rolled forward.  v5.2: full-
    /// goodwill rollforward is computed by callers passing prior-period
    /// activity into [`Self::compute_with_method`]; the NCI rollforward
    /// in `datasynth-group::aggregate::nci` already handles the
    /// period-by-period activity.
    #[serde(with = "crate::serde_decimal")]
    pub total_nci: Decimal,
}

impl NciMeasurement {
    /// Compute NCI measurement using the v5.0 simplified path —
    /// `Proportionate` method, no acquisition-date fair value
    /// recorded.  Equivalent to
    /// `compute_with_method(.., Proportionate, None, ..)`.
    ///
    /// # Arguments
    /// * `entity_code` — entity code of the subsidiary.
    /// * `nci_percentage` — NCI percentage (0–100).
    /// * `net_assets` — subsidiary net assets at period-end (before NCI split).
    /// * `net_income` — subsidiary net income/(loss) for the period.
    pub fn compute(
        entity_code: String,
        nci_percentage: Decimal,
        net_assets: Decimal,
        net_income: Decimal,
    ) -> Self {
        Self::compute_with_method(
            entity_code,
            nci_percentage,
            net_assets,
            net_income,
            NciMeasurementMethod::Proportionate,
            None,
        )
    }

    /// Compute NCI measurement with an explicit IFRS 3 § 19 method
    /// and (for the full-goodwill path) the acquisition-date fair
    /// value.
    ///
    /// # Arguments
    /// * `entity_code` — entity code of the subsidiary.
    /// * `nci_percentage` — NCI percentage (0–100).
    /// * `net_assets` — subsidiary net assets at period-end (before NCI split).
    /// * `net_income` — subsidiary net income/(loss) for the period.
    /// * `method` — IFRS 3.19 measurement choice.
    /// * `acquisition_date_fair_value` — required when
    ///   `method == FullGoodwill`; ignored otherwise (other than
    ///   being recorded for disclosure).  Pass `None` for proportionate
    ///   method when no fair-value disclosure is needed.
    ///
    /// Under `Proportionate`: `total_nci = nci_share_net_assets`.
    /// Under `FullGoodwill`: `total_nci = acquisition_date_fair_value`
    /// (caller's responsibility to roll forward across periods using
    /// the `crate::aggregate::nci::compute_nci_rollforward` path in
    /// `datasynth-group`).  When `FullGoodwill` is requested without
    /// a fair value, falls back to the proportionate calculation and
    /// emits a `tracing::warn!`.
    pub fn compute_with_method(
        entity_code: String,
        nci_percentage: Decimal,
        net_assets: Decimal,
        net_income: Decimal,
        method: NciMeasurementMethod,
        acquisition_date_fair_value: Option<Decimal>,
    ) -> Self {
        let hundred = Decimal::from(100);
        let nci_pct_fraction = nci_percentage / hundred;
        let nci_share_net_assets = net_assets * nci_pct_fraction;
        let nci_share_profit = net_income * nci_pct_fraction;

        let total_nci = match (method, acquisition_date_fair_value) {
            (NciMeasurementMethod::FullGoodwill, Some(fv)) => fv,
            (NciMeasurementMethod::FullGoodwill, None) => {
                // Caller asked for full-goodwill but didn't supply a
                // fair value — fall back to proportionate so the
                // computation still produces a balanced number.
                tracing::warn!(
                    entity_code = %entity_code,
                    "NciMeasurement::compute_with_method: \
                     FullGoodwill method requested without an \
                     acquisition_date_fair_value — falling back to \
                     proportionate computation",
                );
                nci_share_net_assets
            }
            (NciMeasurementMethod::Proportionate, _) => nci_share_net_assets,
        };

        Self {
            entity_code,
            nci_percentage,
            method,
            acquisition_date_fair_value,
            nci_share_net_assets,
            nci_share_profit,
            total_nci,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_group_consolidation_method_from_ownership() {
        assert_eq!(
            GroupConsolidationMethod::from_ownership(dec!(100)),
            GroupConsolidationMethod::FullConsolidation
        );
        assert_eq!(
            GroupConsolidationMethod::from_ownership(dec!(51)),
            GroupConsolidationMethod::FullConsolidation
        );
        assert_eq!(
            GroupConsolidationMethod::from_ownership(dec!(50)),
            GroupConsolidationMethod::EquityMethod
        );
        assert_eq!(
            GroupConsolidationMethod::from_ownership(dec!(20)),
            GroupConsolidationMethod::EquityMethod
        );
        assert_eq!(
            GroupConsolidationMethod::from_ownership(dec!(19)),
            GroupConsolidationMethod::FairValue
        );
        assert_eq!(
            GroupConsolidationMethod::from_ownership(dec!(0)),
            GroupConsolidationMethod::FairValue
        );
    }

    #[test]
    fn nci_compute_legacy_path_uses_proportionate() {
        // The v5.0 `compute()` shortcut must continue to behave as
        // before (proportionate share of net assets).  Net assets =
        // 1_000_000, nci_percentage = 25 → 25% × 1M = 250k.
        let m =
            NciMeasurement::compute("SUB1".to_string(), dec!(25), dec!(1_000_000), dec!(120_000));
        assert_eq!(m.method, NciMeasurementMethod::Proportionate);
        assert_eq!(m.nci_share_net_assets, dec!(250_000));
        assert_eq!(m.nci_share_profit, dec!(30_000));
        assert_eq!(m.total_nci, dec!(250_000));
        assert!(m.acquisition_date_fair_value.is_none());
    }

    #[test]
    fn nci_compute_full_goodwill_uses_acquisition_date_fair_value() {
        // IFRS 3.19(a): NCI is measured at its acquisition-date fair
        // value.  Total NCI = the supplied fair value (310k); the
        // proportionate net-asset figure (250k) becomes the bookkeeping
        // floor for share-of-net-assets disclosures but does NOT drive
        // total_nci.  The "extra 60k" represents the NCI's share of
        // goodwill recognised on consolidation.
        let m = NciMeasurement::compute_with_method(
            "SUB1".to_string(),
            dec!(25),
            dec!(1_000_000),
            dec!(120_000),
            NciMeasurementMethod::FullGoodwill,
            Some(dec!(310_000)),
        );
        assert_eq!(m.method, NciMeasurementMethod::FullGoodwill);
        assert_eq!(m.acquisition_date_fair_value, Some(dec!(310_000)));
        assert_eq!(m.nci_share_net_assets, dec!(250_000));
        assert_eq!(m.total_nci, dec!(310_000), "full-goodwill total NCI = FV");
    }

    #[test]
    fn nci_compute_full_goodwill_without_fair_value_falls_back() {
        // Caller asks for FullGoodwill but doesn't supply a fair
        // value.  Must NOT panic — fall back to the proportionate
        // calculation and emit a warning.  Total = nci_share_net_assets.
        let m = NciMeasurement::compute_with_method(
            "SUB1".to_string(),
            dec!(40),
            dec!(500_000),
            dec!(50_000),
            NciMeasurementMethod::FullGoodwill,
            None,
        );
        assert_eq!(m.method, NciMeasurementMethod::FullGoodwill);
        assert!(m.acquisition_date_fair_value.is_none());
        assert_eq!(
            m.total_nci,
            dec!(200_000),
            "missing FV must fall back to proportionate (40% of 500k)"
        );
    }

    #[test]
    fn nci_compute_proportionate_with_disclosure_fair_value() {
        // Proportionate method but caller wants to record a FV for
        // disclosure (IFRS 3.19 lets the entity choose; some entities
        // disclose the FV they would have used had they elected the
        // alternative method).  total_nci stays proportionate; the FV
        // is preserved for serialisation.
        let m = NciMeasurement::compute_with_method(
            "SUB1".to_string(),
            dec!(30),
            dec!(800_000),
            dec!(100_000),
            NciMeasurementMethod::Proportionate,
            Some(dec!(280_000)),
        );
        assert_eq!(m.total_nci, dec!(240_000), "proportionate: 30% of 800k");
        assert_eq!(m.acquisition_date_fair_value, Some(dec!(280_000)));
    }
}

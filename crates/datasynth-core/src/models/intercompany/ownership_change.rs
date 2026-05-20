//! Ownership-change events under IFRS 3.42 / IFRS 10.B97 — step
//! acquisitions, partial divestitures, and control-loss
//! deconsolidations.
//!
//! v5.0 / v5.1 assumed steady-state ownership: each subsidiary kept
//! the same `ownership_percent` for the whole engagement period, so
//! the consolidation rollforward could attribute profit/OCI/dividends
//! pro-rata once and call it done.  Real engagements often see
//! ownership change mid-period.  v5.2 introduces the accounting-side
//! event model — this module — leaving the rollforward / driver
//! wiring to a follow-up PR.
//!
//! # Standards reference
//!
//! - **IFRS 3 § 41–42** — When an entity obtains control over an
//!   investee in which it previously held a non-controlling interest
//!   (associate, joint venture, or financial-asset position), the
//!   **previously-held interest is re-measured at its acquisition-
//!   date fair value** and the resulting gain or loss is recognised
//!   in profit or loss.
//! - **IFRS 10 § 23** / **IFRS 10.B96** — Changes in a parent's
//!   ownership interest that **do not result in a loss of control**
//!   are accounted for as equity transactions.  No gain or loss is
//!   recognised in profit or loss; the carrying amounts of the
//!   controlling and non-controlling interests are adjusted to
//!   reflect the new relative interests.
//! - **IFRS 10 § 25** / **IFRS 10.B97** — When a parent loses control
//!   of a subsidiary, the parent **derecognises** the subsidiary's
//!   assets and liabilities, **recognises any retained interest at
//!   fair value**, and recognises the resulting gain or loss in
//!   profit or loss.
//! - **ASC 805-10-25-10** / **ASC 810-10-40-4** — US GAAP
//!   equivalents.  Same treatment for control-gained (re-measure
//!   prior interest at FV with P&L gain/loss) and control-lost
//!   (FV the retained interest, recognise gain/loss).
//!
//! # Scope
//!
//! This module ships the **typed event model + arithmetic helpers**.
//! The downstream wiring — extending `compute_nci_rollforward` to
//! consume these events and reflect them in the period's NCI
//! roll-forward — is on the v5.2 follow-up roadmap and tracked in
//! the README.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::group_structure::NciMeasurementMethod;

/// Classification of an ownership-change event.  Each variant maps to
/// a different IFRS 10 / IFRS 3 accounting treatment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipChangeType {
    /// Control gained from a non-control position (associate, JV, or
    /// FV investment).  IFRS 3.42 / ASC 805-10-25-10: the
    /// previously-held interest is re-measured at fair value with the
    /// gain/loss in P&L; the acquiree is consolidated thereafter.
    ControlGained,
    /// Existing controlling stake increased — additional shares
    /// purchased without changing the consolidation method
    /// (e.g. 60% → 80%).  IFRS 10.23 / 10.B96 equity-transaction
    /// treatment: NCI shrinks, no P&L gain/loss.
    ControlIncreased,
    /// Existing controlling stake decreased while still retaining
    /// control (e.g. 80% → 55%).  IFRS 10.23 / 10.B96 equity-
    /// transaction treatment: NCI grows, no P&L gain/loss.
    ControlDecreased,
    /// Control lost — full deconsolidation under IFRS 10.25 /
    /// IFRS 10.B97.  Subsidiary's assets and liabilities are
    /// derecognised, any retained interest is re-measured at fair
    /// value, and the gain/loss flows to P&L.
    ControlLost,
}

/// One mid-period ownership-change event for a subsidiary or
/// associate.  All amounts are in the group presentation currency
/// (translation per IAS 21 must already have been applied — same
/// contract as `NciInputs`).
///
/// The struct captures **all** the inputs an IFRS 3.42 / IFRS 10.B97
/// computation needs; the helper methods derive the gain/loss and
/// the post-event NCI carrying amount.  v5.2 ships the model + the
/// helpers; the rollforward wiring is a follow-up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnershipChangeEvent {
    /// Entity code of the subsidiary / associate whose ownership
    /// changed.  Joins to `ManifestEntity::code` so the rollforward
    /// can attribute the event to the right entity.
    pub entity_code: String,

    /// Code of the parent entity whose interest changed.  Mirrors
    /// `ManifestEntity::parent_code`.
    pub parent_entity_code: String,

    /// What happened — drives which accounting treatment applies.
    pub event_type: OwnershipChangeType,

    /// Date the change took effect (typically the closing date of
    /// the share purchase / sale agreement).
    pub effective_date: NaiveDate,

    /// Parent's ownership percent **immediately before** the event,
    /// in `[0, 1]`.  Zero for a fresh acquisition where the parent
    /// had no prior position; one only for a 100 %-owned subsidiary
    /// that's being sold (rare).
    #[serde(with = "crate::serde_decimal")]
    pub ownership_percent_before: Decimal,

    /// Parent's ownership percent **immediately after** the event,
    /// in `[0, 1]`.  Zero only when control is fully lost AND no
    /// retained interest remains; one when this is a roll-up to
    /// 100 %.
    #[serde(with = "crate::serde_decimal")]
    pub ownership_percent_after: Decimal,

    /// Carrying amount of the **previously-held interest** in the
    /// investor's books immediately before the event, in the group
    /// presentation currency.  For `ControlGained`: typically the
    /// equity-method carrying value of the associate.  For
    /// `ControlLost`: the carrying amount of the retained interest
    /// post-deconsolidation (zero if no retained interest).
    #[serde(default, with = "crate::serde_decimal::option")]
    pub previously_held_interest_carrying: Option<Decimal>,

    /// **Acquisition-date fair value** of the previously-held
    /// interest (`ControlGained`) or the retained interest
    /// (`ControlLost`).  This is the amount the investor re-measures
    /// the prior position to under IFRS 3.42 / IFRS 10.B97 — the
    /// difference between this and `previously_held_interest_carrying`
    /// is the P&L gain or loss.
    #[serde(default, with = "crate::serde_decimal::option")]
    pub previously_held_interest_fair_value: Option<Decimal>,

    /// Cash / share consideration paid (positive on
    /// `ControlGained` / `ControlIncreased`) or received (negative
    /// on `ControlDecreased` / `ControlLost`).  Sign convention:
    /// **positive = outflow from parent**, matching how the cash
    /// flow statement presents acquisitions.
    #[serde(with = "crate::serde_decimal")]
    pub consideration_paid_or_received: Decimal,

    /// IFRS 3 § 19 acquisition-date NCI fair value when this event
    /// triggers a new consolidation (`ControlGained` only — must be
    /// `Some(fv)` when method is `FullGoodwill`; ignored for the
    /// other event types).  Mirrors the field on
    /// [`crate::models::business_combination::BusinessCombination`].
    #[serde(default, with = "crate::serde_decimal::option")]
    pub acquisition_date_nci_fair_value: Option<Decimal>,

    /// Method used to measure the new NCI (only relevant for
    /// `ControlGained`).  Defaults to `Proportionate`.
    #[serde(default)]
    pub nci_measurement_method: NciMeasurementMethod,

    /// Group presentation currency the amounts are denominated in.
    pub currency: String,
}

impl OwnershipChangeEvent {
    /// Compute the **P&L gain or loss** triggered by an IFRS 3.42 /
    /// IFRS 10.B97 re-measurement.  Returns:
    ///
    /// - `Some(gain)` (positive) when the fair value of the prior
    ///   position exceeds its carrying amount on `ControlGained` or
    ///   `ControlLost`.
    /// - `Some(loss)` (negative) when carrying exceeds fair value.
    /// - `None` when the event is `ControlIncreased` or
    ///   `ControlDecreased` (IFRS 10.23 — no gain/loss in P&L for
    ///   equity-transaction changes), OR when either the carrying
    ///   amount or the fair value is missing.
    pub fn p_and_l_gain_or_loss(&self) -> Option<Decimal> {
        match self.event_type {
            OwnershipChangeType::ControlGained | OwnershipChangeType::ControlLost => {
                let carrying = self.previously_held_interest_carrying?;
                let fv = self.previously_held_interest_fair_value?;
                Some(fv - carrying)
            }
            OwnershipChangeType::ControlIncreased | OwnershipChangeType::ControlDecreased => None,
        }
    }

    /// Returns `true` when this event triggers a re-measurement
    /// gain/loss in P&L (IFRS 3.42 / IFRS 10.B97).  False for
    /// equity-transaction changes (IFRS 10.23) where the entire
    /// adjustment runs through equity.
    pub fn triggers_pl_remeasurement(&self) -> bool {
        matches!(
            self.event_type,
            OwnershipChangeType::ControlGained | OwnershipChangeType::ControlLost
        )
    }

    /// Returns `true` when this event triggers a transition between
    /// consolidation methods (e.g. associate → subsidiary or
    /// subsidiary → associate).  False for ownership changes that
    /// stay within full-consolidation scope.
    pub fn triggers_method_transition(&self) -> bool {
        matches!(
            self.event_type,
            OwnershipChangeType::ControlGained | OwnershipChangeType::ControlLost
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 7, 1).unwrap()
    }

    fn control_gained_sample() -> OwnershipChangeEvent {
        // Step acquisition: parent went from 30% (associate) to 80%
        // (full sub).  Prior associate carrying = 100k; fair value at
        // acquisition = 130k → 30k IFRS 3.42 gain in P&L.
        OwnershipChangeEvent {
            entity_code: "SUB1".to_string(),
            parent_entity_code: "PARENT".to_string(),
            event_type: OwnershipChangeType::ControlGained,
            effective_date: date(),
            ownership_percent_before: dec!(0.30),
            ownership_percent_after: dec!(0.80),
            previously_held_interest_carrying: Some(dec!(100_000)),
            previously_held_interest_fair_value: Some(dec!(130_000)),
            consideration_paid_or_received: dec!(900_000),
            acquisition_date_nci_fair_value: Some(dec!(310_000)),
            nci_measurement_method: NciMeasurementMethod::FullGoodwill,
            currency: "EUR".to_string(),
        }
    }

    #[test]
    fn control_gained_recognises_remeasurement_gain() {
        let ev = control_gained_sample();
        assert_eq!(
            ev.p_and_l_gain_or_loss(),
            Some(dec!(30_000)),
            "IFRS 3.42 gain = FV (130k) − carrying (100k)"
        );
        assert!(ev.triggers_pl_remeasurement());
        assert!(ev.triggers_method_transition());
    }

    #[test]
    fn control_lost_with_loss_in_p_and_l() {
        // Parent sold from 80% to 30%; retained 30% interest.  Carrying
        // of the retained interest in old basis = 250k; fair value =
        // 200k → 50k loss in P&L per IFRS 10.B97.
        let ev = OwnershipChangeEvent {
            entity_code: "SUB1".to_string(),
            parent_entity_code: "PARENT".to_string(),
            event_type: OwnershipChangeType::ControlLost,
            effective_date: date(),
            ownership_percent_before: dec!(0.80),
            ownership_percent_after: dec!(0.30),
            previously_held_interest_carrying: Some(dec!(250_000)),
            previously_held_interest_fair_value: Some(dec!(200_000)),
            consideration_paid_or_received: dec!(-600_000), // cash received
            acquisition_date_nci_fair_value: None,
            nci_measurement_method: NciMeasurementMethod::Proportionate,
            currency: "EUR".to_string(),
        };
        assert_eq!(ev.p_and_l_gain_or_loss(), Some(dec!(-50_000)));
        assert!(ev.triggers_pl_remeasurement());
        assert!(ev.triggers_method_transition());
    }

    #[test]
    fn control_increased_is_equity_transaction_no_p_and_l() {
        // 60% → 80%: equity transaction.  No P&L gain/loss, no method
        // transition.
        let ev = OwnershipChangeEvent {
            entity_code: "SUB1".to_string(),
            parent_entity_code: "PARENT".to_string(),
            event_type: OwnershipChangeType::ControlIncreased,
            effective_date: date(),
            ownership_percent_before: dec!(0.60),
            ownership_percent_after: dec!(0.80),
            previously_held_interest_carrying: Some(dec!(500_000)),
            previously_held_interest_fair_value: Some(dec!(550_000)),
            consideration_paid_or_received: dec!(200_000),
            acquisition_date_nci_fair_value: None,
            nci_measurement_method: NciMeasurementMethod::Proportionate,
            currency: "EUR".to_string(),
        };
        assert_eq!(
            ev.p_and_l_gain_or_loss(),
            None,
            "IFRS 10.23 — equity transaction, no P&L gain/loss"
        );
        assert!(!ev.triggers_pl_remeasurement());
        assert!(!ev.triggers_method_transition());
    }

    #[test]
    fn control_decreased_is_equity_transaction_no_p_and_l() {
        let ev = OwnershipChangeEvent {
            entity_code: "SUB1".to_string(),
            parent_entity_code: "PARENT".to_string(),
            event_type: OwnershipChangeType::ControlDecreased,
            effective_date: date(),
            ownership_percent_before: dec!(0.80),
            ownership_percent_after: dec!(0.55),
            previously_held_interest_carrying: Some(dec!(800_000)),
            previously_held_interest_fair_value: Some(dec!(700_000)),
            consideration_paid_or_received: dec!(-300_000),
            acquisition_date_nci_fair_value: None,
            nci_measurement_method: NciMeasurementMethod::Proportionate,
            currency: "EUR".to_string(),
        };
        assert_eq!(ev.p_and_l_gain_or_loss(), None);
        assert!(!ev.triggers_pl_remeasurement());
        assert!(!ev.triggers_method_transition());
    }

    #[test]
    fn p_and_l_returns_none_when_carrying_or_fv_missing() {
        // Even ControlGained returns None when inputs are incomplete —
        // the caller should not interpret missing data as zero gain.
        let mut ev = control_gained_sample();
        ev.previously_held_interest_carrying = None;
        assert_eq!(ev.p_and_l_gain_or_loss(), None);

        let mut ev = control_gained_sample();
        ev.previously_held_interest_fair_value = None;
        assert_eq!(ev.p_and_l_gain_or_loss(), None);
    }

    #[test]
    fn ownership_change_event_round_trips_via_serde() {
        let ev = control_gained_sample();
        let json = serde_json::to_string(&ev).unwrap();
        let back: OwnershipChangeEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(back.entity_code, "SUB1");
        assert_eq!(back.event_type, OwnershipChangeType::ControlGained);
        assert_eq!(back.previously_held_interest_carrying, Some(dec!(100_000)));
        assert_eq!(
            back.previously_held_interest_fair_value,
            Some(dec!(130_000))
        );
        assert_eq!(back.acquisition_date_nci_fair_value, Some(dec!(310_000)));
        assert_eq!(
            back.nci_measurement_method,
            NciMeasurementMethod::FullGoodwill
        );
    }
}

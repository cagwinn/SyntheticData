//! Per-subsidiary NCI rollforward — Task 7.1.
//!
//! Implements the IFRS 10 / ASC 810 non-controlling-interest
//! rollforward identity for one fully-consolidated, non-wholly-owned
//! subsidiary at a time:
//!
//! ```text
//! closing_nci = opening_nci
//!             + (1 - ownership_percent) * net_income
//!             + (1 - ownership_percent) * oci
//!             - (1 - ownership_percent) * dividends_paid
//! ```
//!
//! # Standards reference
//!
//! - **IFRS 10.B94** — profit or loss and each component of OCI is
//!   attributed to the owners of the parent and to the NCI in
//!   proportion to their ownership interest (no current-period
//!   reallocation when the proportion changes).
//! - **IFRS 10.22** — the parent presents NCI in the consolidated
//!   statement of financial position within equity, separately from the
//!   equity of the owners of the parent.
//! - **ASC 810-10-45-15** — the US GAAP equivalent: the share of net
//!   income attributable to the NCI is presented separately on the
//!   consolidated income statement.
//!
//! # Validation
//!
//! - `consolidation_method` **must** be
//!   [`crate::config::ConsolidationMethod::Full`].  All other methods
//!   are rejected with [`GroupError::Aggregate`] naming the entity and
//!   the offending method.  NCI is not meaningful for `Parent` entities
//!   (wholly owned), `EquityMethod` / `Proportional` / `FairValue`
//!   investees (one-line investment treatment).
//! - `ownership_percent` **must** be present and `< 1.0`.  A `Full`
//!   entity with `ownership_percent == 1.0` is a caller bug — that
//!   entity should be `Parent` so the wholly-owned-subsidiary path is
//!   used.  We surface this with a precise error message rather than
//!   silently generating a zero NCI.
//! - All amounts use [`rust_decimal::Decimal`] — never `f64`.  The
//!   final `closing_nci` is rounded to **2 decimal places** so the
//!   on-disk JSON is human-readable and the rollforward identity is
//!   verifiable in a spreadsheet.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::config::ConsolidationMethod;
use crate::errors::{GroupError, GroupResult};
use crate::manifest::ManifestEntity;

// ── Public types ──────────────────────────────────────────────────────────────

/// One subsidiary's NCI rollforward record for the period.
///
/// All amounts are denominated in `currency` (the group presentation
/// currency for the consolidated rollforward — translation per Chunk 6
/// must already have been applied to the contributing P&L / OCI /
/// dividend numbers).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NciRollforward {
    /// Subsidiary entity code (matches
    /// [`crate::manifest::ManifestEntity::code`]).
    pub entity_code: String,
    /// Code of the parent entity that holds the controlling interest.
    /// Mirrors [`crate::manifest::ManifestEntity::parent_code`].
    pub parent_entity_code: String,
    /// Parent's ownership share of the subsidiary's equity, in `[0, 1)`.
    pub ownership_percent: Decimal,
    /// Non-controlling interest's share of the subsidiary's equity =
    /// `1 - ownership_percent`.  Stored separately for downstream
    /// consumers so they don't have to recompute it.
    pub nci_percent: Decimal,
    /// Carrying balance of the NCI brought forward from the prior
    /// period (zero on the first period of an engagement).
    pub opening_nci: Decimal,
    /// NCI's share of the period's net income =
    /// `(1 - ownership_percent) * net_income`.
    pub nci_share_of_profit: Decimal,
    /// NCI's share of the period's OCI =
    /// `(1 - ownership_percent) * oci`.
    pub nci_share_of_oci: Decimal,
    /// NCI's share of dividends paid by the subsidiary =
    /// `(1 - ownership_percent) * total_dividends_paid`.  Reduces the
    /// NCI carrying balance.
    pub nci_dividends: Decimal,
    /// **v5.2** — IFRS 10.23 equity-transaction adjustment to NCI for
    /// mid-period ownership changes that don't affect control.  Signed:
    /// positive = NCI grew (parent sold to NCI per `ControlDecreased`);
    /// negative = NCI shrank (parent acquired from NCI per
    /// `ControlIncreased`).  Zero when the entity has no
    /// `ControlIncreased` / `ControlDecreased` events for the period.
    /// `#[serde(default)]` so v5.0–v5.1 archives without the field
    /// load to zero.
    #[serde(default)]
    pub equity_transaction_adjustments: Decimal,
    /// Closing NCI =
    /// `opening_nci + nci_share_of_profit + nci_share_of_oci - nci_dividends + equity_transaction_adjustments`,
    /// rounded to 2dp (banker's rounding).
    pub closing_nci: Decimal,
    /// Period end date the rollforward is as of.
    pub period_end: NaiveDate,
    /// Currency the amounts are denominated in (group presentation
    /// currency).
    pub currency: String,
}

/// Inputs required to derive an [`NciRollforward`].
///
/// The caller is responsible for already having translated
/// `period_net_income`, `period_oci`, and `total_dividends_paid` into
/// the group presentation currency (Chunk 6).  This function does not
/// know about FX rates.
pub struct NciInputs<'a> {
    /// Reference to the subsidiary's manifest entity.  Provides the
    /// entity code, parent code, ownership percent, and consolidation
    /// method used to validate the inputs.
    pub entity: &'a ManifestEntity,
    /// Subsidiary's period net income (after tax), in the same currency
    /// as `currency`.  Sign convention: profit positive, loss negative.
    pub period_net_income: Decimal,
    /// Subsidiary's period OCI (other comprehensive income), in the
    /// same currency as `currency`.
    pub period_oci: Decimal,
    /// Subsidiary's total dividends paid for the period (gross — both
    /// to controlling and non-controlling shareholders).  Always
    /// non-negative; the NCI share will be subtracted from the closing
    /// balance.
    pub total_dividends_paid: Decimal,
    /// Opening NCI carrying value brought forward from the prior period
    /// (zero on the first period of an engagement).
    pub opening_nci: Decimal,
    /// v5.2: IFRS 3 § 19(a) / ASC 805-30-30-1 acquisition-date NCI
    /// fair value.  Used **only** on the first period of an engagement
    /// (when `opening_nci == 0`).  When supplied, the rollforward uses
    /// this fair value as the opening NCI instead of zero, implementing
    /// the full-goodwill measurement basis where NCI is recognised at
    /// its acquisition-date fair value rather than its proportionate
    /// share of net assets.  `None` defaults to the v5.0–v5.1
    /// behaviour (proportionate basis: opening NCI starts at zero on
    /// the first period and grows via share-of-profit / OCI in
    /// subsequent periods).
    pub acquisition_date_nci_fair_value: Option<Decimal>,
    /// **v5.2** — IFRS 10.23 / IFRS 10.B96 mid-period ownership-change
    /// events affecting this subsidiary.  The rollforward consumes
    /// these to compute the equity-transaction adjustment to NCI.
    /// Empty by default — engagements without ownership-change events
    /// see no behaviour change.  Driver-side wiring reads each entity's
    /// `intercompany/ownership_change_events.json` file (PR #155) and
    /// threads the parsed events here.
    pub ownership_changes: &'a [datasynth_core::models::intercompany::OwnershipChangeEvent],
    /// Period end date.
    pub period_end: NaiveDate,
    /// Group presentation currency.
    pub currency: String,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Derive an [`NciRollforward`] for one subsidiary.
///
/// Pure function: no I/O, no allocation beyond the record itself, no
/// dependence on global state.  Two calls with the same input produce
/// equal records.
///
/// # Validation
///
/// 1. `entity.consolidation_method` **must** be
///    [`ConsolidationMethod::Full`].  Returns
///    [`GroupError::Aggregate`] otherwise (NCI is meaningless for any
///    other consolidation method).
/// 2. `entity.ownership_percent` **must** be present and `< 1.0`.
///    Wholly-owned entities should be modelled as `Parent` so the
///    wholly-owned-subsidiary path applies; an entity that is `Full`
///    but `100%` owned is a caller bug.
///
/// # Math
///
/// ```text
/// nci_percent           = 1 - ownership_percent
/// nci_share_of_profit   = nci_percent * period_net_income
/// nci_share_of_oci      = nci_percent * period_oci
/// nci_dividends         = nci_percent * total_dividends_paid
/// closing_nci           = (opening_nci + nci_share_of_profit
///                          + nci_share_of_oci - nci_dividends).round_dp(2)
/// ```
///
/// # Errors
///
/// - [`GroupError::Aggregate`] if `consolidation_method` is not
///   `Full` — names the entity and the offending method.
/// - [`GroupError::Aggregate`] if `ownership_percent` is missing or
///   `>= 1.0` — names the entity.
pub fn compute_nci_rollforward(inputs: &NciInputs) -> GroupResult<NciRollforward> {
    let entity = inputs.entity;

    // 1. Reject any non-Full consolidation method up front — NCI is
    //    not meaningful for Parent / EquityMethod / Proportional /
    //    FairValue entities.
    if entity.consolidation_method != ConsolidationMethod::Full {
        return Err(GroupError::Aggregate(format!(
            "compute_nci_rollforward: entity `{}` has consolidation_method=\
             {:?} — NCI is only meaningful for ConsolidationMethod::Full \
             (Parent is wholly owned; EquityMethod / Proportional / \
             FairValue use one-line investment treatment)",
            entity.code, entity.consolidation_method,
        )));
    }

    // 2. Ownership must be present and < 1.0 for a Full entity to
    //    have any NCI to measure.
    let ownership_percent = entity.ownership_percent.ok_or_else(|| {
        GroupError::Aggregate(format!(
            "compute_nci_rollforward: entity `{}` is consolidation_method=\
             Full but has no ownership_percent set — supply ownership_percent \
             < 1.0 or change the method to Parent for wholly-owned",
            entity.code,
        ))
    })?;

    if ownership_percent >= Decimal::ONE {
        return Err(GroupError::Aggregate(format!(
            "compute_nci_rollforward: entity `{}` is consolidation_method=\
             Full but has no ownership_percent < 1.0 — use Parent for \
             wholly-owned",
            entity.code,
        )));
    }

    // Derive the parent code from the manifest.  Required for the
    // rollforward record so downstream consumers (consolidation note
    // disclosure, IFRS 12 § 12 schedule) can attribute the NCI to a
    // controlling parent without re-walking the manifest.
    let parent_entity_code = entity.parent_code.clone().ok_or_else(|| {
        GroupError::Aggregate(format!(
            "compute_nci_rollforward: entity `{}` has no parent_code in the \
             manifest — every Full subsidiary must declare its parent",
            entity.code,
        ))
    })?;

    let nci_percent = Decimal::ONE - ownership_percent;

    // v5.2: when this is the first period of an engagement (i.e.
    // `opening_nci == 0`) AND the caller supplied an
    // `acquisition_date_nci_fair_value`, seed the opening NCI from
    // the fair value rather than starting at zero.  This implements
    // IFRS 3 § 19(a) / ASC 805-30-30-1 full-goodwill measurement
    // (NCI recognised at acquisition-date fair value) — the fair
    // value rolls forward through the standard share-of-equity
    // allocation in subsequent periods.
    //
    // For non-first-period rollforwards (`opening_nci != 0`) the
    // fair value is ignored — the prior period's closing balance has
    // already absorbed it via the period-1 calculation.  For
    // proportionate measurement (no fair value supplied) the rollforward
    // works as in v5.0–v5.1: opening starts at zero, share of profit
    // grows it.
    let effective_opening_nci = match (
        inputs.opening_nci.is_zero(),
        inputs.acquisition_date_nci_fair_value,
    ) {
        (true, Some(fv)) => fv,
        _ => inputs.opening_nci,
    };

    // 3. Apply the IFRS 10.B94 / ASC 810 share-of-equity allocation.
    let nci_share_of_profit = nci_percent * inputs.period_net_income;
    let nci_share_of_oci = nci_percent * inputs.period_oci;
    let nci_dividends = nci_percent * inputs.total_dividends_paid;

    // 4. **v5.2** — IFRS 10.23 / IFRS 10.B96 equity-transaction
    //    adjustments for mid-period ownership changes that don't
    //    affect control.  For each event:
    //
    //    - `ControlIncreased` (parent buys NCI shares): NCI carrying
    //      shrinks by approximately the consideration paid.  Sign:
    //      `consideration_paid_or_received` is positive (outflow from
    //      parent), so the adjustment is `-consideration` = negative.
    //    - `ControlDecreased` (parent sells to NCI): NCI carrying
    //      grows by approximately the consideration received.  Sign:
    //      `consideration_paid_or_received` is negative (inflow to
    //      parent), so the adjustment is `-consideration` = positive.
    //
    //    Both cases reduce to a single rule: `adjustment = -consideration`.
    //    This is the no-premium / no-discount approximation — real
    //    engagements may pay above or below NCI carrying value, with
    //    the difference recognised in equity attributable to the
    //    parent (IFRS 10.23 second sentence).  Modelling that residual
    //    requires the subsidiary's net equity at the change date,
    //    which the orchestrator doesn't currently emit per-entity at
    //    sub-period granularity — a v5.4 follow-up.
    //
    //    `ControlGained` mid-period (i.e. *not* the period-1 case
    //    handled by `acquisition_date_nci_fair_value`) and
    //    `ControlLost` are out of scope: they require pro-rated
    //    profit attribution and adding/removing the entity from
    //    consolidation, which is a multi-PR feature.
    use datasynth_core::models::intercompany::OwnershipChangeType;
    let mut equity_transaction_adjustments = Decimal::ZERO;
    for ev in inputs.ownership_changes {
        match ev.event_type {
            OwnershipChangeType::ControlIncreased | OwnershipChangeType::ControlDecreased => {
                equity_transaction_adjustments -= ev.consideration_paid_or_received;
            }
            OwnershipChangeType::ControlGained => {
                return Err(GroupError::Aggregate(format!(
                    "compute_nci_rollforward: entity `{}`: ControlGained \
                     mid-period events are not yet supported by the rollforward — \
                     this is a v5.4 follow-up.  Period-1 acquisitions should use \
                     `acquisition_date_nci_fair_value` instead.",
                    entity.code,
                )));
            }
            OwnershipChangeType::ControlLost => {
                return Err(GroupError::Aggregate(format!(
                    "compute_nci_rollforward: entity `{}`: ControlLost \
                     mid-period deconsolidation events are not yet supported \
                     by the rollforward — this is a v5.4 follow-up.",
                    entity.code,
                )));
            }
        }
    }

    let closing_nci = (effective_opening_nci + nci_share_of_profit + nci_share_of_oci
        - nci_dividends
        + equity_transaction_adjustments)
        .round_dp(2);

    Ok(NciRollforward {
        entity_code: entity.code.clone(),
        parent_entity_code,
        ownership_percent,
        nci_percent,
        opening_nci: effective_opening_nci.round_dp(2),
        nci_share_of_profit: nci_share_of_profit.round_dp(2),
        nci_share_of_oci: nci_share_of_oci.round_dp(2),
        nci_dividends: nci_dividends.round_dp(2),
        equity_transaction_adjustments: equity_transaction_adjustments.round_dp(2),
        closing_nci,
        period_end: inputs.period_end,
        currency: inputs.currency.clone(),
    })
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn make_entity(
        code: &str,
        method: ConsolidationMethod,
        ownership: Option<Decimal>,
    ) -> ManifestEntity {
        ManifestEntity {
            code: code.to_string(),
            name: None,
            country: "DE".to_string(),
            functional_currency: "EUR".to_string(),
            scoping_profile: "significant".to_string(),
            consolidation_method: method,
            ownership_percent: ownership,
            parent_code: Some("PARENT".to_string()),
            accounting_framework: None,
            industry: None,
            hyperinflation_status:
                datasynth_core::models::HyperinflationStatus::NotHyperinflationary,
            ownership_changes: Vec::new(),
            entity_seed: "00".to_string(),
            shard_id: "S_DEFAULT_0001".to_string(),
        }
    }

    fn period_end() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
    }

    #[test]
    fn happy_path_eighty_percent_owned() {
        let entity = make_entity("SUB", ConsolidationMethod::Full, Some(dec!(0.80)));
        let inputs = NciInputs {
            entity: &entity,
            period_net_income: dec!(1000),
            period_oci: dec!(200),
            total_dividends_paid: dec!(500),
            opening_nci: dec!(800),
            acquisition_date_nci_fair_value: None,
            ownership_changes: &[],
            period_end: period_end(),
            currency: "CHF".to_string(),
        };

        let rf = compute_nci_rollforward(&inputs).expect("must succeed");

        assert_eq!(rf.entity_code, "SUB");
        assert_eq!(rf.parent_entity_code, "PARENT");
        assert_eq!(rf.ownership_percent, dec!(0.80));
        assert_eq!(rf.nci_percent, dec!(0.20));
        assert_eq!(rf.nci_share_of_profit, dec!(200.00));
        assert_eq!(rf.nci_share_of_oci, dec!(40.00));
        assert_eq!(rf.nci_dividends, dec!(100.00));
        // 800 + 200 + 40 - 100 = 940
        assert_eq!(rf.closing_nci, dec!(940.00));
    }

    #[test]
    fn full_goodwill_acquisition_date_fair_value_seeds_period_one_opening() {
        // v5.2: IFRS 3.19(a) full-goodwill measurement.  Period 1
        // (opening_nci == 0) with a fair value of 850 → effective
        // opening = 850, then standard period activity rolls forward.
        //
        // Subsidiary:
        //   - 75% owned (NCI = 25%)
        //   - period net income = 1000 → NCI share = 250
        //   - period OCI = 200 → NCI share = 50
        //   - dividends paid = 400 → NCI share = 100
        // Closing = 850 + 250 + 50 - 100 = 1050
        let entity = make_entity("SUB", ConsolidationMethod::Full, Some(dec!(0.75)));
        let inputs = NciInputs {
            entity: &entity,
            period_net_income: dec!(1000),
            period_oci: dec!(200),
            total_dividends_paid: dec!(400),
            opening_nci: Decimal::ZERO,
            acquisition_date_nci_fair_value: Some(dec!(850)),
            ownership_changes: &[],
            period_end: period_end(),
            currency: "CHF".to_string(),
        };
        let rf = compute_nci_rollforward(&inputs).expect("must succeed");

        assert_eq!(
            rf.opening_nci,
            dec!(850.00),
            "opening must be seeded from fair value, not zero"
        );
        assert_eq!(rf.nci_share_of_profit, dec!(250.00));
        assert_eq!(rf.nci_share_of_oci, dec!(50.00));
        assert_eq!(rf.nci_dividends, dec!(100.00));
        assert_eq!(rf.closing_nci, dec!(1050.00));
    }

    #[test]
    fn full_goodwill_fair_value_ignored_when_opening_nci_nonzero() {
        // v5.2: in subsequent periods (opening_nci != 0) the fair
        // value is ignored — the prior period already absorbed it
        // via its own period-1 calculation.  Effective opening must
        // come from `opening_nci`, not the fair value.
        let entity = make_entity("SUB", ConsolidationMethod::Full, Some(dec!(0.80)));
        let inputs = NciInputs {
            entity: &entity,
            period_net_income: dec!(500),
            period_oci: Decimal::ZERO,
            total_dividends_paid: Decimal::ZERO,
            opening_nci: dec!(940), // brought forward from a prior period
            acquisition_date_nci_fair_value: Some(dec!(1000)), // stale FV
            ownership_changes: &[],
            period_end: period_end(),
            currency: "CHF".to_string(),
        };
        let rf = compute_nci_rollforward(&inputs).expect("must succeed");

        assert_eq!(
            rf.opening_nci,
            dec!(940.00),
            "fair value must not override a non-zero opening_nci"
        );
        // Closing = 940 + 100 (20% of 500) = 1040
        assert_eq!(rf.closing_nci, dec!(1040.00));
    }

    #[test]
    fn proportionate_basis_unchanged_when_no_fair_value() {
        // v5.2 sanity: when no fair value is supplied (None), the
        // rollforward must produce byte-identical output to v5.0–v5.1.
        let entity = make_entity("SUB", ConsolidationMethod::Full, Some(dec!(0.80)));
        let inputs = NciInputs {
            entity: &entity,
            period_net_income: dec!(1000),
            period_oci: dec!(200),
            total_dividends_paid: dec!(500),
            opening_nci: dec!(800),
            acquisition_date_nci_fair_value: None, // proportionate basis
            ownership_changes: &[],
            period_end: period_end(),
            currency: "CHF".to_string(),
        };
        let rf = compute_nci_rollforward(&inputs).expect("must succeed");

        // Same numbers as `happy_path_eighty_percent_owned`.
        assert_eq!(rf.opening_nci, dec!(800.00));
        assert_eq!(rf.closing_nci, dec!(940.00));
    }

    #[test]
    fn rejects_parent_method() {
        let entity = make_entity("PARENT_CO", ConsolidationMethod::Parent, None);
        let inputs = NciInputs {
            entity: &entity,
            period_net_income: Decimal::ZERO,
            period_oci: Decimal::ZERO,
            total_dividends_paid: Decimal::ZERO,
            opening_nci: Decimal::ZERO,
            acquisition_date_nci_fair_value: None,
            ownership_changes: &[],
            period_end: period_end(),
            currency: "CHF".to_string(),
        };

        let err = compute_nci_rollforward(&inputs).expect_err("must reject parent");
        match err {
            GroupError::Aggregate(msg) => {
                assert!(msg.contains("PARENT_CO"));
                assert!(msg.contains("Parent"));
            }
            other => panic!("expected Aggregate, got {other:?}"),
        }
    }

    // ── v5.2 IFRS 10.23 equity-transaction tests ─────────────────────────

    fn equity_event(
        ty: datasynth_core::models::intercompany::OwnershipChangeType,
        before: Decimal,
        after: Decimal,
        consideration: Decimal,
    ) -> datasynth_core::models::intercompany::OwnershipChangeEvent {
        datasynth_core::models::intercompany::OwnershipChangeEvent {
            entity_code: "SUB".to_string(),
            parent_entity_code: "PARENT".to_string(),
            event_type: ty,
            effective_date: NaiveDate::from_ymd_opt(2024, 2, 15).unwrap(),
            ownership_percent_before: before,
            ownership_percent_after: after,
            previously_held_interest_carrying: None,
            previously_held_interest_fair_value: None,
            consideration_paid_or_received: consideration,
            acquisition_date_nci_fair_value: None,
            nci_measurement_method: Default::default(),
            currency: "CHF".to_string(),
        }
    }

    #[test]
    fn control_increased_shrinks_nci_by_consideration() {
        // Parent buys 10% from NCI for 100 — NCI shrinks by ~100.
        // Opening NCI 800, share of profit 200 (1000 × 20% pre-event
        // simplification), share of OCI 0, dividends 0,
        // equity-transaction adjustment = -100.
        // Closing = 800 + 200 + 0 - 0 - 100 = 900.
        use datasynth_core::models::intercompany::OwnershipChangeType;
        let entity = make_entity("SUB", ConsolidationMethod::Full, Some(dec!(0.80)));
        let event = equity_event(
            OwnershipChangeType::ControlIncreased,
            dec!(0.80),
            dec!(0.90),
            dec!(100),
        );
        let inputs = NciInputs {
            entity: &entity,
            period_net_income: dec!(1000),
            period_oci: Decimal::ZERO,
            total_dividends_paid: Decimal::ZERO,
            opening_nci: dec!(800),
            acquisition_date_nci_fair_value: None,
            ownership_changes: std::slice::from_ref(&event),
            period_end: period_end(),
            currency: "CHF".to_string(),
        };
        let rf = compute_nci_rollforward(&inputs).unwrap();
        assert_eq!(rf.equity_transaction_adjustments, dec!(-100.00));
        assert_eq!(rf.closing_nci, dec!(900.00));
    }

    #[test]
    fn control_decreased_grows_nci_by_consideration_received() {
        // Parent sells 10% to NCI for 100 (negative sign — inflow).
        // NCI grows by ~100.  Opening 800, profit 200, dividends 0,
        // adjustment = -(-100) = +100.  Closing = 800+200+0-0+100 = 1100.
        use datasynth_core::models::intercompany::OwnershipChangeType;
        let entity = make_entity("SUB", ConsolidationMethod::Full, Some(dec!(0.80)));
        let event = equity_event(
            OwnershipChangeType::ControlDecreased,
            dec!(0.90),
            dec!(0.80),
            dec!(-100),
        );
        let inputs = NciInputs {
            entity: &entity,
            period_net_income: dec!(1000),
            period_oci: Decimal::ZERO,
            total_dividends_paid: Decimal::ZERO,
            opening_nci: dec!(800),
            acquisition_date_nci_fair_value: None,
            ownership_changes: std::slice::from_ref(&event),
            period_end: period_end(),
            currency: "CHF".to_string(),
        };
        let rf = compute_nci_rollforward(&inputs).unwrap();
        assert_eq!(rf.equity_transaction_adjustments, dec!(100.00));
        assert_eq!(rf.closing_nci, dec!(1100.00));
    }

    #[test]
    fn multiple_equity_transactions_sum() {
        // Two equity transactions: +50 (decrease) then -30 (increase).
        // Total adjustment: -(-50) + -(30) = +50 - 30 = +20.
        use datasynth_core::models::intercompany::OwnershipChangeType;
        let entity = make_entity("SUB", ConsolidationMethod::Full, Some(dec!(0.75)));
        let events = vec![
            equity_event(
                OwnershipChangeType::ControlDecreased,
                dec!(0.80),
                dec!(0.75),
                dec!(-50),
            ),
            equity_event(
                OwnershipChangeType::ControlIncreased,
                dec!(0.75),
                dec!(0.78),
                dec!(30),
            ),
        ];
        let inputs = NciInputs {
            entity: &entity,
            period_net_income: Decimal::ZERO,
            period_oci: Decimal::ZERO,
            total_dividends_paid: Decimal::ZERO,
            opening_nci: dec!(500),
            acquisition_date_nci_fair_value: None,
            ownership_changes: &events,
            period_end: period_end(),
            currency: "CHF".to_string(),
        };
        let rf = compute_nci_rollforward(&inputs).unwrap();
        assert_eq!(rf.equity_transaction_adjustments, dec!(20.00));
        assert_eq!(rf.closing_nci, dec!(520.00));
    }

    #[test]
    fn control_gained_mid_period_rejected_with_v54_pointer() {
        use datasynth_core::models::intercompany::OwnershipChangeType;
        let entity = make_entity("SUB", ConsolidationMethod::Full, Some(dec!(0.80)));
        let event = equity_event(
            OwnershipChangeType::ControlGained,
            Decimal::ZERO,
            dec!(0.80),
            dec!(1000),
        );
        let inputs = NciInputs {
            entity: &entity,
            period_net_income: Decimal::ZERO,
            period_oci: Decimal::ZERO,
            total_dividends_paid: Decimal::ZERO,
            opening_nci: Decimal::ZERO,
            acquisition_date_nci_fair_value: None,
            ownership_changes: std::slice::from_ref(&event),
            period_end: period_end(),
            currency: "CHF".to_string(),
        };
        let err = compute_nci_rollforward(&inputs).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("ControlGained"));
        assert!(msg.contains("v5.4"));
    }

    #[test]
    fn control_lost_mid_period_rejected_with_v54_pointer() {
        use datasynth_core::models::intercompany::OwnershipChangeType;
        let entity = make_entity("SUB", ConsolidationMethod::Full, Some(dec!(0.80)));
        let event = equity_event(
            OwnershipChangeType::ControlLost,
            dec!(0.80),
            Decimal::ZERO,
            dec!(-500),
        );
        let inputs = NciInputs {
            entity: &entity,
            period_net_income: Decimal::ZERO,
            period_oci: Decimal::ZERO,
            total_dividends_paid: Decimal::ZERO,
            opening_nci: dec!(800),
            acquisition_date_nci_fair_value: None,
            ownership_changes: std::slice::from_ref(&event),
            period_end: period_end(),
            currency: "CHF".to_string(),
        };
        let err = compute_nci_rollforward(&inputs).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("ControlLost"));
        assert!(msg.contains("v5.4"));
    }

    #[test]
    fn empty_ownership_changes_byte_identical_to_baseline() {
        // With no events, the rollforward must produce the same numbers
        // as the v5.0–v5.1 baseline — equity_transaction_adjustments is
        // exactly zero and closing_nci has no contribution from events.
        let entity = make_entity("SUB", ConsolidationMethod::Full, Some(dec!(0.80)));
        let inputs = NciInputs {
            entity: &entity,
            period_net_income: dec!(1000),
            period_oci: dec!(200),
            total_dividends_paid: dec!(500),
            opening_nci: dec!(800),
            acquisition_date_nci_fair_value: None,
            ownership_changes: &[],
            period_end: period_end(),
            currency: "CHF".to_string(),
        };
        let rf = compute_nci_rollforward(&inputs).unwrap();
        assert_eq!(rf.equity_transaction_adjustments, Decimal::ZERO);
        // 800 + 200 + 40 - 100 = 940 (same as happy_path_eighty_percent_owned)
        assert_eq!(rf.closing_nci, dec!(940.00));
    }
}

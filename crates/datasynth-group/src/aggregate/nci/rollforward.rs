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
    /// Closing NCI =
    /// `opening_nci + nci_share_of_profit + nci_share_of_oci - nci_dividends`,
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

    // 3. Apply the IFRS 10.B94 / ASC 810 share-of-equity allocation.
    let nci_share_of_profit = nci_percent * inputs.period_net_income;
    let nci_share_of_oci = nci_percent * inputs.period_oci;
    let nci_dividends = nci_percent * inputs.total_dividends_paid;

    let closing_nci =
        (inputs.opening_nci + nci_share_of_profit + nci_share_of_oci - nci_dividends).round_dp(2);

    Ok(NciRollforward {
        entity_code: entity.code.clone(),
        parent_entity_code,
        ownership_percent,
        nci_percent,
        opening_nci: inputs.opening_nci.round_dp(2),
        nci_share_of_profit: nci_share_of_profit.round_dp(2),
        nci_share_of_oci: nci_share_of_oci.round_dp(2),
        nci_dividends: nci_dividends.round_dp(2),
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
    fn rejects_parent_method() {
        let entity = make_entity("PARENT_CO", ConsolidationMethod::Parent, None);
        let inputs = NciInputs {
            entity: &entity,
            period_net_income: Decimal::ZERO,
            period_oci: Decimal::ZERO,
            total_dividends_paid: Decimal::ZERO,
            opening_nci: Decimal::ZERO,
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
}

//! Cash-generating units (CGUs) and CGU-level goodwill impairment
//! testing under IAS 36.
//!
//! v5.0 / v5.1 had a per-asset [`datasynth_standards::ImpairmentTest`]
//! that handled individual goodwill or PP&E assets, but no
//! **CGU-level** goodwill impairment test.  In group consolidation,
//! goodwill recognised on a business combination is allocated to one
//! or more CGUs at the acquisition date and tested for impairment
//! annually (IAS 36.10) at the CGU level — not at the individual-
//! goodwill-asset level.  This module fills that gap.
//!
//! # Standards reference
//!
//! - **IAS 36 § 6** — A cash-generating unit is the **smallest
//!   identifiable group of assets that generates cash inflows that
//!   are largely independent of the cash inflows from other assets
//!   or groups of assets**.
//! - **IAS 36 § 10** — Goodwill must be tested for impairment **at
//!   least annually** (and whenever there is an indication of
//!   impairment), regardless of whether indicators exist.
//! - **IAS 36 § 80** — Goodwill acquired in a business combination
//!   shall be allocated to each of the acquirer's CGUs (or groups of
//!   CGUs) that is expected to benefit from the synergies.
//! - **IAS 36 § 18** — The recoverable amount of an asset (or CGU)
//!   is the higher of its **fair value less costs of disposal**
//!   and its **value in use**.
//! - **IAS 36 § 104** — When a CGU's recoverable amount is less than
//!   its carrying amount, the impairment loss is allocated:
//!   1. First, to **goodwill** allocated to the CGU.
//!   2. Then, **pro rata** to the other assets of the CGU based on
//!      the carrying amount of each asset.
//! - **IAS 36 § 124** — An impairment loss recognised for **goodwill
//!   shall not be reversed** in a subsequent period.
//!
//! # Scope
//!
//! v5.2 ships the typed model + arithmetic helpers
//! ([`CashGeneratingUnit`], [`GoodwillAllocation`], [`CguImpairmentTest`],
//! [`CguImpairmentResult`]).  The CGU identification step (mapping
//! manifest entities → CGUs based on operating segments / cash-flow
//! independence) and the auto-test trigger at engagement period-end
//! are downstream wiring and tracked as v5.2 follow-ups.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A cash-generating unit per IAS 36 § 6 — the smallest identifiable
/// group of assets that generates largely independent cash inflows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CashGeneratingUnit {
    /// Unique CGU identifier — typically a stable code so allocations
    /// can be tracked across periods.
    pub cgu_id: String,

    /// Human-readable name (e.g. "EMEA Consumer Products").
    pub name: String,

    /// Codes of the entities (subsidiaries / branches) whose cash
    /// flows are aggregated to form this CGU.  A CGU may span multiple
    /// legal entities or be a sub-division of a single entity.
    pub member_entity_codes: Vec<String>,

    /// Reportable segment this CGU rolls up to (IFRS 8 / ASC 280).
    /// Multiple CGUs can map to the same segment.  None when no
    /// segment-reporting attribution applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_code: Option<String>,
}

impl CashGeneratingUnit {
    /// Construct a CGU.  `member_entity_codes` may be empty if the
    /// CGU is being seeded for later allocation; downstream tests
    /// require at least one member.
    pub fn new(
        cgu_id: impl Into<String>,
        name: impl Into<String>,
        member_entity_codes: Vec<String>,
    ) -> Self {
        Self {
            cgu_id: cgu_id.into(),
            name: name.into(),
            member_entity_codes,
            segment_code: None,
        }
    }

    /// Attach a reportable segment code (IFRS 8 / ASC 280).
    pub fn with_segment(mut self, segment_code: impl Into<String>) -> Self {
        self.segment_code = Some(segment_code.into());
        self
    }
}

/// Goodwill allocated to a CGU at the acquisition date per IAS 36 §
/// 80.  When a single business combination's goodwill spans multiple
/// CGUs, the engagement records one [`GoodwillAllocation`] per CGU
/// totalling the goodwill on the acquisition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GoodwillAllocation {
    /// CGU identifier (matches [`CashGeneratingUnit::cgu_id`]).
    pub cgu_id: String,

    /// Business combination identifier (matches
    /// [`crate::models::business_combination::BusinessCombination::id`])
    /// — links the goodwill to the underlying acquisition for audit
    /// trail and post-implementation review.
    pub business_combination_id: String,

    /// Amount of goodwill allocated, in the group presentation
    /// currency.  Always non-negative — a bargain purchase produces
    /// no goodwill, hence no allocation row.
    #[serde(with = "crate::serde_decimal")]
    pub goodwill_amount: Decimal,

    /// Date the allocation took effect (typically the acquisition
    /// date).
    pub allocation_date: NaiveDate,
}

/// Annual CGU-level goodwill impairment test under IAS 36 § 10.
///
/// Inputs to the test are the CGU's carrying amount (including
/// allocated goodwill + the other assets of the unit), and the
/// recoverable amount components — fair value less costs of disposal
/// and value in use.  IAS 36 § 18 says recoverable = max of the two.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CguImpairmentTest {
    /// CGU being tested.
    pub cgu_id: String,

    /// Period-end test date (IAS 36 § 10 — at least annually).
    pub test_date: NaiveDate,

    /// Total goodwill allocated to this CGU at test date.  Always
    /// non-negative.  Sum of the
    /// [`GoodwillAllocation::goodwill_amount`] entries that land
    /// on this CGU, net of prior-period impairment that reduced the
    /// allocated goodwill (IAS 36 § 124 prohibits reversal of
    /// goodwill impairments).
    #[serde(with = "crate::serde_decimal")]
    pub allocated_goodwill: Decimal,

    /// Carrying amount of the **other assets** of the CGU
    /// (everything except the allocated goodwill) immediately
    /// before this test.  Always non-negative.
    #[serde(with = "crate::serde_decimal")]
    pub other_carrying: Decimal,

    /// Fair value of the CGU less costs of disposal at the test
    /// date.
    #[serde(with = "crate::serde_decimal")]
    pub fair_value_less_costs: Decimal,

    /// Value in use of the CGU at the test date (typically the
    /// present value of the next 5 years of net cash flows + a
    /// terminal value, discounted at the WACC).
    #[serde(with = "crate::serde_decimal")]
    pub value_in_use: Decimal,

    /// Group presentation currency.
    pub currency: String,
}

/// Result of a CGU-level goodwill impairment test.  Contains the
/// recoverable amount, total impairment loss, and the IAS 36 § 104
/// allocation between goodwill and other assets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CguImpairmentResult {
    /// CGU tested.
    pub cgu_id: String,

    /// Period-end test date (matches [`CguImpairmentTest::test_date`]).
    pub test_date: NaiveDate,

    /// Total carrying amount of the CGU before the test
    /// (`allocated_goodwill + other_carrying`).
    #[serde(with = "crate::serde_decimal")]
    pub carrying_total: Decimal,

    /// Recoverable amount = max(fair_value_less_costs, value_in_use)
    /// per IAS 36 § 18.
    #[serde(with = "crate::serde_decimal")]
    pub recoverable_amount: Decimal,

    /// Total impairment loss on the CGU =
    /// `max(0, carrying_total − recoverable_amount)`.  Zero when the
    /// CGU is recoverable.
    #[serde(with = "crate::serde_decimal")]
    pub impairment_loss_total: Decimal,

    /// Portion of `impairment_loss_total` allocated to goodwill
    /// per IAS 36 § 104 — `min(impairment_loss_total, allocated_goodwill)`.
    /// Goodwill impairments are NOT reversible (IAS 36 § 124).
    #[serde(with = "crate::serde_decimal")]
    pub impairment_loss_to_goodwill: Decimal,

    /// Portion of `impairment_loss_total` allocated pro rata to the
    /// CGU's other assets =
    /// `impairment_loss_total − impairment_loss_to_goodwill`.
    #[serde(with = "crate::serde_decimal")]
    pub impairment_loss_to_other_assets: Decimal,

    /// Group presentation currency.
    pub currency: String,
}

impl CguImpairmentTest {
    /// Run the IAS 36 § 18 / § 104 test, producing a
    /// [`CguImpairmentResult`].  Pure function — no I/O, no global
    /// state.
    ///
    /// 1. **Carrying total** = `allocated_goodwill + other_carrying`.
    /// 2. **Recoverable amount** = `max(fair_value_less_costs, value_in_use)`.
    /// 3. **Total loss** = `max(0, carrying − recoverable)`.
    /// 4. **Allocation** (IAS 36 § 104): first to goodwill (capped at
    ///    `allocated_goodwill`), then the residual to other assets.
    pub fn run(&self) -> CguImpairmentResult {
        let carrying_total = self.allocated_goodwill + self.other_carrying;
        let recoverable_amount = self.fair_value_less_costs.max(self.value_in_use);
        let impairment_loss_total = (carrying_total - recoverable_amount).max(Decimal::ZERO);
        let impairment_loss_to_goodwill = impairment_loss_total.min(self.allocated_goodwill);
        let impairment_loss_to_other_assets = impairment_loss_total - impairment_loss_to_goodwill;

        CguImpairmentResult {
            cgu_id: self.cgu_id.clone(),
            test_date: self.test_date,
            carrying_total,
            recoverable_amount,
            impairment_loss_total,
            impairment_loss_to_goodwill,
            impairment_loss_to_other_assets,
            currency: self.currency.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()
    }

    fn impaired_test_sample() -> CguImpairmentTest {
        // CGU carrying = 100k goodwill + 900k other = 1.0M.
        // Fair value less costs = 750k; VIU = 800k → recoverable = 800k.
        // Impairment = 1.0M − 800k = 200k.
        // Allocation: 100k to goodwill (caps it out), 100k pro rata to other.
        CguImpairmentTest {
            cgu_id: "CGU-EMEA".to_string(),
            test_date: date(),
            allocated_goodwill: dec!(100_000),
            other_carrying: dec!(900_000),
            fair_value_less_costs: dec!(750_000),
            value_in_use: dec!(800_000),
            currency: "EUR".to_string(),
        }
    }

    #[test]
    fn impaired_cgu_allocates_to_goodwill_first() {
        let result = impaired_test_sample().run();
        assert_eq!(result.carrying_total, dec!(1_000_000));
        assert_eq!(result.recoverable_amount, dec!(800_000));
        assert_eq!(result.impairment_loss_total, dec!(200_000));
        assert_eq!(
            result.impairment_loss_to_goodwill,
            dec!(100_000),
            "IAS 36 § 104 — goodwill takes the first hit, capped at allocated amount"
        );
        assert_eq!(
            result.impairment_loss_to_other_assets,
            dec!(100_000),
            "residual flows pro-rata to other assets"
        );
    }

    #[test]
    fn recoverable_cgu_has_no_impairment() {
        // VIU well above carrying → no impairment.
        let mut test = impaired_test_sample();
        test.value_in_use = dec!(1_500_000);
        let result = test.run();
        assert_eq!(result.recoverable_amount, dec!(1_500_000));
        assert_eq!(result.impairment_loss_total, Decimal::ZERO);
        assert_eq!(result.impairment_loss_to_goodwill, Decimal::ZERO);
        assert_eq!(result.impairment_loss_to_other_assets, Decimal::ZERO);
    }

    #[test]
    fn impairment_smaller_than_goodwill_only_hits_goodwill() {
        // Carrying = 1.0M, recoverable = 950k → 50k loss.
        // 50k < 100k goodwill, so 100% to goodwill, 0 to other.
        let mut test = impaired_test_sample();
        test.fair_value_less_costs = dec!(950_000);
        test.value_in_use = dec!(900_000);
        let result = test.run();
        assert_eq!(result.impairment_loss_total, dec!(50_000));
        assert_eq!(result.impairment_loss_to_goodwill, dec!(50_000));
        assert_eq!(result.impairment_loss_to_other_assets, Decimal::ZERO);
    }

    #[test]
    fn impairment_uses_higher_of_fv_and_viu() {
        // FV = 700k > VIU = 600k → recoverable = 700k.
        let mut test = impaired_test_sample();
        test.fair_value_less_costs = dec!(700_000);
        test.value_in_use = dec!(600_000);
        let result = test.run();
        assert_eq!(
            result.recoverable_amount,
            dec!(700_000),
            "recoverable = max(FV, VIU) per IAS 36 § 18"
        );
    }

    #[test]
    fn cgu_with_no_allocated_goodwill_only_impairs_other_assets() {
        // Pure CGU impairment test (no goodwill component).  Allowed
        // even though IAS 36 § 10 only mandates annual testing for
        // CGUs containing goodwill — entities can still test other
        // CGUs on indication.
        let test = CguImpairmentTest {
            cgu_id: "CGU-RND".to_string(),
            test_date: date(),
            allocated_goodwill: Decimal::ZERO,
            other_carrying: dec!(500_000),
            fair_value_less_costs: dec!(420_000),
            value_in_use: dec!(400_000),
            currency: "EUR".to_string(),
        };
        let result = test.run();
        assert_eq!(result.impairment_loss_total, dec!(80_000));
        assert_eq!(result.impairment_loss_to_goodwill, Decimal::ZERO);
        assert_eq!(result.impairment_loss_to_other_assets, dec!(80_000));
    }

    #[test]
    fn cgu_test_round_trips_via_serde() {
        let test = impaired_test_sample();
        let json = serde_json::to_string(&test).unwrap();
        let back: CguImpairmentTest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, test);

        let result = test.run();
        let result_json = serde_json::to_string(&result).unwrap();
        let result_back: CguImpairmentResult = serde_json::from_str(&result_json).unwrap();
        assert_eq!(result_back, result);
    }

    #[test]
    fn cgu_with_segment_carries_segment_code() {
        let cgu = CashGeneratingUnit::new(
            "CGU-EMEA",
            "EMEA Consumer",
            vec!["ACME_DE".to_string(), "ACME_FR".to_string()],
        )
        .with_segment("SEG-CONSUMER");
        assert_eq!(cgu.segment_code.as_deref(), Some("SEG-CONSUMER"));
        assert_eq!(cgu.member_entity_codes.len(), 2);
    }

    #[test]
    fn goodwill_allocation_round_trips() {
        let alloc = GoodwillAllocation {
            cgu_id: "CGU-EMEA".to_string(),
            business_combination_id: "BC-001".to_string(),
            goodwill_amount: dec!(750_000),
            allocation_date: date(),
        };
        let json = serde_json::to_string(&alloc).unwrap();
        let back: GoodwillAllocation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, alloc);
    }
}

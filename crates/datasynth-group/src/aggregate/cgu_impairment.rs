//! IAS 36 § 10 CGU goodwill impairment runner — aggregate phase.
//!
//! Joins three inputs to produce per-CGU impairment results:
//!
//! 1. **CGU plan** from [`crate::manifest::CguPlan`] — definitional
//!    CGUs + acquisition-date goodwill allocations carried in the
//!    manifest (PR #152).
//! 2. **Per-period test inputs** ([`CguTestInputs`]) supplied by the
//!    caller via [`crate::aggregate::driver::AggregateOptions`] —
//!    one entry per CGU being tested this period, carrying the
//!    fair-value-less-costs and value-in-use estimates from the
//!    engagement's external valuation source.
//! 3. **Period end + currency** from [`crate::manifest::GroupManifest`].
//!
//! For each CGU with both (a) a manifest definition and (b) a test
//! input, the runner:
//!
//! 1. Sums the goodwill allocated to the CGU from
//!    [`crate::manifest::CguPlan::goodwill_allocations`].
//! 2. Builds a [`datasynth_core::models::CguImpairmentTest`] using
//!    the summed goodwill, the input's other-asset carrying, FV-less-
//!    costs, and VIU.
//! 3. Calls
//!    [`datasynth_core::models::CguImpairmentTest::run`] which applies
//!    IAS 36 § 18 (recoverable = max(FV-less-costs, VIU)) and
//!    IAS 36 § 104 (allocate impairment loss first to goodwill, then
//!    pro-rata to other assets) — pure function, no I/O.
//!
//! # Validation
//!
//! - A `CguTestInputs.cgu_id` that doesn't reference any CGU in the
//!   manifest plan produces a [`crate::errors::GroupError::Aggregate`]
//!   error.
//! - All three numeric inputs (`other_carrying`, `fair_value_less_costs`,
//!   `value_in_use`) must be non-negative; negative values produce a
//!   typed error naming the offending CGU + field.
//! - When the manifest's CGU plan is empty AND `cgu_test_inputs` is
//!   empty, the runner returns an empty result vector and the driver
//!   skips emitting the artefact entirely (preserves backwards
//!   compatibility byte-for-byte for engagements with no CGU
//!   configuration).
//! - When `cgu_test_inputs` references a CGU that has no goodwill
//!   allocation in the plan, the test still runs (with
//!   `allocated_goodwill = 0`); the result records show zero goodwill
//!   impairment and any loss falls entirely to other assets.  This
//!   models a CGU with no goodwill but other indicators of impairment.
//!
//! # Determinism
//!
//! Results are sorted by `cgu_id` for byte-identical ordering across
//! runs with identical inputs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use datasynth_core::models::{CguImpairmentResult, CguImpairmentTest};

use crate::errors::{GroupError, GroupResult};
use crate::manifest::CguPlan;

// ── Public types ──────────────────────────────────────────────────────────────

/// Per-period CGU test inputs — supplied by the caller (typically
/// loaded from an engagement-specific valuation file).  One entry per
/// CGU under test this period.
///
/// The fair-value and value-in-use estimates are external inputs
/// (driven by the engagement's discounted-cash-flow model + external
/// market data); the runner does not derive them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CguTestInputs {
    /// CGU under test (must reference a `cgu_id` in the manifest's
    /// [`CguPlan::cgus`]).
    pub cgu_id: String,
    /// Carrying amount of the **other assets** of the CGU (everything
    /// except the allocated goodwill) immediately before the test, in
    /// the group presentation currency.  Always non-negative.
    ///
    /// **`None` (the default) derives this from the actually-generated
    /// trial balances** — the summed net assets of the CGU's
    /// `member_entity_codes` (per `classify_account`) less the allocated
    /// goodwill — so the impairment test reconciles to the consolidated
    /// balance sheet. Supply `Some(_)` to override with an external amount.
    #[serde(default)]
    pub other_carrying: Option<Decimal>,
    /// Fair value of the CGU less costs of disposal at the test date.
    /// Required together with `value_in_use` unless `recoverable_ratio` is set.
    #[serde(default)]
    pub fair_value_less_costs: Option<Decimal>,
    /// Value in use of the CGU at the test date (PV of future net cash
    /// flows + terminal value at the WACC). Required together with
    /// `fair_value_less_costs` unless `recoverable_ratio` is set.
    #[serde(default)]
    pub value_in_use: Option<Decimal>,
    /// **Recoverable amount expressed as a ratio of the CGU's carrying
    /// amount.** When set, `recoverable = (allocated_goodwill +
    /// other_carrying) * recoverable_ratio`, overriding
    /// `fair_value_less_costs` / `value_in_use` — so the recoverable amount
    /// stays internally consistent with the (TB-derived) carrying rather than
    /// floating free of the financials (e.g. `0.9` → a real 10% shortfall,
    /// `1.2` → headroom). Must be non-negative.
    #[serde(default)]
    pub recoverable_ratio: Option<Decimal>,
}

// ── Filename and path constants ────────────────────────────────────────────────

/// Filename emitted under `{out_dir}/consolidated/` when at least one
/// impairment test ran for the period.
pub const CGU_IMPAIRMENT_TESTS_FILENAME: &str = "cgu_impairment_tests.json";

// ── Public API ────────────────────────────────────────────────────────────────

/// Run IAS 36 § 18 / § 104 impairment tests for every CGU in
/// `test_inputs`, joining each input against the manifest plan to
/// pull the allocated goodwill.  Returns sorted-by-`cgu_id` results.
///
/// Returns an empty vector (without error) when `test_inputs` is
/// empty — engagements that don't supply test inputs skip the
/// impairment test phase entirely.
///
/// # Errors
///
/// - [`GroupError::Aggregate`] when a `CguTestInputs.cgu_id` doesn't
///   reference any CGU in the manifest plan.
/// - [`GroupError::Aggregate`] when any of `other_carrying`,
///   `fair_value_less_costs`, or `value_in_use` is negative.
pub fn run_cgu_impairment_tests(
    cgu_plan: &CguPlan,
    test_inputs: &[CguTestInputs],
    entity_net_assets: &BTreeMap<String, Decimal>,
    test_date: NaiveDate,
    currency: &str,
) -> GroupResult<Vec<CguImpairmentResult>> {
    if test_inputs.is_empty() {
        return Ok(Vec::new());
    }

    let mut results: Vec<CguImpairmentResult> = Vec::with_capacity(test_inputs.len());

    for input in test_inputs {
        // Validate cgu_id references a defined CGU; capture its members so a
        // missing `other_carrying` can be derived from their trial balances.
        let cgu = cgu_plan
            .cgus
            .iter()
            .find(|c| c.cgu_id == input.cgu_id)
            .ok_or_else(|| {
                GroupError::Aggregate(format!(
                    "cgu impairment: test input references cgu_id `{}` which has no matching definition in the manifest plan",
                    input.cgu_id,
                ))
            })?;

        // Sum goodwill allocations for this CGU.
        let allocated_goodwill: Decimal = cgu_plan
            .goodwill_allocations
            .iter()
            .filter(|a| a.cgu_id == input.cgu_id)
            .map(|a| a.goodwill_amount)
            .sum();

        // Other-asset carrying: explicit override, else derived from the member
        // entities' net assets in the generated TBs less the allocated goodwill,
        // so the test reconciles to the consolidated balance sheet (clamped >=0).
        let other_carrying = match input.other_carrying {
            Some(c) => c,
            None => {
                let members_net: Decimal = cgu
                    .member_entity_codes
                    .iter()
                    .map(|e| entity_net_assets.get(e).copied().unwrap_or(Decimal::ZERO))
                    .sum();
                (members_net - allocated_goodwill).max(Decimal::ZERO)
            }
        };
        if other_carrying < Decimal::ZERO {
            return Err(GroupError::Aggregate(format!(
                "cgu impairment: cgu `{}` has negative other_carrying `{other_carrying}`",
                input.cgu_id,
            )));
        }

        // Recoverable amount: a ratio of the (BS-coherent) carrying when
        // `recoverable_ratio` is set, else max(FV-less-costs, VIU). Both are
        // injected into the test as FV-less-costs / VIU so `run`'s max() yields
        // the intended recoverable amount.
        let (fair_value_less_costs, value_in_use) = if let Some(ratio) = input.recoverable_ratio {
            if ratio < Decimal::ZERO {
                return Err(GroupError::Aggregate(format!(
                    "cgu impairment: cgu `{}` has negative recoverable_ratio `{ratio}`",
                    input.cgu_id,
                )));
            }
            let recoverable = (allocated_goodwill + other_carrying) * ratio;
            (recoverable, recoverable)
        } else {
            let fvlc = input.fair_value_less_costs.ok_or_else(|| {
                GroupError::Aggregate(format!(
                    "cgu impairment: cgu `{}` needs fair_value_less_costs + value_in_use (or recoverable_ratio)",
                    input.cgu_id,
                ))
            })?;
            let viu = input.value_in_use.ok_or_else(|| {
                GroupError::Aggregate(format!(
                    "cgu impairment: cgu `{}` needs value_in_use (or recoverable_ratio)",
                    input.cgu_id,
                ))
            })?;
            for (label, val) in [("fair_value_less_costs", fvlc), ("value_in_use", viu)] {
                if val < Decimal::ZERO {
                    return Err(GroupError::Aggregate(format!(
                        "cgu impairment: test input for cgu `{}` has negative {label} `{val}`",
                        input.cgu_id,
                    )));
                }
            }
            (fvlc, viu)
        };

        let test = CguImpairmentTest {
            cgu_id: input.cgu_id.clone(),
            test_date,
            allocated_goodwill,
            other_carrying,
            fair_value_less_costs,
            value_in_use,
            currency: currency.to_string(),
        };
        results.push(test.run());
    }

    results.sort_by(|a, b| a.cgu_id.cmp(&b.cgu_id));
    Ok(results)
}

/// Serialise + write `results` to
/// `{out_dir}/consolidated/cgu_impairment_tests.json`.  No-op (returns
/// `Ok(None)`) when `results` is empty so the artefact is omitted from
/// archives that ran no impairment tests.
///
/// Returns the absolute path written (when non-empty) so the driver
/// can include it in [`crate::aggregate::AggregateSummary::artifacts_written`].
pub fn write_cgu_impairment_tests(
    out_dir: &Path,
    results: &[CguImpairmentResult],
) -> GroupResult<Option<PathBuf>> {
    if results.is_empty() {
        return Ok(None);
    }
    let dir = out_dir.join(crate::aggregate::translation::cta::CONSOLIDATED_SUBDIR);
    std::fs::create_dir_all(&dir).map_err(|e| {
        GroupError::Aggregate(format!(
            "cgu impairment: cannot create directory `{}`: {e}",
            dir.display()
        ))
    })?;
    let path = dir.join(CGU_IMPAIRMENT_TESTS_FILENAME);
    let json = serde_json::to_string_pretty(results).map_err(|e| {
        GroupError::Aggregate(format!(
            "cgu impairment: failed to serialise results to JSON: {e}"
        ))
    })?;
    std::fs::write(&path, json).map_err(|e| {
        GroupError::Aggregate(format!(
            "cgu impairment: cannot write `{}`: {e}",
            path.display()
        ))
    })?;
    Ok(Some(path))
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use datasynth_core::models::{CashGeneratingUnit, GoodwillAllocation};
    use rust_decimal_macros::dec;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()
    }

    fn plan_with(cgus: Vec<&str>, allocs: Vec<(&str, &str, Decimal)>) -> CguPlan {
        CguPlan {
            cgus: cgus
                .into_iter()
                .map(|id| CashGeneratingUnit::new(id, format!("name-{id}"), vec!["E1".to_string()]))
                .collect(),
            goodwill_allocations: allocs
                .into_iter()
                .map(|(cgu_id, bc_id, amt)| GoodwillAllocation {
                    cgu_id: cgu_id.to_string(),
                    business_combination_id: bc_id.to_string(),
                    goodwill_amount: amt,
                    allocation_date: date(),
                })
                .collect(),
        }
    }

    /// Build `CguTestInputs` with explicit carrying + FV/VIU (pre-#120 shape).
    fn ti(cgu_id: &str, oc: Decimal, fvlc: Decimal, viu: Decimal) -> CguTestInputs {
        CguTestInputs {
            cgu_id: cgu_id.to_string(),
            other_carrying: Some(oc),
            fair_value_less_costs: Some(fvlc),
            value_in_use: Some(viu),
            recoverable_ratio: None,
        }
    }

    #[test]
    fn other_carrying_derived_from_member_entity_net_assets() {
        // CGU_X members E1 (net assets 900) + E2 (300) = 1200; goodwill 100.
        // other_carrying derived = 1200 - 100 = 1100; carrying_total = 1200.
        let plan = CguPlan {
            cgus: vec![CashGeneratingUnit::new(
                "CGU_X",
                "name",
                vec!["E1".to_string(), "E2".to_string()],
            )],
            goodwill_allocations: vec![GoodwillAllocation {
                cgu_id: "CGU_X".to_string(),
                business_combination_id: "BC".to_string(),
                goodwill_amount: dec!(100),
                allocation_date: date(),
            }],
        };
        let net_assets: BTreeMap<String, Decimal> =
            [("E1".to_string(), dec!(900)), ("E2".to_string(), dec!(300))]
                .into_iter()
                .collect();
        let inputs = vec![CguTestInputs {
            cgu_id: "CGU_X".to_string(),
            other_carrying: None, // derive from TBs
            fair_value_less_costs: Some(dec!(2000)),
            value_in_use: Some(dec!(0)),
            recoverable_ratio: None,
        }];
        let results = run_cgu_impairment_tests(&plan, &inputs, &net_assets, date(), "EUR").unwrap();
        assert_eq!(results[0].carrying_total, dec!(1200));
        assert_eq!(results[0].impairment_loss_total, Decimal::ZERO); // 1200 < 2000
    }

    #[test]
    fn recoverable_ratio_is_a_coherent_multiple_of_carrying() {
        // goodwill 100 + other 900 = 1000 carrying; ratio 0.9 -> recoverable 900
        // -> impairment 100 (all to goodwill).
        let plan = plan_with(vec!["CGU_X"], vec![("CGU_X", "BC", dec!(100))]);
        let inputs = vec![CguTestInputs {
            cgu_id: "CGU_X".to_string(),
            other_carrying: Some(dec!(900)),
            fair_value_less_costs: None,
            value_in_use: None,
            recoverable_ratio: Some(dec!(0.9)),
        }];
        let results =
            run_cgu_impairment_tests(&plan, &inputs, &BTreeMap::new(), date(), "EUR").unwrap();
        assert_eq!(results[0].carrying_total, dec!(1000));
        assert_eq!(results[0].recoverable_amount, dec!(900));
        assert_eq!(results[0].impairment_loss_total, dec!(100));
        assert_eq!(results[0].impairment_loss_to_goodwill, dec!(100));
    }

    #[test]
    fn empty_inputs_returns_empty_results_no_error() {
        let plan = plan_with(vec!["CGU_X"], vec![]);
        let results =
            run_cgu_impairment_tests(&plan, &[], &BTreeMap::new(), date(), "EUR").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn happy_path_recoverable_no_impairment() {
        // CGU_X: goodwill 100, other 500, FV 800, VIU 700
        // recoverable = max(800, 700) = 800
        // carrying total = 100 + 500 = 600
        // 600 < 800 → no impairment
        let plan = plan_with(vec!["CGU_X"], vec![("CGU_X", "BC_001", dec!(100))]);
        let inputs = vec![ti("CGU_X", dec!(500), dec!(800), dec!(700))];
        let results =
            run_cgu_impairment_tests(&plan, &inputs, &BTreeMap::new(), date(), "EUR").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].cgu_id, "CGU_X");
        assert_eq!(results[0].carrying_total, dec!(600));
        assert_eq!(results[0].recoverable_amount, dec!(800));
        assert_eq!(results[0].impairment_loss_total, Decimal::ZERO);
        assert_eq!(results[0].impairment_loss_to_goodwill, Decimal::ZERO);
        assert_eq!(results[0].impairment_loss_to_other_assets, Decimal::ZERO);
    }

    #[test]
    fn impairment_loss_first_allocated_to_goodwill_per_ias36_para104() {
        // CGU_X: goodwill 100, other 500, FV 400, VIU 350
        // recoverable = max(400, 350) = 400
        // carrying total = 600
        // total impairment = 600 - 400 = 200
        // loss to goodwill = min(200, 100) = 100
        // loss to other = 200 - 100 = 100
        let plan = plan_with(vec!["CGU_X"], vec![("CGU_X", "BC_001", dec!(100))]);
        let inputs = vec![ti("CGU_X", dec!(500), dec!(400), dec!(350))];
        let results =
            run_cgu_impairment_tests(&plan, &inputs, &BTreeMap::new(), date(), "EUR").unwrap();
        assert_eq!(results[0].impairment_loss_total, dec!(200));
        assert_eq!(results[0].impairment_loss_to_goodwill, dec!(100));
        assert_eq!(results[0].impairment_loss_to_other_assets, dec!(100));
    }

    #[test]
    fn multiple_allocations_to_same_cgu_sum() {
        // CGU_X has three goodwill allocations: 50 + 30 + 20 = 100
        let plan = plan_with(
            vec!["CGU_X"],
            vec![
                ("CGU_X", "BC_001", dec!(50)),
                ("CGU_X", "BC_002", dec!(30)),
                ("CGU_X", "BC_003", dec!(20)),
            ],
        );
        let inputs = vec![ti("CGU_X", dec!(0), dec!(60), dec!(0))];
        let results =
            run_cgu_impairment_tests(&plan, &inputs, &BTreeMap::new(), date(), "EUR").unwrap();
        // carrying = 100 + 0 = 100; recoverable = 60; loss = 40 (all to goodwill since 40 < 100)
        assert_eq!(results[0].carrying_total, dec!(100));
        assert_eq!(results[0].impairment_loss_total, dec!(40));
        assert_eq!(results[0].impairment_loss_to_goodwill, dec!(40));
        assert_eq!(results[0].impairment_loss_to_other_assets, Decimal::ZERO);
    }

    #[test]
    fn cgu_with_no_goodwill_allocation_runs_with_zero_goodwill() {
        // CGU_NO_GW exists but has no goodwill allocations
        let plan = plan_with(vec!["CGU_NO_GW"], vec![]);
        let inputs = vec![ti("CGU_NO_GW", dec!(1000), dec!(800), dec!(750))];
        let results =
            run_cgu_impairment_tests(&plan, &inputs, &BTreeMap::new(), date(), "EUR").unwrap();
        // carrying = 0 + 1000 = 1000; recoverable = max(800, 750) = 800
        // loss = 200; goodwill share = 0 (none allocated); other = 200
        assert_eq!(results[0].carrying_total, dec!(1000));
        assert_eq!(results[0].impairment_loss_total, dec!(200));
        assert_eq!(results[0].impairment_loss_to_goodwill, Decimal::ZERO);
        assert_eq!(results[0].impairment_loss_to_other_assets, dec!(200));
    }

    #[test]
    fn unknown_cgu_id_in_inputs_rejected() {
        let plan = plan_with(vec!["DEFINED"], vec![]);
        let inputs = vec![ti("GHOST", dec!(0), dec!(0), dec!(0))];
        let err =
            run_cgu_impairment_tests(&plan, &inputs, &BTreeMap::new(), date(), "EUR").unwrap_err();
        assert!(format!("{err}").contains("no matching definition"));
    }

    #[test]
    fn negative_input_rejected() {
        let plan = plan_with(vec!["CGU_X"], vec![]);
        let inputs = vec![ti("CGU_X", dec!(-1), dec!(0), dec!(0))];
        let err =
            run_cgu_impairment_tests(&plan, &inputs, &BTreeMap::new(), date(), "EUR").unwrap_err();
        assert!(format!("{err}").contains("negative other_carrying"));
    }

    #[test]
    fn results_sorted_by_cgu_id_for_determinism() {
        let plan = plan_with(vec!["CGU_C", "CGU_A", "CGU_B"], vec![]);
        let inputs = vec![
            ti("CGU_C", dec!(0), dec!(100), dec!(0)),
            ti("CGU_A", dec!(0), dec!(100), dec!(0)),
            ti("CGU_B", dec!(0), dec!(100), dec!(0)),
        ];
        let results =
            run_cgu_impairment_tests(&plan, &inputs, &BTreeMap::new(), date(), "EUR").unwrap();
        let ids: Vec<&str> = results.iter().map(|r| r.cgu_id.as_str()).collect();
        assert_eq!(ids, vec!["CGU_A", "CGU_B", "CGU_C"]);
    }

    #[test]
    fn write_skipped_when_results_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let result = write_cgu_impairment_tests(tmp.path(), &[]).unwrap();
        assert!(result.is_none());
        // No directory was created either
        let dir = tmp.path().join("consolidated");
        assert!(
            !dir.exists() || std::fs::read_dir(&dir).unwrap().next().is_none(),
            "consolidated/ should be empty when no impairment results"
        );
    }

    #[test]
    fn write_emits_pretty_json_file_at_canonical_path() {
        let tmp = tempfile::tempdir().unwrap();
        let plan = plan_with(vec!["CGU_X"], vec![("CGU_X", "BC_001", dec!(100))]);
        let inputs = vec![ti("CGU_X", dec!(500), dec!(400), dec!(350))];
        let results =
            run_cgu_impairment_tests(&plan, &inputs, &BTreeMap::new(), date(), "EUR").unwrap();
        let path = write_cgu_impairment_tests(tmp.path(), &results)
            .unwrap()
            .expect("must return Some path when results non-empty");
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("cgu_impairment_tests.json")
        );
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("\"cgu_id\": \"CGU_X\""));
        assert!(content.contains("\"impairment_loss_to_goodwill\""));
    }
}

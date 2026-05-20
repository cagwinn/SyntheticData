//! IAS 36 § 10 cash-generating-unit (CGU) plan resolution.
//!
//! Lifts an engagement-static [`crate::config::CguConfig`] into a
//! [`CguPlan`] consumed by the aggregate phase to drive annual
//! goodwill impairment testing.  The plan is **definitional only** —
//! it carries the CGU shape (id, name, members, segment) and the
//! acquisition-date goodwill allocations that persist across periods.
//!
//! Per-period test inputs (fair value less costs of disposal, value
//! in use) live elsewhere; this module's contract is to produce a
//! validated, deterministic, ordering-stable plan that the aggregate
//! phase can join against per-period FV / VIU estimates to run
//! `datasynth_core::models::cgu::CguImpairmentTest::run` per CGU.
//!
//! # Validation contract
//!
//! - Each [`crate::config::CguDefinitionEntry::cgu_id`] must be unique
//!   within the engagement.
//! - Each [`crate::config::CguDefinitionEntry::member_entity_codes`]
//!   list must be non-empty (a CGU with no members would aggregate
//!   no cash flows).
//! - Each member entity code must reference an entity that exists in
//!   the resolved ownership graph — a dangling member code is a
//!   manifest-time configuration error.
//! - Each [`crate::config::CguGoodwillAllocationEntry::cgu_id`] must
//!   reference an entry in `cgus` (allocations to undefined CGUs are
//!   a manifest-time configuration error).
//! - Each goodwill amount must be non-negative.
//!
//! Cross-validation of `business_combination_id` against the BC
//! files written per-shard is **not** performed here — those files
//! are produced after the manifest phase, so the check belongs to
//! the aggregate-phase wiring (PR follow-up).

use std::collections::BTreeSet;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use datasynth_core::models::{CashGeneratingUnit, GoodwillAllocation};

use crate::config::CguConfig;
use crate::errors::{GroupError, GroupResult};
use crate::manifest::expansion::ExpandedEntity;

// ── Public types ──────────────────────────────────────────────────────────────

/// IAS 36 § 10 CGU plan — output of [`build_cgu_plan`].
///
/// Wraps the standards-domain types from
/// `datasynth_core::models::cgu` so the manifest carries the same
/// shape that downstream consumers (aggregate-phase impairment-test
/// runner, IFRS 8 segment reconciliation) operate on.  Empty when the
/// engagement supplied no [`CguConfig`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CguPlan {
    /// CGU definitions, sorted by `cgu_id` for determinism.
    pub cgus: Vec<CashGeneratingUnit>,
    /// Goodwill allocations, sorted by `(cgu_id, business_combination_id)`
    /// for determinism.
    pub goodwill_allocations: Vec<GoodwillAllocation>,
}

// ── Public builder ────────────────────────────────────────────────────────────

/// Build the [`CguPlan`] from config + the resolved entity list.
///
/// Returns an empty plan when `cfg.cgus` is empty (engagements with
/// no CGU configuration skip impairment testing entirely).
///
/// # Errors
///
/// - Duplicate `cgu_id` across [`CguConfig::cgus`].
/// - Empty `member_entity_codes` on a CGU.
/// - A member entity code that does not resolve to any entity in
///   the expanded entity list.
/// - A goodwill allocation referencing a `cgu_id` that has no
///   matching definition.
/// - A negative `goodwill_amount`.
pub fn build_cgu_plan(cfg: &CguConfig, entities: &[ExpandedEntity]) -> GroupResult<CguPlan> {
    if cfg.cgus.is_empty() && cfg.goodwill_allocations.is_empty() {
        return Ok(CguPlan::default());
    }

    let entity_codes: BTreeSet<&str> = entities.iter().map(|e| e.code.as_str()).collect();

    // ── Validate CGU definitions ──────────────────────────────────────────
    let mut seen_cgu_ids: BTreeSet<&str> = BTreeSet::new();
    for cgu in &cfg.cgus {
        if !seen_cgu_ids.insert(cgu.cgu_id.as_str()) {
            return Err(GroupError::Config(format!(
                "cgu plan: duplicate cgu_id `{}` — every CGU must have a unique id",
                cgu.cgu_id,
            )));
        }
        if cgu.member_entity_codes.is_empty() {
            return Err(GroupError::Config(format!(
                "cgu plan: cgu `{}` has no member_entity_codes — a CGU must aggregate at least one entity's cash flows",
                cgu.cgu_id,
            )));
        }
        for member in &cgu.member_entity_codes {
            if !entity_codes.contains(member.as_str()) {
                return Err(GroupError::Config(format!(
                    "cgu plan: cgu `{}` references member entity `{}` which is not present in the ownership graph",
                    cgu.cgu_id, member,
                )));
            }
        }
    }

    // ── Validate goodwill allocations ─────────────────────────────────────
    for alloc in &cfg.goodwill_allocations {
        if !seen_cgu_ids.contains(alloc.cgu_id.as_str()) {
            return Err(GroupError::Config(format!(
                "cgu plan: goodwill allocation references cgu_id `{}` (BC `{}`) which has no matching CGU definition",
                alloc.cgu_id, alloc.business_combination_id,
            )));
        }
        if alloc.goodwill_amount < Decimal::ZERO {
            return Err(GroupError::Config(format!(
                "cgu plan: goodwill allocation for cgu `{}` (BC `{}`) has negative goodwill_amount `{}` — bargain purchases produce no allocation row",
                alloc.cgu_id, alloc.business_combination_id, alloc.goodwill_amount,
            )));
        }
    }

    // ── Lift to standards-domain types, sorted for determinism ────────────
    let mut cgus: Vec<CashGeneratingUnit> = cfg
        .cgus
        .iter()
        .map(|c| {
            let cgu = CashGeneratingUnit::new(
                c.cgu_id.clone(),
                c.name.clone(),
                c.member_entity_codes.clone(),
            );
            match &c.segment_code {
                Some(seg) => cgu.with_segment(seg.clone()),
                None => cgu,
            }
        })
        .collect();
    cgus.sort_by(|a, b| a.cgu_id.cmp(&b.cgu_id));

    let mut goodwill_allocations: Vec<GoodwillAllocation> = cfg
        .goodwill_allocations
        .iter()
        .map(|a| GoodwillAllocation {
            cgu_id: a.cgu_id.clone(),
            business_combination_id: a.business_combination_id.clone(),
            goodwill_amount: a.goodwill_amount,
            allocation_date: a.allocation_date,
        })
        .collect();
    goodwill_allocations.sort_by(|a, b| {
        a.cgu_id
            .cmp(&b.cgu_id)
            .then_with(|| a.business_combination_id.cmp(&b.business_combination_id))
    });

    Ok(CguPlan {
        cgus,
        goodwill_allocations,
    })
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CguDefinitionEntry, CguGoodwillAllocationEntry, ConsolidationMethod};
    use crate::manifest::expansion::EntitySource;
    use chrono::NaiveDate;
    use datasynth_core::models::HyperinflationStatus;
    use rust_decimal_macros::dec;

    fn entity(code: &str) -> ExpandedEntity {
        ExpandedEntity {
            code: code.to_string(),
            name: None,
            country: "DE".to_string(),
            functional_currency: "EUR".to_string(),
            scoping_profile: "significant".to_string(),
            consolidation_method: ConsolidationMethod::Full,
            ownership_percent: Some(dec!(0.80)),
            parent_code: Some("PARENT".to_string()),
            accounting_framework: None,
            industry: None,
            hyperinflation_status: HyperinflationStatus::NotHyperinflationary,
            ownership_changes: Vec::new(),
            source: EntitySource::Explicit,
            generated_block_index: None,
            rows: None,
        }
    }

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2023, 6, 30).unwrap()
    }

    #[test]
    fn empty_config_produces_empty_plan() {
        let plan = build_cgu_plan(&CguConfig::default(), &[]).unwrap();
        assert!(plan.cgus.is_empty());
        assert!(plan.goodwill_allocations.is_empty());
    }

    #[test]
    fn happy_path_lifts_definitions_and_allocations() {
        let cfg = CguConfig {
            cgus: vec![
                CguDefinitionEntry {
                    cgu_id: "CGU_EMEA".to_string(),
                    name: "EMEA".to_string(),
                    member_entity_codes: vec!["E1".to_string(), "E2".to_string()],
                    segment_code: Some("CONS".to_string()),
                },
                CguDefinitionEntry {
                    cgu_id: "CGU_AMER".to_string(),
                    name: "Americas".to_string(),
                    member_entity_codes: vec!["E3".to_string()],
                    segment_code: None,
                },
            ],
            goodwill_allocations: vec![CguGoodwillAllocationEntry {
                cgu_id: "CGU_EMEA".to_string(),
                business_combination_id: "BC_2023_001".to_string(),
                goodwill_amount: dec!(150000),
                allocation_date: date(),
            }],
        };
        let entities = vec![entity("E1"), entity("E2"), entity("E3")];
        let plan = build_cgu_plan(&cfg, &entities).unwrap();
        assert_eq!(plan.cgus.len(), 2);
        // Sorted by cgu_id for determinism: AMER < EMEA
        assert_eq!(plan.cgus[0].cgu_id, "CGU_AMER");
        assert_eq!(plan.cgus[1].cgu_id, "CGU_EMEA");
        assert_eq!(plan.cgus[1].segment_code.as_deref(), Some("CONS"));
        assert_eq!(plan.goodwill_allocations.len(), 1);
        assert_eq!(plan.goodwill_allocations[0].goodwill_amount, dec!(150000));
    }

    #[test]
    fn duplicate_cgu_id_rejected() {
        let cfg = CguConfig {
            cgus: vec![
                CguDefinitionEntry {
                    cgu_id: "DUP".to_string(),
                    name: "first".to_string(),
                    member_entity_codes: vec!["E1".to_string()],
                    segment_code: None,
                },
                CguDefinitionEntry {
                    cgu_id: "DUP".to_string(),
                    name: "second".to_string(),
                    member_entity_codes: vec!["E2".to_string()],
                    segment_code: None,
                },
            ],
            ..Default::default()
        };
        let entities = vec![entity("E1"), entity("E2")];
        let err = build_cgu_plan(&cfg, &entities).unwrap_err();
        assert!(format!("{err}").contains("duplicate cgu_id"));
    }

    #[test]
    fn empty_members_rejected() {
        let cfg = CguConfig {
            cgus: vec![CguDefinitionEntry {
                cgu_id: "CGU_X".to_string(),
                name: "x".to_string(),
                member_entity_codes: vec![],
                segment_code: None,
            }],
            ..Default::default()
        };
        let err = build_cgu_plan(&cfg, &[]).unwrap_err();
        assert!(format!("{err}").contains("no member_entity_codes"));
    }

    #[test]
    fn unknown_member_entity_rejected() {
        let cfg = CguConfig {
            cgus: vec![CguDefinitionEntry {
                cgu_id: "CGU_X".to_string(),
                name: "x".to_string(),
                member_entity_codes: vec!["NOPE".to_string()],
                segment_code: None,
            }],
            ..Default::default()
        };
        let err = build_cgu_plan(&cfg, &[entity("E1")]).unwrap_err();
        assert!(format!("{err}").contains("not present in the ownership graph"));
    }

    #[test]
    fn allocation_to_undefined_cgu_rejected() {
        let cfg = CguConfig {
            cgus: vec![CguDefinitionEntry {
                cgu_id: "DEFINED".to_string(),
                name: "n".to_string(),
                member_entity_codes: vec!["E1".to_string()],
                segment_code: None,
            }],
            goodwill_allocations: vec![CguGoodwillAllocationEntry {
                cgu_id: "GHOST".to_string(),
                business_combination_id: "BC_X".to_string(),
                goodwill_amount: dec!(100),
                allocation_date: date(),
            }],
        };
        let err = build_cgu_plan(&cfg, &[entity("E1")]).unwrap_err();
        assert!(format!("{err}").contains("no matching CGU definition"));
    }

    #[test]
    fn negative_goodwill_rejected() {
        let cfg = CguConfig {
            cgus: vec![CguDefinitionEntry {
                cgu_id: "CGU_X".to_string(),
                name: "x".to_string(),
                member_entity_codes: vec!["E1".to_string()],
                segment_code: None,
            }],
            goodwill_allocations: vec![CguGoodwillAllocationEntry {
                cgu_id: "CGU_X".to_string(),
                business_combination_id: "BC_X".to_string(),
                goodwill_amount: dec!(-1),
                allocation_date: date(),
            }],
        };
        let err = build_cgu_plan(&cfg, &[entity("E1")]).unwrap_err();
        assert!(format!("{err}").contains("negative goodwill_amount"));
    }

    #[test]
    fn allocations_sort_deterministically_by_cgu_then_bc() {
        let cfg = CguConfig {
            cgus: vec![CguDefinitionEntry {
                cgu_id: "CGU_X".to_string(),
                name: "x".to_string(),
                member_entity_codes: vec!["E1".to_string()],
                segment_code: None,
            }],
            goodwill_allocations: vec![
                CguGoodwillAllocationEntry {
                    cgu_id: "CGU_X".to_string(),
                    business_combination_id: "BC_002".to_string(),
                    goodwill_amount: dec!(200),
                    allocation_date: date(),
                },
                CguGoodwillAllocationEntry {
                    cgu_id: "CGU_X".to_string(),
                    business_combination_id: "BC_001".to_string(),
                    goodwill_amount: dec!(100),
                    allocation_date: date(),
                },
            ],
        };
        let plan = build_cgu_plan(&cfg, &[entity("E1")]).unwrap();
        assert_eq!(
            plan.goodwill_allocations[0].business_combination_id,
            "BC_001"
        );
        assert_eq!(
            plan.goodwill_allocations[1].business_combination_id,
            "BC_002"
        );
    }
}

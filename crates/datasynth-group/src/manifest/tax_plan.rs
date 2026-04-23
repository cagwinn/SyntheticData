//! Tax group plan resolution — spec §4.1, Task 2.7.
//!
//! Produces a [`TaxGroupPlan`] from [`TaxGroupConfig`] and the expanded
//! entity list.  This is a **plan only** — the Pillar 2 calculator is v5.2.

use std::collections::BTreeSet;

use crate::config::TaxGroupConfig;
use crate::errors::{GroupError, GroupResult};
use crate::manifest::expansion::ExpandedEntity;

// ── Public types ──────────────────────────────────────────────────────────────

/// Pillar Two coverage plan (OECD GloBE rules).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PillarTwoPlan {
    /// Jurisdictions declared in-scope via config (validated against entities).
    pub in_scope_jurisdictions: Vec<String>,
    /// Entity codes whose country is in `in_scope_jurisdictions`, sorted.
    pub entities_in_scope: Vec<String>,
}

/// Country-by-country report plan (OECD BEPS Action 13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CbcReportPlan {
    /// Jurisdiction where the CBC report is filed.
    pub reporting_jurisdiction: String,
    /// Sorted set of all distinct entity countries.
    pub jurisdictions_to_report: Vec<String>,
}

/// Transfer pricing documentation plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferPricingPlan {
    /// Whether a group master file is required.
    pub master_file: bool,
    /// Jurisdictions requiring local-file documentation (validated).
    pub local_file_jurisdictions: Vec<String>,
}

/// Combined tax group plan.  Each sub-plan is `None` when its config block is
/// absent or disabled.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaxGroupPlan {
    pub pillar_two: Option<PillarTwoPlan>,
    pub cbc_report: Option<CbcReportPlan>,
    pub transfer_pricing: Option<TransferPricingPlan>,
}

// ── Public builder ────────────────────────────────────────────────────────────

/// Build the [`TaxGroupPlan`] from config + expanded entities.
///
/// # Errors
/// - Pillar 2: unknown jurisdiction (not present in any entity's country).
/// - CbC: `reporting_jurisdiction` is `None` when `enabled = true`.
/// - TP: `local_files_for` contains a jurisdiction not in any entity's country.
pub fn build_tax_group_plan(
    cfg: &TaxGroupConfig,
    entities: &[ExpandedEntity],
) -> GroupResult<TaxGroupPlan> {
    // Collect the set of all entity countries for validation.
    let entity_countries: BTreeSet<String> = entities.iter().map(|e| e.country.clone()).collect();

    // ── Pillar Two ────────────────────────────────────────────────────────────
    let pillar_two = match &cfg.pillar_two {
        Some(p2) if p2.enabled => {
            // Validate every configured jurisdiction exists in the entity list.
            for jur in &p2.jurisdictions {
                if !entity_countries.contains(jur.as_str()) {
                    return Err(GroupError::Config(format!(
                        "pillar_two jurisdiction '{jur}' is not present in any entity's country"
                    )));
                }
            }
            let in_scope_jurisdictions: BTreeSet<String> =
                p2.jurisdictions.iter().cloned().collect();
            let mut entities_in_scope: Vec<String> = entities
                .iter()
                .filter(|e| in_scope_jurisdictions.contains(&e.country))
                .map(|e| e.code.clone())
                .collect();
            entities_in_scope.sort();
            Some(PillarTwoPlan {
                in_scope_jurisdictions: {
                    let mut v: Vec<String> = in_scope_jurisdictions.into_iter().collect();
                    v.sort();
                    v
                },
                entities_in_scope,
            })
        }
        _ => None,
    };

    // ── CbC Report ────────────────────────────────────────────────────────────
    let cbc_report = match &cfg.cbc_report {
        Some(cbc) if cbc.enabled => {
            let reporting_jurisdiction = cbc.reporting_jurisdiction.clone().ok_or_else(|| {
                GroupError::Config(
                    "cbc_report.reporting_jurisdiction is required when enabled = true".to_string(),
                )
            })?;
            let jurisdictions_to_report: Vec<String> = entity_countries.iter().cloned().collect();
            Some(CbcReportPlan {
                reporting_jurisdiction,
                jurisdictions_to_report,
            })
        }
        _ => None,
    };

    // ── Transfer Pricing ──────────────────────────────────────────────────────
    let transfer_pricing = match &cfg.transfer_pricing {
        Some(tp) => {
            // Validate each local-file jurisdiction against entity countries.
            for jur in &tp.local_files_for {
                if !entity_countries.contains(jur.as_str()) {
                    return Err(GroupError::Config(format!(
                        "transfer_pricing.local_files_for jurisdiction '{jur}' is not present in any entity's country"
                    )));
                }
            }
            let mut local_file_jurisdictions = tp.local_files_for.clone();
            local_file_jurisdictions.sort();
            local_file_jurisdictions.dedup();
            Some(TransferPricingPlan {
                master_file: tp.master_file,
                local_file_jurisdictions,
            })
        }
        None => None,
    };

    Ok(TaxGroupPlan {
        pillar_two,
        cbc_report,
        transfer_pricing,
    })
}

//! Audit engagement plan resolution — spec §4.1, Task 2.6.
//!
//! Produces an [`AuditEngagementPlan`] from [`AuditEngagementConfig`] and the
//! expanded entity list.  This is a **plan only** — the ISA 600 component
//! auditor generator is v5.1.

use std::collections::BTreeMap;

use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;

use crate::config::{AuditEngagementConfig, MaterialityBasis};
use crate::errors::{GroupError, GroupResult};
use crate::manifest::expansion::ExpandedEntity;
use serde::{Deserialize, Serialize};

// ── Row proxy constants ───────────────────────────────────────────────────────

/// Fallback row count when `ExpandedEntity` has no explicit `rows` field.
/// 1 M rows is a conservative placeholder; entity-level revenue is a v5.1
/// concern.
const FALLBACK_ENTITY_ROWS: u64 = 1_000_000;

/// Rough revenue proxy per row ($1 000).  Used to approximate the materiality
/// basis value when actual financial data is not yet available in the manifest
/// phase.  Documented here so reviewers know this is intentionally a stub.
const REVENUE_PER_ROW_USD: u64 = 1_000;

// ── Default thresholds ────────────────────────────────────────────────────────

const DEFAULT_FULL_SCOPE: f64 = 0.15;
const DEFAULT_SPECIFIC_SCOPE: f64 = 0.05;

// ── Public types ──────────────────────────────────────────────────────────────

/// ISA 600-level scope assigned to a single component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentScope {
    /// Full-scope audit work performed for the component.
    Full,
    /// Specified procedures performed for the component.
    Specific,
    /// Analytical procedures only.
    Analytical,
}

/// Per-component materiality allocation and scope decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentMaterialityAllocation {
    pub entity_code: String,
    /// Component materiality (≤ group materiality).
    pub materiality: Decimal,
    /// Scope decision based on revenue-share estimate.
    pub scope: ComponentScope,
    /// Revenue-share estimate used for scope / materiality decisions.
    /// = entity_rows / sum_entity_rows (proxy; v5.1 replaces with actuals).
    pub revenue_share_estimate: Decimal,
}

/// Stub component auditor grouping (v5.1 fills in the real generator).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentAuditor {
    /// `"CA_<FIRM>_<JURISDICTION>"`.
    pub id: String,
    pub firm: String,
    pub jurisdiction: String,
    /// Entity codes covered by this component auditor.
    pub entities: Vec<String>,
}

/// Full audit engagement plan produced during the manifest phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEngagementPlan {
    pub engagement_id: String,
    pub lead_auditor: String,
    /// Audit framework string: `"isa"` | `"pcaob"` | `"dual"`.
    pub framework: String,
    /// FSM blueprint reference, e.g. `"builtin:group_fsa"`.
    pub fsm_blueprint: String,
    /// Group-level materiality (basis_value × percent).
    pub group_materiality: Decimal,
    /// Performance materiality = group_materiality × 0.75.
    pub performance_materiality: Decimal,
    /// Clearly trivial threshold = group_materiality × 0.05.
    pub clearly_trivial: Decimal,
    pub component_materiality_allocations: Vec<ComponentMaterialityAllocation>,
    /// One entry per distinct country in the entity list.
    pub component_auditors: Vec<ComponentAuditor>,
}

// ── Public builder ────────────────────────────────────────────────────────────

/// Build the [`AuditEngagementPlan`] from config + expanded entities.
///
/// # Errors
/// - Returns [`GroupError::Config`] if `cfg.group_materiality` is `None`
///   (required for v5.0) or if materiality arithmetic produces an invalid
///   value.
pub fn build_audit_engagement_plan(
    cfg: &AuditEngagementConfig,
    entities: &[ExpandedEntity],
    group_id: &str,
    _aggregate_seed: &[u8; 32],
) -> GroupResult<AuditEngagementPlan> {
    // ── 1. Simple string fields with defaults ────────────────────────────────
    let engagement_id = cfg
        .engagement_id
        .clone()
        .unwrap_or_else(|| format!("{group_id}_ENGAGEMENT"));
    let lead_auditor = cfg
        .lead_auditor
        .clone()
        .unwrap_or_else(|| "UNDESIGNATED".to_string());
    let framework = cfg.framework.clone().unwrap_or_else(|| "isa".to_string());
    let fsm_blueprint = cfg
        .fsm_blueprint
        .clone()
        .unwrap_or_else(|| "builtin:group_fsa".to_string());

    // ── 2. Group materiality ─────────────────────────────────────────────────
    let mat_cfg = cfg.group_materiality.as_ref().ok_or_else(|| {
        GroupError::Config(
            "audit.group_materiality is required for v5.0 but is not set".to_string(),
        )
    })?;

    // Row-based revenue proxy per entity.
    let entity_rows: Vec<u64> = entities
        .iter()
        .map(|e| e.rows.unwrap_or(FALLBACK_ENTITY_ROWS))
        .collect();

    let total_rows: u64 = entity_rows.iter().sum();
    // Guard against empty entity list (avoids divide-by-zero below).
    let total_rows_nonzero = total_rows.max(1);

    // Approximate revenue proxy: sum_of_rows × $1 000/row.
    // For non-revenue bases we apply a multiplier per spec:
    //   Assets      → same as revenue proxy (stub v5.0)
    //   PretaxIncome → 10 % of revenue proxy
    //   Equity      → 30 % of revenue proxy
    let revenue_proxy_usd = Decimal::from(total_rows_nonzero) * Decimal::from(REVENUE_PER_ROW_USD);
    let basis_value = match mat_cfg.basis {
        MaterialityBasis::Revenue => revenue_proxy_usd,
        // Stub: assets approximated as revenue proxy (v5.1 will use real balance-sheet totals)
        MaterialityBasis::Assets => revenue_proxy_usd,
        // Pre-tax income ≈ 10 % of revenue
        MaterialityBasis::PretaxIncome => revenue_proxy_usd / Decimal::from(10u32),
        // Equity ≈ 30 % of revenue
        MaterialityBasis::Equity => {
            revenue_proxy_usd
                * Decimal::from_f64(0.30)
                    .ok_or_else(|| GroupError::Config("decimal conversion error".to_string()))?
        }
    };

    let group_materiality = basis_value * mat_cfg.percent;
    let performance_materiality = group_materiality
        * Decimal::from_f64(0.75)
            .ok_or_else(|| GroupError::Config("decimal conversion error".to_string()))?;
    let clearly_trivial = group_materiality
        * Decimal::from_f64(0.05)
            .ok_or_else(|| GroupError::Config("decimal conversion error".to_string()))?;

    // ── 3. Component scope thresholds ────────────────────────────────────────
    let full_scope_threshold = cfg
        .component_scope_thresholds
        .as_ref()
        .map(|t| t.full_scope)
        .unwrap_or_else(|| Decimal::from_f64(DEFAULT_FULL_SCOPE).unwrap());
    let specific_scope_threshold = cfg
        .component_scope_thresholds
        .as_ref()
        .map(|t| t.specific_scope)
        .unwrap_or_else(|| Decimal::from_f64(DEFAULT_SPECIFIC_SCOPE).unwrap());

    // ── 4. Component materiality allocations ─────────────────────────────────
    // Formula (ISA 600-inspired conservative allocation):
    //   component_materiality = (1 − sqrt(1 − revenue_share)) × group_materiality
    let mut component_materiality_allocations: Vec<ComponentMaterialityAllocation> =
        Vec::with_capacity(entities.len());

    for (entity, &rows) in entities.iter().zip(entity_rows.iter()) {
        let revenue_share_f64 = rows as f64 / total_rows_nonzero as f64;
        // Clamp to [0, 1] before sqrt to guard against floating-point drift.
        let clamped = revenue_share_f64.clamp(0.0, 1.0);
        let allocation_factor = 1.0 - (1.0 - clamped).sqrt();

        let revenue_share_estimate = Decimal::from_f64(revenue_share_f64)
            .ok_or_else(|| GroupError::Config("revenue_share conversion error".to_string()))?;
        let allocation_decimal = Decimal::from_f64(allocation_factor)
            .ok_or_else(|| GroupError::Config("allocation_factor conversion error".to_string()))?;
        let materiality = group_materiality * allocation_decimal;

        let scope = if revenue_share_estimate >= full_scope_threshold {
            ComponentScope::Full
        } else if revenue_share_estimate >= specific_scope_threshold {
            ComponentScope::Specific
        } else {
            ComponentScope::Analytical
        };

        component_materiality_allocations.push(ComponentMaterialityAllocation {
            entity_code: entity.code.clone(),
            materiality,
            scope,
            revenue_share_estimate,
        });
    }

    // ── 5. Component auditors grouped by country ──────────────────────────────
    // One `ComponentAuditor` per distinct country; the lead auditor's country
    // gets "UNDESIGNATED-LEAD" as firm name, all others get "UNDESIGNATED".
    //
    // Lead country = country of first entity in the entity list (reasonable
    // proxy; v5.1 may override from config).
    let lead_country: Option<String> = entities.first().map(|e| e.country.clone());

    // BTreeMap for deterministic ordering.
    let mut country_entities: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entity in entities {
        country_entities
            .entry(entity.country.clone())
            .or_default()
            .push(entity.code.clone());
    }

    let mut component_auditors: Vec<ComponentAuditor> = Vec::new();
    for (country, mut codes) in country_entities {
        codes.sort(); // stable alphabetical order within jurisdiction
        let is_lead = lead_country.as_deref() == Some(country.as_str());
        let firm = if is_lead {
            "UNDESIGNATED-LEAD".to_string()
        } else {
            "UNDESIGNATED".to_string()
        };
        let id = format!("CA_{firm}_{country}");
        component_auditors.push(ComponentAuditor {
            id,
            firm,
            jurisdiction: country,
            entities: codes,
        });
    }

    Ok(AuditEngagementPlan {
        engagement_id,
        lead_auditor,
        framework,
        fsm_blueprint,
        group_materiality,
        performance_materiality,
        clearly_trivial,
        component_materiality_allocations,
        component_auditors,
    })
}

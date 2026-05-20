//! Profit centre hierarchy model for management accounting / segment reporting.
//!
//! Profit centres represent business units, product lines, or geographic
//! regions whose contribution to the consolidated P&L is reported
//! independently — they map to SAP CEPC (`PRCTR`) and to the IFRS 8 /
//! ASC 280 operating-segment dimension.  Unlike cost centres (which are
//! always cost-only), profit centres carry both revenue and cost
//! attribution and are typically organised by business segment, region,
//! or product line.

use serde::{Deserialize, Serialize};

/// Categorisation of a profit centre — drives default account mapping
/// and segment-reporting roll-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProfitCenterCategory {
    /// A product-line profit centre — owns revenue + cost for one offering.
    #[default]
    ProductLine,
    /// A regional / geographic profit centre — Americas, EMEA, APAC, etc.
    Region,
    /// A business-segment profit centre — IFRS 8 reportable operating segment.
    Segment,
    /// A service / shared-service profit centre that allocates internally.
    Service,
    /// A corporate / holding profit centre (often used for consolidation
    /// adjustments and unallocated items in segment reconciliations).
    Corporate,
}

impl std::fmt::Display for ProfitCenterCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProductLine => write!(f, "Product Line"),
            Self::Region => write!(f, "Region"),
            Self::Segment => write!(f, "Segment"),
            Self::Service => write!(f, "Service"),
            Self::Corporate => write!(f, "Corporate"),
        }
    }
}

/// A profit centre node in the organisational profit hierarchy.
///
/// Profit centres are arranged in a two-level tree:
/// - **Level 1** (parent): represents a business segment, geographic region,
///   or major product-line group.  These have `parent_id == None`.
/// - **Level 2** (child): represents a sub-segment, sub-region, or
///   individual product line.  These have `parent_id == Some(...)`.
///
/// Mapping to SAP CEPC: `id` → `PRCTR`, `name` → `KTEXT`,
/// `responsible_person` → `VERAK_USER`, `company_code` →
/// (resolved through controlling area `KOKRS`), `is_active == false`
/// → `LOKKZ` (locked) flag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfitCenter {
    /// Unique profit centre identifier (e.g., "PC-C001-EMEA-DACH")
    pub id: String,

    /// Human-readable name (e.g., "EMEA / DACH region")
    pub name: String,

    /// Parent profit centre ID for level-2 nodes; `None` for level-1
    /// segment / region nodes.
    pub parent_id: Option<String>,

    /// Company code this profit centre belongs to.
    pub company_code: String,

    /// Employee ID of the manager responsible for this profit centre.
    pub responsible_person: Option<String>,

    /// Functional category of this profit centre.
    pub category: ProfitCenterCategory,

    /// Reporting segment code — used by IFRS 8 / ASC 280 segment
    /// reconciliation.  Multiple level-2 profit centres can roll up to
    /// the same segment.  `None` means the profit centre is not
    /// individually reported (e.g., it sits within a larger segment).
    pub segment_code: Option<String>,

    /// Hierarchy level (1 = top-level segment/region, 2 = sub-unit).
    pub level: u8,

    /// Whether this profit centre is currently active.
    pub is_active: bool,
}

impl ProfitCenter {
    /// Create a new level-1 (top-level segment / region / product group)
    /// profit centre.
    pub fn top_level(
        id: impl Into<String>,
        name: impl Into<String>,
        company_code: impl Into<String>,
        category: ProfitCenterCategory,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            parent_id: None,
            company_code: company_code.into(),
            responsible_person: None,
            category,
            segment_code: None,
            level: 1,
            is_active: true,
        }
    }

    /// Create a new level-2 (sub-unit) profit centre.
    pub fn sub_unit(
        id: impl Into<String>,
        name: impl Into<String>,
        parent_id: impl Into<String>,
        company_code: impl Into<String>,
        category: ProfitCenterCategory,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            parent_id: Some(parent_id.into()),
            company_code: company_code.into(),
            responsible_person: None,
            category,
            segment_code: None,
            level: 2,
            is_active: true,
        }
    }

    /// Attach a segment code (IFRS 8 / ASC 280 reportable segment).
    pub fn with_segment(mut self, segment_code: impl Into<String>) -> Self {
        self.segment_code = Some(segment_code.into());
        self
    }

    /// Attach a responsible person.
    pub fn with_responsible_person(mut self, person_id: impl Into<String>) -> Self {
        self.responsible_person = Some(person_id.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_top_level_constructor() {
        let pc = ProfitCenter::top_level("PC-EMEA", "EMEA", "C001", ProfitCenterCategory::Region);
        assert!(pc.parent_id.is_none());
        assert_eq!(pc.level, 1);
        assert!(pc.is_active);
        assert_eq!(pc.category, ProfitCenterCategory::Region);
    }

    #[test]
    fn test_sub_unit_constructor() {
        let pc = ProfitCenter::sub_unit(
            "PC-EMEA-DACH",
            "DACH",
            "PC-EMEA",
            "C001",
            ProfitCenterCategory::Region,
        );
        assert_eq!(pc.parent_id.as_deref(), Some("PC-EMEA"));
        assert_eq!(pc.level, 2);
    }

    #[test]
    fn test_builder_chain() {
        let pc = ProfitCenter::top_level(
            "PC-CONSUMER",
            "Consumer",
            "C001",
            ProfitCenterCategory::Segment,
        )
        .with_segment("SEG-CONSUMER")
        .with_responsible_person("EMP-001");
        assert_eq!(pc.segment_code.as_deref(), Some("SEG-CONSUMER"));
        assert_eq!(pc.responsible_person.as_deref(), Some("EMP-001"));
    }
}

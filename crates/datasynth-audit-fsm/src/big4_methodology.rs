//! Big 4 ISA-derived common spine + firm overlays + equivalence map —
//! AuditMethodology v0.14.
//!
//! Sourced from `docs/blueprints/big4/`.  Provides:
//!
//! - **`Big4Spine`** — jurisdiction-neutral, tool-neutral procedure tree
//!   derived from the ISAs (4 phases, 17 procedures).
//! - **`FirmOverlay`** — firm-specific extensions (firm name, firm tools,
//!   signoff chain) keyed by spine procedure id.  4 overlays for EY GAM,
//!   PwC Aura, KPMG Clara, Deloitte Omnia.
//! - **`Big4EquivalenceMap`** — flat cross-firm naming map for harmonisation
//!   reports (one row per spine procedure × 4 firms).
//!
//! All YAMLs are embedded via `include_str!`, so the loaders are zero-I/O
//! at runtime.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::AuditFsmError;

// ── Spine types ───────────────────────────────────────────────────────────────

/// Big 4 ISA-derived common spine — the harmonisation root that all 4
/// firm overlays inherit from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Big4Spine {
    pub schema_version: String,
    pub blueprint_id: String,
    pub name: String,
    pub description: String,
    pub phases: Vec<Big4SpinePhase>,
}

/// Phase in the Big 4 spine (Pre-engagement / Planning / Execution / Closure).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Big4SpinePhase {
    pub id: String,
    pub name: String,
    pub procedures: Vec<Big4SpineProcedure>,
}

/// Spine procedure — only the ISA-anchored skeleton.  Firm overlays
/// add tool / naming / signoff metadata via `ProcedureExtension`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Big4SpineProcedure {
    pub id: String,
    pub name: String,
    /// ISA citation refs (e.g. `["ISA 220 R12-R14", "IESBA Code R210"]`).
    /// Empty for procedures whose anchors are split across phases.
    #[serde(default)]
    pub isa_refs: Vec<String>,
}

// ── Firm overlay types ────────────────────────────────────────────────────────

/// One of the Big 4 firms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Firm {
    /// Ernst & Young.
    #[serde(rename = "EY")]
    EY,
    /// PricewaterhouseCoopers.
    #[serde(rename = "PwC")]
    PwC,
    /// KPMG.
    #[serde(rename = "KPMG")]
    KPMG,
    /// Deloitte.
    #[serde(rename = "Deloitte")]
    Deloitte,
}

impl Firm {
    /// Lower-case slug used as the key in the equivalence map
    /// (`ey` / `pwc` / `kpmg` / `deloitte`).
    pub fn equivalence_key(self) -> &'static str {
        match self {
            Firm::EY => "ey",
            Firm::PwC => "pwc",
            Firm::KPMG => "kpmg",
            Firm::Deloitte => "deloitte",
        }
    }
}

/// Firm-specific overlay over the Big 4 spine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmOverlay {
    pub schema_version: String,
    pub blueprint_id: String,
    pub name: String,
    /// The spine blueprint id this overlay extends (always
    /// `"big4_spine"` for the built-in overlays).
    pub inherits_from: String,
    pub firm: Firm,
    pub procedure_extensions: Vec<ProcedureExtension>,
}

/// Per-procedure firm extension — overrides the spine procedure's name
/// with a firm-specific name and adds the firm's tooling and signoff chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcedureExtension {
    /// Spine procedure id this extension applies to.
    pub id: String,
    /// Firm-specific procedure name (e.g. "Risk Assessment & Response (RAR)").
    pub firm_name: String,
    /// Firm tools / platforms used for this procedure.
    #[serde(default)]
    pub firm_tools: Vec<String>,
    /// Sign-off chain in role order (e.g. `["staff", "senior", "manager"]`).
    #[serde(default)]
    pub signoff_chain: Vec<String>,
}

/// A spine procedure resolved against a firm overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFirmProcedure {
    /// Spine procedure id.
    pub id: String,
    /// Spine (ISA-anchored) procedure name.
    pub spine_name: String,
    /// ISA citation refs from the spine.
    pub isa_refs: Vec<String>,
    /// Firm-specific procedure name (or `None` if the firm doesn't
    /// extend this procedure).
    pub firm_name: Option<String>,
    /// Firm tools / platforms.
    pub firm_tools: Vec<String>,
    /// Sign-off chain.
    pub signoff_chain: Vec<String>,
}

// ── Equivalence map ───────────────────────────────────────────────────────────

/// Cross-firm equivalence map — flat dictionary
/// `procedure_id → { firm_slug → firm_specific_name }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Big4EquivalenceMap {
    pub schema_version: String,
    pub description: String,
    /// Outer key = spine procedure id; inner key = lower-case firm slug
    /// (`ey` / `pwc` / `kpmg` / `deloitte`).
    pub procedure_equivalence: BTreeMap<String, BTreeMap<String, String>>,
}

// ── Builtin loaders ───────────────────────────────────────────────────────────

const SPINE_YAML: &str = include_str!("../blueprints/big4/big4_spine.yaml");
const EY_GAM_YAML: &str = include_str!("../blueprints/big4/ey_gam_overlay.yaml");
const PWC_AURA_YAML: &str = include_str!("../blueprints/big4/pwc_aura_overlay.yaml");
const KPMG_CLARA_YAML: &str = include_str!("../blueprints/big4/kpmg_clara_overlay.yaml");
const DELOITTE_OMNIA_YAML: &str = include_str!("../blueprints/big4/deloitte_omnia_overlay.yaml");
const EQUIVALENCE_MAP_YAML: &str = include_str!("../blueprints/big4/equivalence_map.yaml");

/// Load the built-in Big 4 ISA-derived common spine.
pub fn builtin_big4_spine() -> Result<Big4Spine, AuditFsmError> {
    serde_yaml::from_str(SPINE_YAML).map_err(|source| AuditFsmError::BlueprintParse {
        path: "blueprints/big4/big4_spine.yaml".to_string(),
        source,
    })
}

/// Load all 4 built-in firm overlays in declaration order
/// (EY GAM, PwC Aura, KPMG Clara, Deloitte Omnia).
pub fn builtin_firm_overlays() -> Result<Vec<FirmOverlay>, AuditFsmError> {
    let pairs: [(&str, &str); 4] = [
        (EY_GAM_YAML, "blueprints/big4/ey_gam_overlay.yaml"),
        (PWC_AURA_YAML, "blueprints/big4/pwc_aura_overlay.yaml"),
        (KPMG_CLARA_YAML, "blueprints/big4/kpmg_clara_overlay.yaml"),
        (
            DELOITTE_OMNIA_YAML,
            "blueprints/big4/deloitte_omnia_overlay.yaml",
        ),
    ];
    pairs
        .into_iter()
        .map(|(yaml, path)| {
            serde_yaml::from_str(yaml).map_err(|source| AuditFsmError::BlueprintParse {
                path: path.to_string(),
                source,
            })
        })
        .collect()
}

/// Load the built-in cross-firm equivalence map.
pub fn builtin_equivalence_map() -> Result<Big4EquivalenceMap, AuditFsmError> {
    serde_yaml::from_str(EQUIVALENCE_MAP_YAML).map_err(|source| AuditFsmError::OverlayParse {
        path: "blueprints/big4/equivalence_map.yaml".to_string(),
        source,
    })
}

// ── Resolution helpers ────────────────────────────────────────────────────────

/// Walk the spine and produce a fully-resolved procedure list joined
/// against a firm overlay.  Procedures the overlay doesn't extend are
/// returned with `firm_name = None` (and empty tools / signoff).
pub fn resolve_spine_with_overlay(
    spine: &Big4Spine,
    overlay: &FirmOverlay,
) -> Vec<ResolvedFirmProcedure> {
    let mut by_id: BTreeMap<&str, &ProcedureExtension> = BTreeMap::new();
    for ext in &overlay.procedure_extensions {
        by_id.insert(ext.id.as_str(), ext);
    }

    let mut out = Vec::new();
    for phase in &spine.phases {
        for proc in &phase.procedures {
            let ext = by_id.get(proc.id.as_str()).copied();
            out.push(ResolvedFirmProcedure {
                id: proc.id.clone(),
                spine_name: proc.name.clone(),
                isa_refs: proc.isa_refs.clone(),
                firm_name: ext.map(|e| e.firm_name.clone()),
                firm_tools: ext.map(|e| e.firm_tools.clone()).unwrap_or_default(),
                signoff_chain: ext.map(|e| e.signoff_chain.clone()).unwrap_or_default(),
            });
        }
    }
    out
}

/// Total procedure count across all spine phases (excludes phase headers
/// — a header without procedures is not counted).
pub fn spine_procedure_count(spine: &Big4Spine) -> usize {
    spine.phases.iter().map(|p| p.procedures.len()).sum()
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spine_loads_and_has_four_phases() {
        let spine = builtin_big4_spine().unwrap();
        assert_eq!(spine.blueprint_id, "big4_spine");
        assert_eq!(spine.phases.len(), 4);
        let phase_ids: Vec<&str> = spine.phases.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            phase_ids,
            [
                "phase_pre_engagement",
                "phase_planning",
                "phase_execution",
                "phase_closure"
            ]
        );
    }

    #[test]
    fn spine_has_seventeen_procedures() {
        let spine = builtin_big4_spine().unwrap();
        assert_eq!(spine_procedure_count(&spine), 17);
    }

    #[test]
    fn spine_procedure_ids_are_unique() {
        let spine = builtin_big4_spine().unwrap();
        let mut ids: Vec<&str> = spine
            .phases
            .iter()
            .flat_map(|p| p.procedures.iter().map(|q| q.id.as_str()))
            .collect();
        let original_len = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), original_len, "duplicate procedure ids");
    }

    #[test]
    fn spine_procedures_carry_isa_refs() {
        let spine = builtin_big4_spine().unwrap();
        let acceptance = spine
            .phases
            .iter()
            .find(|p| p.id == "phase_pre_engagement")
            .unwrap()
            .procedures
            .iter()
            .find(|q| q.id == "proc_acceptance")
            .unwrap();
        assert!(!acceptance.isa_refs.is_empty());
        assert!(acceptance.isa_refs.iter().any(|r| r.contains("ISA 220")));
    }

    #[test]
    fn four_firm_overlays_load_in_declaration_order() {
        let overlays = builtin_firm_overlays().unwrap();
        assert_eq!(overlays.len(), 4);
        let firms: Vec<Firm> = overlays.iter().map(|o| o.firm).collect();
        assert_eq!(firms, [Firm::EY, Firm::PwC, Firm::KPMG, Firm::Deloitte]);
    }

    #[test]
    fn each_firm_overlay_inherits_from_spine() {
        let overlays = builtin_firm_overlays().unwrap();
        for overlay in &overlays {
            assert_eq!(overlay.inherits_from, "big4_spine");
        }
    }

    #[test]
    fn firm_overlays_only_extend_existing_spine_procedures() {
        let spine = builtin_big4_spine().unwrap();
        let overlays = builtin_firm_overlays().unwrap();
        let spine_ids: std::collections::HashSet<&str> = spine
            .phases
            .iter()
            .flat_map(|p| p.procedures.iter().map(|q| q.id.as_str()))
            .collect();
        for overlay in &overlays {
            for ext in &overlay.procedure_extensions {
                assert!(
                    spine_ids.contains(ext.id.as_str()),
                    "overlay {} references unknown procedure {}",
                    overlay.blueprint_id,
                    ext.id
                );
            }
        }
    }

    #[test]
    fn ey_overlay_extends_understanding_with_helix_tools() {
        let overlays = builtin_firm_overlays().unwrap();
        let ey = overlays.iter().find(|o| o.firm == Firm::EY).unwrap();
        let ext = ey
            .procedure_extensions
            .iter()
            .find(|e| e.id == "proc_understanding_entity")
            .unwrap();
        assert_eq!(ext.firm_name, "Understand the Entity (EY)");
        assert!(ext.firm_tools.iter().any(|t| t.contains("Helix")));
    }

    #[test]
    fn deloitte_overlay_extends_understanding_with_cortex() {
        let overlays = builtin_firm_overlays().unwrap();
        let dttl = overlays.iter().find(|o| o.firm == Firm::Deloitte).unwrap();
        let ext = dttl
            .procedure_extensions
            .iter()
            .find(|e| e.id == "proc_understanding_entity")
            .unwrap();
        assert!(ext.firm_tools.iter().any(|t| t.contains("Cortex")));
        assert!(ext.firm_tools.iter().any(|t| t.contains("Argus")));
    }

    #[test]
    fn equivalence_map_loads() {
        let map = builtin_equivalence_map().unwrap();
        assert!(!map.procedure_equivalence.is_empty());
    }

    #[test]
    fn equivalence_map_has_all_four_firms_per_entry() {
        let map = builtin_equivalence_map().unwrap();
        for (proc_id, firms) in &map.procedure_equivalence {
            for slug in ["ey", "pwc", "kpmg", "deloitte"] {
                assert!(
                    firms.contains_key(slug),
                    "procedure {proc_id} missing firm slug {slug}"
                );
            }
        }
    }

    #[test]
    fn equivalence_map_keys_match_spine_procedure_ids() {
        let spine = builtin_big4_spine().unwrap();
        let map = builtin_equivalence_map().unwrap();
        let spine_ids: std::collections::HashSet<&str> = spine
            .phases
            .iter()
            .flat_map(|p| p.procedures.iter().map(|q| q.id.as_str()))
            .collect();
        for proc_id in map.procedure_equivalence.keys() {
            assert!(
                spine_ids.contains(proc_id.as_str()),
                "equivalence map key {proc_id} is not a spine procedure"
            );
        }
    }

    #[test]
    fn firm_equivalence_key_matches_yaml_slugs() {
        assert_eq!(Firm::EY.equivalence_key(), "ey");
        assert_eq!(Firm::PwC.equivalence_key(), "pwc");
        assert_eq!(Firm::KPMG.equivalence_key(), "kpmg");
        assert_eq!(Firm::Deloitte.equivalence_key(), "deloitte");
    }

    #[test]
    fn resolve_spine_with_overlay_yields_one_row_per_spine_procedure() {
        let spine = builtin_big4_spine().unwrap();
        let overlays = builtin_firm_overlays().unwrap();
        for overlay in &overlays {
            let resolved = resolve_spine_with_overlay(&spine, overlay);
            assert_eq!(resolved.len(), 17);
        }
    }

    #[test]
    fn resolve_carries_firm_name_only_for_extended_procedures() {
        let spine = builtin_big4_spine().unwrap();
        let overlays = builtin_firm_overlays().unwrap();
        let pwc = overlays.iter().find(|o| o.firm == Firm::PwC).unwrap();
        let resolved = resolve_spine_with_overlay(&spine, pwc);
        let pwc_extended_ids: std::collections::HashSet<&str> = pwc
            .procedure_extensions
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        for r in &resolved {
            if pwc_extended_ids.contains(r.id.as_str()) {
                assert!(r.firm_name.is_some(), "expected firm_name for {}", r.id);
            } else {
                assert!(r.firm_name.is_none(), "unexpected firm_name for {}", r.id);
            }
        }
    }

    #[test]
    fn json_round_trips_spine_and_overlays_and_map() {
        let spine = builtin_big4_spine().unwrap();
        let json = serde_json::to_string(&spine).unwrap();
        let back: Big4Spine = serde_json::from_str(&json).unwrap();
        assert_eq!(spine, back);

        for overlay in builtin_firm_overlays().unwrap() {
            let json = serde_json::to_string(&overlay).unwrap();
            let back: FirmOverlay = serde_json::from_str(&json).unwrap();
            assert_eq!(overlay, back);
        }

        let map = builtin_equivalence_map().unwrap();
        let json = serde_json::to_string(&map).unwrap();
        let back: Big4EquivalenceMap = serde_json::from_str(&json).unwrap();
        assert_eq!(map, back);
    }
}

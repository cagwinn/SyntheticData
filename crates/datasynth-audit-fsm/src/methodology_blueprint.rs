//! AuditMethodology v0.14 blueprint loader.
//!
//! Sourced from `docs/blueprints/{group_audit,csrd,kyc}/*.yaml` in the
//! AuditMethodology repo.  These blueprints have a slightly different
//! schema from the legacy `loader::Blueprint` types and live alongside
//! them so neither side has to migrate to the other's schema.
//!
//! # Schema (matches AuditMethodology v0.14)
//!
//! ```yaml
//! schema_version: "1.0.0"
//! blueprint_id: <unique id>
//! name: <human readable>
//! description: |
//!   <multi-line>
//! phases:
//!   - id: <phase id>
//!     name: <human readable>
//!     description: <multi-line>
//!     procedures:
//!       - id: <procedure id>
//!         name: <human readable>
//!         description: <multi-line>
//!         isa_refs: [...]                # or esrs_refs / iia_refs / etc.
//!         steps:
//!           - id: <step id>
//!             name: <human readable>
//!             description: <multi-line>
//!             isa_refs: [...]
//!             evidence_kind: <inspection|inquiry|observation|recalculation|...>
//! ```
//!
//! # Built-in blueprints
//!
//! Two YAMLs embedded via `include_str!`:
//!
//! - **`iso_600_revised`** — ISA 600 (Revised, Dec 2022) group-audit
//!   blueprint covering all requirement paragraphs.  Six phases:
//!   acceptance + continuance → planning & scoping → component
//!   communication → evidence & misstatements → subsequent events &
//!   review → documentation & communication.
//! - **`csrd_limited_assurance`** — Corporate Sustainability
//!   Reporting Directive (CSRD) limited-assurance engagement on
//!   ESRS-formatted sustainability statements.  100 % ESRS topical
//!   standard coverage (E1–E5 environmental, S1–S4 social, G1
//!   governance) per Directive (EU) 2022/2464 + IAASB ISSA 5000.

use serde::{Deserialize, Serialize};

use crate::error::AuditFsmError;

// ── Public types ──────────────────────────────────────────────────────────────

/// One AuditMethodology blueprint loaded from a YAML file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodologyBlueprint {
    /// Schema version — currently `"1.0.0"`.
    pub schema_version: String,
    /// Unique blueprint identifier (e.g. `"iso_600_revised"`).
    pub blueprint_id: String,
    /// Human-readable name.
    pub name: String,
    /// Multi-line description of what this blueprint covers.
    pub description: String,
    /// Engagement phases, in execution order.
    pub phases: Vec<MethodologyPhase>,
}

impl MethodologyBlueprint {
    /// Total procedure count across all phases.
    pub fn total_procedures(&self) -> usize {
        self.phases.iter().map(|p| p.procedures.len()).sum()
    }

    /// Total step count across all phases and procedures.
    pub fn total_steps(&self) -> usize {
        self.phases
            .iter()
            .flat_map(|p| p.procedures.iter())
            .map(|proc| proc.steps.len())
            .sum()
    }

    /// All citation references (ISA / ESRS / IIA-GIAS / etc.) found in
    /// procedures and steps, deduplicated and sorted.
    pub fn all_citation_refs(&self) -> Vec<String> {
        let mut refs: Vec<String> = self
            .phases
            .iter()
            .flat_map(|p| &p.procedures)
            .flat_map(|proc| {
                let mut citations = Vec::new();
                if let Some(ir) = &proc.isa_refs {
                    citations.extend(ir.iter().cloned());
                }
                if let Some(er) = &proc.esrs_refs {
                    citations.extend(er.iter().cloned());
                }
                for step in &proc.steps {
                    if let Some(ir) = &step.isa_refs {
                        citations.extend(ir.iter().cloned());
                    }
                    if let Some(er) = &step.esrs_refs {
                        citations.extend(er.iter().cloned());
                    }
                }
                citations
            })
            .collect();
        refs.sort();
        refs.dedup();
        refs
    }
}

/// One engagement phase (acceptance / planning / fieldwork / etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodologyPhase {
    /// Unique phase identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Multi-line description.
    #[serde(default)]
    pub description: String,
    /// Procedures executed within this phase.
    #[serde(default)]
    pub procedures: Vec<MethodologyProcedure>,
}

/// One audit procedure (a coherent unit of work — typically several
/// steps that together address a single ISA paragraph or ESRS
/// disclosure requirement).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodologyProcedure {
    /// Unique procedure identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Multi-line description.
    #[serde(default)]
    pub description: String,
    /// ISA paragraph references (e.g. `["ISA 600 R16"]`).  Optional;
    /// not all procedures cite ISA paragraphs (e.g. CSRD blueprints
    /// cite ESRS instead).
    #[serde(default)]
    pub isa_refs: Option<Vec<String>>,
    /// ESRS / sustainability standard references (e.g. `["ESRS E1-7"]`).
    #[serde(default)]
    pub esrs_refs: Option<Vec<String>>,
    /// Steps comprising this procedure.
    #[serde(default)]
    pub steps: Vec<MethodologyStep>,
}

/// One step within a procedure — the smallest unit of audit work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodologyStep {
    /// Unique step identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Multi-line description.
    #[serde(default)]
    pub description: String,
    /// ISA paragraph references for this specific step.
    #[serde(default)]
    pub isa_refs: Option<Vec<String>>,
    /// ESRS references for this specific step.
    #[serde(default)]
    pub esrs_refs: Option<Vec<String>>,
    /// Kind of evidence the step produces (inspection, inquiry,
    /// observation, recalculation, reperformance, confirmation,
    /// analytical_procedures).  Stored as a free-form string to
    /// accommodate future evidence taxonomy extensions.
    #[serde(default)]
    pub evidence_kind: Option<String>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Parse a methodology blueprint from a YAML string.
pub fn load_methodology_blueprint_yaml(yaml: &str) -> Result<MethodologyBlueprint, AuditFsmError> {
    serde_yaml::from_str(yaml).map_err(|source| AuditFsmError::BlueprintParse {
        path: "<methodology>".to_string(),
        source,
    })
}

/// Load the embedded `iso_600_revised` blueprint (ISA 600 Revised
/// group-audit).
pub fn builtin_iso_600_revised() -> Result<MethodologyBlueprint, AuditFsmError> {
    load_methodology_blueprint_yaml(BUILTIN_ISO_600_REVISED)
}

/// Load the embedded `csrd_limited_assurance` blueprint (CSRD ESRS
/// limited assurance).
pub fn builtin_csrd_limited_assurance() -> Result<MethodologyBlueprint, AuditFsmError> {
    load_methodology_blueprint_yaml(BUILTIN_CSRD_LIMITED_ASSURANCE)
}

/// Load all built-in methodology blueprints.
pub fn builtin_methodology_blueprints() -> Result<Vec<MethodologyBlueprint>, AuditFsmError> {
    Ok(vec![
        builtin_iso_600_revised()?,
        builtin_csrd_limited_assurance()?,
    ])
}

// ── Embedded YAML ─────────────────────────────────────────────────────────────

const BUILTIN_ISO_600_REVISED: &str =
    include_str!("../blueprints/methodology/iso_600_revised.yaml");
const BUILTIN_CSRD_LIMITED_ASSURANCE: &str =
    include_str!("../blueprints/methodology/csrd_limited_assurance.yaml");

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_600_revised_loads_with_six_phases() {
        let bp = builtin_iso_600_revised().unwrap();
        assert_eq!(bp.schema_version, "1.0.0");
        assert_eq!(bp.blueprint_id, "iso_600_revised");
        assert_eq!(bp.phases.len(), 6);
        // Phase IDs in execution order.
        let phase_ids: Vec<&str> = bp.phases.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            phase_ids,
            vec![
                "phase_acceptance_continuance",
                "phase_planning_and_scoping",
                "phase_communication_with_components",
                "phase_evidence_and_misstatements",
                "phase_subsequent_events_and_review",
                "phase_documentation_and_communication",
            ]
        );
    }

    #[test]
    fn iso_600_revised_has_procedures_in_every_phase() {
        let bp = builtin_iso_600_revised().unwrap();
        for phase in &bp.phases {
            assert!(
                !phase.procedures.is_empty(),
                "phase {} has no procedures",
                phase.id
            );
        }
    }

    #[test]
    fn iso_600_revised_total_procedure_count_above_minimum() {
        // Sanity floor — real blueprint has well over 10 procedures
        // across 6 phases.  Lock in a minimum so a regression that
        // accidentally truncates the YAML loud-fails.
        let bp = builtin_iso_600_revised().unwrap();
        assert!(
            bp.total_procedures() >= 10,
            "expected at least 10 procedures; got {}",
            bp.total_procedures()
        );
    }

    #[test]
    fn iso_600_revised_total_step_count_above_minimum() {
        let bp = builtin_iso_600_revised().unwrap();
        assert!(
            bp.total_steps() >= 20,
            "expected at least 20 steps; got {}",
            bp.total_steps()
        );
    }

    #[test]
    fn iso_600_revised_cites_isa_600_paragraphs() {
        let bp = builtin_iso_600_revised().unwrap();
        let citations = bp.all_citation_refs();
        // The blueprint's whole purpose is to cover ISA 600 (Revised)
        // requirement paragraphs — should be plenty of "ISA 600 R*"
        // citations.
        let isa_600_count = citations
            .iter()
            .filter(|c| c.starts_with("ISA 600"))
            .count();
        assert!(
            isa_600_count >= 5,
            "expected ≥5 ISA 600 citations; got {isa_600_count} ({citations:?})"
        );
    }

    #[test]
    fn csrd_limited_assurance_loads() {
        let bp = builtin_csrd_limited_assurance().unwrap();
        assert_eq!(bp.schema_version, "1.0.0");
        assert_eq!(bp.blueprint_id, "csrd_limited_assurance");
        assert!(!bp.phases.is_empty());
    }

    #[test]
    fn csrd_limited_assurance_cites_esrs_standards() {
        let bp = builtin_csrd_limited_assurance().unwrap();
        let citations = bp.all_citation_refs();
        // CSRD assurance covers ESRS topical standards (E1–E5, S1–S4,
        // G1) with citation tags like `esrs_1`, `e1_climate`,
        // `e2_pollution`, etc.  Verify at least 5 citations matching
        // either prefix.
        let esrs_count = citations
            .iter()
            .filter(|c| {
                c.starts_with("esrs_")
                    || c.starts_with("e1_")
                    || c.starts_with("e2_")
                    || c.starts_with("e3_")
                    || c.starts_with("e4_")
                    || c.starts_with("e5_")
                    || c.starts_with("s1_")
                    || c.starts_with("s2_")
                    || c.starts_with("s3_")
                    || c.starts_with("s4_")
                    || c.starts_with("g1_")
            })
            .count();
        assert!(
            esrs_count >= 5,
            "expected ≥5 ESRS citations in CSRD blueprint; got {esrs_count} ({citations:?})"
        );
    }

    #[test]
    fn builtin_methodology_blueprints_loads_both() {
        let bps = builtin_methodology_blueprints().unwrap();
        assert_eq!(bps.len(), 2);
        let ids: Vec<&str> = bps.iter().map(|b| b.blueprint_id.as_str()).collect();
        assert!(ids.contains(&"iso_600_revised"));
        assert!(ids.contains(&"csrd_limited_assurance"));
    }

    #[test]
    fn json_round_trips_methodology_blueprint() {
        let bp = builtin_iso_600_revised().unwrap();
        let json = serde_json::to_string(&bp).unwrap();
        let back: MethodologyBlueprint = serde_json::from_str(&json).unwrap();
        assert_eq!(bp, back);
    }

    #[test]
    fn step_evidence_kind_carried_through() {
        let bp = builtin_iso_600_revised().unwrap();
        // At least some steps should carry an evidence_kind hint —
        // the methodology populates this for every step.
        let step_with_evidence_kind = bp
            .phases
            .iter()
            .flat_map(|p| &p.procedures)
            .flat_map(|proc| &proc.steps)
            .find(|s| s.evidence_kind.is_some());
        assert!(
            step_with_evidence_kind.is_some(),
            "expected at least one step to have evidence_kind"
        );
    }
}

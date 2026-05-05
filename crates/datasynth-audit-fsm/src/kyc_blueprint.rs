//! KYC blueprint loader — AuditMethodology v0.14 KYC workflows.
//!
//! Sourced from `docs/blueprints/kyc/*.yaml` in the AuditMethodology
//! repo.  Six built-in workflow blueprints covering EU AMLR / MiCA /
//! TFR / Wolfsberg correspondent banking / FATF Recommendations 10–11
//! patterns:
//!
//! | Blueprint | Coverage |
//! |-----------|----------|
//! | `kyc_casp_onboarding` | Crypto-asset service provider onboarding (MiCA + TFR + AMLR) |
//! | `kyc_correspondent_onboarding` | Correspondent banking onboarding (Wolfsberg CBDDQ) |
//! | `kyc_onboarding_private_banking` | High-net-worth private-bank onboarding |
//! | `kyc_pkyc_review` | Periodic / event-triggered KYC review |
//! | `kyc_sanctions_hit_remediation` | Sanctions-screening hit remediation workflow |
//! | `kyc_sar_escalation` | SAR escalation workflow (FIU filing pipeline) |
//!
//! # Schema (KYC variant of AuditMethodology v0.14)
//!
//! ```yaml
//! id: kyc_casp_onboarding
//! name: <human readable>
//! description: >
//!   <multi-line>
//! primary_anchors: [<obligation source ids>]
//! discriminator_profile:
//!   tiers: [...]              # e.g. [edd, sdd, simplified]
//!   overlays: [...]           # e.g. [casp, correspondent]
//!   jurisdictions: [...]      # ISO-3166 alpha-2 codes
//! procedures:
//!   - id: <procedure id>
//!     name: <human readable>
//!     description: <multi-line>
//!     obligation_refs: [obl:<source>:<paragraph>:<sub>]
//!     steps:
//!       - id: <step id>
//!         blueprint_ref: <blueprint id>
//!         procedure_ref: <parent procedure id>
//!         description: <multi-line>
//!         obligation_refs: [...]
//!         kind: <generic|inquiry|inspection|recalculation|...>
//! liveness_formulas:
//!   - <optional MTL/STL formulas for runtime liveness checks>
//! ```
//!
//! Citation format: `obl:<source>:<paragraph>:<sub>` — e.g.
//! `obl:eu_mica:60:1` references EU MiCA Article 60 paragraph 1.
//! Differs from `isa_refs` / `esrs_refs` used by audit blueprints in
//! `methodology_blueprint`.

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::error::AuditFsmError;

// ── Public types ──────────────────────────────────────────────────────────────

/// One KYC workflow blueprint loaded from a `kyc_*.yaml` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KycBlueprint {
    /// Unique blueprint identifier (e.g. `"kyc_casp_onboarding"`).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Multi-line description of what this workflow covers.
    pub description: String,
    /// Primary regulatory anchors (e.g. `["eu_mica", "eu_tfr",
    /// "eu_amlr_2024_1624"]`).  Top-level "obligation source"
    /// identifiers — every `obligation_ref` in the procedures /
    /// steps maps back to one of these sources.
    #[serde(default)]
    pub primary_anchors: Vec<String>,
    /// Discriminator profile — tiers / overlays / jurisdictions the
    /// workflow applies to.
    #[serde(default)]
    pub discriminator_profile: Option<KycDiscriminatorProfile>,
    /// Procedures, in execution order.
    pub procedures: Vec<KycProcedure>,
    /// Optional liveness formulas (MTL/STL expressions) the runtime
    /// stream monitor evaluates against produced events.  Free-form
    /// strings until a formal grammar is wired up.
    #[serde(default)]
    pub liveness_formulas: Vec<Value>,
}

impl KycBlueprint {
    /// Total procedure count.
    pub fn total_procedures(&self) -> usize {
        self.procedures.len()
    }

    /// Total step count across all procedures.
    pub fn total_steps(&self) -> usize {
        self.procedures.iter().map(|p| p.steps.len()).sum()
    }

    /// All `obligation_refs` found in procedures and steps,
    /// deduplicated and sorted.  Useful for coverage analysis.
    pub fn all_obligation_refs(&self) -> Vec<String> {
        let mut refs: Vec<String> = self
            .procedures
            .iter()
            .flat_map(|p| {
                p.obligation_refs.iter().cloned().chain(
                    p.steps
                        .iter()
                        .flat_map(|s| s.obligation_refs.iter().cloned()),
                )
            })
            .collect();
        refs.sort();
        refs.dedup();
        refs
    }
}

/// Tiers / overlays / jurisdictions filter — the methodology repo's
/// "discriminator profile" shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct KycDiscriminatorProfile {
    /// Tier names (e.g. `edd` enhanced due diligence, `sdd` simplified).
    #[serde(default)]
    pub tiers: Vec<String>,
    /// Overlay names (e.g. `casp`, `correspondent`).
    #[serde(default)]
    pub overlays: Vec<String>,
    /// ISO-3166 alpha-2 jurisdictions (e.g. `["ch", "eu"]`).
    #[serde(default)]
    pub jurisdictions: Vec<String>,
}

/// One procedure in a KYC workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KycProcedure {
    /// Unique procedure identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Multi-line description.
    #[serde(default)]
    pub description: String,
    /// Obligation references this procedure addresses (e.g.
    /// `["obl:eu_mica:59:0", "obl:eu_mica:60:0"]`).
    #[serde(default)]
    pub obligation_refs: Vec<String>,
    /// Steps comprising this procedure.
    #[serde(default)]
    pub steps: Vec<KycStep>,
}

/// One step within a KYC procedure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KycStep {
    /// Unique step identifier.
    pub id: String,
    /// Backref to the parent blueprint id.
    #[serde(default)]
    pub blueprint_ref: Option<String>,
    /// Backref to the parent procedure id.
    #[serde(default)]
    pub procedure_ref: Option<String>,
    /// Human-readable name (sometimes omitted in KYC YAMLs — they
    /// often only carry the description).
    #[serde(default)]
    pub name: Option<String>,
    /// Multi-line description.
    #[serde(default)]
    pub description: String,
    /// Obligation references this step addresses.
    #[serde(default)]
    pub obligation_refs: Vec<String>,
    /// Step kind — typically one of `generic`, `inspection`,
    /// `inquiry`, `analytical_procedures`, etc.
    #[serde(default)]
    pub kind: Option<String>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Parse a KYC blueprint from a YAML string.
pub fn load_kyc_blueprint_yaml(yaml: &str) -> Result<KycBlueprint, AuditFsmError> {
    serde_yaml::from_str(yaml).map_err(|source| AuditFsmError::BlueprintParse {
        path: "<kyc>".to_string(),
        source,
    })
}

/// Load all six built-in KYC blueprints.
pub fn builtin_kyc_blueprints() -> Result<Vec<KycBlueprint>, AuditFsmError> {
    let yamls: &[&str] = &[
        BUILTIN_CASP_ONBOARDING,
        BUILTIN_CORRESPONDENT_ONBOARDING,
        BUILTIN_PRIVATE_BANKING_ONBOARDING,
        BUILTIN_PKYC_REVIEW,
        BUILTIN_SANCTIONS_HIT_REMEDIATION,
        BUILTIN_SAR_ESCALATION,
    ];
    yamls.iter().map(|y| load_kyc_blueprint_yaml(y)).collect()
}

/// Load a specific built-in KYC blueprint by id.
pub fn load_builtin_kyc_blueprint(id: &str) -> Result<KycBlueprint, AuditFsmError> {
    let yaml = match id {
        "kyc_casp_onboarding" => BUILTIN_CASP_ONBOARDING,
        "kyc_correspondent_onboarding" => BUILTIN_CORRESPONDENT_ONBOARDING,
        "kyc_onboarding_private_banking" => BUILTIN_PRIVATE_BANKING_ONBOARDING,
        "kyc_pkyc_review" => BUILTIN_PKYC_REVIEW,
        "kyc_sanctions_hit_remediation" => BUILTIN_SANCTIONS_HIT_REMEDIATION,
        "kyc_sar_escalation" => BUILTIN_SAR_ESCALATION,
        _ => {
            return Err(AuditFsmError::SourceNotFound {
                source_id: id.to_string(),
            });
        }
    };
    load_kyc_blueprint_yaml(yaml)
}

// ── Embedded YAML ─────────────────────────────────────────────────────────────

const BUILTIN_CASP_ONBOARDING: &str = include_str!("../blueprints/kyc/kyc_casp_onboarding.yaml");
const BUILTIN_CORRESPONDENT_ONBOARDING: &str =
    include_str!("../blueprints/kyc/kyc_correspondent_onboarding.yaml");
const BUILTIN_PRIVATE_BANKING_ONBOARDING: &str =
    include_str!("../blueprints/kyc/kyc_onboarding_private_banking.yaml");
const BUILTIN_PKYC_REVIEW: &str = include_str!("../blueprints/kyc/kyc_pkyc_review.yaml");
const BUILTIN_SANCTIONS_HIT_REMEDIATION: &str =
    include_str!("../blueprints/kyc/kyc_sanctions_hit_remediation.yaml");
const BUILTIN_SAR_ESCALATION: &str = include_str!("../blueprints/kyc/kyc_sar_escalation.yaml");

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn casp_onboarding_loads_with_expected_anchors() {
        let bp = load_kyc_blueprint_yaml(BUILTIN_CASP_ONBOARDING).unwrap();
        assert_eq!(bp.id, "kyc_casp_onboarding");
        assert!(bp.primary_anchors.contains(&"eu_mica".to_string()));
        assert!(bp.primary_anchors.contains(&"eu_tfr".to_string()));
        assert!(bp.procedures.len() >= 4);
    }

    #[test]
    fn casp_onboarding_cites_eu_mica_articles() {
        let bp = load_kyc_blueprint_yaml(BUILTIN_CASP_ONBOARDING).unwrap();
        let refs = bp.all_obligation_refs();
        let mica_count = refs
            .iter()
            .filter(|r| r.starts_with("obl:eu_mica:"))
            .count();
        assert!(
            mica_count >= 5,
            "expected ≥5 EU MiCA citations; got {mica_count}"
        );
    }

    #[test]
    fn builtin_kyc_blueprints_loads_all_six() {
        let bps = builtin_kyc_blueprints().unwrap();
        assert_eq!(bps.len(), 6);
        let ids: Vec<&str> = bps.iter().map(|b| b.id.as_str()).collect();
        assert!(ids.contains(&"kyc_casp_onboarding"));
        assert!(ids.contains(&"kyc_correspondent_onboarding"));
        assert!(ids.contains(&"kyc_onboarding_private_banking"));
        assert!(ids.contains(&"kyc_pkyc_review"));
        assert!(ids.contains(&"kyc_sanctions_hit_remediation"));
        assert!(ids.contains(&"kyc_sar_escalation"));
    }

    #[test]
    fn each_kyc_blueprint_has_at_least_one_procedure() {
        let bps = builtin_kyc_blueprints().unwrap();
        for bp in &bps {
            assert!(!bp.procedures.is_empty(), "{} has no procedures", bp.id);
        }
    }

    #[test]
    fn each_kyc_blueprint_has_at_least_one_step() {
        let bps = builtin_kyc_blueprints().unwrap();
        for bp in &bps {
            assert!(
                bp.total_steps() >= 1,
                "{} has no steps across its procedures",
                bp.id
            );
        }
    }

    #[test]
    fn kyc_total_step_count_above_minimum() {
        // Sanity floor — six KYC workflows should collectively
        // carry well over 30 steps.
        let bps = builtin_kyc_blueprints().unwrap();
        let total: usize = bps.iter().map(|b| b.total_steps()).sum();
        assert!(total >= 30, "expected ≥30 total KYC steps; got {total}");
    }

    #[test]
    fn load_builtin_by_id_returns_correct_blueprint() {
        let bp = load_builtin_kyc_blueprint("kyc_pkyc_review").unwrap();
        assert_eq!(bp.id, "kyc_pkyc_review");
    }

    #[test]
    fn load_builtin_by_unknown_id_returns_error() {
        let err = load_builtin_kyc_blueprint("kyc_nonexistent").unwrap_err();
        match err {
            AuditFsmError::SourceNotFound { source_id } => {
                assert_eq!(source_id, "kyc_nonexistent");
            }
            other => panic!("expected SourceNotFound, got {other:?}"),
        }
    }

    #[test]
    fn json_round_trips_kyc_blueprint() {
        let bp = load_kyc_blueprint_yaml(BUILTIN_CASP_ONBOARDING).unwrap();
        let json = serde_json::to_string(&bp).unwrap();
        let back: KycBlueprint = serde_json::from_str(&json).unwrap();
        assert_eq!(bp, back);
    }

    #[test]
    fn casp_discriminator_profile_lists_crypto_jurisdictions() {
        let bp = load_kyc_blueprint_yaml(BUILTIN_CASP_ONBOARDING).unwrap();
        let prof = bp
            .discriminator_profile
            .as_ref()
            .expect("CASP blueprint must carry a discriminator profile");
        assert!(prof.jurisdictions.contains(&"eu".to_string()));
    }
}

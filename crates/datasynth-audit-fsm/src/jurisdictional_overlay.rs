//! Jurisdictional overlays — adds country-specific audit procedures
//! on top of the ISA / PCAOB / IIA-GIAS baseline.
//!
//! Sourced from the AuditMethodology repo (`docs/blueprints/overlays/`).
//! Each overlay declares **applicability** (jurisdiction codes ISO-3166
//! alpha-2 + registrant types) and a list of **procedures** with
//! citation references to the relevant local statute or auditing
//! standard.
//!
//! # Schema (matches AuditMethodology v0.14)
//!
//! ```yaml
//! schema_version: "1.0.0"
//! overlay_id: <unique id>
//! name: <human readable>
//! description: |
//!   <multi-line>
//! applicability:
//!   jurisdictions: [au]            # ISO-3166 alpha-2 country codes
//!   registrant_types: [au_listed]  # see RegistrantType enum
//!   audit_types: []                # optional filter
//!   threshold: null                # optional materiality gate
//!   rationale: <human readable>
//! procedures:
//!   - id: <unique procedure id>
//!     name: <human readable>
//!     citation_refs: [<list of citation strings>]
//! ```
//!
//! # Built-in overlays
//!
//! 7 overlays embedded via `include_str!` and loaded once in the
//! [`builtin_jurisdictional_overlays`] registry:
//!
//! | Overlay | Jurisdiction | Registrant type            | Procedures |
//! |---------|-------------|---------------------------|------------|
//! | PCAOB   | us          | (any — empty list)         | 7          |
//! | EU CSRD | eu          | eu_listed_first_wave       | 4          |
//! | UK FRC  | uk          | uk_listed                  | 4          |
//! | ASIC    | au          | au_listed                  | 6          |
//! | JFSA    | jp          | jp_listed                  | 6          |
//! | ACRA    | sg          | sg_listed                  | 6          |
//! | HKICPA  | hk          | hk_listed                  | 6          |
//!
//! Total: 39 jurisdiction-specific procedures.
//!
//! # Resolution logic
//!
//! [`resolve_overlays`] applies AND-logic on
//! `(jurisdiction × registrant_type)`: an overlay fires only when
//! **both** the engagement's jurisdiction matches one of the overlay's
//! `applicability.jurisdictions` AND the engagement's `registrant_type`
//! matches one of the overlay's `applicability.registrant_types`.
//!
//! For dual-listed engagements (e.g. AU + JP listed entity), each
//! matching overlay fires independently — call sites typically iterate
//! the result and apply each overlay's procedures cumulatively.

use serde::{Deserialize, Serialize};

use crate::error::AuditFsmError;

// ── Public types ──────────────────────────────────────────────────────────────

/// One jurisdictional overlay loaded from a `*_overlay.yaml` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JurisdictionalOverlay {
    /// Schema version — currently `"1.0.0"`.
    pub schema_version: String,
    /// Unique overlay identifier (e.g. `"asic_overlay"`).
    pub overlay_id: String,
    /// Human-readable name.
    pub name: String,
    /// Multi-line description of what this overlay covers.
    pub description: String,
    /// Applicability — when this overlay fires.
    pub applicability: OverlayApplicability,
    /// Procedures the overlay adds on top of the baseline blueprint.
    pub procedures: Vec<OverlayProcedure>,
}

/// AND-logic applicability gate.  An overlay fires only when an
/// engagement's jurisdiction is in `jurisdictions` AND its
/// `registrant_type` is in `registrant_types`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayApplicability {
    /// ISO-3166 alpha-2 country codes (e.g. `["au"]`, `["us"]`).
    pub jurisdictions: Vec<String>,
    /// Registrant types (matches [`RegistrantType`] when known).
    pub registrant_types: Vec<String>,
    /// Optional audit-type filter (e.g. `["financial_statement"]`).
    /// Empty list means "any audit type matches".
    #[serde(default)]
    pub audit_types: Vec<String>,
    /// Optional materiality threshold (interpretation overlay-
    /// specific; currently informational only).
    #[serde(default)]
    pub threshold: Option<String>,
    /// Human-readable rationale shown in audit reports / debug
    /// output.
    pub rationale: String,
}

/// One procedure added by an overlay.  Procedure IDs are globally
/// unique (overlay-prefixed by convention, e.g. `asic_proc_*`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayProcedure {
    /// Unique procedure identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Citation references — local statute paragraphs, auditing
    /// standard paragraphs, etc.  Used by the FSM engine for evidence
    /// generation and audit-report citation.
    #[serde(default)]
    pub citation_refs: Vec<String>,
}

/// Registrant types known to the jurisdictional overlay registry.
/// Matches the AuditMethodology `RegistrantType` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrantType {
    /// US listed (NYSE / NASDAQ).
    UsListed,
    /// EU listed (any EU stock exchange).
    EuListed,
    /// EU listed — first-wave CSRD reporter (large public-interest
    /// entities, FY 2024 reporting under Directive (EU) 2022/2464).
    EuListedFirstWave,
    /// UK listed (LSE).
    UkListed,
    /// Australian listed (ASX).
    AuListed,
    /// Japanese listed (TSE / Nikkei).
    JpListed,
    /// Singapore listed (SGX).
    SgListed,
    /// Hong Kong listed (HKEX).
    HkListed,
    /// Private / non-listed entity — no jurisdictional overlay
    /// applies by default.
    Private,
}

impl RegistrantType {
    /// Convert to the snake_case string used in YAMLs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UsListed => "us_listed",
            Self::EuListed => "eu_listed",
            Self::EuListedFirstWave => "eu_listed_first_wave",
            Self::UkListed => "uk_listed",
            Self::AuListed => "au_listed",
            Self::JpListed => "jp_listed",
            Self::SgListed => "sg_listed",
            Self::HkListed => "hk_listed",
            Self::Private => "private",
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Parse a jurisdictional overlay from a YAML string.
pub fn load_overlay_yaml(yaml: &str) -> Result<JurisdictionalOverlay, AuditFsmError> {
    serde_yaml::from_str(yaml).map_err(|source| AuditFsmError::OverlayParse {
        path: "<jurisdictional>".to_string(),
        source,
    })
}

/// Return the full set of built-in jurisdictional overlays
/// (PCAOB, EU CSRD, UK FRC, ASIC, JFSA, ACRA, HKICPA).
///
/// Loaded eagerly from embedded YAML — calling this is cheap (just
/// allocates the parsed `Vec`).  Two calls produce structurally
/// identical results.
pub fn builtin_jurisdictional_overlays() -> Result<Vec<JurisdictionalOverlay>, AuditFsmError> {
    let yamls: &[&str] = &[
        BUILTIN_PCAOB,
        BUILTIN_EU_CSRD_LISTED,
        BUILTIN_UK_FRC,
        BUILTIN_ASIC,
        BUILTIN_JFSA,
        BUILTIN_ACRA,
        BUILTIN_HKICPA,
    ];
    yamls.iter().map(|y| load_overlay_yaml(y)).collect()
}

/// Resolve the set of jurisdictional overlays that apply to an
/// engagement with the given `(jurisdiction, registrant_type)` pair.
///
/// AND-logic: an overlay fires only when **both** the jurisdiction
/// matches one of the overlay's `applicability.jurisdictions` AND
/// the registrant type matches one of the overlay's
/// `applicability.registrant_types`.
///
/// `jurisdiction` is the ISO-3166 alpha-2 code (lowercase).  Multi-
/// jurisdictional engagements should call this once per jurisdiction
/// and union the results.
pub fn resolve_overlays(
    overlays: &[JurisdictionalOverlay],
    jurisdiction: &str,
    registrant_type: RegistrantType,
) -> Vec<JurisdictionalOverlay> {
    let registrant_str = registrant_type.as_str();
    overlays
        .iter()
        .filter(|o| {
            // Empty `jurisdictions` = wildcard (any).  Same for
            // `registrant_types`.  This matches the AuditMethodology
            // convention where omitted filters mean "no further
            // restriction beyond what's already specified".
            let jurisdiction_ok = o.applicability.jurisdictions.is_empty()
                || o.applicability
                    .jurisdictions
                    .iter()
                    .any(|j| j == jurisdiction);
            let registrant_ok = o.applicability.registrant_types.is_empty()
                || o.applicability
                    .registrant_types
                    .iter()
                    .any(|r| r == registrant_str);
            jurisdiction_ok && registrant_ok
        })
        .cloned()
        .collect()
}

// ── Embedded YAML ─────────────────────────────────────────────────────────────

const BUILTIN_PCAOB: &str = include_str!("../overlays/jurisdictional/pcaob_overlay.yaml");
const BUILTIN_EU_CSRD_LISTED: &str =
    include_str!("../overlays/jurisdictional/eu_csrd_listed_overlay.yaml");
const BUILTIN_UK_FRC: &str = include_str!("../overlays/jurisdictional/uk_frc_overlay.yaml");
const BUILTIN_ASIC: &str = include_str!("../overlays/jurisdictional/asic_overlay.yaml");
const BUILTIN_JFSA: &str = include_str!("../overlays/jurisdictional/jfsa_overlay.yaml");
const BUILTIN_ACRA: &str = include_str!("../overlays/jurisdictional/acra_overlay.yaml");
const BUILTIN_HKICPA: &str = include_str!("../overlays/jurisdictional/hkicpa_overlay.yaml");

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_overlay_yaml_parses_asic() {
        let overlay = load_overlay_yaml(BUILTIN_ASIC).unwrap();
        assert_eq!(overlay.schema_version, "1.0.0");
        assert_eq!(overlay.overlay_id, "asic_overlay");
        assert_eq!(overlay.applicability.jurisdictions, vec!["au"]);
        assert_eq!(overlay.applicability.registrant_types, vec!["au_listed"]);
        assert_eq!(overlay.procedures.len(), 6);
    }

    #[test]
    fn builtin_jurisdictional_overlays_loads_all_seven() {
        let overlays = builtin_jurisdictional_overlays().unwrap();
        assert_eq!(overlays.len(), 7);
        let ids: Vec<&str> = overlays.iter().map(|o| o.overlay_id.as_str()).collect();
        assert!(ids.contains(&"pcaob_overlay"));
        assert!(ids.contains(&"eu_csrd_listed_overlay"));
        assert!(ids.contains(&"uk_frc_overlay"));
        assert!(ids.contains(&"asic_overlay"));
        assert!(ids.contains(&"jfsa_overlay"));
        assert!(ids.contains(&"acra_overlay"));
        assert!(ids.contains(&"hkicpa_overlay"));
    }

    #[test]
    fn builtin_jurisdictional_overlays_total_procedure_count() {
        let overlays = builtin_jurisdictional_overlays().unwrap();
        let total: usize = overlays.iter().map(|o| o.procedures.len()).sum();
        // PCAOB(7) + EU_CSRD(4) + UK_FRC(4) + ASIC(6) + JFSA(6)
        // + ACRA(6) + HKICPA(6) = 39 jurisdiction-specific procedures.
        assert_eq!(total, 39);
    }

    #[test]
    fn resolve_overlays_au_listed_returns_only_asic() {
        let overlays = builtin_jurisdictional_overlays().unwrap();
        let resolved = resolve_overlays(&overlays, "au", RegistrantType::AuListed);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].overlay_id, "asic_overlay");
    }

    #[test]
    fn resolve_overlays_jp_listed_returns_only_jfsa() {
        let overlays = builtin_jurisdictional_overlays().unwrap();
        let resolved = resolve_overlays(&overlays, "jp", RegistrantType::JpListed);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].overlay_id, "jfsa_overlay");
    }

    #[test]
    fn resolve_overlays_sg_listed_returns_only_acra() {
        let overlays = builtin_jurisdictional_overlays().unwrap();
        let resolved = resolve_overlays(&overlays, "sg", RegistrantType::SgListed);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].overlay_id, "acra_overlay");
    }

    #[test]
    fn resolve_overlays_hk_listed_returns_only_hkicpa() {
        let overlays = builtin_jurisdictional_overlays().unwrap();
        let resolved = resolve_overlays(&overlays, "hk", RegistrantType::HkListed);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].overlay_id, "hkicpa_overlay");
    }

    #[test]
    fn resolve_overlays_us_listed_returns_only_pcaob() {
        // PCAOB has empty `registrant_types` (= wildcard), so it
        // matches any us-jurisdiction registrant including UsListed.
        let overlays = builtin_jurisdictional_overlays().unwrap();
        let resolved = resolve_overlays(&overlays, "us", RegistrantType::UsListed);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].overlay_id, "pcaob_overlay");
    }

    #[test]
    fn resolve_overlays_eu_csrd_first_wave_listed_returns_csrd_overlay() {
        let overlays = builtin_jurisdictional_overlays().unwrap();
        let resolved = resolve_overlays(&overlays, "eu", RegistrantType::EuListedFirstWave);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].overlay_id, "eu_csrd_listed_overlay");
    }

    #[test]
    fn resolve_overlays_uk_listed_returns_uk_frc() {
        let overlays = builtin_jurisdictional_overlays().unwrap();
        let resolved = resolve_overlays(&overlays, "uk", RegistrantType::UkListed);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].overlay_id, "uk_frc_overlay");
    }

    #[test]
    fn resolve_overlays_and_logic_jurisdiction_mismatch_returns_empty() {
        // AU registrant but UK jurisdiction — overlay should NOT fire.
        let overlays = builtin_jurisdictional_overlays().unwrap();
        let resolved = resolve_overlays(&overlays, "gb", RegistrantType::AuListed);
        assert!(resolved.is_empty());
    }

    #[test]
    fn resolve_overlays_and_logic_registrant_mismatch_returns_empty() {
        // AU jurisdiction but JP registrant — overlay should NOT fire.
        let overlays = builtin_jurisdictional_overlays().unwrap();
        let resolved = resolve_overlays(&overlays, "au", RegistrantType::JpListed);
        assert!(resolved.is_empty());
    }

    #[test]
    fn resolve_overlays_private_registrant_only_matches_wildcard_overlays() {
        // PCAOB has empty `registrant_types` so a private registrant
        // in the US matches it.  The strictly typed overlays (ASIC /
        // JFSA / ACRA / HKICPA / UK FRC / EU CSRD) require listed
        // registrants and don't fire for `Private`.
        let overlays = builtin_jurisdictional_overlays().unwrap();
        for jurisdiction in ["uk", "au", "jp", "sg", "hk", "eu"] {
            let resolved = resolve_overlays(&overlays, jurisdiction, RegistrantType::Private);
            assert!(
                resolved.is_empty(),
                "private registrant should match nothing in {jurisdiction}",
            );
        }
        // US is the wildcard exception (PCAOB applicability).
        let us_resolved = resolve_overlays(&overlays, "us", RegistrantType::Private);
        assert_eq!(us_resolved.len(), 1);
        assert_eq!(us_resolved[0].overlay_id, "pcaob_overlay");
    }

    #[test]
    fn registrant_type_as_str_matches_yaml_convention() {
        assert_eq!(RegistrantType::UsListed.as_str(), "us_listed");
        assert_eq!(RegistrantType::AuListed.as_str(), "au_listed");
        assert_eq!(RegistrantType::JpListed.as_str(), "jp_listed");
        assert_eq!(RegistrantType::HkListed.as_str(), "hk_listed");
    }

    #[test]
    fn asia_pacific_overlays_each_have_six_procedures() {
        // Sanity check on the 4 Asia-Pacific overlays (ASIC / JFSA /
        // ACRA / HKICPA) — each carries 6 procedures.
        let overlays = builtin_jurisdictional_overlays().unwrap();
        for id in &[
            "asic_overlay",
            "jfsa_overlay",
            "acra_overlay",
            "hkicpa_overlay",
        ] {
            let o = overlays.iter().find(|o| o.overlay_id == *id).expect(id);
            assert_eq!(o.procedures.len(), 6, "{} should have 6 procedures", id);
        }
    }
}

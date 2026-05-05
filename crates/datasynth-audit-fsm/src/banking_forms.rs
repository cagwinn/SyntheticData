//! Banking form ontologies — public-source reconstructions of
//! industry-standard banking due-diligence forms (Wolfsberg, MROS, UBS).
//!
//! Sourced from `data/kyc/forms/` + `data/kyc/export/banking_form_evidence_index.json`
//! in AuditMethodology v0.14.
//!
//! Each `BankingForm` carries structured field metadata: section grouping,
//! field type, KYC-blueprint step linkage, obligation citations.  Real-world
//! template parsing populates this schema; this module ships 7 built-in
//! reconstructions plus a cross-form evidence-mapping index that unifies
//! fields under canonical terms (e.g. `legal_entity_name` → MROS SAR +
//! Wolfsberg CBDDQ + Wolfsberg FCCQ).
//!
//! # Built-in forms
//!
//! | form_id | kind | source | fields |
//! |---------|------|--------|-------:|
//! | `mros_sar_v1` | SAR | MROS | 36 |
//! | `ubs_form_a_beneficial_owner_v1` | BeneficialOwner | UBS public | 17 |
//! | `ubs_kyc_identification_v1` | DueDiligence | UBS public | 15 |
//! | `ubs_source_of_funds_v1` | SourceOfFunds | UBS public | 12 |
//! | `ubs_tax_compliance_v1` | TaxCompliance | UBS public | 8 |
//! | `wolfsberg_cbddq_v1_4` | CorrespondentBankingQuestionnaire | Wolfsberg | 90 |
//! | `wolfsberg_fccq_v1_2` | FinancialCrimeCompliance | Wolfsberg | 50 |
//! | **total** | | | **228** |

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::AuditFsmError;

// ── Banking form kinds and sources ────────────────────────────────────────────

/// What category of banking form this is (drives downstream validation
/// and report generation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BankingFormKind {
    /// General due-diligence form.
    DueDiligence,
    /// Suspicious Activity Report.
    Sar,
    /// Beneficial-owner declaration (e.g. Swiss CDB Form A).
    BeneficialOwner,
    /// Source-of-funds declaration.
    SourceOfFunds,
    /// Tax-compliance / FATCA / CRS self-certification.
    TaxCompliance,
    /// Wolfsberg CBDDQ (Correspondent Banking Due Diligence Questionnaire).
    CorrespondentBankingQuestionnaire,
    /// Wolfsberg FCCQ (Financial Crime Compliance Questionnaire).
    FinancialCrimeCompliance,
}

/// Issuing body / source the banking form was reconstructed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BankingFormSource {
    /// Wolfsberg Group standards (CBDDQ, FCCQ, PB principles).
    Wolfsberg,
    /// Swiss MROS / Money Laundering Reporting Office.
    Mros,
    /// Swiss FINMA.
    Finma,
    /// Public-source UBS template reconstruction.
    UbsPublic,
    /// EU AMLA-derived form.
    EuAmla,
    /// Generic / framework-agnostic form.
    Generic,
}

/// Field type — drives validation downstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    /// Free-form text.
    Text,
    /// ISO date string.
    Date,
    /// Numeric value.
    Numeric,
    /// Boolean true / false.
    Boolean,
    /// Single-pick from `enum_values`.
    EnumSingle,
    /// Multi-pick from `enum_values`.
    EnumMulti,
    /// List of strings.
    List,
    /// Reference to a stored document.
    DocumentReference,
}

// ── Form / section / field types ──────────────────────────────────────────────

/// One field on a banking form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormField {
    /// Field id, unique within the section.
    pub field_id: String,
    /// Human-readable label.
    pub label: String,
    /// Field type for downstream validation.
    pub field_type: FieldType,
    /// Whether the field must be filled.
    #[serde(default = "default_true")]
    pub is_required: bool,
    /// Free-form description.
    #[serde(default)]
    pub description: String,
    /// Canonical term used to unify this field across forms (e.g.
    /// `legal_entity_name` shared between MROS SAR + Wolfsberg CBDDQ +
    /// Wolfsberg FCCQ).  Empty if no cross-form unification applies.
    #[serde(default)]
    pub canonical_term: String,
    /// Obligation citations (e.g. `obl:eu_amlr_2024_1624:21:0`).
    #[serde(default)]
    pub obligation_refs: Vec<String>,
    /// Allowed values for `EnumSingle` / `EnumMulti` field types.
    #[serde(default)]
    pub enum_values: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// One section in a banking form (groups related fields).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormSection {
    /// Section id, unique within the form.
    pub section_id: String,
    /// Section title.
    pub title: String,
    /// Free-form description.
    #[serde(default)]
    pub description: String,
    /// Fields in this section (must contain at least one).
    pub fields: Vec<FormField>,
}

/// A banking form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankingForm {
    /// Form id (e.g. `wolfsberg_cbddq_v1_4`).
    pub form_id: String,
    /// Display name.
    pub name: String,
    /// Form version.
    pub version: String,
    /// What category of form this is.
    pub kind: BankingFormKind,
    /// Issuing body / source.
    pub source: BankingFormSource,
    /// Free-form description.
    #[serde(default)]
    pub description: String,
    /// Public URL where the original template can be found.
    #[serde(default)]
    pub public_url: Option<String>,
    /// Whether this is a public-source reconstruction (vs a verbatim copy
    /// — for the UBS forms this is always true).
    #[serde(default)]
    pub is_public_reconstruction: bool,
    /// Sections (each must contain at least one field).
    pub sections: Vec<FormSection>,
}

impl BankingForm {
    /// Total field count across all sections.
    pub fn total_fields(&self) -> usize {
        self.sections.iter().map(|s| s.fields.len()).sum()
    }
}

// ── Cross-form evidence index ─────────────────────────────────────────────────

/// One evidence-mapping entry — links a (form_id, section_id, field_id)
/// triple to a canonical term, obligation refs, and the KYC-blueprint
/// step ids that emit that evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankingFormEvidence {
    pub form_id: String,
    pub section_id: String,
    pub field_id: String,
    pub canonical_term: String,
    #[serde(default)]
    pub obligation_refs: Vec<String>,
    #[serde(default)]
    pub blueprint_step_ids: Vec<String>,
}

/// Cross-form evidence index — flat list of every (form, section, field)
/// triple unified by canonical term.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankingFormEvidenceIndex {
    pub entries: Vec<BankingFormEvidence>,
}

// ── Built-in loaders ──────────────────────────────────────────────────────────

const MROS_SAR_YAML: &str = include_str!("../banking_forms/mros_sar_template.yaml");
const UBS_FORM_A_YAML: &str = include_str!("../banking_forms/ubs_form_a_beneficial_owner.yaml");
const UBS_KYC_ID_YAML: &str = include_str!("../banking_forms/ubs_kyc_identification.yaml");
const UBS_SOURCE_OF_FUNDS_YAML: &str = include_str!("../banking_forms/ubs_source_of_funds.yaml");
const UBS_TAX_COMPLIANCE_YAML: &str = include_str!("../banking_forms/ubs_tax_compliance.yaml");
const WOLFSBERG_CBDDQ_YAML: &str = include_str!("../banking_forms/wolfsberg_cbddq_v1_4.yaml");
const WOLFSBERG_FCCQ_YAML: &str = include_str!("../banking_forms/wolfsberg_fccq_v1_2.yaml");

const EVIDENCE_INDEX_JSON: &str = include_str!("../banking_forms/banking_form_evidence_index.json");

/// Load all 7 built-in banking forms in declaration order:
/// MROS SAR → UBS Form A → UBS KYC ID → UBS SoF → UBS Tax →
/// Wolfsberg CBDDQ → Wolfsberg FCCQ.
pub fn builtin_banking_forms() -> Result<Vec<BankingForm>, AuditFsmError> {
    let pairs: [(&str, &str); 7] = [
        (MROS_SAR_YAML, "banking_forms/mros_sar_template.yaml"),
        (
            UBS_FORM_A_YAML,
            "banking_forms/ubs_form_a_beneficial_owner.yaml",
        ),
        (UBS_KYC_ID_YAML, "banking_forms/ubs_kyc_identification.yaml"),
        (
            UBS_SOURCE_OF_FUNDS_YAML,
            "banking_forms/ubs_source_of_funds.yaml",
        ),
        (
            UBS_TAX_COMPLIANCE_YAML,
            "banking_forms/ubs_tax_compliance.yaml",
        ),
        (
            WOLFSBERG_CBDDQ_YAML,
            "banking_forms/wolfsberg_cbddq_v1_4.yaml",
        ),
        (
            WOLFSBERG_FCCQ_YAML,
            "banking_forms/wolfsberg_fccq_v1_2.yaml",
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

/// Load the built-in cross-form evidence index (228 entries pinning
/// every form-section-field triple to a canonical term + obligation
/// refs + KYC-blueprint step ids).
///
/// Parsed via `serde_yaml` because JSON is a valid YAML 1.2 subset —
/// avoids threading a separate JSON-error variant through the FSM
/// error enum for the single embedded JSON payload.
pub fn builtin_banking_form_evidence_index() -> Result<BankingFormEvidenceIndex, AuditFsmError> {
    serde_yaml::from_str(EVIDENCE_INDEX_JSON).map_err(|source| AuditFsmError::BlueprintParse {
        path: "banking_forms/banking_form_evidence_index.json".to_string(),
        source,
    })
}

// ── Cross-form helpers ────────────────────────────────────────────────────────

/// Return all evidence entries that pin to a given canonical term.
/// Useful for walking every form that captures the same conceptual
/// data point (e.g. `legal_entity_name`).
pub fn entries_for_canonical_term<'a>(
    index: &'a BankingFormEvidenceIndex,
    canonical_term: &str,
) -> Vec<&'a BankingFormEvidence> {
    index
        .entries
        .iter()
        .filter(|e| e.canonical_term == canonical_term)
        .collect()
}

/// Return all evidence entries for a given form.
pub fn entries_for_form<'a>(
    index: &'a BankingFormEvidenceIndex,
    form_id: &str,
) -> Vec<&'a BankingFormEvidence> {
    index
        .entries
        .iter()
        .filter(|e| e.form_id == form_id)
        .collect()
}

/// Build `canonical_term → set<form_id>` to find which canonical terms
/// are unified across multiple forms (the cross-domain unification points).
pub fn forms_by_canonical_term(
    index: &BankingFormEvidenceIndex,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for e in &index.entries {
        out.entry(e.canonical_term.clone())
            .or_default()
            .insert(e.form_id.clone());
    }
    out
}

/// Return the canonical terms shared across at least `min_forms` forms.
/// `min_forms = 2` returns every cross-form unification point.
pub fn shared_canonical_terms(
    index: &BankingFormEvidenceIndex,
    min_forms: usize,
) -> BTreeMap<String, BTreeSet<String>> {
    forms_by_canonical_term(index)
        .into_iter()
        .filter(|(_, forms)| forms.len() >= min_forms)
        .collect()
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_seven_built_in_forms() {
        let forms = builtin_banking_forms().unwrap();
        assert_eq!(forms.len(), 7);
        let ids: Vec<&str> = forms.iter().map(|f| f.form_id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "mros_sar_v1",
                "ubs_form_a_beneficial_owner_v1",
                "ubs_kyc_identification_v1",
                "ubs_source_of_funds_v1",
                "ubs_tax_compliance_v1",
                "wolfsberg_cbddq_v1_4",
                "wolfsberg_fccq_v1_2",
            ]
        );
    }

    #[test]
    fn form_kinds_and_sources_round_trip_yaml() {
        let forms = builtin_banking_forms().unwrap();
        let kinds: Vec<BankingFormKind> = forms.iter().map(|f| f.kind).collect();
        assert!(kinds.contains(&BankingFormKind::Sar));
        assert!(kinds.contains(&BankingFormKind::BeneficialOwner));
        assert!(kinds.contains(&BankingFormKind::TaxCompliance));
        assert!(kinds.contains(&BankingFormKind::CorrespondentBankingQuestionnaire));
        assert!(kinds.contains(&BankingFormKind::FinancialCrimeCompliance));

        let sources: Vec<BankingFormSource> = forms.iter().map(|f| f.source).collect();
        assert!(sources.iter().any(|s| matches!(s, BankingFormSource::Mros)));
        assert!(sources
            .iter()
            .any(|s| matches!(s, BankingFormSource::Wolfsberg)));
        assert!(sources
            .iter()
            .any(|s| matches!(s, BankingFormSource::UbsPublic)));
    }

    #[test]
    fn ubs_forms_are_public_reconstructions() {
        let forms = builtin_banking_forms().unwrap();
        for f in &forms {
            if matches!(f.source, BankingFormSource::UbsPublic) {
                assert!(
                    f.is_public_reconstruction,
                    "{} should be flagged",
                    f.form_id
                );
            }
        }
    }

    #[test]
    fn each_section_has_at_least_one_field() {
        let forms = builtin_banking_forms().unwrap();
        for f in &forms {
            assert!(!f.sections.is_empty(), "form {} has no sections", f.form_id);
            for s in &f.sections {
                assert!(
                    !s.fields.is_empty(),
                    "section {} of form {} has no fields",
                    s.section_id,
                    f.form_id
                );
            }
        }
    }

    #[test]
    fn section_ids_unique_within_form() {
        let forms = builtin_banking_forms().unwrap();
        for f in &forms {
            let ids: Vec<&str> = f.sections.iter().map(|s| s.section_id.as_str()).collect();
            let unique: BTreeSet<&str> = ids.iter().copied().collect();
            assert_eq!(
                ids.len(),
                unique.len(),
                "duplicate sections in {}",
                f.form_id
            );
        }
    }

    #[test]
    fn field_ids_unique_within_section() {
        let forms = builtin_banking_forms().unwrap();
        for f in &forms {
            for s in &f.sections {
                let ids: Vec<&str> = s.fields.iter().map(|q| q.field_id.as_str()).collect();
                let unique: BTreeSet<&str> = ids.iter().copied().collect();
                assert_eq!(
                    ids.len(),
                    unique.len(),
                    "duplicate fields in {} / {}",
                    f.form_id,
                    s.section_id
                );
            }
        }
    }

    #[test]
    fn total_field_counts_match_methodology_v014() {
        let forms = builtin_banking_forms().unwrap();
        let by_id: BTreeMap<&str, usize> = forms
            .iter()
            .map(|f| (f.form_id.as_str(), f.total_fields()))
            .collect();
        assert_eq!(by_id["mros_sar_v1"], 36);
        assert_eq!(by_id["ubs_form_a_beneficial_owner_v1"], 17);
        assert_eq!(by_id["ubs_kyc_identification_v1"], 15);
        assert_eq!(by_id["ubs_source_of_funds_v1"], 12);
        assert_eq!(by_id["ubs_tax_compliance_v1"], 8);
        assert_eq!(by_id["wolfsberg_cbddq_v1_4"], 90);
        assert_eq!(by_id["wolfsberg_fccq_v1_2"], 50);
        let total: usize = by_id.values().sum();
        assert_eq!(total, 228);
    }

    #[test]
    fn evidence_index_loads_with_228_entries() {
        let idx = builtin_banking_form_evidence_index().unwrap();
        assert_eq!(idx.entries.len(), 228);
    }

    #[test]
    fn evidence_index_form_ids_match_built_in_forms() {
        let forms = builtin_banking_forms().unwrap();
        let form_ids: BTreeSet<&str> = forms.iter().map(|f| f.form_id.as_str()).collect();
        let idx = builtin_banking_form_evidence_index().unwrap();
        for entry in &idx.entries {
            assert!(
                form_ids.contains(entry.form_id.as_str()),
                "evidence references unknown form {}",
                entry.form_id
            );
        }
    }

    #[test]
    fn evidence_index_field_ids_match_form_field_ids() {
        let forms = builtin_banking_forms().unwrap();
        let mut field_lookup: BTreeMap<(&str, &str, &str), &FormField> = BTreeMap::new();
        for f in &forms {
            for s in &f.sections {
                for q in &s.fields {
                    field_lookup.insert((&f.form_id, &s.section_id, &q.field_id), q);
                }
            }
        }
        let idx = builtin_banking_form_evidence_index().unwrap();
        for entry in &idx.entries {
            let key = (
                entry.form_id.as_str(),
                entry.section_id.as_str(),
                entry.field_id.as_str(),
            );
            assert!(
                field_lookup.contains_key(&key),
                "evidence (form={}, section={}, field={}) not found in any form",
                entry.form_id,
                entry.section_id,
                entry.field_id
            );
        }
    }

    #[test]
    fn entries_for_canonical_term_legal_entity_name_spans_three_forms() {
        let idx = builtin_banking_form_evidence_index().unwrap();
        let entries = entries_for_canonical_term(&idx, "legal_entity_name");
        let forms: BTreeSet<&str> = entries.iter().map(|e| e.form_id.as_str()).collect();
        assert!(forms.contains("mros_sar_v1"));
        assert!(forms.contains("wolfsberg_cbddq_v1_4"));
        assert!(forms.contains("wolfsberg_fccq_v1_2"));
        assert!(forms.len() >= 3);
    }

    #[test]
    fn entries_for_form_returns_only_that_form() {
        let idx = builtin_banking_form_evidence_index().unwrap();
        let entries = entries_for_form(&idx, "ubs_tax_compliance_v1");
        assert!(!entries.is_empty());
        for e in &entries {
            assert_eq!(e.form_id, "ubs_tax_compliance_v1");
        }
    }

    #[test]
    fn forms_by_canonical_term_finds_at_least_ten_shared_terms() {
        let idx = builtin_banking_form_evidence_index().unwrap();
        let shared = shared_canonical_terms(&idx, 2);
        assert!(
            shared.len() >= 10,
            "expected ≥10 cross-form unification points, got {}",
            shared.len()
        );
    }

    #[test]
    fn json_round_trips_each_form_and_index() {
        for form in builtin_banking_forms().unwrap() {
            let json = serde_json::to_string(&form).unwrap();
            let back: BankingForm = serde_json::from_str(&json).unwrap();
            assert_eq!(form, back);
        }
        let idx = builtin_banking_form_evidence_index().unwrap();
        let json = serde_json::to_string(&idx).unwrap();
        let back: BankingFormEvidenceIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(idx, back);
    }

    #[test]
    fn obligation_refs_use_obl_slug_format() {
        let forms = builtin_banking_forms().unwrap();
        let mut total_refs = 0usize;
        for f in &forms {
            for s in &f.sections {
                for q in &s.fields {
                    for r in &q.obligation_refs {
                        total_refs += 1;
                        assert!(r.starts_with("obl:"), "bad ref: {r}");
                    }
                }
            }
        }
        assert!(total_refs > 0, "no obligation refs found");
    }
}

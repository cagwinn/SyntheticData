//! LLM-powered contextual anomaly scheme designer.
//!
//! Given a company profile, industry context, and control environment, the
//! [`AnomalyDesigner`] uses an LLM to design fraud schemes that are realistic
//! for that specific context. This goes beyond template-based anomaly injection
//! by adapting patterns to the control weaknesses and business processes present.
//!
//! Designed schemes are represented as [`DesignedScheme`] structs that can be
//! converted to [`SchemeStage`] sequences for the existing [`FraudScheme`]
//! infrastructure or cached for reuse without further LLM calls.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use datasynth_core::error::SynthError;
use datasynth_core::llm::{LlmProvider, LlmRequest};

/// Company context provided to the LLM for contextual anomaly design.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyContext {
    /// Industry (e.g., "manufacturing", "retail", "financial_services").
    pub industry: String,
    /// Company size: "small", "medium", or "large".
    pub company_size: String,
    /// Country code (e.g., "US", "DE").
    pub country: String,
    /// Number of employees (approximate).
    pub employee_count: Option<u32>,
    /// Annual revenue range (for sizing amounts realistically).
    pub annual_revenue: Option<String>,
}

/// Control environment context — what controls exist and how mature they are.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlContext {
    /// COSO maturity level: "ad_hoc", "repeatable", "defined", "managed", "optimized".
    pub maturity_level: String,
    /// Known weak controls (by name or ID, e.g., "three_way_match", "C003").
    pub weak_controls: Vec<String>,
    /// SOD violations present (e.g., "AP clerk also approves payments").
    pub sod_gaps: Vec<String>,
    /// Whether an external audit is active.
    pub audit_active: bool,
    /// IT general control deficiencies.
    pub itgc_gaps: Vec<String>,
}

/// A fraud scheme designed by the LLM for a specific company/control context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignedScheme {
    /// Unique name for this scheme (e.g., "procurement_kickback_weak_3wm").
    pub name: String,
    /// Human-readable narrative describing the scheme.
    pub narrative: String,
    /// Which control weaknesses this scheme exploits.
    pub exploited_weaknesses: Vec<String>,
    /// Expected detection signals an analyst should look for.
    pub detection_signals: Vec<String>,
    /// Ordered stages of the scheme.
    pub stages: Vec<DesignedStage>,
    /// Overall difficulty rating for detection.
    pub difficulty: String,
    /// Estimated total financial impact range.
    pub impact_range: (f64, f64),
    /// Confidence the LLM has in this scheme's realism (0.0-1.0).
    pub realism_score: f64,
}

/// A single stage within a designed scheme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignedStage {
    /// Stage name (e.g., "setup", "testing", "escalation").
    pub name: String,
    /// What happens in this stage.
    pub description: String,
    /// Duration in months.
    pub duration_months: u32,
    /// Amount range per transaction.
    pub amount_min: f64,
    pub amount_max: f64,
    /// Transaction count range.
    pub transaction_count_min: u32,
    pub transaction_count_max: u32,
    /// Concealment methods used.
    pub concealment: Vec<String>,
}

/// Result of a design request, containing one or more schemes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignResult {
    /// Designed schemes, ordered by realism score descending.
    pub schemes: Vec<DesignedScheme>,
    /// Context summary used for the design.
    pub context_summary: String,
}

/// Designs contextually appropriate anomaly/fraud schemes using an LLM.
///
/// The designer takes a company profile and control environment, then asks
/// the LLM to design realistic fraud patterns that would exploit the specific
/// weaknesses present. Results are cacheable for reuse without LLM calls.
pub struct AnomalyDesigner {
    provider: Arc<dyn LlmProvider>,
}

impl AnomalyDesigner {
    /// Create a new designer with the given LLM provider.
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    /// Design fraud schemes appropriate for the given context.
    ///
    /// Returns up to `max_schemes` designed schemes, or falls back to
    /// template-based schemes if the LLM call fails.
    pub fn design(
        &self,
        company: &CompanyContext,
        controls: &ControlContext,
        max_schemes: usize,
    ) -> Result<DesignResult, SynthError> {
        let context_summary = self.build_context_summary(company, controls);
        let prompt = self.build_design_prompt(company, controls, max_schemes);

        let request = LlmRequest::new(prompt)
            .with_system(Self::system_prompt().to_string())
            .with_temperature(0.4)
            .with_max_tokens(4096);

        let schemes = match self.provider.complete(&request) {
            Ok(response) => {
                let parsed = self.parse_schemes(&response.content, max_schemes);
                if parsed.is_empty() {
                    tracing::debug!("LLM returned no parseable schemes, using fallback");
                    Self::fallback_schemes(company, controls, max_schemes)
                } else {
                    parsed
                }
            }
            Err(e) => {
                tracing::warn!("LLM scheme design failed: {e}, using fallback templates");
                Self::fallback_schemes(company, controls, max_schemes)
            }
        };

        Ok(DesignResult {
            schemes,
            context_summary,
        })
    }

    /// Build a human-readable summary of the context.
    fn build_context_summary(&self, company: &CompanyContext, controls: &ControlContext) -> String {
        format!(
            "{} {} company in {} | Controls: {} maturity, {} weak controls, {} SoD gaps",
            company.company_size,
            company.industry,
            company.country,
            controls.maturity_level,
            controls.weak_controls.len(),
            controls.sod_gaps.len(),
        )
    }

    /// Build the design prompt.
    fn build_design_prompt(
        &self,
        company: &CompanyContext,
        controls: &ControlContext,
        max_schemes: usize,
    ) -> String {
        let mut prompt = String::with_capacity(2048);

        prompt.push_str(&format!(
            "Design {max_schemes} realistic fraud scheme(s) for this company:\n\n"
        ));

        prompt.push_str(&format!("Industry: {}\n", company.industry));
        prompt.push_str(&format!("Size: {}\n", company.company_size));
        prompt.push_str(&format!("Country: {}\n", company.country));
        if let Some(count) = company.employee_count {
            prompt.push_str(&format!("Employees: ~{count}\n"));
        }
        if let Some(ref rev) = company.annual_revenue {
            prompt.push_str(&format!("Revenue: {rev}\n"));
        }

        prompt.push_str(&format!(
            "\nControl maturity: {}\n",
            controls.maturity_level
        ));
        if !controls.weak_controls.is_empty() {
            prompt.push_str(&format!(
                "Weak controls: {}\n",
                controls.weak_controls.join(", ")
            ));
        }
        if !controls.sod_gaps.is_empty() {
            prompt.push_str(&format!("SoD gaps: {}\n", controls.sod_gaps.join(", ")));
        }
        if !controls.itgc_gaps.is_empty() {
            prompt.push_str(&format!("IT gaps: {}\n", controls.itgc_gaps.join(", ")));
        }
        if controls.audit_active {
            prompt.push_str("Note: external audit is currently active\n");
        }

        prompt
    }

    /// Parse LLM response into designed schemes.
    fn parse_schemes(&self, content: &str, max_schemes: usize) -> Vec<DesignedScheme> {
        // Try to extract JSON array from response
        let json_str = datasynth_core::llm::extract_json_array(content);

        match json_str {
            Some(json) => match serde_json::from_str::<Vec<DesignedScheme>>(json) {
                Ok(mut schemes) => {
                    schemes.truncate(max_schemes);
                    // Sort by realism score descending
                    schemes.sort_by(|a, b| {
                        b.realism_score
                            .partial_cmp(&a.realism_score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                    schemes
                }
                Err(e) => {
                    tracing::debug!("Failed to parse LLM schemes: {e}");
                    vec![]
                }
            },
            None => {
                tracing::debug!("No JSON array found in LLM scheme response");
                vec![]
            }
        }
    }

    /// Produce template-based fallback schemes when LLM is unavailable.
    ///
    /// Selects schemes based on industry and control weaknesses.
    fn fallback_schemes(
        company: &CompanyContext,
        controls: &ControlContext,
        max_schemes: usize,
    ) -> Vec<DesignedScheme> {
        let mut schemes = Vec::new();

        // If three-way match is weak, suggest procurement fraud
        if controls
            .weak_controls
            .iter()
            .any(|c| c.contains("three_way") || c.contains("C003") || c.contains("procurement"))
        {
            schemes.push(DesignedScheme {
                name: "procurement_kickback".to_string(),
                narrative: format!(
                    "A procurement manager at this {} {} company exploits weak three-way match \
                     controls to process inflated invoices from a colluding vendor. The kickback \
                     is structured as consulting fees to a separate entity.",
                    company.company_size, company.industry
                ),
                exploited_weaknesses: vec!["three_way_match".to_string()],
                detection_signals: vec![
                    "Vendor invoice amounts consistently 10-20% above market".to_string(),
                    "Single-source procurement without competitive bids".to_string(),
                    "Correlated timing between vendor payments and personal deposits".to_string(),
                ],
                stages: vec![
                    DesignedStage {
                        name: "testing".to_string(),
                        description: "Small inflated invoices to test detection".to_string(),
                        duration_months: 2,
                        amount_min: 500.0,
                        amount_max: 2000.0,
                        transaction_count_min: 2,
                        transaction_count_max: 5,
                        concealment: vec!["document_manipulation".to_string()],
                    },
                    DesignedStage {
                        name: "escalation".to_string(),
                        description: "Larger invoices with fabricated services".to_string(),
                        duration_months: 6,
                        amount_min: 2000.0,
                        amount_max: 15000.0,
                        transaction_count_min: 3,
                        transaction_count_max: 8,
                        concealment: vec![
                            "document_manipulation".to_string(),
                            "approval_circumvention".to_string(),
                        ],
                    },
                    DesignedStage {
                        name: "acceleration".to_string(),
                        description: "Maximum extraction before potential detection".to_string(),
                        duration_months: 3,
                        amount_min: 10000.0,
                        amount_max: 50000.0,
                        transaction_count_min: 5,
                        transaction_count_max: 12,
                        concealment: vec![
                            "transaction_splitting".to_string(),
                            "timing_exploitation".to_string(),
                        ],
                    },
                ],
                difficulty: "hard".to_string(),
                impact_range: (25000.0, 250000.0),
                realism_score: 0.85,
            });
        }

        // If SoD gaps exist, suggest approval override scheme
        if !controls.sod_gaps.is_empty() {
            schemes.push(DesignedScheme {
                name: "sod_exploitation".to_string(),
                narrative: format!(
                    "An employee with conflicting duties ({}) creates and approves their own \
                     expense reimbursements and vendor payments, gradually increasing amounts.",
                    controls.sod_gaps.first().cloned().unwrap_or_default()
                ),
                exploited_weaknesses: controls.sod_gaps.clone(),
                detection_signals: vec![
                    "Same user creates and approves transactions".to_string(),
                    "Expense amounts cluster just below approval thresholds".to_string(),
                    "Weekend/after-hours transaction approvals".to_string(),
                ],
                stages: vec![
                    DesignedStage {
                        name: "setup".to_string(),
                        description: "Establish pattern of legitimate self-approvals".to_string(),
                        duration_months: 1,
                        amount_min: 100.0,
                        amount_max: 500.0,
                        transaction_count_min: 3,
                        transaction_count_max: 6,
                        concealment: vec!["approval_circumvention".to_string()],
                    },
                    DesignedStage {
                        name: "exploitation".to_string(),
                        description: "Fraudulent reimbursements mixed with legitimate ones"
                            .to_string(),
                        duration_months: 8,
                        amount_min: 500.0,
                        amount_max: 9500.0,
                        transaction_count_min: 2,
                        transaction_count_max: 6,
                        concealment: vec![
                            "approval_circumvention".to_string(),
                            "transaction_splitting".to_string(),
                        ],
                    },
                ],
                difficulty: "moderate".to_string(),
                impact_range: (5000.0, 75000.0),
                realism_score: 0.80,
            });
        }

        // Generic revenue manipulation for any company
        if schemes.len() < max_schemes {
            schemes.push(DesignedScheme {
                name: "revenue_timing_manipulation".to_string(),
                narrative: format!(
                    "Management at this {} company accelerates revenue recognition \
                     near quarter-end to meet analyst expectations, booking sales before \
                     delivery criteria are met.",
                    company.industry
                ),
                exploited_weaknesses: vec!["period_close_pressure".to_string()],
                detection_signals: vec![
                    "Spike in revenue entries in last 3 days of quarter".to_string(),
                    "Reversals in first week of new quarter".to_string(),
                    "Revenue entries without matching delivery documentation".to_string(),
                ],
                stages: vec![
                    DesignedStage {
                        name: "channel_stuffing".to_string(),
                        description: "Push sales to distributors with side agreements".to_string(),
                        duration_months: 1,
                        amount_min: 50000.0,
                        amount_max: 500000.0,
                        transaction_count_min: 5,
                        transaction_count_max: 20,
                        concealment: vec![
                            "timing_exploitation".to_string(),
                            "document_manipulation".to_string(),
                        ],
                    },
                    DesignedStage {
                        name: "reversal_concealment".to_string(),
                        description: "Offset reversals through credit notes and returns"
                            .to_string(),
                        duration_months: 2,
                        amount_min: 10000.0,
                        amount_max: 200000.0,
                        transaction_count_min: 3,
                        transaction_count_max: 10,
                        concealment: vec![
                            "account_misclassification".to_string(),
                            "timing_exploitation".to_string(),
                        ],
                    },
                ],
                difficulty: "hard".to_string(),
                impact_range: (100000.0, 2000000.0),
                realism_score: 0.75,
            });
        }

        // IT-specific scheme if ITGC gaps present
        if !controls.itgc_gaps.is_empty() && schemes.len() < max_schemes {
            schemes.push(DesignedScheme {
                name: "it_control_bypass".to_string(),
                narrative: format!(
                    "An IT administrator exploits weak change management controls ({}) \
                     to modify transaction records directly in the database, creating \
                     ghost vendor payments that bypass application-level controls.",
                    controls.itgc_gaps.first().cloned().unwrap_or_default()
                ),
                exploited_weaknesses: controls.itgc_gaps.clone(),
                detection_signals: vec![
                    "Database changes without corresponding application audit trail".to_string(),
                    "Vendor master data changes outside business hours".to_string(),
                    "Payments to newly created vendors with no purchase history".to_string(),
                ],
                stages: vec![DesignedStage {
                    name: "direct_manipulation".to_string(),
                    description: "Direct database inserts for fictitious vendor payments"
                        .to_string(),
                    duration_months: 4,
                    amount_min: 5000.0,
                    amount_max: 25000.0,
                    transaction_count_min: 1,
                    transaction_count_max: 3,
                    concealment: vec![
                        "it_control_bypass".to_string(),
                        "audit_trail_manipulation".to_string(),
                    ],
                }],
                difficulty: "expert".to_string(),
                impact_range: (20000.0, 100000.0),
                realism_score: 0.70,
            });
        }

        schemes.truncate(max_schemes);
        schemes
    }

    /// System prompt for the LLM scheme designer.
    fn system_prompt() -> &'static str {
        concat!(
            "You are a fraud risk expert designing realistic fraud scenarios for synthetic data ",
            "generation. Given a company profile and control environment, design fraud schemes that:\n",
            "1. Exploit the SPECIFIC control weaknesses described\n",
            "2. Follow realistic multi-stage progressions (testing → escalation → acceleration)\n",
            "3. Include concealment techniques appropriate to the control maturity level\n",
            "4. Scale amounts realistically for the company size and industry\n\n",
            "Return a JSON array of scheme objects. Each scheme has:\n",
            "- name: short identifier\n",
            "- narrative: 2-3 sentence description of the scheme\n",
            "- exploited_weaknesses: [list of control weaknesses exploited]\n",
            "- detection_signals: [observable indicators an analyst should look for]\n",
            "- stages: [{name, description, duration_months, amount_min, amount_max, ",
            "transaction_count_min, transaction_count_max, concealment: [techniques]}]\n",
            "- difficulty: \"trivial\" | \"easy\" | \"moderate\" | \"hard\" | \"expert\"\n",
            "- impact_range: [min_total, max_total]\n",
            "- realism_score: 0.0-1.0 confidence in realism\n\n",
            "Valid concealment techniques: document_manipulation, approval_circumvention, ",
            "timing_exploitation, transaction_splitting, account_misclassification, ",
            "collusion, it_control_bypass, audit_trail_manipulation\n\n",
            "Return ONLY the JSON array.\n"
        )
    }
}

/// Extract a JSON array from potentially noisy LLM output (test helper).
#[cfg(test)]
fn extract_json_array(content: &str) -> Option<&str> {
    datasynth_core::llm::extract_json_array(content)
}

/// A library of cached designed schemes for reuse without LLM calls.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SchemeLibrary {
    /// Cached schemes keyed by a context fingerprint.
    entries: Vec<SchemeLibraryEntry>,
}

/// A single entry in the scheme library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemeLibraryEntry {
    /// Context summary that produced these schemes.
    pub context_summary: String,
    /// Industry this was designed for.
    pub industry: String,
    /// Control maturity this was designed for.
    pub maturity_level: String,
    /// The designed schemes.
    pub schemes: Vec<DesignedScheme>,
}

impl SchemeLibrary {
    /// Create a new empty library.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a design result to the library.
    pub fn add(&mut self, result: &DesignResult, industry: &str, maturity_level: &str) {
        self.entries.push(SchemeLibraryEntry {
            context_summary: result.context_summary.clone(),
            industry: industry.to_string(),
            maturity_level: maturity_level.to_string(),
            schemes: result.schemes.clone(),
        });
    }

    /// Find cached schemes matching the given context.
    pub fn find(&self, industry: &str, maturity_level: &str) -> Option<&[DesignedScheme]> {
        self.entries
            .iter()
            .find(|e| e.industry == industry && e.maturity_level == maturity_level)
            .map(|e| e.schemes.as_slice())
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the library is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Save the library to a JSON file.
    pub fn save(&self, path: &std::path::Path) -> Result<(), SynthError> {
        let json = serde_json::to_string_pretty(self).map_err(|e| {
            SynthError::generation(format!("Failed to serialize scheme library: {e}"))
        })?;
        std::fs::write(path, json)
            .map_err(|e| SynthError::generation(format!("Failed to write scheme library: {e}")))?;
        Ok(())
    }

    /// Load the library from a JSON file.
    pub fn load(path: &std::path::Path) -> Result<Self, SynthError> {
        let json = std::fs::read_to_string(path)
            .map_err(|e| SynthError::generation(format!("Failed to read scheme library: {e}")))?;
        let lib: Self = serde_json::from_str(&json)
            .map_err(|e| SynthError::generation(format!("Failed to parse scheme library: {e}")))?;
        Ok(lib)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use datasynth_core::llm::MockLlmProvider;

    fn sample_company() -> CompanyContext {
        CompanyContext {
            industry: "manufacturing".to_string(),
            company_size: "medium".to_string(),
            country: "US".to_string(),
            employee_count: Some(500),
            annual_revenue: Some("$50M-100M".to_string()),
        }
    }

    fn weak_controls() -> ControlContext {
        ControlContext {
            maturity_level: "repeatable".to_string(),
            weak_controls: vec!["three_way_match".to_string(), "C003".to_string()],
            sod_gaps: vec!["AP clerk approves payments".to_string()],
            audit_active: false,
            itgc_gaps: vec![],
        }
    }

    fn strong_controls() -> ControlContext {
        ControlContext {
            maturity_level: "managed".to_string(),
            weak_controls: vec![],
            sod_gaps: vec![],
            audit_active: true,
            itgc_gaps: vec![],
        }
    }

    #[test]
    fn test_design_with_weak_controls() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let designer = AnomalyDesigner::new(provider);

        let result = designer
            .design(&sample_company(), &weak_controls(), 3)
            .unwrap();

        // Should produce fallback schemes since mock doesn't return JSON
        assert!(!result.schemes.is_empty());
        assert!(!result.context_summary.is_empty());

        // Should exploit the weak three-way match
        let has_procurement = result
            .schemes
            .iter()
            .any(|s| s.name.contains("procurement"));
        assert!(
            has_procurement,
            "Should design procurement fraud for weak 3-way match"
        );

        // Should exploit the SoD gap
        let has_sod = result.schemes.iter().any(|s| s.name.contains("sod"));
        assert!(has_sod, "Should design SoD exploitation scheme");
    }

    #[test]
    fn test_design_with_strong_controls() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let designer = AnomalyDesigner::new(provider);

        let result = designer
            .design(&sample_company(), &strong_controls(), 2)
            .unwrap();

        // With strong controls, fewer specific exploits — gets generic revenue scheme
        assert!(!result.schemes.is_empty());
        // Should NOT have procurement kickback (no weak 3-way match)
        let has_procurement = result
            .schemes
            .iter()
            .any(|s| s.name.contains("procurement"));
        assert!(!has_procurement);
    }

    #[test]
    fn test_design_with_itgc_gaps() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let designer = AnomalyDesigner::new(provider);

        let controls = ControlContext {
            maturity_level: "ad_hoc".to_string(),
            weak_controls: vec![],
            sod_gaps: vec![],
            audit_active: false,
            itgc_gaps: vec!["weak_change_management".to_string()],
        };

        let result = designer.design(&sample_company(), &controls, 3).unwrap();

        let has_it = result.schemes.iter().any(|s| s.name.contains("it_control"));
        assert!(has_it, "Should design IT control bypass scheme");
    }

    #[test]
    fn test_fallback_scheme_stages_are_valid() {
        let schemes = AnomalyDesigner::fallback_schemes(&sample_company(), &weak_controls(), 5);

        for scheme in &schemes {
            assert!(!scheme.name.is_empty());
            assert!(!scheme.narrative.is_empty());
            assert!(!scheme.stages.is_empty());
            assert!(scheme.realism_score > 0.0 && scheme.realism_score <= 1.0);
            assert!(scheme.impact_range.0 < scheme.impact_range.1);

            for stage in &scheme.stages {
                assert!(!stage.name.is_empty());
                assert!(stage.duration_months > 0);
                assert!(stage.amount_min <= stage.amount_max);
                assert!(stage.transaction_count_min <= stage.transaction_count_max);
            }
        }
    }

    #[test]
    fn test_scheme_library_roundtrip() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let designer = AnomalyDesigner::new(provider);

        let result = designer
            .design(&sample_company(), &weak_controls(), 2)
            .unwrap();

        let mut lib = SchemeLibrary::new();
        assert!(lib.is_empty());

        lib.add(&result, "manufacturing", "repeatable");
        assert_eq!(lib.len(), 1);

        let found = lib.find("manufacturing", "repeatable");
        assert!(found.is_some());
        assert!(!found.unwrap().is_empty());

        assert!(lib.find("retail", "managed").is_none());
    }

    #[test]
    fn test_scheme_library_save_load() {
        let mut lib = SchemeLibrary::new();
        lib.add(
            &DesignResult {
                schemes: vec![DesignedScheme {
                    name: "test_scheme".to_string(),
                    narrative: "Test".to_string(),
                    exploited_weaknesses: vec![],
                    detection_signals: vec![],
                    stages: vec![],
                    difficulty: "moderate".to_string(),
                    impact_range: (1000.0, 5000.0),
                    realism_score: 0.8,
                }],
                context_summary: "test".to_string(),
            },
            "retail",
            "defined",
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.json");

        lib.save(&path).unwrap();
        let loaded = SchemeLibrary::load(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(loaded.find("retail", "defined").is_some());
    }

    #[test]
    fn test_parse_valid_llm_schemes() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let designer = AnomalyDesigner::new(provider);

        let json = r#"[{
            "name": "llm_scheme",
            "narrative": "A scheme designed by LLM",
            "exploited_weaknesses": ["weak_auth"],
            "detection_signals": ["unusual timing"],
            "stages": [{
                "name": "phase1",
                "description": "Initial phase",
                "duration_months": 3,
                "amount_min": 1000.0,
                "amount_max": 5000.0,
                "transaction_count_min": 2,
                "transaction_count_max": 8,
                "concealment": ["timing_exploitation"]
            }],
            "difficulty": "hard",
            "impact_range": [5000.0, 50000.0],
            "realism_score": 0.9
        }]"#;

        let schemes = designer.parse_schemes(json, 5);
        assert_eq!(schemes.len(), 1);
        assert_eq!(schemes[0].name, "llm_scheme");
        assert!((schemes[0].realism_score - 0.9).abs() < 1e-10);
        assert_eq!(schemes[0].stages.len(), 1);
        assert_eq!(schemes[0].stages[0].duration_months, 3);
    }

    #[test]
    fn test_max_schemes_limit() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let designer = AnomalyDesigner::new(provider);

        // Weak controls + SoD + ITGC = would produce 4 schemes, but limit to 2
        let controls = ControlContext {
            maturity_level: "ad_hoc".to_string(),
            weak_controls: vec!["three_way_match".to_string()],
            sod_gaps: vec!["AP approves".to_string()],
            audit_active: false,
            itgc_gaps: vec!["change_mgmt".to_string()],
        };

        let result = designer.design(&sample_company(), &controls, 2).unwrap();
        assert!(result.schemes.len() <= 2);
    }

    #[test]
    fn test_extract_json_array() {
        let input = "Here are schemes: [{\"name\": \"a\"}] end";
        assert!(extract_json_array(input).is_some());
        assert!(extract_json_array("no array here").is_none());
    }
}

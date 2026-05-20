//! LLM-powered audit finding enrichment (v4.1.1+).
//!
//! Generates realistic finding titles and narratives for audit
//! engagements via an LLM, with deterministic template fallbacks.
//! Mirrors the shape of [`super::vendor_enricher::VendorLlmEnricher`]
//! but covers two paired outputs (title + narrative) per invocation.

use std::sync::Arc;

use datasynth_core::error::SynthError;
use datasynth_core::llm::{LlmProvider, LlmRequest};

/// Enriches audit finding metadata using an LLM provider.
pub struct FindingLlmEnricher {
    provider: Arc<dyn LlmProvider>,
}

impl FindingLlmEnricher {
    /// Create a new enricher with the given LLM provider.
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    /// Generate a single finding title for the given category.
    ///
    /// `finding_type` is a canonical snake_case name like
    /// `"material_weakness"`, `"significant_deficiency"`,
    /// `"control_deficiency"`, `"compliance_issue"`.
    ///
    /// `severity` is `"critical"`, `"high"`, `"medium"`, `"low"`.
    pub fn enrich_finding_title(
        &self,
        finding_type: &str,
        severity: &str,
        area: &str,
    ) -> Result<String, SynthError> {
        let prompt = format!(
            "Generate a single realistic audit finding TITLE for a \
             {severity}-severity {finding_type} in the {area} area. \
             Return ONLY the title — a short noun phrase (5-12 words), \
             no period at the end, no explanation."
        );
        let request = LlmRequest::new(prompt)
            .with_system(
                "You are an audit methodology expert. Return only a \
                 single finding title as a short noun phrase."
                    .to_string(),
            )
            .with_max_tokens(48)
            .with_temperature(0.7);
        match self.provider.complete(&request) {
            Ok(response) => {
                let t = response.content.trim().to_string();
                if t.is_empty() {
                    Ok(Self::fallback_title(finding_type, severity, area))
                } else {
                    Ok(t)
                }
            }
            Err(_) => Ok(Self::fallback_title(finding_type, severity, area)),
        }
    }

    /// Generate a single finding narrative for the given category +
    /// section. Sections: `"condition"`, `"criteria"`, `"cause"`,
    /// `"effect"`, `"recommendation"`.
    pub fn enrich_finding_narrative(
        &self,
        finding_type: &str,
        section: &str,
        area: &str,
    ) -> Result<String, SynthError> {
        let prompt = format!(
            "Write a single-paragraph '{section}' narrative (2-4 \
             sentences, 40-80 words) for an audit finding of type \
             '{finding_type}' in the '{area}' area. Use active voice. \
             Do not include the section name at the start. Do not use \
             bullet points."
        );
        let request = LlmRequest::new(prompt)
            .with_system(
                "You are an audit methodology expert. Write only the \
                 requested narrative text — no heading, no preamble."
                    .to_string(),
            )
            .with_max_tokens(180)
            .with_temperature(0.7);
        match self.provider.complete(&request) {
            Ok(response) => {
                let t = response.content.trim().to_string();
                if t.is_empty() {
                    Ok(Self::fallback_narrative(finding_type, section, area))
                } else {
                    Ok(t)
                }
            }
            Err(_) => Ok(Self::fallback_narrative(finding_type, section, area)),
        }
    }

    /// Batch variant — `requests` is `(finding_type, severity, area)`.
    pub fn enrich_titles_batch(
        &self,
        requests: &[(String, String, String)],
        seed: u64,
    ) -> Result<Vec<String>, SynthError> {
        let llm_requests: Vec<LlmRequest> = requests
            .iter()
            .enumerate()
            .map(|(i, (ft, sev, area))| {
                let prompt = format!(
                    "Generate a single realistic audit finding TITLE for a \
                     {sev}-severity {ft} in the {area} area. Return ONLY the \
                     title as a short noun phrase (5-12 words), no period."
                );
                LlmRequest::new(prompt)
                    .with_system(
                        "You are an audit methodology expert. Return only a \
                         single finding title."
                            .to_string(),
                    )
                    .with_max_tokens(48)
                    .with_temperature(0.7)
                    .with_seed(seed.wrapping_add(i as u64))
            })
            .collect();
        match self.provider.complete_batch(&llm_requests) {
            Ok(responses) => {
                let titles: Vec<String> = responses
                    .iter()
                    .enumerate()
                    .map(|(i, resp)| {
                        let t = resp.content.trim().to_string();
                        if t.is_empty() {
                            let (ref ft, ref sev, ref area) = requests[i];
                            Self::fallback_title(ft, sev, area)
                        } else {
                            t
                        }
                    })
                    .collect();
                Ok(titles)
            }
            Err(_) => Ok(requests
                .iter()
                .map(|(ft, sev, area)| Self::fallback_title(ft, sev, area))
                .collect()),
        }
    }

    fn fallback_title(finding_type: &str, severity: &str, area: &str) -> String {
        let type_phrase = match finding_type.to_lowercase().as_str() {
            "material_weakness" => "Material weakness in",
            "significant_deficiency" => "Significant deficiency in",
            "control_deficiency" => "Control deficiency affecting",
            "compliance_issue" | "compliance" => "Compliance gap in",
            "sox_deficiency" | "sox" => "SOX-relevant deficiency in",
            "design_deficiency" => "Design deficiency in",
            "operating_deficiency" => "Operating deficiency in",
            _ => "Finding in",
        };
        let severity_tag = match severity.to_lowercase().as_str() {
            "critical" => " (critical)",
            "high" => " (high)",
            "medium" | "moderate" => "",
            "low" => " (low)",
            _ => "",
        };
        format!("{type_phrase} {area}{severity_tag}")
    }

    fn fallback_narrative(finding_type: &str, section: &str, area: &str) -> String {
        match section.to_lowercase().as_str() {
            "condition" => format!(
                "During our procedures we noted a {finding_type} in the {area} area. \
                 Specific attributes were observed that fall below the standard expected of \
                 effective internal control."
            ),
            "criteria" => format!(
                "The applicable framework requires effective design and operation of \
                 controls over {area}. This includes defined policies, proper segregation, \
                 and documented review."
            ),
            "cause" => format!(
                "The underlying cause relates to gaps in the design and operation of \
                 {area} controls, compounded by inadequate monitoring of the {finding_type}."
            ),
            "effect" => format!(
                "The effect is an elevated risk of error or fraud in the {area} process, \
                 with potential impact on the reliability of related financial reporting."
            ),
            "recommendation" => format!(
                "We recommend management strengthen the {area} controls by reviewing the \
                 existing procedures, remediating the {finding_type}, and introducing \
                 ongoing monitoring."
            ),
            _ => format!(
                "A {finding_type} was identified in {area}. Management should review \
                 and remediate in line with established control expectations."
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datasynth_core::llm::MockLlmProvider;

    #[test]
    fn title_non_empty() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let enricher = FindingLlmEnricher::new(provider);
        let t = enricher
            .enrich_finding_title("material_weakness", "high", "Revenue Recognition")
            .unwrap();
        assert!(!t.is_empty());
    }

    #[test]
    fn narrative_non_empty() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let enricher = FindingLlmEnricher::new(provider);
        let n = enricher
            .enrich_finding_narrative("control_deficiency", "recommendation", "Purchase Orders")
            .unwrap();
        assert!(!n.is_empty());
    }

    #[test]
    fn fallback_title_material_weakness_high() {
        let t = FindingLlmEnricher::fallback_title("material_weakness", "high", "Treasury");
        assert!(t.starts_with("Material weakness in"));
        assert!(t.contains("Treasury"));
        assert!(t.ends_with("(high)"));
    }

    #[test]
    fn fallback_narrative_recommendation() {
        let n = FindingLlmEnricher::fallback_narrative(
            "sox_deficiency",
            "recommendation",
            "Access Controls",
        );
        assert!(n.contains("recommend"));
        assert!(n.contains("Access Controls"));
    }

    #[test]
    fn batch_length_preserved() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let enricher = FindingLlmEnricher::new(provider);
        let requests = vec![
            (
                "material_weakness".into(),
                "critical".into(),
                "Revenue".into(),
            ),
            ("control_deficiency".into(), "medium".into(), "AP".into()),
        ];
        let titles = enricher.enrich_titles_batch(&requests, 100).unwrap();
        assert_eq!(titles.len(), 2);
    }
}

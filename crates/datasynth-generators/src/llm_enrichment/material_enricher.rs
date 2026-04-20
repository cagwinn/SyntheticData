//! LLM-powered material description enrichment (v3.5.0+).
//!
//! Generates realistic material descriptions from `(material_type, industry)`
//! context with a deterministic fallback.

use std::sync::Arc;

use datasynth_core::error::SynthError;
use datasynth_core::llm::{LlmProvider, LlmRequest};

/// Enriches material descriptions using an LLM provider.
pub struct MaterialLlmEnricher {
    provider: Arc<dyn LlmProvider>,
}

impl MaterialLlmEnricher {
    /// Create a new enricher with the given LLM provider.
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    /// Generate a single realistic material description.
    pub fn enrich_material_description(
        &self,
        material_type: &str,
        industry: &str,
    ) -> Result<String, SynthError> {
        let prompt = format!(
            "Generate a single realistic material/product description for a {industry} company's \
             {material_type} catalog. Return a short phrase (3-8 words) describing one SKU, \
             with no manufacturer name, no units, no SKU number, and no explanation. \
             Examples: 'Stainless steel flange DN50', 'Premium leather office chair', \
             'Injection-molded plastic housing'."
        );

        let request = LlmRequest::new(prompt)
            .with_system(
                "You are a product catalog writer. Return only a single short descriptive \
                 phrase, no extra text."
                    .to_string(),
            )
            .with_max_tokens(48)
            .with_temperature(0.8);

        match self.provider.complete(&request) {
            Ok(response) => {
                let desc = response.content.trim().to_string();
                if desc.is_empty() {
                    Ok(Self::fallback_material_description(material_type, industry))
                } else {
                    Ok(desc)
                }
            }
            Err(_) => Ok(Self::fallback_material_description(material_type, industry)),
        }
    }

    /// Generate material descriptions in batch.
    pub fn enrich_batch(
        &self,
        requests: &[(String, String)],
        seed: u64,
    ) -> Result<Vec<String>, SynthError> {
        let llm_requests: Vec<LlmRequest> = requests
            .iter()
            .enumerate()
            .map(|(i, (material_type, industry))| {
                let prompt = format!(
                    "Generate a single realistic material/product description for a {industry} company's \
                     {material_type} catalog. Return a short phrase (3-8 words) describing one SKU, \
                     with no manufacturer name, no units, no SKU number, and no explanation."
                );
                LlmRequest::new(prompt)
                    .with_system(
                        "You are a product catalog writer. Return only a single short descriptive \
                         phrase, no extra text."
                            .to_string(),
                    )
                    .with_max_tokens(48)
                    .with_temperature(0.8)
                    .with_seed(seed.wrapping_add(i as u64))
            })
            .collect();

        match self.provider.complete_batch(&llm_requests) {
            Ok(responses) => {
                let descs: Vec<String> = responses
                    .iter()
                    .enumerate()
                    .map(|(i, resp)| {
                        let desc = resp.content.trim().to_string();
                        if desc.is_empty() {
                            let (ref mt, ref ind) = requests[i];
                            Self::fallback_material_description(mt, ind)
                        } else {
                            desc
                        }
                    })
                    .collect();
                Ok(descs)
            }
            Err(_) => {
                let descs = requests
                    .iter()
                    .map(|(mt, ind)| Self::fallback_material_description(mt, ind))
                    .collect();
                Ok(descs)
            }
        }
    }

    /// Deterministic fallback description.
    fn fallback_material_description(material_type: &str, industry: &str) -> String {
        let type_phrase = match material_type.to_lowercase().as_str() {
            "raw_materials" | "raw materials" => "Raw material",
            "components" | "component" => "Component part",
            "finished_goods" | "finished goods" => "Finished product",
            "packaging" => "Packaging supply",
            "tooling" | "equipment" => "Equipment unit",
            _ => "Material",
        };

        let industry_suffix = match industry.to_lowercase().as_str() {
            "manufacturing" => "for industrial use",
            "retail" => "for retail distribution",
            "healthcare" => "for medical supply",
            "technology" => "for technology product",
            "financial_services" => "for back-office use",
            _ => "for general use",
        };

        format!("{type_phrase} {industry_suffix}")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use datasynth_core::llm::MockLlmProvider;

    #[test]
    fn enrich_material_nonempty() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let enricher = MaterialLlmEnricher::new(provider);
        let desc = enricher
            .enrich_material_description("raw_materials", "manufacturing")
            .expect("should succeed");
        assert!(!desc.is_empty());
    }

    #[test]
    fn enrich_batch_preserves_length() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let enricher = MaterialLlmEnricher::new(provider);
        let requests = vec![
            ("components".into(), "manufacturing".into()),
            ("finished_goods".into(), "retail".into()),
        ];
        let descs = enricher.enrich_batch(&requests, 100).unwrap();
        assert_eq!(descs.len(), 2);
    }

    #[test]
    fn fallback_manufacturing_raw_materials() {
        let desc =
            MaterialLlmEnricher::fallback_material_description("raw_materials", "manufacturing");
        assert_eq!(desc, "Raw material for industrial use");
    }
}

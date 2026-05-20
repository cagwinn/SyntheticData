//! LLM-powered customer name enrichment (v3.5.0+).
//!
//! Mirrors [`super::vendor_enricher::VendorLlmEnricher`]: generates
//! realistic customer names from `(industry, segment, country)` context
//! with a deterministic template-based fallback when the LLM provider
//! errors or returns empty content.

use std::sync::Arc;

use datasynth_core::error::SynthError;
use datasynth_core::llm::{LlmProvider, LlmRequest};

/// Enriches customer names using an LLM provider.
pub struct CustomerLlmEnricher {
    provider: Arc<dyn LlmProvider>,
}

impl CustomerLlmEnricher {
    /// Create a new enricher with the given LLM provider.
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    /// Generate a single realistic customer name.
    pub fn enrich_customer_name(
        &self,
        industry: &str,
        segment: &str,
        country: &str,
    ) -> Result<String, SynthError> {
        let prompt = format!(
            "Generate a single realistic customer company name for a {industry} business \
             in {country} serving the {segment} segment. Return ONLY the company name, \
             nothing else."
        );

        let request = LlmRequest::new(prompt)
            .with_system(
                "You are a business data generator. Return only a single company name \
                 with no explanation or extra text."
                    .to_string(),
            )
            .with_max_tokens(64)
            .with_temperature(0.8);

        match self.provider.complete(&request) {
            Ok(response) => {
                let name = response.content.trim().to_string();
                if name.is_empty() {
                    Ok(Self::fallback_customer_name(industry, segment, country))
                } else {
                    Ok(name)
                }
            }
            Err(_) => Ok(Self::fallback_customer_name(industry, segment, country)),
        }
    }

    /// Generate customer names in batch. Each tuple is
    /// `(industry, segment, country)`; a deterministic seed is applied
    /// per-request for reproducibility.
    pub fn enrich_batch(
        &self,
        requests: &[(String, String, String)],
        seed: u64,
    ) -> Result<Vec<String>, SynthError> {
        let llm_requests: Vec<LlmRequest> = requests
            .iter()
            .enumerate()
            .map(|(i, (industry, segment, country))| {
                let prompt = format!(
                    "Generate a single realistic customer company name for a {industry} business \
                     in {country} serving the {segment} segment. Return ONLY the company name, \
                     nothing else."
                );
                LlmRequest::new(prompt)
                    .with_system(
                        "You are a business data generator. Return only a single company name \
                         with no explanation or extra text."
                            .to_string(),
                    )
                    .with_max_tokens(64)
                    .with_temperature(0.8)
                    .with_seed(seed.wrapping_add(i as u64))
            })
            .collect();

        match self.provider.complete_batch(&llm_requests) {
            Ok(responses) => {
                let names: Vec<String> = responses
                    .iter()
                    .enumerate()
                    .map(|(i, resp)| {
                        let name = resp.content.trim().to_string();
                        if name.is_empty() {
                            let (ref ind, ref seg, ref cty) = requests[i];
                            Self::fallback_customer_name(ind, seg, cty)
                        } else {
                            name
                        }
                    })
                    .collect();
                Ok(names)
            }
            Err(_) => {
                let names = requests
                    .iter()
                    .map(|(ind, seg, cty)| Self::fallback_customer_name(ind, seg, cty))
                    .collect();
                Ok(names)
            }
        }
    }

    /// Deterministic fallback customer name. Shape mirrors the vendor
    /// fallback but uses customer-leaning descriptors.
    fn fallback_customer_name(industry: &str, segment: &str, country: &str) -> String {
        let industry_prefix = match industry.to_lowercase().as_str() {
            "manufacturing" => "Industrial",
            "retail" => "Retail",
            "financial_services" | "finance" => "Financial",
            "healthcare" => "Health",
            "technology" => "Tech",
            _ => "Commercial",
        };

        let segment_suffix = match segment.to_lowercase().as_str() {
            "enterprise" => "Enterprises",
            "mid_market" | "midmarket" | "mid-market" => "Partners",
            "smb" | "small_business" => "Traders",
            "consumer" | "b2c" => "Consumers",
            "institutional" => "Institute",
            _ => "Group",
        };

        let country_tag = match country.to_uppercase().as_str() {
            "US" | "USA" => "",
            "DE" | "GERMANY" => " GmbH",
            "GB" | "UK" => " PLC",
            "FR" | "FRANCE" => " SAS",
            "IT" | "ITALY" => " SpA",
            "JP" | "JAPAN" => " KK",
            "CN" | "CHINA" => " Ltd (CN)",
            _ => " Intl",
        };

        format!("{industry_prefix} {segment_suffix}{country_tag}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datasynth_core::llm::MockLlmProvider;

    #[test]
    fn enrich_customer_name_nonempty() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let enricher = CustomerLlmEnricher::new(provider);
        let name = enricher
            .enrich_customer_name("retail", "enterprise", "US")
            .expect("should succeed");
        assert!(!name.is_empty());
    }

    #[test]
    fn enrich_batch_preserves_length() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let enricher = CustomerLlmEnricher::new(provider);
        let requests = vec![
            ("retail".into(), "consumer".into(), "US".into()),
            (
                "financial_services".into(),
                "institutional".into(),
                "DE".into(),
            ),
        ];
        let names = enricher.enrich_batch(&requests, 100).unwrap();
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn fallback_country_tag_germany() {
        let name = CustomerLlmEnricher::fallback_customer_name("manufacturing", "mid_market", "DE");
        assert_eq!(name, "Industrial Partners GmbH");
    }

    #[test]
    fn fallback_country_tag_france() {
        let name = CustomerLlmEnricher::fallback_customer_name("retail", "smb", "FR");
        assert_eq!(name, "Retail Traders SAS");
    }
}

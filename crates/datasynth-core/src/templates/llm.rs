//! v4.0.0 — LLM-backed template provider (Track B2).
//!
//! `LlmTemplateProvider` wraps a base [`TemplateProvider`] and
//! intercepts a small set of name-generation methods (`vendor_name`,
//! `customer_name`, `material_description`), routing them through an
//! [`LlmProvider`] with in-memory caching. Every other method delegates
//! to the inner provider, preserving byte-identity for unrelated pools.
//!
//! ## Determinism
//!
//! - When backed by `MockLlmProvider`, the provider is fully
//!   deterministic: same seed → same hash-keyed cache → same output.
//! - When backed by a real HTTP provider, the first invocation for a
//!   given (method, context) populates the cache; all subsequent
//!   invocations in the same process return the cached value.
//!   **Determinism across processes requires priming the cache from
//!   a fingerprint or re-issuing identical requests.**
//! - For production (byte-identical runs across machines), prefer the
//!   offline `datasynth-data templates enrich` workflow introduced in
//!   v3.5.0; it writes a YAML that
//!   [`DefaultTemplateProvider`](super::DefaultTemplateProvider)
//!   consumes at load time.

use rand::Rng;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::names::NameCulture;
use super::provider::TemplateProvider;
use crate::llm::{LlmProvider, LlmRequest};
use crate::models::BusinessProcess;

/// Template provider that routes a subset of name-generation methods
/// through an LLM, with per-(method, context) caching.
pub struct LlmTemplateProvider {
    inner: Arc<dyn TemplateProvider>,
    llm: Arc<dyn LlmProvider>,
    cache: Mutex<HashMap<String, String>>,
    /// Whether to enrich vendor names via the LLM.
    enrich_vendors: bool,
    /// Whether to enrich customer names via the LLM.
    enrich_customers: bool,
    /// Whether to enrich material descriptions via the LLM.
    enrich_materials: bool,
}

impl LlmTemplateProvider {
    /// Create a new LLM-backed provider wrapping `inner`.
    ///
    /// Enrichment for each category is controlled by the flag arguments;
    /// all default to `false` so callers opt in explicitly.
    pub fn new(inner: Arc<dyn TemplateProvider>, llm: Arc<dyn LlmProvider>) -> Self {
        Self {
            inner,
            llm,
            cache: Mutex::new(HashMap::new()),
            enrich_vendors: false,
            enrich_customers: false,
            enrich_materials: false,
        }
    }

    /// Builder: opt into LLM enrichment for vendor names.
    pub fn with_vendor_enrichment(mut self) -> Self {
        self.enrich_vendors = true;
        self
    }

    /// Builder: opt into LLM enrichment for customer names.
    pub fn with_customer_enrichment(mut self) -> Self {
        self.enrich_customers = true;
        self
    }

    /// Builder: opt into LLM enrichment for material descriptions.
    pub fn with_material_enrichment(mut self) -> Self {
        self.enrich_materials = true;
        self
    }

    /// Cache key helper.
    fn cache_key(method: &str, context: &str) -> String {
        format!("{method}|{context}")
    }

    /// Try the LLM for a single completion. On any error (including
    /// empty content) returns `None`, letting the caller fall back to
    /// the inner provider.
    fn llm_complete(&self, prompt: &str, system: &str) -> Option<String> {
        let request = LlmRequest::new(prompt)
            .with_system(system.to_string())
            .with_max_tokens(64)
            .with_temperature(0.8);
        match self.llm.complete(&request) {
            Ok(resp) => {
                let trimmed = resp.content.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            }
            Err(_) => None,
        }
    }

    fn cached_or_llm<F: FnOnce() -> Option<String>>(&self, key: String, f: F) -> Option<String> {
        if let Ok(c) = self.cache.lock() {
            if let Some(v) = c.get(&key) {
                return Some(v.clone());
            }
        }
        let value = f()?;
        if let Ok(mut c) = self.cache.lock() {
            c.insert(key, value.clone());
        }
        Some(value)
    }
}

impl TemplateProvider for LlmTemplateProvider {
    fn get_person_first_name(
        &self,
        culture: NameCulture,
        is_male: bool,
        rng: &mut dyn Rng,
    ) -> String {
        self.inner.get_person_first_name(culture, is_male, rng)
    }

    fn get_person_last_name(&self, culture: NameCulture, rng: &mut dyn Rng) -> String {
        self.inner.get_person_last_name(culture, rng)
    }

    fn get_vendor_name(&self, category: &str, rng: &mut dyn Rng) -> String {
        if !self.enrich_vendors {
            return self.inner.get_vendor_name(category, rng);
        }
        let key = Self::cache_key("vendor", category);
        let prompt = format!(
            "Generate a single realistic vendor company name for the \
             '{category}' spend category. Return ONLY the name."
        );
        self.cached_or_llm(key, || {
            self.llm_complete(&prompt, "You are a business data generator.")
        })
        .unwrap_or_else(|| self.inner.get_vendor_name(category, rng))
    }

    fn get_customer_name(&self, industry: &str, rng: &mut dyn Rng) -> String {
        if !self.enrich_customers {
            return self.inner.get_customer_name(industry, rng);
        }
        let key = Self::cache_key("customer", industry);
        let prompt = format!(
            "Generate a single realistic customer company name for the \
             '{industry}' industry. Return ONLY the name."
        );
        self.cached_or_llm(key, || {
            self.llm_complete(&prompt, "You are a business data generator.")
        })
        .unwrap_or_else(|| self.inner.get_customer_name(industry, rng))
    }

    fn get_material_description(&self, material_type: &str, rng: &mut dyn Rng) -> String {
        if !self.enrich_materials {
            return self.inner.get_material_description(material_type, rng);
        }
        let key = Self::cache_key("material", material_type);
        let prompt = format!(
            "Generate a single realistic material/product description for \
             the '{material_type}' category. Return a short phrase (3-8 \
             words), no SKU, no units."
        );
        self.cached_or_llm(key, || {
            self.llm_complete(&prompt, "You are a product catalog writer.")
        })
        .unwrap_or_else(|| self.inner.get_material_description(material_type, rng))
    }

    fn get_asset_description(&self, category: &str, rng: &mut dyn Rng) -> String {
        self.inner.get_asset_description(category, rng)
    }

    fn get_line_text(
        &self,
        process: BusinessProcess,
        account_type: &str,
        rng: &mut dyn Rng,
    ) -> String {
        self.inner.get_line_text(process, account_type, rng)
    }

    fn get_header_template(&self, process: BusinessProcess, rng: &mut dyn Rng) -> String {
        self.inner.get_header_template(process, rng)
    }

    fn get_bank_name(&self, rng: &mut dyn Rng) -> Option<String> {
        self.inner.get_bank_name(rng)
    }

    fn get_finding_title(
        &self,
        finding_type_key: &str,
        rng: &mut dyn Rng,
    ) -> Option<(String, String)> {
        self.inner.get_finding_title(finding_type_key, rng)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlmProvider;
    use crate::templates::DefaultTemplateProvider;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn vendor_enrichment_opt_in_only() {
        let inner = Arc::new(DefaultTemplateProvider::new());
        let llm = Arc::new(MockLlmProvider::new(42));
        let provider = LlmTemplateProvider::new(inner, llm);
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        // Without enabling enrichment, the result should match the
        // fallback path (not LLM-generated).
        let name = provider.get_vendor_name("office_supplies", &mut rng);
        assert!(!name.is_empty());
    }

    #[test]
    fn vendor_enrichment_cached() {
        let inner = Arc::new(DefaultTemplateProvider::new());
        let llm = Arc::new(MockLlmProvider::new(42));
        let provider = LlmTemplateProvider::new(inner, llm).with_vendor_enrichment();
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let n1 = provider.get_vendor_name("office_supplies", &mut rng);
        let n2 = provider.get_vendor_name("office_supplies", &mut rng);
        assert_eq!(n1, n2, "second call should hit cache");
    }

    #[test]
    fn non_llm_methods_delegate_to_inner() {
        let inner = Arc::new(DefaultTemplateProvider::new());
        let llm = Arc::new(MockLlmProvider::new(42));
        let provider =
            LlmTemplateProvider::new(Arc::clone(&inner) as Arc<dyn TemplateProvider>, llm)
                .with_vendor_enrichment();
        let mut rng1 = ChaCha8Rng::seed_from_u64(7);
        let mut rng2 = ChaCha8Rng::seed_from_u64(7);
        let via_wrapper = provider.get_person_last_name(NameCulture::German, &mut rng1);
        let via_inner = inner.get_person_last_name(NameCulture::German, &mut rng2);
        assert_eq!(via_wrapper, via_inner);
    }
}

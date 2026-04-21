//! Template provider trait and implementations.
//!
//! This module defines the `TemplateProvider` trait for accessing template data,
//! along with implementations that combine embedded and file-based templates.

use rand::seq::IndexedRandom;
use rand::Rng;
use std::sync::Arc;

use super::loader::{MergeStrategy, TemplateData, TemplateLoader};
use super::names::NameCulture;
use crate::models::BusinessProcess;

/// Trait for providing template data to generators.
///
/// This trait abstracts the source of template data, allowing generators
/// to work with either embedded templates, file-based templates, or a
/// combination of both.
///
/// Methods use `&mut dyn Rng` to allow the trait to be dyn-compatible.
pub trait TemplateProvider: Send + Sync {
    /// Get a random person first name for the given culture and gender.
    fn get_person_first_name(
        &self,
        culture: NameCulture,
        is_male: bool,
        rng: &mut dyn Rng,
    ) -> String;

    /// Get a random person last name for the given culture.
    fn get_person_last_name(&self, culture: NameCulture, rng: &mut dyn Rng) -> String;

    /// Get a random vendor name for the given category.
    fn get_vendor_name(&self, category: &str, rng: &mut dyn Rng) -> String;

    /// Get a random customer name for the given industry.
    fn get_customer_name(&self, industry: &str, rng: &mut dyn Rng) -> String;

    /// Get a random material description for the given type.
    fn get_material_description(&self, material_type: &str, rng: &mut dyn Rng) -> String;

    /// Get a random asset description for the given category.
    fn get_asset_description(&self, category: &str, rng: &mut dyn Rng) -> String;

    /// Get a random line text for the given process and account type.
    fn get_line_text(
        &self,
        process: BusinessProcess,
        account_type: &str,
        rng: &mut dyn Rng,
    ) -> String;

    /// Get a random header text template for the given process.
    fn get_header_template(&self, process: BusinessProcess, rng: &mut dyn Rng) -> String;

    /// Get a random bank name from the flat pool. (v3.2.0+)
    ///
    /// Default impl returns `None` — implementors with a bank-name pool
    /// (like `DefaultTemplateProvider`) override this. `None` means
    /// "caller should use its own fallback" so existing
    /// `BANK_NAMES`-based callers keep working until the rewire lands.
    fn get_bank_name(&self, _rng: &mut dyn Rng) -> Option<String> {
        None
    }

    /// Get a (title, account) pair for an audit finding of the given type.
    /// (v3.2.0+)
    ///
    /// `finding_type_key` is a lowercase-snake-case canonical name
    /// (e.g. "material_weakness", "control_deficiency"). Default impl
    /// returns `None` so the caller falls back to its inline tables.
    fn get_finding_title(
        &self,
        _finding_type_key: &str,
        _rng: &mut dyn Rng,
    ) -> Option<(String, String)> {
        None
    }

    /// Get a narrative template string for an audit finding section.
    /// (v3.2.0+)
    ///
    /// `section` must be one of: "condition", "criteria", "cause",
    /// "effect", "recommendation". Returns `None` to trigger caller
    /// fallback. Templates may contain `{placeholder}` tokens
    /// (e.g. `{account}`, `{amount}`) that the caller substitutes.
    fn get_finding_narrative(
        &self,
        _finding_type_key: &str,
        _section: &str,
        _rng: &mut dyn Rng,
    ) -> Option<String> {
        None
    }

    /// Get a display name for a department by code. (v3.2.0+)
    ///
    /// `department_code` is one of: "finance", "procurement", "sales",
    /// "warehouse", "it". Returns `None` to trigger caller fallback.
    fn get_department_name(&self, _department_code: &str, _rng: &mut dyn Rng) -> Option<String> {
        None
    }
}

/// Default template provider using embedded templates with optional file overrides.
pub struct DefaultTemplateProvider {
    /// Loaded template data (file-based)
    template_data: Option<TemplateData>,
    /// Merge strategy for combining embedded and file templates
    merge_strategy: MergeStrategy,
}

/// Bundled default YAML — v4.1.4+ proof-of-concept for the
/// YAML-as-source-of-truth migration path. The file at
/// `crates/datasynth-core/templates/defaults.yaml` is included at
/// compile time and made available via
/// [`DefaultTemplateProvider::bundled`].
pub const BUNDLED_DEFAULTS_YAML: &str = include_str!("../../templates/defaults.yaml");

impl DefaultTemplateProvider {
    /// Create a new provider with embedded templates only.
    pub fn new() -> Self {
        Self {
            template_data: None,
            merge_strategy: MergeStrategy::Extend,
        }
    }

    /// v4.1.4+ — create a provider backed by the bundled `defaults.yaml`
    /// (included at compile time) *extended on top of* the embedded
    /// arrays.
    ///
    /// This is the first step toward full YAML-as-source-of-truth
    /// for the default name pools. The bundled YAML supplements (not
    /// replaces) the hardcoded `const` arrays, so byte-identity under
    /// the same seed is preserved when callers use
    /// [`DefaultTemplateProvider::new`].
    ///
    /// If the bundled YAML is malformed at build time, `include_str!`
    /// fails the build; at runtime this fn only fails if the YAML
    /// structure doesn't match the `TemplateData` shape, which a
    /// build-time lint would catch.
    pub fn bundled() -> Result<Self, super::loader::TemplateError> {
        let data = TemplateLoader::load_from_yaml_str(BUNDLED_DEFAULTS_YAML)?;
        Ok(Self::with_templates(data, MergeStrategy::Extend))
    }

    /// Create a provider with file-based templates.
    pub fn with_templates(template_data: TemplateData, strategy: MergeStrategy) -> Self {
        Self {
            template_data: Some(template_data),
            merge_strategy: strategy,
        }
    }

    /// Load templates from a file path.
    pub fn from_file(path: &std::path::Path) -> Result<Self, super::loader::TemplateError> {
        let data = TemplateLoader::load_from_file(path)?;
        Ok(Self::with_templates(data, MergeStrategy::Extend))
    }

    /// Load templates from a directory.
    pub fn from_directory(path: &std::path::Path) -> Result<Self, super::loader::TemplateError> {
        let data = TemplateLoader::load_from_directory(path)?;
        Ok(Self::with_templates(data, MergeStrategy::Extend))
    }

    /// Set the merge strategy.
    pub fn with_merge_strategy(mut self, strategy: MergeStrategy) -> Self {
        self.merge_strategy = strategy;
        self
    }

    /// Get embedded German first names (sample).
    fn embedded_german_first_names_male() -> Vec<&'static str> {
        vec![
            "Hans", "Klaus", "Wolfgang", "Dieter", "Michael", "Stefan", "Thomas", "Andreas",
            "Peter", "Jürgen", "Matthias", "Frank", "Martin", "Bernd",
        ]
    }

    fn embedded_german_first_names_female() -> Vec<&'static str> {
        vec![
            "Anna",
            "Maria",
            "Elisabeth",
            "Ursula",
            "Monika",
            "Petra",
            "Karin",
            "Sabine",
            "Andrea",
            "Christine",
            "Gabriele",
            "Heike",
            "Birgit",
        ]
    }

    fn embedded_german_last_names() -> Vec<&'static str> {
        vec![
            "Müller",
            "Schmidt",
            "Schneider",
            "Fischer",
            "Weber",
            "Meyer",
            "Wagner",
            "Becker",
            "Schulz",
            "Hoffmann",
            "Schäfer",
            "Koch",
            "Bauer",
            "Richter",
        ]
    }

    fn embedded_us_first_names_male() -> Vec<&'static str> {
        vec![
            "James",
            "John",
            "Robert",
            "Michael",
            "William",
            "David",
            "Richard",
            "Joseph",
            "Thomas",
            "Charles",
            "Christopher",
            "Daniel",
            "Matthew",
        ]
    }

    fn embedded_us_first_names_female() -> Vec<&'static str> {
        vec![
            "Mary",
            "Patricia",
            "Jennifer",
            "Linda",
            "Barbara",
            "Elizabeth",
            "Susan",
            "Jessica",
            "Sarah",
            "Karen",
            "Lisa",
            "Nancy",
            "Betty",
            "Margaret",
        ]
    }

    fn embedded_us_last_names() -> Vec<&'static str> {
        vec![
            "Smith",
            "Johnson",
            "Williams",
            "Brown",
            "Jones",
            "Garcia",
            "Miller",
            "Davis",
            "Rodriguez",
            "Martinez",
            "Hernandez",
            "Lopez",
            "Gonzalez",
        ]
    }

    fn embedded_vendor_names_manufacturing() -> Vec<&'static str> {
        vec![
            "Precision Parts Inc.",
            "Industrial Components LLC",
            "Advanced Materials Corp.",
            "Steel Solutions GmbH",
            "Quality Fasteners Ltd.",
            "Machining Excellence Inc.",
        ]
    }

    fn embedded_vendor_names_services() -> Vec<&'static str> {
        vec![
            "Consulting Partners LLP",
            "Technical Services Inc.",
            "Professional Solutions LLC",
            "Business Advisory Group",
            "Strategic Consulting Co.",
            "Expert Services Ltd.",
        ]
    }

    fn embedded_customer_names_automotive() -> Vec<&'static str> {
        vec![
            "AutoWerke Industries",
            "Vehicle Tech Solutions",
            "Motor Parts Direct",
            "Automotive Excellence Corp.",
            "Drive Systems Inc.",
            "Engine Components Ltd.",
        ]
    }

    fn embedded_customer_names_retail() -> Vec<&'static str> {
        vec![
            "Retail Solutions Corp.",
            "Consumer Goods Direct",
            "Shop Smart Inc.",
            "Merchandise Holdings LLC",
            "Retail Distribution Co.",
            "Store Systems Ltd.",
        ]
    }

    fn culture_to_key(culture: NameCulture) -> &'static str {
        match culture {
            NameCulture::WesternUs => "us",
            NameCulture::German => "german",
            NameCulture::Hispanic => "hispanic",
            NameCulture::French => "french",
            NameCulture::Chinese => "chinese",
            NameCulture::Japanese => "japanese",
            NameCulture::Indian => "indian",
        }
    }

    fn process_to_key(process: BusinessProcess) -> &'static str {
        match process {
            BusinessProcess::P2P => "p2p",
            BusinessProcess::O2C => "o2c",
            BusinessProcess::H2R => "h2r",
            BusinessProcess::R2R => "r2r",
            _ => "other",
        }
    }
}

impl Default for DefaultTemplateProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateProvider for DefaultTemplateProvider {
    fn get_person_first_name(
        &self,
        culture: NameCulture,
        is_male: bool,
        rng: &mut dyn Rng,
    ) -> String {
        let key = Self::culture_to_key(culture);

        // Try file templates first
        if let Some(ref data) = self.template_data {
            if let Some(culture_names) = data.person_names.cultures.get(key) {
                let names = if is_male {
                    &culture_names.male_first_names
                } else {
                    &culture_names.female_first_names
                };
                if !names.is_empty() {
                    if let Some(name) = names.choose(rng) {
                        return name.clone();
                    }
                }
            }
        }

        // Fall back to embedded templates
        let embedded = match culture {
            NameCulture::German => {
                if is_male {
                    Self::embedded_german_first_names_male()
                } else {
                    Self::embedded_german_first_names_female()
                }
            }
            _ => {
                if is_male {
                    Self::embedded_us_first_names_male()
                } else {
                    Self::embedded_us_first_names_female()
                }
            }
        };

        embedded.choose(rng).unwrap_or(&"Unknown").to_string()
    }

    fn get_person_last_name(&self, culture: NameCulture, rng: &mut dyn Rng) -> String {
        let key = Self::culture_to_key(culture);

        // Try file templates first
        if let Some(ref data) = self.template_data {
            if let Some(culture_names) = data.person_names.cultures.get(key) {
                if !culture_names.last_names.is_empty() {
                    if let Some(name) = culture_names.last_names.choose(rng) {
                        return name.clone();
                    }
                }
            }
        }

        // Fall back to embedded templates
        let embedded = match culture {
            NameCulture::German => Self::embedded_german_last_names(),
            _ => Self::embedded_us_last_names(),
        };

        embedded.choose(rng).unwrap_or(&"Unknown").to_string()
    }

    fn get_vendor_name(&self, category: &str, rng: &mut dyn Rng) -> String {
        // Try file templates first
        if let Some(ref data) = self.template_data {
            if let Some(names) = data.vendor_names.categories.get(category) {
                if !names.is_empty() {
                    if let Some(name) = names.choose(rng) {
                        return name.clone();
                    }
                }
            }
        }

        // Fall back to embedded templates
        let embedded = match category {
            "manufacturing" => Self::embedded_vendor_names_manufacturing(),
            "services" => Self::embedded_vendor_names_services(),
            _ => {
                tracing::debug!(
                    "Unknown vendor name category '{}', falling back to manufacturing",
                    category
                );
                Self::embedded_vendor_names_manufacturing()
            }
        };

        embedded
            .choose(rng)
            .unwrap_or(&"Unknown Vendor")
            .to_string()
    }

    fn get_customer_name(&self, industry: &str, rng: &mut dyn Rng) -> String {
        // Try file templates first
        if let Some(ref data) = self.template_data {
            if let Some(names) = data.customer_names.industries.get(industry) {
                if !names.is_empty() {
                    if let Some(name) = names.choose(rng) {
                        return name.clone();
                    }
                }
            }
        }

        // Fall back to embedded templates
        let embedded = match industry {
            "automotive" => Self::embedded_customer_names_automotive(),
            "retail" => Self::embedded_customer_names_retail(),
            _ => {
                tracing::debug!(
                    "Unknown customer name industry '{}', falling back to retail",
                    industry
                );
                Self::embedded_customer_names_retail()
            }
        };

        embedded
            .choose(rng)
            .unwrap_or(&"Unknown Customer")
            .to_string()
    }

    fn get_material_description(&self, material_type: &str, rng: &mut dyn Rng) -> String {
        // Try file templates first
        if let Some(ref data) = self.template_data {
            if let Some(descs) = data.material_descriptions.by_type.get(material_type) {
                if !descs.is_empty() {
                    if let Some(desc) = descs.choose(rng) {
                        return desc.clone();
                    }
                }
            }
        }

        // Fall back to generic
        format!("{material_type} material")
    }

    fn get_asset_description(&self, category: &str, rng: &mut dyn Rng) -> String {
        // Try file templates first
        if let Some(ref data) = self.template_data {
            if let Some(descs) = data.asset_descriptions.by_category.get(category) {
                if !descs.is_empty() {
                    if let Some(desc) = descs.choose(rng) {
                        return desc.clone();
                    }
                }
            }
        }

        // Fall back to generic
        format!("{category} asset")
    }

    fn get_line_text(
        &self,
        process: BusinessProcess,
        account_type: &str,
        rng: &mut dyn Rng,
    ) -> String {
        let key = Self::process_to_key(process);

        // Try file templates first
        if let Some(ref data) = self.template_data {
            let descs_map = match process {
                BusinessProcess::P2P => &data.line_item_descriptions.p2p,
                BusinessProcess::O2C => &data.line_item_descriptions.o2c,
                BusinessProcess::H2R => &data.line_item_descriptions.h2r,
                BusinessProcess::R2R => &data.line_item_descriptions.r2r,
                _ => &data.line_item_descriptions.p2p,
            };

            if let Some(descs) = descs_map.get(account_type) {
                if !descs.is_empty() {
                    if let Some(desc) = descs.choose(rng) {
                        return desc.clone();
                    }
                }
            }
        }

        // Fall back to generic
        format!("{} posting", key.to_uppercase())
    }

    fn get_header_template(&self, process: BusinessProcess, rng: &mut dyn Rng) -> String {
        let key = Self::process_to_key(process);

        // Try file templates first
        if let Some(ref data) = self.template_data {
            if let Some(templates) = data.header_text_templates.by_process.get(key) {
                if !templates.is_empty() {
                    if let Some(template) = templates.choose(rng) {
                        return template.clone();
                    }
                }
            }
        }

        // Fall back to generic
        format!("{} Transaction", key.to_uppercase())
    }

    fn get_bank_name(&self, rng: &mut dyn Rng) -> Option<String> {
        if let Some(ref data) = self.template_data {
            if !data.bank_names.names.is_empty() {
                if let Some(name) = data.bank_names.names.choose(rng) {
                    return Some(name.clone());
                }
            }
        }
        None
    }

    fn get_finding_title(
        &self,
        finding_type_key: &str,
        rng: &mut dyn Rng,
    ) -> Option<(String, String)> {
        if let Some(ref data) = self.template_data {
            if let Some(entries) = data.finding_titles.by_type.get(finding_type_key) {
                if !entries.is_empty() {
                    if let Some(entry) = entries.choose(rng) {
                        return Some((entry.title.clone(), entry.account.clone()));
                    }
                }
            }
        }
        None
    }

    fn get_finding_narrative(
        &self,
        finding_type_key: &str,
        section: &str,
        rng: &mut dyn Rng,
    ) -> Option<String> {
        if let Some(ref data) = self.template_data {
            if let Some(sections) = data.finding_narratives.by_type.get(finding_type_key) {
                if let Some(templates) = sections.get(section) {
                    if !templates.is_empty() {
                        if let Some(tpl) = templates.choose(rng) {
                            return Some(tpl.clone());
                        }
                    }
                }
            }
        }
        None
    }

    fn get_department_name(&self, department_code: &str, _rng: &mut dyn Rng) -> Option<String> {
        if let Some(ref data) = self.template_data {
            if let Some(name) = data.department_names.by_code.get(department_code) {
                if !name.is_empty() {
                    return Some(name.clone());
                }
            }
        }
        None
    }
}

/// A thread-safe wrapper around a template provider.
pub type SharedTemplateProvider = Arc<dyn TemplateProvider>;

/// Create a default shared template provider.
pub fn default_provider() -> SharedTemplateProvider {
    Arc::new(DefaultTemplateProvider::new())
}

/// Create a shared template provider from a file.
pub fn provider_from_file(
    path: &std::path::Path,
) -> Result<SharedTemplateProvider, super::loader::TemplateError> {
    Ok(Arc::new(DefaultTemplateProvider::from_file(path)?))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_default_provider() {
        let provider = DefaultTemplateProvider::new();
        let mut rng = ChaCha8Rng::seed_from_u64(12345);

        let name = provider.get_person_first_name(NameCulture::German, true, &mut rng);
        assert!(!name.is_empty());

        let last_name = provider.get_person_last_name(NameCulture::German, &mut rng);
        assert!(!last_name.is_empty());
    }

    #[test]
    fn bundled_defaults_loads() {
        // v4.1.4+ — the bundled YAML must parse and produce a provider
        // that supplies names without panicking.
        let provider = DefaultTemplateProvider::bundled().expect("bundled YAML parses");
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let vendor = provider.get_vendor_name("office_supplies", &mut rng);
        assert!(!vendor.is_empty());
        let customer = provider.get_customer_name("retail", &mut rng);
        assert!(!customer.is_empty());
    }

    #[test]
    fn bundled_matches_embedded_for_person_names() {
        // v4.1.7 byte-identity regression: the bundled YAML mirrors the
        // embedded arrays (same entries, same order). Therefore
        // `bundled()` and `new()` must produce the same sequence of
        // names for the same seed.
        for culture in [NameCulture::German, NameCulture::WesternUs] {
            for is_male in [true, false] {
                let p_embedded = DefaultTemplateProvider::new();
                let p_bundled = DefaultTemplateProvider::bundled().unwrap();
                let mut rng_e = ChaCha8Rng::seed_from_u64(12345);
                let mut rng_b = ChaCha8Rng::seed_from_u64(12345);
                for _ in 0..500 {
                    let ne = p_embedded.get_person_first_name(culture, is_male, &mut rng_e);
                    let nb = p_bundled.get_person_first_name(culture, is_male, &mut rng_b);
                    assert_eq!(
                        ne, nb,
                        "first name mismatch for culture={culture:?} male={is_male}"
                    );
                }
            }
            let p_embedded = DefaultTemplateProvider::new();
            let p_bundled = DefaultTemplateProvider::bundled().unwrap();
            let mut rng_e = ChaCha8Rng::seed_from_u64(54321);
            let mut rng_b = ChaCha8Rng::seed_from_u64(54321);
            for _ in 0..500 {
                let ne = p_embedded.get_person_last_name(culture, &mut rng_e);
                let nb = p_bundled.get_person_last_name(culture, &mut rng_b);
                assert_eq!(ne, nb, "last name mismatch for culture={culture:?}");
            }
        }
    }

    #[test]
    fn bundled_matches_embedded_for_vendor_customer_names() {
        // Categories mirrored in YAML: manufacturing, services (vendors);
        // automotive, retail (customers).
        for category in ["manufacturing", "services"] {
            let p_embedded = DefaultTemplateProvider::new();
            let p_bundled = DefaultTemplateProvider::bundled().unwrap();
            let mut rng_e = ChaCha8Rng::seed_from_u64(99);
            let mut rng_b = ChaCha8Rng::seed_from_u64(99);
            for _ in 0..200 {
                let ne = p_embedded.get_vendor_name(category, &mut rng_e);
                let nb = p_bundled.get_vendor_name(category, &mut rng_b);
                assert_eq!(ne, nb, "vendor mismatch for category={category}");
            }
        }
        for industry in ["automotive", "retail"] {
            let p_embedded = DefaultTemplateProvider::new();
            let p_bundled = DefaultTemplateProvider::bundled().unwrap();
            let mut rng_e = ChaCha8Rng::seed_from_u64(77);
            let mut rng_b = ChaCha8Rng::seed_from_u64(77);
            for _ in 0..200 {
                let ne = p_embedded.get_customer_name(industry, &mut rng_e);
                let nb = p_bundled.get_customer_name(industry, &mut rng_b);
                assert_eq!(ne, nb, "customer mismatch for industry={industry}");
            }
        }
    }

    #[test]
    fn bundled_defaults_include_embedded_mirrored_entries() {
        // v4.1.7: YAML now mirrors the embedded arrays. Spot-check
        // that the loader isn't silently swallowing the file by
        // confirming drawn names match the known pool contents.
        let provider = DefaultTemplateProvider::bundled().expect("bundled YAML parses");
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let expected = [
            "Retail Solutions Corp.",
            "Consumer Goods Direct",
            "Shop Smart Inc.",
            "Merchandise Holdings LLC",
            "Retail Distribution Co.",
            "Store Systems Ltd.",
        ];
        let mut saw_expected = false;
        for _ in 0..500 {
            let name = provider.get_customer_name("retail", &mut rng);
            if expected.contains(&name.as_str()) {
                saw_expected = true;
                break;
            }
        }
        assert!(
            saw_expected,
            "bundled retail customer names (mirrored from embedded) should appear in the draw stream"
        );
    }

    #[test]
    fn test_vendor_names() {
        let provider = DefaultTemplateProvider::new();
        let mut rng = ChaCha8Rng::seed_from_u64(12345);

        let name = provider.get_vendor_name("manufacturing", &mut rng);
        assert!(!name.is_empty());
        assert!(!name.contains("Unknown"));
    }

    #[test]
    fn test_shared_provider() {
        let provider = default_provider();
        let mut rng = ChaCha8Rng::seed_from_u64(12345);

        let name = provider.get_customer_name("retail", &mut rng);
        assert!(!name.is_empty());
    }
}

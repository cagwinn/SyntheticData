//! Template loader for external template files.
//!
//! This module provides functionality to load template data from YAML/JSON files,
//! supporting regional and sector-specific customization.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Error type for template loading operations.
#[derive(Debug, Clone)]
pub struct TemplateError {
    pub message: String,
    pub path: Option<String>,
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref path) = self.path {
            write!(f, "{}: {}", path, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for TemplateError {}

impl TemplateError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            path: None,
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

/// Metadata about a template file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateMetadata {
    /// Template name
    pub name: String,
    /// Version string
    #[serde(default = "default_version")]
    pub version: String,
    /// Region/locale (e.g., "de", "us", "gb")
    pub region: Option<String>,
    /// Industry sector (e.g., "manufacturing", "retail")
    pub sector: Option<String>,
    /// Template author
    pub author: Option<String>,
    /// Description
    pub description: Option<String>,
}

fn default_version() -> String {
    "1.0".to_string()
}

impl Default for TemplateMetadata {
    fn default() -> Self {
        Self {
            name: "Default Templates".to_string(),
            version: default_version(),
            region: None,
            sector: None,
            author: None,
            description: None,
        }
    }
}

/// Person name templates by culture.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersonNameTemplates {
    /// Names organized by culture
    #[serde(default)]
    pub cultures: HashMap<String, CultureNames>,
}

/// Names for a specific culture.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CultureNames {
    /// Male first names
    #[serde(default)]
    pub male_first_names: Vec<String>,
    /// Female first names
    #[serde(default)]
    pub female_first_names: Vec<String>,
    /// Last names / family names
    #[serde(default)]
    pub last_names: Vec<String>,
}

/// Vendor name templates by category.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VendorNameTemplates {
    /// Vendor names by category (e.g., "manufacturing", "services")
    #[serde(default)]
    pub categories: HashMap<String, Vec<String>>,
}

/// Customer name templates by industry.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomerNameTemplates {
    /// Customer names by industry (e.g., "automotive", "retail")
    #[serde(default)]
    pub industries: HashMap<String, Vec<String>>,
}

/// Material description templates.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MaterialDescriptionTemplates {
    /// Descriptions by material type
    #[serde(default)]
    pub by_type: HashMap<String, Vec<String>>,
}

/// Asset description templates.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AssetDescriptionTemplates {
    /// Descriptions by asset category
    #[serde(default)]
    pub by_category: HashMap<String, Vec<String>>,
}

/// Line item description templates by business process.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LineItemDescriptionTemplates {
    /// P2P line descriptions
    #[serde(default)]
    pub p2p: HashMap<String, Vec<String>>,
    /// O2C line descriptions
    #[serde(default)]
    pub o2c: HashMap<String, Vec<String>>,
    /// H2R line descriptions
    #[serde(default)]
    pub h2r: HashMap<String, Vec<String>>,
    /// R2R line descriptions
    #[serde(default)]
    pub r2r: HashMap<String, Vec<String>>,
}

/// Header text templates by business process.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HeaderTextTemplates {
    /// Templates organized by business process
    #[serde(default)]
    pub by_process: HashMap<String, Vec<String>>,
}

/// Flat pool of bank names (used for vendor-bank assignment and
/// banking-customer name generation). Unstructured because the current
/// generator picks uniformly without industry/region keys — see
/// `vendor_generator.rs::BANK_NAMES`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BankNameTemplates {
    /// Bank names; picked uniformly at random.
    #[serde(default)]
    pub names: Vec<String>,
}

/// Audit finding title templates keyed by finding type.
///
/// Each entry is a (title, account-context) pair so the finding generator
/// can pick a coherent title + account binding. Replaces the inline
/// `const TITLES: &[(title, account)]` match arms in
/// `audit/finding_generator.rs::generate_finding_title`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FindingTitleTemplates {
    /// Titles organised by finding-type key (e.g. "material_weakness",
    /// "significant_deficiency", "control_deficiency", "material_misstatement",
    /// "immaterial_misstatement", "compliance_exception", "it_deficiency",
    /// "other_matter", "process_improvement").
    #[serde(default)]
    pub by_type: HashMap<String, Vec<FindingTitleEntry>>,
}

/// A single finding-title entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingTitleEntry {
    /// The finding title shown in workpapers.
    pub title: String,
    /// Account/area affected — paired with the title so the generator
    /// doesn't accidentally tag a journal-entry finding to an unrelated
    /// account like "Property, Plant & Equipment".
    pub account: String,
}

/// Audit finding narrative templates keyed by finding type and section.
///
/// Each narrative has five sections (condition / criteria / cause /
/// effect / recommendation). Templates may contain `{placeholder}`
/// tokens that the generator substitutes at runtime (e.g. `{account}`,
/// `{amount}`, `{period}`). Replaces the inline format!() macros in
/// `audit/finding_generator.rs::generate_ccce` and
/// `generate_recommendation`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FindingNarrativeTemplates {
    /// Nested: by_type → by_section → list of candidate templates.
    /// Section keys: "condition", "criteria", "cause", "effect", "recommendation".
    #[serde(default)]
    pub by_type: HashMap<String, HashMap<String, Vec<String>>>,
}

/// Department name templates keyed by department code.
///
/// Replaces the hardcoded strings in
/// `master_data/employee_generator.rs::DepartmentDefinition::*`.
/// Department codes: "finance", "procurement", "sales", "warehouse",
/// "it".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DepartmentNameTemplates {
    /// Display name per department code.
    #[serde(default)]
    pub by_code: HashMap<String, String>,
}

/// Complete template data structure loaded from files.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TemplateData {
    /// Metadata about the template
    #[serde(default)]
    pub metadata: TemplateMetadata,
    /// Person name templates
    #[serde(default)]
    pub person_names: PersonNameTemplates,
    /// Vendor name templates
    #[serde(default)]
    pub vendor_names: VendorNameTemplates,
    /// Customer name templates
    #[serde(default)]
    pub customer_names: CustomerNameTemplates,
    /// Material description templates
    #[serde(default)]
    pub material_descriptions: MaterialDescriptionTemplates,
    /// Asset description templates
    #[serde(default)]
    pub asset_descriptions: AssetDescriptionTemplates,
    /// Line item description templates
    #[serde(default)]
    pub line_item_descriptions: LineItemDescriptionTemplates,
    /// Header text templates
    #[serde(default)]
    pub header_text_templates: HeaderTextTemplates,
    /// Bank name pool (v3.2.0+)
    #[serde(default)]
    pub bank_names: BankNameTemplates,
    /// Audit finding titles by finding-type (v3.2.0+)
    #[serde(default)]
    pub finding_titles: FindingTitleTemplates,
    /// Audit finding narratives by finding-type and section (v3.2.0+)
    #[serde(default)]
    pub finding_narratives: FindingNarrativeTemplates,
    /// Department display names by code (v3.2.0+)
    #[serde(default)]
    pub department_names: DepartmentNameTemplates,
}

/// Strategy for merging template data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    /// Replace embedded templates entirely
    Replace,
    /// Extend embedded templates with file data
    #[default]
    Extend,
    /// Merge, preferring file data for conflicts
    MergePreferFile,
}

/// Template loader for reading and validating template files.
pub struct TemplateLoader;

impl TemplateLoader {
    /// Load template data from a YAML file.
    pub fn load_from_yaml(path: &Path) -> Result<TemplateData, TemplateError> {
        let contents = std::fs::read_to_string(path).map_err(|e| {
            TemplateError::new(format!("Failed to read file: {e}"))
                .with_path(path.display().to_string())
        })?;

        serde_yaml::from_str(&contents).map_err(|e| {
            TemplateError::new(format!("Failed to parse YAML: {e}"))
                .with_path(path.display().to_string())
        })
    }

    /// Load template data from an in-memory YAML string.  v4.0.0+.
    ///
    /// Intended for callers that embed template YAML at compile time
    /// via `include_str!("…")`, which is the primary migration step
    /// toward v4.1's planned YAML-as-source-of-truth for the default
    /// name pools.
    pub fn load_from_yaml_str(yaml: &str) -> Result<TemplateData, TemplateError> {
        serde_yaml::from_str(yaml)
            .map_err(|e| TemplateError::new(format!("Failed to parse YAML: {e}")))
    }

    /// Load template data from a JSON file.
    pub fn load_from_json(path: &Path) -> Result<TemplateData, TemplateError> {
        let contents = std::fs::read_to_string(path).map_err(|e| {
            TemplateError::new(format!("Failed to read file: {e}"))
                .with_path(path.display().to_string())
        })?;

        serde_json::from_str(&contents).map_err(|e| {
            TemplateError::new(format!("Failed to parse JSON: {e}"))
                .with_path(path.display().to_string())
        })
    }

    /// Load template data from a file (auto-detect format by extension).
    pub fn load_from_file(path: &Path) -> Result<TemplateData, TemplateError> {
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        match extension.to_lowercase().as_str() {
            "yaml" | "yml" => Self::load_from_yaml(path),
            "json" => Self::load_from_json(path),
            _ => Err(TemplateError::new(format!(
                "Unsupported file extension: {extension}. Use .yaml, .yml, or .json"
            ))
            .with_path(path.display().to_string())),
        }
    }

    /// Save template data to a YAML file. v3.5.0+ — used by
    /// `datasynth-data templates enrich` to persist LLM-enriched pools.
    ///
    /// Creates the parent directory if it doesn't exist.
    pub fn save_to_yaml(data: &TemplateData, path: &Path) -> Result<(), TemplateError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    TemplateError::new(format!("Failed to create parent directory: {e}"))
                        .with_path(parent.display().to_string())
                })?;
            }
        }

        let yaml = serde_yaml::to_string(data).map_err(|e| {
            TemplateError::new(format!("Failed to serialize YAML: {e}"))
                .with_path(path.display().to_string())
        })?;

        std::fs::write(path, yaml).map_err(|e| {
            TemplateError::new(format!("Failed to write file: {e}"))
                .with_path(path.display().to_string())
        })
    }

    /// Save template data to a file (auto-detect format by extension).
    /// v3.5.0+.
    pub fn save_to_file(data: &TemplateData, path: &Path) -> Result<(), TemplateError> {
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("yaml");
        match extension.to_lowercase().as_str() {
            "yaml" | "yml" => Self::save_to_yaml(data, path),
            "json" => {
                let json = serde_json::to_string_pretty(data).map_err(|e| {
                    TemplateError::new(format!("Failed to serialize JSON: {e}"))
                        .with_path(path.display().to_string())
                })?;
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() && !parent.exists() {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            TemplateError::new(format!("Failed to create parent directory: {e}"))
                                .with_path(parent.display().to_string())
                        })?;
                    }
                }
                std::fs::write(path, json).map_err(|e| {
                    TemplateError::new(format!("Failed to write file: {e}"))
                        .with_path(path.display().to_string())
                })
            }
            _ => Err(TemplateError::new(format!(
                "Unsupported file extension: {extension}. Use .yaml, .yml, or .json"
            ))
            .with_path(path.display().to_string())),
        }
    }

    /// Load all template files from a directory.
    pub fn load_from_directory(dir: &Path) -> Result<TemplateData, TemplateError> {
        if !dir.is_dir() {
            return Err(
                TemplateError::new("Path is not a directory").with_path(dir.display().to_string())
            );
        }

        let mut merged = TemplateData::default();

        let entries = std::fs::read_dir(dir).map_err(|e| {
            TemplateError::new(format!("Failed to read directory: {e}"))
                .with_path(dir.display().to_string())
        })?;

        for entry in entries {
            let entry =
                entry.map_err(|e| TemplateError::new(format!("Failed to read entry: {e}")))?;
            let path = entry.path();

            if path.is_file() {
                let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

                if matches!(extension.to_lowercase().as_str(), "yaml" | "yml" | "json") {
                    match Self::load_from_file(&path) {
                        Ok(data) => {
                            merged = Self::merge(merged, data, MergeStrategy::Extend);
                        }
                        Err(e) => {
                            // Log but continue with other files
                            eprintln!(
                                "Warning: Failed to load template file {}: {}",
                                path.display(),
                                e
                            );
                        }
                    }
                }
            }
        }

        Ok(merged)
    }

    /// Validate template data.
    pub fn validate(data: &TemplateData) -> Vec<String> {
        let mut errors = Vec::new();

        // Check metadata
        if data.metadata.name.is_empty() {
            errors.push("Metadata: name is required".to_string());
        }

        // Check for empty template sections (warnings, not errors)
        if data.person_names.cultures.is_empty() {
            // This is OK - will fall back to embedded templates
        }

        // Validate culture names have required fields
        for (culture, names) in &data.person_names.cultures {
            if names.male_first_names.is_empty() && names.female_first_names.is_empty() {
                errors.push(format!("Culture '{culture}': no first names defined"));
            }
            if names.last_names.is_empty() {
                errors.push(format!("Culture '{culture}': no last names defined"));
            }
        }

        errors
    }

    /// Merge two template data sets according to the specified strategy.
    pub fn merge(
        base: TemplateData,
        overlay: TemplateData,
        strategy: MergeStrategy,
    ) -> TemplateData {
        match strategy {
            MergeStrategy::Replace => overlay,
            MergeStrategy::Extend => Self::merge_extend(base, overlay),
            MergeStrategy::MergePreferFile => Self::merge_prefer_overlay(base, overlay),
        }
    }

    fn merge_extend(mut base: TemplateData, overlay: TemplateData) -> TemplateData {
        // Extend cultures
        for (culture, names) in overlay.person_names.cultures {
            base.person_names
                .cultures
                .entry(culture)
                .or_default()
                .extend_from(&names);
        }

        // Extend vendor categories
        for (category, names) in overlay.vendor_names.categories {
            base.vendor_names
                .categories
                .entry(category)
                .or_default()
                .extend(names);
        }

        // Extend customer industries
        for (industry, names) in overlay.customer_names.industries {
            base.customer_names
                .industries
                .entry(industry)
                .or_default()
                .extend(names);
        }

        // Extend material descriptions
        for (mat_type, descs) in overlay.material_descriptions.by_type {
            base.material_descriptions
                .by_type
                .entry(mat_type)
                .or_default()
                .extend(descs);
        }

        // Extend asset descriptions
        for (category, descs) in overlay.asset_descriptions.by_category {
            base.asset_descriptions
                .by_category
                .entry(category)
                .or_default()
                .extend(descs);
        }

        // Extend line item descriptions
        for (account_type, descs) in overlay.line_item_descriptions.p2p {
            base.line_item_descriptions
                .p2p
                .entry(account_type)
                .or_default()
                .extend(descs);
        }
        for (account_type, descs) in overlay.line_item_descriptions.o2c {
            base.line_item_descriptions
                .o2c
                .entry(account_type)
                .or_default()
                .extend(descs);
        }

        // Extend header templates
        for (process, templates) in overlay.header_text_templates.by_process {
            base.header_text_templates
                .by_process
                .entry(process)
                .or_default()
                .extend(templates);
        }

        // Extend h2r / r2r line descriptions (previously missing from merge)
        for (account_type, descs) in overlay.line_item_descriptions.h2r {
            base.line_item_descriptions
                .h2r
                .entry(account_type)
                .or_default()
                .extend(descs);
        }
        for (account_type, descs) in overlay.line_item_descriptions.r2r {
            base.line_item_descriptions
                .r2r
                .entry(account_type)
                .or_default()
                .extend(descs);
        }

        // Extend bank names (flat pool).
        base.bank_names.names.extend(overlay.bank_names.names);

        // Extend finding titles by type.
        for (ft, entries) in overlay.finding_titles.by_type {
            base.finding_titles
                .by_type
                .entry(ft)
                .or_default()
                .extend(entries);
        }

        // Extend finding narratives by (type, section).
        for (ft, sections) in overlay.finding_narratives.by_type {
            let base_sections = base.finding_narratives.by_type.entry(ft).or_default();
            for (section, templates) in sections {
                base_sections.entry(section).or_default().extend(templates);
            }
        }

        // Department names: keyed by code — overlay wins per code, since
        // a department has one display name (not a pool).
        for (code, name) in overlay.department_names.by_code {
            base.department_names.by_code.insert(code, name);
        }

        base
    }

    fn merge_prefer_overlay(mut base: TemplateData, overlay: TemplateData) -> TemplateData {
        // Use overlay metadata if present
        if !overlay.metadata.name.is_empty() && overlay.metadata.name != "Default Templates" {
            base.metadata = overlay.metadata;
        }

        // For prefer overlay, we replace entire categories if present in overlay
        for (culture, names) in overlay.person_names.cultures {
            base.person_names.cultures.insert(culture, names);
        }

        for (category, names) in overlay.vendor_names.categories {
            if !names.is_empty() {
                base.vendor_names.categories.insert(category, names);
            }
        }

        for (industry, names) in overlay.customer_names.industries {
            if !names.is_empty() {
                base.customer_names.industries.insert(industry, names);
            }
        }

        // Material descriptions — replace per material-type key
        for (mat_type, descs) in overlay.material_descriptions.by_type {
            if !descs.is_empty() {
                base.material_descriptions.by_type.insert(mat_type, descs);
            }
        }

        // Asset descriptions — replace per category key
        for (category, descs) in overlay.asset_descriptions.by_category {
            if !descs.is_empty() {
                base.asset_descriptions.by_category.insert(category, descs);
            }
        }

        // Header templates — replace per process key
        for (process, templates) in overlay.header_text_templates.by_process {
            if !templates.is_empty() {
                base.header_text_templates
                    .by_process
                    .insert(process, templates);
            }
        }

        // Bank names — if overlay has any, replace entirely
        if !overlay.bank_names.names.is_empty() {
            base.bank_names = overlay.bank_names;
        }

        // Finding titles — replace per finding type
        for (ft, entries) in overlay.finding_titles.by_type {
            if !entries.is_empty() {
                base.finding_titles.by_type.insert(ft, entries);
            }
        }

        // Finding narratives — replace per finding type (whole sections map)
        for (ft, sections) in overlay.finding_narratives.by_type {
            if !sections.is_empty() {
                base.finding_narratives.by_type.insert(ft, sections);
            }
        }

        // Department names — replace per code
        for (code, name) in overlay.department_names.by_code {
            base.department_names.by_code.insert(code, name);
        }

        base
    }
}

impl CultureNames {
    fn extend_from(&mut self, other: &CultureNames) {
        self.male_first_names
            .extend(other.male_first_names.iter().cloned());
        self.female_first_names
            .extend(other.female_first_names.iter().cloned());
        self.last_names.extend(other.last_names.iter().cloned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_template_data() {
        let data = TemplateData::default();
        assert_eq!(data.metadata.version, "1.0");
        assert!(data.person_names.cultures.is_empty());
    }

    #[test]
    fn test_validate_empty_culture() {
        let mut data = TemplateData::default();
        data.person_names.cultures.insert(
            "test".to_string(),
            CultureNames {
                male_first_names: vec![],
                female_first_names: vec![],
                last_names: vec![],
            },
        );

        let errors = TemplateLoader::validate(&data);
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_merge_extend() {
        let mut base = TemplateData::default();
        base.vendor_names
            .categories
            .insert("services".to_string(), vec!["Company A".to_string()]);

        let mut overlay = TemplateData::default();
        overlay
            .vendor_names
            .categories
            .insert("services".to_string(), vec!["Company B".to_string()]);

        let merged = TemplateLoader::merge(base, overlay, MergeStrategy::Extend);
        let services = merged.vendor_names.categories.get("services").unwrap();
        assert_eq!(services.len(), 2);
        assert!(services.contains(&"Company A".to_string()));
        assert!(services.contains(&"Company B".to_string()));
    }

    #[test]
    fn test_merge_replace() {
        let mut base = TemplateData::default();
        base.vendor_names
            .categories
            .insert("services".to_string(), vec!["Company A".to_string()]);

        let mut overlay = TemplateData::default();
        overlay
            .vendor_names
            .categories
            .insert("manufacturing".to_string(), vec!["Company B".to_string()]);

        let merged = TemplateLoader::merge(base, overlay, MergeStrategy::Replace);
        assert!(!merged.vendor_names.categories.contains_key("services"));
        assert!(merged.vendor_names.categories.contains_key("manufacturing"));
    }

    #[test]
    fn test_load_example_templates() {
        // This test verifies all example templates can be loaded
        let examples_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples")
            .join("templates");

        if !examples_dir.exists() {
            // Skip if examples directory doesn't exist (e.g., in CI)
            return;
        }

        let template_files = [
            "german_manufacturing.yaml",
            "japanese_technology.yaml",
            "british_financial_services.yaml",
            "brazilian_retail.yaml",
            "indian_healthcare.yaml",
        ];

        for file in &template_files {
            let path = examples_dir.join(file);
            if path.exists() {
                let result = TemplateLoader::load_from_file(&path);
                assert!(
                    result.is_ok(),
                    "Failed to load {}: {:?}",
                    file,
                    result.err()
                );

                let data = result.unwrap();
                assert!(
                    !data.metadata.name.is_empty(),
                    "{}: metadata.name is empty",
                    file
                );
                assert!(
                    data.metadata.region.is_some(),
                    "{}: metadata.region is missing",
                    file
                );
                assert!(
                    data.metadata.sector.is_some(),
                    "{}: metadata.sector is missing",
                    file
                );

                // Validate the template
                let errors = TemplateLoader::validate(&data);
                assert!(
                    errors.is_empty(),
                    "{}: validation errors: {:?}",
                    file,
                    errors
                );
            }
        }
    }

    #[test]
    fn test_load_example_templates_directory() {
        let examples_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples")
            .join("templates");

        if !examples_dir.exists() {
            return;
        }

        let result = TemplateLoader::load_from_directory(&examples_dir);
        assert!(
            result.is_ok(),
            "Failed to load directory: {:?}",
            result.err()
        );

        let merged = result.unwrap();

        // Should have multiple cultures merged
        assert!(
            merged.person_names.cultures.len() >= 4,
            "Expected at least 4 cultures, got {}",
            merged.person_names.cultures.len()
        );

        // Check specific cultures exist
        assert!(
            merged.person_names.cultures.contains_key("german"),
            "Missing german culture"
        );
        assert!(
            merged.person_names.cultures.contains_key("japanese"),
            "Missing japanese culture"
        );
        assert!(
            merged.person_names.cultures.contains_key("british"),
            "Missing british culture"
        );
        assert!(
            merged.person_names.cultures.contains_key("brazilian"),
            "Missing brazilian culture"
        );
        assert!(
            merged.person_names.cultures.contains_key("indian"),
            "Missing indian culture"
        );
    }
}

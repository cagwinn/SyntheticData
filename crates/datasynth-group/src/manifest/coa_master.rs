//! Chart of Accounts master resolution — spec §4.1.
//!
//! Scans the expanded entity list to discover the set of distinct
//! `accounting_framework` values, then loads one [`ChartOfAccounts`] per
//! framework into a [`ChartOfAccountsMaster`].  The master is the canonical
//! CoA registry for the whole group; shard generators look up their entity's
//! framework CoA from it (Chunk 4).

use std::collections::{BTreeMap, HashMap};

use datasynth_core::models::{ChartOfAccounts, CoAComplexity, IndustrySector};
use datasynth_core::pcg_loader;
use datasynth_core::skr_loader;
use datasynth_generators::ChartOfAccountsGenerator;

use crate::errors::{GroupError, GroupResult};
use crate::manifest::expansion::ExpandedEntity;

// ── Framework string normalisation ──────────────────────────────────────────

/// Canonical framework strings accepted by this module.
///
/// | Input string(s)               | Canonical key |
/// |-------------------------------|---------------|
/// | "ifrs", "IFRS"                | "ifrs"        |
/// | "us_gaap", "UsGaap", "usgaap"| "us_gaap"     |
/// | "hgb", "german_gaap"          | "hgb"         |
/// | "pcg", "french_gaap"          | "pcg"         |
/// | "swiss_or"                    | "swiss_or"    |
/// | anything else                 | kept as-is    |
fn canonicalize_framework(raw: &str) -> String {
    match raw.to_lowercase().replace('-', "_").as_str() {
        "ifrs" => "ifrs".to_string(),
        "us_gaap" | "usgaap" | "usgaap2" => "us_gaap".to_string(),
        "hgb" | "german_gaap" | "germangaap" => "hgb".to_string(),
        "pcg" | "french_gaap" | "frenchgaap" => "pcg".to_string(),
        "swiss_or" | "swissor" => "swiss_or".to_string(),
        other => other.to_string(),
    }
}

/// Parse a `serde_yaml::Value` `defaults` block and return the raw
/// `accounting_framework` string (e.g. "ifrs") if present.
fn defaults_accounting_framework(defaults: &serde_yaml::Value) -> Option<String> {
    defaults
        .get("accounting_framework")
        .and_then(|v| v.as_str())
        .map(canonicalize_framework)
}

/// Parse a `serde_yaml::Value` `defaults` block and return the
/// `complexity` string ("small" | "medium" | "large").  Falls back to
/// "medium".
fn defaults_complexity(defaults: &serde_yaml::Value) -> CoAComplexity {
    defaults
        .get("complexity")
        .and_then(|v| v.as_str())
        .map(|s| match s.to_lowercase().as_str() {
            "small" => CoAComplexity::Small,
            "large" => CoAComplexity::Large,
            _ => CoAComplexity::Medium,
        })
        .unwrap_or(CoAComplexity::Medium)
}

/// Parse a `serde_yaml::Value` `defaults` block and return an
/// `IndustrySector`.  Falls back to `Manufacturing`.
fn defaults_industry(defaults: &serde_yaml::Value) -> IndustrySector {
    defaults
        .get("industry")
        .and_then(|v| v.as_str())
        .map(|s| match s.to_lowercase().as_str() {
            "retail" => IndustrySector::Retail,
            "financial_services" | "financialservices" => IndustrySector::FinancialServices,
            "healthcare" => IndustrySector::Healthcare,
            "technology" => IndustrySector::Technology,
            "professional_services" | "professionalservices" => {
                IndustrySector::ProfessionalServices
            }
            "energy" => IndustrySector::Energy,
            "transportation" => IndustrySector::Transportation,
            "real_estate" | "realestate" => IndustrySector::RealEstate,
            "telecommunications" => IndustrySector::Telecommunications,
            _ => IndustrySector::Manufacturing,
        })
        .unwrap_or(IndustrySector::Manufacturing)
}

// ── ChartOfAccountsMaster ────────────────────────────────────────────────────

/// Canonical CoA registry for the whole group.
///
/// One [`ChartOfAccounts`] entry per distinct framework string encountered
/// across the expanded entity list.  Shard generators resolve their entity's
/// framework and look up the pre-built CoA here.
#[derive(Debug, Clone)]
pub struct ChartOfAccountsMaster {
    /// The group's primary / consolidation framework.
    ///
    /// = `defaults.accounting_framework` when set, else the first
    /// framework in `frameworks` (BTree order → lexicographic).
    pub primary_framework: String,

    /// One fully-populated [`ChartOfAccounts`] per distinct framework.
    pub frameworks: BTreeMap<String, ChartOfAccounts>,

    /// Deterministic identifier — blake3 hash (first 8 bytes, lower hex)
    /// of `"{sorted_frameworks}|{complexity}"`.  Stable under entity
    /// reordering; changes when the framework set or complexity changes.
    pub coa_id: String,
}

// ── Public builder ───────────────────────────────────────────────────────────

/// Build the group-level [`ChartOfAccountsMaster`].
///
/// # Parameters
/// - `expanded_entities` — output of [`expand_ownership`](crate::manifest::expansion::expand_ownership)
/// - `defaults` — `GroupConfig.defaults` as a raw `serde_yaml::Value` mapping
/// - `group_id` — group identifier used to name each CoA
///
/// # Errors
/// Returns [`GroupError::Manifest`] if a framework-specific builder (PCG /
/// SKR04) fails to load its embedded JSON.  This should only happen if the
/// binary was built without the resource files.
pub fn build_coa_master(
    expanded_entities: &[ExpandedEntity],
    defaults: &serde_yaml::Value,
    group_id: &str,
) -> GroupResult<ChartOfAccountsMaster> {
    let fallback_framework =
        defaults_accounting_framework(defaults).unwrap_or_else(|| "ifrs".to_string());
    let complexity = defaults_complexity(defaults);
    let industry = defaults_industry(defaults);

    // ── 1. Collect distinct framework strings ────────────────────────────
    let mut framework_country: HashMap<String, Vec<String>> = HashMap::new();

    for entity in expanded_entities {
        let fw = entity
            .accounting_framework
            .as_deref()
            .map(canonicalize_framework)
            .unwrap_or_else(|| fallback_framework.clone());

        framework_country
            .entry(fw)
            .or_default()
            .push(entity.country.clone());
    }

    // Edge case: if no entities, still build one CoA for the fallback.
    if framework_country.is_empty() {
        framework_country.insert(fallback_framework.clone(), vec![]);
    }

    // ── 2. Build one CoA per framework ──────────────────────────────────
    let mut frameworks: BTreeMap<String, ChartOfAccounts> = BTreeMap::new();

    for (fw, countries) in &framework_country {
        // Most-common country for this framework (used as CoA `country`).
        let country = most_common_country(countries).unwrap_or("XX");

        let coa = build_coa_for_framework(fw, group_id, country, industry, complexity)?;
        frameworks.insert(fw.clone(), coa);
    }

    // ── 3. Primary framework ─────────────────────────────────────────────
    let primary_framework = if let Some(d) = defaults_accounting_framework(defaults) {
        d
    } else {
        // First key in BTreeMap = lexicographic minimum.
        frameworks
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| fallback_framework.clone())
    };

    // ── 4. Deterministic coa_id ──────────────────────────────────────────
    let sorted_keys: Vec<&str> = frameworks.keys().map(String::as_str).collect();
    let hash_input = format!("{}|{complexity:?}", sorted_keys.join(","));
    let digest = blake3::hash(hash_input.as_bytes());
    let coa_id = format!("GROUP_CoA_{}", hex::encode(&digest.as_bytes()[..8]));

    Ok(ChartOfAccountsMaster {
        primary_framework,
        frameworks,
        coa_id,
    })
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Return the most frequently occurring country string in `countries`, or
/// `None` if the slice is empty.
fn most_common_country(countries: &[String]) -> Option<&str> {
    if countries.is_empty() {
        return None;
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for c in countries {
        *counts.entry(c.as_str()).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(_, cnt)| cnt)
        .map(|(c, _)| c)
}

/// Build a populated [`ChartOfAccounts`] for `framework`.
///
/// | framework  | builder                                       |
/// |------------|-----------------------------------------------|
/// | "hgb"      | `skr_loader::build_chart_of_accounts_from_skr04` |
/// | "pcg"      | `pcg_loader::build_chart_of_accounts_from_pcg_2024` |
/// | anything   | `ChartOfAccountsGenerator` (US GAAP default)  |
///
/// Swiss OR uses the IFRS/GAAP generator path — there is no separate Swiss
/// chart embedded in `datasynth-core`.
fn build_coa_for_framework(
    framework: &str,
    group_id: &str,
    country: &str,
    industry: IndustrySector,
    complexity: CoAComplexity,
) -> GroupResult<ChartOfAccounts> {
    match framework {
        "hgb" => {
            let mut coa = skr_loader::build_chart_of_accounts_from_skr04(complexity, industry)
                .map_err(|e| GroupError::Manifest(format!("SKR04 load failed: {e}")))?;
            // Override the auto-generated ID/name with group-scoped values.
            coa.coa_id = format!("{group_id}_{framework}_coa");
            coa.name = format!("{group_id} {framework} chart of accounts");
            coa.accounting_framework = Some(framework.to_string());
            Ok(coa)
        }
        "pcg" => {
            let mut coa = pcg_loader::build_chart_of_accounts_from_pcg_2024(complexity, industry)
                .map_err(|e| GroupError::Manifest(format!("PCG load failed: {e}")))?;
            coa.coa_id = format!("{group_id}_{framework}_coa");
            coa.name = format!("{group_id} {framework} chart of accounts");
            coa.accounting_framework = Some(framework.to_string());
            Ok(coa)
        }
        // "ifrs", "us_gaap", "swiss_or", and any unknown framework.
        _ => {
            // Use the ChartOfAccountsGenerator (US-style + canonical seeds).
            // A fixed seed (0) is fine — the CoA structure is not random-dependent.
            let mut gen = ChartOfAccountsGenerator::new(complexity, industry, 0);
            let mut coa = gen.generate();
            coa.coa_id = format!("{group_id}_{framework}_coa");
            coa.name = format!("{group_id} {framework} chart of accounts");
            coa.country = country.to_string();
            coa.accounting_framework = Some(framework.to_string());
            Ok(coa)
        }
    }
}

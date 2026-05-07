//! Industry-specific account-pack definitions.
//!
//! v5.7.0 — adds sector-specific sub-account expansion to the chart of
//! accounts. Real-world ERPs decompose a canonical control account
//! (e.g. `4000` Product Revenue) into many product-line / channel /
//! cost-center sub-accounts (`400010` Steel Products, `400020` Aluminum
//! Components, …). Synthetic data without that decomposition stands out
//! as obviously synthetic — flat consecutive numbering with one record
//! per canonical category.
//!
//! Packs are YAML files embedded via `include_str!` — zero-I/O at
//! runtime, all five industries fit in <30 KB total.
//!
//! ## Sub-account number convention
//!
//! Sub-account numbers are formed by concatenating the parent's 4-digit
//! canonical account number with a 2-digit suffix → 6-digit:
//! `parent="4000" + suffix="10" → "400010"`. Real-world ERPs (especially
//! SAP-FI) routinely mix 4- and 6-digit accounts in this way — the
//! 4-digit codes act as control accounts (parent / GL-summary level)
//! and the 6-digit codes are the detail postings.
//!
//! Suffixes are deliberately not consecutive: gaps simulate retired,
//! migrated, or reserved suffixes that real COAs accumulate over time.
//!
//! ## Picker semantics
//!
//! The `weight` on each sub-account drives the deterministic-by-document
//! picker exposed via [`ChartOfAccounts::pick_subaccount_for_document`]
//! (in `models::chart_of_accounts`). Higher weight = more likely.
//! Hashing on `document_id` keeps every regeneration of the same dataset
//! byte-identical.

use serde::{Deserialize, Serialize};

use crate::models::IndustrySector;

// ──────────────────────────────────────────────────────────────────────
// Embedded pack sources
// ──────────────────────────────────────────────────────────────────────

const PACK_MANUFACTURING: &str = include_str!("industry_packs/manufacturing.yaml");
const PACK_RETAIL: &str = include_str!("industry_packs/retail.yaml");
const PACK_FINANCIAL_SERVICES: &str = include_str!("industry_packs/financial_services.yaml");
const PACK_HEALTHCARE: &str = include_str!("industry_packs/healthcare.yaml");
const PACK_TECHNOLOGY: &str = include_str!("industry_packs/technology.yaml");

// ──────────────────────────────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────────────────────────────

/// One sub-account within a canonical-account expansion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAccountSpec {
    /// 2-character digit suffix appended to the parent's 4-digit
    /// canonical account number. Must be `0`-`99` formatted as 2 digits.
    pub suffix: String,
    /// Descriptive name suffix; full sub-account name is rendered as
    /// `"<parent_name> — <name>"`.
    pub name: String,
    /// Picker weight (relative; higher = more likely to be selected
    /// for a given document under the deterministic-by-document picker).
    pub weight: f64,
}

/// One canonical-account expansion: a parent canonical account number
/// plus the list of sub-accounts that should fan out from it when
/// industry-pack expansion is enabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountExpansion {
    /// 4-digit canonical account number from
    /// [`crate::accounts`] (e.g. `"4000"` for `PRODUCT_REVENUE`).
    pub parent_account: String,
    /// Human name of the parent. Used as the prefix when rendering
    /// sub-account names.
    pub parent_name: String,
    /// Sub-accounts that fan out from the parent.
    pub sub_accounts: Vec<SubAccountSpec>,
}

/// A complete industry pack — parsed from one of the embedded YAML files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndustryPack {
    /// Industry identifier (matches [`IndustrySector`] in snake_case).
    pub industry: String,
    /// Human description.
    pub description: String,
    /// Pack schema version.
    #[serde(default = "default_version")]
    pub version: u32,
    /// All canonical-account expansions defined by this pack.
    pub expansions: Vec<AccountExpansion>,
}

fn default_version() -> u32 {
    1
}

impl IndustryPack {
    /// Parse a pack from a YAML string.
    pub fn parse(yaml: &str) -> Result<Self, String> {
        serde_yaml::from_str(yaml).map_err(|e| format!("industry pack parse error: {e}"))
    }

    /// Look up an expansion for a given canonical parent account.
    pub fn expansion_for(&self, parent_account: &str) -> Option<&AccountExpansion> {
        self.expansions
            .iter()
            .find(|e| e.parent_account == parent_account)
    }
}

/// Return the embedded YAML source for `sector`, if a pack exists.
pub fn raw_pack_for(sector: IndustrySector) -> Option<&'static str> {
    match sector {
        IndustrySector::Manufacturing => Some(PACK_MANUFACTURING),
        IndustrySector::Retail => Some(PACK_RETAIL),
        IndustrySector::FinancialServices => Some(PACK_FINANCIAL_SERVICES),
        IndustrySector::Healthcare => Some(PACK_HEALTHCARE),
        IndustrySector::Technology => Some(PACK_TECHNOLOGY),
        // v5.7.0 MVP ships 5 packs; remaining sectors fall through to
        // None (no expansion) — packs can be added without API change.
        IndustrySector::ProfessionalServices
        | IndustrySector::Energy
        | IndustrySector::Transportation
        | IndustrySector::RealEstate
        | IndustrySector::Telecommunications => None,
    }
}

/// Load and parse the pack for `sector`. Returns `Ok(None)` if no pack
/// is shipped for that sector (no expansion will happen).
pub fn load_pack(sector: IndustrySector) -> Result<Option<IndustryPack>, String> {
    match raw_pack_for(sector) {
        Some(yaml) => IndustryPack::parse(yaml).map(Some),
        None => Ok(None),
    }
}

/// Render the full sub-account number from a parent + suffix.
///
/// `parent` is normalised to 4 digits (zero-padded if shorter, or
/// truncated to its first 4 chars if longer); `suffix` is normalised to
/// 2 digits (zero-padded if shorter); the result is parent + suffix.
pub fn render_sub_account_number(parent: &str, suffix: &str) -> String {
    let parent4: String = parent.chars().take(4).collect();
    let parent4 = format!("{parent4:0>4}");
    let suffix2 = format!("{suffix:0>2}");
    format!("{parent4}{suffix2}")
}

/// Render the full sub-account name from a parent name + sub-account name.
pub fn render_sub_account_name(parent_name: &str, sub_name: &str) -> String {
    format!("{parent_name} — {sub_name}")
}

// ──────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shipped pack must parse, declare a valid industry name,
    /// and contain at least one expansion with at least two sub-accounts.
    #[test]
    fn all_shipped_packs_parse_and_are_well_formed() {
        for sector in [
            IndustrySector::Manufacturing,
            IndustrySector::Retail,
            IndustrySector::FinancialServices,
            IndustrySector::Healthcare,
            IndustrySector::Technology,
        ] {
            let pack = load_pack(sector)
                .unwrap_or_else(|e| panic!("{:?} pack failed to load: {e}", sector))
                .unwrap_or_else(|| panic!("{:?} pack should be shipped", sector));
            assert!(!pack.industry.is_empty(), "{:?}: industry empty", sector);
            assert!(
                !pack.expansions.is_empty(),
                "{:?}: at least one expansion required",
                sector
            );
            for exp in &pack.expansions {
                assert_eq!(
                    exp.parent_account.len(),
                    4,
                    "{:?} parent {:?} should be 4 digits",
                    sector,
                    exp.parent_account
                );
                assert!(
                    exp.parent_account.chars().all(|c| c.is_ascii_digit()),
                    "{:?} parent {:?} should be all digits",
                    sector,
                    exp.parent_account
                );
                assert!(
                    exp.sub_accounts.len() >= 2,
                    "{:?} parent {} should have ≥2 sub-accounts",
                    sector,
                    exp.parent_account
                );
                for sub in &exp.sub_accounts {
                    assert_eq!(
                        sub.suffix.len(),
                        2,
                        "{:?} sub suffix {:?} should be 2 chars",
                        sector,
                        sub.suffix
                    );
                    assert!(
                        sub.suffix.chars().all(|c| c.is_ascii_digit()),
                        "{:?} sub suffix {:?} should be digits",
                        sector,
                        sub.suffix
                    );
                    assert!(
                        sub.weight > 0.0,
                        "{:?} sub {:?} weight must be positive",
                        sector,
                        sub.name
                    );
                    assert!(!sub.name.is_empty(), "{:?} sub name empty", sector);
                }
                // Suffixes must be unique within an expansion.
                let mut suffixes: Vec<&str> =
                    exp.sub_accounts.iter().map(|s| s.suffix.as_str()).collect();
                suffixes.sort();
                let before = suffixes.len();
                suffixes.dedup();
                assert_eq!(
                    before,
                    suffixes.len(),
                    "{:?} parent {} has duplicate suffixes",
                    sector,
                    exp.parent_account
                );
            }
        }
    }

    #[test]
    fn unsupported_sectors_return_none() {
        assert!(matches!(load_pack(IndustrySector::Energy), Ok(None)));
        assert!(matches!(
            load_pack(IndustrySector::ProfessionalServices),
            Ok(None)
        ));
    }

    #[test]
    fn render_sub_account_number_concatenates_padded() {
        assert_eq!(render_sub_account_number("4000", "10"), "400010");
        assert_eq!(render_sub_account_number("4000", "5"), "400005");
        assert_eq!(render_sub_account_number("60", "10"), "006010");
    }

    #[test]
    fn render_sub_account_name_uses_em_dash() {
        assert_eq!(
            render_sub_account_name("Product Revenue", "Steel Products"),
            "Product Revenue — Steel Products"
        );
    }

    #[test]
    fn manufacturing_has_expected_revenue_split() {
        let pack = load_pack(IndustrySector::Manufacturing).unwrap().unwrap();
        let rev = pack
            .expansion_for("4000")
            .expect("manufacturing must expand 4000");
        let names: Vec<&str> = rev.sub_accounts.iter().map(|s| s.name.as_str()).collect();
        assert!(names.iter().any(|n| n.contains("Steel")));
        assert!(names.iter().any(|n| n.contains("Aluminum")));
    }
}

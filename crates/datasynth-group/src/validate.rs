//! Structural validation of a parsed [`GroupConfig`].
//!
//! Call [`validate`] immediately after deserialisation and before any resolution
//! or manifest-building steps. All detected problems are collected into a single
//! `GroupError::Config` so the caller sees every issue at once.

use crate::config::{GroupConfig, IcRelationshipConfig};
use crate::errors::{GroupError, GroupResult};

/// Known named fields on [`crate::config::EntityConfig`] used for Levenshtein
/// typo detection against keys that land in `overrides`.
const ENTITY_CONFIG_FIELDS: &[&str] = &[
    "code",
    "name",
    "country",
    "functional_currency",
    "scoping_profile",
    "consolidation_method",
    "ownership_percent",
    "parent_code",
    "acquisition_date",
    "accounting_framework",
    "industry",
    "rows",
];

/// Maximum Levenshtein distance considered a "likely typo".
const TYPO_DISTANCE_THRESHOLD: usize = 2;

/// Validate the structural consistency of `cfg`.
///
/// Collects **all** errors before returning so the caller sees the complete
/// picture rather than having to fix and re-run for each problem in turn.
///
/// Returns `Ok(())` when no problems are found.
pub fn validate(cfg: &GroupConfig) -> GroupResult<()> {
    let mut errors: Vec<String> = Vec::new();

    let entity_codes: std::collections::BTreeSet<&str> = cfg
        .ownership
        .entities
        .iter()
        .map(|e| e.code.as_str())
        .collect();

    // -----------------------------------------------------------------------
    // Check 1: parent_entity_code must appear in ownership.entities
    // -----------------------------------------------------------------------
    let parent = cfg.ownership.parent_entity_code.as_str();
    if !entity_codes.contains(parent) {
        errors.push(format!(
            "parent_entity_code {parent} is not present in ownership.entities"
        ));
    }

    // -----------------------------------------------------------------------
    // Check 2 + 3 + 4 + 8: per-entity checks
    // -----------------------------------------------------------------------
    for entity in &cfg.ownership.entities {
        let code = entity.code.as_str();

        // Check 2: scoping_profile must exist in scoping_profiles
        if !cfg.scoping_profiles.contains_key(&entity.scoping_profile) {
            errors.push(format!(
                "entity {code} references unknown scoping_profile {}",
                entity.scoping_profile
            ));
        }

        // Check 3: parent_code (if Some) must exist in ownership.entities
        if let Some(ref pc) = entity.parent_code {
            if !entity_codes.contains(pc.as_str()) {
                errors.push(format!(
                    "entity {code} has parent_code {pc} which is not in ownership.entities"
                ));
            }
        }

        // Check 4: ownership_percent must be in [0.0, 1.0]
        if let Some(pct) = entity.ownership_percent {
            if pct < rust_decimal::Decimal::ZERO || pct > rust_decimal::Decimal::ONE {
                errors.push(format!(
                    "entity {code} has ownership_percent {pct} outside [0.0, 1.0]"
                ));
            }
        }

        // Check 8 (I2): typo detection on entity.overrides keys
        for key in entity.overrides.keys() {
            if let Some(suggestion) = find_closest_field(key) {
                errors.push(format!(
                    "entity {code}: possible typo in field '{key}' — did you mean '{suggestion}'?"
                ));
            }
        }
    }

    // -----------------------------------------------------------------------
    // Check 5 + 6: IC relationship validation
    // -----------------------------------------------------------------------
    for rel in &cfg.intercompany.relationships {
        match rel {
            IcRelationshipConfig::Explicit(ex) => {
                // Check 5a: seller must be in entities
                if !entity_codes.contains(ex.seller.as_str()) {
                    errors.push(format!(
                        "IC relationship references unknown entity {}",
                        ex.seller
                    ));
                }
                // Check 5b: buyer must be in entities
                if !entity_codes.contains(ex.buyer.as_str()) {
                    errors.push(format!(
                        "IC relationship references unknown entity {}",
                        ex.buyer
                    ));
                }
                // Check 5c: self-pair
                if ex.seller == ex.buyer {
                    errors.push(format!(
                        "IC relationship has seller == buyer ({})",
                        ex.seller
                    ));
                }
            }
            IcRelationshipConfig::Pattern(pat) => {
                let p = &pat.pattern;

                // Check 6a: pattern.seller (if Some) must be in entities
                if let Some(ref seller) = p.seller {
                    if !entity_codes.contains(seller.as_str()) {
                        errors.push(format!(
                            "IC pattern references unknown entity/profile {seller}"
                        ));
                    }
                }
                // Check 6b: pattern.buyer (if Some) must be in entities
                if let Some(ref buyer) = p.buyer {
                    if !entity_codes.contains(buyer.as_str()) {
                        errors.push(format!(
                            "IC pattern references unknown entity/profile {buyer}"
                        ));
                    }
                }
                // Check 6c: seller_scoping_profile (if Some and not "any") must exist
                if let Some(ref sp) = p.seller_scoping_profile {
                    if sp != "any" && !cfg.scoping_profiles.contains_key(sp) {
                        errors.push(format!("IC pattern references unknown entity/profile {sp}"));
                    }
                }
                // Check 6d: buyer_scoping_profile (if Some and not "any") must exist
                if let Some(ref sp) = p.buyer_scoping_profile {
                    if sp != "any" && !cfg.scoping_profiles.contains_key(sp) {
                        errors.push(format!("IC pattern references unknown entity/profile {sp}"));
                    }
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(GroupError::Config(errors.join("\n")))
    }
}

/// Return the closest known `EntityConfig` field name if `key` is within
/// [`TYPO_DISTANCE_THRESHOLD`] of one, otherwise `None`.
fn find_closest_field(key: &str) -> Option<&'static str> {
    let mut best_dist = usize::MAX;
    let mut best_field = None;
    for &field in ENTITY_CONFIG_FIELDS {
        let d = strsim::levenshtein(key, field);
        if d < best_dist {
            best_dist = d;
            best_field = Some(field);
        }
    }
    if best_dist <= TYPO_DISTANCE_THRESHOLD && best_dist > 0 {
        best_field
    } else {
        None
    }
}

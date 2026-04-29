//! Three-level inheritance: defaults → scoping_profile → per-entity override.
//! Produces a [`ResolvedEntity`] with every field the orchestrator needs.

use crate::config::{EntityConfig, GroupConfig};
use crate::errors::{GroupError, GroupResult};

/// Fully resolved entity ready for shard execution.
#[derive(Debug, Clone)]
pub struct ResolvedEntity {
    pub code: String,
    pub name: Option<String>,
    pub country: String,
    pub functional_currency: String,
    pub scoping_profile_name: String,
    pub consolidation_method: crate::config::ConsolidationMethod,
    pub ownership_percent: Option<rust_decimal::Decimal>,
    pub parent_code: Option<String>,
    pub accounting_framework: String,
    pub industry: String,
    pub process_models: Vec<String>,
    pub rows: Option<u64>,
    /// The merged YAML map of defaults ⊕ scoping_profile ⊕ per-entity overrides.
    /// Downstream consumers project their required keys out of this (e.g. audit, tax).
    pub merged_config: serde_yaml::Value,
}

/// Resolve a single entity by code.
pub fn resolve_entity(cfg: &GroupConfig, code: &str) -> GroupResult<ResolvedEntity> {
    let entity = cfg
        .ownership
        .entities
        .iter()
        .find(|e| e.code == code)
        .ok_or_else(|| GroupError::Config(format!("entity {code} not found")))?;
    resolve_entity_inner(cfg, entity)
}

fn resolve_entity_inner(cfg: &GroupConfig, entity: &EntityConfig) -> GroupResult<ResolvedEntity> {
    // Start from defaults (may be serde_yaml::Value::Null if absent).
    let mut merged = cfg.defaults.clone();
    if merged.is_null() {
        merged = serde_yaml::Value::Mapping(Default::default());
    }

    // Layer the scoping profile.
    let profile = cfg
        .scoping_profiles
        .get(&entity.scoping_profile)
        .ok_or_else(|| {
            GroupError::Config(format!(
                "entity {} references unknown scoping_profile {}",
                entity.code, entity.scoping_profile
            ))
        })?;
    deep_merge(&mut merged, profile);

    // Layer per-entity overrides (the `#[serde(default, flatten)]` map).
    for (k, v) in &entity.overrides {
        deep_merge_key(&mut merged, k, v);
    }

    // Pull fields out with fallbacks. Named EntityConfig fields win over merged map when set.
    let accounting_framework = entity
        .accounting_framework
        .clone()
        .or_else(|| read_str(&merged, "accounting_framework"))
        .unwrap_or_else(|| "ifrs".to_string());
    let industry = entity
        .industry
        .clone()
        .or_else(|| read_str(&merged, "industry"))
        .unwrap_or_else(|| "manufacturing".to_string());
    let process_models = read_str_vec(&merged, "process_models").unwrap_or_default();

    Ok(ResolvedEntity {
        code: entity.code.clone(),
        name: entity.name.clone(),
        country: entity.country.clone(),
        functional_currency: entity.functional_currency.clone(),
        scoping_profile_name: entity.scoping_profile.clone(),
        consolidation_method: entity.consolidation_method,
        ownership_percent: entity.ownership_percent,
        parent_code: entity.parent_code.clone(),
        accounting_framework,
        industry,
        process_models,
        rows: entity.rows,
        merged_config: merged,
    })
}

/// Deep-merge `overlay` into `base` (overlay wins on scalar conflicts; maps merge recursively;
/// sequences are replaced, not concatenated).
fn deep_merge(base: &mut serde_yaml::Value, overlay: &serde_yaml::Value) {
    use serde_yaml::Value::Mapping;
    match (base, overlay) {
        (Mapping(bm), Mapping(om)) => {
            for (k, v) in om {
                if let Some(bv) = bm.get_mut(k) {
                    deep_merge(bv, v);
                } else {
                    bm.insert(k.clone(), v.clone());
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

fn deep_merge_key(base: &mut serde_yaml::Value, key: &str, value: &serde_yaml::Value) {
    let k = serde_yaml::Value::String(key.into());
    match base {
        serde_yaml::Value::Mapping(m) => {
            if let Some(existing) = m.get_mut(&k) {
                deep_merge(existing, value);
            } else {
                m.insert(k, value.clone());
            }
        }
        _ => {
            let mut m = serde_yaml::Mapping::new();
            m.insert(k, value.clone());
            *base = serde_yaml::Value::Mapping(m);
        }
    }
}

fn read_str(v: &serde_yaml::Value, key: &str) -> Option<String> {
    v.as_mapping()?
        .get(serde_yaml::Value::String(key.into()))?
        .as_str()
        .map(String::from)
}

fn read_str_vec(v: &serde_yaml::Value, key: &str) -> Option<Vec<String>> {
    let seq = v
        .as_mapping()?
        .get(serde_yaml::Value::String(key.into()))?
        .as_sequence()?;
    Some(
        seq.iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
    )
}

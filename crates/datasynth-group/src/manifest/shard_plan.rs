//! Shard plan assignment — spec §4.1, Task 2.8.
//!
//! Groups entities by `scoping_profile`, reads the per-profile row budget from
//! the scoping-profiles YAML map, and batches entities into shards sized to
//! roughly 1 TB of archive output.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::errors::GroupResult;
use crate::manifest::expansion::ExpandedEntity;

// ── Shard sizing constants ────────────────────────────────────────────────────

/// Maximum estimated rows per shard.  At ~800 bytes/row (see below) this
/// corresponds to roughly 1 TB of uncompressed archive output — a practical
/// upper bound for a single shard process.
const MAX_ROWS_PER_SHARD: u64 = 10_000_000_000; // 10 B rows

/// Estimated bytes per generated row across all output files (JSON, CSV,
/// Parquet combined).  Used only for the `estimated_archive_size_mb` field;
/// the actual figure depends on schema width and compression ratio.
const BYTES_PER_ROW_ESTIMATE: u64 = 800;

/// Fallback row budget when `scoping_profiles[profile].row_budget` is absent.
const FALLBACK_ROW_BUDGET: u64 = 1_000_000;

/// Bytes per megabyte — standard SI definition.
const BYTES_PER_MB: u64 = 1_000_000;

// ── Public types ──────────────────────────────────────────────────────────────

/// A single shard assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardAssignment {
    /// Shard identifier, e.g. `"S_SIG_0001"`.
    ///
    /// Format: `"S_" + <3-letter uppercase initial> + "_" + <4-digit zero-padded index>`
    ///
    /// Profile → 3-letter initial mapping:
    /// | Profile            | Initial |
    /// |--------------------|---------|
    /// | `significant`      | `SIG`   |
    /// | `material`         | `MAT`   |
    /// | `consolidation_only` | `CON` |
    /// | anything else      | first 3 chars uppercased |
    pub shard_id: String,
    /// Name of the scoping profile that produced this shard.
    pub scoping_profile: String,
    /// Entity codes assigned to this shard (sorted ascending).
    pub entity_codes: Vec<String>,
    /// Sum of row budgets across all entities in this shard.
    pub estimated_rows: u64,
    /// Rough archive size: `estimated_rows × 800 bytes / 1 MB`.
    pub estimated_archive_size_mb: u64,
}

/// Full shard plan: all shard assignments across all profiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardPlan {
    /// All shard assignments, in profile-name order, then shard-index order.
    pub shards: Vec<ShardAssignment>,
}

// ── Public builder ────────────────────────────────────────────────────────────

/// Build the [`ShardPlan`] from the expanded entity list and raw scoping-profiles map.
///
/// Entities are grouped by `scoping_profile`.  Within each profile, entities
/// are sorted by `entity_code` ascending, then batched into shards such that
/// each shard's `estimated_rows ≤ MAX_ROWS_PER_SHARD` (10 B rows, ~1 TB).
///
/// Profiles with no entities produce no shards (silently tolerated).
pub fn build_shard_plan(
    entities: &[ExpandedEntity],
    scoping_profiles: &BTreeMap<String, serde_yaml::Value>,
) -> GroupResult<ShardPlan> {
    // ── 1. Group entities by scoping profile ──────────────────────────────────
    // BTreeMap gives deterministic profile iteration order.
    let mut profile_entities: BTreeMap<String, Vec<&ExpandedEntity>> = BTreeMap::new();

    for entity in entities {
        profile_entities
            .entry(entity.scoping_profile.clone())
            .or_default()
            .push(entity);
    }

    // ── 2. Sort entities within each profile by code ascending ───────────────
    for codes in profile_entities.values_mut() {
        codes.sort_by(|a, b| a.code.cmp(&b.code));
    }

    // ── 3. Batch into shards ──────────────────────────────────────────────────
    let mut shards: Vec<ShardAssignment> = Vec::new();

    for (profile, profile_ents) in &profile_entities {
        if profile_ents.is_empty() {
            continue;
        }

        let row_budget_per_entity = read_row_budget(scoping_profiles, profile);
        let initial = profile_initial(profile);

        // Batch accumulation.
        let mut shard_index: u32 = 1;
        let mut current_codes: Vec<String> = Vec::new();
        let mut current_rows: u64 = 0;

        for entity in profile_ents {
            let entity_rows = entity.rows.unwrap_or(row_budget_per_entity);

            // If adding this entity would overflow the shard, flush first.
            // Exception: if current shard is empty, always add (avoids infinite
            // loop when a single entity exceeds MAX_ROWS_PER_SHARD).
            if !current_codes.is_empty()
                && current_rows.saturating_add(entity_rows) > MAX_ROWS_PER_SHARD
            {
                shards.push(make_shard(
                    &initial,
                    shard_index,
                    profile,
                    std::mem::take(&mut current_codes),
                    current_rows,
                ));
                shard_index += 1;
                current_rows = 0;
            }

            current_codes.push(entity.code.clone());
            current_rows = current_rows.saturating_add(entity_rows);
        }

        // Flush remaining entities.
        if !current_codes.is_empty() {
            shards.push(make_shard(
                &initial,
                shard_index,
                profile,
                current_codes,
                current_rows,
            ));
        }
    }

    Ok(ShardPlan { shards })
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Derive the 3-letter shard initial for a scoping profile name.
///
/// | Profile               | Initial |
/// |-----------------------|---------|
/// | `significant`         | `SIG`   |
/// | `material`            | `MAT`   |
/// | `consolidation_only`  | `CON`   |
/// | anything else         | first 3 uppercase chars of the profile name |
fn profile_initial(profile: &str) -> String {
    match profile {
        "significant" => "SIG".to_string(),
        "material" => "MAT".to_string(),
        "consolidation_only" => "CON".to_string(),
        other => {
            // Take the first 3 ASCII letters (skip non-alpha), uppercase.
            let letters: String = other
                .chars()
                .filter(|c| c.is_ascii_alphabetic())
                .take(3)
                .collect::<String>()
                .to_uppercase();
            if letters.is_empty() {
                "UNK".to_string()
            } else {
                // Pad to 3 chars with 'X' if shorter than 3.
                format!("{letters:X<3}")
            }
        }
    }
}

/// Read the `row_budget` key from `scoping_profiles[profile]`, defaulting to
/// `FALLBACK_ROW_BUDGET` when absent or unparseable.
fn read_row_budget(scoping_profiles: &BTreeMap<String, serde_yaml::Value>, profile: &str) -> u64 {
    scoping_profiles
        .get(profile)
        .and_then(|v| v.get("row_budget"))
        .and_then(|v| v.as_u64())
        .unwrap_or(FALLBACK_ROW_BUDGET)
}

impl ShardPlan {
    /// Build a `entity_code → shard_id` lookup map from the shard plan.
    ///
    /// Used by the manifest builder to stamp `shard_id` onto each
    /// [`crate::manifest::builder::ManifestEntity`] without an O(n²) scan.
    pub fn shard_by_code(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        for s in &self.shards {
            for code in &s.entity_codes {
                m.insert(code.clone(), s.shard_id.clone());
            }
        }
        m
    }
}

/// Construct a [`ShardAssignment`] from accumulated data.
fn make_shard(
    initial: &str,
    index: u32,
    profile: &str,
    entity_codes: Vec<String>,
    estimated_rows: u64,
) -> ShardAssignment {
    let shard_id = format!("S_{initial}_{index:04}");
    let estimated_archive_size_mb =
        estimated_rows.saturating_mul(BYTES_PER_ROW_ESTIMATE) / BYTES_PER_MB;
    ShardAssignment {
        shard_id,
        scoping_profile: profile.to_string(),
        entity_codes,
        estimated_rows,
        estimated_archive_size_mb,
    }
}

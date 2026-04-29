//! Deterministic seed derivation for the group manifest (spec §2.4).
//!
//! All RNG state in the group engine derives from a single user-supplied
//! `group_seed: u64` via a blake3 hash tree. The key property is that
//! derivations are **order-independent**: adding, removing, or reordering
//! entities in the YAML does not change any other entity's derived seed.

use chrono::NaiveDate;

/// Deterministic seed for manifest-phase work (ownership graph population,
/// shared-master ID allocations, FX rate master, IC pair plan layout,
/// audit materiality allocation).
pub fn derive_manifest_seed(group_seed: u64, period_start: NaiveDate) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"manifest");
    hasher.update(&group_seed.to_le_bytes());
    hasher.update(period_start.to_string().as_bytes());
    *hasher.finalize().as_bytes()
}

/// Deterministic per-entity seed, order-independent on entity code.
pub fn derive_entity_seed(group_seed: u64, entity_code: &str) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"entity");
    hasher.update(&group_seed.to_le_bytes());
    hasher.update(entity_code.as_bytes());
    *hasher.finalize().as_bytes()
}

/// Deterministic seed for aggregate-phase work (IC matching tiebreaks,
/// component auditor assignment tiebreaks).
pub fn derive_aggregate_seed(group_seed: u64, period_start: NaiveDate) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"aggregate");
    hasher.update(&group_seed.to_le_bytes());
    hasher.update(period_start.to_string().as_bytes());
    *hasher.finalize().as_bytes()
}

/// Deterministic ID for a single intercompany pair instance within a
/// relationship. Both sides of the pair (seller shard and buyer shard)
/// derive the identical value from the same inputs.
///
/// This returns a 32-byte hash; Task 3.1 (datasynth-core) will wrap it in
/// an `IcPairId` newtype. For now, the raw array is the interface.
pub fn derive_ic_pair_id(group_seed: u64, ic_relationship_id: &str, pair_index: u64) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"ic_pair");
    hasher.update(&group_seed.to_le_bytes());
    hasher.update(ic_relationship_id.as_bytes());
    hasher.update(&pair_index.to_le_bytes());
    *hasher.finalize().as_bytes()
}

/// Convenience: seed a ChaCha8 RNG from a 32-byte blake3 digest.
/// Takes the first 32 bytes of the digest as the ChaCha seed.
pub fn chacha_rng_from_seed(seed: &[u8; 32]) -> rand_chacha::ChaCha8Rng {
    use rand::SeedableRng;
    rand_chacha::ChaCha8Rng::from_seed(*seed)
}

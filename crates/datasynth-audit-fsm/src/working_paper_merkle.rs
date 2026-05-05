//! Tamper-evident working-paper bundles via SHA-256 Merkle trees —
//! AuditMethodology v0.14.
//!
//! Sourced from `src/gam_scraper/working_papers/{merkle,models,bundle_hash}.py`.
//!
//! Produces a single 32-byte (64 hex chars) bundle root over an
//! engagement's working papers, plus per-paper inclusion proofs that
//! any inspector can verify against the published root — so any change
//! to a working paper after sign-off causes the root to diverge.
//!
//! # Layout
//!
//! - **`merkle`** sub-section — generic SHA-256 binary Merkle tree over
//!   hex-string nodes with the standard odd-leaf-last-duplicate rule.
//! - **`models`** sub-section — `WorkingPaper` / `EngagementBundle`
//!   per ISA 230, plus `Preparer` / `Reviewer` / `Signoff` /
//!   `Jurisdiction`.
//! - **`bundle`** sub-section — `bundle_root(bundle)` (sorts WPs by
//!   `wp_id` for canonical ordering) and
//!   `verify_bundle_integrity(bundle, claimed_root)`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ── Merkle primitives ─────────────────────────────────────────────────────────

/// SHA-256 hex digest of raw bytes.
pub fn leaf_hash(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    hex_encode(&digest)
}

/// Internal pair-hash: SHA-256 of `bytes(left) || bytes(right)` where
/// `left` and `right` are hex-encoded SHA-256 digests.
fn pair_hash(left: &str, right: &str) -> Result<String, MerkleError> {
    let l = hex_decode(left)?;
    let r = hex_decode(right)?;
    let mut hasher = Sha256::new();
    hasher.update(&l);
    hasher.update(&r);
    Ok(hex_encode(&hasher.finalize()))
}

/// Errors raised by Merkle / bundle operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MerkleError {
    /// `merkle_root` / `merkle_proof` called on an empty leaf list.
    EmptyLeaves,
    /// Index out of range for the leaf list.
    IndexOutOfRange { index: usize, len: usize },
    /// Hex string is not 64 chars or contains non-hex digits.
    NotHexSha256(String),
}

impl std::fmt::Display for MerkleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MerkleError::EmptyLeaves => write!(f, "merkle_root requires at least one leaf"),
            MerkleError::IndexOutOfRange { index, len } => {
                write!(f, "index {index} out of range for {len} leaves")
            }
            MerkleError::NotHexSha256(s) => {
                write!(f, "hex string {s:?} is not a valid SHA-256 digest")
            }
        }
    }
}

impl std::error::Error for MerkleError {}

/// Compute the Merkle root over a list of hex-encoded SHA-256 leaf
/// hashes.  Odd-leaf rule: duplicate the last leaf at each level.
pub fn merkle_root(leaves: &[String]) -> Result<String, MerkleError> {
    if leaves.is_empty() {
        return Err(MerkleError::EmptyLeaves);
    }
    let mut level: Vec<String> = leaves.to_vec();
    while level.len() > 1 {
        if !level.len().is_multiple_of(2) {
            level.push(level.last().unwrap().clone());
        }
        let mut next = Vec::with_capacity(level.len() / 2);
        for pair in level.chunks(2) {
            next.push(pair_hash(&pair[0], &pair[1])?);
        }
        level = next;
    }
    Ok(level.into_iter().next().unwrap())
}

/// Direction of a sibling in a Merkle proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofDirection {
    /// Sibling is on the left of the current node.
    Left,
    /// Sibling is on the right of the current node.
    Right,
}

/// One step of an inclusion proof: the sibling hash at this level and
/// its position relative to the path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofStep {
    pub sibling: String,
    pub direction: ProofDirection,
}

/// Produce an inclusion proof for `leaves[index]`: ordered sibling
/// hashes plus their position relative to the path.  Caller verifies by
/// hashing upward.
pub fn merkle_proof(leaves: &[String], index: usize) -> Result<Vec<ProofStep>, MerkleError> {
    if index >= leaves.len() {
        return Err(MerkleError::IndexOutOfRange {
            index,
            len: leaves.len(),
        });
    }
    let mut steps = Vec::new();
    let mut level: Vec<String> = leaves.to_vec();
    let mut idx = index;
    while level.len() > 1 {
        if !level.len().is_multiple_of(2) {
            level.push(level.last().unwrap().clone());
        }
        if idx.is_multiple_of(2) {
            steps.push(ProofStep {
                sibling: level[idx + 1].clone(),
                direction: ProofDirection::Right,
            });
        } else {
            steps.push(ProofStep {
                sibling: level[idx - 1].clone(),
                direction: ProofDirection::Left,
            });
        }
        let mut next = Vec::with_capacity(level.len() / 2);
        for pair in level.chunks(2) {
            next.push(pair_hash(&pair[0], &pair[1])?);
        }
        level = next;
        idx /= 2;
    }
    Ok(steps)
}

/// Verify that `leaf` at `index` rolls up to `root` given `proof`.
///
/// Direction is re-derived from `index` at each level — independent of
/// the direction stored in each `ProofStep` — to ensure a proof
/// generated for a different index cannot be replayed at a wrong
/// position.
pub fn verify_merkle_proof(
    leaf: &str,
    proof: &[ProofStep],
    index: usize,
    root: &str,
) -> Result<bool, MerkleError> {
    let mut current = leaf.to_string();
    let mut idx = index;
    for step in proof {
        if idx.is_multiple_of(2) {
            current = pair_hash(&current, &step.sibling)?;
        } else {
            current = pair_hash(&step.sibling, &current)?;
        }
        idx /= 2;
    }
    Ok(current == root)
}

// ── ISA 230 working-paper models ──────────────────────────────────────────────

/// Jurisdiction governing retention period and documentation deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Jurisdiction {
    /// IAASB / IFRS-based.
    Iaasb,
    /// US listed (PCAOB).
    UsPcaob,
    /// UK FRC.
    UkFrc,
    /// EU CSRD / ESRS.
    EuCsrd,
    /// Swiss FER.
    SwissFer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preparer {
    pub verifier_id: i64,
    pub name: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reviewer {
    pub verifier_id: i64,
    pub name: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signoff {
    pub verifier_id: i64,
    pub name: String,
    pub role: String,
    pub signed_at: DateTime<Utc>,
    pub attestation: String,
}

/// Working paper per ISA 230.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingPaper {
    pub wp_id: String,
    pub title: String,
    /// Lower-case 64-char SHA-256 hex digest of the working paper's
    /// payload bytes.
    pub payload_hash: String,
    pub preparer: Preparer,
    pub prepared_at: DateTime<Utc>,
    #[serde(default)]
    pub reviewer: Option<Reviewer>,
    #[serde(default)]
    pub signoff: Option<Signoff>,
    /// Retention period in years (must be 5–10 per ISA 230 / PCAOB AS 1215 /
    /// FRC ISA 230 / Swiss equivalent).
    pub retention_years: u8,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngagementBundle {
    pub engagement_id: String,
    pub jurisdiction: Jurisdiction,
    pub period_end: DateTime<Utc>,
    pub report_date: DateTime<Utc>,
    #[serde(default)]
    pub working_papers: Vec<WorkingPaper>,
    #[serde(default)]
    pub description: String,
}

/// Validation errors for working-paper bundles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleError {
    /// `payload_hash` is not 64 chars or contains non-hex digits.
    BadPayloadHash { wp_id: String, payload_hash: String },
    /// Retention period out of the 5–10-year range required by ISA 230.
    RetentionOutOfRange { wp_id: String, years: u8 },
    /// Internal Merkle error.
    Merkle(MerkleError),
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleError::BadPayloadHash {
                wp_id,
                payload_hash,
            } => {
                write!(
                    f,
                    "working paper {wp_id:?} has invalid payload_hash {payload_hash:?} (must be 64-char hex SHA-256)"
                )
            }
            BundleError::RetentionOutOfRange { wp_id, years } => {
                write!(
                    f,
                    "working paper {wp_id:?} retention {years} years out of range (must be 5-10)"
                )
            }
            BundleError::Merkle(e) => write!(f, "merkle error: {e}"),
        }
    }
}

impl std::error::Error for BundleError {}

impl From<MerkleError> for BundleError {
    fn from(e: MerkleError) -> Self {
        BundleError::Merkle(e)
    }
}

// ── Bundle root + verification ────────────────────────────────────────────────

/// Canonical empty-bundle root (64 zeroed hex chars) — matches upstream
/// Python.  An empty bundle is allowed; downstream tools may treat it
/// as "no working papers", not "empty SHA-256".
pub const EMPTY_BUNDLE_ROOT: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// Validate working-paper invariants before computing the root.
fn validate(bundle: &EngagementBundle) -> Result<(), BundleError> {
    for wp in &bundle.working_papers {
        if wp.payload_hash.len() != 64 || !wp.payload_hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(BundleError::BadPayloadHash {
                wp_id: wp.wp_id.clone(),
                payload_hash: wp.payload_hash.clone(),
            });
        }
        if !(5..=10).contains(&wp.retention_years) {
            return Err(BundleError::RetentionOutOfRange {
                wp_id: wp.wp_id.clone(),
                years: wp.retention_years,
            });
        }
    }
    Ok(())
}

/// Compute the Merkle root over all working-paper payload hashes,
/// ordered by `wp_id` (lexical) for canonicity.  Returns
/// `EMPTY_BUNDLE_ROOT` for an empty bundle.
pub fn bundle_root(bundle: &EngagementBundle) -> Result<String, BundleError> {
    validate(bundle)?;
    if bundle.working_papers.is_empty() {
        return Ok(EMPTY_BUNDLE_ROOT.to_string());
    }
    let mut wps: Vec<&WorkingPaper> = bundle.working_papers.iter().collect();
    wps.sort_by(|a, b| a.wp_id.cmp(&b.wp_id));
    let leaves: Vec<String> = wps.iter().map(|w| w.payload_hash.to_lowercase()).collect();
    Ok(merkle_root(&leaves)?)
}

/// Return `Ok(true)` iff the bundle's working papers reproduce
/// `claimed_root`.  Returns `Ok(false)` on mismatch.
pub fn verify_bundle_integrity(
    bundle: &EngagementBundle,
    claimed_root: &str,
) -> Result<bool, BundleError> {
    Ok(bundle_root(bundle)? == claimed_root)
}

// ── Hex helpers ───────────────────────────────────────────────────────────────

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        write!(&mut s, "{b:02x}").unwrap();
    }
    s
}

fn hex_decode(s: &str) -> Result<Vec<u8>, MerkleError> {
    if s.len() != 64 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(MerkleError::NotHexSha256(s.to_string()));
    }
    let mut out = Vec::with_capacity(32);
    let bytes = s.as_bytes();
    for i in 0..32 {
        let high = ascii_hex_to_nibble(bytes[i * 2])?;
        let low = ascii_hex_to_nibble(bytes[i * 2 + 1])?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn ascii_hex_to_nibble(c: u8) -> Result<u8, MerkleError> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(MerkleError::NotHexSha256(
            String::from_utf8_lossy(&[c]).into_owned(),
        )),
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
    }

    fn mk_wp(wp_id: &str, payload: &[u8]) -> WorkingPaper {
        WorkingPaper {
            wp_id: wp_id.to_string(),
            title: format!("WP {wp_id}"),
            payload_hash: leaf_hash(payload),
            preparer: Preparer {
                verifier_id: 1,
                name: "Alice".to_string(),
                role: "senior".to_string(),
            },
            prepared_at: ts(2026, 5, 4),
            reviewer: None,
            signoff: None,
            retention_years: 7,
            description: String::new(),
        }
    }

    fn mk_bundle(wps: Vec<WorkingPaper>) -> EngagementBundle {
        EngagementBundle {
            engagement_id: "ENG-001".to_string(),
            jurisdiction: Jurisdiction::Iaasb,
            period_end: ts(2026, 12, 31),
            report_date: ts(2027, 3, 15),
            working_papers: wps,
            description: String::new(),
        }
    }

    #[test]
    fn leaf_hash_matches_known_sha256() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assert_eq!(
            leaf_hash(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        assert_eq!(
            leaf_hash(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn merkle_root_single_leaf_returns_that_leaf() {
        let leaf = leaf_hash(b"only");
        assert_eq!(merkle_root(std::slice::from_ref(&leaf)).unwrap(), leaf);
    }

    #[test]
    fn merkle_root_empty_errors() {
        assert!(matches!(merkle_root(&[]), Err(MerkleError::EmptyLeaves)));
    }

    #[test]
    fn merkle_root_two_leaves_is_pair_hash() {
        let l1 = leaf_hash(b"a");
        let l2 = leaf_hash(b"b");
        let root = merkle_root(&[l1.clone(), l2.clone()]).unwrap();
        assert_eq!(root, pair_hash(&l1, &l2).unwrap());
    }

    #[test]
    fn merkle_root_odd_leaves_duplicates_last() {
        let l1 = leaf_hash(b"a");
        let l2 = leaf_hash(b"b");
        let l3 = leaf_hash(b"c");
        let root3 = merkle_root(&[l1.clone(), l2.clone(), l3.clone()]).unwrap();
        // Manual: hash(hash(a,b), hash(c,c))
        let hash_ab = pair_hash(&l1, &l2).unwrap();
        let hash_cc = pair_hash(&l3, &l3).unwrap();
        let expected = pair_hash(&hash_ab, &hash_cc).unwrap();
        assert_eq!(root3, expected);
    }

    #[test]
    fn merkle_proof_round_trips_for_each_leaf() {
        let leaves: Vec<String> = (0..7)
            .map(|i| leaf_hash(format!("leaf-{i}").as_bytes()))
            .collect();
        let root = merkle_root(&leaves).unwrap();
        for (i, leaf) in leaves.iter().enumerate() {
            let proof = merkle_proof(&leaves, i).unwrap();
            let ok = verify_merkle_proof(leaf, &proof, i, &root).unwrap();
            assert!(ok, "proof failed for index {i}");
        }
    }

    #[test]
    fn merkle_proof_rejects_replay_at_wrong_index() {
        let leaves: Vec<String> = (0..4)
            .map(|i| leaf_hash(format!("leaf-{i}").as_bytes()))
            .collect();
        let root = merkle_root(&leaves).unwrap();
        let proof = merkle_proof(&leaves, 0).unwrap();
        // Replay against index 1 — direction is re-derived from the index,
        // so it should fail (assuming a non-trivial 2-level tree).
        let ok = verify_merkle_proof(&leaves[0], &proof, 1, &root).unwrap();
        assert!(!ok);
    }

    #[test]
    fn merkle_proof_index_out_of_range_errors() {
        let leaves = vec![leaf_hash(b"only")];
        let err = merkle_proof(&leaves, 5).unwrap_err();
        assert!(matches!(err, MerkleError::IndexOutOfRange { .. }));
    }

    #[test]
    fn empty_bundle_yields_canonical_zero_root() {
        let b = mk_bundle(vec![]);
        assert_eq!(bundle_root(&b).unwrap(), EMPTY_BUNDLE_ROOT);
    }

    #[test]
    fn bundle_root_is_canonical_independent_of_insertion_order() {
        let wps = vec![
            mk_wp("WP-002", b"two"),
            mk_wp("WP-001", b"one"),
            mk_wp("WP-003", b"three"),
        ];
        let b1 = mk_bundle(wps.clone());
        // Reverse order of insertion should NOT change the root because
        // bundle_root sorts WPs by wp_id.
        let mut wps_rev = wps.clone();
        wps_rev.reverse();
        let b2 = mk_bundle(wps_rev);
        assert_eq!(bundle_root(&b1).unwrap(), bundle_root(&b2).unwrap());
    }

    #[test]
    fn tamper_changes_bundle_root() {
        let wps = vec![
            mk_wp("WP-001", b"original payload"),
            mk_wp("WP-002", b"another"),
        ];
        let original = mk_bundle(wps.clone());
        let original_root = bundle_root(&original).unwrap();

        let mut tampered_wps = wps.clone();
        tampered_wps[0].payload_hash = leaf_hash(b"tampered payload");
        let tampered = mk_bundle(tampered_wps);

        let tampered_root = bundle_root(&tampered).unwrap();
        assert_ne!(original_root, tampered_root);
        // verify_bundle_integrity catches the tamper.
        assert!(!verify_bundle_integrity(&tampered, &original_root).unwrap());
        assert!(verify_bundle_integrity(&original, &original_root).unwrap());
    }

    #[test]
    fn invalid_payload_hash_rejected_at_root_time() {
        let mut wp = mk_wp("WP-001", b"x");
        wp.payload_hash = "not_a_hex_string_oh_no".to_string();
        let b = mk_bundle(vec![wp]);
        let err = bundle_root(&b).unwrap_err();
        assert!(matches!(err, BundleError::BadPayloadHash { .. }));
    }

    #[test]
    fn retention_out_of_range_rejected() {
        let mut wp = mk_wp("WP-001", b"x");
        wp.retention_years = 3;
        let b = mk_bundle(vec![wp]);
        let err = bundle_root(&b).unwrap_err();
        assert!(matches!(err, BundleError::RetentionOutOfRange { .. }));

        let mut wp = mk_wp("WP-001", b"x");
        wp.retention_years = 11;
        let b = mk_bundle(vec![wp]);
        let err = bundle_root(&b).unwrap_err();
        assert!(matches!(err, BundleError::RetentionOutOfRange { .. }));
    }

    #[test]
    fn json_round_trips_bundle() {
        let bundle = mk_bundle(vec![mk_wp("WP-001", b"a"), mk_wp("WP-002", b"b")]);
        let json = serde_json::to_string(&bundle).unwrap();
        let back: EngagementBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(bundle, back);
        // Discriminator `jurisdiction` serializes as snake_case.
        assert!(json.contains("\"iaasb\""));
    }

    #[test]
    fn bundle_root_uses_lowercase_hex() {
        let mut wp = mk_wp("WP-001", b"x");
        wp.payload_hash = wp.payload_hash.to_uppercase();
        let b = mk_bundle(vec![wp.clone()]);
        // bundle_root accepts upper-case input but normalises to lower-case
        // for the leaf, matching the merkle_root output format.
        let r = bundle_root(&b).unwrap();
        assert!(r.chars().all(|c| !c.is_ascii_uppercase()));
        // Bundle with the same payload but stored lower-case should match.
        let mut wp2 = wp.clone();
        wp2.payload_hash = wp.payload_hash.to_lowercase();
        let b2 = mk_bundle(vec![wp2]);
        assert_eq!(r, bundle_root(&b2).unwrap());
    }

    #[test]
    fn proof_step_serde_round_trip() {
        let step = ProofStep {
            sibling: leaf_hash(b"s"),
            direction: ProofDirection::Left,
        };
        let json = serde_json::to_string(&step).unwrap();
        let back: ProofStep = serde_json::from_str(&json).unwrap();
        assert_eq!(step, back);
        assert!(json.contains("\"left\""));
    }
}

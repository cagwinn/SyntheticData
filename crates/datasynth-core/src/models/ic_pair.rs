//! Intercompany pair identifier — links a seller-side journal entry to a
//! buyer-side journal entry across entities in a consolidated group.
//!
//! The 32-byte payload is the output of `blake3("ic_pair" || group_seed ||
//! ic_relationship_id || pair_index)`, derived deterministically by the
//! group engine's seed tree. Serializes as a lowercase hex string
//! (64 characters) for human-readable JSON output.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 32-byte intercompany pair identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IcPairId([u8; 32]);

impl IcPairId {
    /// Construct from a raw 32-byte array (typically the output of
    /// `datasynth_group::manifest::seeds::derive_ic_pair_id`).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Raw bytes accessor.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hex encoding (64 chars).
    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for byte in &self.0 {
            use std::fmt::Write;
            write!(&mut s, "{byte:02x}").expect("writing to String never fails");
        }
        s
    }

    /// Parse from a 64-character lowercase hex string.
    pub fn from_hex(hex: &str) -> Result<Self, IcPairIdParseError> {
        if hex.len() != 64 {
            return Err(IcPairIdParseError::WrongLength(hex.len()));
        }
        let mut out = [0u8; 32];
        for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
            let high = hex_digit(pair[0])?;
            let low = hex_digit(pair[1])?;
            out[i] = (high << 4) | low;
        }
        Ok(Self(out))
    }
}

fn hex_digit(c: u8) -> Result<u8, IcPairIdParseError> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(IcPairIdParseError::InvalidDigit(c as char)),
    }
}

/// Error returned when parsing an [`IcPairId`] from a hex string.
#[derive(Debug, thiserror::Error)]
pub enum IcPairIdParseError {
    #[error("expected 64 hex chars, got {0}")]
    WrongLength(usize),
    #[error("invalid hex digit: {0}")]
    InvalidDigit(char),
}

impl std::fmt::Display for IcPairId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for IcPairId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for IcPairId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let hex = String::deserialize(d)?;
        Self::from_hex(&hex).map_err(serde::de::Error::custom)
    }
}

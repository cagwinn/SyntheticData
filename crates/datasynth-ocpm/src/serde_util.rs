//! Serde helpers for OCPM datetime serialization.
//!
//! Chrono's default `DateTime<Utc>` serializer emits nanosecond precision
//! (`2026-04-16T18:23:11.567853252Z`), which pandas `to_datetime(..., utc=True)`
//! silently drops back to microsecond precision — collapsing events that differ
//! only in nanoseconds. These helpers force microsecond precision on the wire
//! while preserving full precision in memory.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Serde helper for `DateTime<Utc>` that serializes with microsecond precision
/// RFC3339 strings. Deserialization accepts any RFC3339 precision.
pub mod rfc3339_micros {
    use super::*;

    pub fn serialize<S: Serializer>(dt: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        dt.to_rfc3339_opts(SecondsFormat::Micros, true).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        let s = String::deserialize(d)?;
        DateTime::parse_from_rfc3339(&s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(serde::de::Error::custom)
    }
}

/// Serde helper for `Option<DateTime<Utc>>` with microsecond precision.
pub mod rfc3339_micros_opt {
    use super::*;

    pub fn serialize<S: Serializer>(dt: &Option<DateTime<Utc>>, s: S) -> Result<S::Ok, S::Error> {
        match dt {
            Some(dt) => dt.to_rfc3339_opts(SecondsFormat::Micros, true).serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DateTime<Utc>>, D::Error> {
        let s: Option<String> = Option::deserialize(d)?;
        match s {
            Some(s) => DateTime::parse_from_rfc3339(&s)
                .map(|dt| Some(dt.with_timezone(&Utc)))
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Wrap {
        #[serde(with = "rfc3339_micros")]
        ts: DateTime<Utc>,
        #[serde(with = "rfc3339_micros_opt")]
        ts_opt: Option<DateTime<Utc>>,
    }

    #[test]
    fn serializes_with_microsecond_precision() {
        // Build a timestamp with full nanosecond precision.
        let ts = Utc
            .with_ymd_and_hms(2026, 4, 16, 18, 23, 11)
            .single()
            .expect("valid datetime")
            .with_nanosecond(567_853_252)
            .expect("valid nanos");
        let w = Wrap {
            ts,
            ts_opt: Some(ts),
        };
        let json = serde_json::to_string(&w).expect("serialize ok");
        // Nanoseconds must not appear; only 6 fractional digits (microseconds).
        assert!(
            json.contains(".567853Z"),
            "expected microsecond precision, got: {json}"
        );
        assert!(
            !json.contains(".567853252"),
            "nanoseconds leaked through: {json}"
        );
    }

    #[test]
    fn roundtrip_preserves_microsecond_value() {
        let ts = Utc
            .with_ymd_and_hms(2026, 4, 16, 18, 23, 11)
            .single()
            .expect("valid datetime")
            .with_nanosecond(567_853_000) // already micro-aligned
            .expect("valid nanos");
        let w = Wrap { ts, ts_opt: None };
        let json = serde_json::to_string(&w).expect("serialize ok");
        let back: Wrap = serde_json::from_str(&json).expect("deserialize ok");
        assert_eq!(back.ts, ts);
        assert_eq!(back.ts_opt, None);
    }
}

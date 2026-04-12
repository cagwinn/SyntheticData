//! Normalized timestamp serialization.
//!
//! Truncates all timestamps to microsecond precision and normalizes to UTC
//! with a `Z` suffix. This ensures consistent timestamp formats across all
//! output files, preventing pandas "Mixed timezones detected" errors.
//!
//! ## Modules
//!
//! - [`utc`] — for `DateTime<Utc>` fields
//! - [`utc::option`] — for `Option<DateTime<Utc>>` fields
//! - [`naive`] — for `NaiveDateTime` fields (serializes with Z suffix)
//! - [`naive::option`] — for `Option<NaiveDateTime>` fields

use std::fmt;

use std::io::Write;

use chrono::{DateTime, Datelike, NaiveDateTime, Timelike, Utc};
use serde::{self, Deserializer, Serializer};

/// Format a NaiveDateTime as `YYYY-MM-DDTHH:MM:SS[.ffffff]Z` into a stack buffer.
/// Truncates to microsecond precision. Returns the formatted length.
fn format_normalized(dt: NaiveDateTime, buf: &mut [u8; 32]) -> usize {
    let micros = (dt.nanosecond() / 1_000) % 1_000_000;
    let mut cursor = std::io::Cursor::new(&mut buf[..]);
    if micros > 0 {
        let _ = write!(
            cursor,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}Z",
            dt.year(),
            dt.month(),
            dt.day(),
            dt.hour(),
            dt.minute(),
            dt.second(),
            micros,
        );
    } else {
        let _ = write!(
            cursor,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            dt.year(),
            dt.month(),
            dt.day(),
            dt.hour(),
            dt.minute(),
            dt.second(),
        );
    }
    cursor.position() as usize
}

/// Serialize a NaiveDateTime as a normalized UTC timestamp string.
fn serialize_normalized<S: Serializer>(
    dt: NaiveDateTime,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut buf = [0u8; 32];
    let len = format_normalized(dt, &mut buf);
    let s = std::str::from_utf8(&buf[..len]).expect("timestamp is always ASCII");
    serializer.serialize_str(s)
}

// =========================================================================
// utc — for DateTime<Utc> fields
// =========================================================================

/// Normalized serialization for `DateTime<Utc>` fields.
///
/// ```ignore
/// #[serde(with = "datasynth_core::serde_timestamp::utc")]
/// pub created_at: DateTime<Utc>,
/// ```
pub mod utc {
    use super::*;

    pub fn serialize<S: Serializer>(
        value: &DateTime<Utc>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serialize_normalized(value.naive_utc(), serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<DateTime<Utc>, D::Error> {
        deserializer.deserialize_any(UtcVisitor)
    }

    /// Normalized serialization for `Option<DateTime<Utc>>` fields.
    pub mod option {
        use super::*;

        pub fn serialize<S: Serializer>(
            value: &Option<DateTime<Utc>>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            match value {
                Some(dt) => serialize_normalized(dt.naive_utc(), serializer),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<DateTime<Utc>>, D::Error> {
            deserializer.deserialize_any(OptionUtcVisitor)
        }
    }
}

// =========================================================================
// naive — for NaiveDateTime fields (serialized with Z suffix)
// =========================================================================

/// Normalized serialization for `NaiveDateTime` fields.
///
/// Appends Z suffix so all timestamps are in a uniform UTC-like format,
/// preventing "Mixed timezones detected" errors in pandas.
///
/// ```ignore
/// #[serde(with = "datasynth_core::serde_timestamp::naive")]
/// pub entry_timestamp: NaiveDateTime,
/// ```
pub mod naive {
    use super::*;

    pub fn serialize<S: Serializer>(
        value: &NaiveDateTime,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serialize_normalized(*value, serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<NaiveDateTime, D::Error> {
        deserializer.deserialize_any(NaiveVisitor)
    }

    /// Normalized serialization for `Option<NaiveDateTime>` fields.
    pub mod option {
        use super::*;

        pub fn serialize<S: Serializer>(
            value: &Option<NaiveDateTime>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            match value {
                Some(dt) => serialize_normalized(*dt, serializer),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<NaiveDateTime>, D::Error> {
            deserializer.deserialize_any(OptionNaiveVisitor)
        }
    }
}

// =========================================================================
// Visitors
// =========================================================================

struct UtcVisitor;

impl<'de> serde::de::Visitor<'de> for UtcVisitor {
    type Value = DateTime<Utc>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "an RFC 3339 timestamp string")
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<DateTime<Utc>, E> {
        // Accept both "...Z" and naive formats (treat as UTC)
        if let Ok(dt) = DateTime::parse_from_rfc3339(v) {
            return Ok(dt.with_timezone(&Utc));
        }
        if let Ok(ndt) = NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M:%S%.fZ") {
            return Ok(ndt.and_utc());
        }
        if let Ok(ndt) = NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M:%S%.f") {
            return Ok(ndt.and_utc());
        }
        if let Ok(ndt) = NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M:%S") {
            return Ok(ndt.and_utc());
        }
        Err(E::custom(format!("invalid timestamp: {v}")))
    }
}

struct OptionUtcVisitor;

impl<'de> serde::de::Visitor<'de> for OptionUtcVisitor {
    type Value = Option<DateTime<Utc>>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "an RFC 3339 timestamp string or null")
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Option<DateTime<Utc>>, E> {
        Ok(None)
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Option<DateTime<Utc>>, E> {
        Ok(None)
    }

    fn visit_some<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Option<DateTime<Utc>>, D::Error> {
        deserializer.deserialize_any(UtcVisitor).map(Some)
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Option<DateTime<Utc>>, E> {
        UtcVisitor.visit_str(v).map(Some)
    }
}

struct NaiveVisitor;

impl<'de> serde::de::Visitor<'de> for NaiveVisitor {
    type Value = NaiveDateTime;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a datetime string")
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<NaiveDateTime, E> {
        // Strip trailing Z if present (we store as NaiveDateTime)
        let s = v.trim_end_matches('Z');
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
            return Ok(ndt);
        }
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
            return Ok(ndt);
        }
        Err(E::custom(format!("invalid datetime: {v}")))
    }
}

struct OptionNaiveVisitor;

impl<'de> serde::de::Visitor<'de> for OptionNaiveVisitor {
    type Value = Option<NaiveDateTime>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a datetime string or null")
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Option<NaiveDateTime>, E> {
        Ok(None)
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Option<NaiveDateTime>, E> {
        Ok(None)
    }

    fn visit_some<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Option<NaiveDateTime>, D::Error> {
        deserializer.deserialize_any(NaiveVisitor).map(Some)
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Option<NaiveDateTime>, E> {
        NaiveVisitor.visit_str(v).map(Some)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    struct TestUtc {
        #[serde(with = "super::utc")]
        ts: DateTime<Utc>,
        #[serde(default, with = "super::utc::option")]
        opt_ts: Option<DateTime<Utc>>,
    }

    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    struct TestNaive {
        #[serde(with = "super::naive")]
        ts: NaiveDateTime,
        #[serde(default, with = "super::naive::option")]
        opt_ts: Option<NaiveDateTime>,
    }

    #[test]
    fn test_utc_truncates_nanoseconds() {
        let dt = NaiveDate::from_ymd_opt(2024, 1, 15)
            .unwrap()
            .and_hms_nano_opt(8, 30, 45, 123_456_789)
            .unwrap()
            .and_utc();
        let s = TestUtc {
            ts: dt,
            opt_ts: Some(dt),
        };
        let json = serde_json::to_string(&s).unwrap();
        // Should truncate to microseconds: 123456, not 123456789
        assert!(json.contains(".123456Z"), "got: {json}");
        assert!(!json.contains("789"), "nanoseconds leaked: {json}");
    }

    #[test]
    fn test_utc_omits_zero_micros() {
        let dt = NaiveDate::from_ymd_opt(2024, 1, 15)
            .unwrap()
            .and_hms_opt(8, 30, 45)
            .unwrap()
            .and_utc();
        let s = TestUtc {
            ts: dt,
            opt_ts: None,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("08:30:45Z"), "got: {json}");
        assert!(!json.contains(".000"), "unnecessary decimals: {json}");
    }

    #[test]
    fn test_naive_gets_z_suffix() {
        let dt = NaiveDate::from_ymd_opt(2024, 3, 10)
            .unwrap()
            .and_hms_nano_opt(14, 0, 0, 500_000_000)
            .unwrap();
        let s = TestNaive {
            ts: dt,
            opt_ts: Some(dt),
        };
        let json = serde_json::to_string(&s).unwrap();
        // NaiveDateTime should also get Z suffix
        assert!(json.contains(".500000Z"), "got: {json}");
    }

    #[test]
    fn test_deserialize_both_formats() {
        // With Z
        let json = r#"{"ts":"2024-01-15T08:30:45.123456Z"}"#;
        let v: TestUtc = serde_json::from_str(json).unwrap();
        assert_eq!(v.ts.nanosecond(), 123_456_000);

        // Without Z (accept as UTC)
        let json = r#"{"ts":"2024-01-15T08:30:45.123456"}"#;
        let v: TestUtc = serde_json::from_str(json).unwrap();
        assert_eq!(v.ts.nanosecond(), 123_456_000);
    }

    #[test]
    fn test_naive_deserialize_strips_z() {
        let json = r#"{"ts":"2024-01-15T08:30:45.123456Z"}"#;
        let v: TestNaive = serde_json::from_str(json).unwrap();
        assert_eq!(v.ts.nanosecond(), 123_456_000);
    }

    #[test]
    fn test_option_null() {
        let json = r#"{"ts":"2024-01-15T08:30:45Z","opt_ts":null}"#;
        let v: TestUtc = serde_json::from_str(json).unwrap();
        assert!(v.opt_ts.is_none());
    }
}

//! Configurable decimal serialization.
//!
//! By default, `Decimal` values serialize as JSON strings (e.g. `"1729237.30"`)
//! to avoid IEEE 754 precision loss. When `set_numeric_native(true)` is called,
//! they serialize as JSON numbers (e.g. `1729237.30`).
//!
//! Usage on struct fields:
//! ```ignore
//! #[serde(with = "datasynth_core::serde_decimal")]
//! pub amount: Decimal,
//!
//! #[serde(default, with = "datasynth_core::serde_decimal::option")]
//! pub tax: Option<Decimal>,
//! ```

use std::cell::Cell;
use std::fmt;

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{self, Deserializer, Serializer};

thread_local! {
    static NUMERIC_NATIVE: Cell<bool> = const { Cell::new(false) };
}

/// Set whether decimals serialize as JSON numbers (`true`) or strings (`false`).
///
/// This is a thread-local setting. Call it before serialization begins.
/// If serialization runs on rayon worker threads, each worker must set this independently.
pub fn set_numeric_native(native: bool) {
    NUMERIC_NATIVE.with(|c| c.set(native));
}

/// Returns whether native numeric mode is active.
pub fn is_numeric_native() -> bool {
    NUMERIC_NATIVE.with(|c| c.get())
}

/// Serialize a `Decimal`. Mode-aware: string or f64.
pub fn serialize<S: Serializer>(value: &Decimal, serializer: S) -> Result<S::Ok, S::Error> {
    if is_numeric_native() {
        match value.to_f64() {
            Some(f) => serializer.serialize_f64(f),
            None => serializer.serialize_str(&value.to_string()),
        }
    } else {
        rust_decimal::serde::str::serialize(value, serializer)
    }
}

/// Deserialize a `Decimal`. Accepts both string and number inputs.
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Decimal, D::Error> {
    deserializer.deserialize_any(DecimalVisitor)
}

/// Configurable serialization for `Option<Decimal>` fields.
pub mod option {
    use rust_decimal::prelude::ToPrimitive;
    use rust_decimal::Decimal;
    use serde::{Deserializer, Serializer};

    use super::{is_numeric_native, OptionDecimalVisitor};

    /// Serialize an `Option<Decimal>`. Mode-aware: string or f64.
    pub fn serialize<S: Serializer>(
        value: &Option<Decimal>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(d) => {
                if is_numeric_native() {
                    match d.to_f64() {
                        Some(f) => serializer.serialize_f64(f),
                        None => serializer.serialize_str(&d.to_string()),
                    }
                } else {
                    rust_decimal::serde::str_option::serialize(value, serializer)
                }
            }
            None => serializer.serialize_none(),
        }
    }

    /// Deserialize an `Option<Decimal>`. Accepts string, number, or null.
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Decimal>, D::Error> {
        deserializer.deserialize_any(OptionDecimalVisitor)
    }
}

// -- Visitors --

struct DecimalVisitor;

impl<'de> serde::de::Visitor<'de> for DecimalVisitor {
    type Value = Decimal;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a decimal as a string or number")
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Decimal, E> {
        v.parse::<Decimal>().map_err(E::custom)
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Decimal, E> {
        Decimal::try_from(v).map_err(E::custom)
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Decimal, E> {
        Ok(Decimal::from(v))
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Decimal, E> {
        Ok(Decimal::from(v))
    }
}

struct OptionDecimalVisitor;

impl<'de> serde::de::Visitor<'de> for OptionDecimalVisitor {
    type Value = Option<Decimal>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a decimal as a string or number, or null")
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Option<Decimal>, E> {
        Ok(None)
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Option<Decimal>, E> {
        Ok(None)
    }

    fn visit_some<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Option<Decimal>, D::Error> {
        deserializer.deserialize_any(DecimalVisitor).map(Some)
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Option<Decimal>, E> {
        v.parse::<Decimal>().map(Some).map_err(E::custom)
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Option<Decimal>, E> {
        Decimal::try_from(v).map(Some).map_err(E::custom)
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Option<Decimal>, E> {
        Ok(Some(Decimal::from(v)))
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Option<Decimal>, E> {
        Ok(Some(Decimal::from(v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
    struct TestStruct {
        #[serde(with = "super")]
        amount: Decimal,
        #[serde(default, with = "super::option")]
        tax: Option<Decimal>,
    }

    #[test]
    fn test_string_mode() {
        set_numeric_native(false);
        let s = TestStruct {
            amount: dec!(1729237.30),
            tax: Some(dec!(99.95)),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"1729237.30\""), "expected string: {json}");
        assert!(json.contains("\"99.95\""), "expected string: {json}");
    }

    #[test]
    fn test_native_mode() {
        set_numeric_native(true);
        let s = TestStruct {
            amount: dec!(1729237.30),
            tax: Some(dec!(99.95)),
        };
        let json = serde_json::to_string(&s).unwrap();
        // In native mode, numbers should not be quoted
        assert!(
            json.contains(":1729237.3") || json.contains(":1729237.30"),
            "expected number: {json}"
        );
        // Reset for other tests
        set_numeric_native(false);
    }

    #[test]
    fn test_deserialize_from_string() {
        let json = r#"{"amount":"1729237.30","tax":"99.95"}"#;
        let s: TestStruct = serde_json::from_str(json).unwrap();
        assert_eq!(s.amount, dec!(1729237.30));
        assert_eq!(s.tax, Some(dec!(99.95)));
    }

    #[test]
    fn test_deserialize_from_number() {
        let json = r#"{"amount":1729237.30,"tax":99.95}"#;
        let s: TestStruct = serde_json::from_str(json).unwrap();
        // f64 precision: 1729237.30 round-trips fine at this magnitude
        assert_eq!(s.amount, dec!(1729237.3));
        assert_eq!(s.tax, Some(dec!(99.95)));
    }

    #[test]
    fn test_deserialize_null_option() {
        let json = r#"{"amount":"100.00","tax":null}"#;
        let s: TestStruct = serde_json::from_str(json).unwrap();
        assert_eq!(s.tax, None);
    }

    #[test]
    fn test_deserialize_missing_option() {
        let json = r#"{"amount":"100.00"}"#;
        let s: TestStruct = serde_json::from_str(json).unwrap();
        assert_eq!(s.tax, None);
    }
}

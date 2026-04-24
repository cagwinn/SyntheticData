use datasynth_core::models::IcPairId;

#[test]
fn test_from_bytes_and_back() {
    let bytes = [1u8; 32];
    let id = IcPairId::from_bytes(bytes);
    assert_eq!(*id.as_bytes(), bytes);
}

#[test]
fn test_hex_roundtrip_lowercase() {
    let bytes = [
        0xab, 0xcd, 0xef, 0x12, 0x34, 0x56, 0x78, 0x90, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
        0x07, 0x08,
    ];
    let id = IcPairId::from_bytes(bytes);
    let hex = id.to_hex();
    assert_eq!(hex.len(), 64);
    assert!(hex
        .chars()
        .all(|c| c.is_ascii_hexdigit() && (!c.is_alphabetic() || c.is_lowercase())));
    let parsed = IcPairId::from_hex(&hex).expect("roundtrip");
    assert_eq!(id, parsed);
}

#[test]
fn test_hex_accepts_uppercase_on_parse() {
    let hex_upper = "ABCDEF1234567890".repeat(4);
    let id = IcPairId::from_hex(&hex_upper).unwrap();
    // Serialization is always lowercase regardless of parse casing.
    assert_eq!(id.to_hex(), hex_upper.to_lowercase());
}

#[test]
fn test_wrong_length_rejected() {
    let err = IcPairId::from_hex("abcd").unwrap_err();
    assert!(err.to_string().contains("64"));
}

#[test]
fn test_invalid_digit_rejected() {
    let mut bad = "ab".repeat(32);
    bad.replace_range(10..11, "Z");
    let err = IcPairId::from_hex(&bad).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("invalid"));
}

#[test]
fn test_serde_roundtrip() {
    let bytes: [u8; 32] = [7u8; 32];
    let id = IcPairId::from_bytes(bytes);
    let json = serde_json::to_string(&id).unwrap();
    // Should be a quoted lowercase hex string.
    assert_eq!(json, format!(r#""{}""#, id.to_hex()));
    let back: IcPairId = serde_json::from_str(&json).unwrap();
    assert_eq!(id, back);
}

#[test]
fn test_equality_and_ordering() {
    let a = IcPairId::from_bytes([1u8; 32]);
    let b = IcPairId::from_bytes([2u8; 32]);
    assert_ne!(a, b);
    assert!(a < b);
}

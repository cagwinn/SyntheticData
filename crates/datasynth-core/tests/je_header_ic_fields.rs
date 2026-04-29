use datasynth_core::models::{journal_entry::JournalEntryHeader, IcPairId};

#[test]
fn test_je_header_ic_fields_default_none() {
    let h = JournalEntryHeader::new(
        "C001".to_string(),
        chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
    );
    assert!(h.ic_pair_id.is_none());
    assert!(h.ic_partner_entity.is_none());
}

#[test]
fn test_je_header_ic_fields_settable() {
    let mut h = JournalEntryHeader::new(
        "C001".to_string(),
        chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
    );
    h.ic_pair_id = Some(IcPairId::from_bytes([5u8; 32]));
    h.ic_partner_entity = Some("C002".to_string());
    assert_eq!(h.ic_partner_entity.as_deref(), Some("C002"));
    assert_eq!(*h.ic_pair_id.unwrap().as_bytes(), [5u8; 32]);
}

#[test]
fn test_je_header_ic_fields_skipped_when_none_in_json() {
    let h = JournalEntryHeader::new(
        "C001".to_string(),
        chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
    );
    let json = serde_json::to_string(&h).unwrap();
    // The `skip_serializing_if = "Option::is_none"` on both fields keeps legacy JSON compact.
    assert!(!json.contains("ic_pair_id"));
    assert!(!json.contains("ic_partner_entity"));
}

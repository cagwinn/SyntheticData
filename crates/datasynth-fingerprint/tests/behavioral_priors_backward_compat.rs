//! Backward-compat: pre-SP2 .dsf files (without `behavioral` field) must still
//! deserialise into the new Fingerprint struct, with `behavioral: None`.

use datasynth_fingerprint::models::{
    Fingerprint, Manifest, PrivacyAudit, PrivacyLevel, PrivacyMetadata, SchemaFingerprint,
    SourceMetadata, StatisticsFingerprint,
};

/// Build a minimal Fingerprint with no behavioral field, serialise to JSON, strip the
/// "behavioral" key to simulate a pre-SP2 .dsf, then deserialise and assert `behavioral`
/// is `None`.
#[test]
fn old_dsf_without_behavioral_field_loads_cleanly() {
    // --- build a minimal but fully-valid Fingerprint programmatically ---
    let source = SourceMetadata::new("pre-sp2-fixture", vec!["journal_entries".into()], 1000);
    let privacy = PrivacyMetadata::from_level(PrivacyLevel::Standard);
    let manifest = Manifest::new(source, privacy);
    let schema = SchemaFingerprint::new();
    let statistics = StatisticsFingerprint::new();
    let privacy_audit = PrivacyAudit::new(1.0, 5);

    let fp = Fingerprint::new(manifest, schema, statistics, privacy_audit);

    // Sanity-check: behavioral is None on a freshly-constructed Fingerprint.
    assert!(
        fp.behavioral.is_none(),
        "Fingerprint::new must not populate behavioral"
    );

    // --- serialise to JSON, then remove the "behavioral" key (pre-SP2 simulation) ---
    let json = serde_json::to_string(&fp).expect("serialize");

    // Verify the key is absent in the serialised form (skip_serializing_if = None).
    // If it IS present (future regression), the strip-then-deserialise path still works.
    let mut value: serde_json::Value = serde_json::from_str(&json).expect("parse json");
    if let Some(obj) = value.as_object_mut() {
        obj.remove("behavioral");
    }
    let stripped = serde_json::to_string(&value).expect("re-serialize");

    // --- deserialise: must succeed and behavioral must be None ---
    let loaded: Fingerprint =
        serde_json::from_str(&stripped).expect("pre-SP2 .dsf must deserialise without error");

    assert!(
        loaded.behavioral.is_none(),
        "a .dsf without the 'behavioral' key must deserialise with behavioral: None"
    );
}

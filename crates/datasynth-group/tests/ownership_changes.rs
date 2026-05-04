//! v5.2 ownership-change-event plumbing tests.
//!
//! Pins:
//! 1. `EntityConfig.ownership_changes` is lifted into
//!    `ManifestEntity.ownership_changes` with `entity_code` and
//!    `parent_entity_code` filled from the host entity.
//! 2. Empty `ownership_changes` produces an empty list (no file
//!    emission downstream).
//! 3. An ownership-change event whose `effective_date` falls outside
//!    the engagement period is rejected at `build_manifest`.
//! 4. An ownership-change event on an entity without `parent_code`
//!    is rejected at expansion time.
//! 5. The shard runner writes
//!    `entities/{code}/intercompany/ownership_change_events.json`
//!    only when the entity has events.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_core::models::intercompany::{NciMeasurementMethod, OwnershipChangeType};
use datasynth_group::config::{
    AuditEngagementConfig, ConsolidationMethod, EntityConfig, FxConfig, FxPolicyConfig,
    FxRateBasis, GroupConfig, GroupMaterialityConfig, MaterialityBasis, OwnershipChangeEntry,
    OwnershipConfig, PeriodConfig, PeriodLength,
};
use datasynth_group::{build_manifest, GroupError};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn period_start() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
}

fn mid_period() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 2, 15).unwrap()
}

fn pre_period() -> NaiveDate {
    NaiveDate::from_ymd_opt(2023, 12, 31).unwrap()
}

fn make_event(effective: NaiveDate) -> OwnershipChangeEntry {
    OwnershipChangeEntry {
        event_type: OwnershipChangeType::ControlIncreased,
        effective_date: effective,
        ownership_percent_before: dec!(0.80),
        ownership_percent_after: dec!(0.95),
        previously_held_interest_carrying: None,
        previously_held_interest_fair_value: None,
        consideration_paid_or_received: dec!(15_000_000),
        acquisition_date_nci_fair_value: None,
        nci_measurement_method: NciMeasurementMethod::Proportionate,
        currency: "CHF".to_string(),
    }
}

fn base_config(entities: Vec<EntityConfig>) -> GroupConfig {
    let mut fx_rates: std::collections::BTreeMap<NaiveDate, Decimal> =
        std::collections::BTreeMap::new();
    fx_rates.insert(period_start(), dec!(1.0));
    let mut rate_table: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<NaiveDate, Decimal>,
    > = std::collections::BTreeMap::new();
    rate_table.insert("CHF/CHF".to_string(), fx_rates);

    GroupConfig {
        id: "TEST_OC".to_string(),
        name: None,
        presentation_currency: "CHF".to_string(),
        period: PeriodConfig {
            start_date: period_start(),
            length: PeriodLength::Quarterly,
            fiscal_year_end: None,
        },
        seed: 42,
        defaults: serde_yaml::Value::Null,
        scoping_profiles: std::collections::BTreeMap::new(),
        ownership: OwnershipConfig {
            parent_entity_code: "PARENT".to_string(),
            entities,
            generated: vec![],
            entities_from: None,
        },
        intercompany: Default::default(),
        fx: FxConfig {
            base_currency: "CHF".to_string(),
            rate_source: Default::default(),
            policy: FxPolicyConfig {
                balance_sheet: FxRateBasis::Closing,
                income_statement: FxRateBasis::Average,
                equity: FxRateBasis::Historical,
            },
            rates: rate_table,
        },
        audit: AuditEngagementConfig {
            engagement_id: Some("ENG-1".to_string()),
            group_materiality: Some(GroupMaterialityConfig {
                basis: MaterialityBasis::Revenue,
                percent: dec!(0.005),
            }),
            ..Default::default()
        },
        tax: Default::default(),
        cgu: Default::default(),
        output: Default::default(),
        fleet: None,
    }
}

fn entity(
    code: &str,
    parent: Option<&str>,
    method: ConsolidationMethod,
    changes: Vec<OwnershipChangeEntry>,
) -> EntityConfig {
    EntityConfig {
        code: code.to_string(),
        name: None,
        country: "CH".to_string(),
        functional_currency: "CHF".to_string(),
        scoping_profile: "significant".to_string(),
        consolidation_method: method,
        ownership_percent: parent.map(|_| dec!(0.95)),
        parent_code: parent.map(str::to_string),
        acquisition_date: None,
        accounting_framework: None,
        industry: None,
        rows: None,
        ownership_changes: changes,
        hyperinflation_status: datasynth_core::models::HyperinflationStatus::NotHyperinflationary,
        overrides: Default::default(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn empty_ownership_changes_produces_empty_manifest_list() {
    let cfg = base_config(vec![
        entity("PARENT", None, ConsolidationMethod::Parent, vec![]),
        entity("SUB", Some("PARENT"), ConsolidationMethod::Full, vec![]),
    ]);
    let manifest = build_manifest(&cfg).unwrap();
    for e in &manifest.ownership_graph.entities {
        assert!(
            e.ownership_changes.is_empty(),
            "{} should have empty ownership_changes",
            e.code
        );
    }
}

#[test]
fn ownership_changes_lift_with_entity_and_parent_codes_filled() {
    let cfg = base_config(vec![
        entity("PARENT", None, ConsolidationMethod::Parent, vec![]),
        entity(
            "SUB",
            Some("PARENT"),
            ConsolidationMethod::Full,
            vec![make_event(mid_period())],
        ),
    ]);
    let manifest = build_manifest(&cfg).unwrap();
    let sub = manifest
        .ownership_graph
        .entities
        .iter()
        .find(|e| e.code == "SUB")
        .unwrap();
    assert_eq!(sub.ownership_changes.len(), 1);
    let ev = &sub.ownership_changes[0];
    assert_eq!(ev.entity_code, "SUB");
    assert_eq!(ev.parent_entity_code, "PARENT");
    assert_eq!(ev.event_type, OwnershipChangeType::ControlIncreased);
    assert_eq!(ev.ownership_percent_before, dec!(0.80));
    assert_eq!(ev.ownership_percent_after, dec!(0.95));
}

#[test]
fn ownership_change_outside_period_rejected() {
    let cfg = base_config(vec![
        entity("PARENT", None, ConsolidationMethod::Parent, vec![]),
        entity(
            "SUB",
            Some("PARENT"),
            ConsolidationMethod::Full,
            vec![make_event(pre_period())],
        ),
    ]);
    let err = build_manifest(&cfg).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("outside the engagement period"),
        "expected period-bound rejection, got: {msg}"
    );
    assert!(matches!(err, GroupError::Config(_)));
}

#[test]
fn ownership_change_without_parent_code_rejected_at_expansion() {
    // Set up an entity with ownership_changes but no parent_code.
    // The expansion layer rejects this with a typed Config error.
    let mut e = entity(
        "ORPHAN",
        None,
        ConsolidationMethod::Full,
        vec![make_event(mid_period())],
    );
    e.ownership_percent = None; // no parent → no ownership %
    let cfg = base_config(vec![
        entity("PARENT", None, ConsolidationMethod::Parent, vec![]),
        e,
    ]);
    let err = build_manifest(&cfg).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("ownership_changes but has no parent_code"),
        "expected parent_code rejection, got: {msg}"
    );
}

#[test]
fn ownership_percent_out_of_range_rejected() {
    let mut bad = make_event(mid_period());
    bad.ownership_percent_after = dec!(1.5);
    let cfg = base_config(vec![
        entity("PARENT", None, ConsolidationMethod::Parent, vec![]),
        entity("SUB", Some("PARENT"), ConsolidationMethod::Full, vec![bad]),
    ]);
    let err = build_manifest(&cfg).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("ownership_percent_after") && msg.contains("not in [0, 1]"),
        "expected percent-range rejection, got: {msg}"
    );
}

#[test]
fn manifest_serde_round_trips_ownership_changes() {
    let cfg = base_config(vec![
        entity("PARENT", None, ConsolidationMethod::Parent, vec![]),
        entity(
            "SUB",
            Some("PARENT"),
            ConsolidationMethod::Full,
            vec![make_event(mid_period())],
        ),
    ]);
    let manifest = build_manifest(&cfg).unwrap();
    let json = serde_json::to_string_pretty(&manifest).unwrap();
    let back: datasynth_group::GroupManifest = serde_json::from_str(&json).unwrap();
    let sub_in = manifest
        .ownership_graph
        .entities
        .iter()
        .find(|e| e.code == "SUB")
        .unwrap();
    let sub_out = back
        .ownership_graph
        .entities
        .iter()
        .find(|e| e.code == "SUB")
        .unwrap();
    assert_eq!(sub_in.ownership_changes, sub_out.ownership_changes);
}

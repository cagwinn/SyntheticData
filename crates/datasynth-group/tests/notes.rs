//! Task 8.6 — notes to consolidated FS integration tests.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use datasynth_group::aggregate::ic_matcher::{IcMatchResult, UnmatchedSide};
use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::{
    build_coverage_report, build_manifest, build_notes_to_consolidated_fs, CtaRollforward,
    EquityMethodInvestment, GroupConfig, IcRelationshipConfig, NciRollforward, NotesInputs,
};

use datasynth_standards::framework::AccountingFramework;

fn period_end() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
}

/// Build a trimmed two-entity manifest from `mini_nestle.yaml`.
fn load_two_entity_manifest() -> GroupManifest {
    let yaml = include_str!("fixtures/mini_nestle.yaml");
    let mut cfg: GroupConfig =
        serde_yaml::from_str(yaml).expect("mini_nestle.yaml must parse");

    cfg.ownership
        .entities
        .retain(|e| e.code == "NESTLE_SA" || e.code == "NESTLE_USA");
    cfg.intercompany.relationships.retain(|rel| match rel {
        IcRelationshipConfig::Explicit(e) => {
            (e.seller == "NESTLE_SA" || e.seller == "NESTLE_USA")
                && (e.buyer == "NESTLE_SA" || e.buyer == "NESTLE_USA")
        }
        IcRelationshipConfig::Pattern(_) => true,
    });
    if let Some(p2) = cfg.tax.pillar_two.as_mut() {
        p2.jurisdictions.retain(|j| j == "CH" || j == "US");
    }
    if let Some(tp) = cfg.tax.transfer_pricing.as_mut() {
        tp.local_files_for.retain(|j| j == "CH" || j == "US");
    }
    build_manifest(&cfg).expect("trimmed manifest must build")
}

fn empty_coverage() -> datasynth_group::CoverageReport {
    let result = IcMatchResult {
        matched: Vec::new(),
        unmatched: Vec::<UnmatchedSide>::new(),
        total_planned: 0,
        coverage: 0.0,
    };
    build_coverage_report(&result)
}

fn nci_rf(entity: &str, opening: Decimal, share: Decimal, closing: Decimal) -> NciRollforward {
    NciRollforward {
        entity_code: entity.to_string(),
        parent_entity_code: "NESTLE_SA".to_string(),
        ownership_percent: dec!(0.80),
        nci_percent: dec!(0.20),
        opening_nci: opening,
        nci_share_of_profit: share,
        nci_share_of_oci: Decimal::ZERO,
        nci_dividends: Decimal::ZERO,
        closing_nci: closing,
        period_end: period_end(),
        currency: "CHF".to_string(),
    }
}

fn cta_rf(entity: &str, opening: Decimal, period: Decimal) -> CtaRollforward {
    CtaRollforward {
        entity_code: entity.to_string(),
        functional_currency: "USD".to_string(),
        presentation_currency: "CHF".to_string(),
        opening_cta: opening,
        period_cta: period,
        closing_cta: opening + period,
    }
}

fn em_inv(investee: &str, closing: Decimal, share: Decimal) -> EquityMethodInvestment {
    EquityMethodInvestment {
        investee_code: investee.to_string(),
        investor_entity_code: "NESTLE_SA".to_string(),
        ownership_percent: dec!(0.30),
        opening_carrying_value: Decimal::ZERO,
        share_of_profit: share,
        dividends_received: Decimal::ZERO,
        impairment: Decimal::ZERO,
        closing_carrying_value: closing,
        period_end: period_end(),
        currency: "CHF".to_string(),
    }
}

#[test]
fn always_emits_8_notes() {
    let manifest = load_two_entity_manifest();
    let coverage = empty_coverage();
    let inputs = NotesInputs {
        manifest: &manifest,
        framework: AccountingFramework::Ifrs,
        ic_coverage: &coverage,
        nci_rollforwards: &[],
        cta_rollforwards: &[],
        equity_method_investments: &[],
    };

    let notes = build_notes_to_consolidated_fs(&inputs, period_end());
    assert_eq!(notes.notes.len(), 8, "must always emit 8 notes");

    let numbers: Vec<u32> = notes.notes.iter().map(|n| n.note_number).collect();
    assert_eq!(numbers, vec![1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn nci_summary_lists_each_rollforward() {
    let manifest = load_two_entity_manifest();
    let coverage = empty_coverage();
    let nci = vec![
        nci_rf("SUB1", dec!(1000), dec!(200), dec!(1200)),
        nci_rf("SUB2", dec!(500), dec!(100), dec!(600)),
    ];
    let inputs = NotesInputs {
        manifest: &manifest,
        framework: AccountingFramework::Ifrs,
        ic_coverage: &coverage,
        nci_rollforwards: &nci,
        cta_rollforwards: &[],
        equity_method_investments: &[],
    };
    let notes = build_notes_to_consolidated_fs(&inputs, period_end());
    let nci_note = &notes.notes[3]; // note 4
    assert_eq!(nci_note.note_number, 4);
    assert!(
        nci_note.body.contains("SUB1"),
        "NCI note must list SUB1: {}",
        nci_note.body,
    );
    assert!(
        nci_note.body.contains("SUB2"),
        "NCI note must list SUB2: {}",
        nci_note.body,
    );
    assert!(nci_note.body.contains("1200"));
    assert!(nci_note.body.contains("600"));
}

#[test]
fn cta_summary_lists_each_rollforward() {
    let manifest = load_two_entity_manifest();
    let coverage = empty_coverage();
    let cta = vec![
        cta_rf("ENT_USA", Decimal::ZERO, dec!(1234)),
        cta_rf("ENT_BR", dec!(500), dec!(-250)),
    ];
    let inputs = NotesInputs {
        manifest: &manifest,
        framework: AccountingFramework::Ifrs,
        ic_coverage: &coverage,
        nci_rollforwards: &[],
        cta_rollforwards: &cta,
        equity_method_investments: &[],
    };
    let notes = build_notes_to_consolidated_fs(&inputs, period_end());
    let cta_note = &notes.notes[4]; // note 5
    assert_eq!(cta_note.note_number, 5);
    assert!(
        cta_note.body.contains("ENT_USA"),
        "CTA note must list ENT_USA"
    );
    assert!(
        cta_note.body.contains("ENT_BR"),
        "CTA note must list ENT_BR"
    );
    assert!(cta_note.body.contains("1234"));
}

#[test]
fn determinism_two_calls_match() {
    let manifest = load_two_entity_manifest();
    let coverage = empty_coverage();
    let nci = vec![nci_rf("SUB", dec!(1000), dec!(200), dec!(1200))];
    let cta = vec![cta_rf("ENT", Decimal::ZERO, dec!(500))];
    let em = vec![em_inv("ASSOC", dec!(50_000), dec!(10_000))];

    let make_notes = || {
        let inputs = NotesInputs {
            manifest: &manifest,
            framework: AccountingFramework::Ifrs,
            ic_coverage: &coverage,
            nci_rollforwards: &nci,
            cta_rollforwards: &cta,
            equity_method_investments: &em,
        };
        build_notes_to_consolidated_fs(&inputs, period_end())
    };
    let a = make_notes();
    let b = make_notes();
    assert_eq!(a, b, "two calls must produce equal notes");

    // And byte-identical JSON.
    let json_a = serde_json::to_string_pretty(&a).unwrap();
    let json_b = serde_json::to_string_pretty(&b).unwrap();
    assert_eq!(json_a, json_b, "JSON must be byte-identical");

    // Silence unused for placeholder vars.
    let _ = BTreeMap::<String, Decimal>::new();
}

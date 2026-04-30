//! Notes to the consolidated financial statements — Task 8.6.
//!
//! Emits a basic 8-note set that auditors and statutory preparers
//! expect alongside the consolidated FS.  Each note is template-
//! assembled from manifest + coverage + NCI / CTA / equity-method
//! inputs so two runs with the same inputs produce byte-identical
//! note bodies (verified by the determinism test).
//!
//! # Note set (v5.0)
//!
//! 1. **Significant accounting policies** — derived from
//!    [`AccountingFramework`] and the canonical IFRS / ASC standards
//!    it points to.
//! 2. **Basis of consolidation** — names the IFRS 10 / ASC 810
//!    reference, lists fully-consolidated entities and equity-method
//!    investees from the manifest.
//! 3. **IC eliminations summary** — total pairs planned, matched,
//!    coverage % from the [`CoverageReport`].
//! 4. **NCI summary** — per-subsidiary opening / closing NCI from
//!    the rollforwards (IFRS 12.10 disclosure).
//! 5. **CTA summary** — per-entity period CTA + closing CTA from the
//!    rollforwards (IAS 21.39 OCI accumulation).
//! 6. **Operating segments** — placeholder ("deferred to v5.1").
//! 7. **Subsequent events** — placeholder ("none identified").
//! 8. **Related parties** — placeholder pointing at the manifest's IC
//!    relationships (full IAS 24 treatment in v5.1).
//!
//! # v5.1 deferrals
//!
//! - Per-component segment breakdown (note 6).
//! - Auto-derived subsequent events from the manifest period (note 7).
//! - Full IAS 24 related-party disclosure including KMP, transactions
//!   with affiliates outside the group, etc. (note 8).

use chrono::NaiveDate;
use datasynth_standards::framework::AccountingFramework;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::aggregate::coverage_report::CoverageReport;
use crate::aggregate::equity_method::EquityMethodInvestment;
use crate::aggregate::nci::NciRollforward;
use crate::aggregate::translation::CtaRollforward;
use crate::config::ConsolidationMethod;
use crate::manifest::GroupManifest;

// ── Public types ──────────────────────────────────────────────────────────────

/// Notes to the consolidated financial statements.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotesToConsolidatedFs {
    /// Group identifier.
    pub group_id: String,
    /// Reporting date.
    pub period_end: NaiveDate,
    /// Accounting framework label (e.g., "IFRS").
    pub framework: String,
    /// The note bodies, in note-number order.
    pub notes: Vec<Note>,
}

/// One note in the disclosure set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Note {
    /// Sequential note number (1..=N).
    pub note_number: u32,
    /// Note title (e.g., "Significant accounting policies").
    pub title: String,
    /// Plain-text / lightweight markdown body.
    pub body: String,
}

/// Inputs for [`build_notes_to_consolidated_fs`].
pub struct NotesInputs<'a> {
    /// Group manifest — used to enumerate entities and pull the
    /// presentation currency / period.
    pub manifest: &'a GroupManifest,
    /// Accounting framework for the standards-policies note.
    pub framework: AccountingFramework,
    /// IC matching coverage report — feeds note 3.
    pub ic_coverage: &'a CoverageReport,
    /// NCI rollforwards — feed note 4.
    pub nci_rollforwards: &'a [NciRollforward],
    /// CTA rollforwards — feed note 5.
    pub cta_rollforwards: &'a [CtaRollforward],
    /// Equity-method investments — feed note 2 (basis of consolidation
    /// list of equity-method investees).
    pub equity_method_investments: &'a [EquityMethodInvestment],
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Build the [`NotesToConsolidatedFs`] from `inputs`.
///
/// Pure function: every body is template-assembled from the inputs so
/// two calls with the same inputs produce equal records (and
/// byte-identical JSON output).
pub fn build_notes_to_consolidated_fs(
    inputs: &NotesInputs,
    period_end: NaiveDate,
) -> NotesToConsolidatedFs {
    let mut notes: Vec<Note> = Vec::with_capacity(8);

    // ── Note 1: Significant accounting policies ────────────────────────────
    notes.push(Note {
        note_number: 1,
        title: "Significant accounting policies".to_string(),
        body: format!(
            "These consolidated financial statements have been prepared in \
             accordance with {framework}. Key standards applied: revenue \
             recognition under {revenue}, lease accounting under {leases}, \
             fair value measurement under {fair_value}, and impairment \
             under {impairment}. The financial statements are presented in \
             {currency}.",
            framework = inputs.framework,
            revenue = inputs.framework.revenue_standard(),
            leases = inputs.framework.lease_standard(),
            fair_value = inputs.framework.fair_value_standard(),
            impairment = inputs.framework.impairment_standard(),
            currency = inputs.manifest.presentation_currency,
        ),
    });

    // ── Note 2: Basis of consolidation ─────────────────────────────────────
    let full_consolidated: Vec<&str> = inputs
        .manifest
        .ownership_graph
        .entities
        .iter()
        .filter(|e| {
            matches!(
                e.consolidation_method,
                ConsolidationMethod::Parent | ConsolidationMethod::Full
            )
        })
        .map(|e| e.code.as_str())
        .collect();
    let equity_method_entities: Vec<&str> = inputs
        .manifest
        .ownership_graph
        .entities
        .iter()
        .filter(|e| e.consolidation_method == ConsolidationMethod::EquityMethod)
        .map(|e| e.code.as_str())
        .collect();
    notes.push(Note {
        note_number: 2,
        title: "Basis of consolidation".to_string(),
        body: format!(
            "These consolidated financial statements include the parent \
             entity and all subsidiaries over which the group has control \
             (IFRS 10 / ASC 810). Fully-consolidated entities ({full_count}): \
             {full_list}. Equity-method investees ({eq_count}): {eq_list}. \
             Intercompany transactions and balances have been eliminated on \
             consolidation.",
            full_count = full_consolidated.len(),
            full_list = if full_consolidated.is_empty() {
                "none".to_string()
            } else {
                full_consolidated.join(", ")
            },
            eq_count = equity_method_entities.len(),
            eq_list = if equity_method_entities.is_empty() {
                "none".to_string()
            } else {
                equity_method_entities.join(", ")
            },
        ),
    });

    // ── Note 3: IC eliminations summary ────────────────────────────────────
    notes.push(Note {
        note_number: 3,
        title: "Intercompany eliminations".to_string(),
        body: format!(
            "Out of {planned} intercompany transaction pairs planned, \
             {matched} were matched and eliminated on consolidation \
             (coverage {coverage:.2}%). Unmatched residuals were retained \
             with full pair-level diagnostics.",
            planned = inputs.ic_coverage.total_pairs_planned,
            matched = inputs.ic_coverage.matched,
            coverage = inputs.ic_coverage.coverage * 100.0,
        ),
    });

    // ── Note 4: Non-controlling interest summary ───────────────────────────
    let nci_body = if inputs.nci_rollforwards.is_empty() {
        "The group has no non-controlling interest as of the reporting date.".to_string()
    } else {
        let mut lines: Vec<String> = Vec::with_capacity(inputs.nci_rollforwards.len() + 1);
        lines.push(
            "Non-controlling interest by subsidiary (opening, share of profit, dividends, closing):"
                .to_string(),
        );
        for rf in inputs.nci_rollforwards {
            lines.push(format!(
                "- {entity}: NCI {nci_pct}, opening {opening}, share of profit \
                 {sop}, dividends {div}, closing {closing} {currency}",
                entity = rf.entity_code,
                nci_pct = rf.nci_percent,
                opening = rf.opening_nci,
                sop = rf.nci_share_of_profit,
                div = rf.nci_dividends,
                closing = rf.closing_nci,
                currency = rf.currency,
            ));
        }
        lines.join("\n")
    };
    notes.push(Note {
        note_number: 4,
        title: "Non-controlling interest".to_string(),
        body: nci_body,
    });

    // ── Note 5: Currency translation summary ───────────────────────────────
    let cta_body = if inputs.cta_rollforwards.is_empty() {
        "All entities report in the group presentation currency; no \
         translation adjustment was required for the period."
            .to_string()
    } else {
        let mut lines: Vec<String> = Vec::with_capacity(inputs.cta_rollforwards.len() + 1);
        lines.push(
            "Cumulative translation adjustment by entity (opening, period, closing):".to_string(),
        );
        for rf in inputs.cta_rollforwards {
            lines.push(format!(
                "- {entity} ({fc}→{pc}): opening {opening}, period {period}, \
                 closing {closing}",
                entity = rf.entity_code,
                fc = rf.functional_currency,
                pc = rf.presentation_currency,
                opening = rf.opening_cta,
                period = rf.period_cta,
                closing = rf.closing_cta,
            ));
        }
        lines.join("\n")
    };
    notes.push(Note {
        note_number: 5,
        title: "Foreign currency translation".to_string(),
        body: cta_body,
    });

    // ── Note 6: Operating segments (IFRS 8 / ASC 280 — v5.1) ───────────────
    notes.push(Note {
        note_number: 6,
        title: "Operating segments".to_string(),
        body: build_operating_segments_note(inputs),
    });

    // ── Note 7: Subsequent events (placeholder) ────────────────────────────
    notes.push(Note {
        note_number: 7,
        title: "Subsequent events".to_string(),
        body: "No material subsequent events identified.".to_string(),
    });

    // ── Note 8: Related parties (placeholder) ──────────────────────────────
    let _equity_invest_count = inputs.equity_method_investments.len();
    notes.push(Note {
        note_number: 8,
        title: "Related parties".to_string(),
        body: "Refer to manifest IC relationships for related-party balances.".to_string(),
    });

    NotesToConsolidatedFs {
        group_id: inputs.manifest.group_id.clone(),
        period_end,
        framework: inputs.framework.to_string(),
        notes,
    }
}

// Avoid an unused-import warning when none of the `_` lookups land —
// keep `Decimal` and `EquityMethodInvestment` imports referenced via
// the function body.
#[allow(dead_code)]
fn _silence_unused(_: Decimal, _: &EquityMethodInvestment) {}

/// Build the body of Note 6 (Operating segments) from the manifest.
///
/// v5.1 implements IFRS 8 / ASC 280 segment disclosure on a
/// **geographic basis** — entities are grouped by country code, with
/// per-country headcount and consolidation-method breakdown.  This
/// honours IFRS 8.13 (entity-wide disclosure of geographic
/// information) when the operating-segments-by-product-line basis
/// (IFRS 8.5) isn't yet wired through from per-entity segment data.
///
/// Future v5.2+: read the per-entity
/// `financial_reporting/segment_reporting/segment_reports.json`
/// archives during aggregate, sum revenue / operating profit / assets
/// by reportable segment, and emit a full `OperatingSegment` table
/// alongside this note body.
fn build_operating_segments_note(inputs: &NotesInputs) -> String {
    use std::collections::BTreeMap;

    let mut by_country: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for entity in &inputs.manifest.ownership_graph.entities {
        by_country
            .entry(entity.country.as_str())
            .or_default()
            .push(entity.code.as_str());
    }

    let total_entities = inputs.manifest.ownership_graph.entities.len();
    let total_countries = by_country.len();

    let mut body = String::new();
    body.push_str(&format!(
        "Geographic segmentation (IFRS 8.13 / ASC 280-10-50-41 \
         entity-wide disclosure).  The group consolidates {} entities \
         across {} countries.\n\n",
        total_entities, total_countries
    ));
    body.push_str("Per-country entity breakdown:\n");
    for (country, entities) in &by_country {
        body.push_str(&format!(
            "  - {}: {} entit{} ({})\n",
            country,
            entities.len(),
            if entities.len() == 1 { "y" } else { "ies" },
            entities.join(", "),
        ));
    }
    body.push('\n');
    body.push_str(
        "Operating segments by product line / business unit (IFRS 8.5) \
         are sourced from per-entity profit-centre hierarchies and \
         segment_reports artefacts; the consolidated segment \
         aggregation across entities is on the v5.2 roadmap.  Refer \
         to each contributing entity's \
         `financial_reporting/segment_reporting/segment_reports.json` \
         for the per-entity segment detail.",
    );
    body
}

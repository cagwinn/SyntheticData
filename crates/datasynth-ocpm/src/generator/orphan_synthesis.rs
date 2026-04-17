//! Synthesize OCPM events for journal entries that otherwise have no
//! process-mining linkage.
//!
//! Whole classes of JEs bypass document-flow generation — period-close
//! postings (depreciation, accruals, currency-translation adjustment),
//! intercompany eliminations, opening balances, and standards-driven entries
//! (revenue recognition, leases, impairment, ECL, provisions). Those JEs
//! never receive a matching OCEL event from the document-flow generators,
//! so process-mining tools see them as orphans.
//!
//! [`synthesize_events_for_orphan_entries`] is a post-generation sweep that
//! mints a minimal [`OcpmEvent`] for every JE with empty `ocpm_event_ids`.
//! The synthesized events carry an `activity_id` classified from the
//! entry's flags (post-close, elimination, depreciation, accrual, etc.) and
//! a reference back to the entry, yielding ~100 % JE → OCPM coverage.

use datasynth_core::models::{BusinessProcess, JournalEntry};

use crate::models::{EventLogMetadata, OcpmEvent, OcpmEventLog};

/// Synthesize minimal OCPM events for every journal entry with an empty
/// `ocpm_event_ids` vector. Appends the events to `event_log` and records the
/// new `event_id` on each entry header. Returns the number of entries
/// newly linked.
///
/// This is idempotent: once an entry has at least one event id, subsequent
/// calls leave it alone.
pub fn synthesize_events_for_orphan_entries(
    entries: &mut [JournalEntry],
    event_log: &mut OcpmEventLog,
) -> usize {
    let mut synthesized = 0;
    for entry in entries.iter_mut() {
        if !entry.header.ocpm_event_ids.is_empty() {
            continue;
        }
        let activity = classify_activity(entry);
        let mut event = OcpmEvent::new(
            activity.id,
            activity.name,
            entry.header.created_at,
            &entry.header.created_by,
            &entry.header.company_code,
        );
        // Link back to the JE via a reference so downstream joins match.
        event.document_ref = Some(entry.header.document_id.to_string());
        event.journal_entry_id = Some(entry.header.document_id);

        entry.header.ocpm_event_ids.push(event.event_id);
        event_log.add_event(event);
        synthesized += 1;
    }
    if synthesized > 0 {
        // Keep metadata in sync with synthesized events.
        refresh_metadata(event_log);
    }
    synthesized
}

struct ActivityClass {
    id: &'static str,
    name: &'static str,
}

/// Best-effort classification of a JE's originating activity. The mapping
/// favours specificity — depreciation, elimination, accrual etc. are
/// identified before falling back to the generic "post_journal_entry".
fn classify_activity(entry: &JournalEntry) -> ActivityClass {
    let doc_type = entry.header.document_type.to_ascii_uppercase();
    let header_text = entry
        .header
        .header_text
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();

    if entry.header.is_elimination {
        return ActivityClass {
            id: "post_elimination",
            name: "Post Intercompany Elimination",
        };
    }
    if doc_type.contains("ZD") || header_text.contains("deprec") {
        return ActivityClass {
            id: "run_depreciation",
            name: "Run Depreciation",
        };
    }
    if doc_type.contains("AF") || header_text.contains("accrual") {
        return ActivityClass {
            id: "post_accrual",
            name: "Post Accrual",
        };
    }
    if header_text.contains("cta") || header_text.contains("translation") {
        return ActivityClass {
            id: "translate_currency",
            name: "Translate Currency",
        };
    }
    if header_text.contains("opening balance") || header_text.contains("opening_balance") {
        return ActivityClass {
            id: "post_opening_balance",
            name: "Post Opening Balance",
        };
    }
    if header_text.contains("revenue recognition") || header_text.contains("asc 606") {
        return ActivityClass {
            id: "recognize_revenue",
            name: "Recognize Revenue",
        };
    }
    if header_text.contains("lease") {
        return ActivityClass {
            id: "post_lease_entry",
            name: "Post Lease Entry",
        };
    }
    if header_text.contains("impairment") {
        return ActivityClass {
            id: "post_impairment",
            name: "Post Impairment",
        };
    }
    if header_text.contains("ecl") || header_text.contains("expected credit loss") {
        return ActivityClass {
            id: "post_ecl_provision",
            name: "Post ECL Provision",
        };
    }
    if header_text.contains("provision") {
        return ActivityClass {
            id: "post_provision",
            name: "Post Provision",
        };
    }
    if entry.header.is_post_close {
        return ActivityClass {
            id: "period_close_entry",
            name: "Post Period-Close Entry",
        };
    }

    match entry.header.business_process {
        Some(BusinessProcess::R2R) => ActivityClass {
            id: "post_adjusting_entry",
            name: "Post Adjusting Entry",
        },
        Some(BusinessProcess::P2P) => ActivityClass {
            id: "post_journal_entry",
            name: "Post Journal Entry (P2P)",
        },
        Some(BusinessProcess::O2C) => ActivityClass {
            id: "post_journal_entry",
            name: "Post Journal Entry (O2C)",
        },
        _ => ActivityClass {
            id: "post_journal_entry",
            name: "Post Journal Entry",
        },
    }
}

fn refresh_metadata(event_log: &mut OcpmEventLog) {
    let md = EventLogMetadata {
        event_count: event_log.events.len(),
        ..event_log.metadata.clone()
    };
    event_log.metadata = md;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, Utc};
    use datasynth_core::models::{JournalEntry, JournalEntryHeader};

    fn mk_entry(doc_type: &str, header_text: Option<&str>) -> JournalEntry {
        let mut h =
            JournalEntryHeader::new("C001".into(), NaiveDate::from_ymd_opt(2024, 6, 15).unwrap());
        h.document_type = doc_type.into();
        h.header_text = header_text.map(|s| s.to_string());
        h.created_at = Utc::now();
        JournalEntry {
            header: h,
            lines: Default::default(),
        }
    }

    #[test]
    fn covers_orphan_entries_exactly_once() {
        let mut entries = vec![
            mk_entry("AF", Some("monthly accrual")),
            mk_entry("ZD", Some("depreciation run")),
            mk_entry("SA", Some("CTA translation")),
        ];
        let mut log = OcpmEventLog::new();
        let n = synthesize_events_for_orphan_entries(&mut entries, &mut log);
        assert_eq!(n, 3);
        assert_eq!(log.events.len(), 3);
        for e in &entries {
            assert_eq!(e.header.ocpm_event_ids.len(), 1);
        }
        // A second pass must be a no-op.
        let n2 = synthesize_events_for_orphan_entries(&mut entries, &mut log);
        assert_eq!(n2, 0);
        assert_eq!(log.events.len(), 3);
    }

    #[test]
    fn skips_entries_with_existing_links() {
        let mut entry = mk_entry("SA", None);
        entry.header.ocpm_event_ids.push(uuid::Uuid::new_v4());
        let mut entries = vec![entry];
        let mut log = OcpmEventLog::new();
        let n = synthesize_events_for_orphan_entries(&mut entries, &mut log);
        assert_eq!(n, 0);
        assert_eq!(log.events.len(), 0);
    }

    #[test]
    fn classifies_elimination_over_document_type() {
        let mut entry = mk_entry("SA", Some("intercompany elimination"));
        entry.header.is_elimination = true;
        let mut entries = vec![entry];
        let mut log = OcpmEventLog::new();
        synthesize_events_for_orphan_entries(&mut entries, &mut log);
        assert_eq!(log.events[0].activity_id, "post_elimination");
    }

    #[test]
    fn classifies_depreciation_by_header_text() {
        let mut entries = vec![mk_entry("SA", Some("monthly depreciation charge"))];
        let mut log = OcpmEventLog::new();
        synthesize_events_for_orphan_entries(&mut entries, &mut log);
        assert_eq!(log.events[0].activity_id, "run_depreciation");
    }

    #[test]
    fn classifies_post_close_as_generic_close_entry() {
        let mut entry = mk_entry("SA", None);
        entry.header.is_post_close = true;
        let mut entries = vec![entry];
        let mut log = OcpmEventLog::new();
        synthesize_events_for_orphan_entries(&mut entries, &mut log);
        assert_eq!(log.events[0].activity_id, "period_close_entry");
    }
}

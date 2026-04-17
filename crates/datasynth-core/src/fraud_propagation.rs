//! Bidirectional fraud-label propagation between documents and journal entries.
//!
//! * [`propagate_documents_to_entries`]: for every JE whose `source_document`
//!   matches a fraudulent document, set `is_fraud`, copy `fraud_type`, and
//!   record `fraud_source_document_id` + `is_fraud_propagated`. Used when
//!   fraud is injected at document granularity and should cascade to the
//!   derived accounting lines.
//! * [`propagate_entries_to_documents`]: the inverse — when fraud is
//!   injected line-level, mark each JE's source document so document flags
//!   stay in sync with line flags.
//!
//! Both functions are pure and idempotent.

use std::collections::HashMap;

use rand::{Rng, RngExt};

use crate::models::documents::DocumentHeader;
use crate::models::{FraudType, JournalEntry};

/// Propagate fraud labels from documents onto every journal entry whose
/// `source_document` matches a fraudulent document. Returns the number of
/// entries newly tagged.
///
/// The JE's `is_fraud_propagated` flag is set to `true` and
/// `fraud_source_document_id` records the document id, so the two populations
/// (document-propagated vs. line-injected) can be distinguished downstream.
/// An entry that was already `is_fraud = true` before this call is left
/// unchanged (fraud_type preserved, propagation flags *not* retroactively set).
pub fn propagate_documents_to_entries(
    documents: &[DocumentHeader],
    entries: &mut [JournalEntry],
) -> usize {
    let fraud_map = build_document_fraud_map(documents);
    if fraud_map.is_empty() {
        return 0;
    }
    let mut tagged = 0;
    for entry in entries.iter_mut() {
        if entry.header.is_fraud {
            continue;
        }
        if entry.header.propagate_fraud_from_documents(&fraud_map) {
            tagged += 1;
        }
    }
    tagged
}

/// Propagate fraud labels from journal entries back to their source
/// documents. Returns the number of documents newly tagged.
///
/// Uses the existing [`DocumentHeader::propagate_fraud`] helper so the
/// semantics match (fraud-map keyed by document id and journal-entry id).
pub fn propagate_entries_to_documents(
    entries: &[JournalEntry],
    documents: &mut [DocumentHeader],
) -> usize {
    let fraud_map = build_entry_fraud_map(entries);
    if fraud_map.is_empty() {
        return 0;
    }
    let mut tagged = 0;
    for doc in documents.iter_mut() {
        if doc.is_fraud {
            continue;
        }
        if doc.propagate_fraud(&fraud_map) {
            tagged += 1;
        }
    }
    tagged
}

/// Mark each document in `headers` as fraudulent with probability `rate`,
/// selecting a [`FraudType`] via `pick_fraud_type`. Returns the number of
/// headers tagged.
///
/// Document-level counterpart to the JE-level anomaly injector — when
/// paired with [`propagate_documents_to_entries`] this produces the
/// scheme-level fraud pattern where one fraudulent source document spawns
/// several correlated fraudulent JE lines.
///
/// An already-fraudulent header is skipped. `rate` is clamped to
/// `[0.0, 1.0]`; non-finite rates yield a no-op.
pub fn inject_document_fraud<R, F>(
    headers: &mut [&mut DocumentHeader],
    rate: f64,
    rng: &mut R,
    mut pick_fraud_type: F,
) -> usize
where
    R: Rng,
    F: FnMut(&mut R) -> FraudType,
{
    if !rate.is_finite() || rate <= 0.0 {
        return 0;
    }
    let rate = rate.min(1.0);
    let mut tagged = 0;
    for h in headers.iter_mut() {
        if h.is_fraud {
            continue;
        }
        if rng.random::<f64>() < rate {
            h.is_fraud = true;
            h.fraud_type = Some(pick_fraud_type(rng));
            tagged += 1;
        }
    }
    tagged
}

/// Build a fraud-map keyed by document id from a slice of [`DocumentHeader`].
pub fn build_document_fraud_map(documents: &[DocumentHeader]) -> HashMap<String, FraudType> {
    let mut map = HashMap::with_capacity(documents.len());
    for doc in documents {
        if doc.is_fraud {
            if let Some(ft) = doc.fraud_type {
                map.insert(doc.document_id.clone(), ft);
            }
        }
    }
    map
}

/// Build a fraud-map keyed by JE source-document id from a slice of
/// [`JournalEntry`]. Only entries with `is_fraud = true`, a concrete
/// `fraud_type`, and a resolvable `source_document` contribute.
pub fn build_entry_fraud_map(entries: &[JournalEntry]) -> HashMap<String, FraudType> {
    let mut map = HashMap::new();
    for entry in entries {
        if !entry.header.is_fraud {
            continue;
        }
        let Some(ft) = entry.header.fraud_type else {
            continue;
        };
        if let Some(doc_id) = entry
            .header
            .source_document
            .as_ref()
            .and_then(|r| r.document_id())
        {
            // First write wins (stable when multiple JEs share a source doc).
            map.entry(doc_id.to_string()).or_insert(ft);
        }
    }
    map
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::models::documents::{DocumentHeader, DocumentStatus, DocumentType};
    use crate::models::{DocumentRef, FraudType, JournalEntry};
    use chrono::NaiveDate;

    fn mk_doc(id: &str, is_fraud: bool, ft: Option<FraudType>) -> DocumentHeader {
        let mut h = DocumentHeader::new(
            id,
            DocumentType::PurchaseOrder,
            "C001",
            2024,
            6,
            NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
            "user",
        );
        h.status = DocumentStatus::Posted;
        h.is_fraud = is_fraud;
        h.fraud_type = ft;
        h
    }

    fn mk_entry(doc_id: &str) -> JournalEntry {
        let mut e = JournalEntry::new_simple(
            format!("JE-{doc_id}"),
            "C001".into(),
            NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
            "test".into(),
        );
        e.header.source_document = Some(DocumentRef::PurchaseOrder(doc_id.to_string()));
        e
    }

    #[test]
    fn documents_to_entries_tags_matching_entries() {
        let docs = vec![mk_doc("PO-1", true, Some(FraudType::FictitiousEntry))];
        let mut entries = vec![mk_entry("PO-1"), mk_entry("PO-2")];
        let tagged = propagate_documents_to_entries(&docs, &mut entries);
        assert_eq!(tagged, 1);
        assert!(entries[0].header.is_fraud);
        assert!(entries[0].header.is_fraud_propagated);
        assert_eq!(
            entries[0].header.fraud_source_document_id.as_deref(),
            Some("PO-1")
        );
        assert_eq!(
            entries[0].header.fraud_type,
            Some(FraudType::FictitiousEntry)
        );
        assert!(!entries[1].header.is_fraud);
    }

    #[test]
    fn documents_to_entries_preserves_already_fraudulent_entries() {
        let docs = vec![mk_doc("PO-1", true, Some(FraudType::FictitiousEntry))];
        let mut entries = vec![mk_entry("PO-1")];
        entries[0].header.is_fraud = true;
        entries[0].header.fraud_type = Some(FraudType::RevenueManipulation);
        let tagged = propagate_documents_to_entries(&docs, &mut entries);
        assert_eq!(tagged, 0, "should not overwrite pre-existing fraud labels");
        // fraud_type must not be retro-propagated
        assert_eq!(
            entries[0].header.fraud_type,
            Some(FraudType::RevenueManipulation)
        );
        assert!(!entries[0].header.is_fraud_propagated);
    }

    #[test]
    fn entries_to_documents_tags_matching_documents() {
        let mut docs = vec![mk_doc("PO-1", false, None), mk_doc("PO-2", false, None)];
        let mut entries = vec![mk_entry("PO-1")];
        entries[0].header.is_fraud = true;
        entries[0].header.fraud_type = Some(FraudType::DuplicatePayment);
        let tagged = propagate_entries_to_documents(&entries, &mut docs);
        assert_eq!(tagged, 1);
        assert!(docs[0].is_fraud);
        assert_eq!(docs[0].fraud_type, Some(FraudType::DuplicatePayment));
        assert!(!docs[1].is_fraud);
    }

    #[test]
    fn empty_inputs_are_noops() {
        let docs: Vec<DocumentHeader> = Vec::new();
        let mut entries: Vec<JournalEntry> = Vec::new();
        assert_eq!(propagate_documents_to_entries(&docs, &mut entries), 0);
        assert_eq!(propagate_entries_to_documents(&entries, &mut Vec::new()), 0);
    }

    #[test]
    fn idempotent() {
        let docs = vec![mk_doc("PO-1", true, Some(FraudType::FictitiousEntry))];
        let mut entries = vec![mk_entry("PO-1")];
        let first = propagate_documents_to_entries(&docs, &mut entries);
        let second = propagate_documents_to_entries(&docs, &mut entries);
        assert_eq!(first, 1);
        assert_eq!(second, 0, "second pass should be a no-op");
    }

    #[test]
    fn inject_document_fraud_rate_zero_is_noop() {
        use rand_chacha::{rand_core::SeedableRng, ChaCha8Rng};
        let mut docs = [mk_doc("PO-1", false, None)];
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let mut refs: Vec<&mut DocumentHeader> = docs.iter_mut().collect();
        let tagged =
            inject_document_fraud(&mut refs, 0.0, &mut rng, |_| FraudType::FictitiousEntry);
        assert_eq!(tagged, 0);
        assert!(!docs[0].is_fraud);
    }

    #[test]
    fn inject_document_fraud_rate_one_tags_everything_once() {
        use rand_chacha::{rand_core::SeedableRng, ChaCha8Rng};
        let mut docs = [
            mk_doc("PO-1", false, None),
            mk_doc("PO-2", false, None),
            mk_doc("PO-3", true, Some(FraudType::DuplicatePayment)),
        ];
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let mut refs: Vec<&mut DocumentHeader> = docs.iter_mut().collect();
        let tagged =
            inject_document_fraud(&mut refs, 1.0, &mut rng, |_| FraudType::FictitiousEntry);
        // PO-3 is already fraud and must be skipped; PO-1 + PO-2 get tagged.
        assert_eq!(tagged, 2);
        assert!(docs[0].is_fraud);
        assert!(docs[1].is_fraud);
        // Pre-existing fraud type on PO-3 is preserved.
        assert_eq!(docs[2].fraud_type, Some(FraudType::DuplicatePayment));
    }

    #[test]
    fn inject_document_fraud_respects_rate_approximately() {
        use rand_chacha::{rand_core::SeedableRng, ChaCha8Rng};
        let mut docs: Vec<DocumentHeader> = (0..1_000)
            .map(|i| mk_doc(&format!("D-{i}"), false, None))
            .collect();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let mut refs: Vec<&mut DocumentHeader> = docs.iter_mut().collect();
        let tagged =
            inject_document_fraud(&mut refs, 0.10, &mut rng, |_| FraudType::FictitiousEntry);
        // Expect roughly 100 ± ~30 tagged at rate 0.10 with n=1000.
        assert!(tagged > 60 && tagged < 140, "got {tagged}");
    }
}

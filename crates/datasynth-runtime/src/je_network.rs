//! Shared Method-A / Method-B/C edge-list builder for the JE network export.
//!
//! v5.10: extracted from [`output_writer::write_je_network_csv`] so the
//! same logic can be reused by the `datasynth-group` aggregate emitter,
//! which builds both per-entity and consolidated edge lists from many
//! per-entity JE batches.
//!
//! The builder is pure — no I/O — and returns a `Vec<JeNetworkEdge>`.
//! Single-entity callers feed the result into the existing CSV writer
//! (preserving v5.8.0 byte-identical output); group callers serialise
//! the same struct with extra contextual columns (entity_code,
//! is_eliminated, eliminates_ic_pair_id) added at write time.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::collections::HashMap;
use uuid::Uuid;

use datasynth_config::JeNetworkMethod;
use datasynth_core::models::JournalEntry;

/// One row of the je_network output.
///
/// The 13 columns matching v5.8.0 CSV output are at the top; v5.10
/// added the optional IC fields (Some only on group-context inputs).
#[derive(Debug, Clone)]
pub struct JeNetworkEdge {
    pub edge_id: String,
    pub document_id: Uuid,
    pub posting_date: NaiveDate,
    pub from_account: String,
    pub to_account: String,
    pub from_line_id: String,
    pub to_line_id: String,
    pub amount: Decimal,
    pub confidence: f64,
    pub predecessor_edge_id: String,
    pub business_process: String,
    pub is_fraud: bool,
    pub is_anomaly: bool,
    /// v5.10 — surfaces `JournalEntryHeader::ic_pair_id` when present.
    /// `None` for non-IC postings (and on every edge from a single-entity run).
    pub ic_pair_id: Option<String>,
    /// v5.10 — surfaces `JournalEntryHeader::ic_partner_entity` when present.
    pub ic_partner_entity: Option<String>,
}

/// Build the Method-A / Method-B/C edge list for a batch of JEs.
///
/// JEs are processed in the iteration order of the slice; predecessor
/// edge ids are resolved against earlier JEs in the same batch (so the
/// caller must pass JEs that share predecessor chains in the same call).
///
/// Method semantics — see Ivertowski et al. (2024) Methods A through E:
/// * `JeNetworkMethod::A` — bijective on 2-line entries only; multi-line
///   JEs are skipped. Confidence on every emitted edge = `1.0`.
/// * `JeNetworkMethod::Cartesian` — full Cartesian debit × credit product
///   with proportional amount allocation; confidence = `1 / (n × m)`.
pub fn build_je_network_edges(jes: &[JournalEntry], method: JeNetworkMethod) -> Vec<JeNetworkEdge> {
    let mut edges = Vec::with_capacity(jes.len() * 2);
    let mut line_id_to_edge_id: HashMap<String, String> = HashMap::with_capacity(jes.len() * 2);

    for je in jes {
        let h = &je.header;

        let line_ids: Vec<String> = je
            .lines
            .iter()
            .map(|l| {
                l.transaction_id.clone().unwrap_or_else(|| {
                    datasynth_core::models::JournalEntryLine::derive_transaction_id(
                        l.document_id,
                        l.line_number,
                    )
                })
            })
            .collect();

        let debits: Vec<usize> = je
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.debit_amount > Decimal::ZERO)
            .map(|(i, _)| i)
            .collect();
        let credits: Vec<usize> = je
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.credit_amount > Decimal::ZERO)
            .map(|(i, _)| i)
            .collect();
        if debits.is_empty() || credits.is_empty() {
            continue;
        }

        if method == JeNetworkMethod::A && !(debits.len() == 1 && credits.len() == 1) {
            continue;
        }

        let total_debit: Decimal = debits.iter().map(|i| je.lines[*i].debit_amount).sum();
        let total_credit: Decimal = credits.iter().map(|i| je.lines[*i].credit_amount).sum();
        if total_debit.is_zero() || total_credit.is_zero() {
            continue;
        }

        let confidence: f64 = if debits.len() == 1 && credits.len() == 1 {
            1.0
        } else {
            1.0 / (debits.len() * credits.len()) as f64
        };

        let bp = h
            .business_process
            .map(|bp| format!("{bp:?}"))
            .unwrap_or_default();
        let ic_pair_id_str = h.ic_pair_id.as_ref().map(|id| id.to_string());
        let ic_partner = h.ic_partner_entity.clone();

        for &di in &debits {
            let debit_line = &je.lines[di];
            let to_line_id = &line_ids[di];
            for &ci in &credits {
                let credit_line = &je.lines[ci];
                let from_line_id = &line_ids[ci];

                // Edge id = UUID v5 of (document_id, debit.line_number,
                // credit.line_number). Stable across regenerations.
                let mut input = Vec::with_capacity(16 + 8);
                input.extend_from_slice(h.document_id.as_bytes());
                input.extend_from_slice(&debit_line.line_number.to_le_bytes());
                input.extend_from_slice(&credit_line.line_number.to_le_bytes());
                let edge_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, &input).to_string();

                // Proportional allocation matches
                // TransactionGraphBuilder::add_journal_entry_debit_credit.
                let proportion = (debit_line.debit_amount / total_debit)
                    * (credit_line.credit_amount / total_credit);
                let amount = debit_line.debit_amount * proportion;

                let predecessor_edge_id: String = credit_line
                    .predecessor_line_id
                    .as_ref()
                    .or(debit_line.predecessor_line_id.as_ref())
                    .and_then(|tx_id| line_id_to_edge_id.get(tx_id).cloned())
                    .unwrap_or_default();

                edges.push(JeNetworkEdge {
                    edge_id: edge_id.clone(),
                    document_id: h.document_id,
                    posting_date: h.posting_date,
                    from_account: credit_line.gl_account.clone(),
                    to_account: debit_line.gl_account.clone(),
                    from_line_id: from_line_id.clone(),
                    to_line_id: to_line_id.clone(),
                    amount,
                    confidence,
                    predecessor_edge_id,
                    business_process: bp.clone(),
                    is_fraud: h.is_fraud,
                    is_anomaly: h.is_anomaly,
                    ic_pair_id: ic_pair_id_str.clone(),
                    ic_partner_entity: ic_partner.clone(),
                });

                line_id_to_edge_id
                    .entry(from_line_id.clone())
                    .or_insert(edge_id);
            }
        }
    }

    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use datasynth_core::models::{JournalEntry, JournalEntryHeader, JournalEntryLine};

    fn dec(v: i64) -> Decimal {
        Decimal::from(v)
    }

    fn header_for(doc: Uuid) -> JournalEntryHeader {
        let mut h = JournalEntryHeader::new(
            "C001".to_string(),
            NaiveDate::from_ymd_opt(2026, 5, 9).expect("2026-05-09 is a valid date"),
        );
        h.document_id = doc;
        h
    }

    fn make_line(doc: Uuid, n: u32, account: &str, debit: i64, credit: i64) -> JournalEntryLine {
        JournalEntryLine {
            document_id: doc,
            line_number: n,
            gl_account: account.into(),
            debit_amount: dec(debit),
            credit_amount: dec(credit),
            ..Default::default()
        }
    }

    fn make_two_line_je(debit_account: &str, credit_account: &str, amount: i64) -> JournalEntry {
        let document_id = Uuid::new_v4();
        let header = header_for(document_id);
        let lines = smallvec::smallvec![
            make_line(document_id, 1, debit_account, amount, 0),
            make_line(document_id, 2, credit_account, 0, amount),
        ];
        JournalEntry { header, lines }
    }

    #[test]
    fn method_a_emits_one_edge_per_two_line_je() {
        let jes = vec![
            make_two_line_je("1000", "2000", 1_000),
            make_two_line_je("1000", "4000", 5_000),
        ];
        let edges = build_je_network_edges(&jes, JeNetworkMethod::A);
        assert_eq!(edges.len(), 2, "one edge per 2-line JE");
        for e in &edges {
            assert_eq!(e.confidence, 1.0, "Method A confidence is exactly 1.0");
            assert!(e.ic_pair_id.is_none());
            assert!(e.ic_partner_entity.is_none());
        }
    }

    #[test]
    fn method_a_skips_multi_line_jes() {
        let document_id = Uuid::new_v4();
        let header = header_for(document_id);
        let lines = smallvec::smallvec![
            make_line(document_id, 1, "1000", 1_000, 0),
            make_line(document_id, 2, "1010", 500, 0),
            make_line(document_id, 3, "2000", 0, 1_500),
        ];
        let je = JournalEntry { header, lines };
        let edges = build_je_network_edges(&[je], JeNetworkMethod::A);
        assert_eq!(edges.len(), 0, "3-line JE skipped under Method A");
    }

    #[test]
    fn cartesian_emits_n_times_m_edges_per_je() {
        let document_id = Uuid::new_v4();
        let header = header_for(document_id);
        let lines = smallvec::smallvec![
            make_line(document_id, 1, "D1", 100, 0),
            make_line(document_id, 2, "D2", 50, 0),
            make_line(document_id, 3, "C1", 0, 80),
            make_line(document_id, 4, "C2", 0, 70),
        ];
        let je = JournalEntry { header, lines };
        let edges = build_je_network_edges(&[je], JeNetworkMethod::Cartesian);
        assert_eq!(
            edges.len(),
            4,
            "2 debits × 2 credits = 4 edges under Cartesian"
        );
        for e in &edges {
            assert!((e.confidence - 0.25).abs() < 1e-9, "1/(n*m) = 0.25");
        }
    }

    #[test]
    fn ic_fields_surface_when_present_on_header() {
        let document_id = Uuid::new_v4();
        let mut header = header_for(document_id);
        header.ic_partner_entity = Some("ACME_EUR".to_string());

        let lines = smallvec::smallvec![
            make_line(document_id, 1, "1150", 1000, 0),
            make_line(document_id, 2, "4500", 0, 1000),
        ];
        let je = JournalEntry { header, lines };
        let edges = build_je_network_edges(&[je], JeNetworkMethod::A);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].ic_partner_entity, Some("ACME_EUR".to_string()));
        // ic_pair_id requires construction via IcPairId — covered in group tests.
    }
}

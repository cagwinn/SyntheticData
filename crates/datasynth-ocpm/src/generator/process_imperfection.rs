//! Post-hoc realism mutations for process-mining output.
//!
//! Without these mutations, generated event logs produce unrealistically
//! clean process-mining results (few variants, 100 % Inductive Miner
//! fitness). Real ERP data has dozens of variants and fitness in the 0.7–0.9
//! range due to rework, skipped activities, and out-of-order events.
//!
//! The three mutations applied per case, each gated by its own rate:
//! * **Rework** — duplicate one activity mid-case.
//! * **Skipped steps** — drop one non-terminal activity from the trace
//!   while leaving the underlying event intact in the event log.
//! * **Out-of-order events** — swap the timestamps of two adjacent events.
//!
//! [`propagate_je_anomalies_to_ocel`] mirrors `JournalEntry.header.is_anomaly`
//! / `is_fraud` onto linked OCEL events and the owning `CaseTrace`.

use std::collections::{HashMap, HashSet};

use chrono::Duration;
use datasynth_core::models::JournalEntry;
use rand::{Rng, RngExt};
use uuid::Uuid;

use crate::models::{OcpmEvent, OcpmEventLog};

/// Rates controlling how often each imperfection is applied per case.
#[derive(Debug, Clone)]
pub struct ImperfectionConfig {
    /// Probability that a case gains a duplicate activity (rework).
    pub rework_rate: f64,
    /// Probability that a case has a non-terminal activity removed.
    pub skip_rate: f64,
    /// Probability that two adjacent events in a case have their timestamps swapped.
    pub out_of_order_rate: f64,
}

impl Default for ImperfectionConfig {
    fn default() -> Self {
        Self {
            rework_rate: 0.15,
            skip_rate: 0.10,
            out_of_order_rate: 0.08,
        }
    }
}

/// Counts of imperfections applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImperfectionStats {
    pub rework: usize,
    pub skipped: usize,
    pub out_of_order: usize,
}

/// Apply rework / skip / out-of-order mutations to the log and mark the
/// mutated cases on their variants.
pub fn inject_process_imperfections<R: Rng>(
    event_log: &mut OcpmEventLog,
    config: &ImperfectionConfig,
    rng: &mut R,
) -> ImperfectionStats {
    let mut stats = ImperfectionStats::default();
    let case_ids: Vec<Uuid> = event_log.cases.keys().copied().collect();
    // Build an event_id → index map once. `apply_rework` and
    // `apply_out_of_order` otherwise scan `event_log.events` per call,
    // turning the mutation pass into O(cases × events).
    let mut index: HashMap<Uuid, usize> = event_log
        .events
        .iter()
        .enumerate()
        .map(|(i, e)| (e.event_id, i))
        .collect();
    for case_id in case_ids {
        if rng.random::<f64>() < config.rework_rate && apply_rework(event_log, case_id, &mut index)
        {
            stats.rework += 1;
        }
        if rng.random::<f64>() < config.skip_rate && apply_skip(event_log, case_id, rng) {
            stats.skipped += 1;
        }
        if rng.random::<f64>() < config.out_of_order_rate
            && apply_out_of_order(event_log, case_id, rng, &index)
        {
            stats.out_of_order += 1;
        }
    }
    stats
}

/// Duplicate a randomly-selected non-terminal activity at the end of the
/// case, creating a "returned for rework" pattern (e.g. `Approve → …
/// → Approve` again). Returns `true` on success.
fn apply_rework(
    event_log: &mut OcpmEventLog,
    case_id: Uuid,
    index: &mut HashMap<Uuid, usize>,
) -> bool {
    let Some(case) = event_log.cases.get(&case_id) else {
        return false;
    };
    if case.event_ids.len() < 2 {
        return false;
    }
    // Pick the middle event to duplicate — avoids trivial repeats of the
    // first/last activity.
    let middle_idx = case.event_ids.len() / 2;
    let source_event_id = case.event_ids[middle_idx];
    let Some(&src_idx) = index.get(&source_event_id) else {
        return false;
    };
    let source = event_log.events[src_idx].clone();
    // Bump the rework event's timestamp past the latest event already in the
    // case. We already have `case.event_ids` and the map, so the scan is
    // O(case_size) instead of O(events).
    let end_ts = case
        .event_ids
        .iter()
        .filter_map(|id| index.get(id).map(|i| event_log.events[*i].timestamp))
        .max()
        .unwrap_or(source.timestamp)
        + Duration::minutes(15);
    let mut rework = OcpmEvent::new(
        &source.activity_id,
        &source.activity_name,
        end_ts,
        &source.resource_id,
        &source.company_code,
    );
    rework.case_id = Some(case_id);
    rework.object_refs = source.object_refs.clone();

    let new_event_id = rework.event_id;
    event_log.add_event(rework);
    index.insert(new_event_id, event_log.events.len() - 1);

    if let Some(case_mut) = event_log.cases.get_mut(&case_id) {
        case_mut.event_ids.push(new_event_id);
        case_mut.activity_sequence.push(source.activity_id.clone());
        if let Some(variant_id) = case_mut.variant_id.clone() {
            if let Some(v) = event_log.variants.get_mut(&variant_id) {
                v.has_rework = true;
            }
        }
    }
    true
}

/// Remove one non-terminal activity from a case's sequence. The underlying
/// event stays in `event_log.events` (so the OCEL export remains consistent)
/// but the case trace no longer references it.
fn apply_skip<R: Rng>(event_log: &mut OcpmEventLog, case_id: Uuid, rng: &mut R) -> bool {
    let Some(case) = event_log.cases.get_mut(&case_id) else {
        return false;
    };
    // Need ≥ 3 events to have a non-terminal one.
    if case.event_ids.len() < 3 {
        return false;
    }
    // Drop a random interior index (not first or last).
    let drop_idx = rng.random_range(1..case.event_ids.len() - 1);
    case.event_ids.remove(drop_idx);
    case.activity_sequence.remove(drop_idx);
    let variant_id = case.variant_id.clone();
    if let Some(variant_id) = variant_id {
        if let Some(v) = event_log.variants.get_mut(&variant_id) {
            v.has_skipped_steps = true;
        }
    }
    true
}

/// Swap the timestamps of two adjacent events in a case, creating a
/// superficially-valid trace whose temporal order no longer matches the
/// logical sequence.
fn apply_out_of_order<R: Rng>(
    event_log: &mut OcpmEventLog,
    case_id: Uuid,
    rng: &mut R,
    index: &HashMap<Uuid, usize>,
) -> bool {
    let Some(case) = event_log.cases.get(&case_id) else {
        return false;
    };
    if case.event_ids.len() < 2 {
        return false;
    }
    let pivot = rng.random_range(0..case.event_ids.len() - 1);
    let a_id = case.event_ids[pivot];
    let b_id = case.event_ids[pivot + 1];
    let (Some(&i), Some(&j)) = (index.get(&a_id), index.get(&b_id)) else {
        return false;
    };
    let ts_a = event_log.events[i].timestamp;
    let ts_b = event_log.events[j].timestamp;
    event_log.events[i].timestamp = ts_b;
    event_log.events[j].timestamp = ts_a;

    let variant_id = event_log
        .cases
        .get(&case_id)
        .and_then(|c| c.variant_id.clone());
    if let Some(variant_id) = variant_id {
        if let Some(v) = event_log.variants.get_mut(&variant_id) {
            v.has_out_of_order = true;
        }
    }
    true
}

/// Mirror `JournalEntry.header.is_anomaly` / `is_fraud` onto the OCEL events
/// each JE is linked to via `ocpm_event_ids`, and mark the owning
/// `CaseTrace` as anomalous. Returns the number of events updated.
///
/// Without this, the `is_anomaly` boolean flag on every serialized OCEL
/// event is always `false`, making the JE-level anomaly labels invisible to
/// process-mining consumers.
pub fn propagate_je_anomalies_to_ocel(
    entries: &[JournalEntry],
    event_log: &mut OcpmEventLog,
) -> usize {
    // Build a set of anomalous event ids and map of (event_id → anomaly type).
    let mut anomalous: HashMap<Uuid, Option<String>> = HashMap::new();
    for entry in entries {
        let flagged = entry.header.is_anomaly || entry.header.is_fraud;
        if !flagged {
            continue;
        }
        let label = entry
            .header
            .anomaly_type
            .clone()
            .or_else(|| entry.header.fraud_type.as_ref().map(|ft| format!("{ft:?}")));
        for eid in &entry.header.ocpm_event_ids {
            anomalous.insert(*eid, label.clone());
        }
    }
    if anomalous.is_empty() {
        return 0;
    }
    // Apply to events. Collect the set of affected case ids so we can tag them.
    let mut affected_cases: HashSet<Uuid> = HashSet::new();
    let mut updated = 0usize;
    for event in event_log.events.iter_mut() {
        if let Some(label) = anomalous.get(&event.event_id) {
            if !event.is_anomaly {
                event.is_anomaly = true;
                if let Some(l) = label {
                    event.anomaly_type = Some(l.clone());
                }
                if let Some(cid) = event.case_id {
                    affected_cases.insert(cid);
                }
                updated += 1;
            }
        }
    }
    for cid in affected_cases {
        if let Some(case) = event_log.cases.get_mut(&cid) {
            case.is_anomaly = true;
        }
    }
    updated
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone, Utc};
    use datasynth_core::models::{BusinessProcess, JournalEntryHeader};
    use rand_chacha::{rand_core::SeedableRng, ChaCha8Rng};

    fn mk_case(log: &mut OcpmEventLog, n_events: usize) -> Uuid {
        let case_id = Uuid::new_v4();
        let mut trace = crate::models::CaseTrace::new(
            BusinessProcess::P2P,
            Uuid::new_v4(),
            "purchase_order",
            "C001",
        )
        .with_id(case_id);
        let base = Utc.with_ymd_and_hms(2024, 6, 15, 9, 0, 0).unwrap();
        for i in 0..n_events {
            let mut ev = OcpmEvent::new(
                &format!("act_{i}"),
                &format!("Activity {i}"),
                base + Duration::minutes(i as i64 * 10),
                "user",
                "C001",
            );
            ev.case_id = Some(case_id);
            let eid = ev.event_id;
            log.add_event(ev);
            trace.event_ids.push(eid);
            trace.activity_sequence.push(format!("act_{i}"));
        }
        // Add a variant so flags can be set.
        let variant_id = format!("VAR-{case_id}");
        log.variants.insert(
            variant_id.clone(),
            crate::models::ProcessVariant::new(&variant_id, BusinessProcess::P2P),
        );
        trace.variant_id = Some(variant_id);
        log.cases.insert(case_id, trace);
        case_id
    }

    fn index_for(log: &OcpmEventLog) -> HashMap<Uuid, usize> {
        log.events
            .iter()
            .enumerate()
            .map(|(i, e)| (e.event_id, i))
            .collect()
    }

    #[test]
    fn rework_adds_one_event_and_sets_variant_flag() {
        let mut log = OcpmEventLog::new();
        let case_id = mk_case(&mut log, 4);
        let variant_id = log.cases[&case_id].variant_id.clone().unwrap();
        let before = log.events.len();
        let mut index = index_for(&log);
        assert!(apply_rework(&mut log, case_id, &mut index));
        assert_eq!(log.events.len(), before + 1);
        assert_eq!(log.cases[&case_id].event_ids.len(), 5);
        assert!(log.variants[&variant_id].has_rework);
    }

    #[test]
    fn skip_drops_interior_event_and_sets_flag() {
        let mut log = OcpmEventLog::new();
        let case_id = mk_case(&mut log, 4);
        let variant_id = log.cases[&case_id].variant_id.clone().unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        assert!(apply_skip(&mut log, case_id, &mut rng));
        assert_eq!(log.cases[&case_id].event_ids.len(), 3);
        assert!(log.variants[&variant_id].has_skipped_steps);
        // Underlying events are preserved.
        assert_eq!(log.events.len(), 4);
    }

    #[test]
    fn out_of_order_swaps_timestamps() {
        let mut log = OcpmEventLog::new();
        let case_id = mk_case(&mut log, 3);
        let variant_id = log.cases[&case_id].variant_id.clone().unwrap();
        let before: Vec<_> = log.events.iter().map(|e| e.timestamp).collect();
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let index = index_for(&log);
        assert!(apply_out_of_order(&mut log, case_id, &mut rng, &index));
        let after: Vec<_> = log.events.iter().map(|e| e.timestamp).collect();
        assert_ne!(before, after);
        assert!(log.variants[&variant_id].has_out_of_order);
    }

    #[test]
    fn inject_zero_rates_is_noop() {
        let mut log = OcpmEventLog::new();
        mk_case(&mut log, 3);
        let cfg = ImperfectionConfig {
            rework_rate: 0.0,
            skip_rate: 0.0,
            out_of_order_rate: 0.0,
        };
        let mut rng = ChaCha8Rng::seed_from_u64(2);
        let stats = inject_process_imperfections(&mut log, &cfg, &mut rng);
        assert_eq!(stats, ImperfectionStats::default());
    }

    #[test]
    fn propagate_je_anomalies_marks_events_and_cases() {
        let mut log = OcpmEventLog::new();
        let case_id = mk_case(&mut log, 3);
        let event_id = log.cases[&case_id].event_ids[0];

        let mut header =
            JournalEntryHeader::new("C001".into(), NaiveDate::from_ymd_opt(2024, 6, 15).unwrap());
        header.is_anomaly = true;
        header.anomaly_type = Some("TestAnomaly".into());
        header.ocpm_event_ids = vec![event_id];
        let je = datasynth_core::models::JournalEntry {
            header,
            lines: Default::default(),
        };
        let updated = propagate_je_anomalies_to_ocel(&[je], &mut log);
        assert_eq!(updated, 1);
        let ev = log.events.iter().find(|e| e.event_id == event_id).unwrap();
        assert!(ev.is_anomaly);
        assert_eq!(ev.anomaly_type.as_deref(), Some("TestAnomaly"));
        assert!(log.cases[&case_id].is_anomaly);
    }
}

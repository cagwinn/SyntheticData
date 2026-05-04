//! Optional shard-mode context attached to
//! [`crate::enhanced_orchestrator::EnhancedOrchestrator`].
//!
//! When a group engine (crate `datasynth-group`) drives per-entity
//! generation, it pre-builds any IC journal entries via
//! `datasynth_group::shard::ic_je_injector::inject_ic_journal_entries`
//! and installs them here. The orchestrator appends them to its JE
//! accumulator at the end of the journal-entry phase — no knowledge of IC
//! pair plans or group-crate types leaks into this crate.
//!
//! # Architecture note — deviation from the v5.0 plan
//!
//! The plan originally placed the IC injector in `datasynth-generators`.
//! We host it in `datasynth-group` instead: `IcPairPlan` lives there, and
//! `datasynth-generators` deliberately does not depend on
//! `datasynth-group`.  Keeping the generator-facing surface here opaque
//! (a pre-built `Vec<JournalEntry>`) preserves layer direction:
//! `datasynth-group` depends on `datasynth-runtime`, not the other way
//! around.

use datasynth_core::models::balance::EntityOpeningBalance;
use datasynth_core::models::journal_entry::JournalEntry;

/// Context for shard-mode generation.
///
/// When `EnhancedOrchestrator.shard_context == None` (the default), the
/// orchestrator behaves byte-for-byte like the pre-v5.0 single-entity
/// flow.  When `Some(ShardContext { extra_journal_entries, .. })`, those
/// JEs are appended to Phase 4's accumulator just before the
/// post-journal-entries resource check.
#[derive(Debug, Clone, Default)]
pub struct ShardContext {
    /// This shard's entity code. Informational in v5.0 (the orchestrator
    /// already carries the entity via `GeneratorConfig`); future phases
    /// may use it for routing or logging.
    pub entity_code: String,
    /// 32-byte per-entity seed from
    /// `manifest.ownership_graph.entities[i].entity_seed`.  Informational
    /// in v5.0.
    pub entity_seed: [u8; 32],
    /// IC journal entries pre-built by the shard runner before calling
    /// `EnhancedOrchestrator.generate()`.  Appended to the JE accumulator
    /// at the end of phase 4.
    pub extra_journal_entries: Vec<JournalEntry>,
    /// **v5.3** — Opening-balance carryover from a prior period.  When
    /// non-empty, the orchestrator's Phase 3b (opening balances)
    /// **replaces** its generated openings with these carryover values
    /// for this entity, seeding period-N+1 from period-N's closing TB
    /// instead of generating a fresh opening from the industry-mix
    /// generator.
    ///
    /// Empty by default — engagements that don't supply a prior period
    /// (the v5.0–v5.2 default) see no behaviour change: the orchestrator
    /// generates openings via `OpeningBalanceGenerator` as before.
    ///
    /// Callers prepare these by reading the prior period's per-entity
    /// `period_close/trial_balances.json` and projecting onto BS-only
    /// positions via `datasynth_group::aggregate::extract_opening_balances`.
    pub opening_balances: Vec<EntityOpeningBalance>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use datasynth_core::models::journal_entry::{
        JournalEntry, JournalEntryHeader, JournalEntryLine,
    };
    use rust_decimal::Decimal;

    #[test]
    fn test_default_is_empty() {
        let ctx = ShardContext::default();
        assert!(ctx.entity_code.is_empty());
        assert_eq!(ctx.entity_seed, [0u8; 32]);
        assert!(ctx.extra_journal_entries.is_empty());
        assert!(ctx.opening_balances.is_empty());
    }

    #[test]
    fn test_can_hold_opening_balances_for_v53_carryover() {
        use datasynth_core::models::balance::AccountType;
        let mut ctx = ShardContext::default();
        ctx.opening_balances.push(EntityOpeningBalance {
            account_code: "1000".to_string(),
            account_type: AccountType::Asset,
            debit: Decimal::from(50_000),
            credit: Decimal::ZERO,
        });
        ctx.opening_balances.push(EntityOpeningBalance {
            account_code: "2000".to_string(),
            account_type: AccountType::Liability,
            debit: Decimal::ZERO,
            credit: Decimal::from(30_000),
        });
        assert_eq!(ctx.opening_balances.len(), 2);
        assert_eq!(ctx.opening_balances[0].account_code, "1000");
        assert_eq!(ctx.opening_balances[1].net_balance(), Decimal::from(30_000));
    }

    #[test]
    fn test_can_hold_journal_entries() {
        let header = JournalEntryHeader::new(
            "E_TEST".to_string(),
            NaiveDate::from_ymd_opt(2024, 6, 15).expect("valid date"),
        );
        let mut je = JournalEntry::new(header);
        let doc_id = je.header.document_id;
        je.add_line(JournalEntryLine::debit(
            doc_id,
            1,
            "1150".to_string(),
            Decimal::from(100),
        ));
        je.add_line(JournalEntryLine::credit(
            doc_id,
            2,
            "4500".to_string(),
            Decimal::from(100),
        ));
        assert!(je.is_balanced());

        let mut ctx = ShardContext {
            entity_code: "E_TEST".to_string(),
            entity_seed: [42u8; 32],
            extra_journal_entries: Vec::new(),
            opening_balances: Vec::new(),
        };
        ctx.extra_journal_entries.push(je);
        assert_eq!(ctx.extra_journal_entries.len(), 1);
        assert_eq!(ctx.extra_journal_entries[0].header.company_code, "E_TEST");
        assert!(ctx.extra_journal_entries[0].is_balanced());
    }
}

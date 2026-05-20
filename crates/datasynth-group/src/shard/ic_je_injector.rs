//! Turn shard-derived [`IcPairPlan`]s into balanced [`JournalEntry`]s.
//!
//! # Architecture note — deviation from the v5.0 plan
//!
//! The plan literally placed this module in `datasynth-generators`.  We host
//! it here instead:  [`IcPairPlan`] lives in `datasynth-group`, and the
//! `datasynth-generators` crate deliberately does **not** depend on
//! `datasynth-group`.  Adding such a dependency would invert the intended
//! layering (group → generators → core).  Putting the injector in
//! `datasynth-group` keeps the layering intact: the injector reaches *down*
//! to `datasynth-core::models::journal_entry`, never *up*.  The
//! orchestrator in `datasynth-runtime` consumes the output through the
//! opaque `crate::shard_context::ShardContext`-equivalent surface
//! ([`datasynth_runtime::ShardContext`]) — no group-crate types leak
//! across that boundary.
//!
//! # v5.0 scope
//!
//! - One JE per plan.  Each JE has exactly two lines: one debit, one credit.
//! - Flat account mapping via two lookup functions — [`seller_accounts`]
//!   and [`buyer_accounts`].  String literals are used (with comments
//!   pointing at the constants in [`datasynth_core::accounts`]) to keep the
//!   mapping easy to audit at a glance.  Later phases may swap these for
//!   CoA-driven lookups.
//! - No FX, no markup, no anomaly/fraud bias — the injector just wires the
//!   plan into a balanced JE in the shard's own entity code.

use rust_decimal::Decimal;

use datasynth_core::models::journal_entry::{JournalEntry, JournalEntryHeader, JournalEntryLine};

use crate::config::IcTransactionType;
use crate::shard::ic_plan::{IcPairPlan, IcRole};

// ── Types ─────────────────────────────────────────────────────────────────────

/// Context the injector needs for one shard.
///
/// v5.0 is deliberately minimal — later phases may extend this with CoA
/// lookups, FX info, etc.
#[derive(Debug, Clone)]
pub struct InjectionCtx {
    /// This shard's entity code (becomes `header.company_code`).
    pub entity_code: String,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Convert each [`IcPairPlan`] into a balanced [`JournalEntry`] for the
/// shard.
///
/// Every emitted JE:
/// - has one debit line (account from [`seller_accounts`] or
///   [`buyer_accounts`] depending on `plan.role`) and one credit line;
/// - carries `header.ic_pair_id = Some(plan.pair_id)` and
///   `header.ic_partner_entity = Some(plan.partner_entity)`;
/// - has `header.company_code == ctx.entity_code`;
/// - has `header.posting_date == plan.date`;
/// - satisfies [`JournalEntry::is_balanced`].
pub fn inject_ic_journal_entries(plans: &[IcPairPlan], ctx: &InjectionCtx) -> Vec<JournalEntry> {
    plans
        .iter()
        .map(|plan| build_je_for_plan(plan, ctx))
        .collect()
}

/// Seller-side `(DR, CR)` account pair for a given transaction type.
///
/// Accounts match the constants in
/// [`datasynth_core::accounts::control_accounts`],
/// [`datasynth_core::accounts::revenue_accounts`], and
/// [`datasynth_core::accounts::cash_accounts`].  String literals are
/// intentional for v5.0 — see the module docs.
pub fn seller_accounts(t: IcTransactionType) -> (&'static str, &'static str) {
    match t {
        // DR IC_AR_CLEARING (1150) / CR IC_REVENUE (4500)
        IcTransactionType::GoodsSale
        | IcTransactionType::ServiceProvided
        | IcTransactionType::ManagementFee
        | IcTransactionType::Royalty
        | IcTransactionType::CostSharing
        | IcTransactionType::ExpenseRecharge => ("1150", "4500"),
        // DR IC_AR_CLEARING (1150) / CR interest income (local "7000" —
        // mirrors INTEREST_EXPENSE on buyer side).
        IcTransactionType::LoanInterest => ("1150", "7000"),
        // DR OPERATING_CASH (1000) / CR OTHER_REVENUE (4900 — dividend
        // income).
        IcTransactionType::Dividend => ("1000", "4900"),
    }
}

/// Buyer-side `(DR, CR)` account pair for a given transaction type.
///
/// Accounts match the constants in
/// [`datasynth_core::accounts::control_accounts`],
/// `datasynth_core::accounts::cogs_accounts`, and
/// [`datasynth_core::accounts::expense_accounts`].
pub fn buyer_accounts(t: IcTransactionType) -> (&'static str, &'static str) {
    match t {
        // DR COGS (5000) / CR IC_AP_CLEARING (2050)
        IcTransactionType::GoodsSale => ("5000", "2050"),
        // DR IC expense (local "6800" — same slot as INSURANCE; used here
        // as the generic IC-expense landing account) / CR IC_AP_CLEARING.
        IcTransactionType::ServiceProvided
        | IcTransactionType::ManagementFee
        | IcTransactionType::Royalty
        | IcTransactionType::CostSharing
        | IcTransactionType::ExpenseRecharge => ("6800", "2050"),
        // DR INTEREST_EXPENSE (7100) / CR IC_AP_CLEARING.
        IcTransactionType::LoanInterest => ("7100", "2050"),
        // DR OPERATING_CASH (1000) / CR IC_AP_CLEARING.
        IcTransactionType::Dividend => ("1000", "2050"),
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Build one journal entry for one pair plan.  See the module docs for
/// invariants.
fn build_je_for_plan(plan: &IcPairPlan, ctx: &InjectionCtx) -> JournalEntry {
    let (dr_acct, cr_acct) = match plan.role {
        IcRole::Seller => seller_accounts(plan.transaction_type),
        IcRole::Buyer => buyer_accounts(plan.transaction_type),
    };

    let mut header = JournalEntryHeader::new(ctx.entity_code.clone(), plan.date);
    header.ic_pair_id = Some(plan.pair_id);
    header.ic_partner_entity = Some(plan.partner_entity.clone());

    // Human-readable summary.  Keep the arrow pointing seller → buyer
    // irrespective of which shard we're running on.
    let (from, to) = match plan.role {
        IcRole::Seller => (&ctx.entity_code, &plan.partner_entity),
        IcRole::Buyer => (&plan.partner_entity, &ctx.entity_code),
    };
    header.header_text = Some(format!(
        "IC {:?}: {} \u{2192} {} (pair {})",
        plan.transaction_type, from, to, plan.index,
    ));

    let mut je = JournalEntry::new(header);
    let doc_id = je.header.document_id;
    let amount: Decimal = plan.amount;
    je.add_line(JournalEntryLine::debit(
        doc_id,
        1,
        dr_acct.to_string(),
        amount,
    ));
    je.add_line(JournalEntryLine::credit(
        doc_id,
        2,
        cr_acct.to_string(),
        amount,
    ));
    je
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seller_accounts_covers_all_variants() {
        assert_eq!(
            seller_accounts(IcTransactionType::GoodsSale),
            ("1150", "4500")
        );
        assert_eq!(
            seller_accounts(IcTransactionType::ServiceProvided),
            ("1150", "4500")
        );
        assert_eq!(
            seller_accounts(IcTransactionType::ManagementFee),
            ("1150", "4500")
        );
        assert_eq!(
            seller_accounts(IcTransactionType::Royalty),
            ("1150", "4500")
        );
        assert_eq!(
            seller_accounts(IcTransactionType::CostSharing),
            ("1150", "4500")
        );
        assert_eq!(
            seller_accounts(IcTransactionType::ExpenseRecharge),
            ("1150", "4500")
        );
        assert_eq!(
            seller_accounts(IcTransactionType::LoanInterest),
            ("1150", "7000")
        );
        assert_eq!(
            seller_accounts(IcTransactionType::Dividend),
            ("1000", "4900")
        );
    }

    #[test]
    fn buyer_accounts_covers_all_variants() {
        assert_eq!(
            buyer_accounts(IcTransactionType::GoodsSale),
            ("5000", "2050")
        );
        assert_eq!(
            buyer_accounts(IcTransactionType::ServiceProvided),
            ("6800", "2050")
        );
        assert_eq!(
            buyer_accounts(IcTransactionType::ManagementFee),
            ("6800", "2050")
        );
        assert_eq!(buyer_accounts(IcTransactionType::Royalty), ("6800", "2050"));
        assert_eq!(
            buyer_accounts(IcTransactionType::CostSharing),
            ("6800", "2050")
        );
        assert_eq!(
            buyer_accounts(IcTransactionType::ExpenseRecharge),
            ("6800", "2050")
        );
        assert_eq!(
            buyer_accounts(IcTransactionType::LoanInterest),
            ("7100", "2050")
        );
        assert_eq!(
            buyer_accounts(IcTransactionType::Dividend),
            ("1000", "2050")
        );
    }
}

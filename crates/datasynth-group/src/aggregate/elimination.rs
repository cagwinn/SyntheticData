//! Elimination entry generation from matched IC pairs — Task 5.4.
//!
//! After [`crate::aggregate::ic_matcher::match_ic_pairs`] has joined the
//! seller-side and buyer-side journal entries of every IC pair into an
//! [`IcMatchedPair`], the aggregate phase needs to rewrite each pair to
//! zero in the consolidated trial balance.  This module is the IC →
//! [`EliminationEntry`] converter.
//!
//! # "For each matched pair" — clarification
//!
//! The plan's bullet "for each matched pair, generate an
//! `EliminationEntry`" is shorthand.  A single IC `GoodsSale` between
//! SA and USA produces **both** a balance-sheet pair (IC AR vs IC AP)
//! and an income-statement pair (IC revenue vs COGS).  Eliminating it
//! fully requires two journal entries — one per balance/P&L track.
//! The same logic applies to most transaction types.  So
//! [`generate_eliminations`] returns a `Vec<EliminationEntry>` of length
//! **one or two per matched pair** depending on the
//! [`IcTransactionType`].
//!
//! # Per-type elimination mapping
//!
//! Source of truth for the account codes is the IC injector at
//! [`crate::shard::ic_je_injector::seller_accounts`] and
//! [`crate::shard::ic_je_injector::buyer_accounts`].  This module
//! mirrors that mapping into the consolidation eliminations:
//!
//! | `IcTransactionType` | Eliminations emitted | Seller / buyer accounts (DR/CR) |
//! |---|---|---|
//! | `GoodsSale`         | `ICBalances` + `ICRevenueExpense` | seller `1150`/`4500`, buyer `5000`/`2050` |
//! | `ServiceProvided`   | `ICBalances` + `ICRevenueExpense` | seller `1150`/`4500`, buyer `6800`/`2050` |
//! | `ManagementFee`     | `ICBalances` + `ICRevenueExpense` | seller `1150`/`4500`, buyer `6800`/`2050` |
//! | `Royalty`           | `ICBalances` + `ICRevenueExpense` | seller `1150`/`4500`, buyer `6800`/`2050` |
//! | `CostSharing`       | `ICBalances` + `ICRevenueExpense` | seller `1150`/`4500`, buyer `6800`/`2050` |
//! | `ExpenseRecharge`   | `ICBalances` + `ICRevenueExpense` | seller `1150`/`4500`, buyer `6800`/`2050` |
//! | `LoanInterest`      | `ICBalances` + `ICInterest`       | seller `1150`/`7000`, buyer `7100`/`2050` |
//! | `Dividend`          | `ICBalances` + `ICDividends`      | seller `1000`/`4900`, buyer `1000`/`2050` |
//!
//! ## Decision: `Dividend` and `LoanInterest` go through IC clearing
//!
//! Conceptually, dividends typically flow through equity / cash rather
//! than the IC clearing accounts — but the v5.0 IC injector
//! ([`crate::shard::ic_je_injector::buyer_accounts`]) deliberately routes
//! the buyer side of `Dividend` through `IC_AP_CLEARING (2050)` so the
//! aggregate-phase matcher can find both legs by `ic_pair_id`.  Since
//! the IC AP balance is real on the buyer's books, we emit an
//! `ICBalances` entry to zero the clearing-account legs alongside the
//! `ICDividends` entry that handles the income/equity legs.  The same
//! reasoning applies to `LoanInterest`, where the buyer's
//! `INTEREST_EXPENSE (7100)` and the seller's interest income `7000`
//! are eliminated through `ICInterest`.
//!
//! # Deferred to later chunks
//!
//! - `EliminationType::ICProfitInInventory` (unrealised profit on
//!   inventory still on hand) — Chunk 6.
//! - `EliminationType::ICProfitInFixedAssets` (unrealised profit on
//!   intercompany asset transfers) — Chunk 6.
//! - `EliminationType::InvestmentEquity` (parent investment vs
//!   subsidiary equity at acquisition) — Chunk 7 (consolidation
//!   reset / startup elimination).
//! - `EliminationType::MinorityInterest` (NCI roll-forward) — Chunk 7.
//! - `EliminationType::Goodwill` (recognised at acquisition) — Chunk 7.
//! - `EliminationType::CurrencyTranslation` (CTA on
//!   [`ICAggregatedBalance`] mismatches due to FX drift) — Chunk 8.
//!
//! # Determinism
//!
//! Output `entries` is sorted by `(elimination_type, entry_id)` so two
//! callers with the same `matched` slice get byte-identical output.
//! `by_type_counts` is a [`BTreeMap`] for the same reason.

use std::collections::BTreeMap;

use chrono::Datelike;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use datasynth_core::models::intercompany::{EliminationEntry, EliminationLine, EliminationType};
use datasynth_core::models::{IcPairId, JournalEntry};

use crate::aggregate::ic_matcher::IcMatchedPair;
use crate::config::IcTransactionType;
use crate::errors::{GroupError, GroupResult};
use crate::manifest::builder::GroupManifest;
use crate::shard::ic_plan::{derive_ic_pair_plans, IcPairPlan};

// ── Public types ──────────────────────────────────────────────────────────────

/// Result of [`generate_eliminations`].
///
/// `entries` is sorted by `(elimination_type, entry_id)` for
/// determinism, and totals are the sum of every emitted entry's
/// `total_debit` / `total_credit`.  By construction
/// `total_debit == total_credit` (every emitted entry is balanced).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EliminationResult {
    /// All emitted elimination entries, sorted by
    /// `(elimination_type, entry_id)`.
    pub entries: Vec<EliminationEntry>,
    /// Sum of every entry's `total_debit`.
    pub total_debit: Decimal,
    /// Sum of every entry's `total_credit`.  Equals `total_debit` when
    /// every entry is balanced (always, by construction — verified
    /// before push).
    pub total_credit: Decimal,
    /// Per-`EliminationType` count of entries emitted.  A `BTreeMap` so
    /// serialisation is order-stable.
    pub by_type_counts: BTreeMap<EliminationType, usize>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Generate balanced [`EliminationEntry`] objects from matched IC pairs.
///
/// # Arguments
///
/// - `matched`: the slice of [`IcMatchedPair`] produced by
///   [`crate::aggregate::ic_matcher::match_ic_pairs`].  Each pair carries
///   the seller's and buyer's journal entries verbatim.
/// - `manifest`: the group manifest.  Needed to (a) re-derive the
///   [`IcPairPlan`] for each pair so we know its `IcTransactionType`
///   (which selects the eliminations to emit) and (b) source the
///   group-level fields (`group_id`, `presentation_currency`).
///
/// # Behaviour
///
/// 1. Build a per-entity `IcPairPlan` cache so lookups are O(1) per pair.
/// 2. For each [`IcMatchedPair`]:
///    - Resolve the seller's plan to discover the
///      [`IcTransactionType`].
///    - Compute the elimination amount as the seller's
///      `IC_AR_CLEARING` (or equivalent debit) line amount on the
///      seller-side JE — i.e. the actual notional that hit the
///      books, not just the planned amount.
///    - Emit one or two [`EliminationEntry`]s per the per-type table
///      above.  Every entry is verified balanced before push (rounding
///      tolerance: `0.01` of the presentation currency, matching the
///      tolerance the upstream JE constructors use).
/// 3. Sort the resulting `entries` by `(elimination_type, entry_id)`.
/// 4. Aggregate `total_debit` / `total_credit` and `by_type_counts`.
///
/// # Errors
///
/// - [`GroupError::Aggregate`] if the seller's entity has no plan for
///   `pair.pair_id` (stale shard or manifest mismatch).
/// - [`GroupError::Aggregate`] if the seller-side JE has no debit line
///   (empty / corrupted JE).
/// - [`GroupError::Aggregate`] if any emitted entry fails its balance
///   invariant — should be impossible given the factory contracts but
///   we guard it as a defensive postcondition.
pub fn generate_eliminations(
    matched: &[IcMatchedPair],
    manifest: &GroupManifest,
) -> GroupResult<EliminationResult> {
    // Per-entity plan cache keyed by entity code → (pair_id → plan).
    // The matcher already paid this cost once; rebuilding it here keeps
    // this module decoupled from the matcher's internal state.
    let mut plan_cache: BTreeMap<String, BTreeMap<IcPairId, IcPairPlan>> = BTreeMap::new();

    let mut entries: Vec<EliminationEntry> = Vec::new();

    // Group iteration — by (seller_entity, buyer_entity, transaction_type)
    // then by pair_id within — for diagnostics and to make the
    // pre-sort traversal predictable.  We re-sort entries after by
    // (elimination_type, entry_id) so the input ordering does not leak
    // into the output ordering.
    let mut sorted_pairs: Vec<&IcMatchedPair> = matched.iter().collect();
    sorted_pairs.sort_by(|a, b| {
        a.seller_entity
            .cmp(&b.seller_entity)
            .then(a.buyer_entity.cmp(&b.buyer_entity))
            .then(a.pair_id.cmp(&b.pair_id))
    });

    for pair in sorted_pairs {
        let plan = lookup_plan(
            &mut plan_cache,
            manifest,
            &pair.seller_entity,
            &pair.pair_id,
        )?;
        let amount = elimination_amount(&pair.seller_je, pair)?;
        let fiscal_period = format_fiscal_period(pair.seller_je.header.posting_date);

        for mut entry in build_entries_for_pair(pair, &plan, amount, &fiscal_period, manifest)? {
            // [`EliminationEntry::new`] (and the factory functions that
            // delegate to it) set `created_at = Utc::now()`, which would
            // make the output non-deterministic.  Pin it to the entry
            // date at 00:00:00 so two calls with the same input produce
            // byte-identical JSON.
            entry.created_at = pair
                .seller_je
                .header
                .posting_date
                .and_hms_opt(0, 0, 0)
                .expect("00:00:00 is always a valid time");
            verify_balanced(&entry, pair)?;
            entries.push(entry);
        }
    }

    // Deterministic output: sort by (elimination_type, entry_id).
    entries.sort_by(|a, b| {
        elimination_type_order(a.elimination_type)
            .cmp(&elimination_type_order(b.elimination_type))
            .then_with(|| a.entry_id.cmp(&b.entry_id))
    });

    let mut total_debit = Decimal::ZERO;
    let mut total_credit = Decimal::ZERO;
    let mut by_type_counts: BTreeMap<EliminationType, usize> = BTreeMap::new();
    for entry in &entries {
        total_debit += entry.total_debit;
        total_credit += entry.total_credit;
        *by_type_counts.entry(entry.elimination_type).or_insert(0) += 1;
    }

    Ok(EliminationResult {
        entries,
        total_debit,
        total_credit,
        by_type_counts,
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Build the (one or two) [`EliminationEntry`]s for a single matched
/// pair, dispatching on the plan's [`IcTransactionType`].
fn build_entries_for_pair(
    pair: &IcMatchedPair,
    plan: &IcPairPlan,
    amount: Decimal,
    fiscal_period: &str,
    manifest: &GroupManifest,
) -> GroupResult<Vec<EliminationEntry>> {
    let entry_date = pair.seller_je.header.posting_date;
    let consolidation_entity = manifest.group_id.clone();
    let currency = manifest.presentation_currency.clone();
    let pair_short = short_pair_id(&pair.pair_id);

    let tx = plan.transaction_type;
    let mut out = Vec::with_capacity(2);

    // Most types share the (ICBalances + ICRevenueExpense / ICInterest /
    // ICDividends) shape.  The match below picks the second-track
    // factory; the ICBalances track is uniform across types except
    // Dividend uses different clearing-account legs (handled in
    // build_balance_entry via the buyer's IC_AP_CLEARING regardless of
    // the seller's accounting path).
    match tx {
        IcTransactionType::GoodsSale
        | IcTransactionType::ServiceProvided
        | IcTransactionType::ManagementFee
        | IcTransactionType::Royalty
        | IcTransactionType::CostSharing
        | IcTransactionType::ExpenseRecharge => {
            out.push(build_balance_entry(
                pair,
                amount,
                fiscal_period,
                entry_date,
                consolidation_entity.clone(),
                currency.clone(),
                &pair_short,
            ));
            out.push(build_revenue_expense_entry(
                pair,
                tx,
                amount,
                fiscal_period,
                entry_date,
                consolidation_entity,
                currency,
                &pair_short,
            ));
        }
        IcTransactionType::LoanInterest => {
            out.push(build_balance_entry(
                pair,
                amount,
                fiscal_period,
                entry_date,
                consolidation_entity.clone(),
                currency.clone(),
                &pair_short,
            ));
            out.push(build_interest_entry(
                pair,
                amount,
                fiscal_period,
                entry_date,
                consolidation_entity,
                currency,
                &pair_short,
            ));
        }
        IcTransactionType::Dividend => {
            out.push(build_balance_entry(
                pair,
                amount,
                fiscal_period,
                entry_date,
                consolidation_entity.clone(),
                currency.clone(),
                &pair_short,
            ));
            out.push(build_dividend_entry(
                pair,
                amount,
                fiscal_period,
                entry_date,
                consolidation_entity,
                currency,
                &pair_short,
            ));
        }
    }

    Ok(out)
}

/// `ICBalances` entry: zero out the seller's `IC_AR_CLEARING` against
/// the buyer's `IC_AP_CLEARING`.  Uses
/// [`EliminationEntry::create_ic_balance_elimination`] which already
/// emits a balanced 2-line entry.
fn build_balance_entry(
    pair: &IcMatchedPair,
    amount: Decimal,
    fiscal_period: &str,
    entry_date: chrono::NaiveDate,
    consolidation_entity: String,
    currency: String,
    pair_short: &str,
) -> EliminationEntry {
    let mut entry = EliminationEntry::create_ic_balance_elimination(
        format!("ELIM-BAL-{pair_short}"),
        consolidation_entity,
        fiscal_period.to_string(),
        entry_date,
        &pair.seller_entity, // company1 = seller / receivable side
        &pair.buyer_entity,  // company2 = buyer / payable side
        IC_AR_CLEARING,
        IC_AP_CLEARING,
        amount,
        currency,
    );
    entry
        .ic_references
        .push(format!("ic_pair_id={}", pair.pair_id));
    entry
}

/// `ICRevenueExpense` entry: zero out the seller's IC revenue against
/// the buyer's matching expense / COGS account, picked from the
/// per-type table mirroring
/// [`crate::shard::ic_je_injector::buyer_accounts`].
#[allow(clippy::too_many_arguments)]
fn build_revenue_expense_entry(
    pair: &IcMatchedPair,
    tx: IcTransactionType,
    amount: Decimal,
    fiscal_period: &str,
    entry_date: chrono::NaiveDate,
    consolidation_entity: String,
    currency: String,
    pair_short: &str,
) -> EliminationEntry {
    let revenue_account = IC_REVENUE;
    let expense_account = match tx {
        IcTransactionType::GoodsSale => COGS_ACCOUNT,
        // Service / Management / Royalty / CostSharing / ExpenseRecharge all
        // hit the generic IC-expense slot 6800 in ic_je_injector::buyer_accounts.
        _ => IC_EXPENSE_ACCOUNT,
    };
    let mut entry = EliminationEntry::create_ic_revenue_expense_elimination(
        format!("ELIM-REV-{pair_short}"),
        consolidation_entity,
        fiscal_period.to_string(),
        entry_date,
        &pair.seller_entity,
        &pair.buyer_entity,
        revenue_account,
        expense_account,
        amount,
        currency,
    );
    entry
        .ic_references
        .push(format!("ic_pair_id={}", pair.pair_id));
    entry
}

/// `ICInterest` entry: zero out the seller's interest income (account
/// 7000) against the buyer's INTEREST_EXPENSE (7100).  No factory in
/// `datasynth-core` for this; we compose it from
/// [`EliminationEntry::new`] + [`EliminationEntry::add_line`] as the
/// task spec invites.
fn build_interest_entry(
    pair: &IcMatchedPair,
    amount: Decimal,
    fiscal_period: &str,
    entry_date: chrono::NaiveDate,
    consolidation_entity: String,
    currency: String,
    pair_short: &str,
) -> EliminationEntry {
    let mut entry = EliminationEntry::new(
        format!("ELIM-INT-{pair_short}"),
        EliminationType::ICInterest,
        consolidation_entity,
        fiscal_period.to_string(),
        entry_date,
        currency.clone(),
    );
    entry.related_companies = vec![pair.seller_entity.clone(), pair.buyer_entity.clone()];
    entry.description = format!(
        "Eliminate IC interest between {} and {}",
        pair.seller_entity, pair.buyer_entity
    );
    entry.ic_references = vec![format!("ic_pair_id={}", pair.pair_id)];

    // Debit the seller's interest income (reduce income).
    entry.add_line(EliminationLine {
        line_number: 1,
        company: pair.seller_entity.clone(),
        account: IC_INTEREST_INCOME.to_string(),
        is_debit: true,
        amount,
        currency: currency.clone(),
        description: format!("Eliminate IC interest income from {}", pair.buyer_entity),
    });
    // Credit the buyer's interest expense (reduce expense).
    entry.add_line(EliminationLine {
        line_number: 2,
        company: pair.buyer_entity.clone(),
        account: INTEREST_EXPENSE.to_string(),
        is_debit: false,
        amount,
        currency,
        description: format!("Eliminate IC interest expense to {}", pair.seller_entity),
    });
    entry
}

/// `ICDividends` entry: zero the seller's dividend income (account
/// 4900 — `OTHER_REVENUE` slot used by the IC injector for IC dividend
/// receipts) against the buyer's dividend-paid leg.  In v5.0 the buyer
/// side of a dividend hits the cash account 1000 / IC_AP_CLEARING 2050
/// in the IC injector — for the dividend track elimination we book
/// against retained earnings (3300) on the paying side, mirroring the
/// pattern used in
/// `datasynth-generators::intercompany::elimination_generator`.
fn build_dividend_entry(
    pair: &IcMatchedPair,
    amount: Decimal,
    fiscal_period: &str,
    entry_date: chrono::NaiveDate,
    consolidation_entity: String,
    currency: String,
    pair_short: &str,
) -> EliminationEntry {
    let mut entry = EliminationEntry::new(
        format!("ELIM-DIV-{pair_short}"),
        EliminationType::ICDividends,
        consolidation_entity,
        fiscal_period.to_string(),
        entry_date,
        currency.clone(),
    );
    entry.related_companies = vec![pair.seller_entity.clone(), pair.buyer_entity.clone()];
    entry.description = format!(
        "Eliminate IC dividend from {} to {}",
        pair.buyer_entity, pair.seller_entity,
    );
    entry.ic_references = vec![format!("ic_pair_id={}", pair.pair_id)];

    // Debit the receiving entity's dividend income (reduce income).
    // The IC injector treats the seller-side as the receiver
    // (DR cash 1000 / CR dividend income 4900), so the seller is the
    // "receiving_company" here.
    entry.add_line(EliminationLine {
        line_number: 1,
        company: pair.seller_entity.clone(),
        account: DIVIDEND_INCOME.to_string(),
        is_debit: true,
        amount,
        currency: currency.clone(),
        description: format!("Eliminate dividend income from {}", pair.buyer_entity),
    });
    // Credit retained earnings on the paying entity (restore equity).
    entry.add_line(EliminationLine {
        line_number: 2,
        company: pair.buyer_entity.clone(),
        account: RETAINED_EARNINGS.to_string(),
        is_debit: false,
        amount,
        currency,
        description: "Restore retained earnings".to_string(),
    });
    entry
}

/// Resolve the elimination amount from the seller-side JE.  By
/// construction (see [`crate::shard::ic_je_injector::build_je_for_plan`])
/// the seller's JE has exactly one debit line whose amount is the
/// pair's notional.  We take that line's `debit_amount` as the
/// authoritative elimination amount — this lets any upstream rescaling
/// (rounding, FX, manual adjustments) flow through without re-deriving
/// the amount from the manifest.
fn elimination_amount(seller_je: &JournalEntry, pair: &IcMatchedPair) -> GroupResult<Decimal> {
    seller_je
        .lines
        .iter()
        .find(|l| l.is_debit())
        .map(|l| l.debit_amount)
        .ok_or_else(|| {
            GroupError::Aggregate(format!(
                "generate_eliminations: seller-side JE for pair {} has no debit line — \
                 expected exactly one debit per IC injector contract",
                pair.pair_id
            ))
        })
}

/// Re-derive the [`IcPairPlan`] for the seller's pair_id with a per-entity
/// cache.  Mirrors [`crate::aggregate::ic_matcher::lookup_plan`] but
/// duplicated here to keep the modules independently testable.
fn lookup_plan(
    cache: &mut BTreeMap<String, BTreeMap<IcPairId, IcPairPlan>>,
    manifest: &GroupManifest,
    entity_code: &str,
    pair_id: &IcPairId,
) -> GroupResult<IcPairPlan> {
    if !cache.contains_key(entity_code) {
        let plans = derive_ic_pair_plans(manifest, entity_code);
        let by_pair: BTreeMap<IcPairId, IcPairPlan> =
            plans.into_iter().map(|p| (p.pair_id, p)).collect();
        cache.insert(entity_code.to_string(), by_pair);
    }
    let entity_plans = cache.get(entity_code).expect("just inserted");
    entity_plans.get(pair_id).cloned().ok_or_else(|| {
        GroupError::Aggregate(format!(
            "generate_eliminations: entity `{}` is the seller of pair {} but \
             the manifest derives no plan with that pair_id for that entity — \
             stale shard output or manifest mismatch",
            entity_code, pair_id
        ))
    })
}

/// `YYYYMM` from a posting date (matches the format used by the
/// existing [`EliminationEntry`] factories' tests).
fn format_fiscal_period(d: chrono::NaiveDate) -> String {
    format!("{:04}{:02}", d.year(), d.month())
}

/// First 8 hex chars of the pair_id — used in entry_id construction.
/// Not cryptographically necessary; we only need a deterministic short
/// label that's stable across runs and visually distinct between
/// neighbouring pairs.
fn short_pair_id(pair_id: &IcPairId) -> String {
    pair_id.to_hex()[..8].to_string()
}

/// Defensive balance check before pushing.  The factory functions
/// already guarantee balance, but a future refactor (or a new factory
/// added to `datasynth-core` that breaks the contract) should not
/// silently leak unbalanced eliminations into the consolidation.
fn verify_balanced(entry: &EliminationEntry, pair: &IcMatchedPair) -> GroupResult<()> {
    // Same 0.01 tolerance as the JE constructors use (rust_decimal
    // exact comparisons can fail on legitimate rounding from upstream
    // FX / markup scaling).
    let diff = (entry.total_debit - entry.total_credit).abs();
    let tolerance = Decimal::new(1, 2); // 0.01
    if diff > tolerance {
        return Err(GroupError::Aggregate(format!(
            "generate_eliminations: pair {} produced unbalanced {:?} entry \
             (total_debit={}, total_credit={}, diff={})",
            pair.pair_id, entry.elimination_type, entry.total_debit, entry.total_credit, diff
        )));
    }
    Ok(())
}

/// Stable ordering key for [`EliminationType`] — lets us sort by
/// elimination type without depending on the variant declaration order
/// in `datasynth-core`.  Smaller numbers sort first.
fn elimination_type_order(t: EliminationType) -> u8 {
    match t {
        EliminationType::ICBalances => 0,
        EliminationType::ICRevenueExpense => 1,
        EliminationType::ICInterest => 2,
        EliminationType::ICDividends => 3,
        EliminationType::ICLoans => 4,
        EliminationType::ICProfitInInventory => 5,
        EliminationType::ICProfitInFixedAssets => 6,
        EliminationType::InvestmentEquity => 7,
        EliminationType::MinorityInterest => 8,
        EliminationType::Goodwill => 9,
        EliminationType::CurrencyTranslation => 10,
    }
}

// ── GL account constants ──────────────────────────────────────────────────────
//
// These mirror the constants used by
// [`crate::shard::ic_je_injector::seller_accounts`] /
// [`crate::shard::ic_je_injector::buyer_accounts`].  See that file's
// docs for the rationale: string literals match the `datasynth_core::accounts`
// constants and keep the per-type elimination mapping easy to audit at
// a glance.

/// Seller-side IC AR clearing account (matches `IC_AR_CLEARING` /
/// `1150` in the IC injector).
const IC_AR_CLEARING: &str = "1150";
/// Buyer-side IC AP clearing account (matches `IC_AP_CLEARING` /
/// `2050`).
const IC_AP_CLEARING: &str = "2050";
/// Seller-side IC revenue (matches `IC_REVENUE` / `4500`).
const IC_REVENUE: &str = "4500";
/// Buyer-side COGS for IC goods purchases (matches `COGS` / `5000`).
const COGS_ACCOUNT: &str = "5000";
/// Buyer-side IC expense slot for non-COGS IC purchases (matches `6800`
/// in `ic_je_injector::buyer_accounts`).
const IC_EXPENSE_ACCOUNT: &str = "6800";
/// Seller-side IC interest income slot (matches `7000` in
/// `ic_je_injector::seller_accounts` for `LoanInterest`).
const IC_INTEREST_INCOME: &str = "7000";
/// Buyer-side IC interest expense slot (matches `INTEREST_EXPENSE` /
/// `7100`).
const INTEREST_EXPENSE: &str = "7100";
/// Seller-side dividend income slot (matches `OTHER_REVENUE` / `4900`
/// in `ic_je_injector::seller_accounts` for `Dividend`).
const DIVIDEND_INCOME: &str = "4900";
/// Retained earnings — used for the dividend-track elimination (mirror
/// of `datasynth-generators::intercompany::elimination_generator`'s
/// dividend handling).
const RETAINED_EARNINGS: &str = "3300";

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fiscal_period_zero_pads_month() {
        let d = chrono::NaiveDate::from_ymd_opt(2024, 3, 15).unwrap();
        assert_eq!(format_fiscal_period(d), "202403");
        let d = chrono::NaiveDate::from_ymd_opt(2024, 12, 1).unwrap();
        assert_eq!(format_fiscal_period(d), "202412");
    }

    #[test]
    fn elimination_type_order_is_a_permutation() {
        // Sanity: every variant maps to a distinct ordinal so the sort
        // is stable across runs.
        let all = [
            EliminationType::ICBalances,
            EliminationType::ICRevenueExpense,
            EliminationType::ICInterest,
            EliminationType::ICDividends,
            EliminationType::ICLoans,
            EliminationType::ICProfitInInventory,
            EliminationType::ICProfitInFixedAssets,
            EliminationType::InvestmentEquity,
            EliminationType::MinorityInterest,
            EliminationType::Goodwill,
            EliminationType::CurrencyTranslation,
        ];
        let mut keys: Vec<u8> = all.iter().map(|t| elimination_type_order(*t)).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), all.len(), "ordinals must be unique");
    }

    #[test]
    fn short_pair_id_is_eight_hex_chars() {
        let id = IcPairId::from_bytes([0xab; 32]);
        let s = short_pair_id(&id);
        assert_eq!(s.len(), 8);
        assert_eq!(s, "abababab");
    }
}

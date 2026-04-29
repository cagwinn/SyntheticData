//! Post-elimination consolidated trial balance — Task 5.6.
//!
//! After [`crate::aggregate::pre_elim::aggregate_pre_elimination`] has
//! produced an [`AggregatedTb`] (the simple sum of every Parent + Full
//! entity's standalone TB) and
//! [`crate::aggregate::elimination::eliminations_to_journal_entries`]
//! has converted the matched IC pairs into balanced GL [`JournalEntry`]
//! records, this module folds those elimination JEs into the
//! pre-elimination totals to produce the consolidated post-elimination
//! TB.
//!
//! # v5.0 narrow contract
//!
//! Only IC eliminations are applied here.  The other consolidation
//! adjustments — currency translation (CTA), NCI roll-forward, segment
//! reporting, financial-statement assembly — land in later chunks:
//!
//! - **Chunk 6**: IAS 21 currency translation (CTA), unrealised IC
//!   profit-in-inventory and profit-in-fixed-assets eliminations.
//! - **Chunk 7**: Equity-method / proportional / fair-value branches
//!   for the deferred entities, NCI measurement, goodwill and
//!   investment-equity elimination at acquisition.
//! - **Chunk 8**: Consolidated financial-statement assembly (P&L,
//!   balance sheet, cash flow), segment reporting roll-up.
//!
//! # Core behaviour
//!
//! Each elimination JE is balanced by construction (the IC elimination
//! engine emits balanced 2-line entries — [`crate::aggregate::elimination::generate_eliminations`]
//! verifies this before pushing).  Folding them line-by-line into the
//! pre-elim per-account totals therefore preserves the
//! `total_debits == total_credits` invariant.  We re-verify the
//! aggregate balance after folding as a defensive postcondition.
//!
//! # New-account behaviour
//!
//! Eliminations may post against accounts that **no contributing
//! entity** had a balance on.  The simplest example is
//! `RETAINED_EARNINGS (3300)` on a dividend elimination: the buyer has
//! cash going out and an IC payable, but neither entity hits 3300 —
//! that's a consolidation-only adjustment.  When this module sees a
//! line whose `gl_account` is not in `pre_elim.account_totals`, it
//! creates a fresh [`AggregatedAccount`] with zero starting balances
//! and applies the elimination on top.  The new account is included in
//! the post-elim TB's `account_totals` but its
//! `contributing_entities` count stays zero — eliminations are
//! group-level adjustments, not entity contributions.
//!
//! # Determinism
//!
//! Output's `account_totals` is a [`BTreeMap`] keyed by GL account
//! code, preserving deterministic iteration order across runs.  The
//! `contributing_entities` and `deferred_entities` lists are passed
//! through verbatim from the input (already sorted by
//! [`aggregate_pre_elimination`]).
//!
//! # Errors
//!
//! All failures surface as [`GroupError::Aggregate`] with a message
//! that names the offending JE / account / currency, so a grep over
//! the aggregate-phase log pinpoints the exact regression.

use rust_decimal::Decimal;

use datasynth_core::models::JournalEntry;

use crate::aggregate::pre_elim::{AggregatedAccount, AggregatedTb};
use crate::errors::{GroupError, GroupResult};

// ── Public API ────────────────────────────────────────────────────────────────

/// Fold a slice of elimination [`JournalEntry`] records into the
/// pre-elimination [`AggregatedTb`] to produce the consolidated
/// post-elimination TB.
///
/// # Arguments
///
/// - `pre_elim`: the [`AggregatedTb`] produced by
///   [`crate::aggregate::pre_elim::aggregate_pre_elimination`].  Not
///   mutated — callers may keep both pre- and post-elim views.
/// - `elim_jes`: the elimination JEs produced by
///   [`crate::aggregate::elimination::eliminations_to_journal_entries`].
///   Tolerated to also contain non-elimination entries (filter is
///   defensive — see below).
///
/// # Behaviour
///
/// 1. **Filter to elimination JEs.** Only entries with
///    `header.is_elimination == true` affect the totals.  Defensive —
///    Task 5.5's converter only emits elimination JEs, but the
///    function signature accepts a generic `&[JournalEntry]`.
/// 2. **Currency check.** Each elimination JE's `header.currency` must
///    match `pre_elim.currency`.  Mismatches error
///    ([`GroupError::Aggregate`]) — translation must happen first
///    (Chunk 6).
/// 3. **Apply each line.** For every line in every elimination JE:
///    - Look up `gl_account` in the cloned per-account map; if absent,
///      insert a zero-balance [`AggregatedAccount`] (see
///      "new-account behaviour" in module docs).
///    - Add `debit_amount` / `credit_amount` to the running totals.
///    - Recompute `net_balance = debit_total - credit_total`.
///    - **Don't** increment `contributing_entities` — eliminations
///      are group-level adjustments.
/// 4. **Recompute aggregate totals.** `total_debits` / `total_credits`
///    are re-derived from the post-fold per-account totals so they
///    stay in sync with the per-account view (and so an elimination
///    against a brand-new account is reflected without bookkeeping
///    drift).
/// 5. **Verify balance.** Each elimination JE is balanced by
///    construction; folding balanced entries into a balanced base
///    produces a balanced result.  We re-verify within `0.01`
///    tolerance — should be impossible to fail given upstream
///    contracts but guarded as a defensive postcondition.
///
/// # Errors
///
/// - [`GroupError::Aggregate`] if any elimination JE has a currency
///   that doesn't match `pre_elim.currency`.
/// - [`GroupError::Aggregate`] if the post-elim TB fails the
///   `total_debits == total_credits` invariant after folding (should
///   be impossible given upstream balance contracts).
pub fn apply_eliminations_to_tb(
    pre_elim: &AggregatedTb,
    elim_jes: &[JournalEntry],
) -> GroupResult<AggregatedTb> {
    // Clone the input — the contract is "do not mutate the caller's
    // copy".  `AggregatedTb` is `#[derive(Clone)]` so this is one
    // BTreeMap walk plus a couple of Vec clones; the cost is modest
    // for the v5.0 entity counts (≤ ~100) and the API clarity wins
    // beat shaving the clone.
    let mut post = pre_elim.clone();

    for je in elim_jes {
        if !je.header.is_elimination {
            // Defensive filter: silently ignore non-elimination JEs so
            // mixed slices don't corrupt the consolidation.  Task 5.5's
            // converter only emits elimination JEs but the function
            // signature is `&[JournalEntry]` — defend the contract.
            continue;
        }

        // Currency must match the manifest's presentation currency
        // already encoded in the pre-elim TB.  Translation is Chunk 6;
        // until then mismatches are fatal.
        if je.header.currency != post.currency {
            return Err(GroupError::Aggregate(format!(
                "apply_eliminations_to_tb: JE currency `{}` ≠ pre-elim currency \
                 `{}` — translation needed first (Chunk 6)",
                je.header.currency, post.currency,
            )));
        }

        for line in &je.lines {
            apply_line_to_account(
                &mut post,
                &line.gl_account,
                line.debit_amount,
                line.credit_amount,
            );
        }
    }

    // Re-derive aggregate totals from the per-account view so they
    // stay in sync — easier to reason about and avoids drift if a
    // future refactor changes the line-application path.
    let (total_debits, total_credits) = recompute_totals(&post);
    post.total_debits = total_debits;
    post.total_credits = total_credits;

    verify_balance_invariant(&post)?;

    Ok(post)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Apply one elimination line to `post.account_totals[account_code]`.
///
/// Creates the per-account entry from zero if it doesn't exist (an
/// elimination might post to an account no contributing entity
/// touched — see module docs).  Recomputes `net_balance` after each
/// application.  Does **not** increment `contributing_entities` —
/// eliminations are group-level adjustments, not entity contributions.
fn apply_line_to_account(
    post: &mut AggregatedTb,
    account_code: &str,
    debit_amount: Decimal,
    credit_amount: Decimal,
) {
    let entry = post
        .account_totals
        .entry(account_code.to_string())
        .or_insert_with(|| AggregatedAccount {
            account_code: account_code.to_string(),
            debit_total: Decimal::ZERO,
            credit_total: Decimal::ZERO,
            net_balance: Decimal::ZERO,
            contributing_entities: 0,
        });
    entry.debit_total += debit_amount;
    entry.credit_total += credit_amount;
    entry.net_balance = entry.debit_total - entry.credit_total;
}

/// Sum every per-account `debit_total` / `credit_total` to derive the
/// aggregate roll-up totals.  Used after every line has been applied so
/// the totals stay consistent with the per-account view.
fn recompute_totals(post: &AggregatedTb) -> (Decimal, Decimal) {
    let mut td = Decimal::ZERO;
    let mut tc = Decimal::ZERO;
    for entry in post.account_totals.values() {
        td += entry.debit_total;
        tc += entry.credit_total;
    }
    (td, tc)
}

/// Defensive postcondition: post-fold `total_debits == total_credits`.
///
/// Pre-elim is balanced (sum of balanced standalone TBs).  Each
/// elimination JE is balanced by construction (the elimination engine
/// verifies this before push).  Folding balanced into balanced is
/// balanced — but if a future refactor breaks one of those upstream
/// contracts, the regression should fail loudly here rather than leak
/// an unbalanced consolidation into Chunk 6/7/8.
fn verify_balance_invariant(post: &AggregatedTb) -> GroupResult<()> {
    let diff = (post.total_debits - post.total_credits).abs();
    let tolerance = Decimal::new(1, 2); // 0.01
    if diff > tolerance {
        return Err(GroupError::Aggregate(format!(
            "apply_eliminations_to_tb: post-elim TB unbalanced \
             (total_debits={}, total_credits={}, diff={}) — upstream \
             balance contract regression",
            post.total_debits, post.total_credits, diff,
        )));
    }
    Ok(())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    fn empty_aggregated_tb(currency: &str) -> AggregatedTb {
        AggregatedTb {
            group_id: "TEST_GROUP".to_string(),
            currency: currency.to_string(),
            as_of_date: NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            account_totals: BTreeMap::new(),
            contributing_entities: vec!["E1".to_string()],
            deferred_entities: Vec::new(),
            total_debits: Decimal::ZERO,
            total_credits: Decimal::ZERO,
        }
    }

    #[test]
    fn apply_line_creates_new_account_from_zero() {
        let mut tb = empty_aggregated_tb("CHF");
        apply_line_to_account(&mut tb, "9999", dec!(100), Decimal::ZERO);
        let acct = tb.account_totals.get("9999").expect("must be created");
        assert_eq!(acct.debit_total, dec!(100));
        assert_eq!(acct.credit_total, Decimal::ZERO);
        assert_eq!(acct.net_balance, dec!(100));
        assert_eq!(
            acct.contributing_entities, 0,
            "elimination must not bump contributing_entities"
        );
    }

    #[test]
    fn apply_line_accumulates_into_existing_account() {
        let mut tb = empty_aggregated_tb("CHF");
        tb.account_totals.insert(
            "1100".to_string(),
            AggregatedAccount {
                account_code: "1100".to_string(),
                debit_total: dec!(500),
                credit_total: Decimal::ZERO,
                net_balance: dec!(500),
                contributing_entities: 2,
            },
        );
        apply_line_to_account(&mut tb, "1100", Decimal::ZERO, dec!(200));
        let acct = tb.account_totals.get("1100").unwrap();
        assert_eq!(acct.debit_total, dec!(500));
        assert_eq!(acct.credit_total, dec!(200));
        assert_eq!(acct.net_balance, dec!(300));
        assert_eq!(
            acct.contributing_entities, 2,
            "preserve existing contributing_entities count"
        );
    }

    #[test]
    fn recompute_totals_sums_the_per_account_view() {
        let mut tb = empty_aggregated_tb("CHF");
        tb.account_totals.insert(
            "1100".to_string(),
            AggregatedAccount {
                account_code: "1100".to_string(),
                debit_total: dec!(1000),
                credit_total: Decimal::ZERO,
                net_balance: dec!(1000),
                contributing_entities: 1,
            },
        );
        tb.account_totals.insert(
            "3100".to_string(),
            AggregatedAccount {
                account_code: "3100".to_string(),
                debit_total: Decimal::ZERO,
                credit_total: dec!(1000),
                net_balance: dec!(-1000),
                contributing_entities: 1,
            },
        );
        let (td, tc) = recompute_totals(&tb);
        assert_eq!(td, dec!(1000));
        assert_eq!(tc, dec!(1000));
    }

    #[test]
    fn verify_balance_invariant_passes_on_balanced_tb() {
        let mut tb = empty_aggregated_tb("CHF");
        tb.total_debits = dec!(500);
        tb.total_credits = dec!(500);
        verify_balance_invariant(&tb).expect("balanced must pass");
    }

    #[test]
    fn verify_balance_invariant_passes_on_within_tolerance() {
        let mut tb = empty_aggregated_tb("CHF");
        tb.total_debits = dec!(500);
        tb.total_credits = dec!(500.005);
        // 0.005 < 0.01 tolerance.
        verify_balance_invariant(&tb).expect("within tolerance must pass");
    }

    #[test]
    fn verify_balance_invariant_fails_on_unbalanced_tb() {
        let mut tb = empty_aggregated_tb("CHF");
        tb.total_debits = dec!(500);
        tb.total_credits = dec!(400);
        let err = verify_balance_invariant(&tb).expect_err("unbalanced must error");
        match err {
            GroupError::Aggregate(msg) => {
                assert!(msg.contains("unbalanced"));
                assert!(msg.contains("500"));
                assert!(msg.contains("400"));
            }
            other => panic!("expected Aggregate, got {other:?}"),
        }
    }
}

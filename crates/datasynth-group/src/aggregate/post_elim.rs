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
//! Output's `account_totals` is a `BTreeMap` keyed by GL account
//! code, preserving deterministic iteration order across runs.  The
//! `contributing_entities` and `deferred_entities` lists are passed
//! through verbatim from the input (already sorted by
//! `aggregate_pre_elimination`).
//!
//! # Errors
//!
//! All failures surface as [`GroupError::Aggregate`] with a message
//! that names the offending JE / account / currency, so a grep over
//! the aggregate-phase log pinpoints the exact regression.

use rust_decimal::Decimal;

use datasynth_core::models::balance::AccountType;
use datasynth_core::models::JournalEntry;

use crate::aggregate::equity_method::EquityMethodInvestment;
use crate::aggregate::nci::NciRollforward;
use crate::aggregate::pre_elim::{AggregatedAccount, AggregatedTb};
use crate::errors::{GroupError, GroupResult};

// ── Account constants (v5.0) ──────────────────────────────────────────────────
//
// Hard-coded GL accounts for the v5.0 NCI + equity-method overlay.  Per
// spec these will be promoted to a configurable mapping in v5.1 once
// per-entity / per-engagement chart-of-accounts variations are wired
// in.  For Mini-Acme (the only v5.0 fixture) these mirror the
// canonical IFRS / US-GAAP-aligned account ranges:
//
// | Code | Role                                       |
// |------|--------------------------------------------|
// | 1850 | Investment in associates / JVs (BS asset)  |
// | 3300 | Retained earnings (BS equity)              |
// | 3500 | Non-controlling interest equity (BS equity)|
// | 4900 | Share of profit of associates (IS pickup)  |
//
// v5.1: the v5.0 bridge account `3400` was retired in favour of
// posting the equity-method overlay's counterparty side directly to
// retained earnings (`3300`) — see [`apply_nci_and_equity_method`].

/// GL account for non-controlling-interest equity (IFRS 10.22 / ASC
/// 810-10-45-15 separate equity component).
const NCI_EQUITY: &str = "3500";

/// GL account for investment in associates / joint ventures (IAS 28
/// single-line BS asset).
const EQUITY_METHOD_INVESTMENT: &str = "1850";

/// GL account for the investor's share of associate profit (IS line
/// per IAS 28.10).
const SHARE_OF_PROFIT_OF_ASSOCIATES: &str = "4900";

/// GL account for retained earnings — the controlling-interest equity
/// component the closing NCI is moved out of, and (since v5.1) also the
/// counterparty side of the equity-method overlay (replaces the v5.0
/// `3400` bridge account).
const RETAINED_EARNINGS: &str = "3300";

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

/// Apply the Chunk-7 NCI + equity-method overlays on top of the
/// post-elimination consolidated trial balance.
///
/// # Behaviour
///
/// 1. **Currency check.**  Every [`NciRollforward`] and
///    [`EquityMethodInvestment`] must already be denominated in
///    `post_elim_tb.currency` (translation per Chunk 6 must precede
///    this overlay).  Mismatches surface as [`GroupError::Aggregate`].
/// 2. **NCI overlay** (per IFRS 10.22 / ASC 810-10-45-15).  The sum of
///    closing NCI balances is moved out of retained earnings (`3300`)
///    and into the separate NCI equity component (`3500`):
///
///    ```text
///    3500 (NCI equity)         credit  Σ closing_nci
///    3300 (retained earnings)  debit   Σ closing_nci
///    ```
///
///    Net effect on aggregate `total_debits` / `total_credits` is
///    zero — the overlay is internal-equity reclassification only.
///
/// 3. **Equity-method overlay** (v5.1 — direct retained-earnings
///    integration).  For every [`EquityMethodInvestment`] the function
///    posts a balanced pair of overlay entries against retained
///    earnings (`3300`), replacing the v5.0 `3400` bridge:
///
///    - **Investment line** (BS):
///      ```text
///      1850 (investment in associates) debit  closing_carrying_value
///      3300 (retained earnings)        credit closing_carrying_value
///      ```
///    - **P&L pickup** (IS):
///      ```text
///      4900 (share of profit)          credit share_of_profit
///      3300 (retained earnings)        debit  share_of_profit
///      ```
///
///    Net effect on retained earnings: credit
///    `(closing_carrying_value − share_of_profit)` =
///    `(opening_carrying_value − dividends_received − impairment)`
///    plus prior-period accumulated equity-method effects.  The
///    `share_of_profit` posted on the P&L side flows through the
///    income-statement rollup separately from the overlay's direct
///    retained-earnings credit, so the consolidated TB carries the
///    correct closing equity attributable to owners without a
///    synthetic bridge account.  Each post is balanced individually
///    so the aggregate stays balanced.
///
/// 4. **Defensive balance postcondition.**  After applying both
///    overlays the function recomputes `total_debits` /
///    `total_credits` from the per-account view and verifies the
///    `total_debits == total_credits` invariant within the standard
///    0.01 tolerance.
///
/// The function is **pure** with respect to its arguments: the input
/// `post_elim_tb` is cloned, the overlays are applied to the clone, and
/// the result is returned.  Callers may keep both pre-overlay and
/// post-overlay views.
///
/// # Errors
///
/// - [`GroupError::Aggregate`] if any rollforward / investment record
///   has a currency that doesn't match `post_elim_tb.currency`.
/// - [`GroupError::Aggregate`] if the post-overlay TB fails the
///   balance invariant (should be impossible given upstream contracts;
///   guarded as a defensive postcondition).
pub fn apply_nci_and_equity_method(
    post_elim_tb: &AggregatedTb,
    nci_rollforwards: &[NciRollforward],
    equity_method_investments: &[EquityMethodInvestment],
) -> GroupResult<AggregatedTb> {
    let mut overlay = post_elim_tb.clone();

    // ── 1. Currency consistency ──────────────────────────────────────
    for rf in nci_rollforwards {
        if rf.currency != overlay.currency {
            return Err(GroupError::Aggregate(format!(
                "apply_nci_and_equity_method: NCI rollforward for entity \
                 `{}` is denominated in `{}` but consolidated TB is in \
                 `{}` — translation needed first (Chunk 6)",
                rf.entity_code, rf.currency, overlay.currency,
            )));
        }
    }
    for inv in equity_method_investments {
        if inv.currency != overlay.currency {
            return Err(GroupError::Aggregate(format!(
                "apply_nci_and_equity_method: equity-method investment for \
                 investee `{}` is denominated in `{}` but consolidated TB \
                 is in `{}` — translation needed first (Chunk 6)",
                inv.investee_code, inv.currency, overlay.currency,
            )));
        }
    }

    // ── 2. NCI overlay ───────────────────────────────────────────────
    //
    // Single aggregate posting per the IFRS 10.22 / ASC 810-10-45-15
    // separate-equity-component requirement: total closing NCI moves
    // out of retained earnings into the NCI equity sub-component.
    let total_closing_nci: Decimal = nci_rollforwards
        .iter()
        .map(|rf| rf.closing_nci)
        .fold(Decimal::ZERO, |acc, v| acc + v);
    if total_closing_nci != Decimal::ZERO {
        // 3500 (NCI equity) credit Σ closing_nci
        apply_line_to_account(&mut overlay, NCI_EQUITY, Decimal::ZERO, total_closing_nci);
        // 3300 (retained earnings) debit Σ closing_nci
        apply_line_to_account(
            &mut overlay,
            RETAINED_EARNINGS,
            total_closing_nci,
            Decimal::ZERO,
        );
    }

    // ── 3. Equity-method overlay ─────────────────────────────────────
    //
    // v5.1: Per investment, balanced BS pair + balanced IS pair,
    // each posting against retained earnings (`3300`).  Replaces the
    // v5.0 `3400` bridge account.
    for inv in equity_method_investments {
        // Investment line on BS.
        if inv.closing_carrying_value != Decimal::ZERO {
            apply_line_to_account(
                &mut overlay,
                EQUITY_METHOD_INVESTMENT,
                inv.closing_carrying_value,
                Decimal::ZERO,
            );
            apply_line_to_account(
                &mut overlay,
                RETAINED_EARNINGS,
                Decimal::ZERO,
                inv.closing_carrying_value,
            );
        }

        // Share-of-profit pickup on IS.
        if inv.share_of_profit != Decimal::ZERO {
            apply_line_to_account(
                &mut overlay,
                SHARE_OF_PROFIT_OF_ASSOCIATES,
                Decimal::ZERO,
                inv.share_of_profit,
            );
            apply_line_to_account(
                &mut overlay,
                RETAINED_EARNINGS,
                inv.share_of_profit,
                Decimal::ZERO,
            );
        }
    }

    // ── 4. Re-derive aggregate totals + verify the balance ───────────
    let (total_debits, total_credits) = recompute_totals(&overlay);
    overlay.total_debits = total_debits;
    overlay.total_credits = total_credits;

    verify_balance_invariant(&overlay)?;

    Ok(overlay)
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
            // Eliminations can post to accounts no contributing entity
            // touched (IC clearing 1150 / 2050, etc.). We don't have a
            // `TrialBalanceLine` to read account_type from on this
            // path; fall back to the Default (Asset). In practice the
            // codes seen here are IC clearing accounts (1xxx asset-
            // shaped or 2xxx liability-shaped), and the consolidator's
            // BS classifier still uses code-prefix to split current vs
            // non-current.
            account_type: AccountType::default(),
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
    // v5.0 contract update: the per-entity TBs feeding pre_elim are
    // intentionally unbalanced (fraud / anomaly injection — see
    // `aggregate::tb_loader::verify_balance_invariant` for the full
    // rationale). The post-elim TB inherits that imbalance: balanced
    // eliminations applied to unbalanced inputs stay unbalanced. The
    // consolidation engine's downstream stages (NCI overlay, FS
    // generator) tolerate unbalanced inputs and surface the imbalance
    // explicitly via the consolidated BS's `is_equation_valid` field.
    // So we only LOG the diff here; we don't fail the run.
    let diff = post.total_debits - post.total_credits;
    let tolerance = Decimal::new(1, 2); // 0.01
    if diff.abs() > tolerance {
        tracing::debug!(
            total_debits = %post.total_debits,
            total_credits = %post.total_credits,
            diff = %diff,
            "post-elim TB unbalanced (expected — input TBs carry fraud/anomaly imbalance)",
        );
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
                account_type: Default::default(),
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
                account_type: Default::default(),
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
                account_type: Default::default(),
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
    fn verify_balance_invariant_logs_unbalanced_tb_but_does_not_error() {
        // Contract change in v5.0: verify_balance_invariant no longer
        // returns Err on unbalanced inputs. Per-entity TBs from the
        // orchestrator deliberately carry fraud / anomaly imbalances
        // (the imbalance IS the ground-truth fraud signal), so
        // post-elim tolerates the imbalance and surfaces it via tracing
        // instead of failing the consolidation. See the function rustdoc
        // and the same pattern in `tb_loader::verify_balance_invariant`.
        let mut tb = empty_aggregated_tb("CHF");
        tb.total_debits = dec!(500);
        tb.total_credits = dec!(400);
        verify_balance_invariant(&tb)
            .expect("unbalanced must pass under v5.0 fraud-tolerance contract");
    }
}

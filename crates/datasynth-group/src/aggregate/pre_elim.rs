//! Pre-elimination trial balance aggregation — Task 5.2.
//!
//! After [`crate::aggregate::tb_loader::load_entity_trial_balance`] has
//! produced a balanced standalone [`TrialBalance`] for every entity, the
//! aggregate phase needs to combine them into a single group-level
//! pre-elimination TB.  This module is that combiner.
//!
//! # v5.0 scope
//!
//! - **Presentation currency only.**  Every contributing TB must already
//!   be denominated in the manifest's `presentation_currency`.  IAS 21
//!   currency translation is Chunk 6 — until it lands, callers must
//!   either configure all entities with a matching functional currency
//!   (Mini-Nestlé fixture: every entity CHF) or run the no-op identity
//!   translation explicitly.
//!
//! - **Parent + Full only.**  Equity-method, fair-value, and
//!   proportional consolidation entities are *not* summed in.  They are
//!   captured in [`AggregatedTb::deferred_entities`] for Chunk 7 to
//!   process via the equity / proportional consolidation engine
//!   (one-line equity pickup, share-of-net-assets, etc.).
//!
//! - **Pre-elimination.**  IC matching (Task 5.3) and elimination
//!   journal posting (Task 5.4) operate on the [`AggregatedTb`] this
//!   function produces; the post-elimination TB (Task 5.6) is the same
//!   shape with elimination JEs applied.
//!
//! # Determinism
//!
//! `account_totals` is a [`BTreeMap`] keyed by GL account code so two
//! runs over the same input serialise to byte-identical JSON.
//! `contributing_entities` and `deferred_entities` are sorted
//! lexicographically by entity code for the same reason — the caller
//! may walk `entity_tbs` in any order without affecting downstream
//! diffability.
//!
//! # Errors
//!
//! All failures surface as [`GroupError::Aggregate`] with a message
//! that names the offending entity / currency, so a grep over the
//! aggregate-phase log pinpoints which entity directory broke the
//! invariant.

use std::collections::BTreeMap;

use datasynth_core::models::balance::TrialBalance;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::config::ConsolidationMethod;
use crate::errors::{GroupError, GroupResult};
use crate::manifest::builder::GroupManifest;

// ── Public types ──────────────────────────────────────────────────────────────

/// Pre-elimination aggregated trial balance.
///
/// Sum of `Parent` + `Full` consolidation entities' per-account
/// balances, plus a sidecar list of entities that were held aside for
/// Chunk 7 (equity-method, proportional, fair-value).  Tasks 5.3
/// (IC matcher), 5.4 (elimination engine), and 5.6 (post-elim TB)
/// build directly on this shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregatedTb {
    /// Group identifier — copied from [`GroupManifest::group_id`].
    pub group_id: String,
    /// Presentation currency — copied from
    /// [`GroupManifest::presentation_currency`] (ISO 4217).
    pub currency: String,
    /// Period end date the aggregation is as of.  Taken from
    /// [`GroupManifest::period::end`].
    pub as_of_date: chrono::NaiveDate,
    /// Per-account totals across contributing entities, keyed by GL
    /// account code.  [`BTreeMap`] guarantees deterministic iteration
    /// order for downstream serialisation.
    pub account_totals: BTreeMap<String, AggregatedAccount>,
    /// Entity codes whose TBs were summed in (Parent + Full), sorted
    /// lexicographically.
    pub contributing_entities: Vec<String>,
    /// Entity codes held back for Chunk 7 special-method consolidation
    /// (equity-method, proportional, fair-value), sorted
    /// lexicographically by entity code.
    pub deferred_entities: Vec<DeferredEntity>,
    /// Sum of `total_debits` across all contributing TBs.
    pub total_debits: Decimal,
    /// Sum of `total_credits` across all contributing TBs.
    pub total_credits: Decimal,
}

/// Per-account aggregated balance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregatedAccount {
    /// GL account code (matches the key in [`AggregatedTb::account_totals`]).
    pub account_code: String,
    /// Sum of [`TrialBalanceLine::debit_balance`] across contributing
    /// entities.
    pub debit_total: Decimal,
    /// Sum of [`TrialBalanceLine::credit_balance`] across contributing
    /// entities.
    pub credit_total: Decimal,
    /// Net balance (`debit_total - credit_total`).  Sign preserved so
    /// debit-natured accounts read positive and credit-natured accounts
    /// read negative — downstream (financial-statement assembly,
    /// elimination engine) can rely on the sign.
    pub net_balance: Decimal,
    /// Number of contributing entities that posted to this account.
    /// Diagnostic only — neither IC matching nor elimination depends
    /// on it.
    pub contributing_entities: u32,
}

/// An entity held back from the [`Parent`] / [`Full`] aggregation for
/// Chunk-7 special-method consolidation.
///
/// [`Parent`]: ConsolidationMethod::Parent
/// [`Full`]: ConsolidationMethod::Full
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeferredEntity {
    /// Entity code from [`crate::manifest::builder::ManifestEntity::code`].
    pub entity_code: String,
    /// The consolidation method that held this entity back.  Carried
    /// through so Chunk 7 can route to the equity vs proportional
    /// vs fair-value branch without re-walking the manifest.
    pub method: ConsolidationMethod,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Aggregate a slice of `(entity_code, TrialBalance)` pairs into the
/// pre-elimination group TB described by [`AggregatedTb`].
///
/// The caller already knows the entity-code → TB mapping from how it
/// walked the manifest (the shard runner writes per-entity TBs under
/// directories keyed by entity code, but `TrialBalance::company_code`
/// is set from the per-entity orchestrator config and may not equal
/// the manifest entity code in all setups).  Passing the pair
/// explicitly makes the contract obvious and avoids relying on a
/// fragile naming convention.
///
/// # Behaviour
///
/// 1. Each `entity_code` must resolve to a [`crate::manifest::builder::ManifestEntity`]
///    in `manifest.ownership_graph.entities`.  Unknown codes error.
/// 2. Each contributing TB must have `currency ==
///    manifest.presentation_currency`.  Mismatches error (translation
///    is Chunk 6).
/// 3. Entities with [`ConsolidationMethod::Parent`] or
///    [`ConsolidationMethod::Full`] are summed into `account_totals`,
///    `total_debits`, and `total_credits`.
/// 4. Entities with [`ConsolidationMethod::EquityMethod`],
///    [`ConsolidationMethod::Proportional`], or
///    [`ConsolidationMethod::FairValue`] are appended to
///    `deferred_entities` and *not* summed.
/// 5. `contributing_entities` and `deferred_entities` are sorted
///    lexicographically before return so two callers that walk the
///    same manifest in different orders produce byte-identical
///    serialised output.
/// 6. Empty `entity_tbs` is *not* an error: the function returns an
///    [`AggregatedTb`] with empty maps and zero totals.  The aggregate
///    driver may legitimately invoke the combiner with an empty list
///    during recovery scenarios.
///
/// # Errors
///
/// - [`GroupError::Aggregate`] if any `entity_code` is missing from
///   the manifest's ownership graph.
/// - [`GroupError::Aggregate`] if any contributing TB has a currency
///   that does not match `manifest.presentation_currency`.
pub fn aggregate_pre_elimination(
    manifest: &GroupManifest,
    entity_tbs: &[(String, TrialBalance)],
) -> GroupResult<AggregatedTb> {
    // Empty input → empty aggregate.  Callers may legitimately invoke
    // the combiner with no entities (recovery / dry-run paths).
    let mut agg = AggregatedTb {
        group_id: manifest.group_id.clone(),
        currency: manifest.presentation_currency.clone(),
        as_of_date: manifest.period.end,
        account_totals: BTreeMap::new(),
        contributing_entities: Vec::new(),
        deferred_entities: Vec::new(),
        total_debits: Decimal::ZERO,
        total_credits: Decimal::ZERO,
    };

    for (entity_code, tb) in entity_tbs {
        let method = lookup_consolidation_method(manifest, entity_code)?;

        match method {
            ConsolidationMethod::Parent | ConsolidationMethod::Full => {
                ensure_currency_matches(manifest, entity_code, tb)?;
                accumulate_tb(&mut agg, entity_code, tb);
            }
            ConsolidationMethod::EquityMethod
            | ConsolidationMethod::Proportional
            | ConsolidationMethod::FairValue => {
                agg.deferred_entities.push(DeferredEntity {
                    entity_code: entity_code.clone(),
                    method,
                });
            }
        }
    }

    // Deterministic ordering for downstream serialisation.
    agg.contributing_entities.sort();
    agg.deferred_entities
        .sort_by(|a, b| a.entity_code.cmp(&b.entity_code));

    Ok(agg)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Look up `entity_code` in the manifest's ownership graph and return
/// its [`ConsolidationMethod`].
///
/// Errors with [`GroupError::Aggregate`] if the code is not present,
/// naming the missing code so the aggregate-phase log pinpoints the
/// drift between the loader's input list and the manifest snapshot.
fn lookup_consolidation_method(
    manifest: &GroupManifest,
    entity_code: &str,
) -> GroupResult<ConsolidationMethod> {
    manifest
        .ownership_graph
        .entities
        .iter()
        .find(|e| e.code == entity_code)
        .map(|e| e.consolidation_method)
        .ok_or_else(|| {
            GroupError::Aggregate(format!(
                "aggregate_pre_elimination: entity `{entity_code}` not in manifest"
            ))
        })
}

/// Verify the contributing TB is denominated in the manifest's
/// presentation currency.  Mismatches surface as
/// [`GroupError::Aggregate`] with a message that names the entity, the
/// TB currency, and the expected currency — so a grep over the log
/// pinpoints exactly which entity needs translating.
fn ensure_currency_matches(
    manifest: &GroupManifest,
    entity_code: &str,
    tb: &TrialBalance,
) -> GroupResult<()> {
    if tb.currency == manifest.presentation_currency {
        Ok(())
    } else {
        Err(GroupError::Aggregate(format!(
            "aggregate_pre_elimination: entity `{}` TB currency `{}` ≠ \
             presentation currency `{}` — translation needed before aggregation \
             (Chunk 6)",
            entity_code, tb.currency, manifest.presentation_currency,
        )))
    }
}

/// Add `tb` into `agg`: per-line debit/credit accumulation, total
/// roll-up, and contributing-entity recording.
///
/// Mirrors the shape of [`TrialBalance::add_line`] but accumulates
/// across entities rather than within one TB — the [`BTreeMap`] keyed
/// by `account_code` is the only deduplication we need: two lines on
/// the same code from different entities sum into one
/// [`AggregatedAccount`] with `contributing_entities` incremented once
/// per *entity* (even if a single entity were to post two lines on the
/// same account, we still count it once).
fn accumulate_tb(agg: &mut AggregatedTb, entity_code: &str, tb: &TrialBalance) {
    // Track which accounts this entity touches so we increment
    // `contributing_entities` once per entity, not once per line.
    let mut accounts_touched_by_this_entity: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    for line in &tb.lines {
        let entry = agg
            .account_totals
            .entry(line.account_code.clone())
            .or_insert_with(|| AggregatedAccount {
                account_code: line.account_code.clone(),
                debit_total: Decimal::ZERO,
                credit_total: Decimal::ZERO,
                net_balance: Decimal::ZERO,
                contributing_entities: 0,
            });
        entry.debit_total += line.debit_balance;
        entry.credit_total += line.credit_balance;
        entry.net_balance = entry.debit_total - entry.credit_total;

        if accounts_touched_by_this_entity.insert(line.account_code.clone()) {
            // First time this entity touched this account in the
            // current TB — bump the contributor count.
            entry.contributing_entities += 1;
        }
    }

    agg.total_debits += tb.total_debits;
    agg.total_credits += tb.total_credits;
    agg.contributing_entities.push(entity_code.to_string());
}

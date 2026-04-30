//! IC pair matcher — Task 5.3.
//!
//! After the shard phase has produced [`JournalEntry`]s tagged with
//! `header.ic_pair_id` for every IC posting (Task 4.2 / `ic_je_injector`),
//! the aggregate phase needs to join the seller's leg and the buyer's leg
//! of every pair into a single [`IcMatchedPair`] that the elimination
//! engine (Task 5.4) can rewrite to zero.
//!
//! # v5.0 contract: manifest-driven, no fuzzy logic
//!
//! The pair_id is a **no-tiebreak join key**.  The shard phase derives
//! pair_ids deterministically from the manifest seed
//! ([`crate::manifest::seeds::derive_ic_pair_id`]) so the seller's shard
//! and the buyer's shard produce *byte-identical* pair_ids for the same
//! logical pair (the mirror-image determinism property tested in
//! `tests/ic_matching_property.rs`).  The matcher therefore needs:
//!
//! - **No amount tolerance.**  By construction, both sides post the same
//!   `plan.amount` in their JE.  Drift can only mean an upstream bug.
//! - **No date tolerance.**  Both sides use `plan.date` verbatim.
//! - **No fuzzy entity matching.**  Roles are determined by looking up the
//!   plan associated with each side's `pair_id`.
//!
//! # v5.3 placeholder: [`UnmatchedReason::AmountDriftAboveTolerance`]
//!
//! Reserved for the emergent-fuzzy mode landing in v5.3.  In that mode
//! the shard phase will inject a small amount drift (FX-rate jitter,
//! late-posting truncation, manual journal corrections) on top of the
//! manifest-driven base, and the matcher will need to handle pairs that
//! are within tolerance but not byte-identical.  v5.0 never emits this
//! reason — see `module-level rustdoc` in the function body.
//!
//! # Coverage semantics
//!
//! `coverage = matched.len() as f64 / total_planned as f64`.  When
//! `total_planned == 0` we return `0.0` — not `1.0`.  Zero-of-zero is
//! mathematically undefined; a downstream reviewer staring at "100 %
//! coverage" on a group with no IC relationships at all would be misled
//! into thinking elimination ran successfully when in fact there was
//! nothing to eliminate.  Reporting 0.0 makes the "no IC relationships"
//! case visually distinct from "all IC relationships matched".
//!
//! # Determinism
//!
//! - `matched` is sorted lexicographically by `pair_id` hex.
//! - `unmatched` is sorted lexicographically by `pair_id` hex.
//! - `total_planned` is computed by summing
//!   `derive_ic_pair_plans(manifest, entity_code)` across every entity
//!   filtered to `IcRole::Seller` (one seller plan per logical pair).
//!
//! # Invisible loss
//!
//! Pairs whose **neither** side is observed in `entity_jes` (e.g. an
//! entity whose JEs were never loaded) are reflected only in
//! `total_planned > matched.len() + unmatched.len()`.  We don't
//! synthesize "missing both sides" reports — `unmatched` is the list of
//! sides we *observed* but failed to pair.  v5.0 is happy to leave that
//! gap signalled implicitly by the coverage figure; v5.3 may revisit.

use std::collections::BTreeMap;

use datasynth_core::models::{IcPairId, JournalEntry};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::errors::{GroupError, GroupResult};
use crate::manifest::builder::GroupManifest;
use crate::shard::ic_plan::{derive_ic_pair_plans, IcPairPlan, IcRole};

// ── Public types ──────────────────────────────────────────────────────────────

/// Result of [`match_ic_pairs`] — the joined pairs, the unmatched sides,
/// the total number of planned distinct pairs, and the coverage ratio.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcMatchResult {
    /// Successfully joined pairs, sorted lexicographically by `pair_id` hex.
    pub matched: Vec<IcMatchedPair>,
    /// Sides we observed in `entity_jes` whose counterparty we did not
    /// observe.  Sorted lexicographically by `pair_id` hex.
    pub unmatched: Vec<UnmatchedSide>,
    /// Total number of distinct logical IC pairs the manifest plans —
    /// i.e. the sum of seller-side plans across all entities (one
    /// seller plan per logical pair).  This is the denominator of
    /// `coverage`.
    pub total_planned: usize,
    /// `matched.len() as f64 / total_planned as f64`, with the
    /// degenerate `0 / 0` case explicitly mapped to `0.0` (not `1.0`)
    /// so a "no IC relationships" group is visually distinguishable
    /// from "all IC relationships matched".  See the module-level
    /// docs for the rationale.
    pub coverage: f64,
}

/// A successfully joined IC pair: both seller and buyer JEs present and
/// associated through their shared `pair_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcMatchedPair {
    /// 32-byte deterministic pair identifier shared by both sides.
    pub pair_id: IcPairId,
    /// Seller-side entity code (matches
    /// [`JournalEntry::header::company_code`] on `seller_je`).
    pub seller_entity: String,
    /// Buyer-side entity code (matches
    /// [`JournalEntry::header::company_code`] on `buyer_je`).
    pub buyer_entity: String,
    /// The seller-side journal entry, verbatim from `entity_jes`.
    pub seller_je: JournalEntry,
    /// The buyer-side journal entry, verbatim from `entity_jes`.
    pub buyer_je: JournalEntry,
}

/// An IC side we observed in the input but failed to pair with a
/// counterparty in the same input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnmatchedSide {
    /// The pair_id this orphan side was tagged with.
    pub pair_id: IcPairId,
    /// Which role the present side plays in the pair (the *missing*
    /// side has the opposite role — see [`UnmatchedReason`]).
    pub present_role: IcRole,
    /// Entity code of the side we observed.
    pub present_entity: String,
    /// The journal entry of the side we observed, verbatim from
    /// `entity_jes`.
    pub present_je: JournalEntry,
    /// What's missing.  See variants for v5.0 vs v5.3 semantics.
    pub reason: UnmatchedReason,
}

/// Why a side could not be matched.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum UnmatchedReason {
    /// We observed a seller side; the buyer side is missing from
    /// `entity_jes`.
    MissingBuyerSide,
    /// We observed a buyer side; the seller side is missing from
    /// `entity_jes`.
    MissingSellerSide,
    /// **v5.3 placeholder.**  Reserved for the emergent-fuzzy matching
    /// mode where shard-phase amount drift can leave both sides
    /// present but with non-equal amounts.  Never emitted by v5.0 —
    /// the manifest-driven contract guarantees byte-identical amounts
    /// on both sides.
    AmountDriftAboveTolerance,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Join JEs across entities by their `header.ic_pair_id` and produce a
/// matched / unmatched split with coverage.
///
/// # Arguments
///
/// - `manifest`: the group manifest the shards were derived from.  Used
///   to (a) re-derive [`IcPairPlan`]s per entity to recover each
///   pair_id's role (seller vs buyer) and (b) compute the total number
///   of planned distinct pairs for the coverage denominator.
/// - `entity_jes`: `(entity_code, jes)` pairs.  The `jes` slice may
///   contain non-IC JEs — they are silently filtered out.
///
/// # Behavior
///
/// 1. Filter inputs to JEs with `header.ic_pair_id.is_some()`.
/// 2. Group JEs by `pair_id` into a [`BTreeMap`] for deterministic
///    iteration.
/// 3. For each pair_id with one or two sides present, look up the
///    planned role of each side using a per-entity
///    [`derive_ic_pair_plans`] cache (so we re-derive at most once per
///    entity).
/// 4. Classify:
///    - **Both sides present** with the expected (seller, buyer)
///      arrangement → [`IcMatchedPair`].
///    - **One side present** → [`UnmatchedSide`] with
///      [`UnmatchedReason::MissingBuyerSide`] or
///      [`UnmatchedReason::MissingSellerSide`].
///    - **Three or more sides** OR **two sides with the same role**:
///      these are corruption signals — return
///      [`GroupError::Aggregate`] naming the pair_id and the observed
///      shape.
/// 5. Sort `matched` and `unmatched` lexicographically by `pair_id`
///    hex.
/// 6. Compute `coverage = matched.len() / total_planned`, with `0 / 0`
///    mapped to `0.0`.
///
/// # Errors
///
/// - [`GroupError::Aggregate`] if a pair_id is observed in `entity_jes`
///   but no plan in the manifest derives it (typically a stale shard
///   output or a manifest mismatch).
/// - [`GroupError::Aggregate`] if a pair_id has more than two observed
///   sides, or two sides with the same role.
pub fn match_ic_pairs(
    manifest: &GroupManifest,
    entity_jes: &[(String, Vec<JournalEntry>)],
) -> GroupResult<IcMatchResult> {
    // Per-entity plan cache: keyed by entity code.  Each entry is a map
    // from `pair_id` to the plan that owns it, so we can look up role,
    // partner_entity, etc. in O(1) per JE.
    let mut plan_cache: BTreeMap<String, BTreeMap<IcPairId, IcPairPlan>> = BTreeMap::new();

    // Group observed JEs by pair_id.  Vec values keep insertion order for
    // diagnostics on bizarre shapes; we sort the final outputs separately.
    let mut by_pair: BTreeMap<IcPairId, Vec<ObservedSide>> = BTreeMap::new();
    for (entity_code, jes) in entity_jes {
        for je in jes {
            let Some(pair_id) = je.header.ic_pair_id else {
                continue;
            };
            by_pair.entry(pair_id).or_default().push(ObservedSide {
                entity_code: entity_code.clone(),
                je: je.clone(),
            });
        }
    }

    let mut matched: Vec<IcMatchedPair> = Vec::new();
    let mut unmatched: Vec<UnmatchedSide> = Vec::new();

    for (pair_id, sides) in &by_pair {
        match sides.len() {
            1 => {
                let side = &sides[0];
                let plan = lookup_plan(&mut plan_cache, manifest, &side.entity_code, pair_id)?;
                let reason = match plan.role {
                    IcRole::Seller => UnmatchedReason::MissingBuyerSide,
                    IcRole::Buyer => UnmatchedReason::MissingSellerSide,
                };
                unmatched.push(UnmatchedSide {
                    pair_id: *pair_id,
                    present_role: plan.role,
                    present_entity: side.entity_code.clone(),
                    present_je: side.je.clone(),
                    reason,
                });
            }
            2 => {
                let plan_a =
                    lookup_plan(&mut plan_cache, manifest, &sides[0].entity_code, pair_id)?;
                let plan_b =
                    lookup_plan(&mut plan_cache, manifest, &sides[1].entity_code, pair_id)?;

                // Identify which observed side is the seller and which is
                // the buyer by consulting the plan cache.  Anything other
                // than exactly one Seller + one Buyer is corruption.
                let (seller_idx, buyer_idx) = match (plan_a.role, plan_b.role) {
                    (IcRole::Seller, IcRole::Buyer) => (0usize, 1usize),
                    (IcRole::Buyer, IcRole::Seller) => (1usize, 0usize),
                    (IcRole::Seller, IcRole::Seller) => {
                        return Err(GroupError::Aggregate(format!(
                            "match_ic_pairs: pair {} has two seller-side observations \
                             (entities `{}` and `{}`) — expected one seller + one buyer",
                            pair_id, sides[0].entity_code, sides[1].entity_code
                        )));
                    }
                    (IcRole::Buyer, IcRole::Buyer) => {
                        return Err(GroupError::Aggregate(format!(
                            "match_ic_pairs: pair {} has two buyer-side observations \
                             (entities `{}` and `{}`) — expected one seller + one buyer",
                            pair_id, sides[0].entity_code, sides[1].entity_code
                        )));
                    }
                };
                // v5.3 — fuzzy-mode amount-drift gate.  When the
                // engagement opted into `EmergentFuzzy` matching, we
                // also compare the seller's and buyer's total
                // recorded amounts.  `tolerance_percent` is the
                // maximum allowed |delta| / max(|seller|, |buyer|)
                // ratio.  When exceeded, the pair is rejected as
                // unmatched on both sides with
                // `AmountDriftAboveTolerance`.
                //
                // `ManifestDriven` (the v5.0–v5.2 default) skips this
                // check entirely — the manifest contract guarantees
                // byte-identical amounts, so any drift would be a
                // contract bug rather than an engagement-level
                // reconciliation break.
                if matches!(
                    manifest.matching.strategy,
                    crate::config::IcMatchingStrategy::EmergentFuzzy
                ) {
                    let seller_je = &sides[seller_idx].je;
                    let buyer_je = &sides[buyer_idx].je;
                    let seller_amount = seller_je.total_debit().abs();
                    let buyer_amount = buyer_je.total_debit().abs();
                    let max_amount = seller_amount.max(buyer_amount);
                    if max_amount > Decimal::ZERO {
                        let drift = (seller_amount - buyer_amount).abs();
                        let drift_ratio = drift / max_amount;
                        if drift_ratio > manifest.matching.tolerance_percent {
                            // Both sides land in `unmatched` so the
                            // coverage report attributes the drift to
                            // both entities.
                            for idx in [seller_idx, buyer_idx] {
                                let plan = lookup_plan(
                                    &mut plan_cache,
                                    manifest,
                                    &sides[idx].entity_code,
                                    pair_id,
                                )?;
                                unmatched.push(UnmatchedSide {
                                    pair_id: *pair_id,
                                    present_role: plan.role,
                                    present_entity: sides[idx].entity_code.clone(),
                                    present_je: sides[idx].je.clone(),
                                    reason: UnmatchedReason::AmountDriftAboveTolerance,
                                });
                            }
                            continue;
                        }
                    }
                }
                matched.push(IcMatchedPair {
                    pair_id: *pair_id,
                    seller_entity: sides[seller_idx].entity_code.clone(),
                    buyer_entity: sides[buyer_idx].entity_code.clone(),
                    seller_je: sides[seller_idx].je.clone(),
                    buyer_je: sides[buyer_idx].je.clone(),
                });
            }
            n => {
                let observed: Vec<String> = sides.iter().map(|s| s.entity_code.clone()).collect();
                return Err(GroupError::Aggregate(format!(
                    "match_ic_pairs: pair {} has {} observed sides ({:?}) — \
                     expected at most 2 (one seller + one buyer)",
                    pair_id, n, observed
                )));
            }
        }
    }

    // Deterministic ordering: lexicographic by pair_id hex.  IcPairId's
    // Display impl emits lowercase hex, and the underlying [u8; 32] has a
    // natural Ord that matches lexicographic hex (modulo case), so we
    // sort directly on the IcPairId field.
    matched.sort_by_key(|p| p.pair_id);
    unmatched.sort_by_key(|p| p.pair_id);

    let total_planned = total_planned_pairs(manifest);
    let coverage = if total_planned == 0 {
        0.0
    } else {
        matched.len() as f64 / total_planned as f64
    };

    Ok(IcMatchResult {
        matched,
        unmatched,
        total_planned,
        coverage,
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// One observed side of a pair: the entity that posted it and the JE.
struct ObservedSide {
    entity_code: String,
    je: JournalEntry,
}

/// Sum the number of seller-side planned pairs across every entity in
/// the manifest's ownership graph.  Each logical pair has exactly one
/// seller plan, so this is the count of distinct planned pairs.
fn total_planned_pairs(manifest: &GroupManifest) -> usize {
    manifest
        .ownership_graph
        .entities
        .iter()
        .map(|e| {
            derive_ic_pair_plans(manifest, &e.code)
                .into_iter()
                .filter(|p| p.role == IcRole::Seller)
                .count()
        })
        .sum()
}

/// Cache-aware lookup of `(entity_code, pair_id) → IcPairPlan`.
///
/// Re-derives `derive_ic_pair_plans(manifest, entity_code)` at most once
/// per entity.  Errors if the pair_id is observed but no plan exists for
/// it on this entity — that's a shard / manifest desync the aggregate
/// phase shouldn't paper over.
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
            "match_ic_pairs: entity `{}` posted JE for pair {} but the manifest \
             derives no plan with that pair_id for that entity — \
             stale shard output or manifest mismatch",
            entity_code, pair_id
        ))
    })
}

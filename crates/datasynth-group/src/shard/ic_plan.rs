//! Derive per-shard IC pair plans from the manifest (Task 3.2, spec §5.2).
//!
//! A shard belonging to entity `E` runs [`derive_ic_pair_plans`] once with
//! its own `entity_code` and receives the list of every IC pair it must
//! materialize as a journal entry — either as seller or buyer.
//!
//! # Mirror-image determinism
//!
//! The cross-shard property that ties this module to the aggregate phase:
//! running the derivation on the seller's shard and on the buyer's shard
//! produces **mirror-image** plans.  For every index `i`:
//!
//! - `pair_id`, `amount`, `date`, `transaction_type`, `ic_relationship_id`,
//!   and `index` are byte-identical across both shards.
//! - `role` is `Seller` on one side and `Buyer` on the other.
//! - `partner_entity` is swapped so each side knows its counterparty.
//!
//! This is what the aggregate-phase IC matching step relies on: matching
//! by `pair_id` yields a perfect one-to-one join with no tiebreakers needed.
//!
//! # v5.0 scope
//!
//! - The *dominant* transaction type (`types.first()`) drives pair count
//!   and amount.  Multi-type spreads (royalty + goods_sale on the same
//!   relationship) land in v5.1.
//! - Per-pair amount is flat — every pair uses [`avg_amount`] for its
//!   transaction type.  Variance lands in v5.1.
//! - Dates are spread evenly across `[period.start, period.end]` with no
//!   business-day projection.  Weekends and holidays are permitted.

use chrono::{Duration, NaiveDate};
use datasynth_core::models::IcPairId;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::config::IcTransactionType;
use crate::manifest::builder::GroupManifest;
use crate::manifest::seeds::derive_ic_pair_id;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Which side of an IC pair an entity is playing in a given transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IcRole {
    /// The entity is the seller (revenue side) of the pair.
    Seller,
    /// The entity is the buyer (expense / asset side) of the pair.
    Buyer,
}

/// One planned IC transaction that a shard will materialize as a journal
/// entry in the corresponding entity's output.
///
/// Two shards (seller's and buyer's) derive mirror-image plans for the
/// same pair — same `pair_id`, same `amount`, same `date`, same
/// `transaction_type`, opposite `role`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IcPairPlan {
    /// 32-byte deterministic pair identifier shared by both sides.
    pub pair_id: IcPairId,
    /// The `ResolvedIcRelationship.id` this pair belongs to.
    pub ic_relationship_id: String,
    /// This shard's role in the pair (seller or buyer).
    pub role: IcRole,
    /// The counterparty entity code.
    pub partner_entity: String,
    /// Transaction type (the dominant type of the relationship in v5.0).
    pub transaction_type: IcTransactionType,
    /// Flat per-pair amount (in the relationship's native units — downstream
    /// consumers apply FX / markup separately).
    pub amount: Decimal,
    /// Planned posting date, spread evenly across the engagement period.
    pub date: NaiveDate,
    /// 0-based index of this pair within the relationship.
    pub index: u64,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Derive every [`IcPairPlan`] that the shard for `entity_code` must
/// materialize — whether as seller or buyer of each relationship.
///
/// Returns an empty `Vec` when:
/// - `entity_code` is not a participant in any IC relationship.
/// - `entity_code` does not exist in `manifest.ownership_graph.entities`
///   at all (the function does a plain filter by relationship endpoints,
///   so no lookup is needed — a typo just yields an empty result).
///
/// # Ordering
///
/// - Relationships preserve their order in `manifest.ic_relationships`
///   (which is already deterministic: explicit first, then patterns in
///   YAML order).
/// - Pairs within a relationship are ordered by `index` (0..N).
///
/// # Deterministic
///
/// Two calls with the same manifest and entity code return two `Vec`s
/// that compare equal.
pub fn derive_ic_pair_plans(manifest: &GroupManifest, entity_code: &str) -> Vec<IcPairPlan> {
    let mut out: Vec<IcPairPlan> = Vec::new();

    for rel in &manifest.ic_relationships {
        // Only relationships where this entity is a participant.
        let is_seller = rel.seller == entity_code;
        let is_buyer = rel.buyer == entity_code;
        if !is_seller && !is_buyer {
            continue;
        }

        // v5.0: dominant transaction type = first type in the relationship.
        // Skip relationships whose `types` is empty (defensive — upstream
        // validation in ic_expansion.rs already rejects these, but we don't
        // want to panic if a future change ever relaxes that).
        let Some(tx_type) = rel.types.first().copied() else {
            continue;
        };

        // Avg-amount lookup.  Zero would cause division by zero below.
        let avg = avg_amount(tx_type);
        if avg.is_zero() {
            continue;
        }

        // N = round(annual_volume / avg), clamped to [1, ..].
        // If the divide or conversion yields nonsense (NaN / overflow /
        // negative), we bail out on the whole relationship rather than
        // silently generating an unreasonable number of pairs.
        let n = match compute_pair_count(rel.annual_volume, avg) {
            Some(n) => n.max(1),
            None => continue,
        };

        let role = if is_seller {
            IcRole::Seller
        } else {
            IcRole::Buyer
        };
        let partner_entity = if is_seller {
            rel.buyer.clone()
        } else {
            rel.seller.clone()
        };

        for i in 0..n {
            let pair_id_bytes = derive_ic_pair_id(manifest.group_seed, &rel.id, i);
            out.push(IcPairPlan {
                pair_id: IcPairId::from_bytes(pair_id_bytes),
                ic_relationship_id: rel.id.clone(),
                role,
                partner_entity: partner_entity.clone(),
                transaction_type: tx_type,
                amount: avg,
                date: spread_date(manifest.period.start, manifest.period.end, i, n),
                index: i,
            });
        }
    }

    out
}

/// Average per-pair amount for a given transaction type (USD, in the
/// relationship's native units — downstream FX conversion is separate).
///
/// These are the v5.0 defaults.  They drive the pair count derived from
/// `annual_volume`:
///
/// ```text
/// N = round(annual_volume / avg_amount(tx_type))
/// ```
///
/// Public so downstream consumers (tests, the aggregate phase) can verify
/// expected pair counts without having to reach into this module's private
/// constants.
///
/// | Transaction type     | avg_amount (USD) |
/// |----------------------|-----------------:|
/// | `GoodsSale`          |           50_000 |
/// | `ServiceProvided`    |           30_000 |
/// | `ManagementFee`      |           25_000 |
/// | `Royalty`            |          100_000 |
/// | `CostSharing`        |           30_000 |
/// | `LoanInterest`       |           10_000 |
/// | `Dividend`           |           75_000 |
/// | `ExpenseRecharge`    |           20_000 |
pub fn avg_amount(t: IcTransactionType) -> Decimal {
    // Integer literals are exact in Decimal, so this is trivially precise.
    match t {
        IcTransactionType::GoodsSale => Decimal::from(50_000),
        IcTransactionType::ServiceProvided => Decimal::from(30_000),
        IcTransactionType::ManagementFee => Decimal::from(25_000),
        IcTransactionType::Royalty => Decimal::from(100_000),
        IcTransactionType::CostSharing => Decimal::from(30_000),
        IcTransactionType::LoanInterest => Decimal::from(10_000),
        IcTransactionType::Dividend => Decimal::from(75_000),
        IcTransactionType::ExpenseRecharge => Decimal::from(20_000),
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Compute `round(annual_volume / avg)` as a `u64`, returning `None` on
/// any non-finite / overflow / negative scenario so the caller can skip
/// the relationship rather than blow up.
fn compute_pair_count(annual_volume: Decimal, avg: Decimal) -> Option<u64> {
    if avg.is_zero() {
        return None;
    }
    // Banker's-rounding-free: round half away from zero via `.round()`.
    let raw = annual_volume / avg;
    let rounded = raw.round();
    // `to_u64` returns None on negative, NaN, or out-of-range — exactly
    // the "unreasonable config" signal the spec calls for.
    rounded.to_u64()
}

/// Spread `n` pairs evenly across `[start, end]` (both inclusive).
///
/// For `i == 0` the result is `start`; for `i == n - 1` the result is
/// `end`.  When `n == 1` or `start == end` the function returns `start`.
/// No business-day projection happens here — weekends and holidays are
/// deliberately allowed in v5.0.
fn spread_date(start: NaiveDate, end: NaiveDate, i: u64, n: u64) -> NaiveDate {
    // `num_days` is i64; clamp to non-negative and widen to u64 for safe
    // arithmetic with `n` and `i` (both already u64).
    let period_days = (end - start).num_days().max(0) as u64;
    if n <= 1 || period_days == 0 {
        return start;
    }
    // `(period_days * i) / (n - 1)` — this is the classic
    // evenly-distribute-n-stops-on-a-line formula.  `i` ranges over
    // `0..n`, so `(n - 1)` is the divisor that forces `i=n-1` to land
    // on `period_days` exactly.
    let offset = period_days.saturating_mul(i) / (n - 1);
    start + Duration::days(offset as i64)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_avg_amount_per_transaction_type() {
        // Compile-time exhaustiveness: exercising every variant keeps the
        // constants honest — adding a new variant forces a new row here.
        assert_eq!(
            avg_amount(IcTransactionType::GoodsSale),
            Decimal::from(50_000)
        );
        assert_eq!(
            avg_amount(IcTransactionType::ServiceProvided),
            Decimal::from(30_000)
        );
        assert_eq!(
            avg_amount(IcTransactionType::ManagementFee),
            Decimal::from(25_000)
        );
        assert_eq!(
            avg_amount(IcTransactionType::Royalty),
            Decimal::from(100_000)
        );
        assert_eq!(
            avg_amount(IcTransactionType::CostSharing),
            Decimal::from(30_000)
        );
        assert_eq!(
            avg_amount(IcTransactionType::LoanInterest),
            Decimal::from(10_000)
        );
        assert_eq!(
            avg_amount(IcTransactionType::Dividend),
            Decimal::from(75_000)
        );
        assert_eq!(
            avg_amount(IcTransactionType::ExpenseRecharge),
            Decimal::from(20_000)
        );
    }

    #[test]
    fn test_spread_date_first_and_last_land_on_bounds() {
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 3, 31).unwrap();
        let n = 5u64;

        assert_eq!(spread_date(start, end, 0, n), start);
        assert_eq!(spread_date(start, end, n - 1, n), end);
    }

    #[test]
    fn test_spread_date_n_one_returns_start() {
        let start = NaiveDate::from_ymd_opt(2024, 5, 15).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 5, 31).unwrap();
        assert_eq!(spread_date(start, end, 0, 1), start);
    }

    #[test]
    fn test_spread_date_zero_length_period_returns_start() {
        let start = NaiveDate::from_ymd_opt(2024, 5, 15).unwrap();
        // Degenerate period: start == end.
        assert_eq!(spread_date(start, start, 0, 10), start);
        assert_eq!(spread_date(start, start, 5, 10), start);
        assert_eq!(spread_date(start, start, 9, 10), start);
    }

    #[test]
    fn test_spread_date_monotonic() {
        // Sanity: dates never move backwards as `i` increases.
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
        let n = 20u64;
        let mut prev = spread_date(start, end, 0, n);
        for i in 1..n {
            let here = spread_date(start, end, i, n);
            assert!(
                here >= prev,
                "dates regressed at i={}: {} < {}",
                i,
                here,
                prev
            );
            prev = here;
        }
    }

    #[test]
    fn test_compute_pair_count_rounds() {
        // 100_000 / 30_000 = 3.333... → 3
        assert_eq!(
            compute_pair_count(Decimal::from(100_000), Decimal::from(30_000)),
            Some(3)
        );
        // 200_000 / 30_000 = 6.666... → 7
        assert_eq!(
            compute_pair_count(Decimal::from(200_000), Decimal::from(30_000)),
            Some(7)
        );
        // Exact: 150_000 / 50_000 = 3.0 → 3
        assert_eq!(
            compute_pair_count(Decimal::from(150_000), Decimal::from(50_000)),
            Some(3)
        );
        // Below half: 10_000 / 50_000 = 0.2 → 0 (caller clamps to 1).
        assert_eq!(
            compute_pair_count(Decimal::from(10_000), Decimal::from(50_000)),
            Some(0)
        );
    }

    #[test]
    fn test_compute_pair_count_rejects_zero_avg() {
        assert_eq!(
            compute_pair_count(Decimal::from(100_000), Decimal::ZERO),
            None
        );
    }
}

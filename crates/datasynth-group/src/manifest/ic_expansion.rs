//! Expand intercompany.relationships patterns into concrete IC edges.
//!
//! Each pattern like `{seller_scoping_profile: significant, buyer_scoping_profile: any}`
//! fans out to every matching (seller, buyer) pair excluding self-pairs and
//! buyers whose consolidation_method is FairValue. Explicit relationships
//! win on conflict (same seller, buyer, and first transaction type).

use crate::config::{
    ConsolidationMethod, IcRelationshipConfig, IcRelationshipExplicit, IcRelationshipPattern,
    IcTransactionType, IntercompanyConfig, TransferPricingMethod,
};
use crate::errors::{GroupError, GroupResult};
use crate::manifest::expansion::ExpandedEntity;
use rust_decimal::Decimal;

/// A fully-resolved IC relationship edge with a stable id. Both explicit
/// and pattern-derived relationships land here as a flat list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIcRelationship {
    /// Stable id: blake3("icr" || group_seed || seller || buyer || types[0] (as snake_case)).
    /// Written lowercase hex so JSON round-trips cleanly.
    pub id: String,
    pub seller: String,
    pub buyer: String,
    pub types: Vec<IcTransactionType>,
    pub annual_volume: Decimal,
    pub transfer_pricing: Option<TransferPricingMethod>,
    pub markup_percent: Option<Decimal>,
    pub source: IcSource,
    /// Only populated for Pattern sources — the originating pattern's index in intercompany.relationships.
    pub pattern_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IcSource {
    Explicit,
    Pattern,
}

pub fn expand_ic_relationships(
    ic: &IntercompanyConfig,
    expanded_entities: &[ExpandedEntity],
    group_seed: u64,
) -> GroupResult<Vec<ResolvedIcRelationship>> {
    // Index entities by code for O(1) lookup.
    let by_code: std::collections::BTreeMap<&str, &ExpandedEntity> = expanded_entities
        .iter()
        .map(|e| (e.code.as_str(), e))
        .collect();

    let mut out: Vec<ResolvedIcRelationship> = Vec::new();

    // Keyed by (seller, buyer, first-type) for explicit-wins dedup.
    let mut seen: std::collections::BTreeSet<(String, String, IcTransactionType)> =
        Default::default();

    // Pass 1 — explicit relationships first.
    for rel in ic.relationships.iter() {
        if let IcRelationshipConfig::Explicit(e) = rel {
            let first_type =
                e.types.first().copied().ok_or_else(|| {
                    GroupError::Config("IC relationship has empty types".to_string())
                })?;
            if e.seller == e.buyer {
                return Err(GroupError::Config(format!(
                    "IC relationship has seller == buyer ({})",
                    e.seller
                )));
            }
            if !by_code.contains_key(e.seller.as_str()) {
                return Err(GroupError::Config(format!(
                    "IC relationship references unknown seller entity {}",
                    e.seller
                )));
            }
            if !by_code.contains_key(e.buyer.as_str()) {
                return Err(GroupError::Config(format!(
                    "IC relationship references unknown buyer entity {}",
                    e.buyer
                )));
            }
            seen.insert((e.seller.clone(), e.buyer.clone(), first_type));
            out.push(resolve_explicit(e, group_seed));
        }
    }

    // Pass 2 — pattern expansions; skip any triple already present in `seen`.
    for (idx, rel) in ic.relationships.iter().enumerate() {
        if let IcRelationshipConfig::Pattern(p) = rel {
            let first_type = p
                .types
                .first()
                .copied()
                .ok_or_else(|| GroupError::Config("IC pattern has empty types".to_string()))?;

            for seller in filter_by_pattern_side(
                expanded_entities,
                &p.pattern.seller,
                &p.pattern.seller_scoping_profile,
            ) {
                for buyer in filter_by_pattern_side(
                    expanded_entities,
                    &p.pattern.buyer,
                    &p.pattern.buyer_scoping_profile,
                ) {
                    if seller.code == buyer.code {
                        continue;
                    }
                    if buyer.consolidation_method == ConsolidationMethod::FairValue {
                        continue;
                    }
                    let triple = (seller.code.clone(), buyer.code.clone(), first_type);
                    if !seen.insert(triple.clone()) {
                        continue;
                    }
                    out.push(resolve_pattern(p, seller, buyer, idx, group_seed));
                }
            }
        }
    }

    Ok(out)
}

fn filter_by_pattern_side<'a>(
    entities: &'a [ExpandedEntity],
    fixed: &Option<String>,
    profile: &Option<String>,
) -> Vec<&'a ExpandedEntity> {
    let mut out = Vec::new();
    for e in entities {
        // Explicit entity wins: a fixed seller/buyer restricts the pool to
        // that one entity regardless of the profile filter.
        if let Some(code) = fixed.as_ref() {
            if &e.code == code {
                out.push(e);
            }
            continue;
        }
        // Profile filter: "any" (case-insensitive) acts as wildcard;
        // otherwise require exact match. If no profile specified, keep all.
        match profile.as_deref() {
            None => out.push(e),
            Some(p) if p.eq_ignore_ascii_case("any") => out.push(e),
            Some(p) if p == e.scoping_profile => out.push(e),
            Some(_) => {}
        }
    }
    out
}

fn resolve_explicit(e: &IcRelationshipExplicit, group_seed: u64) -> ResolvedIcRelationship {
    let first_type = e.types[0];
    ResolvedIcRelationship {
        id: hash_id(group_seed, &e.seller, &e.buyer, first_type),
        seller: e.seller.clone(),
        buyer: e.buyer.clone(),
        types: e.types.clone(),
        annual_volume: e.annual_volume,
        transfer_pricing: e.transfer_pricing,
        markup_percent: e.markup_percent,
        source: IcSource::Explicit,
        pattern_index: None,
    }
}

fn resolve_pattern(
    p: &IcRelationshipPattern,
    seller: &ExpandedEntity,
    buyer: &ExpandedEntity,
    pattern_idx: usize,
    group_seed: u64,
) -> ResolvedIcRelationship {
    let first_type = p.types[0];
    ResolvedIcRelationship {
        id: hash_id(group_seed, &seller.code, &buyer.code, first_type),
        seller: seller.code.clone(),
        buyer: buyer.code.clone(),
        types: p.types.clone(),
        annual_volume: p.per_pair_volume,
        transfer_pricing: p.transfer_pricing,
        markup_percent: None,
        source: IcSource::Pattern,
        pattern_index: Some(pattern_idx),
    }
}

fn hash_id(group_seed: u64, seller: &str, buyer: &str, type_: IcTransactionType) -> String {
    let type_str = match type_ {
        IcTransactionType::GoodsSale => "goods_sale",
        IcTransactionType::ServiceProvided => "service_provided",
        IcTransactionType::ManagementFee => "management_fee",
        IcTransactionType::Royalty => "royalty",
        IcTransactionType::CostSharing => "cost_sharing",
        IcTransactionType::LoanInterest => "loan_interest",
        IcTransactionType::Dividend => "dividend",
        IcTransactionType::ExpenseRecharge => "expense_recharge",
    };
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"icr");
    hasher.update(&group_seed.to_le_bytes());
    hasher.update(seller.as_bytes());
    hasher.update(b"|");
    hasher.update(buyer.as_bytes());
    hasher.update(b"|");
    hasher.update(type_str.as_bytes());
    format!("ICR_{}", hex::encode(&hasher.finalize().as_bytes()[..8]))
    // 8 bytes = 16 hex chars → 64 bits of disambiguation, plenty for a single group.
}

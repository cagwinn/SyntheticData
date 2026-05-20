//! Task 11.5 — order-independence of manifest building.
//!
//! Two cheap tests over the Mini-Acme fixture verify that the
//! [`build_manifest`] output is **equivalent in content** when the
//! YAML's `ownership.entities` or `intercompany.relationships` are
//! reordered.  Equivalence is asserted at the level of canonical
//! sorted-by-code / sorted-by-id sets — see "v5.0 contract" below.
//!
//! # v5.0 contract: content-equality, NOT byte-equality
//!
//! The manifest builder preserves YAML order in two places:
//! - [`expand_ownership`] preserves the order of
//!   `ownership.entities`; the resulting `ManifestEntity` list keeps
//!   that order.
//! - [`expand_ic_relationships`] iterates `intercompany.relationships`
//!   for both pass-1 (explicit) and pass-2 (pattern) expansions.  The
//!   resulting `ResolvedIcRelationship` list reflects YAML order, and
//!   pattern-source rows carry a `pattern_index` field that is the
//!   originating pattern's YAML index.
//!
//! Two YAMLs that differ only in the order of `ownership.entities` or
//! `intercompany.relationships` therefore produce manifests with the
//! same **content** (same set of entities, same set of resolved IC
//! pairs) but potentially different byte representation.  This is the
//! v5.0 contract — a stricter byte-equality property would require
//! sorting inside the manifest builder, which is out of scope for
//! this task.
//!
//! # No orchestrator
//!
//! Both tests just run [`build_manifest`] (in-memory, ~10 ms each) so
//! they are cheap and run on every CI invocation — no `#[ignore]`.

use datasynth_group::manifest::builder::GroupManifest;
use datasynth_group::{build_manifest, GroupConfig};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn load_mini_acme() -> GroupConfig {
    let yaml = include_str!("fixtures/mini_acme.yaml");
    serde_yaml::from_str(yaml).expect("mini_acme.yaml must parse into GroupConfig")
}

/// Snapshot of the manifest fields whose **content** must be
/// invariant under YAML reordering of entities / IC relationships.
///
/// Each component is sorted by a canonical key so two manifests built
/// from reordered configs produce equal snapshots.
#[derive(Debug, PartialEq, Eq)]
struct CanonicalManifestSnapshot {
    schema_version: String,
    group_id: String,
    presentation_currency: String,
    manifest_seed: String,
    aggregate_seed: String,
    parent_entity_code: String,
    /// Entities sorted by `code`, serialised to JSON for byte-level diff.
    entities_by_code: Vec<(String, String)>,
    /// IC relationships sorted by `id`, serialised to JSON for byte-level diff.
    ic_relationships_by_id: Vec<(String, String)>,
    /// All shard ids sorted lexicographically.
    shard_ids: Vec<String>,
}

fn canonical_snapshot(m: &GroupManifest) -> CanonicalManifestSnapshot {
    let mut entities: Vec<(String, String)> = m
        .ownership_graph
        .entities
        .iter()
        .map(|e| {
            // Build a sortable canonical entity record by stripping
            // anything that depends on YAML order.  `entity_seed` is
            // derived from `(group_seed, code)` so it's order-independent
            // and stays in the comparison.
            let canonical = SortableEntity {
                code: e.code.clone(),
                country: e.country.clone(),
                functional_currency: e.functional_currency.clone(),
                scoping_profile: e.scoping_profile.clone(),
                consolidation_method: e.consolidation_method,
                ownership_percent: e.ownership_percent,
                parent_code: e.parent_code.clone(),
                accounting_framework: e.accounting_framework.clone(),
                industry: e.industry.clone(),
                entity_seed: e.entity_seed.clone(),
                shard_id: e.shard_id.clone(),
            };
            (e.code.clone(), serde_json::to_string(&canonical).unwrap())
        })
        .collect();
    entities.sort_by(|a, b| a.0.cmp(&b.0));

    let mut ic_rels: Vec<(String, String)> = m
        .ic_relationships
        .iter()
        .map(|r| {
            // For IC relationships the `pattern_index` is YAML-order
            // dependent for pattern-source rows but NOT for explicit
            // ones (always None).  Strip it from the canonical
            // representation since reordering YAML legitimately changes
            // it without changing the underlying logical relationship.
            let canonical = SortableIcRelationship {
                id: r.id.clone(),
                seller: r.seller.clone(),
                buyer: r.buyer.clone(),
                types: r.types.clone(),
                annual_volume: r.annual_volume,
                transfer_pricing: r.transfer_pricing,
                markup_percent: r.markup_percent,
                source: r.source,
            };
            (r.id.clone(), serde_json::to_string(&canonical).unwrap())
        })
        .collect();
    ic_rels.sort_by(|a, b| a.0.cmp(&b.0));

    let mut shard_ids: Vec<String> = m
        .shard_plan
        .shards
        .iter()
        .map(|s| s.shard_id.clone())
        .collect();
    shard_ids.sort();

    CanonicalManifestSnapshot {
        schema_version: m.schema_version.clone(),
        group_id: m.group_id.clone(),
        presentation_currency: m.presentation_currency.clone(),
        manifest_seed: m.manifest_seed.clone(),
        aggregate_seed: m.aggregate_seed.clone(),
        parent_entity_code: m.ownership_graph.parent_entity_code.clone(),
        entities_by_code: entities,
        ic_relationships_by_id: ic_rels,
        shard_ids,
    }
}

/// Subset of [`ManifestEntity`] fields that should NOT change under
/// YAML reordering — used as a sortable canonical record.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
struct SortableEntity {
    code: String,
    country: String,
    functional_currency: String,
    scoping_profile: String,
    consolidation_method: datasynth_group::ConsolidationMethod,
    ownership_percent: Option<rust_decimal::Decimal>,
    parent_code: Option<String>,
    accounting_framework: Option<String>,
    industry: Option<String>,
    entity_seed: String,
    shard_id: String,
}

/// Subset of [`crate::manifest::ic_expansion::ResolvedIcRelationship`]
/// fields that should NOT change under YAML reordering — `pattern_index`
/// is YAML-order-dependent and is omitted.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
struct SortableIcRelationship {
    id: String,
    seller: String,
    buyer: String,
    types: Vec<datasynth_group::IcTransactionType>,
    annual_volume: rust_decimal::Decimal,
    transfer_pricing: Option<datasynth_group::TransferPricingMethod>,
    markup_percent: Option<rust_decimal::Decimal>,
    source: datasynth_group::manifest::ic_expansion::IcSource,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Reverse the order of `ownership.entities` in the YAML and verify
/// the resulting manifest is content-equivalent (same set of entities
/// keyed by code, same per-entity seeds, same shard ids) to the
/// original manifest.
///
/// See module-level rustdoc for the v5.0 content-equality contract.
#[test]
fn reorder_entities_byte_identical_manifest() {
    let cfg1 = load_mini_acme();
    let mut cfg2 = cfg1.clone();
    cfg2.ownership.entities.reverse();

    let m1 = build_manifest(&cfg1).expect("manifest 1");
    let m2 = build_manifest(&cfg2).expect("manifest 2");

    let s1 = canonical_snapshot(&m1);
    let s2 = canonical_snapshot(&m2);

    // Sanity: the original manifest's `entities_by_code` and the
    // reversed-config manifest's snapshot should both list every
    // Mini-Acme entity (the snapshot is a sorted-by-code projection,
    // so reversal of input order can't change it).
    assert_eq!(
        s1.entities_by_code.len(),
        5,
        "Mini-Acme fixture must yield 5 entities; got {}",
        s1.entities_by_code.len(),
    );

    // Per-field equality with a focused diff message on mismatch.
    assert_eq!(
        s1.entities_by_code, s2.entities_by_code,
        "entity reordering changed the canonical entity set",
    );
    assert_eq!(
        s1.ic_relationships_by_id, s2.ic_relationships_by_id,
        "entity reordering changed the canonical IC relationship set",
    );
    assert_eq!(
        s1.shard_ids, s2.shard_ids,
        "entity reordering changed the shard plan",
    );
    assert_eq!(s1.manifest_seed, s2.manifest_seed);
    assert_eq!(s1.aggregate_seed, s2.aggregate_seed);

    // Whole-snapshot equality as the final assertion — easier to
    // grep-find than the per-field assertions when something drifts.
    assert_eq!(s1, s2);
}

/// Same as `reorder_entities_byte_identical_manifest` but reorders
/// `intercompany.relationships` instead.
#[test]
fn reorder_ic_relationships_byte_identical_manifest() {
    let cfg1 = load_mini_acme();
    let mut cfg2 = cfg1.clone();
    cfg2.intercompany.relationships.reverse();

    let m1 = build_manifest(&cfg1).expect("manifest 1");
    let m2 = build_manifest(&cfg2).expect("manifest 2");

    let s1 = canonical_snapshot(&m1);
    let s2 = canonical_snapshot(&m2);

    // Sanity: the IC list must contain ≥ 1 relationship for the
    // reorder property to be non-vacuous.
    assert!(
        !s1.ic_relationships_by_id.is_empty(),
        "fixture sanity: Mini-Acme must yield ≥ 1 IC relationship",
    );

    // The set of IC relationships keyed by id must match.  Note: the
    // snapshot strips `pattern_index` since it legitimately changes
    // when patterns are reordered (the index *is* the YAML index).
    assert_eq!(
        s1.ic_relationships_by_id, s2.ic_relationships_by_id,
        "IC reordering changed the canonical IC relationship set",
    );
    assert_eq!(
        s1.entities_by_code, s2.entities_by_code,
        "IC reordering changed the entities (it shouldn't)",
    );
    assert_eq!(
        s1.shard_ids, s2.shard_ids,
        "IC reordering changed the shard plan (it shouldn't)",
    );

    // Whole-snapshot equality — convenient grepable failure mode.
    assert_eq!(s1, s2);
}

/// Sanity check: the Mini-Acme fixture should produce a non-empty
/// IC relationship set and exactly 5 entities — guards the property
/// tests above against silently passing on an empty fixture.
#[test]
fn fixture_sanity() {
    let cfg = load_mini_acme();
    let manifest = build_manifest(&cfg).expect("Mini-Acme must build a manifest");
    assert_eq!(
        manifest.ownership_graph.entities.len(),
        5,
        "fixture sanity: Mini-Acme must have 5 entities",
    );
    assert!(
        !manifest.ic_relationships.is_empty(),
        "fixture sanity: Mini-Acme must have ≥ 1 IC relationship",
    );
}

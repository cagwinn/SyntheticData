# Group Audit Simulation — Design Spec

**Date:** 2026-04-23
**Status:** Draft (post-brainstorming approval)
**Target releases:** v5.0 → v5.3 (foundation → fleet)
**Companion docs:** [`docs/engagements/nestle-group-audit.md`][nestle-eng] and [`docs/engagements/ds-team-work-for-big4-group-audits.md`][ds-gap] in the VynFi Python SDK repo

[nestle-eng]: file:///home/michael/DEV/Repos/VynFi-python/VynFi-python/docs/engagements/nestle-group-audit.md
[ds-gap]: file:///home/michael/DEV/Repos/VynFi-python/VynFi-python/docs/engagements/ds-team-work-for-big4-group-audits.md

## 1. Overview

DataSynth today generates high-fidelity synthetic financial data for a *single* consolidated entity (or a flat list of independent entities). Real Big-4 group audits — Nestlé, Allianz, Unilever — require a qualitatively different deliverable: a **consolidated group** of 50–2,000 legal entities with ownership hierarchies, multi-currency functional reporting, cross-entity intercompany eliminations, non-controlling interests, ISA 600 component audit coordination, and group-level tax (Pillar 2, CbCR, transfer pricing).

This spec describes the engineering that transforms DataSynth from *entity-level synth* to **group-level synth**: a new `datasynth-group` crate that orchestrates manifest → shard → aggregate phases on top of the existing `EnhancedOrchestrator`, a `datasynth-fleet` crate for distributed execution, and the supporting config surface, artifacts, and audit-standards coverage that make the output recognizable to an audit partner on their first look.

The work lands across four releases (v5.0–v5.3), each shippable on its own.

### 1.1 Primary use case

Simulate a group engagement end-to-end: the group's consolidated financial statements (IFRS 10/11/27/28 compliant), the underlying per-entity books and records (ISA 315 / 330 / 500 evidence), the ISA 600 group audit machinery (component auditors, instructions, reports, group opinion, KAMs), and the multi-jurisdictional tax disclosure package (Pillar 2, CbCR, TP) — all bit-deterministically reproducible from a single group seed.

### 1.2 Goals

- Model an entity hierarchy (parent / subsidiaries / associates / JVs) with explicit ownership percentages and consolidation methods
- Support 50–2,000 entities in one logical group engagement
- Cross-entity intercompany elimination with deterministic pair matching (target: ≥98 % coverage)
- IAS 21 currency translation + CTA accumulation at group level
- NCI rollforward for non-wholly-owned fully-consolidated subsidiaries
- ISA 600 component auditor coordination (lead auditor → component auditors → component reports → aggregated misstatements → group opinion)
- Group-level tax: Pillar 2 aggregator, CbCR, transfer pricing master file + local files
- Per-entity accounting framework variety (IFRS, US GAAP, HGB, PCG, Swiss OR) with automatic GAAP bridges
- Bit-identical output across in-process / subprocess / distributed execution modes
- One archive artifact with per-entity subtrees, not N × per-entity archives
- Zero breaking changes to existing single-entity workflows

### 1.3 Non-goals

- Live production SAP/ERP integration (we emit SAP CSV; we don't post to a live system)
- Real-time/streaming group generation during the engagement (batch generation only)
- Custom per-firm branding on output files beyond what `exportFormat` already supports
- IFRS 17 insurance contracts (separately scoped; Tier 2 item 2.2 from the gap analysis)
- Cross-period engagement continuity as a first-class abstraction (optional: prior-period NCI rollforward accepted as input; full multi-period "engagement" concept deferred to a potential v5.4+)
- SDK work (Python/TS/C#/Rust) — no breaking SDK changes; all new config passes through verbatim, new archive readers are additive per SDK

## 2. Architecture

### 2.1 Crate layering

```
datasynth-fleet             [new, Phase 4 (v5.3)]   Distribution: dispatchers, worker adapters
    └── datasynth-group     [new, Phase 1 (v5.0)]   Group lifecycle: manifest / shard / aggregate
            ├── datasynth-runtime                    (unchanged — per-entity EnhancedOrchestrator)
            ├── datasynth-audit-fsm                  (used; +group_fsa blueprint in v5.1)
            ├── datasynth-generators                 (unchanged)
            ├── datasynth-standards                  (extended with multi-GAAP bridges in v5.2)
            ├── datasynth-core                       (extended: GroupManifest, IC pair IDs, Pillar 2 models)
            └── datasynth-output                     (extended: per-entity subtree mode)
```

The existing `EnhancedOrchestrator` is **not modified** for the shard path — it is invoked verbatim per-entity, with its input config constructed from the manifest and its output directory rooted under `entities/{code}/`. All group-level concerns (ownership, IC eliminations, consolidation, translation, group audit, group tax) live above it in `datasynth-group`.

### 2.2 Three-phase flow

```
 GroupConfig ──▶ [MANIFEST] ──▶ GroupManifest ──▶ [SHARD × N] ──▶ ShardResults ──▶ [AGGREGATE] ──▶ GroupArchive
   (YAML)       manifest_seed    (JSON ~10–50MB)   entity_seed       (per-entity       aggregate_seed    (consolidation,
                                                   per shard)        archives)                           audit pack,
                                                                                                         tax pack)
```

Phases are explicit, strongly typed, and independently invocable via CLI:

```
datasynth-data group manifest  --config group.yaml                              --out ./manifest.json
datasynth-data group shard     --manifest ./manifest.json --shard S_A_0001      --out ./shards/S_A_0001/
datasynth-data group aggregate --manifest ./manifest.json --shards ./shards/    --out ./group_archive/
datasynth-data group generate  --config group.yaml                              --out ./group_archive/
# `group generate` is a convenience that runs 1→2→3 in-process for small-to-mid groups.
```

### 2.3 Why three phases

1. **Manifest** is the only phase with global state: ownership graph resolution (patterns + generated expansion), shared-master ID allocations, FX rate master, IC relationship edges with pattern expansion, audit engagement plan (group materiality + component allocations), tax group plan (Pillar 2 jurisdictions, CbCR scope, TP policy), shard plan, entity seed derivation. Generated once. Immutable input to everything downstream.
2. **Shard** is embarrassingly parallel. One process per shard, typically 5–500 entities per shard. Reads the manifest read-only; emits per-entity archives. Calls the existing orchestrator unchanged.
3. **Aggregate** runs after all shards complete. Reads manifest + all shard outputs; emits group-level artifacts (consolidation, ISA 600 audit pack, tax pack). Single process, bounded CPU/RAM.

This is a classic map-reduce. Phase 4 (fleet) is a distributed scheduler over this same three-phase contract — in-process rayon, subprocess spawn, remote HTTP/gRPC — swappable via a `Dispatcher` trait.

### 2.4 Seed tree

All determinism derives from one user-supplied `group_seed` (64-bit):

```
group_seed (u64)
    │
    ├── manifest_seed = blake3("manifest" ‖ group_seed ‖ period_start)
    │      └── used for: ownership graph population, ID allocations, FX rates,
    │                    IC pair plan layout, audit materiality allocations
    │
    ├── entity_seed[code] = blake3("entity" ‖ group_seed ‖ entity_code)
    │      └── order-independent, stable per entity; passed to EnhancedOrchestrator
    │
    └── aggregate_seed = blake3("aggregate" ‖ group_seed ‖ period_start)
           └── used for: IC matching tiebreaks, component auditor assignment tiebreaks
```

Property: **adding or removing an entity, or reordering entities in YAML, does not change any other entity's output.** Critical for regression testing and debugging.

## 3. `group:` config schema

### 3.1 Illustrative example (Nestlé-class)

```yaml
group:
  id: "NESTLE_2024_Q1"
  name: "Nestlé S.A. Consolidated"
  presentation_currency: "CHF"
  period: { start_date: "2024-01-01", length: quarterly, fiscal_year_end: "2024-12-31" }
  seed: 0xDEADBEEF

  # Three-level inheritance: defaults ⊕ scoping_profile ⊕ per-entity overrides

  defaults:
    accounting_framework: ifrs
    industry: manufacturing
    process_models: [o2c, p2p, h2r, r2r, audit]
    fraud: { fraud_rate: 0.003, document_fraud_rate: 0.003 }
    accounting_standards: { enabled: true, leases_enabled: true }

  scoping_profiles:
    significant:                          # ISA 600 "significant component"
      row_budget: 8_000_000
      process_models: [o2c, p2p, h2r, r2r, s2c, manufacturing, banking, audit]
      audit: { generate_workpapers: true, min_team_size: 12, max_team_size: 24 }
      tax: { pillar_two: true }
      treasury: { hedge_accounting: true }
      esg: { enabled: true }
      llm: { enrichment_enabled: true }
    material:                             # material non-significant
      row_budget: 750_000
      process_models: [o2c, p2p, h2r, audit]
    consolidation_only:                   # GL consolidation package only
      row_budget: 30_000
      process_models: [o2c, p2p]
      audit: { enabled: false }

  ownership:
    parent_entity_code: NESTLE_SA
    entities:                             # explicit stanzas for notable entities
      - { code: NESTLE_SA,        country: CH, functional_currency: CHF,
          scoping_profile: significant, consolidation_method: parent }
      - { code: NESPRESSO_SA,     country: CH, functional_currency: CHF,
          scoping_profile: significant, consolidation_method: full,
          ownership_percent: 1.0, parent_code: NESTLE_SA, rows: 8_000_000 }
      - { code: NESTLE_USA,       country: US, functional_currency: USD,
          scoping_profile: significant, consolidation_method: full,
          accounting_framework: us_gaap, parent_code: NESTLE_SA }
      - { code: NESTLE_WATERS_BR, country: BR, functional_currency: BRL,
          scoping_profile: material, consolidation_method: full,
          ownership_percent: 0.80, parent_code: NESTLE_SA }   # 20 % NCI
      - { code: CEREAL_PARTNERS_WW, consolidation_method: equity_method,
          ownership_percent: 0.50, parent_code: NESTLE_SA }

    generated:                            # bulk entity generation for scale
      - { count: 200, code_prefix: NESTLE_EU_,    scoping_profile: material,
          country: [DE, FR, IT, ES, PL, NL], functional_currency: EUR,
          consolidation_method: full, ownership_percent_range: [0.85, 1.00] }
      - { count: 1750, code_prefix: NESTLE_LOCAL_, scoping_profile: consolidation_only,
          country: [CN, IN, JP, BR, MX, ZA, ID, TH, VN, TR, SA, AE],
          consolidation_method: full }

    entities_from: ./entities.csv         # optional: external entity list import

  intercompany:
    relationships:
      - { seller: NESPRESSO_SA, buyer: NESTLE_USA,  types: [goods_sale, royalty],
          annual_volume: 50_000_000, transfer_pricing: cost_plus, markup_percent: 0.08 }
      - { pattern: { seller_scoping_profile: significant, buyer_scoping_profile: any },
          types: [management_fee], per_pair_volume: 1_000_000 }
    matching: { strategy: manifest_driven, coverage_target: 0.98 }

  fx:
    base_currency: CHF
    rate_source: inline                   # inline | user_supplied | historical_series
    rates:
      "CHF/USD": { "2024-01-31": 0.8870, "2024-02-29": 0.8812, "2024-03-31": 0.9012 }
      "CHF/EUR": { "2024-01-31": 0.9520, "2024-02-29": 0.9488, "2024-03-31": 0.9611 }
    policy: { balance_sheet: closing, income_statement: average, equity: historical }

  audit:
    engagement_id: "EY_NESTLE_2024_Q1"
    lead_auditor: EY_ZURICH
    framework: isa
    fsm_blueprint: "builtin:group_fsa"    # v5.1: ISA 600 group audit blueprint
    group_materiality: { basis: revenue, percent: 0.005 }
    component_scope_thresholds: { full_scope: 0.15, specific_scope: 0.05 }
    generate_kams: true
    generate_group_opinion: true

  tax:
    pillar_two:        { enabled: true, jurisdictions: [CH, DE, FR, IT, ES, NL, US, UK, JP] }
    cbc_report:        { enabled: true, reporting_jurisdiction: CH }
    transfer_pricing:  { master_file: true, local_files_for: [CH, US, DE, FR, BR, JP] }

  output:
    layout: per_entity_subtree            # per_entity_subtree | flat
    shared_masters_at_root: true
    compression: parquet                  # json | csv | parquet
```

### 3.2 Design decisions

1. **Three-level inheritance** — `defaults` → `scoping_profiles` → per-entity overrides. Matches how real engagements are scoped (materiality tiers, not bespoke per-entity configs). A 2,000-entity group is a ~300-line YAML, not 300 KB.
2. **`ownership.entities` is authoritative.** When `group:` is present, the legacy top-level `companies:` is auto-derived from `group.ownership.entities` for backward-compat readers and ignored if inconsistent.
3. **Bulk `generated:` blocks** — 1,750 "local" entities described once with country lists and ownership ranges; deterministic (same seed → same generated codes/attributes).
4. **`entities_from: ./file.csv|yaml`** escape hatch — real engagements typically import the entity master from the client.
5. **`consolidation_method` drives rollup semantics** — `parent | full | equity_method | proportional | fair_value`. Controls whether the entity's TB rolls up fully, as a single investment line, or with proportional P&L treatment. Required for IFRS 10 / 11 / 28 compliance.
6. **Per-entity `accounting_framework` override** enables multi-GAAP groups (US entities on US GAAP, German on HGB, French on PCG, Swiss on Swiss OR) with automatic GAAP bridges at aggregate time.
7. **IC relationships support explicit + pattern expansion** — a few dozen named-named pairs plus "every significant entity recharges management fees to every subsidiary" as a pattern. Manifest expands patterns deterministically.
8. **Auditor-native terminology** — `scoping_profiles`, `component_materiality_allocations`, `consolidation_method` use the terms an audit partner uses, not software-engineer-invented names.

### 3.3 Backward compatibility

- `companies: [...]` flat multi-company configs continue to work exactly as today. No `group:` means today's behavior.
- `group:` is opt-in via the presence of the key. Detection is automatic at config-load time.
- All existing top-level sections (`fraud:`, `audit:`, `accounting_standards:`, `tax:`, `treasury:`, etc.) continue to work at the top level as the `defaults` block when `group:` is absent.
- Existing industry presets (`manufacturing`, `retail`, etc.) are unchanged. New preset `group_audit` overlays `group:` on a base industry preset.

## 4. `GroupManifest` contract

The manifest is the sole input handoff between phase 1 (manifest) and phases 2/3 (shard, aggregate). It is JSON-serialized, ~10–50 MB for a 2,000-entity group, and small enough to pass wholesale to every shard (no streaming/reference complexity).

### 4.1 Schema

```jsonc
{
  "schema_version": "1.0",
  "group_id": "NESTLE_2024_Q1",
  "group_seed": 16045690981371582975,
  "presentation_currency": "CHF",
  "period": {"start": "2024-01-01", "end": "2024-03-31", "length": "quarterly"},

  "ownership_graph": {
    "parent_entity_code": "NESTLE_SA",
    "entities": [
      {
        "code": "NESTLE_SA",
        "country": "CH",
        "functional_currency": "CHF",
        "scoping_profile": "significant",
        "consolidation_method": "parent",
        "accounting_framework": "ifrs",
        "industry": "manufacturing",
        "entity_seed": "a3f2b1e9...",
        "shard_id": "S_A_0001"
      },
      /* ... 2,000 entries */
    ]
  },

  "scoping_profiles": {
    "significant":        {/* resolved GeneratorConfig fragment */},
    "material":           {/* ... */},
    "consolidation_only": {/* ... */}
  },

  "chart_of_accounts_master": {
    "primary_framework": "ifrs",
    "frameworks": {"ifrs": {...}, "us_gaap": {...}, "hgb": {...}, "pcg": {...}, "swiss_or": {...}},
    "coa_id": "NESTLE_GROUP_CoA"
  },

  "fx_rate_master": {
    "base_currency": "CHF",
    "rates": {
      "CHF/USD": {"2024-01-31": 0.8870, "2024-02-29": 0.8812, "2024-03-31": 0.9012},
      "CHF/EUR": {...}, "CHF/BRL": {...}, "CHF/JPY": {...}
    },
    "policy": {"balance_sheet": "closing", "income_statement": "average", "equity": "historical"}
  },

  "shared_masters": {
    "vendors":   {"pool_seed": "b4c1...", "pool_size": 50000,  "shared_by_entities": ["all_significant"]},
    "customers": {"pool_seed": "7ae3...", "pool_size": 200000, "shared_by_entities": ["all"]},
    "materials": {"pool_seed": "1f2c...", "pool_size": 20000,  "shared_by_entities": ["all_manufacturing"]}
  },

  "ic_relationships": [
    {
      "id": "ICR_001",
      "seller": "NESPRESSO_SA",
      "buyer": "NESTLE_USA",
      "types": ["goods_sale", "royalty"],
      "annual_volume": 50000000,
      "transfer_pricing": "cost_plus",
      "markup_percent": 0.08
    },
    /* ~thousands of edges after pattern expansion */
  ],

  "audit_engagement_plan": {
    "engagement_id": "EY_NESTLE_2024_Q1",
    "lead_auditor": "EY_ZURICH",
    "fsm_blueprint": "builtin:group_fsa",
    "group_materiality": 475000000,
    "performance_materiality": 356250000,
    "clearly_trivial": 23750000,
    "component_materiality_allocations": [
      {"entity_code": "NESPRESSO_SA", "materiality": 84000000, "scope": "full"},
      {"entity_code": "NESTLE_USA",   "materiality": 92000000, "scope": "full"},
      {"entity_code": "NESTLE_DE_GMBH", "materiality": 28000000, "scope": "specific",
       "account_areas": ["revenue", "inventory", "leases"]},
      /* <5% revenue → analytical only */
    ],
    "component_auditors": [
      {"id": "CA_EY_CH", "firm": "EY", "jurisdiction": "CH",
       "entities": ["NESTLE_SA", "NESPRESSO_SA", ...]},
      {"id": "CA_EY_US", "firm": "EY", "jurisdiction": "US",
       "entities": ["NESTLE_USA", ...]}
    ]
  },

  "tax_group_plan": {
    "pillar_two": {"enabled": true, "jurisdictions": ["CH", "DE", "FR", "US", "JP"]},
    "cbc_report": {"enabled": true, "reporting_jurisdiction": "CH"},
    "transfer_pricing": {"master_file": true, "local_files_for": ["CH", "US", "DE", "FR", "BR"]}
  },

  "shard_plan": [
    {"shard_id": "S_A_0001", "entity_codes": [/*50 entries*/], "scoping_profile": "significant",
     "estimated_rows": 400000000, "estimated_archive_size_mb": 4200},
    {"shard_id": "S_B_0001", "entity_codes": [/*100 entries*/], "scoping_profile": "material",
     "estimated_rows": 75000000, "estimated_archive_size_mb": 400},
    {"shard_id": "S_C_0001", "entity_codes": [/*500 entries*/], "scoping_profile": "consolidation_only",
     "estimated_rows": 15000000, "estimated_archive_size_mb": 20}
    /* target: ≤~1 TB per shard */
  ],

  "aggregate_seed": "c8e2d193..."
}
```

### 4.2 Allocations vs instances

| Artifact | Inlined in manifest | Derived in shard/aggregate |
|---|:---:|:---:|
| Ownership graph (entity list) | ✅ | |
| Scoping profiles (resolved configs) | ✅ | |
| Chart of accounts master | ✅ | |
| FX rate master | ✅ | |
| IC *relationships* (edges) | ✅ | |
| Audit engagement plan | ✅ | |
| Tax group plan | ✅ | |
| Shard plan | ✅ | |
| Entity seeds | ✅ | |
| IC *pair instances* (pair_id, amount, date) | | ✅ from IC edge + group_seed |
| Shared-master vendor/customer/material objects | | ✅ from pool_seed (both shards regenerate identical) |
| Per-entity masters (employees, local vendors) | | ✅ from entity_seed |
| Component workpapers/evidence/findings | | ✅ from entity_seed during shard audit phase |

The single discipline: **the manifest carries allocations and relationships; phases derive instances deterministically.** This keeps the manifest tractable at 2,000-entity scale while preserving bit-exact reproducibility.

## 5. Cross-entity intercompany matching

### 5.1 Manifest-driven strategy (default)

For each IC relationship `R = {id, seller, buyer, types, annual_volume, transfer_pricing, ...}`:

- Pair count `N = annual_volume / avg_amount` where `avg_amount` is derived from the transaction-type defaults and transfer-pricing method
- Each pair `i ∈ [0, N)` has:
  - `pair_id = blake3("ic_pair" ‖ group_seed ‖ R.id ‖ i)`
  - `amount  = lognormal_sample(seed = blake3(pair_id ‖ "amount"), mu, sigma)`
  - `date    = temporal_sample(seed = blake3(pair_id ‖ "date"), period_bounds)`
  - `type    = R.types[i mod len(R.types)]`

Both shards (seller's entity and buyer's entity) run the identical derivation:

- The seller shard injects a seller-side JE carrying `ic_pair_id = pair_id`, `ic_partner_entity = R.buyer`, with revenue/AR accounts
- The buyer shard injects a buyer-side JE carrying `ic_pair_id = pair_id`, `ic_partner_entity = R.seller`, with COGS/AP accounts

Aggregate phase joins shard outputs on `ic_pair_id` → 100 % match coverage by construction. Unmatched pairs (if any — e.g., a shard crashed) are reported in `ic_eliminations/ic_matching_coverage.json`.

### 5.2 Emergent-fuzzy strategy (opt-in)

Each shard generates IC transactions per its local logic (as today — amount from transfer-pricing policy + random draw, date from temporal sampler, no coordination). Aggregate runs fuzzy matching:

```
match_key = (amount_rounded_to_bucket, date ± N days, partner_entity, transaction_type)
```

Unmatched residual is realistic (real engagements typically run 1–5 % IC imbalance; the whole point of IC reconciliation procedures in ISA 330). Not bit-deterministic but closer to production reality.

Configured via:
```yaml
intercompany:
  matching: { strategy: emergent_fuzzy, amount_tolerance: 0.005, date_tolerance_days: 3 }
```

### 5.3 Hybrid "realistic drift" (future opt-in)

Starts from manifest-driven pairs but intentionally perturbs the buyer side's amount (±0.5 %) and date (±2 days) to simulate real-world IC reconciliation friction while preserving approximate ground truth. Useful for training IC reconciliation ML models. Not in v5.0; tracked as a v5.3+ enhancement.

### 5.4 Coverage reporting

`ic_eliminations/ic_matching_coverage.json`:

```json
{
  "total_pairs_planned": 250000,
  "matched": 248750,
  "coverage": 0.995,
  "unmatched_by_reason": {
    "missing_buyer_side": 780,
    "amount_drift_above_tolerance": 120,
    "date_drift_above_tolerance": 350,
    "partner_entity_filtered_out": 0
  },
  "unmatched_sample": [/* first 100 unmatched pairs for debugging */]
}
```

"Good engagement" gate: `coverage ≥ 0.98`.

## 6. Shard phase

### 6.1 Input

- `GroupManifest` (complete, read-only)
- Shard spec: `{shard_id, entity_codes: [...]}`
- Output directory

### 6.2 Execution

For each entity in the shard's entity list:

1. Look up the entity record in `manifest.ownership_graph.entities`
2. Resolve `GeneratorConfig` by merging: `defaults` → `scoping_profiles[entity.scoping_profile]` → per-entity overrides
3. Inject:
   - Entity seed from `manifest.ownership_graph.entities[code].entity_seed`
   - Chart of accounts from `manifest.chart_of_accounts_master.frameworks[entity.accounting_framework]`
   - FX rates from `manifest.fx_rate_master`
   - Shared master pool references (vendor/customer/material pools the entity draws from)
   - IC relationship edges for this entity (as seller or buyer) — used by IC injection in the JE generator
4. Call `EnhancedOrchestrator::generate(config)` → produces today's per-entity artifact set
5. Write output to `entities/{entity_code}/` — unchanged structure from today's single-entity orchestrator
6. Emit `shard_summary.json` with per-entity row counts and IC pair stats

### 6.3 IC injection protocol

The JE generator receives a list of `ICPairPlan` entries for the entity. Each plan carries `pair_id, partner_entity_code, type, amount, date, role (seller|buyer), accounts`. The generator injects the JE as part of the entity's regular transaction stream, flagging it with `ic_pair_id` and `ic_partner_entity` on the JE header.

### 6.4 Audit injection protocol

If the entity's scoping profile enables audit, the audit generators run within the shard — producing per-entity workpapers, evidence, findings, risk assessments, and optionally a per-entity FSA engagement via the existing `builtin:fsa` blueprint. These feed into the aggregate audit pack (Section 8).

## 7. Aggregate phase

Three sub-phases, each self-contained, each emitting to its own directory. Sub-phase 7a runs first; 7b and 7c depend on 7a's consolidated TB and can run in parallel.

### 7a. Consolidation engine

Reads manifest + per-shard `entities/{code}/subledger/trial_balance.json`.

```
per-entity TBs ──▶ pre-elim TB ──IC match──▶ post-elim TB ──IAS 21──▶ translated TB ──▶ consolidated FS
                                                              + CTA                     + NCI rollforward
                                                                                        + consolidation schedule
                                                                                        + GAAP bridges
```

Steps:

1. **Aggregate pre-elimination TB** — sum by account across entities with `consolidation_method ∈ {parent, full}`. Equity-method/fair-value entities appear only as investment lines, not rolled up.
2. **Match IC pairs** on `ic_pair_id` → `EliminationEntry` records → consolidation JEs. Emitted pair categories:
   - AR ↔ AP (trade IC)
   - Revenue ↔ COGS (goods sale)
   - Loan ↔ Borrowing + accrued interest on both sides
   - Dividend paid ↔ Dividend received
   - Management recharge ↔ Expense
   - Royalty ↔ Royalty expense
   - Cost sharing (symmetric)
   Unmatched recorded in `ic_matching_coverage.json` (Section 5.4).
3. **Translate entity-by-entity** (IAS 21):
   - BS monetary items at closing rate
   - BS non-monetary items at historical rate (tracked at source during shard; if unavailable, approximated via weighted-average over origination)
   - P&L items at average rate
   - Equity items at historical rate
   - Residual → CTA (OCI). Per-entity rollforward in `cta_rollforward.json`.
   - Full per-entity translation worksheet (line-by-line local-to-presentation with rate applied and translated amount) emitted to `translation_worksheet.json` for audit traceability.
4. **Consolidate** translated post-elim TBs: sum by account across fully-consolidated entities.
5. **Equity-method adjustments** — per entity with `consolidation_method = equity_method`:
   - Single-line "Investment in associate" at carrying value (opening + share of profit − dividends received − impairment)
   - "Share of profit of associates" in P&L (ownership % × associate's net income)
6. **NCI rollforward** — per non-wholly-owned fully-consolidated subsidiary:
   - Opening NCI (from prior period's `nci_rollforward.json` if supplied, else 0)
   - + (1 − ownership%) × subsidiary net income
   - + (1 − ownership%) × subsidiary OCI
   - − dividends to NCI (if dividend declared)
   - = Closing NCI
   - Emitted to `nci_rollforward.json`.
7. **Generate consolidated FS** — BS, IS, CF (indirect method), Changes in Equity. Written to `consolidated/consolidated_financial_statements.json`.
8. **Consolidation schedule** — per line item: entity-by-entity pre-elimination breakdown → elimination adjustments → post-elimination total. `consolidation_schedule.json`.
9. **GAAP bridges** — for entities with `local_framework ≠ group_framework`, synthesize a bridge schedule covering:
   - Revenue recognition (ASC 606 vs IFRS 15 timing)
   - Leases (ASC 842 vs IFRS 16 P&L classification)
   - Inventory (LIFO vs weighted-average / FIFO)
   - Goodwill (ASC 350 vs IAS 36 impairment model)
   - Pensions (ASC 715 corridor vs IAS 19 OCI)
   - Stock comp (ASC 718 vs IFRS 2 vesting expense pattern)
   Written to `consolidated/gaap_bridges/{entity_code}.json`.
10. **Notes to consolidated FS** — template-driven assembly from consolidated data. Baseline set: significant accounting policies, revenue, leases, income taxes, provisions, related parties, subsequent events, PP&E, intangibles + goodwill, employee benefits, segment information (when §7a.11 is enabled), financial instruments, commitments. Written to `consolidated/notes_to_consolidated_fs.json`.
11. **Segment reporting (v5.1)** — IFRS 8 / ASC 280 operating segments derived from the entity/geography/product-line axes of the ownership graph and consolidated P&L. `consolidated/segment_reporting.json`. Deferred to v5.1 because segments feed into group KAMs and the segment-level-materiality component of the audit pack.

Outputs: `consolidated/*`, `ic_eliminations/*`.

### 7b. Group audit pack (ISA 600)

Reads manifest's `audit_engagement_plan` + per-shard `entities/{code}/audit/*`.

Steps:

1. Emit `GroupAuditPlan` and `ComponentAuditor` records directly from manifest.
2. Emit `ComponentInstruction` per entity — scope derived from `component_materiality_allocations`:
   - ≥15 % group revenue → `full`
   - 5-15 % → `specific` (named account areas)
   - <5 % → `analytical` only
3. Load component outputs from shard archives (workpapers, evidence, findings, misstatements).
4. Synthesize `ComponentAuditorReport` per entity: opinion on component, misstatements classified (factual / judgmental / projected), scope limitations, significant findings.
5. Aggregate uncorrected misstatements to group level; compare to group materiality and performance materiality. `aggregated_misstatements.json`.
6. Derive group KAMs from aggregated risk assessments + accounting estimates + subsequent events, scored by group-level significance. `group_key_audit_matters.json`.
7. Emit group opinion (`unmodified | qualified | adverse | disclaimer`) based on aggregated misstatements × materiality + scope limitations + KAM references. `group_opinion.json`.
8. Related-party disclosures (IAS 24) — derived from ownership graph + KMP (from per-entity h2r data) + IC transactions. `related_party_disclosures.json`.
9. (v5.1) Run `builtin:group_fsa` FSM blueprint modeling ISA 600 lifecycle: acceptance → planning → component scoping → component oversight → aggregation → group reporting. Emits `group_engagement_fsm/event_trail.json`.

Outputs: `audit/*`.

### 7c. Tax pack

Reads manifest's `tax_group_plan` + per-entity TBs + IC transactions.

Steps:

1. **Pillar 2 aggregator**:
   - Per-jurisdiction rollup of GloBE income, covered taxes, substance-based carve-out (payroll cost + tangible asset NBV)
   - Compute jurisdictional ETR; if <15 %, compute top-up tax per entity in that jurisdiction
   - QDMTT computation where applicable (CH, EU Member States, UK, KR)
   - Emits `consolidated/pillar_two_analysis.json`
2. **CbCR**:
   - Per jurisdiction: related-party revenue, unrelated-party revenue, profit before tax, income taxes paid (cash basis), income taxes accrued, stated capital, accumulated earnings, employees, tangible assets other than cash
   - Aggregates all entities in each jurisdiction
   - `tax/cbc_report.json`
3. **TP master file**:
   - Group structure, business description, intangibles policy, intragroup financial activities (loans, cash pooling), group TP policies
   - Derived from `intercompany.relationships` + `transfer_pricing` config + ownership graph
   - `tax/tp/master_file.json`
4. **TP local files** (per configured jurisdiction):
   - Controlled transactions for entities in that jurisdiction
   - Method applied (CUP / RPM / CPM / TNMM / PSM)
   - Benchmarking analysis / comparables
   - Economic analysis
   - `tax/tp/local_files/{jurisdiction}.json`

Outputs: `tax/*`.

### 7d. Ordering & fault tolerance

- 7a → {7b ∥ 7c} — 7b and 7c are independent given 7a and can parallelize
- If 7a fails, 7b/7c do not run; partial output reported
- Each sub-phase is idempotent: re-running aggregate on the same manifest + shards produces byte-identical output

## 8. Fleet driver

New crate `datasynth-fleet` (Phase 4, v5.3).

### 8.1 Dispatcher trait

```rust
#[async_trait]
pub trait Dispatcher: Send + Sync {
    async fn dispatch(
        &self,
        manifest: &GroupManifest,
        shard: &ShardPlan,
    ) -> Result<ShardArchive>;

    async fn dispatch_many(
        &self,
        manifest: &GroupManifest,
        shards: &[ShardPlan],
    ) -> Result<Vec<ShardArchive>> {
        // default impl: concurrent via tokio::spawn, bounded by max_concurrent_shards
    }
}
```

### 8.2 Shipped implementations

- `InProcessDispatcher` — rayon pool, each worker calls `datasynth_group::run_shard()` directly. Fastest for small groups. Single-process memory envelope.
- `SubprocessDispatcher` — spawns `datasynth-data group shard --manifest ... --shard ...` subprocesses. Memory-isolated per shard. Fault-isolated (one crash doesn't lose everything).
- `RemoteDispatcher` — submits shard jobs to `datasynth-server` via HTTP/gRPC. Scales across machines. Shard jobs become first-class portal jobs.

### 8.3 Fleet config

```yaml
fleet:
  dispatcher: in_process                # in_process | subprocess | remote
  max_concurrent_shards: 8
  per_shard_timeout_seconds: 7200
  retry_policy: { max_retries: 2, backoff: exponential, initial_delay_seconds: 30 }
  progress: { interval_seconds: 30, emit_stdout: true, emit_json: true }
  remote:                               # only if dispatcher = remote
    server_url: "https://portal.vynfi.com"
    api_key_env: "VYNFI_API_KEY"
```

### 8.4 Failure semantics

- Any shard failure → aggregate does not run
- Partial archive emitted with manifest + successful shards, plus `fleet_state.json` listing failed shards and their error classifications
- `datasynth-data fleet resume --state fleet_state.json` re-dispatches only the failed shards (useful for long-running engagements)
- Aggregate exits non-zero when fewer than all shards are present; can be forced with `--tolerate-missing-shards` (reports become "limited scope")

## 9. Output archive layout

```
group_archive/
├── group_manifest.json                    # Phase 1 output
├── group/
│   ├── ownership_graph.json               # resolved, human-readable view
│   ├── scoping_profiles.json
│   ├── chart_of_accounts_master.json
│   ├── fx_rate_master.json
│   └── ic_relationships.json              # relationship graph (not pair instances)
├── entities/
│   ├── NESTLE_SA/
│   │   ├── journal_entries.json
│   │   ├── journal_entries.csv
│   │   ├── acdoca.csv
│   │   ├── master_data/
│   │   ├── document_flows/
│   │   ├── subledger/
│   │   ├── financial_reporting/
│   │   ├── audit/
│   │   ├── sourcing/
│   │   ├── hr/
│   │   ├── tax/
│   │   ├── treasury/
│   │   ├── manufacturing/
│   │   ├── sap_export/                    # if exportFormat: sap per entity
│   │   └── ...                            # full per-entity output identical to today
│   ├── NESPRESSO_SA/
│   └── ...
├── consolidated/
│   ├── consolidated_financial_statements.json
│   ├── consolidation_schedule.json
│   ├── nci_rollforward.json
│   ├── cta_rollforward.json
│   ├── translation_worksheet.json
│   ├── gaap_bridges/
│   │   └── {entity_code}.json
│   ├── segment_reporting.json
│   ├── notes_to_consolidated_fs.json
│   └── pillar_two_analysis.json
├── ic_eliminations/
│   ├── ar_ap_pairs.json
│   ├── revenue_cos_pairs.json
│   ├── loan_instruments.json
│   ├── dividend_flows.json
│   ├── management_recharges.json
│   ├── elimination_journal_entries.json
│   └── ic_matching_coverage.json
├── audit/
│   ├── group_audit_plan.json
│   ├── component_auditors.json
│   ├── component_instructions.json
│   ├── component_reports.json
│   ├── aggregated_misstatements.json
│   ├── group_key_audit_matters.json
│   ├── group_opinion.json
│   ├── related_party_disclosures.json
│   └── group_engagement_fsm/
│       └── event_trail.json
├── tax/
│   ├── cbc_report.json
│   ├── pillar_two_calculation.json
│   └── tp/
│       ├── master_file.json
│       └── local_files/
│           └── {jurisdiction}.json
└── evaluation/
    └── group_evaluation.json              # optional
```

The per-entity subtree under `entities/{code}/` is byte-identical to today's single-entity orchestrator output. Any existing consumer working with single-entity archives can be pointed at `entities/{code}/` as its root directory and continue to work unchanged.

## 10. Determinism guarantees

- **Byte-identical across execution modes** — in-process standalone, subprocess-sharded, fleet-distributed all produce identical `group_archive/` given the same `group_seed`.
- **Order-independent** — reordering `ownership.entities` in the YAML does not change any entity's output.
- **Additive** — adding an entity doesn't perturb existing entities; removing one only drops its IC pairs (reported as unmatched on the partner side).
- **Commutative sharding** — changing the shard assignment (which entity goes in which shard) does not change any per-entity output.

### 10.1 Determinism caveats

- LLM enrichment (vendor/customer name enrichment, finding narratives) is not bit-deterministic. Flagged in archive metadata (`manifest.llm_enrichment = true` → output is stochastic). Users who need bit-determinism disable LLM enrichment.
- FX historical-rate translation requires manifest-pinned rates. If `fx.rate_source: historical_series` (external market data feed), rates can drift between runs — users requiring bit-determinism use `inline` or `user_supplied`.
- Audit FSM runs can be stochastic if the FSM overlay specifies probabilistic branching. The same `aggregate_seed` produces identical FSM event trails; different seeds produce different-but-coherent trails.

## 11. Migration & backward compatibility

### 11.1 Zero breaking changes

- Existing `companies: [...]` flat multi-company configs continue to work exactly as today.
- No `group:` key → today's single-orchestrator-pass behavior, unchanged.
- All existing config sections (`fraud:`, `audit:`, `accounting_standards:`, `tax:`, `treasury:`, `intercompany:`, `fx:`, `period_close:`, etc.) continue to work at the top level.
- Existing presets (`manufacturing`, `retail`, `financial_services`, `healthcare`, `technology`) unchanged.
- Existing CLI commands (`datasynth-data generate --config X`, `datasynth-data init`, `datasynth-data validate`, `datasynth-data fingerprint`) unchanged.
- Existing output archive shape unchanged when `group:` is absent.

### 11.2 Opt-in surface

- Presence of `group:` key switches behavior to group mode.
- When `group:` is present, legacy top-level `companies:` is auto-populated from `group.ownership.entities` for backward-compat readers; ignored if both specified inconsistently (warning emitted).
- New CLI subcommands: `datasynth-data group {manifest,shard,aggregate,generate,fleet}`.
- New preset `group_audit` overlays `group:` on a base industry preset (e.g., `--preset group_audit --base manufacturing`).
- New output archive layout `per_entity_subtree` is the default when `group:` is present; explicit `output.layout: flat` reverts to single-stream output (all entities in root files, discriminated by `company_code` field).

### 11.3 Upgrade path for existing users

No migration required. Existing configs run exactly as before.

Users wanting group features:

1. Wrap their existing `companies:` list under `group.ownership.entities:`
2. Add `group.ownership.parent_entity_code`
3. Add `group.presentation_currency`
4. Optionally add `group.scoping_profiles` and `group.intercompany.relationships`

Incremental adoption is supported — a minimal `group:` with just `ownership.entities` enables the new orchestrator path with defaults everywhere.

## 12. Testing strategy

| Kind | Coverage | Location |
|---|---|---|
| Golden data | 5-entity "Mini-Nestlé" reference group: IFRS, CHF/USD/EUR, 80 %-owned BR subsidiary, IFRS 11 JV (50/50), full-scope audit. Byte-identical check per release. | `crates/datasynth-group/tests/golden/` + fixture archive |
| Property | IC matching coverage ≥98 % across randomized pair plans; post-elim consolidated balance `A = L + E + NCI` within ε; `sum(entity P&L) = consolidated P&L + eliminations ± ε`; NCI rollforward `opening + movements = closing` | `crates/datasynth-group/tests/properties.rs` |
| Determinism | {in-process, subprocess, remote} dispatchers → bit-identical group archive for same seed | `crates/datasynth-fleet/tests/determinism.rs` |
| Backward-compat | Existing `companies: [list]` configs produce byte-identical output to today | `crates/datasynth-group/tests/backcompat.rs` |
| Scale | 100-entity group in <5 min; 500-entity in <30 min; 2,000-entity nightly only (ignored in CI) | `crates/datasynth-group/tests/scale.rs` (`--ignored` by default) |
| End-to-end | Submit Mini-Nestlé to local `datasynth-server`, download archive, verify every expected file exists and parses | `crates/datasynth-server/tests/group_e2e.rs` |
| Config validation | Every illustrative config in this spec validates; Mini-Nestlé config validates; invalid configs produce actionable errors | `crates/datasynth-config/tests/group_schema.rs` |

Reference artifacts:
- `configs/examples/group/mini_nestle.yaml` — 5-entity reference config
- `configs/examples/group/mid_market.yaml` — 20-entity mid-market group
- `configs/examples/group/nestle_class.yaml` — 100-entity stress test (CI), 2000-entity nightly

## 13. Phased rollout

Four releases, each shippable independently. Dependencies flow strictly v5.0 → v5.1, v5.0 → v5.2, v5.0 → v5.3; v5.1 and v5.2 can overlap.

### v5.0 — Group engine foundation

- `datasynth-group` crate with manifest / shard / aggregate phases
- `group:` config schema; `scoping_profiles`; `ownership.entities` (explicit + generated); `intercompany.relationships`; `fx`; `audit` engagement plan (plan only, no component auditor generator yet); `tax` group plan (plan only, no generators yet)
- Manifest-driven IC matching + cross-entity elimination
- Elimination → GL JE flowing into consolidated TB
- IAS 21 per-entity functional-to-presentation translation + CTA rollup
- NCI rollforward (opening from prior period if supplied, else 0)
- Consolidated FS (BS, IS, CF, Equity) + consolidation schedule
- Per-entity output subtree
- Determinism across in-process execution modes
- New CLI: `datasynth-data group {manifest,shard,aggregate,generate}`
- Mini-Nestlé reference config + golden fixture

**Unlocks:** real group simulation ≤50 entities with full consolidation machinery.

### v5.1 — Group audit pack (ISA 600)

- `ComponentAuditor` generator populating from manifest jurisdictional rollup
- `ComponentInstruction` generator with scope allocation (full/specific/analytical)
- `ComponentAuditorReport` generator synthesizing from shard outputs
- Group-level misstatement aggregation
- Group KAMs from aggregated risk
- Group audit opinion derivation
- Related-party disclosures (IAS 24) from ownership graph + KMP + IC transactions
- Segment reporting (IFRS 8 / ASC 280) — enables segment-level KAMs + segment materiality
- `builtin:group_fsa` FSM blueprint (ISA 600 lifecycle)
- Group engagement FSM wiring into aggregate phase

**Unlocks:** "a group audit" as audit partners recognize it — engagement artifacts on top of consolidated numbers.

### v5.2 — Multi-GAAP + tax pack

- Per-entity `accounting_framework` variety: HGB (German), PCG (French), Swiss OR, in addition to existing IFRS / US GAAP / French GAAP
- GAAP bridge generator synthesizing local-to-group framework deltas per entity
- Pillar 2 jurisdictional aggregator with ETR + substance-based carve-out + top-up tax + QDMTT
- CbCR generator per-jurisdiction rollup
- TP master file generator (group-level)
- TP local file generator (per configured jurisdiction)

**Unlocks:** OECD / BEPS audits; real multi-GAAP groups; EU listings requiring Pillar 2 + CSRD.

### v5.3 — Fleet driver

- `datasynth-fleet` crate
- `InProcessDispatcher` (rayon)
- `SubprocessDispatcher` (spawn `datasynth-data group shard`)
- `RemoteDispatcher` (HTTP/gRPC to `datasynth-server`)
- Fleet config schema + progress reporting + retry policy
- `fleet_state.json` + `datasynth-data fleet resume`
- Emergent-fuzzy IC matching as opt-in strategy
- Distributed aggregate (optional — can also run locally on collected shards)

**Unlocks:** Nestlé-scale 2,000-entity engagements; distributed generation; multi-machine throughput.

### Aggregate estimate

12–20 weeks aggregate engineering, assuming ~1–2 FTE per phase with v5.1 and v5.2 overlapping where dependencies allow.

## 14. Risks & open questions

### 14.1 Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Manifest schema evolution breaks shard/aggregate compatibility across versions | Sharded fleet mid-engagement becomes uncallable after a DS upgrade | `schema_version` pinned in manifest; aggregate refuses to run against manifest from a different major version; backwards-compatible additions only within a major version |
| IC pair derivation amount/date sampling produces unrealistic distributions at scale | Aggregate consolidation looks wrong to auditors | Calibrate against real IC distributions from Benford analysis + industry benchmarks; add property tests on IC amount distribution shape per transaction type |
| NCI rollforward without a prior-period input assumes opening NCI = 0 | First-period output looks like every subsidiary was just acquired | Document the limitation; require explicit `nci_opening_balances:` config override for period 1 when entities weren't just acquired; full multi-period continuity deferred to a potential v5.4 engagement abstraction |
| GAAP bridge generator produces unrealistic deltas | Multi-GAAP groups look wrong | Start with a small set of well-understood differences per framework pair; expand iteratively; flag bridge coverage in output (e.g., `coverage: "standard_differences_only"`) |
| Shard determinism is fragile to orchestrator changes | A refactor to the single-entity orchestrator silently breaks group determinism | CI property test: single-entity orchestrator output is byte-stable across refactors (already partly in place); extend to cover the shard input contract |
| Pillar 2 is a fast-moving regulatory target (QDMTT, IIR, UTPR) | Output goes out of date | Implement the GloBE Model Rules version explicitly; version the output artifact; accept that specific jurisdictional detail will evolve |
| 2,000-entity manifest (~50 MB) is slow to load/parse in every shard | Shard startup time dominates for tiny Tier-C shards | Consider manifest partitioning (per-shard manifest projection) as a v5.3 optimization; measure first |
| Existing 27 audit generators behave subtly differently inside a shard vs standalone | Per-entity audit output drifts in group mode | Integration test: same entity config run standalone vs as a single-entity shard produces byte-identical output |

### 14.2 Open questions

1. **Shared-master pool sizing** — when two entities both draw from a 50 k vendor pool, what's the overlap target? (Too much overlap = unrealistic; too little = not really shared.) Suggest configurable via `shared_masters.vendors.per_entity_draw: 1000` with deterministic sampling.
2. **Reference Mini-Nestlé config** — exact entity composition for the golden fixture. Propose: NESTLE_SA (parent, CH, IFRS), NESTLE_USA (US, US GAAP, full), NESTLE_DE (DE, IFRS + HGB bridge, full, 80 %-owned), NESTLE_BR (BR, IFRS, full, 100 %-owned), NESTLE_JV (CH, IFRS, equity method, 50 %).
3. **Group FSA blueprint scope** — should `builtin:group_fsa` model the full ISA 600 lifecycle as a single blueprint, or be composable from the existing `builtin:fsa` + a thin "group overlay"? Leaning toward separate blueprint for clarity.
4. **Nightly 2,000-entity scale test cost** — needs a dedicated CI machine budget. Gate behind a manual workflow dispatch rather than daily schedule until we have data on wall-clock + compute cost.
5. **`entities_from: .csv` column contract** — exact column headers (`code`, `name`, `country`, `functional_currency`, `scoping_profile`, `consolidation_method`, `ownership_percent`, `parent_code`, `industry`, `accounting_framework`). Document and version this contract separately.

## 15. Appendix A — Reference Mini-Nestlé config

Smallest viable group config exercising every v5.0 feature.

```yaml
group:
  id: "MINI_NESTLE_2024_Q1"
  name: "Mini Nestlé Reference Group"
  presentation_currency: "CHF"
  period: { start_date: "2024-01-01", length: quarterly }
  seed: 0x1234567890ABCDEF

  defaults:
    accounting_framework: ifrs
    industry: manufacturing
    process_models: [o2c, p2p, h2r, r2r, audit]
    accounting_standards: { enabled: true, leases_enabled: true }

  scoping_profiles:
    significant:
      row_budget: 100_000
      process_models: [o2c, p2p, h2r, r2r, audit, manufacturing]
      audit: { generate_workpapers: true, min_team_size: 4, max_team_size: 8 }
    material:
      row_budget: 25_000
      audit: { generate_workpapers: true, min_team_size: 2, max_team_size: 4 }

  ownership:
    parent_entity_code: NESTLE_SA
    entities:
      - { code: NESTLE_SA, country: CH, functional_currency: CHF,
          scoping_profile: significant, consolidation_method: parent }
      - { code: NESTLE_USA, country: US, functional_currency: USD,
          scoping_profile: significant, consolidation_method: full,
          ownership_percent: 1.0, parent_code: NESTLE_SA,
          accounting_framework: us_gaap }
      - { code: NESTLE_DE, country: DE, functional_currency: EUR,
          scoping_profile: significant, consolidation_method: full,
          ownership_percent: 0.80, parent_code: NESTLE_SA,
          accounting_framework: hgb }      # bridges HGB → IFRS at aggregate
      - { code: NESTLE_BR, country: BR, functional_currency: BRL,
          scoping_profile: material, consolidation_method: full,
          ownership_percent: 1.0, parent_code: NESTLE_SA }
      - { code: NESTLE_JV, country: CH, functional_currency: CHF,
          scoping_profile: material, consolidation_method: equity_method,
          ownership_percent: 0.50, parent_code: NESTLE_SA }

  intercompany:
    relationships:
      - { seller: NESTLE_SA, buyer: NESTLE_USA, types: [goods_sale, royalty],
          annual_volume: 5_000_000, transfer_pricing: cost_plus, markup_percent: 0.08 }
      - { seller: NESTLE_SA, buyer: NESTLE_DE, types: [goods_sale, management_fee],
          annual_volume: 3_000_000, transfer_pricing: cost_plus, markup_percent: 0.06 }
      - { pattern: { seller: NESTLE_SA, buyer_scoping_profile: any },
          types: [management_fee], per_pair_volume: 200_000 }
    matching: { strategy: manifest_driven, coverage_target: 0.98 }

  fx:
    base_currency: CHF
    rate_source: inline
    rates:
      "CHF/USD": { "2024-01-31": 0.8870, "2024-02-29": 0.8812, "2024-03-31": 0.9012 }
      "CHF/EUR": { "2024-01-31": 0.9520, "2024-02-29": 0.9488, "2024-03-31": 0.9611 }
      "CHF/BRL": { "2024-01-31": 5.5210, "2024-02-29": 5.4880, "2024-03-31": 5.6700 }
    policy: { balance_sheet: closing, income_statement: average, equity: historical }

  audit:
    engagement_id: "EY_MINI_NESTLE_2024_Q1"
    lead_auditor: EY_ZURICH
    framework: isa
    fsm_blueprint: "builtin:group_fsa"
    group_materiality: { basis: revenue, percent: 0.01 }
    component_scope_thresholds: { full_scope: 0.15, specific_scope: 0.05 }
    generate_kams: true
    generate_group_opinion: true

  tax:
    pillar_two:       { enabled: true, jurisdictions: [CH, DE, US] }
    cbc_report:       { enabled: true, reporting_jurisdiction: CH }
    transfer_pricing: { master_file: true, local_files_for: [CH, US, DE] }

  output:
    layout: per_entity_subtree
    shared_masters_at_root: true
```

Runs in-process standalone in well under a minute, exercises: parent entity, full-consolidated subsidiary, 80 %-owned subsidiary (NCI rollforward), multi-GAAP bridge (HGB → IFRS), equity-method JV, multi-currency translation + CTA, manifest-driven IC with pattern expansion, component audit allocation, Pillar 2 across three jurisdictions, TP master file + three local files, group opinion + KAMs.

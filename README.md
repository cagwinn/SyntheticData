# DataSynth v5.0.0

[![License](https://img.shields.io/badge/license-Apache%202.0-green.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)
[![CI](https://github.com/mivertowski/SyntheticData/actions/workflows/ci.yml/badge.svg)](https://github.com/mivertowski/SyntheticData/actions/workflows/ci.yml)
[![DOI](https://img.shields.io/badge/DOI-10.13140%2FRG.2.2.13943.79523-blue.svg)](https://doi.org/10.13140/RG.2.2.13943.79523)

**Synthetic enterprise data generation for ML training, audit analytics, and system testing.**

DataSynth generates statistically realistic, fully interconnected enterprise financial data across 20+ process families. Generated data respects accounting identities (debits = credits, Assets = Liabilities + Equity), follows empirical distributions (Benford's Law, log-normal mixtures, Pareto heavy tails, Gaussian copula correlations), and maintains referential integrity across 100+ output tables. Generation-time assertions enforce these invariants at scale.

**v5.0 ships group audit simulation** — multi-entity consolidation with manifest / shard / aggregate three-phase model, IFRS / IAS 21 / IAS 28 / IFRS 10 compliant by construction. **A 2 000-entity reference archive runs in 5 min 32 s** on a 40 vCPU host (60 GiB peak RSS) and is published as [VynFi/vynfi-group-audit-enterprise-2000](https://huggingface.co/datasets/VynFi/vynfi-group-audit-enterprise-2000).

**[Full Documentation](docs/book/src/SUMMARY.md)** | **[Commercial SDKs](https://vynfi.com)** | **[CHANGELOG](CHANGELOG.md)** | **[Roadmap](#roadmap)**

**What's new in v5.0.0 (April 2026)**
- **Group audit simulation engine** — new `datasynth-group` crate with three-phase model (manifest / shard / aggregate). Backwards-compatible: existing single-entity configs auto-dispatch to the v4.x flow byte-for-byte unchanged.
- **4 new CLI subcommands** — `datasynth-data group {manifest, shard, aggregate, generate}` with rayon-parallel shard execution.
- **9 new consolidated output artefacts** — consolidated FS (BS/IS/CF/Equity), consolidation schedule, notes (8-note disclosure set), NCI rollforward, CTA rollforward, IAS 21 translation worksheet, equity-method investments, IC matching coverage report, and group manifest.
- **Determinism guarantees** — manifest-driven IC matching produces 100 % pair coverage by construction (proven by property test); standalone in-process generation with `parallel_shards: false` produces byte-identical archives.
- **Released alongside [VynFi/vynfi-group-audit-enterprise-2000](https://huggingface.co/datasets/VynFi/vynfi-group-audit-enterprise-2000)** — 2 000-entity multinational consolidation showcase.

**What's new in v4.1 → v4.4.3 (April 2026)**
- **Python SDK retired** (v4.4.3) — the in-tree `datasynth-py` wrapper is gone; use the official commercial SDKs from [VynFi](https://vynfi.com), or drive the CLI via `subprocess` for ad-hoc Python work.
- **SAP Integration Pack** — 27-table export (BKPF/BSEG/ACDOCA transactional; LFA1/LFB1/KNA1/KNB1/MARA/MARD/ANLA/CSKS/SKA1/SKB1 master data; EKKO/EKPO/VBAK/VBAP/LIKP/LIPS/MKPF/MSEG plus BSIS/BSAS/BSID/BSAD/BSIK/BSAK subledger). Classic R/3 and S/4 HANA dialects (delimiter, decimal separator, UTF-8 BOM, date format). Priority-sorted so BKPF always precedes BSEG — foreign-key integrity guaranteed across multi-table configs.
- **SAF-T XML export** — Standard Audit File for Tax for Portugal, Poland, Romania, Norway, and Luxembourg.
- **Neural diffusion — GPU-enabled end-to-end** — candle-core score network wires opportunistically to CUDA via the `neural-cuda` feature; `preferred_device()` / `cuda_available()` helpers gracefully fall back to CPU. End-to-end smoke test validates training + sampling on a 1D log-normal dataset.
- **SDK-friendly camelCase aliases** — 30+ `#[serde(alias = "...")]` attributes across `GeneratorConfig`, `OutputConfig`, `CompanyConfig`, `GlobalConfig`; custom deserializer handles `exportFormat: "json"` single-string form. Feature-matrix configs that previously collapsed from 99 files to 19 now produce the full archive.
- **AML / fraud wiring fixes** — `document_fraud_rate` defaults to `Some(0.01)` (restoring `is_fraud_propagated` coverage); AML typology coverage now uses canonical/alias matching (7-of-7 on the retail demo); ShellLink triggers expanded from 2 to 5 conditions with Trust-UBO shell-indicator injection.
- **Null-field compat aliases** — `risk_level` mirrored alongside `risk_tier` on banking customers; `from_type/id` + `to_type/id` mirrored on `DocumentReference`; OCEL 2.0 `object_type` mirrored on every object ref.
- **YAML-as-Source-of-Truth** — embedded default pools mirrored into version-controlled YAML with a `build.rs` validator enforcing byte-identity between YAML and the compiled constants.
- **Rank-preserving inverse-CDF copula sampling** (v4.1.6) for exact marginal preservation under Gaussian/Clayton/Gumbel/Frank/Student-t dependence.
- **Security**: `rustls-webpki` bumped 0.103.12 → 0.103.13 (RUSTSEC-2026-0104).

---

## Example Datasets

Pre-generated datasets at [huggingface.co/VynFi](https://huggingface.co/VynFi):

| Dataset | Scale | Description |
|---------|-------|-------------|
| [vynfi-group-audit-enterprise-2000](https://huggingface.co/datasets/VynFi/vynfi-group-audit-enterprise-2000) | **2 000 entities** | **NEW** v5.0 group-audit reference archive: multinational ACME holding with 4 functional currencies, 4 759 IC pairs (91.6 % matched), full IFRS-compliant consolidated FS + schedule + notes + CTA + NCI + equity-method rollforwards. |
| [vynfi-journal-entries-1m](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m) | 2.1M JE lines | Manufacturing-sector denormalised JE table with 6.92 % fraud rate, ISA 240 manual flag, GL chart of accounts |
| [vynfi-aml-100k](https://huggingface.co/datasets/VynFi/vynfi-aml-100k) | 749K | Banking transactions with AML labels, 14 velocity features, 59 columns |
| [vynfi-audit-p2p](https://huggingface.co/datasets/VynFi/vynfi-audit-p2p) | 234 | P2P document chain (PO/GR/VI/Payment) with fraud labels |
| [vynfi-ocel-manufacturing](https://huggingface.co/datasets/VynFi/vynfi-ocel-manufacturing) | 344 | OCEL event log for process mining (pm4py, Celonis) |
| [vynfi-supply-chain-ocel](https://huggingface.co/datasets/VynFi/vynfi-supply-chain-ocel) | — | Supply-chain OCEL event log for cross-process mining |
| [vynfi-sar-narratives](https://huggingface.co/datasets/VynFi/vynfi-sar-narratives) | — | Suspicious activity report (SAR) narratives for AML training |

```python
from datasets import load_dataset
ds = load_dataset("VynFi/vynfi-aml-100k", split="train")
df = ds.to_pandas()
```

All datasets: Apache 2.0, entirely synthetic, no PII.

---

## Quick Start

```bash
# Build
git clone https://github.com/mivertowski/SyntheticData.git && cd SyntheticData
cargo build --release

# Demo — generates a complete dataset with defaults
./target/release/datasynth-data generate --demo --output ./output

# Full audit simulation (113+ output files)
./target/release/datasynth-data generate --demo --preset audit-group --output ./audit

# Configure and generate
./target/release/datasynth-data init --industry manufacturing --complexity medium -o config.yaml
./target/release/datasynth-data generate --config config.yaml --output ./output

# AI-powered config generation (set OPENAI_API_KEY, ANTHROPIC_API_KEY, or OPENROUTER_API_KEY)
cargo build --release --features llm
OPENAI_API_KEY=sk-... ./target/release/datasynth-data init \
  --from-description "12 months of mid-market retail data with fraud and SOX controls" -o config.yaml

# LLM-powered template enrichment — expand name pools via OpenRouter / OpenAI / Anthropic
# (v3.5.0+ offline CLI; the LLM only runs here, not at `generate` time)
OPENROUTER_API_KEY=sk-or-... cargo run --release --features llm -- \
  templates enrich \
  --input ./templates/in.yaml --output ./templates/enriched.yaml \
  --category customer_name --industry retail --region DE \
  --sub-category enterprise --count 50 \
  --backend http --model anthropic/claude-sonnet-4.5

# Counterfactual scenario simulation
./target/release/datasynth-data scenario list --config config.yaml
./target/release/datasynth-data scenario generate --config config.yaml --output ./output

# Auto-tuning: generate → evaluate → AI patch → regenerate
./target/release/datasynth-data generate --config config.yaml --output ./output --auto-tune --max-iterations 3
```

See the [CLI Reference](docs/book/src/cli-reference.md) for all commands and flags.

---

## Group audit simulation (v5.0+)

The v5.0 release adds a dedicated **multi-entity group audit engine** as a sibling
to the single-entity `generate` flow. The engine is layered above (not into)
the existing `EnhancedOrchestrator`: the per-entity flow stays byte-identical
to v4.x, and consolidation logic lives entirely in the new `datasynth-group`
crate.

The pipeline is a **three-phase model** — `manifest` resolves a `GroupConfig`
into a deterministic, content-addressable `GroupManifest` (entities, periods,
ownership graph, IC pair plan, shard plan, FX/CoA masters); `shard` drives
the orchestrator for one shard of entities and writes a full single-entity
archive per entity under `entities/{code}/`; `aggregate` reads the shard
outputs and runs IC matching, eliminations, IAS 21 translation, NCI
rollforward, equity-method investments, and produces consolidated FS, schedule,
and notes. Output is IFRS / IAS 21 / IAS 28 / IFRS 10 compliant by construction.

```yaml
# Excerpt — see configs/examples/group/mini_nestle.yaml for the full file
id: "MINI_NESTLE_2024_Q1"
presentation_currency: "CHF"
period: { start_date: "2024-01-01", length: quarterly }
defaults:
  accounting_framework: ifrs
  industry: manufacturing
  process_models: [o2c, p2p, h2r, r2r, audit]
ownership:
  parent_entity_code: NESTLE_SA
  entities:
    - { code: NESTLE_SA, country: CH, functional_currency: CHF,
        consolidation_method: parent }
    - { code: NESTLE_USA, country: US, functional_currency: USD,
        consolidation_method: full, ownership_percent: 1.0,
        parent_code: NESTLE_SA }
    - { code: NESTLE_DE, country: DE, functional_currency: EUR,
        consolidation_method: full, ownership_percent: 0.80,
        parent_code: NESTLE_SA }
intercompany:
  relationships:
    - { seller: NESTLE_SA, buyer: NESTLE_USA, types: [goods_sale],
        annual_volume: 5_000_000, transfer_pricing: cost_plus, markup_percent: 0.08 }
```

```bash
datasynth-data group generate \
  --config configs/examples/group/mini_nestle.yaml \
  --out ./group_archive
```

Expected output layout:

```
./group_archive/
├── manifest.json                       # canonical group manifest
├── entities/
│   ├── NESTLE_SA/                      # full single-entity archive per shard
│   ├── NESTLE_USA/
│   └── ...
├── consolidated/
│   ├── consolidated_financial_statements.json
│   ├── consolidation_schedule.json
│   ├── notes_to_consolidated_fs.json
│   ├── nci_rollforward.json
│   ├── cta_rollforward.json
│   ├── translation_worksheet.json
│   └── equity_method_investments.json
├── ic_eliminations/
│   └── ic_matching_coverage.json
└── shard_summary.json                  # per shard
```

Existing single-entity configs are untouched: `datasynth-data generate`
auto-detects whether the input is a `GroupConfig` (looking for the
`presentation_currency` / `ownership` keys) and dispatches to the v4.x
flow byte-for-byte unchanged when it isn't.

### Performance — measured on Standard_NC40ads_H100_v5 (40 vCPU / 320 GiB)

The v5.0 release was verified end-to-end on an Azure GPU host (the H100
sits idle for the audit pipeline — it's allocated for upcoming
RustGraph / RustCompute GPU work). All numbers come from
`/usr/bin/time -v` against `--release` builds.

| Workload | Wall-clock | Peak RSS | Output |
|---|---|---|---|
| Cold workspace build (17 crates, release) | 8 min 11 s | — | binaries |
| `cargo test --workspace --release` (229 result lines) | ~9 min | — | 0 failures |
| Mini-Nestlé `group generate` (5 entities, quarterly) | ~5 min | — | 1.5 GB |
| **ACME 2 000-entity `group generate`** | **5 min 32 s** | **60 GiB** | **66 GB** |
| ACME archive packed (zstd -3) | 35 s | — | 3.1 GB |

Per-entity output is **34 MB** (material profile) to **250 MB**
(flagship profile) — heterogeneous by `scoping_profile`. Banking /
KYC / AML data is intentionally disabled in shard mode (saves 29 GB
per entity); use the [vynfi-aml-100k](https://huggingface.co/datasets/VynFi/vynfi-aml-100k)
companion dataset for banking workloads.

### v5.0 release learnings

The Chunk-12 release verification on a real Azure VM caught a class of
integration bugs the chunk-level unit tests missed because they used
hand-rolled fixtures matching canonical-shape types. All seven were
fixed in commit `76ce191`:

1. **`build_entity_generator_config` didn't enable `financial_reporting`** —
   silently skipped Phase 15 of the orchestrator, so
   `period_close/trial_balances.json` was never written and the
   aggregate phase failed at "missing shard archive".
2. **Loader / orchestrator type mismatch** — `tb_loader` deserialised
   as `Vec<TrialBalance>` (datasynth-core canonical type) but the
   orchestrator emits `Vec<PeriodTrialBalance>` (datasynth-runtime
   type with different fields). Loader now reads both shapes.
3. **Strict "exactly 1 TB" check** rejected the orchestrator's
   per-month TBs (3 for a quarterly period). Loader now picks the
   latest fiscal period.
4. **Strict balance check** rejected fraud-injected unbalanced totals
   as corruption. Per CLAUDE.md the synthetic engine deliberately
   injects fraud entries with mismatched debits/credits — the
   imbalance IS the ground-truth fraud signal. Loader / pre_elim /
   post_elim / consolidated BS now log the imbalance instead of
   failing.
5. **Pre-elim currency check** rejected non-presentation-currency TBs
   as "needing translation first" — but in v5.0 the IAS 21 translation
   step is a sidecar artefact (`consolidated/translation_worksheet.json`),
   not run inline by the aggregate driver. Pre-elim's per-account sum
   doesn't actually need single-currency input. Downgraded to log.
6. **Equity-method clamp** — `compute_equity_method_investment`
   returned an error when share-of-loss would push carrying value
   below zero. Per IAS 28.38 / ASC 323-10-35-20 the investor
   *clamps at zero* and discontinues recognising further losses. Now
   clamps + warns (the suppressed loss should land in a separate
   v5.1 artefact).
7. **Banking bloat** — `BankingConfig.enabled` defaults to `true`,
   producing ~29 GB of AML transaction labels per entity. Disabled in
   shard mode for v5.0 (group-audit pipeline doesn't consume banking).

### Mini-Nestlé example

The repo's reference fixture is [`configs/examples/group/mini_nestle.yaml`](configs/examples/group/mini_nestle.yaml) — 5 entities (CHF/USD/EUR/BRL/CHF), 6 IC relationships including pattern-derived. Use it for local debugging:

```bash
./target/release/datasynth-data group generate \
  --config configs/examples/group/mini_nestle.yaml \
  --out ./mini_nestle/
```

---

## Roadmap

### v5.1 — Q3 2026 (planned)

- **Suppressed loss tracking** for equity-method investments below zero (IAS 28.38 memorandum record), surfaced as a separate `consolidated/equity_method_suppressed_losses.json` artefact.
- **Operating segments** per IFRS 8 — the v5.0 `notes_to_consolidated_fs.json` has a placeholder note; v5.1 wires it to the per-entity segment generator.
- **Full retained-earnings integration** of equity-method postings (replacing the v5.0 bridge account `3400`).
- **Configurable account-name dictionary** (currently a hard-coded static map of canonical codes) — wire through to the per-engagement chart-of-accounts master.
- **`tb_loader` cleanup** — once the orchestrator's writer emits the canonical `TrialBalance` shape directly (instead of `PeriodTrialBalance`), drop the dual-shape detection in the loader.

### v5.2 — Q4 2026 (planned)

- **Acquisition-date NCI measurement** — full-goodwill vs partial-goodwill choice per IFRS 3.19; currently v5.0 only handles steady-state.
- **Step-acquisition / divestiture rollforwards** — multi-period scenarios where ownership changes mid-year.
- **Goodwill impairment testing** (IAS 36) under the v5.0 group structure.
- **Hyperinflationary economies** (IAS 29) handling — special CTA treatment.

### v5.3 — H1 2027 (planned)

- **Emergent-fuzzy IC matching strategy** for group consolidation against external/legacy data (vs the v5.0 manifest-driven 100 %-by-construction strategy). Adds the `AmountDriftAboveTolerance` reason currently reserved as a placeholder in `UnmatchedReason`.
- **Multi-period consolidation** — v5.0 emits a single period; v5.3 runs N consecutive periods with NCI / CTA / equity-method rollforwards stitching automatically (currently the `--prior-period-aggregate` flag plumbs in opening NCI but doesn't drive a multi-period pipeline).

### Beyond — backlog (no committed timeline)

- **Segment-level intercompany matching** (IFRS 8 plus v5.0 IC).
- **Subsequent events** automation (currently a placeholder note).
- **Related-party transactions** disclosures generated from the manifest's IC graph.
- **Goodwill-impairment indicator** ML training data.
- **Performance**: target sub-2-min wall-clock for the 2 000-entity archive on a 96-vCPU host.

---

## Key Capabilities

### Enterprise Process Simulation

Every process chain generates cross-referenced master data, documents, and journal entries:

| Process Family | Scope |
|----------------|-------|
| **General Ledger** | Journal entries, chart of accounts (small/medium/large), ACDOCA |
| **Procure-to-Pay** | POs, goods receipts, vendor invoices, payments, three-way match |
| **Order-to-Cash** | Sales orders, deliveries, customer invoices, receipts, dunning |
| **Source-to-Contract** | Spend analysis, sourcing, RFx, bids, contracts, scorecards |
| **Hire-to-Retire** | Payroll, time & attendance, expenses, benefits, pensions, stock comp |
| **Manufacturing** | Production orders, BOM, WIP costing, quality inspections, cycle counts |
| **Financial Reporting** | BS/IS/CF, equity changes, KPIs, budgets, segment reporting, notes, XBRL |
| **Tax** | Multi-jurisdiction, VAT/GST, ASC 740/IAS 12 provisions, deferred tax |
| **Treasury** | Cash positioning, forecasts, pooling, hedging (ASC 815/IFRS 9), covenants |
| **ESG** | GHG Scope 1/2/3, energy/water/waste, diversity, GRI/SASB/TCFD |
| **Banking / AML** | 20 AML typologies, criminal networks, velocity features, KYC |
| **Audit** | ISA lifecycle, ISA 600 group audit, SOX 302/404, 10 methodology blueprints |
| **Intercompany** | IC matching, transfer pricing, eliminations, currency translation |
| **Period Close** | Depreciation, accruals, year-end closing, tax provisions |

### AI Capabilities

| Feature | Description | Feature Flag |
|---------|-------------|--------------|
| Neural Diffusion | Candle-powered score network (DDPM); end-to-end training + sampling. Orchestrator wired to honor `diffusion.backend: neural \| hybrid \| statistical` with graceful CPU fallback. GPU via `neural-cuda`. | `neural` / `neural-cuda` |
| Statistical Diffusion | Denoising / enhancement via the statistical `DiffusionBackend` — always on | — |
| LLM Config Generation | Natural language → YAML config (OpenAI/Anthropic/OpenRouter) | `llm` |
| **LLM Template Enrichment** | Offline deterministic CLI: expand vendor/customer/material pools via any OpenAI-compatible endpoint. Cached YAML, byte-identical runs. | `llm` |
| **LlmTemplateProvider** | Runtime LLM-backed provider wrapping the default one; opt-in per category with in-memory cache. | `llm` |
| Auto-Tune | Generate → evaluate → AI patch → regenerate closed loop | — |
| Adversarial Testing | ONNX model boundary probing via `ort` | `adversarial` |
| Anomaly Designer | LLM-designed fraud schemes adapted to control environment | — |
| Tabular Transformer | Masked column prediction for conditional generation | `neural` |
| GNN Graph Generator | Message-passing GNN for entity relationship structure | `neural` |

See [AI Capabilities](docs/book/src/ai/README.md) for details.

### Advanced distributions (v3.4–v4.0)

Every distribution knob in `config.distributions` now drives runtime behaviour:

| Sub-block | Effect |
|-----------|--------|
| `amounts` | Log-normal / Gaussian mixture models override the legacy amount sampler |
| `industry_profile` | Retail / manufacturing / financial-services / healthcare / technology preset mixtures |
| `pareto` | Heavy-tailed amount sampling (capex, strategic contracts, fraud) |
| `regime_changes` | Point-in-time regime events (acquisition, price-increase, …) + economic cycles + parameter drifts |
| `conditional` | Calendar-conditional amount distributions (e.g. Q4-larger) via `input_field ∈ {month, quarter, constant}` |
| `correlations` | Gaussian copula drives amount↔line_count correlation; Clayton/Gumbel/Frank/Student-t parsed and scheduled for v4.1 |
| `validation` | Benford, chi-squared, KS-log-uniform tests run post-generation; report attached to `EnhancedGenerationResult.statistical_validation` |

### Temporal awareness (v3.4.1–v3.4.3)

Shared `TemporalContext` bundle (multi-year holiday union + business-day calculator) threaded through P2P, O2C, time entries, expense reports, production orders, and accrual reversals. Posting dates snap to business days; 15 region calendars (US, DE, GB, FR, IT, ES, CA, CN, JP, IN, BR, MX, AU, SG, KR) supported out of the box.

### Counterfactual Simulation

Define scenarios with typed interventions, generate paired baseline/counterfactual datasets with causal DAG propagation:

```yaml
scenarios:
  enabled: true
  scenarios:
    - name: supply_chain_disruption
      interventions:
        - type: parameter_shift
          target: distributions.amounts.components[0].mu
          value: "6.5"
          timing: { start_month: 7, duration_months: 4, onset: sudden }
      constraints:
        preserve_accounting_identity: true
      output:
        paired: true
```

**11 pre-built scenarios** across fraud, control failures, macro shocks, and operational disruptions. See [Scenario Library](docs/book/src/scenarios/library.md).

### Accounting & Compliance Standards

US GAAP, IFRS, French GAAP (PCG), German GAAP (HGB), dual reporting. Revenue recognition (ASC 606/IFRS 15), leases (ASC 842/IFRS 16), fair value (ASC 820/IFRS 13), impairment, deferred tax, ECL, pensions, stock comp, business combinations, segment reporting. ISA (34 standards), PCAOB (19+), SOX 302/404, COSO 2013 (5 components, 17 principles). FEC, GoBD, and SAF-T (PT / PL / RO / NO / LU) audit file exports.

### Audit FSM Engine

YAML-driven methodology-agnostic state machine with 10 built-in blueprints (FSA, IA, KPMG, PwC, Deloitte, EY GAM, SOC 2, PCAOB, Regulatory). See [Audit FSM](docs/book/src/audit-fsm.md).

---

## Architecture

16 crates in a Rust workspace:

```
datasynth-cli              CLI binary (generate, validate, init, scenario, adversarial, audit, templates)
datasynth-server           REST / gRPC / WebSocket server with auth and rate limiting
datasynth-runtime          EnhancedOrchestrator (~30 phases, assertions, streaming, validation phase)
datasynth-generators       50+ generators across all process families, LLM enrichers
datasynth-banking          KYC/AML with 20 typologies and criminal networks
datasynth-eval             Evaluation framework, auto-tuning, adversarial testing
datasynth-config           YAML configuration, validation, industry presets
datasynth-core             306 domain models, distributions, diffusion, LLM provider, TemplateProvider, TemporalContext
datasynth-graph            Graph export (PyG, Neo4j, DGL, hypergraph)
datasynth-standards        IFRS, US GAAP, ISA, SOX, PCAOB standards
datasynth-audit-fsm        YAML-driven audit FSM (10 blueprints)
datasynth-audit-optimizer  Audit optimization, Monte Carlo, group audit simulation
datasynth-ocpm             OCEL 2.0 / XES 2.0 process mining
datasynth-fingerprint      Privacy-preserving fingerprint extraction and synthesis
datasynth-output           CSV, JSON, Parquet sinks with streaming
datasynth-test-utils       Test fixtures and utilities
```

See [Architecture](docs/book/src/architecture/crates.md) and [Generation Pipeline](docs/book/src/architecture/pipeline.md).

---

## Performance

| Metric | Value |
|--------|-------|
| Generation throughput | ~14,000 JEs/sec |
| XXL dataset (200K+ JEs, 3 companies, 36 months) | **20.6s** CSV-only |
| CSV-only speedup | 4x faster (skips JSON serialization) |
| Peak memory at scale | ~4.3 GB for 200K+ JEs |
| Determinism | Fully reproducible via seeded ChaCha8 RNG |

See [Performance Benchmarks](docs/book/src/architecture/performance.md).

---

## Python SDK

The previous open-source Python wrapper (`datasynth-py`) has been retired. For production Python integrations — including first-class support for Spark, dbt, Apache Airflow, MLflow, and enterprise blueprints — use the official commercial SDKs from **[VynFi](https://vynfi.com)**.

For ad-hoc Python usage against the open-source core, invoke the `datasynth-data` CLI via `subprocess` and read the generated CSV/JSON/Parquet outputs with pandas / polars / pyarrow.

---

## Server & Deployment

```bash
cargo run -p datasynth-server -- --rest-port 3000 --grpc-port 50051 --api-keys "key1,key2"
```

REST, gRPC, and WebSocket APIs with JWT/OIDC authentication, rate limiting, and RBAC. Docker + Kubernetes Helm chart included. See [Server & API](docs/book/src/server-api.md) and [Deployment Guide](deploy/README.md).

---

## Documentation

| Guide | Content |
|-------|---------|
| [Getting Started](docs/book/src/getting-started.md) | Installation, quick start, demo mode |
| [Configuration](docs/book/src/configuration.md) | YAML reference (40+ sections), presets, NL config |
| [CLI Reference](docs/book/src/cli-reference.md) | All commands and flags |
| [AI Capabilities](docs/book/src/ai/README.md) | Neural diffusion, auto-tune, adversarial, anomaly designer |
| [Scenario Engine](docs/book/src/scenarios/README.md) | Counterfactual simulation, scenario library, .dss format |
| [Audit FSM](docs/book/src/audit-fsm.md) | 10 blueprints, step dispatcher, C2CE lifecycle |
| [Banking & AML](docs/book/src/banking-aml.md) | 20 typologies, networks, velocity features |
| [Fingerprinting](docs/book/src/fingerprinting.md) | Extract → synthesize pipeline |
| [Architecture](docs/book/src/architecture/crates.md) | 16 crates, pipeline phases, performance |
| [Server & API](docs/book/src/server-api.md) | REST/gRPC/WebSocket, auth, rate limiting |
| [Deployment](deploy/README.md) | Docker, Kubernetes, systemd |
| [Contributing](CONTRIBUTING.md) | Development setup, PR guidelines |
| [Changelog](CHANGELOG.md) | Full version history |

Build the documentation site locally: `cd docs/book && mdbook serve`

---

## Citation

If you use DataSynth in academic work, please cite:

> Ivertowski, M. (2026). *DataSynth: Synthetic enterprise data generation for ML training, audit analytics, and system testing.* https://doi.org/10.13140/RG.2.2.13943.79523

```bibtex
@software{ivertowski_datasynth_2026,
  author  = {Ivertowski, Michael},
  title   = {DataSynth: Synthetic enterprise data generation for ML training, audit analytics, and system testing},
  year    = {2026},
  doi     = {10.13140/RG.2.2.13943.79523},
  url     = {https://doi.org/10.13140/RG.2.2.13943.79523}
}
```

---

## License

Copyright 2024-2026 Michael Ivertowski. Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).

---

## Support

Commercial support, custom development, and enterprise licensing: [vynfi.com](https://vynfi.com) | [GitHub Issues](https://github.com/mivertowski/SyntheticData/issues)

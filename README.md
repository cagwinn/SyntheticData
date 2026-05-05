# DataSynth

[![License](https://img.shields.io/badge/license-Apache%202.0-green.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)
[![CI](https://github.com/mivertowski/SyntheticData/actions/workflows/ci.yml/badge.svg)](https://github.com/mivertowski/SyntheticData/actions/workflows/ci.yml)
[![DOI](https://img.shields.io/badge/DOI-10.13140%2FRG.2.2.13943.79523-blue.svg)](https://doi.org/10.13140/RG.2.2.13943.79523)

**Synthetic enterprise data generation for ML training, audit analytics, and system testing.**

DataSynth generates statistically realistic, fully interconnected enterprise
financial data across 20+ process families. Generated data respects accounting
identities (debits = credits, Assets = Liabilities + Equity), follows empirical
distributions (Benford's Law, log-normal mixtures, Pareto heavy tails, Gaussian
copula correlations), and maintains referential integrity across 100+ output
tables. Generation-time assertions enforce these invariants at scale.

The release ships a **multi-entity group audit engine** with three-phase
manifest / shard / aggregate execution, IFRS / IAS 21 / IAS 28 / IFRS 10
compliant by construction, plus a typed **audit-methodology layer** covering
the Big 4 ISA spine, jurisdictional overlays for seven jurisdictions,
KYC / AML workflows, banking-form ontologies, Bayesian RMM scoring, the
L4 audit-graph schema, and SHA-256 Merkle tamper-evidence for working-paper
bundles.

**[Documentation](docs/book/src/SUMMARY.md)** · **[Commercial SDKs](https://vynfi.com)** · **[Changelog](CHANGELOG.md)**

---

## Example Datasets

Pre-generated reference datasets at [huggingface.co/VynFi](https://huggingface.co/VynFi):

| Dataset | Scale | Description |
|---------|------:|-------------|
| [vynfi-group-audit-enterprise-2000](https://huggingface.co/datasets/VynFi/vynfi-group-audit-enterprise-2000) | 2 000 entities | Multinational ACME holding with 4 functional currencies, 4 759 IC pairs (91.6 % matched), full IFRS-compliant consolidated FS + schedule + notes + CTA + NCI + equity-method rollforwards. |
| [vynfi-journal-entries-1m](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m) | 2.1 M JE lines | Manufacturing-sector denormalised JE table with 6.92 % fraud rate, ISA 240 manual flag, GL chart of accounts. |
| [vynfi-aml-100k](https://huggingface.co/datasets/VynFi/vynfi-aml-100k) | 749 K | Banking transactions with AML labels, 14 velocity features, 59 columns. |
| [vynfi-audit-p2p](https://huggingface.co/datasets/VynFi/vynfi-audit-p2p) | 234 docs | P2P document chain (PO/GR/VI/Payment) with fraud labels. |
| [vynfi-ocel-manufacturing](https://huggingface.co/datasets/VynFi/vynfi-ocel-manufacturing) | 344 events | OCEL event log for process mining (pm4py, Celonis). |
| [vynfi-supply-chain-ocel](https://huggingface.co/datasets/VynFi/vynfi-supply-chain-ocel) | — | Supply-chain OCEL event log for cross-process mining. |
| [vynfi-sar-narratives](https://huggingface.co/datasets/VynFi/vynfi-sar-narratives) | — | Suspicious-activity-report narratives for AML training. |

```python
from datasets import load_dataset
ds = load_dataset("VynFi/vynfi-aml-100k", split="train")
df = ds.to_pandas()
```

All datasets are Apache 2.0, entirely synthetic, no PII.

---

## Quick Start

```bash
git clone https://github.com/mivertowski/SyntheticData.git && cd SyntheticData
cargo build --release

# Demo — generates a complete dataset with defaults
./target/release/datasynth-data generate --demo --output ./output

# Initialise + generate from a config
./target/release/datasynth-data init --industry manufacturing --complexity medium -o config.yaml
./target/release/datasynth-data generate --config config.yaml --output ./output

# Group audit pipeline (multi-entity consolidation)
./target/release/datasynth-data group generate \
  --config configs/examples/group/mini_nestle.yaml \
  --out ./group_archive

# Counterfactual scenarios
./target/release/datasynth-data scenario list --config config.yaml
./target/release/datasynth-data scenario generate --config config.yaml --output ./output

# Auto-tuning loop: generate → evaluate → AI patch → regenerate
./target/release/datasynth-data generate --config config.yaml --output ./output \
  --auto-tune --max-iterations 3

# AI-powered config generation (set OPENAI_API_KEY, ANTHROPIC_API_KEY, or OPENROUTER_API_KEY)
cargo build --release --features llm
OPENAI_API_KEY=sk-... ./target/release/datasynth-data init \
  --from-description "12 months of mid-market retail data with fraud and SOX controls" \
  -o config.yaml
```

See the [CLI reference](docs/book/src/cli-reference.md) for all commands and flags.

---

## Group Audit Simulation

A **multi-entity group audit engine** layered above the single-entity pipeline.
Per-entity generation stays byte-identical to the standalone flow; consolidation
logic lives entirely in the `datasynth-group` crate.

The pipeline is a **three-phase model**:

1. **`manifest`** resolves a `GroupConfig` into a deterministic, content-addressable
   `GroupManifest` (entities, periods, ownership graph, IC pair plan, shard plan,
   FX / CoA masters).
2. **`shard`** drives the orchestrator for one shard of entities and writes a
   full single-entity archive per entity under `entities/{code}/`.
3. **`aggregate`** reads the shard outputs and runs IC matching, eliminations,
   IAS 21 translation, NCI rollforward, equity-method investments, CGU
   goodwill-impairment testing, and produces consolidated FS, schedule, and
   notes.

Output is IFRS / IAS 21 / IAS 28 / IFRS 10 compliant by construction.
Multi-period engagements stitch opening balances + NCI + CTA + equity-method
carryforwards forward through the chain helpers — no caller plumbing required.

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

Output layout:

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
│   ├── equity_method_investments.json
│   ├── equity_method_suppressed_losses.json
│   └── cgu_impairment_tests.json
├── ic_eliminations/
│   └── ic_matching_coverage.json
└── shard_summary.json
```

Existing single-entity configs continue to work unchanged: `datasynth-data
generate` auto-detects whether the input is a `GroupConfig` and dispatches to
the single-entity flow when it isn't.

### Standards coverage

| Standard | What's modelled |
|----------|-----------------|
| **IAS 21** § 39 / § 42(b) | Closing-rate translation, CTA rollforward; closing-rate-for-all-items in hyperinflationary economies |
| **IAS 28** § 22 / § 38 | Equity-method investments; suppressed-loss tracking with recovery-against-future-profits |
| **IAS 29** § 12 | Indexed restatement before IAS 21 closing-rate translation |
| **IAS 36** § 10 / § 80 / § 104 / § 124 | CGU definition, acquisition-date goodwill allocation, impairment-loss allocation, no-reversal rule |
| **IFRS 3** § 19 / § 42 | Acquisition-date NCI measurement (full vs partial goodwill); mid-period ControlGained re-measurement at fair value |
| **IFRS 5 / ASC 810** | NCI presented separately from controlling-interest equity |
| **IFRS 8.13** / **ASC 280-10-50-41** | Operating-segment disclosures derived from the ownership graph |
| **IFRS 10.23** / **IFRS 10.B97** | Equity-transaction adjustment for within-control ownership changes; deconsolidation on control loss |

---

## Audit-Methodology Layer

The `datasynth-audit-fsm` crate ships a typed audit-methodology layer derived
from the [AuditMethodology](https://github.com/mivertowski/AuditMethodology)
companion repo. All blueprints, overlays, ontologies, and form schemas are
embedded via `include_str!` so loaders are zero-I/O at runtime.

| Module | What it carries |
|--------|-----------------|
| `big4_methodology` | ISA-derived common spine (4 phases, 17 procedures) + 4 firm overlays (EY GAM, PwC Aura, KPMG Clara, Deloitte Omnia) + cross-firm equivalence map |
| `jurisdictional_overlay` | 7 overlays — PCAOB, EU CSRD, UK FRC, ASIC, JFSA, ACRA, HKICPA — 39 procedures total, AND-logic resolution on `(jurisdiction × registrant_type)` |
| `methodology_blueprint` | ISA 600 (Revised, Dec 2022) + CSRD limited-assurance methodology blueprints |
| `kyc_blueprint` | 6 KYC / AML workflows — private banking, correspondent banking, crypto-CASP, periodic KYC, SAR escalation, sanctions-hit remediation |
| `banking_forms` | 7 banking form ontologies (MROS SAR + 4 UBS + Wolfsberg CBDDQ + FCCQ) and a 228-entry cross-form evidence index unifying fields under canonical terms |
| `scenario_library` | 15 deterministic engagement scenarios with expected outcomes (opinion type, going-concern conclusion, EOM paragraph, acceptance gate) for FSM verification |
| `rmm_scoring` | Bayesian RMM scoring — 12-factor taxonomy (7 inherent + 5 control), conjugate Beta-Bernoulli updates, RMM = IR × CR aggregation |
| `l4_graph` | Typed property-graph schema (11 node types × 10 edge types) — entity → component → account → assertion / risk → control → procedure → evidence / working paper / finding |
| `working_paper_merkle` | SHA-256 Merkle tree with replay-protected inclusion proofs; ISA 230 working-paper / engagement-bundle types; canonical `bundle_root` ordered by `wp_id` |

The legacy YAML-driven FSM engine (10 built-in blueprints — FSA, IA, KPMG, PwC,
Deloitte, EY GAM, SOC 2, PCAOB, Regulatory) remains available alongside the
new modules.

---

## Capabilities

### Enterprise process simulation

Every process chain generates cross-referenced master data, documents, and
journal entries:

| Process Family | Scope |
|----------------|-------|
| **General Ledger** | Journal entries, chart of accounts (small / medium / large), ACDOCA |
| **Procure-to-Pay** | POs, goods receipts, vendor invoices, payments, three-way match |
| **Order-to-Cash** | Sales orders, deliveries, customer invoices, receipts, dunning |
| **Source-to-Contract** | Spend analysis, sourcing, RFx, bids, contracts, scorecards |
| **Hire-to-Retire** | Payroll, time & attendance, expenses, benefits, pensions, stock comp |
| **Manufacturing** | Production orders, BOM, WIP costing, quality inspections, cycle counts |
| **Financial Reporting** | BS / IS / CF, equity changes, KPIs, budgets, segment reporting, notes, XBRL |
| **Tax** | Multi-jurisdiction, VAT / GST, ASC 740 / IAS 12 provisions, deferred tax |
| **Treasury** | Cash positioning, forecasts, pooling, hedging (ASC 815 / IFRS 9), covenants |
| **ESG** | GHG Scope 1/2/3, energy / water / waste, diversity, GRI / SASB / TCFD |
| **Banking / AML** | 20 AML typologies, criminal networks, velocity features, KYC |
| **Audit** | ISA lifecycle, ISA 600 group audit, SOX 302/404, methodology blueprints |
| **Intercompany** | IC matching, transfer pricing, eliminations, currency translation |
| **Period Close** | Depreciation, accruals, year-end closing, tax provisions |

### AI capabilities

| Feature | Description | Feature Flag |
|---------|-------------|--------------|
| Neural diffusion | Candle-powered score network (DDPM); end-to-end training + sampling. GPU via `neural-cuda` with graceful CPU fallback. | `neural` / `neural-cuda` |
| Statistical diffusion | Denoising / enhancement via the statistical `DiffusionBackend` — always on | — |
| LLM config generation | Natural language → YAML config (OpenAI / Anthropic / OpenRouter) | `llm` |
| LLM template enrichment | Offline deterministic CLI: expand vendor / customer / material pools via any OpenAI-compatible endpoint. Cached YAML, byte-identical runs. | `llm` |
| Auto-tune | Generate → evaluate → AI patch → regenerate closed loop | — |
| Adversarial testing | ONNX model boundary probing via `ort` | `adversarial` |
| Anomaly designer | LLM-designed fraud schemes adapted to the control environment | — |
| Tabular transformer | Masked column prediction for conditional generation | `neural` |
| GNN graph generator | Message-passing GNN for entity-relationship structure | `neural` |

See [AI Capabilities](docs/book/src/ai/README.md) for details.

### Distributions and temporal awareness

Log-normal / Gaussian mixture amount sampling with industry presets
(retail / manufacturing / financial-services / healthcare / technology),
Pareto heavy tails, Gaussian / Clayton / Gumbel / Frank / Student-t
copulas with rank-preserving inverse-CDF marginals, point-in-time regime
events, calendar-conditional distributions, Benford / chi-squared / KS
post-generation validation. A shared `TemporalContext` (multi-year holiday
union + business-day calculator across 15 region calendars: US, DE, GB, FR,
IT, ES, CA, CN, JP, IN, BR, MX, AU, SG, KR) is threaded through every
process family.

### Counterfactual simulation

Define scenarios with typed interventions, then generate paired
baseline / counterfactual datasets with causal-DAG propagation:

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

11 pre-built scenarios across fraud, control failures, macro shocks, and
operational disruptions. See [Scenario Library](docs/book/src/scenarios/library.md).

### Accounting and compliance standards

US GAAP, IFRS, French GAAP (PCG), German GAAP (HGB), dual reporting. Revenue
recognition (ASC 606 / IFRS 15), leases (ASC 842 / IFRS 16), fair value
(ASC 820 / IFRS 13), impairment, deferred tax, ECL, pensions, stock comp,
business combinations, segment reporting. ISA (34 standards), PCAOB (19+),
SOX 302 / 404, COSO 2013 (5 components, 17 principles). FEC, GoBD, and SAF-T
(PT / PL / RO / NO / LU) audit-file exports plus a 27-table SAP integration
pack (BKPF / BSEG / ACDOCA + master data + subledger).

---

## Architecture

17 crates in a Rust workspace:

```
datasynth-cli              CLI binary (generate, validate, init, scenario, adversarial, audit, templates)
datasynth-server           REST / gRPC / WebSocket server with auth and rate limiting
datasynth-runtime          EnhancedOrchestrator (~30 phases), assertions, streaming, validation phase
datasynth-generators       50+ generators across all process families, LLM enrichers
datasynth-banking          KYC / AML with 20 typologies and criminal networks
datasynth-eval             Evaluation framework, auto-tuning, adversarial testing
datasynth-config           YAML configuration, validation, industry presets
datasynth-core             306 domain models, distributions, diffusion, LLM provider, TemplateProvider, TemporalContext
datasynth-graph            Graph export (PyTorch Geometric, Neo4j, DGL, hypergraph)
datasynth-standards        IFRS, US GAAP, French GAAP, German GAAP, ISA, SOX, PCAOB
datasynth-audit-fsm        YAML-driven audit FSM + Big 4 spine + jurisdictional overlays + KYC + banking forms + RMM + L4 graph + Merkle bundle
datasynth-audit-optimizer  Audit optimization, Monte Carlo, group-audit simulation
datasynth-group            Group audit engine (manifest / shard / aggregate)
datasynth-ocpm             OCEL 2.0 / XES 2.0 process mining
datasynth-fingerprint      Privacy-preserving fingerprint extraction and synthesis
datasynth-output           CSV / JSON / Parquet sinks with streaming
datasynth-test-utils       Test fixtures and utilities
```

See [Architecture](docs/book/src/architecture/crates.md) and [Generation Pipeline](docs/book/src/architecture/pipeline.md).

---

## Performance

Measured on Standard_NC40ads_H100_v5 (40 vCPU / 320 GiB) against `--release`
builds:

| Workload | Wall-clock | Peak RSS | Output |
|----------|-----------:|---------:|-------:|
| Single-entity generation throughput | ~14 000 JEs/sec | — | — |
| XXL dataset (200 K+ JEs, 3 companies, 36 months) | 20.6 s | 4.3 GB | CSV |
| Mini-Nestlé `group generate` (5 entities, quarterly) | ~5 min | — | 1.5 GB |
| ACME 2 000-entity `group generate` | 5 min 32 s | 60 GiB | 66 GB |
| ACME archive packed (zstd −3) | 35 s | — | 3.1 GB |

Per-entity output ranges from 34 MB (material profile) to 250 MB (flagship
profile), heterogeneous by `scoping_profile`. Banking / KYC / AML data is
disabled by default in shard mode (saves ~29 GB per entity); use the
[vynfi-aml-100k](https://huggingface.co/datasets/VynFi/vynfi-aml-100k)
companion dataset for banking workloads.

Generation is fully reproducible via seeded ChaCha8 RNG; standalone
in-process generation with `parallel_shards: false` produces byte-identical
archives across runs. See [Performance Benchmarks](docs/book/src/architecture/performance.md).

---

## Server and Deployment

```bash
cargo run -p datasynth-server -- --rest-port 3000 --grpc-port 50051 --api-keys "key1,key2"
```

REST, gRPC, and WebSocket APIs with JWT / OIDC authentication, rate limiting,
and RBAC. Docker + Kubernetes Helm chart included. See [Server & API](docs/book/src/server-api.md)
and [Deployment Guide](deploy/README.md).

---

## Python integration

The previous open-source Python wrapper (`datasynth-py`) has been retired.
For production Python integrations — including first-class support for Spark,
dbt, Apache Airflow, MLflow, and enterprise blueprints — use the official
commercial SDKs from **[VynFi](https://vynfi.com)**.

For ad-hoc Python usage against the open-source core, invoke the
`datasynth-data` CLI via `subprocess` and read the generated CSV / JSON /
Parquet outputs with pandas / polars / pyarrow.

---

## Documentation

| Guide | Content |
|-------|---------|
| [Getting Started](docs/book/src/getting-started.md) | Installation, quick start, demo mode |
| [Configuration](docs/book/src/configuration.md) | YAML reference (40+ sections), presets, NL config |
| [CLI Reference](docs/book/src/cli-reference.md) | All commands and flags |
| [AI Capabilities](docs/book/src/ai/README.md) | Neural diffusion, auto-tune, adversarial, anomaly designer |
| [Scenario Engine](docs/book/src/scenarios/README.md) | Counterfactual simulation, scenario library |
| [Audit FSM](docs/book/src/audit-fsm.md) | Methodology blueprints, step dispatcher, C2CE lifecycle |
| [Banking & AML](docs/book/src/banking-aml.md) | 20 typologies, networks, velocity features |
| [Fingerprinting](docs/book/src/fingerprinting.md) | Extract → synthesize pipeline |
| [Architecture](docs/book/src/architecture/crates.md) | 17 crates, pipeline phases, performance |
| [Server & API](docs/book/src/server-api.md) | REST / gRPC / WebSocket, auth, rate limiting |
| [Deployment](deploy/README.md) | Docker, Kubernetes, systemd |
| [Contributing](CONTRIBUTING.md) | Development setup, PR guidelines |
| [Changelog](CHANGELOG.md) | Full version history |

Build the documentation site locally: `cd docs/book && mdbook serve`.

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

Copyright 2024–2026 Michael Ivertowski. Licensed under the Apache License,
Version 2.0. See [LICENSE](LICENSE).

---

## Support

- **Issues**: [github.com/mivertowski/SyntheticData/issues](https://github.com/mivertowski/SyntheticData/issues)
- **Commercial support and SDKs**: [vynfi.com](https://vynfi.com)

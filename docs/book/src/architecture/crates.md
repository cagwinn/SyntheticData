# Crate Structure

DataSynth is a Rust workspace with 17 crates. Each crate has a focused responsibility.

## Crate Map

| Crate | Purpose |
|-------|---------|
| `datasynth-cli` | Binary entry point. Subcommands: generate, validate, init, info, fingerprint, scenario, adversarial, audit |
| `datasynth-server` | REST + gRPC + WebSocket server for remote generation |
| `datasynth-runtime` | `EnhancedOrchestrator` coordinates the full generation pipeline |
| `datasynth-generators` | 50+ data generators: JEs, master data, document flows, HR, manufacturing, anomalies, LLM enrichment |
| `datasynth-banking` | KYC/AML banking with 20 fraud typologies and criminal network generation |
| `datasynth-ocpm` | OCEL 2.0 process mining event logs (P2P/O2C) |
| `datasynth-fingerprint` | Privacy-preserving fingerprint extraction, synthesis, and evaluation |
| `datasynth-standards` | Accounting standards (IFRS, US GAAP, French/German GAAP) and audit standards (ISA, PCAOB, SOX) |
| `datasynth-graph` | Graph export: PyTorch Geometric, Neo4j, DGL, hypergraph |
| `datasynth-graph-export` | Unified graph export with node/edge builders (v1.3.0+ schema) |
| `datasynth-eval` | Evaluation framework: Benford, distributions, coherence, quality, ML, auto-tuning, adversarial |
| `datasynth-config` | Configuration schema, validation, presets, YAML deserialization |
| `datasynth-core` | Domain models (306 types), traits, distributions, resource guards, LLM provider, diffusion |
| `datasynth-output` | Output sinks: CSV, JSON, Parquet, SAP, FEC, GoBD writers |
| `datasynth-audit-fsm` | YAML-driven audit FSM engine with 10 built-in methodology blueprints |
| `datasynth-audit-optimizer` | Audit coverage optimization |
| `datasynth-test-utils` | Shared test utilities and fixtures |

## Dependency Graph (Simplified)

```
datasynth-cli
  ├── datasynth-runtime
  │     ├── datasynth-generators
  │     │     ├── datasynth-core
  │     │     └── datasynth-config
  │     ├── datasynth-banking
  │     ├── datasynth-standards
  │     ├── datasynth-ocpm
  │     ├── datasynth-eval
  │     ├── datasynth-graph
  │     ├── datasynth-fingerprint
  │     ├── datasynth-output
  │     └── datasynth-audit-fsm
  ├── datasynth-config
  └── datasynth-output

datasynth-server
  └── datasynth-runtime (same tree)
```

`datasynth-core` is the leaf dependency -- it defines all domain models, distributions, and traits that other crates consume. `datasynth-config` depends only on `datasynth-core`.

## Key Design Decisions

- **No circular dependencies** -- Strict DAG enforced by Cargo workspace
- **Feature-gated AI** -- `llm`, `adversarial`, `neural` are optional
- **Trait-based extensibility** -- `DiffusionBackend`, `LlmProvider`, `DataSource` traits allow swapping implementations
- **Zero-copy where possible** -- Streaming JSON writer avoids buffering entire datasets in memory

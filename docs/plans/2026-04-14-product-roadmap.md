# DataSynth Product Roadmap

**Date**: 2026-04-14
**Status**: Active
**Replaces**: Individual phase plans from docs/plans/ (which remain as reference)

---

## Where We Stand

DataSynth v2.4.0 is a mature, production-grade synthetic enterprise data platform:

- **16 crates**, 306 domain models, 50+ generators, 20 AML typologies
- **Full accounting lifecycle**: GL, AR/AP, P2P/O2C/S2C, manufacturing cost flow, treasury, tax, payroll, ESG
- **10 audit methodology blueprints** (ISA, PCAOB, Big 4 approaches, SOC 2)
- **AI layer**: neural diffusion, LLM config generation, adversarial testing, GNN graph generation
- **Dual API**: gRPC + REST with WebSocket streaming, JWT/RBAC, Redis rate limiting
- **Python SDK**: async client, Spark/dbt/Airflow/MLflow integrations
- **40+ releases** shipped in 3.5 months

### What's been shipped vs. what was planned

| Plan | Status |
|------|--------|
| Domain expansion (tax, treasury, project, ESG) | Shipped (v0.9-0.11) |
| Country pack integration (17 points) | Shipped (v0.7-1.0) |
| Feature-complete reference release | Shipped (v1.0.0) |
| Enterprise group audit (ISA 600) | Shipped (v1.3.0) |
| Audit FSM engine (YAML-driven, 10 blueprints) | Shipped (v1.5.0-2.0.0) |
| Financial coherence (manufacturing→treasury→tax→statements) | Shipped (v2.1.0-2.2.0) |
| Banking/AML realism (14 typologies, networks, lifecycle) | Shipped (v2.3.0) |
| AI capabilities Phase 1 (neural, LLM config, tuning, adversarial) | Shipped (v2.4.0) |
| Counterfactual simulation engine | Designed, not started |
| Unified generation pipeline (sessions, checkpoints) | Designed, not started |
| Compliance standards framework (registry, jurisdictions) | Designed, not started |
| AI Phase 2-3 (LLM intelligence, advanced models) | Designed, stubs shipped |

---

## The Roadmap

```
v2.5               v3.0                  v3.1+
Hardening &        Simulation            AI-Native &
Coherence          Platform              Ecosystem
──────────────     ──────────────        ──────────────
May 2026           Jun-Jul 2026          Aug+ 2026
```

---

## v2.5 — Hardening & Coherence (May 2026)

**Theme**: Close the gaps. Every generated dataset should be internally consistent across all domains — GL, subledgers, document flows, banking, audit, OCPM — without manual post-processing.

### 2.5.1 Cross-Domain Coherence Tightening

| Item | Description | Crate |
|------|-------------|-------|
| IC reconciliation validation | Verify that intercompany balances net to zero after elimination. Currently generates IC transactions but doesn't validate the consolidated result. | runtime, eval |
| Manufacturing end-to-end orchestration | Tie production orders → WIP JEs → FG receipt → delivery → COGS in a single coherent flow. Components exist but orchestration is loose. | generators, runtime |
| Payroll↔HR↔GL three-way proof | Payroll JEs should reconcile to HR headcount × compensation rates. Add a coherence validator. | eval |
| Treasury↔Cash flow↔Bank reconciliation | Cash positions should match GL cash accounts should match bank reconciliation should match cash flow statement operating section. | eval |
| Period-close balance proof | Trial balance should foot. Assets = L + E. Retained earnings = prior RE + NI - dividends. Enforce as a generation-time assertion, not just an eval check. | runtime |

### 2.5.2 Performance & Scale

| Item | Description | Crate |
|------|-------------|-------|
| Large dataset benchmarking | Validate generation at 100K, 500K, 1M+ JEs. Profile memory, identify bottlenecks. Current throughput is ~200K entries/sec but hasn't been tested at scale with all modules enabled. | runtime, cli |
| Streaming memory optimization | With all 113+ output files enabled, peak memory can spike. Implement streaming-to-disk for large collections (subledger aging, banking transactions). | output, cli |
| Parallel phase execution | Some orchestrator phases are independent (e.g., ESG and treasury) but run sequentially. Identify safe parallelization opportunities. | runtime |
| Neural backend CPU throughput | Target: 10K rows/sec for 20-column dataset on CPU. Benchmark and optimize the candle-based diffusion generation. | core (neural) |

### 2.5.3 Evaluation Hardening

| Item | Description | Crate |
|------|-------------|-------|
| Drift detection across periods | When generating multi-period data (12+ months), validate that drift parameters produce realistic temporal evolution — not just that individual periods pass Benford. | eval |
| Distribution calibration to industry benchmarks | The eval module checks statistical properties but doesn't compare against real-world industry norms (e.g., retail has different amount distributions than manufacturing). Add benchmark datasets per industry. | eval, config |
| Causal consistency validation | When causal DAG parameters are set, verify that downstream effects actually appear in the generated data (e.g., recession → lower revenue → higher bad debt). | eval |
| End-to-end golden-path test | A single integration test that generates a medium-complexity dataset with ALL modules enabled and runs ALL evaluators. Currently modules are tested in isolation. | test-utils, eval |

### 2.5.4 Developer Experience

| Item | Description | Crate |
|------|-------------|-------|
| `--from-description` with real LLM | Currently uses MockLlmProvider. Wire the HttpLlmProvider when `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` is set. | cli, core |
| Config validation CLI | `datasynth-data validate --config config.yaml` should report ALL issues (not just the first parse error), with suggestions. | cli, config |
| Preset gallery | `datasynth-data info --presets` should show a rich listing of all presets with what they enable, expected output size, and example commands. | cli |
| Error messages | Review the 50+ `SynthError::generation()` calls in the AI modules for actionable error messages. Many just wrap candle errors without context. | core, eval |

---

## v3.0 — Simulation Platform (Jun-Jul 2026)

**Theme**: Transform DataSynth from a one-shot generator into a stateful simulation engine. Users define scenarios, explore what-if questions, and get paired baseline/counterfactual datasets.

### 3.0.1 Unified Generation Pipeline

| Item | Description | Ref |
|------|-------------|-----|
| `GenerationSession` state machine | Stateful session supporting: init → configure → generate(period) → checkpoint → resume. Enables incremental period-by-period generation. | [unified-pipeline](2026-03-02-unified-generation-pipeline-design.md) |
| Checkpoint serialization (`.dss` files) | Serialize full generation state (RNG, accumulators, entity registries) to disk. Resume from any checkpoint. | unified-pipeline |
| Period-incremental orchestrator | Refactor `EnhancedOrchestrator::generate()` to accept a period range and prior-state checkpoint. Currently generates all periods in one call. | runtime |
| NDJSON streaming with phase sinks | Each orchestrator phase writes to its own sink in real time. Enables monitoring and early-abort. | runtime, output |

### 3.0.2 Counterfactual Simulation Engine

| Item | Description | Ref |
|------|-------------|-----|
| Scenario definition schema | YAML `scenarios:` section with typed interventions (entity events, parameter shifts, control failures, macro shocks). | [counterfactual](2026-02-19-counterfactual-simulation-roadmap.md) Phase 1 |
| Paired generation engine | Generate baseline + counterfactual sharing the same seed. Divergence occurs naturally at the intervention point. | counterfactual Phase 1 |
| Scenario diff output | `impact_summary.json`, `record_level_diff.csv`, `intervention_trace.json` comparing baseline vs counterfactual. | counterfactual Phase 1 |
| CLI integration | `datasynth-data generate --scenario supply_chain_disruption_q3` | counterfactual Phase 1 |
| Fraud scenario pack library | 10+ pre-built scenarios: vendor collusion ring, management override, procurement kickback, ghost employee, channel stuffing, SOX material weakness, IT control breakdown. | counterfactual Phase 5 (accelerated) |

### 3.0.3 Compliance Standards Framework

| Item | Description | Ref |
|------|-------------|-----|
| Standards registry | `ComplianceStandard` model with versioned requirements, cross-references between IFRS/ISA/SOX/ASC. | [compliance](2026-03-09-compliance-regulations-implementation.md) Phase 1-2 |
| Jurisdiction profiles | Country-specific standard applicability (US → SOX + US GAAP + PCAOB; DE → HGB + ISA + EU). | compliance Phase 3 |
| Compliance finding generation | Given a dataset and jurisdiction, generate realistic compliance findings with severity, remediation, and deadline. | compliance Phase 4-5 |
| Graph integration | Standard, Regulation, AuditProcedure as node types in the entity graph. Cross-reference edges to JEs, controls, findings. | compliance Phase 6 |

---

## v3.1+ — AI-Native & Ecosystem (Aug+ 2026)

**Theme**: AI moves from optional augmentation to a core generation pathway. The ecosystem expands with community scenarios, model marketplace, and first-class integrations.

### 3.1.1 AI Phase 2: Intelligent Generation

| Item | Description |
|------|-------------|
| Real LLM integration for NL config | Multi-turn refinement: "make it larger", "add fraud", "switch to IFRS". Interactive session via CLI or API. |
| Eval-driven generation loop (closed) | `datasynth-data generate --auto-tune --max-iterations 5` — generate → evaluate → AI suggests patches → regenerate → converge. |
| Smart anomaly injection | LLM designs novel fraud patterns per run based on the specific company profile and control weaknesses. Goes beyond the 4 fallback templates. |
| Training data from fingerprints | `fingerprint extract` → train neural diffusion → generate. Privacy-preserving pipeline: real data → fingerprint → neural model → synthetic data that matches the actual joint distribution. |

### 3.1.2 AI Phase 3: Advanced Models

| Item | Description |
|------|-------------|
| Tabular transformer integration | Wire `TrainedTabularTransformer` into the generation pipeline for conditional column prediction (e.g., "given this vendor profile, generate realistic invoice amounts"). Currently a standalone module. |
| GNN-informed entity graph generation | Wire `TrainedGnnGenerator` into the entity graph builder to produce structurally realistic relationship networks. Currently standalone. |
| ONNX adversarial CLI | `datasynth-data adversarial --model fraud_detector.onnx --probes 10000` — probe decision boundaries and report vulnerability analysis. |
| GPU acceleration | Test and document CUDA feature flag for neural backends. Benchmark CPU vs GPU for different dataset sizes. |

### 3.1.3 Ecosystem

| Item | Description |
|------|-------------|
| Scenario marketplace | Community-contributed `.dss` scenario files. Browse, rate, compose named scenarios. |
| Challenge platform | Fraud detection challenge: generate scenario with embedded fraud, participants build detection models, score by AUC on held-out counterfactuals. |
| VS Code extension | Language server for DataSynth YAML configs — autocomplete, validation, hover docs. |
| Terraform/Pulumi provider | Infrastructure-as-code for DataSynth server deployment. |
| Webhook integrations | Post generation events to Slack, Teams, PagerDuty. Server already has WebSocket; add outbound hooks. |

### 3.1.4 Hardening (Ongoing)

| Item | Description |
|------|-------------|
| API versioning | gRPC + REST endpoint versioning strategy (v1/v2 prefixes, deprecation headers). |
| Security audit | Third-party review of authentication, RBAC, and data isolation in multi-tenant server mode. |
| SBOM generation | Software bill of materials for enterprise compliance. |
| Determinism certification | Formal test that identical config+seed produces byte-identical output across platforms (Linux/macOS/Windows). Already seed-controlled but not formally verified at the output file level. |
| Documentation overhaul | Move from CLAUDE.md as sole reference to a proper mdBook or Docusaurus site with tutorials, API reference, and cookbook. |

---

## Priority Matrix

| Priority | Item | Version | Effort |
|----------|------|---------|--------|
| **P0 — Ship quality** | Cross-domain coherence (IC, manufacturing, payroll) | v2.5 | M |
| **P0 — Ship quality** | End-to-end golden-path test | v2.5 | M |
| **P0 — Ship quality** | Period-close balance proof as assertion | v2.5 | S |
| **P1 — Differentiation** | Scenario definition + paired generation | v3.0 | L |
| **P1 — Differentiation** | GenerationSession + checkpoints | v3.0 | L |
| **P1 — Differentiation** | Fraud scenario pack library (10+) | v3.0 | M |
| **P1 — Differentiation** | Compliance standards framework (Phase 1-3) | v3.0 | L |
| **P2 — Scale** | Large dataset benchmarking (1M+ JEs) | v2.5 | M |
| **P2 — Scale** | Streaming memory optimization | v2.5 | M |
| **P2 — Scale** | Neural backend CPU throughput | v2.5 | S |
| **P3 — AI depth** | Closed-loop auto-tune CLI | v3.1 | M |
| **P3 — AI depth** | Real LLM multi-turn config refinement | v3.1 | M |
| **P3 — AI depth** | Fingerprint → neural → generate pipeline | v3.1 | L |
| **P3 — AI depth** | Tabular transformer + GNN pipeline integration | v3.1 | M |
| **P4 — Ecosystem** | Config validation improvements | v2.5 | S |
| **P4 — Ecosystem** | Scenario marketplace | v3.1+ | L |
| **P4 — Ecosystem** | Documentation overhaul (mdBook) | v3.1+ | L |

**Effort**: S = 1-2 weeks, M = 2-4 weeks, L = 4-8 weeks

---

## Success Metrics

| Milestone | Metric | Target |
|-----------|--------|--------|
| v2.5 | IC elimination nets to zero | 100% of generated datasets |
| v2.5 | Golden-path test passes with all modules | Single `cargo test` command |
| v2.5 | Generation at 1M JEs | < 60s, < 4GB peak memory |
| v3.0 | Scenarios definable in YAML | 10+ intervention types |
| v3.0 | Paired generation overhead | < 2.1x baseline time |
| v3.0 | Compliance standards coverage | IFRS, US GAAP, ISA, SOX, PCAOB |
| v3.1 | Auto-tune convergence | Stable by iteration 3-4 |
| v3.1 | NL config → valid YAML rate (real LLM) | > 95% first attempt |
| v3.1 | Neural generation vs statistical fidelity | Neural wins on 70%+ column pairs |

---

## Relationship to Prior Plans

This roadmap consolidates and supersedes the individual plan documents:

| Prior Plan | Disposition |
|------------|-------------|
| `2026-02-19-counterfactual-simulation-roadmap.md` | Absorbed into v3.0.2 |
| `2026-03-02-unified-generation-pipeline-design.md` | Absorbed into v3.0.1 |
| `2026-03-09-compliance-regulations-implementation.md` | Absorbed into v3.0.3 |
| `2026-04-13-ai-capabilities-roadmap.md` | Phase 1 shipped; Phase 2-3 absorbed into v3.1 |
| All `v2.x` release plans | Shipped and closed |
| All codebase quality plans | Shipped and closed |
| Domain expansion / country pack plans | Shipped and closed |

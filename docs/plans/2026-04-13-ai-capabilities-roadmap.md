# AI Capabilities Roadmap

**Date**: 2026-04-13
**Status**: Active
**Scope**: Multi-phase integration of AI/ML capabilities into the DataSynth stack

---

## Executive Summary

DataSynth already has three production-ready AI-adjacent layers: a statistical diffusion backend (~2K lines), LLM enrichment (provider trait + 3 enrichers), and comprehensive ML-readiness evaluation (13 modules). This roadmap extends those foundations with neural generation, LLM-powered configuration, and closed-loop AI tuning — turning DataSynth from a rule-based generator with statistical polish into a hybrid rule+AI engine.

**Design principle**: AI augments the rule engine; it does not replace it. Accounting identities, document chain integrity, Benford compliance, and COSO controls remain rule-enforced. AI handles what rules can't: learning real-world joint distributions, generating natural language, and closing the feedback loop between evaluation and generation.

---

## Current State

| Layer | Location | Lines | Status |
|-------|----------|-------|--------|
| Statistical Diffusion | `datasynth-core/src/diffusion/` | ~2,075 | Production |
| `DiffusionBackend` trait (pluggable) | `diffusion/backend.rs` | 97 | Production |
| `HybridGenerator` (rule+diffusion blend) | `diffusion/hybrid.rs` | 290 | Production |
| `DiffusionTrainer` + `TrainedDiffusionModel` | `diffusion/training.rs` | 747 | Production |
| LLM Provider trait (Mock/OpenAI/Anthropic) | `datasynth-core/src/llm/` | ~500 | Production |
| LLM Enrichers (transaction, vendor, anomaly) | `datasynth-generators/src/llm_enrichment/` | ~600 | Production |
| Natural language config (stub) | `datasynth-core/src/llm/nl_config.rs` | Started | Stub |
| ML-Readiness Evaluation (13 modules) | `datasynth-eval/src/ml/` | ~2,000+ | Production |
| Graph ML Features | `datasynth-graph/src/ml/` | ~500 | Production |
| AutoTuner / Recommendation Engine | `datasynth-eval/src/enhancement/` | ~800 | Beta |
| Baseline Task Definitions | `datasynth-eval/src/ml/baselines.rs` | ~500 | Production |

---

## Roadmap

```
Phase 1                Phase 2                Phase 3
Neural Generation      LLM Intelligence       Advanced AI
+ LLM Config           + Eval Loop            Applications
─────────────────────  ─────────────────────  ─────────────────────
Apr–May 2026           Jun–Jul 2026           Aug–Oct 2026
```

---

## Phase 1 — Neural Generation + LLM Config (Apr–May 2026)

### 1A. Neural Diffusion Backend

**Goal**: Implement `NeuralDiffusionBackend` using `candle` (pure Rust ML framework) that implements the existing `DiffusionBackend` trait.

**Why**: The statistical backend approximates distributions parametrically (mean, std, Cholesky correlations). A neural score network can learn the *actual* joint distribution from data — capturing nonlinear relationships between columns that parametric models miss (e.g., vendor industry × payment terms × invoice amount patterns).

**Architecture**:

```
                    ┌──────────────────────────────┐
                    │      DiffusionBackend trait   │
                    │  forward() / reverse() / gen  │
                    └──────────┬───────────────────┘
                               │
               ┌───────────────┼───────────────────┐
               │               │                   │
    StatisticalBackend   NeuralBackend      (future backends)
    (existing, ~550 LOC)  (candle U-Net)
               │               │
               └───────┬───────┘
                       │
                HybridGenerator
         (blend at any weight/strategy)
```

**Implementation**:

| Component | Description |
|-----------|-------------|
| `neural.rs` | `NeuralDiffusionBackend` struct + `DiffusionBackend` impl |
| `score_network.rs` | Small MLP/U-Net score network for tabular data (candle) |
| `neural_training.rs` | Training loop: denoising score matching objective |
| Config extension | `diffusion.backend: neural` in YAML schema |
| Integration | Wire into `HybridGenerator` — configurable blend weight |

**Key decisions**:
- **Framework**: `candle` (Hugging Face, pure Rust, no Python/C++ deps, GPU optional)
- **Network size**: Small MLP (2-4 hidden layers, 128-256 units) — tabular data doesn't need a giant model
- **Training data**: Column statistics from `DiffusionTrainer` or raw samples from fingerprint extraction
- **Inference**: CPU by default, CUDA feature flag for GPU acceleration
- **Determinism**: Seed-controlled via `candle` tensor RNG for reproducibility

**Config**:

```yaml
diffusion:
  enabled: true
  backend: neural           # statistical (default) | neural | hybrid
  neural:
    hidden_layers: [256, 256, 128]
    activation: silu
    learning_rate: 0.001
    training_epochs: 100
    batch_size: 256
  hybrid:
    weight: 0.6             # 0.0 = pure rule-based, 1.0 = pure neural
    strategy: ensemble       # interpolate | select | ensemble
    neural_columns: [amount, quantity, unit_price, discount_pct]
```

**Estimated effort**: 3-4 weeks

---

### 1B. LLM-Powered Configuration

**Goal**: Natural language → full YAML config generation. Users describe what they want in plain English; the LLM maps to the existing config schema.

**Why**: The DataSynth config schema has ~50 top-level sections and hundreds of parameters. New users face a steep learning curve. NL config collapses this to a conversation.

**Architecture**:

```
User: "Generate 12 months of mid-market manufacturing data
       with a Q3 supply chain disruption and SOX controls"
                          │
                          ▼
              ┌───────────────────────┐
              │  NlConfigGenerator    │
              │  (LlmProvider-backed) │
              │                       │
              │  1. Parse intent      │
              │  2. Select presets    │
              │  3. Apply overrides   │
              │  4. Validate schema   │
              │  5. Return YAML       │
              └───────────────────────┘
                          │
                          ▼
               Full DataSynth YAML config
```

**Implementation**:

| Component | Description |
|-----------|-------------|
| `nl_config.rs` (extend) | `NlConfigGenerator` with schema-aware prompt construction |
| System prompt | Embeds full config schema + presets + validation rules |
| `config_intent.rs` | Structured intermediate representation of user intent |
| CLI integration | `datasynth-data init --from-description "..."` |
| Validation | Generated config passes `datasynth-data validate` before returning |
| Fallback | If LLM fails, offer closest preset match with explanation |

**Key decisions**:
- Schema is injected into the system prompt (structured output guarantees valid YAML)
- Multi-turn refinement: "make it larger", "add fraud", "switch to IFRS"
- Presets as shortcuts: the LLM starts from the closest preset and applies overrides
- Config is always validated before returning — hallucinated field names are caught

**Config**:

```yaml
llm:
  provider: anthropic
  model: claude-sonnet-4-20250514
  nl_config:
    enabled: true
    max_refinement_turns: 5
    include_explanations: true    # annotate generated YAML with comments
```

**Estimated effort**: 2-3 weeks

---

## Phase 2 — LLM Intelligence + Eval Loop (Jun–Jul 2026)

### 2A. Evaluation-Driven AI Tuning Loop

**Goal**: Close the loop between generation and evaluation. Generate → evaluate → LLM interprets gaps → generates config patches → regenerate.

**Why**: The AutoTuner already generates config patches from evaluation results, but its rules are hand-coded. An LLM can interpret arbitrary evaluation failures and propose creative fixes that the rule-based tuner wouldn't consider.

**Architecture**:

```
┌─────────────┐     ┌──────────────┐     ┌───────────────────┐
│  Generate   │────→│   Evaluate   │────→│  LLM Interpreter  │
│  (runtime)  │     │  (13+ mods)  │     │  (gap analysis)   │
└─────────────┘     └──────────────┘     └────────┬──────────┘
       ▲                                          │
       │            ┌──────────────┐              │
       └────────────│ Config Patch │◀─────────────┘
                    │  Generator   │
                    └──────────────┘
```

**Implementation**:

| Component | Description |
|-----------|-------------|
| `ai_tuner.rs` | `AiTuner` that wraps `AutoTuner` + `LlmProvider` |
| Eval summary format | Structured JSON summary of all evaluation results |
| LLM prompt | "Given these evaluation results, which config parameters should change and by how much?" |
| Patch application | Merge LLM-suggested patches with existing config |
| Convergence detection | Stop when eval metrics stabilize or max iterations reached |
| CLI | `datasynth-data generate --auto-tune --max-iterations 5` |

**Key decisions**:
- Max 5 iterations by default (diminishing returns beyond that)
- LLM sees evaluation results + current config + change history
- Each iteration logs the diff so users can audit what changed
- Guardrails: LLM cannot disable structural constraints (balance, chains)

**Estimated effort**: 3-4 weeks

---

### 2B. Contextual Anomaly Design

**Goal**: LLM-designed fraud schemes that are contextually appropriate for the company profile, industry, and control environment.

**Why**: Template-based anomaly injection produces patterns that are always structurally identical. Real fraud adapts to the control environment. An LLM can design novel schemes: "given weak AP controls and high vendor concentration, what fraud patterns would emerge?"

**Architecture**:

```
Company Profile + Control Assessment + Industry
                    │
                    ▼
        ┌───────────────────────┐
        │  AnomalyDesigner LLM │
        │                       │
        │  1. Assess weak spots │
        │  2. Design scheme     │
        │  3. Map to injection  │
        │     parameters        │
        │  4. Generate memos    │
        └───────────────────────┘
                    │
                    ▼
         AnomalyInjectionConfig
         (feeds existing injector)
```

**Implementation**:

| Component | Description |
|-----------|-------------|
| `anomaly_designer.rs` | LLM-powered anomaly scheme generator |
| Context builder | Assembles company profile, control maturity, industry context |
| Scheme-to-config mapper | Maps LLM's scheme description to `AnomalyInjectionConfig` parameters |
| Narrative generator | Pre-generates realistic memos/descriptions for each anomaly instance |
| Library | Cache designed schemes for reuse without LLM calls |

**Estimated effort**: 2-3 weeks

---

## Phase 3 — Advanced AI Applications (Aug–Oct 2026)

### 3A. Tabular Transformer for Conditional Generation

**Goal**: Small transformer model that learns conditional distributions: "given this vendor profile, what does a realistic invoice sequence look like?"

**Why**: Diffusion excels at marginal and joint distributions. Transformers excel at *sequential* and *conditional* patterns — invoice timing sequences, payment behavior over time, escalation patterns. Particularly relevant for the banking module where transaction patterns are highly behavioral.

**Implementation**:

| Component | Description |
|-----------|-------------|
| `tabular_transformer.rs` | Small encoder-only transformer for conditional tabular generation |
| `sequence_model.rs` | Autoregressive model for temporal sequences (invoice timing, payment patterns) |
| Training pipeline | Train from fingerprint data or generated samples |
| Integration | New generator type that composes with existing rule-based generators |

**Estimated effort**: 4-5 weeks

---

### 3B. ONNX Model Integration for Adversarial Testing

**Goal**: Load customer fraud detection models → generate synthetic data that probes decision boundaries.

**Why**: DataSynth becomes a red-team tool. "Here's your model — here are the scenarios it misses." Directly implements Phase 4 of the counterfactual simulation roadmap.

**Implementation**:

| Component | Description |
|-----------|-------------|
| ONNX runtime integration | `ort` crate for model loading and inference |
| `adversarial_generator.rs` | Gradient-guided sample generation near decision boundaries |
| `fairness_tester.rs` | Counterfactual fairness: vary protected attributes, measure prediction changes |
| `robustness_suite.rs` | Distribution shift testing, temporal degradation analysis |
| CLI | `datasynth-data adversarial --model model.onnx --target fraud_detection_auc` |

**Estimated effort**: 4-5 weeks

---

### 3C. GNN-Informed Graph Generation

**Goal**: Train a GNN on entity relationship graphs → use learned structure to condition transaction generation.

**Why**: Vendor collusion rings, circular payment patterns, and layered money laundering structures have characteristic graph signatures. A GNN learns these from data; generation produces structurally realistic relationship networks instead of hand-coded templates.

**Implementation**:

| Component | Description |
|-----------|-------------|
| Graph structure model | GNN (via candle) that learns plausible edge patterns |
| Conditional transaction gen | Given graph structure, generate transactions consistent with relationships |
| Integration with banking | AML typologies emerge from graph model rather than templates |
| Evaluation | Compare generated graph metrics against `datasynth-eval/src/ml/gnn_readiness.rs` targets |

**Estimated effort**: 5-6 weeks

---

## Dependencies and Risk Mitigation

| Risk | Impact | Mitigation |
|------|--------|------------|
| `candle` API instability | Neural backend breaks on updates | Pin version, wrap in thin adapter layer |
| Model size / compile time | Slow CI, large binary | Feature-gate all neural code behind `neural` cargo feature |
| GPU not available | Neural backend unusable | CPU inference by default; GPU is an optional acceleration |
| LLM API costs | Auto-tuning loop burns tokens | Cache aggressively, mock provider for tests, token budget limits |
| LLM hallucinated configs | Invalid YAML generated | Always validate before returning; reject + retry on failure |
| Training data quality | Neural model learns noise | Fingerprint module's differential privacy as guardrail |
| Determinism regression | Neural models less reproducible | Seed-controlled RNG in candle; document tolerance bounds |

---

## Feature Gating

All AI capabilities are additive and feature-gated:

```toml
# Cargo.toml feature flags
[features]
default = []
neural = ["candle-core", "candle-nn"]
neural-cuda = ["neural", "candle-core/cuda"]
llm = ["reqwest"]        # already present
ai-tuning = ["llm"]
adversarial = ["ort"]    # ONNX runtime
```

```yaml
# Config — everything disabled by default
diffusion:
  backend: statistical   # no neural deps unless explicitly chosen
llm:
  provider: mock         # no API calls unless configured
```

---

## Success Metrics

| Phase | Metric | Target |
|-------|--------|--------|
| 1A | Neural vs statistical distribution fidelity (KS test) | Neural wins on 70%+ of column pairs |
| 1A | Generation throughput (neural, CPU) | > 10K rows/sec for 20-column dataset |
| 1B | NL config → valid YAML rate | > 95% first-attempt validity |
| 1B | Config generation latency | < 5 seconds for typical descriptions |
| 2A | Eval score improvement per iteration | > 5% average across metrics |
| 2A | Convergence within N iterations | Stable by iteration 3-4 |
| 2B | Novel anomaly patterns vs templates | > 30% schemes not in existing library |
| 3A | Temporal pattern fidelity | Transformer beats diffusion on sequence metrics |
| 3B | Adversarial sample generation rate | > 1K boundary samples/minute |
| 3C | Graph structural realism | Homophily, clustering within 10% of targets |

---

## Relationship to Counterfactual Roadmap

This AI roadmap complements the [counterfactual simulation roadmap](2026-02-19-counterfactual-simulation-roadmap.md):

- **Phase 1A (Neural Diffusion)** feeds counterfactual Phase 4 (ML training data) — neural generation produces more realistic paired samples
- **Phase 1B (LLM Config)** feeds counterfactual Phase 3 (Interactive Exploration) — natural language scenario description
- **Phase 2A (Eval Loop)** feeds counterfactual Phase 2 (Causal Propagation) — AI-tuned propagation parameters
- **Phase 3B (ONNX/Adversarial)** *is* counterfactual Phase 4's fairness and robustness testing
- **Phase 3C (GNN Generation)** produces more realistic entity graphs for counterfactual graph scenarios

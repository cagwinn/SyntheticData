# AI Capabilities

DataSynth integrates AI at multiple points in the generation pipeline, from data synthesis to configuration to quality assurance. All AI features are optional and gated behind feature flags or config toggles.

## Features

| Capability | Feature Flag | Description |
|------------|-------------|-------------|
| [Neural Diffusion](neural-diffusion.md) | `neural` | Candle-based diffusion models for learning real data distributions |
| [NL Config Generation](../configuration/nl-config.md) | `llm` | Generate YAML configs from natural language descriptions |
| [Auto-Tune Loop](auto-tune.md) | `llm` | Evaluate generated data and iteratively improve config via AI patches |
| [Adversarial Testing](adversarial.md) | `adversarial` | Probe ML model decision boundaries with ONNX Runtime |
| [Anomaly Designer](anomaly-designer.md) | `llm` | LLM-designed fraud schemes tailored to company context |
| LLM Enrichment | `llm` | Contextual narrative generation for audit findings and anomalies |
| Smart Patching | `llm` | AI-driven config patches from evaluation feedback |

## Architecture

The core generation pipeline is fully deterministic and runs without any AI dependencies. AI providers are abstracted behind the `LlmProvider` trait in `datasynth-core`, supporting Anthropic, OpenAI, and OpenRouter backends. A `MockLlmProvider` is used in tests.

```bash
# Build with all AI features
cargo build --release --features "llm,adversarial"
```

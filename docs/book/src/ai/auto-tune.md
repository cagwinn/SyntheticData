# Auto-Tune Loop

The auto-tune feature runs an iterative generate-evaluate-patch cycle to improve data quality without manual config editing.

## CLI Usage

```bash
datasynth-data generate --config config.yaml --output ./output \
  --auto-tune --max-iterations 3
```

## How It Works

Each iteration:

1. **Generate** -- Run the full generation pipeline with the current config
2. **Evaluate** -- Score the output using the evaluation framework (Benford compliance, distribution fit, coherence checks, balance validation)
3. **Diagnose** -- `AiTuner` sends the evaluation results and current config to the LLM
4. **Patch** -- The LLM produces targeted config patches (e.g., adjust distribution weights, increase anomaly rates, fix correlation matrices)
5. **Apply** -- Patches are merged into the config and the cycle repeats

The loop stops when:
- `max_iterations` is reached (default: 3)
- The health score improvement drops below the convergence threshold
- The evaluation passes all quality gates

## Components

| Component | Crate | Purpose |
|-----------|-------|---------|
| `AiTuner` | `datasynth-eval` | Orchestrates the generate/evaluate/patch loop |
| `AutoTuner` | `datasynth-eval` | Rule-based config patching from evaluation gaps |
| `RecommendationEngine` | `datasynth-eval` | Generates human-readable improvement suggestions |
| `LlmProvider` | `datasynth-core` | Sends evaluation context to LLM for smart patches |

## Configuration

```yaml
auto_tune:
  max_iterations: 3
  convergence_threshold: 0.01  # Stop if improvement < 1%
```

## When to Use

Auto-tune is most useful when:
- You have strict statistical requirements (Benford MAD < 0.015, specific distribution fits)
- The initial config is approximate (from NL generation or a generic preset)
- You need to meet quality gate thresholds for ML training data

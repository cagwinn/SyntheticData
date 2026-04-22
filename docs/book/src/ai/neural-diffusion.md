# Neural Diffusion

DataSynth supports neural diffusion models for learning and reproducing the statistical properties of real data, complementing the rule-based generators.

## DiffusionBackend Trait

The `DiffusionBackend` trait (`datasynth-core/src/diffusion/backend.rs`) defines the interface:

```rust
pub trait DiffusionBackend: Send + Sync {
    fn name(&self) -> &str;
    fn forward(&self, x: &[Vec<f64>], t: usize) -> Vec<Vec<f64>>;
    fn reverse(&self, x: &[Vec<f64>], t: usize) -> Vec<Vec<f64>>;
    fn sample(&self, n: usize, dims: usize) -> Vec<Vec<f64>>;
}
```

Two implementations exist:

| Backend | Description |
|---------|-------------|
| **Statistical** | Default. Uses learned mean/variance from training data without neural networks. |
| **Neural** | Candle-based denoising network. Requires the `neural` feature flag. |

## HybridGenerator

`HybridGenerator` blends rule-based and diffusion outputs with a configurable weight:

- `weight = 0.0` -- Pure rule-based (default)
- `weight = 1.0` -- Pure diffusion
- `weight = 0.3` -- 30% diffusion, 70% rule-based (recommended starting point)

## Training Pipeline

1. **Extract** a fingerprint from real data: `datasynth-data fingerprint extract --input data.csv --output fp.dsf`
2. **Synthesize** using neural backend: `datasynth-data fingerprint synthesize --fingerprint fp.dsf --neural --rows 10000`

The neural backend trains a small denoising model on the fingerprint's statistical profile, then generates samples through iterative reverse diffusion.

## Configuration

Neural diffusion is configured through the `diffusion` section or invoked via the fingerprint CLI:

```yaml
diffusion:
  enabled: true
  backend: neural       # statistical (default) or neural
  weight: 0.3           # Blending weight
  timesteps: 100        # Diffusion timesteps
```

## When to Use

- **Statistical backend**: When you need fast, reproducible generation with known distributions
- **Neural backend**: When matching complex multivariate distributions from real data that rule-based generators cannot capture

## v4.4.0 — Orchestrator wiring

As of v4.4.0 the generation pipeline's Phase 15 (diffusion
enhancement) honours the `config.diffusion.backend` string and routes
through the neural backend when requested. Until v4.4.0 the phase was
hardcoded to statistical; the field existed on the schema but was
ignored at runtime.

### Feature flag

The neural path is behind the `neural` Cargo feature (CPU) or
`neural-cuda` (GPU):

```bash
cargo build --release --features neural
cargo build --release --features neural-cuda   # + CUDA toolkit
```

Default builds that encounter `backend: neural` log a warn and fall
back to statistical — no hard failure, so configs copied between
build variants just degrade gracefully.

### Checkpoint reuse

Training from scratch on every generation run adds ~5-15s on CPU.
`config.diffusion.neural.checkpoint_path` lets you point at a
directory containing a previously-saved model (`config.json` +
`weights.safetensors` from `NeuralDiffusionBackend::save`) so the
orchestrator skips training:

```yaml
diffusion:
  enabled: true
  backend: neural
  neural:
    checkpoint_path: ./models/neural-retail-v1
```

Train once (via the fingerprint CLI or a custom driver), save, then
reuse across runs for stable production pipelines.

### Dispatch matrix

| Backend value  | Feature on? | Path taken                                         |
|----------------|-------------|----------------------------------------------------|
| `statistical`  | n/a         | moment-matching (always fast; default)             |
| `neural`       | yes         | candle score network + denoising score matching    |
| `neural`       | no          | falls back to statistical, logs a warn             |
| `hybrid`       | yes         | neural (hybrid weighting arrives post-v4.4)        |
| `hybrid`       | no          | falls back to statistical, logs a warn             |
| anything else  | n/a         | falls back to statistical, logs a warn             |

### Tracked follow-ups

- Neural samples are still generated standalone, not fed back into
  downstream generators (fraud / anomaly / subledger). Wiring the
  neural output into the entry-enhancement path is tracked for
  post-v4.4.
- Feature extraction is limited to `(total_amount, line_count,
  approval_level)`. Richer features (per-account amount buckets,
  temporal, fraud flags) — also post-v4.4.
- True `hybrid` blending (weighted mix of statistical + neural per
  the `hybrid_weight` config) — currently the hybrid path runs the
  neural backend only; full blending post-v4.4.

### See also

- `crates/datasynth-runtime/src/enhanced_orchestrator.rs` — Phase 15 dispatch
- `crates/datasynth-runtime/tests/neural_diffusion_wiring.rs` — smoke tests (ignored by default; `cargo test -- --ignored`)
- `crates/datasynth-core/src/diffusion/neural.rs` — `NeuralDiffusionBackend`
- `crates/datasynth-core/src/diffusion/neural_training.rs` — `NeuralDiffusionTrainer`

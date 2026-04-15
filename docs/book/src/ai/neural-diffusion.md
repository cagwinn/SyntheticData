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

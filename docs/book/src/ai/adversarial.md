# Adversarial Testing

DataSynth can probe ML model decision boundaries to generate targeted synthetic data near those boundaries -- useful for stress-testing fraud detection and anomaly classifiers.

> Requires the `adversarial` feature: `cargo build --release --features adversarial`

## CLI Usage

```bash
datasynth-data adversarial \
  --model fraud_detector.onnx \
  --features 10 \
  --probes 1000 \
  --threshold 0.5 \
  --perturbation 0.05 \
  --output probes.json \
  --seed 42
```

| Flag | Description |
|------|-------------|
| `--model` | Path to ONNX model file |
| `--features` | Number of input features the model expects |
| `--probes` | Number of probe samples to generate (default: 1000) |
| `--threshold` | Decision threshold for classification (default: 0.5) |
| `--perturbation` | Perturbation budget 0.0--1.0 (default: 0.05) |
| `--output` | Output JSON file for results |
| `--seed` | Random seed (default: 42) |

## How It Works

`ModelProbe` (`datasynth-eval/src/adversarial/model_probe.rs`) uses ONNX Runtime to:

1. **Generate seed samples** across the feature space
2. **Evaluate** each sample through the model
3. **Identify boundary points** where the model's confidence is near the threshold
4. **Perturb** boundary samples within the budget to find minimal perturbations that flip the classification
5. **Report** the boundary topology, including which feature dimensions are most sensitive

## Output Format

The probe report (JSON) includes:
- Boundary point coordinates
- Model confidence at each point
- Minimal perturbation vectors that cross the decision boundary
- Feature sensitivity rankings

## Use Cases

- Stress-test fraud detection models before deployment
- Find blind spots in anomaly classifiers
- Generate adversarial training data to improve model robustness
- Validate that model decisions are stable near realistic data distributions

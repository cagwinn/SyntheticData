# Fingerprinting

The fingerprint pipeline extracts statistical profiles from real data, stores them in a privacy-preserving `.dsf` format, and uses them to generate synthetic data that matches the original distribution.

## Pipeline

```
Real Data (CSV) → Extract → .dsf file → Synthesize → Synthetic Data
                                      → Evaluate → Fidelity Report
```

## 1. Extract

```bash
datasynth-data fingerprint extract \
  --input ./real_data.csv \
  --output ./fingerprint.dsf \
  --privacy-level standard
```

Privacy levels:

| Level | Epsilon | k-Anonymity | Description |
|-------|---------|-------------|-------------|
| `minimal` | High | Low | Fastest, least privacy |
| `standard` | Balanced | 5 | Recommended default |
| `high` | Low | 10 | Suitable for sensitive data |
| `maximum` | Very low | 20+ | Maximum privacy, some fidelity loss |

Custom privacy parameters:

```bash
--privacy-epsilon 1.0 --privacy-k 10
```

The `--sign` flag adds a cryptographic signature for integrity verification.

## 2. Validate

```bash
datasynth-data fingerprint validate fingerprint.dsf
```

Checks file integrity, format version, and optional signature.

## 3. Info & Diff

```bash
datasynth-data fingerprint info fingerprint.dsf --detailed
datasynth-data fingerprint diff file1.dsf file2.dsf
```

## 4. Synthesize

Generate synthetic data from an extracted fingerprint:

```bash
datasynth-data fingerprint synthesize \
  --fingerprint fingerprint.dsf \
  --output ./synthetic \
  --rows 10000 \
  --seed 42
```

Add `--neural` to use the neural diffusion backend (requires `neural` feature).

## 5. Evaluate

Measure how well synthetic data matches the fingerprint:

```bash
datasynth-data fingerprint evaluate \
  --fingerprint fingerprint.dsf \
  --synthetic ./synthetic \
  --threshold 0.8
```

The fidelity evaluator checks:
- Column-level distribution similarity
- Correlation structure preservation
- Missing value pattern fidelity
- Categorical frequency matching

## Fingerprint-Based Generation

You can also use a fingerprint to guide the full generation pipeline:

```bash
datasynth-data generate \
  --fingerprint fingerprint.dsf \
  --scale 2.0 \
  --output ./output
```

The `--scale` factor multiplies the record counts from the fingerprint.

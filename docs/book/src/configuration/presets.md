# Industry Presets

DataSynth includes pre-built configurations for common industries. Use `datasynth-data info` to list all available presets.

## Industry Profiles

| Industry | Key Characteristics |
|----------|-------------------|
| `manufacturing` | Production orders, BOM, quality inspections, multi-tier supply chain |
| `retail` | High transaction volume, seasonal patterns, customer segmentation |
| `financial_services` | Complex intercompany, treasury, FX, hedging instruments |
| `healthcare` | Compliance-heavy, department structure, expense controls |
| `technology` | Revenue recognition (ASC 606), stock comp, project accounting |

## Complexity Levels

Each industry preset supports three scale levels:

| Level | GL Accounts | Typical Use |
|-------|-------------|-------------|
| `small` | ~100 | Quick testing, CI pipelines, demos |
| `medium` | ~400 | Development, analytics prototyping |
| `large` | ~2,500 | Production ML training, full audit simulation |

## Using Presets

### Via CLI Init

```bash
datasynth-data init --industry manufacturing --complexity medium -o config.yaml
```

Generates a complete YAML config you can customize before running.

### Via Preset Overlay

```bash
datasynth-data generate --demo --preset audit-group --output ./output
```

The `audit-group` overlay enables ISA/PCAOB/SOX, COSO controls, anomaly injection, and network features on top of any base config.

### Via Python SDK

```python
from datasynth_py import blueprints

config = blueprints.manufacturing_large()
config = blueprints.retail_small()
config = blueprints.banking_medium()
config = blueprints.ml_training()
config = blueprints.statistical_validation()
```

## Transaction Volume

Presets also control transaction volume, which determines the number of journal entries per company per month. The demo preset uses minimal volume for fast iteration.

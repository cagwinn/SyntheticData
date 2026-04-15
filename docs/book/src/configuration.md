# Configuration

DataSynth is configured through a single YAML file that controls every aspect of generation. Each top-level section maps to a specific generator family or infrastructure feature.

## Sections

- [YAML Reference](configuration/yaml-reference.md) -- All top-level config sections
- [Industry Presets](configuration/presets.md) -- Pre-built configurations by industry and scale
- [NL Config Generation](configuration/nl-config.md) -- Generate configs from natural language descriptions

## Basic Structure

```yaml
global:
  industry: manufacturing
  start_date: "2024-01-01"
  period_months: 12
  seed: 42

companies:
  - code: C001
    name: "Acme Manufacturing"
    currency: USD
    country: US

chart_of_accounts:
  complexity: medium    # small (~100), medium (~400), large (~2,500)

output:
  formats: [csv, json]  # csv, json, parquet
  compression_level: 0
```

## Validation Rules

- `period_months`: 1--120
- `compression_level`: 1--9 (0 = disabled)
- Rates/percentages: 0.0--1.0
- Approval thresholds: must be in ascending order
- Distribution weights: must sum to 1.0 (within 0.01 tolerance)

Run validation before generating:

```bash
datasynth-data validate --config config.yaml
```

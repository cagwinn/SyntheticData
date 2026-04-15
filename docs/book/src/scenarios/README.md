# Scenario Engine

The scenario engine generates paired baseline and counterfactual datasets for causal analysis, ML training, and what-if simulation.

## Sections

- [Counterfactual Simulation](counterfactual.md) -- How paired generation works with causal DAGs
- [Scenario Library](library.md) -- Pre-built scenario definitions

## Key Concepts

A **scenario** defines one or more **interventions** applied to a base configuration. The engine generates two datasets from the same seed:

1. **Baseline** -- Generated with the original config
2. **Counterfactual** -- Generated with interventions applied (e.g., recession, fraud injection, control failure)

Because both use the same RNG seed and causal DAG, differences between the datasets are attributable solely to the interventions.

## Quick Example

```yaml
scenarios:
  enabled: true
  scenarios:
    - name: recession_impact
      description: "Simulate a 6-month recession starting in month 4"
      interventions:
        - type: macro_shock
          parameters:
            revenue_multiplier: 0.7
            expense_multiplier: 1.1
          timing:
            start_month: 4
            duration_months: 6
            onset: gradual
            ramp_months: 2
```

```bash
datasynth-data scenario generate --config config.yaml --output ./output
```

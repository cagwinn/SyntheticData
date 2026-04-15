# DataSynth Scenario Pack Library

Pre-built scenario definitions for counterfactual simulation. Each YAML file defines interventions that modify the baseline generation to produce paired baseline/counterfactual datasets.

## Usage

```bash
# Copy a scenario into your config
cat scenarios/fraud/vendor_collusion_ring.yaml >> config.yaml

# List available scenarios
datasynth-data scenario list --config config.yaml

# Validate before generating
datasynth-data scenario validate --config config.yaml

# Generate paired baseline + counterfactual datasets
datasynth-data scenario generate --config config.yaml --output ./output

# Diff the results
datasynth-data scenario diff --baseline ./output/baseline --counterfactual ./output/scenarios/vendor_collusion_ring/data
```

## Scenarios

### Fraud (5)
| Scenario | Description |
|----------|-------------|
| [vendor_collusion_ring](fraud/vendor_collusion_ring.yaml) | Coordinated vendor bids with kickbacks via ghost invoices |
| [management_override](fraud/management_override.yaml) | Quarter-end revenue acceleration with next-quarter reversals |
| [ghost_employee](fraud/ghost_employee.yaml) | Fictitious employees on payroll, escalating over 6 months |
| [procurement_kickback](fraud/procurement_kickback.yaml) | Inflated contracts from testing phase to escalation |
| [channel_stuffing](fraud/channel_stuffing.yaml) | Pushing excess inventory to distributors with side return agreements |

### Control Failures (2)
| Scenario | Description |
|----------|-------------|
| [sox_material_weakness](control_failures/sox_material_weakness.yaml) | ERP migration degrades three-way match + SoD violations |
| [it_control_breakdown](control_failures/it_control_breakdown.yaml) | ITGC failure enables ghost vendor payments via DB manipulation |

### Macro Shocks (3)
| Scenario | Description |
|----------|-------------|
| [recession](macro/recession.yaml) | GDP contraction, revenue decline, vendor defaults, cash pressure |
| [supply_chain_disruption](macro/supply_chain_disruption.yaml) | Strategic vendor defaults force spot purchasing at premium |
| [interest_rate_shock](macro/interest_rate_shock.yaml) | +300bp rate hike impacting debt service and hedge effectiveness |

### Operational (1)
| Scenario | Description |
|----------|-------------|
| [erp_migration](operational/erp_migration.yaml) | Parallel running causes duplicate entries and control gaps |

## Creating Custom Scenarios

See the [counterfactual simulation roadmap](../docs/plans/2026-02-19-counterfactual-simulation-roadmap.md) for the full intervention type library and scenario schema reference.

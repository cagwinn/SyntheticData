# Scenario Library

DataSynth ships with 11 pre-built scenario definitions spanning fraud, control failure, macroeconomic stress, and operational disruption. Scenarios can be loaded via `--scenario-pack`, `--fraud-scenario`, or composed directly in config YAML.

## Fraud Scenarios (5)

| Scenario | Description |
|----------|-------------|
| `vendor_collusion` | Vendor kickbacks with split invoices below approval thresholds |
| `management_override` | Override of controls by senior management, fictitious entries |
| `ghost_employee` | Ghost employees on payroll with duplicate payment schemes |
| `procurement_kickback` | Procurement staff colluding with preferred vendors |
| `channel_stuffing` | Premature revenue recognition through end-of-period loading |

Apply with `--fraud-scenario` (repeatable):

```bash
datasynth-data generate --config config.yaml \
  --fraud-scenario vendor_collusion \
  --fraud-scenario ghost_employee \
  --fraud-rate 0.05
```

## Control Failure Scenarios (2)

| Scenario | Description |
|----------|-------------|
| `sox_material_weakness` | SOX material weakness in financial reporting controls |
| `it_control_breakdown` | ITGC deficiencies affecting automated controls and access management |

## Macroeconomic Scenarios (3)

| Scenario | Description |
|----------|-------------|
| `recession` | Revenue decline with expense pressure and cash flow squeeze |
| `supply_chain_disruption` | Supplier failures cascading through P2P with inventory impact |
| `interest_rate_shock` | Rapid rate changes affecting treasury, debt covenants, and hedging |

## Operational Scenarios (1)

| Scenario | Description |
|----------|-------------|
| `erp_migration` | ERP system migration with data quality issues, dual-posting periods, and reconciliation gaps |

## Loading Scenario Packs

Packs are YAML files in `templates/scenarios/`:

```bash
datasynth-data generate --scenario-pack manufacturing/supplier_fraud
```

The CLI searches for packs in:
1. `templates/scenarios/<name>.yaml` (relative to working directory)
2. `templates/scenarios/<name>.yaml` (relative to binary location)

## Composing in Config

Combine multiple interventions in a single scenario:

```yaml
scenarios:
  enabled: true
  scenarios:
    - name: combined_stress
      description: "Recession with vendor fraud"
      interventions:
        - type: macro_shock
          parameters:
            revenue_multiplier: 0.75
          timing: { start_month: 3, duration_months: 9, onset: gradual }
        - type: fraud_injection
          parameters:
            fraud_rate: 0.08
            schemes: [vendor_collusion, procurement_kickback]
          timing: { start_month: 6, onset: sudden }
```

See [Counterfactual Simulation](counterfactual.md) for details on how paired generation works.

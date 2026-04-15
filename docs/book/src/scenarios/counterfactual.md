# Counterfactual Simulation

## ScenarioEngine

`ScenarioEngine` (`datasynth-runtime/src/scenario_engine.rs`) manages paired dataset generation:

```rust
let engine = ScenarioEngine::new(config)?;
let scenarios = engine.list_scenarios();
let validation = engine.validate_all();
let result = engine.generate("recession_impact", &output_dir)?;
```

## Causal DAG

Each scenario operates on a causal directed acyclic graph that defines how interventions propagate through the data model. For example:

```
revenue_multiplier → transaction_amounts → journal_entries
                   → document_flow_volumes → subledger_balances
                   → trial_balance → financial_statements
```

The DAG ensures that when you intervene on revenue, all downstream tables reflect the change consistently.

## Interventions

An intervention modifies a specific variable in the causal model:

```yaml
interventions:
  - type: macro_shock
    parameters:
      revenue_multiplier: 0.7
    timing:
      start_month: 4
      duration_months: 6
      onset: gradual        # sudden, gradual, oscillating
      ramp_months: 2
    label: "Revenue decline"
    priority: 1             # Higher priority wins conflicts
```

### Timing Options

| Field | Description |
|-------|-------------|
| `start_month` | Month offset (1-indexed) when intervention begins |
| `duration_months` | How long the intervention lasts |
| `onset` | `sudden` (step function), `gradual` (linear ramp), `oscillating` |
| `ramp_months` | Transition period for gradual onset |

## Constraints

Scenarios can enforce invariants during counterfactual generation:

```yaml
constraints:
  preserve_accounting_identity: true   # Assets = Liabilities + Equity
  preserve_referential_integrity: true
```

## Paired Output

The engine produces two directory trees:

```
output/
  baseline/        # Original generation
  counterfactual/  # With interventions applied
```

Use `datasynth-data scenario diff` to compare them:

```bash
datasynth-data scenario diff \
  --baseline output/baseline \
  --counterfactual output/counterfactual \
  --format summary
```

Diff formats: `summary`, `record_level`, `aggregate`, `all`.

# Demo Mode

## Default Demo

```bash
datasynth-data generate --demo --output ./output
```

The demo preset creates a manufacturing company with:
- 1 company entity
- 3-month period
- Small chart of accounts (~100 GL accounts)
- Core generators: JEs, master data, document flows, subledgers, period close

Output lands in `./output/` with JSON files organized by domain directory.

## Preset Overlays

Apply a named overlay on top of any config (or the demo default):

```bash
datasynth-data generate --demo --preset audit-group --output ./output
```

### `audit-group` Preset

Enables all audit simulation features in one flag:

- Audit standards: ISA, PCAOB, SOX compliance
- Internal controls with COSO 2013 framework (entity-level + transaction-level)
- Accounting standards: revenue recognition, leases, fair value, impairment
- Anomaly injection (5% fraud + 3% error rate)
- Vendor network and customer segmentation
- Cross-process relationship links
- Scenario tags: `audit_group`, `audit_simulation`

This is the recommended starting point for audit analytics and fraud detection ML training.

## Scenario Packs

Load a full scenario configuration from a YAML template:

```bash
datasynth-data generate --scenario-pack manufacturing/supplier_fraud --output ./output
```

Scenario packs are searched in `templates/scenarios/` relative to the binary or working directory.

## Combining Flags

Flags compose additively:

```bash
datasynth-data generate --demo \
  --preset audit-group \
  --banking \
  --graph-export \
  --seed 42 \
  --output ./output
```

This produces a deterministic dataset with audit, banking, and graph exports enabled.

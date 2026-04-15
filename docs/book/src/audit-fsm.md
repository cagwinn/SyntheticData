# Audit FSM Engine

DataSynth includes a YAML-driven finite state machine engine for simulating audit engagements. Each engagement follows a methodology blueprint that defines phases, steps, transitions, and decision points.

## Built-in Blueprints

| Blueprint | Source | Description |
|-----------|--------|-------------|
| `builtin:fsa` | `generic_fsa.yaml` | Generic financial statement audit (ISA) |
| `builtin:ia` | `generic_ia.yaml` | Internal audit methodology |
| `builtin:kpmg` | `kpmg_isa_complete.yaml` | KPMG ISA-based approach |
| `builtin:pwc` | `pwc_isa_complete.yaml` | PwC ISA-based approach |
| `builtin:deloitte` | `deloitte_isa_complete.yaml` | Deloitte ISA-based approach |
| `builtin:ey_gam_lite` | `ey_gam_lite.yaml` | EY Global Audit Methodology (lite) |
| `builtin:soc2` | `soc2_type2.yaml` | SOC 2 Type II examination |
| `builtin:pcaob` | `pcaob_integrated.yaml` | PCAOB integrated audit |
| `builtin:regulatory` | `regulatory_exam.yaml` | Regulatory examination |

Custom blueprints can be loaded from any YAML file path.

## CLI Commands

### Validate a Blueprint

```bash
datasynth-data audit validate --blueprint builtin:fsa
```

### Display Blueprint Info

```bash
datasynth-data audit info --blueprint builtin:kpmg
```

### Run an Engagement

```bash
datasynth-data audit run \
  --blueprint builtin:fsa \
  --overlay builtin:default \
  --output ./audit_output \
  --seed 42
```

Overlay presets: `builtin:default`, `builtin:thorough`, `builtin:rushed`.

### Compare Blueprints

```bash
datasynth-data audit diff --blueprint-a builtin:kpmg --blueprint-b builtin:pwc
```

### Generate Benchmark Data

```bash
datasynth-data audit benchmark \
  --complexity complex \
  --anomaly-rate 0.1 \
  --output ./audit_benchmark
```

Complexity levels: `simple`, `medium`, `complex`.

## Blueprint Structure

A blueprint YAML defines:
- **Phases** -- Major engagement stages (planning, fieldwork, reporting)
- **Steps** -- Individual procedures within phases
- **Transitions** -- Conditions for moving between steps/phases
- **Decision points** -- Branching logic based on risk assessment or findings
- **Overlays** -- Adjustments for thoroughness, time pressure, or risk profile

## Integration with Generation

When `--audit` is passed to `generate`, the FSM engine runs an engagement in parallel with data generation. The audit trail references the same entities, transactions, and documents produced by the core generators.

The `--preset audit-group` overlay enables the FSM along with all supporting features (controls, anomalies, standards).

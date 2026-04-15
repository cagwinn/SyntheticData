# .dss Portable Format

The `.dss` format is DataSynth's portable scenario and session file format, used for sharing scenarios between teams and for incremental generation.

## Scenario Export

Export a scenario definition from a config:

```bash
datasynth-data scenario export \
  --config config.yaml \
  --scenario recession_impact \
  --output recession.dss
```

The `.dss` file contains the scenario definition, interventions, causal model, and constraints in a self-contained format.

## Scenario Import

Import a `.dss` file into an existing or new config:

```bash
datasynth-data scenario import recession.dss --config config.yaml
```

The scenario is merged into the config's `scenarios` section. If the config does not exist, a new one is created.

## Session Files

When you run `datasynth-data generate`, a `session.dss` file is written to the output directory. This file captures:

- Generation state (RNG position, entity registries, balance carry-forward)
- Config snapshot at time of generation
- Output file manifest with checksums

The session file is required for `--append` incremental generation:

```bash
# Initial generation: 12 months
datasynth-data generate --config config.yaml --output ./output

# Append 3 more months
datasynth-data generate --config config.yaml --output ./output --append --months 3
```

The `--append` flag reads `session.dss` from the output directory, continues from the saved state, and writes an updated session file.

## File Structure

The `.dss` file is a compressed (zstd) binary envelope containing:
- Header with format version and checksums
- Serialized scenario or session data
- Optional metadata (tags, authorship, timestamps)

Files can be validated without loading:

```bash
datasynth-data fingerprint validate session.dss
```

# Quick Start

## 1. Generate Demo Data

The fastest way to see DataSynth in action:

```bash
datasynth-data generate --demo --output ./output
```

This generates a small manufacturing dataset (1 company, 3 months, ~100 GL accounts) in seconds.

## 2. Initialize a Custom Config

Create a config file for your industry and scale:

```bash
datasynth-data init --industry retail --complexity medium -o config.yaml
```

Available industries: `manufacturing`, `retail`, `financial_services`, `healthcare`, `technology`.
Complexity levels: `small` (~100 accounts), `medium` (~400), `large` (~2,500).

## 3. Validate Before Generating

```bash
datasynth-data validate --config config.yaml
```

Checks all config sections for invalid values (rate ranges, distribution sums, ascending thresholds).

## 4. Generate

```bash
datasynth-data generate --config config.yaml --output ./output
```

Key flags:

| Flag | Purpose |
|------|---------|
| `--seed 42` | Deterministic output (ChaCha8 RNG) |
| `--banking` | Enable KYC/AML transaction generation |
| `--audit` | Enable audit engagement generation |
| `--graph-export` | Export PyTorch Geometric graph files |
| `--memory-limit 2048` | Set memory ceiling in MB |
| `--quality-gate strict` | Fail on quality violations |

## 5. Generate from Natural Language (requires `llm` feature)

```bash
datasynth-data init --from-description "A mid-size German manufacturer with \
  3 subsidiaries, 12 months of data, intercompany transfers, and 5% anomaly rate"
```

This calls the configured LLM to produce a YAML config. See [NL Config Generation](../configuration/nl-config.md) for provider setup.

## 6. Incremental Generation

Append new periods to an existing dataset:

```bash
datasynth-data generate --config config.yaml --output ./output --append --months 3
```

Carries forward balances and entity state from the previous `session.dss` file.

# datasynth-cli

Command-line interface for synthetic accounting data generation.

## Overview

`datasynth-cli` provides the `datasynth-data` binary for command-line usage:

- **generate**: Generate synthetic data from configuration
- **init**: Create configuration files with industry presets (supports `--from-description` natural-language mode via `llm` feature)
- **validate**: Validate configuration files
- **info**: Display available presets and options
- **scenario**: List / validate / export / generate counterfactual scenarios
- **fingerprint**: Extract / validate / synthesize via the privacy-preserving fingerprint format
- **templates**: Export starter template packs, validate, or **enrich** (v3.5.0+) name pools via mock or live LLM (OpenAI-compatible HTTP; OpenRouter-friendly defaults)
- **audit**: Audit FSM operations (blueprint diff, validate)

## Installation

```bash
cargo build --release
# Binary at: target/release/datasynth-data
```

## Commands

### Generate Data

```bash
# From configuration file
datasynth-data generate --config config.yaml --output ./output

# Demo mode with defaults
datasynth-data generate --demo --output ./demo-output

# With verbose logging
datasynth-data generate --config config.yaml --output ./output -v
```

### Create Configuration

```bash
# Industry preset with complexity level
datasynth-data init --industry manufacturing --complexity medium -o config.yaml

# Available industries:
#   manufacturing, retail, financial_services, healthcare,
#   technology, energy, telecom, transportation, hospitality
```

### Validate Configuration

```bash
datasynth-data validate --config config.yaml
```

### Show Options

```bash
datasynth-data info
```

### Template Enrichment (v3.5.0+, `llm` feature)

Offline-deterministic enrichment of vendor / customer / material name pools.
Runs **outside** the generate pipeline — the enriched YAML is then consumed at
`generate` time via `--templates <path>`.

```bash
# Mock backend: deterministic, seed-driven, no network.
datasynth-data templates enrich \
  --input ./in.yaml --output ./enriched.yaml \
  --category vendor_name --industry retail --region DE \
  --sub-category office_supplies --count 50 \
  --backend mock --seed 42

# Live HTTP backend (default targets OpenRouter so any vendor works with one key).
OPENROUTER_API_KEY=sk-or-... \
datasynth-data templates enrich \
  --input ./in.yaml --output ./enriched.yaml \
  --category customer_name --industry retail --region DE \
  --sub-category enterprise --count 50 \
  --backend http \
  --model anthropic/claude-sonnet-4.5 \
  --api-key-env OPENROUTER_API_KEY \
  --base-url https://openrouter.ai/api

# Then generate with the enriched templates:
datasynth-data generate --config config.yaml --templates ./enriched.yaml --output ./output
```

`--category` accepts `vendor_name`, `customer_name`, or `material_desc`.
`--backend http` requires building with `--features llm`.

### Scenarios

```bash
# Counterfactual scenario generation (baseline + intervention pairs)
datasynth-data scenario list --config config.yaml
datasynth-data scenario generate --config config.yaml --output ./output
datasynth-data scenario export --config config.yaml --scenario supply_chain_disruption -o scenario.dss
```

## Signal Handling (Unix)

Toggle pause during generation:

```bash
kill -USR1 $(pgrep datasynth-data)
```

## Exit Codes

| Code | Description |
|------|-------------|
| 0 | Success |
| 1 | Configuration error |
| 2 | Generation error |
| 3 | I/O error |

## License

Apache-2.0 - See [LICENSE](../../LICENSE) for details.

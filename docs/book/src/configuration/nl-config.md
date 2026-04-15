# NL Config Generation

Generate YAML configurations from natural language descriptions using LLM providers.

> Requires the `llm` feature: `cargo build --release --features llm`

## Usage

```bash
datasynth-data init --from-description "A mid-size French retailer with \
  2 subsidiaries, IFRS reporting, 18 months of data, and moderate anomaly injection"
```

The LLM generates a complete, schema-aware YAML config and writes it to `datasynth_config.yaml` (or the path specified with `-o`).

## LLM Provider Configuration

DataSynth auto-detects the provider from environment variables. It checks in order:

1. `ANTHROPIC_API_KEY` -- Uses Anthropic (Claude)
2. `OPENROUTER_API_KEY` -- Uses OpenRouter (auto-detects key prefix and sets base URL)
3. `OPENAI_API_KEY` -- Uses OpenAI

### Setup

```bash
# Anthropic (recommended)
export ANTHROPIC_API_KEY=sk-ant-...

# OpenAI
export OPENAI_API_KEY=sk-...

# OpenRouter
export OPENROUTER_API_KEY=sk-or-...
```

## How It Works

The `NlConfigGenerator` in `datasynth-core` performs schema-aware prompting:

1. Sends the description along with the full config schema to the LLM
2. The LLM produces valid YAML matching `GeneratorConfig`
3. DataSynth validates the generated config before writing
4. Falls back to a rule-based parser if the LLM call fails

This means the generated config will always pass `datasynth-data validate`.

## Tips

- Be specific about industry, company count, country, currency, and time period
- Mention features you want enabled (e.g., "with banking", "ISA audit", "anomaly injection at 5%")
- The LLM respects accounting framework names: `us_gaap`, `ifrs`, `french_gaap`, `german_gaap`, `dual_reporting`

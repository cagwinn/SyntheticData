# LLM Config Generation

The `NlConfigGenerator` converts natural language descriptions into valid DataSynth YAML configurations.

## CLI Usage

```bash
datasynth-data init --from-description "A large German manufacturer with \
  5 subsidiaries across EU, IFRS reporting, 24 months, intercompany transfers, \
  treasury with hedging, and 3% anomaly injection rate"
```

Output: a validated YAML config written to `datasynth_config.yaml` (or `-o <path>`).

## How It Works

1. **Schema injection** -- `NlConfigGenerator` sends the full `GeneratorConfig` schema to the LLM alongside the user description
2. **Structured generation** -- The LLM produces YAML that conforms to the schema, mapping natural language concepts to config fields
3. **Validation** -- The generated YAML is deserialized and validated before writing
4. **Fallback** -- If the LLM call fails (network error, invalid output), the generator falls back to rule-based parsing that maps keywords to preset configurations

## Provider Auto-Detection

The CLI checks environment variables in order:

| Variable | Provider |
|----------|----------|
| `ANTHROPIC_API_KEY` | Anthropic (Claude) |
| `OPENROUTER_API_KEY` | OpenRouter |
| `OPENAI_API_KEY` | OpenAI |

The first key found is used. OpenRouter keys (`sk-or-*`) are auto-detected and the correct base URL is set.

## `generate_full` Method

For programmatic use:

```rust
use datasynth_core::llm::{NlConfigGenerator, LlmProvider};

let provider: Box<dyn LlmProvider> = /* ... */;
let yaml = NlConfigGenerator::generate_full(
    &*provider,
    "Retail company, 12 months, small CoA, with banking"
)?;
```

Returns a complete YAML string ready for `serde_yaml::from_str::<GeneratorConfig>()`.

## Tips

- Include specific numbers: "3 companies", "18 months", "5% fraud rate"
- Name accounting frameworks explicitly: "IFRS", "US GAAP", "dual reporting"
- Mention optional modules: "with banking", "audit enabled", "graph export"

# Anomaly Designer

The `AnomalyDesigner` uses an LLM to create contextually realistic fraud schemes tailored to a specific company's industry, size, and control environment.

## Architecture

Located in `datasynth-generators/src/llm_enrichment/anomaly_designer.rs`.

### Key Types

**`CompanyContext`** -- Describes the target company:
- `industry` (e.g., "manufacturing", "retail")
- `company_size` ("small", "medium", "large")
- `country` (e.g., "US", "DE")
- `employee_count`, `annual_revenue`

**`ControlContext`** -- Describes the control environment:
- `maturity_level` ("ad_hoc", "repeatable", "defined", "managed", "optimized")
- `weak_controls` (e.g., `["three_way_match", "C003"]`)
- `sod_gaps` (e.g., `["AP clerk also approves payments"]`)
- `audit_active`, `itgc_gaps`

**`AnomalyDesigner`** -- Sends context to an LLM and parses the response into `DesignedScheme` objects.

**`SchemeLibrary`** -- Caches designed schemes by industry and maturity level. Supports save/load to JSON for reuse across runs.

## How It Works

1. `AnomalyDesigner::design()` builds a prompt containing the company and control context
2. The LLM generates fraud schemes that exploit the specific weaknesses described
3. Each scheme includes: narrative, exploited weaknesses, detection signals, stages, difficulty, impact range
4. Schemes are cached in a `SchemeLibrary` so identical contexts reuse previous designs

## Output: DesignedScheme

Each designed scheme contains:
- **name** -- Short identifier (e.g., `vendor_kickback_scheme`)
- **narrative** -- Human-readable description of how the fraud works
- **exploited_weaknesses** -- Which controls are circumvented
- **detection_signals** -- Red flags an auditor should look for
- **stages** -- Multi-step scheme progression
- **difficulty** -- Estimated sophistication level
- **impact_range** -- Dollar range of potential losses

## Integration

Designed schemes feed into the anomaly injection pipeline, where they are translated into concrete journal entries and document flow anomalies during generation.

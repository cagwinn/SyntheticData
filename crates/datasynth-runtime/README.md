# datasynth-runtime

Runtime orchestration, parallel execution, and memory management.

## Overview

`datasynth-runtime` provides the execution layer for DataSynth:

- **`EnhancedOrchestrator`**: The primary orchestrator — ~30 phases,
  full enterprise feature integration, multi-threaded, streaming-capable.
- **`StreamingOrchestrator`**: Real-time per-phase streaming variant.
- Parallel execution via Rayon, memory / CPU / disk guards,
  progress reporting, pause/resume.

> **v4.0 note:** the legacy `GenerationOrchestrator` (basic 2-phase
> CoA + JE) was removed in v4.0.0 after a v3.x deprecation window.
> All production call paths already routed through
> `EnhancedOrchestrator`. Migration:
> ```rust
> use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
> let phases = PhaseConfig::from_config(&config);
> let mut orch = EnhancedOrchestrator::new(config, phases)?;
> let result = orch.generate()?;
> ```

## Key components

| Component | Description |
|-----------|-------------|
| `EnhancedOrchestrator` | Full workflow coordinator; `generate()` returns `EnhancedGenerationResult` |
| `StreamingOrchestrator` | Phase-by-phase streaming variant |
| `PhaseConfig` | Per-phase enable flags, auto-derived from `GeneratorConfig` |
| `EnhancedGenerationResult` | Complete snapshot of generated data + statistics + reports |

## Generation pipeline (v4.0.1)

Roughly ~30 phases in sequence. Key ones in execution order:

1. **Chart of accounts** — industry-specific structure
2. **Master data** — vendors, customers, materials, fixed assets, employees; LLM-enrichable via `phase_llm_enrichment`
3. **Document flows** — P2P and O2C chains, business-day snapped (v3.4.1+)
4. **Intercompany** — IC transactions, matching, eliminations
5. **OCPM events** — OCEL 2.0 event log
6. **Journal entries** — from documents + standalone; applies the distribution cascade (fraud → advanced mixture → Pareto → copula → conditional → drift → seasonality)
7. **Anomaly injection** — entity-aware, risk-adjusted
8. **Fraud-bias sweep** — applies weekend / round-dollar / off-hours / post-close bias to every `is_fraud=true` entry
9. **Balance validation** — debits = credits, Assets = Liabilities + Equity
10. **Accruals + period close** — reversal dates snapped to business days (v3.4.3+)
11. **Financial reporting** — BS/IS/CF, segments, notes
12. **HR** — payroll, time entries (holiday-aware v3.4.2+), expense reports
13. **Manufacturing** — production orders with business-day-snapped dates
14. **Treasury / tax / ESG / project accounting**
15. **Audit data** — engagements, workpapers, evidence, findings, SOX
16. **Banking + AML** — KYC, transactions, typology labels
17. **Analytics metadata** (v3.3.0+) — prior-year comparatives, benchmarks, drift events
18. **Statistical validation** (v3.5.1+) — Benford / chi² / KS; attaches `StatisticalValidationReport`
19. **Graph + hypergraph export** — PyG, Neo4j, DGL, hypergraph

Determinism: seed-in → byte-identical-out on default configs.

## TemporalContext (v3.4.1+)

Shared `Arc<TemporalContext>` bundle built once per pipeline from
`config.temporal_patterns`, threaded into:

- Document-flow generators (P2P, O2C)
- HR (time entries, expense reports)
- Manufacturing (production orders)
- Period-close (accrual reversals)

Methods: `is_business_day`, `adjust_to_business_day`,
`sample_business_day_in_range`. 15 region calendars pre-loaded.

## Statistical validation phase (v3.5.1+)

New `phase_statistical_validation` runs after all JE-adding phases
and emits a `StatisticalValidationReport` on
`EnhancedGenerationResult.statistical_validation`. Supported tests
today: Benford first-digit (MAD), chi-squared on log-uniform,
KS-on-log-uniform. CorrelationCheck + AndersonDarling scheduled for
v4.1.0 (see [v4.1 plan](../../docs/plans/2026-04-21-v4.1-plan.md)).

## Usage

```rust
use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};
use datasynth_config::schema::GeneratorConfig;

let config: GeneratorConfig = /* load from YAML or build in code */;
let phases = PhaseConfig::from_config(&config);
let mut orch = EnhancedOrchestrator::new(config, phases)?;
let result = orch.generate()?;

println!("Generated {} journal entries", result.journal_entries.len());
if let Some(report) = &result.statistical_validation {
    println!("Validation: {} tests, all passed = {}",
        report.results.len(), report.all_passed());
}
```

## Pause/Resume

On Unix systems, send `SIGUSR1` to toggle pause:

```bash
kill -USR1 $(pgrep datasynth-data)
```

## License

Apache-2.0 — see [LICENSE](../../LICENSE) for details.

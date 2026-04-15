# Generation Pipeline

The `EnhancedOrchestrator` in `datasynth-runtime` coordinates the full generation workflow through a sequence of phases.

## Phase Overview

| Phase | Name | Description |
|-------|------|-------------|
| 1 | Config & Seed | Parse config, initialize ChaCha8 RNG, set up resource guards |
| 2 | Chart of Accounts | Generate GL accounts based on complexity level and industry |
| 3 | Master Data | Vendors, customers, materials, fixed assets, employees, cost centers |
| 4 | Document Flows | P2P chain (PO -> GR -> VI -> Payment), O2C chain (SO -> Delivery -> CI -> Receipt) |
| 5 | Journal Entries | Generate JEs from document flows, intercompany, period close |
| 6 | Subledgers | AR, AP, FA, inventory sub-ledger records |
| 7 | Balance & Close | Opening balances, trial balances, accruals, depreciation, year-end |
| 8 | Financial Reporting | Financial statements, consolidation, segment reporting |
| 9 | Anomaly Injection | Inject fraud, errors, and process anomalies per config rates |
| 10 | Standards & Audit | Accounting standards (revenue, leases, fair value), audit engagement |
| 10c | Assertions | Balance validation, IC elimination net-to-zero checks |
| 11 | Export | Write output files (JSON, CSV, Parquet), graph export, labels |

## Orchestrator Flow

```
Config YAML
    │
    ▼
EnhancedOrchestrator::new(config)
    │
    ▼
Phase 1-3: Foundation (CoA, master data)
    │
    ▼
Phase 4-5: Transaction generation (documents → JEs)
    │
    ▼
Phase 6-8: Derived data (subledgers, balances, statements)
    │
    ▼
Phase 9: Anomaly injection (targeted, preserving balances for clean entries)
    │
    ▼
Phase 10: Standards, audit, banking (parallel where possible)
    │
    ▼
Phase 10c: Assertions
  - Every non-anomaly JE balances (debits = credits)
  - IC elimination entries net to zero
  - Trial balance foots
    │
    ▼
Phase 11: Output writing
```

## PhaseConfig

The `PhaseConfig` struct controls which phases run:

```rust
pub struct PhaseConfig {
    pub generate_master_data: bool,
    pub generate_document_flows: bool,
    pub generate_journal_entries: bool,
    pub generate_subledgers: bool,
    pub generate_audit: bool,
    pub generate_banking: bool,
    pub generate_graph: bool,
    // ...
}
```

Phases are toggled by config sections (`master_data.enabled`, `audit.enabled`, etc.) and CLI flags (`--banking`, `--audit`, `--graph-export`).

## Resource Guards

Throughout generation, the orchestrator monitors:
- **Memory** (`MemoryGuard`) -- Reads `/proc/self/statm` on Linux, `ps` on macOS
- **Disk** (`DiskGuard`) -- Checks available space via `statvfs`
- **CPU** (`CpuMonitor`) -- Auto-throttles at 0.95 utilization

If resources are constrained, the `DegradationLevel` escalates: Normal -> Reduced -> Minimal -> Emergency, progressively disabling non-essential generators.

## Determinism

All generation uses ChaCha8 RNG with a configurable seed. Given the same config and seed, the output is bit-identical across runs (same platform).

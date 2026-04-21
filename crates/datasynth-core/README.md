# datasynth-core

Core domain models, traits, and distributions for synthetic accounting data generation.

## Overview

`datasynth-core` provides the foundational building blocks for the SyntheticData workspace:

- **Domain Models**: Journal entries, chart of accounts, master data, documents, anomalies
- **Statistical Distributions**: Line item sampling, amount generation, temporal patterns
- **Core Traits**: Generator and Sink interfaces for extensibility
- **Template System**: File-based templates for regional/sector customization
- **Infrastructure**: UUID factory, memory guard, GL account constants

## Key Components

### Domain Models (`models/`)

| Module | Description |
|--------|-------------|
| `journal_entry.rs` | Journal entry header and balanced line items |
| `chart_of_accounts.rs` | Hierarchical GL accounts with account types |
| `master_data.rs` | Enhanced vendors, customers with payment behavior |
| `documents.rs` | Purchase orders, invoices, goods receipts, payments |
| `temporal.rs` | Bi-temporal data model for audit trails |
| `anomaly.rs` | Anomaly types and labels for ML training |
| `internal_control.rs` | SOX 404 control definitions |

### Enterprise Process Chain Models (v0.6.0)

| Module | Description |
|--------|-------------|
| `sourcing/` | SourcingProject, RfxEvent, SupplierBid, ProcurementContract, CatalogItem and related procurement models |
| `bank_reconciliation.rs` | Bank reconciliation statements and matching rules |
| `financial_statements.rs` | Income statement, balance sheet, cash flow statement models |
| `payroll.rs` | Payroll runs, pay stubs, deductions, tax withholdings |
| `time_entry.rs` | Time tracking entries, approval workflows |
| `expense_report.rs` | Expense reports, line items, receipt matching |
| `production_order.rs` | Manufacturing production orders and operations |
| `quality_inspection.rs` | Quality inspection lots, results, defect codes |
| `cycle_count.rs` | Inventory cycle count programs and variances |
| `sales_quote.rs` | Sales quotations and quote-to-order conversion |
| `management_kpi.rs` | Management KPIs and scorecard metrics |
| `budget.rs` | Budget plans, line items, variance analysis |

### UUID Factory Extensions (v0.6.0)

The UUID factory (`uuid_factory.rs`) has been extended with 18 new `GeneratorType` discriminators (0x28-0x39) covering sourcing, HR, manufacturing, financial reporting, and sales/KPI/budget entities. This ensures collision-free deterministic UUID generation across all new model types.

### Statistical Distributions (`distributions/`)

| Module | Description |
|--------|-------------|
| `LineItemSampler` | Empirical distribution (60.68% two-line, 88% even counts) |
| `AmountSampler` | Log-normal with round-number bias, Benford compliance (legacy path) |
| `AdvancedAmountSampler` | Enum over LogNormal / Gaussian mixture + **Pareto** heavy-tailed (v3.4.4+) |
| `LogNormalMixtureSampler` / `GaussianMixtureSampler` | Multi-component mixture models |
| `BivariateCopulaSampler` | Gaussian / Clayton / Gumbel / Frank / Student-t (Gaussian wired at runtime in v3.5.4; others v4.1.0) |
| `ConditionalSampler` | Breakpoint-based distribution selection; `input_field` from calendar context |
| `TemporalSampler` | Seasonality patterns with industry integration |
| `BenfordSampler` | First-digit distribution following P(d) = log10(1 + 1/d) |
| `IndustryAmountProfile` | Pre-configured mixtures: retail / manufacturing / financial_services / healthcare / technology |
| `DriftController` | Regime changes + economic cycles + parameter drifts |
| `TemporalContext` (v3.4.1+) | Multi-year holiday calendar + business-day calculator bundle |
| `StatisticalValidationReport` (v3.5.1+) | Benford / chi² / KS goodness-of-fit runners |

### LLM + Template Infrastructure

| Component | Description |
|-----------|-------------|
| `llm::LlmProvider` | Trait for LLM backends (Mock, HttpLlmProvider via `llm` feature) |
| `llm::HttpLlmProvider` | OpenAI-compatible HTTP client; OpenRouter / Anthropic / OpenAI |
| `templates::TemplateProvider` | Abstraction for name/description pools |
| `templates::DefaultTemplateProvider` | Embedded arrays (v4.0 — v4.1.4 migrates to YAML-as-SoT) |
| `templates::LlmTemplateProvider` (v4.0.0+) | Runtime LLM-backed provider wrapping a base; opt-in per category with in-memory cache |
| `templates::TemplateLoader` | YAML/JSON load + save + merge; `load_from_yaml_str` for compile-time bundling |

### Infrastructure

| Component | Description |
|-----------|-------------|
| `uuid_factory.rs` | Deterministic FNV-1a hash-based UUID generation, 18 generator-type discriminators |
| `memory_guard.rs` / `disk_guard.rs` / `cpu_monitor.rs` | Resource monitors |
| `resource_guard.rs` | Unified resource orchestration with graceful degradation |
| `accounts.rs` | Centralized GL control account numbers |
| `fraud_bias.rs` | Weekend / round-dollar / off-hours / post-close bias applied to every `is_fraud=true` entry |
| `templates/` | YAML/JSON template loading, merging, LlmTemplateProvider |

## Usage

```rust
use datasynth_core::models::{JournalEntry, JournalEntryLine};
use datasynth_core::distributions::AmountSampler;

// Create a balanced journal entry
let mut entry = JournalEntry::new(header);
entry.add_line(JournalEntryLine::debit("1100", amount, "AR Invoice"));
entry.add_line(JournalEntryLine::credit("4000", amount, "Revenue"));

// Sample realistic amounts
let sampler = AmountSampler::new(seed);
let amount = sampler.sample_benford_compliant(1000.0, 100000.0);
```

## License

Apache-2.0 - See [LICENSE](../../LICENSE) for details.

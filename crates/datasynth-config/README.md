# datasynth-config

Configuration schema, validation, and industry presets for synthetic data generation.

## Overview

`datasynth-config` provides the configuration layer for SyntheticData:

- **Schema Definition**: Complete YAML configuration schema
- **Validation**: Bounds checking, constraint validation, distribution sum verification
- **Industry Presets**: Pre-configured settings for common industries
- **Complexity Levels**: Small, medium, and large organization profiles

## Configuration Sections

| Section | Description |
|---------|-------------|
| `global` | Industry, dates, seed, performance settings |
| `companies` | Company codes, currencies, volume weights |
| `chart_of_accounts` | COA complexity and structure |
| `transactions` | Line items, amounts, sources, temporal patterns |
| `master_data` | Vendors, customers, materials, assets, employees |
| `document_flows` | P2P, O2C configuration |
| `intercompany` | IC transaction types and transfer pricing |
| `balance` | Opening balances, trial balance generation |
| `subledger` | AR, AP, FA, inventory settings |
| `fx` | Currency and exchange rate settings |
| `period_close` | Close tasks and schedules |
| `fraud` | Fraud injection rates and types |
| `internal_controls` | SOX controls and SoD rules |
| `anomaly_injection` | Anomaly rates and labeling |
| `data_quality` | Missing values, typos, duplicates |
| `graph_export` | ML graph export formats |
| `output` | Output format and compression |
| `templates` | User-supplied template packs (`path`, `merge_strategy`) |
| `llm` | LLM provider + enrichment flags (gated by `llm` feature) |

### Advanced distributions (v3.4–v4.0)

Every knob in `distributions` now drives runtime behavior:

| Sub-section | Controls |
|---|---|
| `distributions.amounts` | Log-normal / Gaussian mixture components; `industry_profile` fallback |
| `distributions.pareto` | Heavy-tailed amount override (`enabled`, `alpha`, `x_min`, `max_value`) |
| `distributions.regime_changes` | Acquisition / price-increase / … events + economic cycles + parameter drifts |
| `distributions.conditional` | Breakpoint-based distribution switching; `input_field ∈ {month, quarter, constant}` |
| `distributions.correlations` | Gaussian copula for amount↔line_count (others v4.1); full symmetric or upper-triangular matrix |
| `distributions.validation` | Benford / chi² / KS tests run post-generation; report on `EnhancedGenerationResult.statistical_validation` |

### Temporal patterns (v3.4.1+)

| Sub-section | Controls |
|---|---|
| `temporal_patterns.business_days` | Weekend / half-day / month-end conventions; T+N settlement rules |
| `temporal_patterns.calendars` | 15 regions (US, DE, GB, FR, IT, ES, CA, CN, JP, IN, BR, MX, AU, SG, KR) |
| `temporal_patterns.period_end` | Flat / exponential / extended-crunch / daily-profile decay curves |
| `temporal_patterns.processing_lags` | Event-to-posting lag distributions |
| `temporal_patterns.fiscal_calendar` | Calendar / custom / 4-4-5 patterns |
| `temporal_patterns.timezones` | Multi-region timezone handling |
| `temporal_patterns.intraday` | Per-segment multipliers (morning spike / lunch dip / eod rush) |

### Enterprise Process Chain Sections (v0.6.0)

| Section | Description |
|---------|-------------|
| `source_to_pay` | `SourceToPayConfig` -- sourcing projects, RFx events, supplier bids, procurement contracts, catalogs |
| `financial_reporting` | `FinancialReportingConfig` -- financial statements, `ManagementKpisConfig`, `BudgetConfig` |
| `hr` | `HrConfig` -- `PayrollConfig`, `TimeAttendanceConfig`, `ExpenseConfig` |
| `manufacturing_process` | `ManufacturingProcessConfig` -- `ProductionOrderConfig`, `ManufacturingCostingConfig`, `RoutingConfig` |
| `sales_quotes` | `SalesQuoteConfig` -- quotation generation and quote-to-order conversion |

All new sections default to `enabled: false` for full backward compatibility with existing configurations.

## Industry Presets

| Industry | Description |
|----------|-------------|
| `manufacturing` | Heavy P2P, inventory, fixed assets |
| `retail` | High O2C volume, seasonal patterns |
| `financial_services` | Complex intercompany, high controls |
| `healthcare` | Regulatory focus, seasonal insurance |
| `technology` | SaaS revenue patterns, R&D capitalization |

## Usage

```rust
use datasynth_config::{Config, ConfigValidator};

// Load and validate configuration
let config = Config::from_yaml_file("config.yaml")?;
let validator = ConfigValidator::new();
validator.validate(&config)?;

// Use industry preset
let config = Config::preset_manufacturing(Complexity::Medium);
```

## Validation Rules

- `period_months`: 1-120 (max 10 years)
- `compression_level`: 1-9 when enabled
- All rate/percentage fields: 0.0-1.0
- Approval thresholds: strictly ascending order
- Distribution sums: must equal 1.0 (±0.01 tolerance)

## License

Apache-2.0 - See [LICENSE](../../LICENSE) for details.

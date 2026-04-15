# YAML Reference

Every top-level section in the config file is optional. Omitted sections use sensible defaults.

## Top-Level Sections

| Section | Purpose |
|---------|---------|
| `global` | Industry, start date, period, seed, Benford compliance |
| `companies` | Company entities (code, name, currency, country) |
| `chart_of_accounts` | GL structure and complexity level |
| `transactions` | Journal entry volume and posting patterns |
| `output` | Formats (csv/json/parquet), compression, sink |
| `fraud` | Fraud injection rates and scheme types |
| `internal_controls` | COSO 2013, SoD rules, maturity level, exception rates |
| `enterprise` | Multi-entity group structure |
| `master_data` | Vendor, customer, material, asset, employee counts |
| `document_flows` | P2P and O2C document chain settings |
| `intercompany` | IC matching, elimination, NCI |
| `balance` | Opening balances, trial balance generation |
| `subledger` | AR, AP, FA, inventory sub-ledger records |
| `fx` | FX rates, currency translation, CTA |
| `period_close` | Close engine, accruals, depreciation, year-end |
| `distributions` | Amount distributions, copulas, correlations, Benford |
| `temporal_patterns` | Business days, calendars, period-end dynamics, intraday |
| `accounting_standards` | Revenue recognition, leases, fair value, impairment |
| `audit_standards` | ISA/PCAOB compliance, analytical procedures, SOX |
| `audit` | Audit engagement and workpaper generation |
| `vendor_network` | Multi-tier supply chain, clusters, dependencies |
| `customer_segmentation` | Value segments, lifecycle stages, networks |
| `relationship_strength` | Cross-entity relationship scoring |
| `cross_process_links` | P2P/O2C inventory links, payment reconciliation |
| `source_to_pay` | Sourcing, RFx, contracts, catalogs, scorecards |
| `financial_reporting` | Financial statements, KPIs, budgets |
| `hr` | Payroll, time & attendance, expenses, benefits |
| `manufacturing` | Production orders, quality, cycle counts, BOM |
| `sales_quotes` | Quote generation settings |
| `graph_export` | PyTorch Geometric, Neo4j, DGL export config |
| `anomaly_injection` | Anomaly types, rates, and targeting rules |
| `data_quality` | Missing values, format variations, typos, duplicates |
| `diffusion` | Neural/statistical diffusion backend settings |
| `scenarios` | Counterfactual scenario definitions and causal models |
| `templates` | YAML/JSON template loading with merge strategies |
| `approval` | Approval threshold levels |
| `departments` | Department structure |
| `streaming` | Streaming output API configuration |
| `llm` | LLM enrichment provider settings |
| `causal` | Causal DAG templates (fraud_detection, revenue_cycle) |
| `session` | Period-by-period generation with balance carry-forward |
| `compliance_regulations` | Standards registry, jurisdictions, regulatory filings |
| `treasury` | Cash positioning, forecasting, debt instruments, hedging |
| `project_accounting` | Projects, earned value, change orders, milestones |
| `esg` | Emissions, energy, water, waste, governance metrics |
| `banking` | KYC/AML customer and transaction generation |

## Example: Enabling a Section

Most sections follow the same pattern:

```yaml
hr:
  enabled: true
  payroll:
    enabled: true
  time_attendance:
    enabled: true
  expenses:
    enabled: true
```

Set `enabled: false` (or omit the section) to skip generation for that domain.

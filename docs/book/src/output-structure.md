# Output Structure

DataSynth writes output files organized by domain directory. All files are JSON by default; CSV and Parquet are available via config.

## Format-Aware Output

Control which formats are written per run:

```yaml
output:
  formats: [csv]          # Only write CSV (4x faster for large runs)
  formats: [csv, json]    # Both formats
  formats: [json]         # JSON only (default)
```

CSV-only mode skips JSON serialization entirely, producing significant speedups on large datasets.

## Directory Layout

```
output/
  journal_entries.csv
  journal_entries.json
  acdoca.csv
  session.dss                          # Session state for --append
  manifest.json                        # Run manifest with checksums
  master_data/
    vendors.json
    customers.json
    materials.json
    fixed_assets.json
    employees.json
    cost_centers.json
  document_flows/
    purchase_orders.json
    goods_receipts.json
    vendor_invoices.json
    payments.json
    sales_orders.json
    deliveries.json
    customer_invoices.json
    customer_receipts.json
    document_references.json
  subledger/
    ar_invoices.json, ap_invoices.json
    fa_records.json, inventory_positions.json
    ar_aging.json, ap_aging.json
    depreciation_runs.json, inventory_valuation.json
  balance/
    opening_balances.json
    subledger_reconciliation.json
  period_close/
    trial_balances.json
  financial_reporting/
    financial_statements.json
    bank_reconciliations.json
    standalone/          # Per-entity statements
    consolidated/        # Group consolidation
    segment_reporting/   # Operating segments
  intercompany/
    group_structure.json
    ic_matched_pairs.json
    ic_elimination_entries.json
  fx/
    fx_rates.json, cta_entries.json
  tax/
    tax_jurisdictions.json, tax_provisions.json
    tax_returns.json, deferred_tax_rollforward.json
  treasury/
    cash_positions.json, debt_instruments.json
    hedging_instruments.json
  hr/
    payroll_runs.json, time_entries.json
    expense_reports.json, pension_plans.json
  manufacturing/
    production_orders.json
    quality_inspections.json
    bom_components.json
  sourcing/
    sourcing_projects.json, procurement_contracts.json
  project_accounting/
    projects.json, earned_value_metrics.json
  esg/
    emission_records.json, energy_consumption.json
  banking/
    banking_customers.json, banking_transactions.json
    aml_transaction_labels.json
  audit/
    audit_engagements.json, audit_workpapers.json
    audit_opinions.json, key_audit_matters.json
    sox_302_certifications.json, sox_404_assessments.json
  internal_controls/
    internal_controls.csv, sod_violations.json
    coso_control_mapping.csv
  labels/
    anomaly_labels.json, fraud_labels.json
    quality_issues.json
  graphs/
    *.pt (PyTorch Geometric)
    neo4j/ (CSV + Cypher)
  process_mining/
    event_log.json        # OCEL 2.0
  accounting_standards/
    customer_contracts.json
    ecl_models.json, provisions.json
```

## Additional Export Formats

Use `--export-format` to produce regulatory exports alongside the standard output:

```bash
datasynth-data generate --config config.yaml --export-format sap --export-format fec
```

| Format | Description |
|--------|-------------|
| `sap` | SAP S/4HANA BKPF/BSEG/ACDOCA tables (CSV) |
| `fec` | FEC Fichier des Ecritures Comptables (French GAAP, 18 columns) |
| `gobd` | GoBD journal + accounts + index.xml (German GAAP) |

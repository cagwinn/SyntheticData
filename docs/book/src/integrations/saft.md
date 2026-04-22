# SAF-T Export

DataSynth emits SAF-T (Standard Audit File for Tax) XML for
jurisdictions that have adopted the OECD standard. Shipped in v4.3.1.

## Supported jurisdictions

| Code | Country     | Nominal spec      | Output filename  |
|------|-------------|-------------------|------------------|
| `pt` | Portugal    | SAF-T PT 1.04_01  | `saft_pt.xml`    |
| `pl` | Poland      | JPK_KR 1.0        | `jpk_kr.xml`     |
| `ro` | Romania     | D406 3.0          | `d406.xml`       |
| `no` | Norway      | SAF-T Financial 1.10 | `saft_no.xml` |
| `lu` | Luxembourg  | FAIA 2.01         | `faia.xml`       |

Default: `pt` (most mature variant).

## Quick start

```yaml
# config.yaml
output:
  output_directory: ./out
  formats: [json]
  saft:
    jurisdiction: pt
    company_tax_id: "PT500123456"
    company_name: "Demo Portugal Lda"
```

```bash
datasynth-data generate --config config.yaml --output ./out \
                        --export-format saft
```

Produces a single XML file at `./out/saft_pt.xml` (filename varies by
jurisdiction). The company name and tax ID fall back to the first
configured company when `company_name` / `company_tax_id` are left
empty.

## Emitted structure

```
<AuditFile xmlns="urn:OECD:StandardAuditFile-Tax:PT_1.04_01">
  <Header>
    AuditFileVersion, CompanyID, TaxRegistrationNumber, CompanyName,
    FiscalYear, StartDate, EndDate, CurrencyCode, DateCreated, ...
  </Header>
  <MasterFiles>
    <GeneralLedgerAccounts>  — from ChartOfAccounts
    <Customer>               — from master_data.customers
    <Supplier>               — from master_data.vendors
    <Product>                — from master_data.materials
    <TaxTable>               — stub (single row, for schema compliance)
  </MasterFiles>
  <GeneralLedgerEntries>
    NumberOfEntries, TotalDebit, TotalCredit,
    <Journal JournalID="GEN">
      <Transaction>          — one per journal entry
        <Lines>               — debit/credit line per JE line
      </Transaction>
    </Journal>
  </GeneralLedgerEntries>
</AuditFile>
```

## Conformance notes

This is a **structurally valid** SAF-T file — XML well-formed, correct
element tree, matching namespace per jurisdiction — but it is **not a
fully conformant regulatory submission**. In particular:

- `TaxTable` is a single-row stub. Real filings need the entity's
  actual VAT / tax codes.
- `TaxAccountingBasis` is hardcoded to `F` (standard basis). Some
  jurisdictions require `S` (self-billing), `C` (contact), or others.
- `SourceDocuments` (invoice-level detail) is not emitted. Most
  jurisdictions require this for VAT returns but not for GL audit.
- Opening / closing balances on GeneralLedgerAccounts are zeroed —
  DataSynth doesn't track period-specific balances in this export yet.

Audit firms should validate against the jurisdiction's XSD
(`xmllint --schema …`) before regulatory filing.

## Where to extend

The writer lives in `crates/datasynth-output/src/formats/saft.rs` and
is ~500 lines. Per-jurisdiction extensions (e.g. adding
`SourceDocuments` for Portugal, Polish `JPK_VAT` extensions, Romanian
D406-specific fields) slot naturally into `write_master_files` and
`write_general_ledger_entries` with a `match cfg.jurisdiction { .. }`
block.

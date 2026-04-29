# SAP Integration

DataSynth emits a complete set of SAP tables — 27 in total — directly
from the synthetic-data generation pipeline. The export covers the
classic BKPF/BSEG/ACDOCA transactional triple, 10 master-data tables,
8 document-flow tables, and 6 subledger open/cleared-item tables.

Shipped in the v4.3.0 release line (v4.3.0a → v4.3.0d).

## Quick start

Enable SAP export with a single CLI flag and a YAML config block:

```yaml
# config.yaml
output:
  output_directory: ./out
  formats: [json]
  sap:
    client: "200"
    ledger: "0L"
    source_system: "DATASYNTH"
    local_currency: "EUR"
    dialect: hana
    tables: [bkpf, bseg, acdoca, lfa1, lfb1, kna1, knb1, mara, mard,
             anla, csks, ska1, skb1, ekko, ekpo, vbak, vbap,
             likp, lips, mkpf, mseg, bsis, bsas, bsid, bsad, bsik, bsak]
    include_extension_fields: true
```

```bash
datasynth-data generate --config config.yaml --output ./out \
                        --export-format sap
```

Outputs land under `./out/sap_export/`, one CSV per requested table,
all in the configured dialect (delimiter / decimal separator / date
format / UTF-8 BOM).

## Dialects

| Dialect  | Delimiter | Decimal | Date Format  | BOM      | Target import  |
|----------|-----------|---------|--------------|----------|----------------|
| `classic`| `,`       | `.`     | `YYYYMMDD`   | none     | SAP BODS / R/3 CSV flat-file loader |
| `hana`   | `;`       | `,`     | `YYYY-MM-DD` | UTF-8    | S/4HANA `IMPORT FROM CSV FILE` (German locale default) |

`classic` is the default for backward compatibility. Switch to `hana`
when loading into S/4HANA with a non-UTF-8 client locale (the typical
EU deployment).

## Supported tables

### GL transactional triple

| Table   | Source                | Rows emitted |
|---------|-----------------------|--------------|
| BKPF    | `JournalEntry` header | 1 per JE     |
| BSEG    | `JournalEntry` lines  | 1 per line   |
| ACDOCA  | `JournalEntry` lines  | 1 per line   |

ACDOCA carries optional `ZSIM_*` extension columns when
`include_extension_fields: true` — these expose fraud / SOX / SoD
labels directly so ML consumers can train on the SAP table instead of
re-joining the label files.

### Master data (10 tables)

| Table   | Source                          | Rows                           |
|---------|---------------------------------|--------------------------------|
| LFA1    | `Vendor`                        | 1 per vendor (general data)    |
| LFB1    | `Vendor` × companies            | 1 per (vendor, company code)   |
| KNA1    | `Customer`                      | 1 per customer (general data)  |
| KNB1    | `Customer` × companies          | 1 per (customer, company code) |
| MARA    | `Material`                      | 1 per material (general data)  |
| MARD    | `Material` × plants             | 1 per (material, plant, lgort) |
| ANLA    | `FixedAsset`                    | 1 per asset                    |
| CSKS    | `CostCenter`                    | 1 per cost centre              |
| SKA1    | `ChartOfAccounts` → `GLAccount` | 1 per GL account (chart-wide)  |
| SKB1    | `GLAccount` × companies         | 1 per (GL account, company)    |

> **CEPC (profit-centre master) is not emitted in v5.0.** The
> `SapTableType::Cepc` enum variant exists for future expansion and
> appears in the CLI's accepted-tables list, but no `ProfitCenter`
> master-data model / generator / writer is wired up yet. Requesting
> `cepc` in `output.sap.tables` surfaces a `tracing::warn!` and emits
> nothing — drop the entry to silence the warning. Full implementation
> is tracked as Gap 6 in the v5.1 roadmap.

### Document flow (8 tables)

| Table   | Source            | Rows                         |
|---------|-------------------|------------------------------|
| EKKO    | `PurchaseOrder`   | 1 per PO (header)            |
| EKPO    | `PurchaseOrderItem` | 1 per PO item             |
| VBAK    | `SalesOrder`      | 1 per SO (header)            |
| VBAP    | `SalesOrderItem`  | 1 per SO item                |
| LIKP    | `Delivery`        | 1 per delivery (header)      |
| LIPS    | `DeliveryItem`    | 1 per delivery item — VGBEL refs SO |
| MKPF    | `GoodsReceipt`    | 1 per material document      |
| MSEG    | `GoodsReceiptItem`| 1 per material-doc item — EBELN refs PO |

### Subledger open/cleared items (6 tables)

| Table | Subledger | Open items      | Source                     |
|-------|-----------|-----------------|----------------------------|
| BSIS  | GL        | open GL items   | JE lines (all lines)       |
| BSAS  | GL        | cleared GL items| empty placeholder          |
| BSID  | AR (KUNNR)| open AR items   | `ARInvoice.amount_remaining` |
| BSAD  | AR (KUNNR)| cleared AR items| `ARInvoice.clearing_info[]`  |
| BSIK  | AP (LIFNR)| open AP items   | `APInvoice.amount_remaining` |
| BSAK  | AP (LIFNR)| cleared AP items| `APInvoice.clearing_info[]`  |

## Semantic mappings

Enum translations happen inside the exporters so the CSV values match
what a real SAP transport would contain:

- **VendorType → KTOKK**: Supplier→LIEF, ServiceProvider/ProfessionalServices→SERV,
  Technology→TECH, Logistics→LOGI, Contractor→CONT, RealEstate→REST,
  Financial→FINA, Utility→UTIL, EmployeeReimbursement→EMPL.
- **CustomerType → KTOKD**: Corporate→KUNA, SmallBusiness→KUN1,
  Consumer→CPDB, Government→GOVT, NonProfit→NPRF, Intercompany→INTR,
  Distributor→DIST.
- **MaterialType → MTART**: RawMaterial→ROH, SemiFinished→HALB,
  FinishedGood→FERT, TradingGood→HAWA, OperatingSupplies→HIBE,
  SparePart→ERSA, Packaging→VERP, Service→DIEN.
- **MaterialGroup → MATKL**: 4-char codes (ELEC, MECH, CHEM, OFFC,
  ITEQ, FURN, PACK, SAFE, TOOL, SERV, CONS, FINI).
- **AssetClass → ANLKL**: 4-digit numeric classes — Buildings→1000,
  Land→1100, MachineryEquipment→2000, Vehicles→3000, Furniture→4000,
  LeaseholdImprovements→4500, ComputerHardware→5000, Intangibles→7000,
  ConstructionInProgress→8000, LowValueAssets→9000.
- **CostCenterCategory → KOSAR**: Production→F, Administration→H,
  Sales→V, RAndD→E, Corporate→1.
- **PurchaseOrderType → BSART**: Standard→NB, Framework→FO, Service→DB,
  StockTransfer→UB, Subcontracting→LB, Consignment→K.
- **SalesOrderType → AUART**: Standard→OR, Rush→SO, CashSale→BV,
  Return→RE, FreeOfCharge→FD, Consignment→KB, Service→DS,
  CreditMemoRequest→G2, DebitMemoRequest→L2.
- **DeliveryType → LFART**: Outbound→LF, Return→LR, StockTransfer→UL,
  Replenishment→NL, ConsignmentIssue→LK, ConsignmentReturn→RL.
- **Country → SPRAS**: DE/AT/CH→D, FR/BE/LU→F, ES/MX/AR/CO/CL→S, IT→I,
  PT/BR→P, CN→1, JP→J, RU→R, PL→L, else→E.

## Cross-table references

Foreign-key-style columns carry over between tables so consumers can
reconstruct the document flow graph without joining against the
DataSynth native JSON:

| Source table | Column | Points to        |
|--------------|--------|------------------|
| EKPO         | EBELN  | EKKO.EBELN       |
| VBAP         | VBELN  | VBAK.VBELN       |
| LIPS         | VBELN  | LIKP.VBELN       |
| LIPS         | VGBEL  | VBAK.VBELN (ref SO) |
| MSEG         | MBLNR  | MKPF.MBLNR       |
| MSEG         | EBELN  | EKKO.EBELN (ref PO) |
| BSID / BSAD  | KUNNR  | KNA1.KUNNR       |
| BSIK / BSAK  | LIFNR  | LFA1.LIFNR       |
| BSEG         | BELNR  | BKPF.BELNR       |
| ACDOCA       | BELNR  | BKPF.BELNR       |
| BSAD         | AUGBL  | Clearing doc ID  |
| BSAK         | AUGBL  | Clearing doc ID  |

## Typical ingestion flow

### HANA-native via `IMPORT FROM CSV FILE`

1. Set `dialect: hana` in the YAML config.
2. Generate — tables land as semicolon-CSV with a UTF-8 BOM and ISO
   dates.
3. Create HANA target tables matching the column order in each CSV
   (column names are in the header row).
4. Run `IMPORT FROM CSV FILE '<path>' INTO <table> WITH RECORD DELIMITED
   BY '\n' FIELD DELIMITED BY ';' OPTIONALLY ENCLOSED BY '"' ERROR LOG
   '<logfile>'`.

Note: HANA's default locale for the German market expects decimal
comma and date `YYYY-MM-DD` — the `hana` dialect matches that out of
the box.

### BODS / Data Services (classic R/3)

1. Leave `dialect: classic` (default) or set it explicitly.
2. Generate — tables land as comma-CSV with `YYYYMMDD` dates and dot
   decimal.
3. Point BODS at the folder; auto-discovery treats each file as a
   flat-file data source.
4. Create flow objects that map `MANDT / BUKRS / BELNR / ...` to the
   target SAP system.

## Reproducible seed runs

The full SAP pack respects the global seed — a given config + seed
always produces the same rows with the same IDs, so downstream tests
that snapshot-compare the CSVs remain deterministic across runs.

## See also

- `crates/datasynth-output/src/formats/sap.rs` — BKPF/BSEG/ACDOCA + `SapDialect`
- `crates/datasynth-output/src/formats/sap_master_data.rs` — LFA1 … SKB1
- `crates/datasynth-output/src/formats/sap_transactional.rs` — EKKO … MSEG
- `crates/datasynth-output/src/formats/sap_subledger.rs` — BSIS … BSAK

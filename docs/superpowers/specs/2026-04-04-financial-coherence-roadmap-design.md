# Financial Coherence Roadmap: v2.2 – v2.4

**Date:** 2026-04-04
**Status:** Approved
**Drivers:** AssureTwin audit simulation completeness, compliance testing tooling

## Design Principle

Maximize cross-module interconnectedness. Every wave is sequenced by what creates the most connections in the data web. New sub-domains are not bolted on — they emerge from making audit trails complete. Each wave's acceptance test: "can an AssureTwin audit engagement fully exercise this pathway end-to-end?"

## Overview

| Wave | Theme | Audit Need |
|------|-------|-----------|
| **v2.2** | COGS & Inventory Valuation | Trace raw material → WIP → FG → sale → COGS; audit IC inventory transfers |
| **v2.3** | Treasury, Debt & Tax | Audit interest expense, hedge effectiveness, tax provision, payroll |
| **v2.4** | Full Financial Statement Package | Opine on complete FS: cash flow, segments, notes, ESG, XBRL export |

---

## Wave v2.2 — "Audit COGS and Inventory Valuation"

### Problem Statement

Manufacturing generates production orders with an `actual_cost` field, but there's no proper cost flow through the accounting cycle. An auditor in AssureTwin trying to audit inventory valuation or COGS hits dead ends — no WIP balance, no FG transfers, no standard cost variances, no warranty provisions from quality failures. Intercompany COGS is worse: eliminations exist but the source IC purchase/sale transactions that created them don't.

### 1. Manufacturing Cost Accounting Pipeline

New generation flow inside the orchestrator:

```
ProductionOrder (started)     → Debit WIP, Credit Raw Materials
RoutingOperation (completed)  → Debit WIP, Credit Labor Accrual (using setup_time + run_time × rate)
ProductionOrder (completed)   → Debit Finished Goods, Credit WIP
SalesOrder (delivered)        → Debit COGS, Credit Finished Goods
QualityInspection (failed)    → Debit Scrap/Rework Expense, Credit WIP or FG
```

Each step produces JEs with full traceability (source document ID, line references). The `actual_cost` on production orders becomes a derived value (sum of material issues + labor absorption + overhead allocation) rather than a standalone number.

**Standard cost variances** emerge naturally:
- Material price variance (actual vs standard cost per unit)
- Material usage variance (actual vs standard quantity)
- Labor rate & efficiency variances
- Overhead volume variance

These post as separate JEs to variance accounts — auditable, reconcilable.

### 2. Warranty Provisions (emergent sub-domain)

When `QualityInspection` results show failure patterns, a `WarrantyProvisionGenerator` creates:
- `Provision` records (using existing model in datasynth-core) with warranty-specific provision types
- Provision JEs: Debit Warranty Expense, Credit Warranty Provision liability
- `ProvisionMovement` records as claims are realized against the provision
- Links back to the product/production order that created the exposure

Connects: Manufacturing → Quality → Provisions → GL → Financial Statements → Audit (ISA 540 estimates).

### 3. Intercompany Source Transactions

Today: `EliminationGenerator` creates elimination JEs in isolation.

New flow:
```
IC SalesOrder (seller entity)  → IC CustomerInvoice → IC AR
IC PurchaseOrder (buyer entity) → IC GoodsReceipt → IC VendorInvoice → IC AP
ICMatchedPair                  → links seller-side and buyer-side documents
EliminationGenerator           → now consumes actual IC doc amounts (not generated independently)
```

IC docs flow through existing P2P/O2C generators with an `is_intercompany: true` flag and counterparty entity references. Transfer pricing method (from existing `TransferPricingMethod` model) determines the IC price.

### 4. New Coherence Validators

- **COGS Reconciliation**: Beginning FG + Production Completions - COGS - Scrap = Ending FG
- **WIP Reconciliation**: Beginning WIP + Material Issues + Labor + Overhead - Completions - Scrap = Ending WIP
- **IC Elimination Completeness**: Every IC matched pair has corresponding elimination entries that net to zero
- **Variance Analysis**: Sum of variances = Actual Cost - Standard Cost (per production order)

### GL Accounts Added

- WIP (1400-range), Finished Goods (1410), COGS (5100), Scrap Expense (5200)
- Labor Accrual (2100-range), Overhead Applied (5300-range)
- Variance accounts: Material Price (5110), Material Usage (5120), Labor Rate (5130), Labor Efficiency (5140), Overhead Volume (5150)
- Warranty Provision (2400), Warranty Expense (5400)

### Files Impacted

| Crate | Files | Change |
|-------|-------|--------|
| datasynth-core | `accounts.rs`, `models/manufacturing.rs`, `models/provisions.rs` | New GL constants, cost breakdown fields on ProductionOrder |
| datasynth-generators | `manufacturing/production_order_generator.rs` | Cost roll-up logic, WIP/FG/COGS JE generation |
| datasynth-generators | `manufacturing/quality_inspection_generator.rs` | Scrap/rework JE generation |
| datasynth-generators | New: `manufacturing/warranty_provision_generator.rs` | Warranty provision from quality failure rates |
| datasynth-generators | `intercompany/ic_generator.rs` | Generate source IC P2P/O2C documents |
| datasynth-generators | `intercompany/elimination_generator.rs` | Consume actual IC doc amounts |
| datasynth-runtime | `enhanced_orchestrator.rs` | Wire new generation sequence, dependency ordering |
| datasynth-eval | New validators in `coherence/` | COGS reconciliation, WIP reconciliation, IC completeness |
| datasynth-config | `schema.rs` | Manufacturing cost accounting config section |

### Acceptance Test

> An AssureTwin audit engagement can select "Inventory Valuation" and "COGS" as in-scope areas and trace from raw material purchase → WIP → finished goods → sale → COGS, with every step backed by JEs, source documents, and reconciling balances. IC inventory transfers are fully eliminable. Warranty provisions are auditable as accounting estimates (ISA 540).

---

## Wave v2.3 — "Audit Treasury, Debt, and Tax"

### Problem Statement

Treasury generates debt instruments, hedging instruments, cash positions, and cash pools — but none produce journal entries. An auditor examining interest expense sees nothing. Hedge effectiveness is declared but has no P&L impact. Tax provisions are computed independently from actual pre-tax income, so the effective tax rate proof doesn't tie. Transaction-level tax coding doesn't exist, making VAT/GST return validation impossible against source documents.

### 1. Treasury → GL Pipeline

**Debt Instruments:**
```
Origination       → Debit Cash, Credit Debt Liability (at face/issue price)
Interest Accrual  → Debit Interest Expense, Credit Interest Payable (each period)
Interest Payment  → Debit Interest Payable, Credit Cash
Amortization      → Debit/Credit Debt Liability, Credit/Debit Interest Expense (effective interest method)
Maturity/Repayment → Debit Debt Liability, Credit Cash
```

Amortization uses the effective interest method — the discount/premium amortizes over the life of the instrument, so the carrying amount converges to face value at maturity.

**Covenant Compliance (emergent sub-domain):**

`DebtCovenant` records already exist. New: each period, the engine evaluates covenant ratios against actual financial data:
- Debt-to-equity from real trial balance
- Interest coverage from real interest expense and EBITDA
- Current ratio from real current assets/liabilities

Covenant breaches generate:
- `CovenantBreachEvent` record with actual vs. required ratio
- Reclassification JE (long-term debt → current, if material breach)
- Links to going concern assessment (ISA 570) in audit FSM

**Hedging Instruments:**
```
Designation       → Memo entry (hedge relationship documented)
Period-end MTM    → Fair value change on derivative:
  Cash flow hedge → Debit/Credit Derivative Asset/Liability, Credit/Debit OCI
  Fair value hedge → Debit/Credit Derivative Asset/Liability, Credit/Debit P&L
                     + Debit/Credit Hedged Item, Credit/Debit P&L (offsetting)
Ineffectiveness   → Excess fair value change → P&L (not OCI)
Settlement        → Debit/Credit Cash, Credit/Debit Derivative Asset/Liability
Reclassification  → OCI → P&L when hedged transaction occurs
```

Hedge effectiveness testing uses the dollar-offset method (ratio of derivative fair value change to hedged item fair value change). Effectiveness ratio outside 80-125% triggers discontinuation.

**Cash Pool Sweeps:**
```
Physical pooling  → Debit/Credit IC Receivable/Payable between participant and header
Notional pooling  → No JEs (off-balance-sheet), but interest optimization calculated
```

### 2. Tax-to-Transaction Linkage

New flow:
```
Transaction generated (PO, Invoice, etc.)
  → Tax determination: jurisdiction from entity country + transaction type
  → TaxCode assigned to each line item
  → TaxLine generated: taxable_amount × rate = tax_amount
  → Tax GL posting: Debit Input VAT / Credit Output VAT
  → Tax Return aggregation: sum TaxLines by jurisdiction + period
```

**Tax Provision ← Actual Pre-Tax Income:**
```
Trial Balance (period-end)
  → Compute pre-tax income from actual revenue - expense accounts
  → Current tax = pre-tax income × statutory rate × (1 + permanent differences)
  → Deferred tax = temporary differences × rate (from existing TemporaryDifference model)
  → ETR reconciliation: statutory rate → effective rate (with each reconciling item traced)
  → Tax provision JEs: Debit Tax Expense, Credit Tax Payable / Deferred Tax Liability
```

**Deferred Tax Proof (emergent sub-domain):**

Temporary differences now derived from real data:
- Depreciation: tax depreciation schedule vs. book depreciation (already generated by FA module)
- Warranty provisions: book expense recognized, tax deduction on cash basis
- Revenue: percentage-of-completion (book) vs. completed contract (tax) for project revenue
- Lease liabilities: ROU asset/liability timing differences

Each temporary difference produces a `DeferredTaxRollforward` entry that reconciles opening → current year → closing.

### 3. Payroll ↔ HR Change Events

`PayrollGenerator` consumes `EmployeeChangeHistory` to determine:
- Base salary for the period (effective date logic — pro-rate if mid-period change)
- Bonus eligibility changes (promotion → new bonus tier)
- Benefit enrollment changes → deduction adjustments
- Termination → final paycheck with accrued PTO payout

An auditor testing payroll expense can trace a variance to a specific HR event (hire, promotion, termination) with dates and approval documentation.

### 4. New Coherence Validators

- **Interest Expense Proof**: Total interest expense = Σ(carrying amount × effective rate × days/360) across all debt instruments
- **Hedge Effectiveness**: Every designated hedge has effectiveness ratio calculated; discontinued hedges have P&L reclassification entries
- **ETR Reconciliation**: Statutory rate × pre-tax income + reconciling items = actual tax expense (±tolerance)
- **Deferred Tax Balance**: Opening DTA/DTL + current year movement = closing DTA/DTL per the trial balance
- **VAT/GST Return**: Sum of output tax - input tax per jurisdiction per period = net payable, traceable to source documents
- **Payroll-to-HR Reconciliation**: Payroll amount changes trace 1:1 to HR change events within the period

### GL Accounts Added

- Interest Payable (2150), Interest Expense (6100), Debt Premium/Discount (2060)
- Derivative Asset (1500), Derivative Liability (2500), OCI - Cash Flow Hedges (3200)
- Input VAT (1160), Output VAT (2160), Tax Payable (2170), Deferred Tax Asset (1600), Deferred Tax Liability (2600)
- Tax Expense - Current (7100), Tax Expense - Deferred (7200)

### Files Impacted

| Crate | Files | Change |
|-------|-------|--------|
| datasynth-core | `accounts.rs`, `models/treasury.rs`, `models/tax.rs` | New GL constants, covenant breach model, tax line linkage fields |
| datasynth-generators | `treasury/debt_generator.rs` | Interest accrual, amortization, maturity JEs |
| datasynth-generators | `treasury/hedging_generator.rs` | MTM, effectiveness testing, OCI/P&L JEs |
| datasynth-generators | `treasury/cash_pool_generator.rs` | Physical pooling IC JEs |
| datasynth-generators | `tax/tax_provision_generator.rs` | Consume actual pre-tax income from TB |
| datasynth-generators | `tax/tax_line_generator.rs` (new) | Transaction-level tax determination and GL posting |
| datasynth-generators | `tax/deferred_tax_generator.rs` (new) | Temporary difference derivation from real data |
| datasynth-generators | `hr/payroll_generator.rs` | Consume employee change history for salary determination |
| datasynth-runtime | `enhanced_orchestrator.rs` | Treasury JE sequencing after period-close, tax after TB |
| datasynth-eval | New validators in `coherence/` | Interest proof, hedge effectiveness, ETR, DTA/DTL, VAT, payroll-HR |
| datasynth-config | `schema.rs` | Treasury accounting method config, tax determination config |

### Ordering Dependencies

v2.2 must complete first because:
- COGS feeds pre-tax income → tax provision accuracy
- IC source transactions feed IC tax (transfer pricing adjustments)
- Warranty provisions create temporary differences → deferred tax

### Acceptance Test

> An AssureTwin audit engagement can scope "Treasury & Debt", "Tax Provision", or "Payroll" and trace: debt instrument → interest accrual → payment → covenant test → going concern indicator. Hedging instruments show effectiveness testing with P&L impact. Tax expense reconciles from statutory rate to effective rate with every reconciling item sourced. Payroll changes trace to HR events. VAT returns tie to source invoices.

---

## Wave v2.4 — "Audit the Full Financial Statements as a Coherent Package"

### Problem Statement

After v2.2 and v2.3, every major transaction cycle produces JEs and has audit trail coverage. But the financial statements as a whole — the thing an auditor actually opines on — still have gaps. There's no generated cash flow statement. ESG disclosures float disconnected from operational reality. Segment reporting uses placeholder allocations. And there's no way to export the package in regulatory formats for compliance testing.

This wave closes the loop: complete financial statements, fully derived from generated data, auditable as a package, exportable for compliance tooling.

### 1. Cash Flow Statement Generation

Purely derived from data generated in v2.2/v2.3:

```
Operating Activities:
  Net income (from income statement)
  + Depreciation & amortization (from FA module period-close JEs)
  + Impairment charges (from existing impairment_generator)
  + Provision movements (warranty, ECL, contingent — net of cash settlements)
  + Deferred tax movement (from v2.3 deferred tax rollforward)
  + Working capital changes:
      ΔAR (opening vs closing AR subledger)
      ΔAP (opening vs closing AP subledger)
      ΔInventory (opening vs closing — now accurate from v2.2 FG/WIP)
      ΔAccruals (interest payable, tax payable, warranty)

Investing Activities:
  - Capital expenditure (from FixedAsset acquisitions)
  - Proceeds from asset disposals (from FA disposal JEs)
  - Business combination consideration paid (from existing PurchasePriceAllocation)
  + Project capital spend (from project cost lines flagged as capex)

Financing Activities:
  + Debt issuance proceeds (from v2.3 debt origination JEs)
  - Debt repayments (from v2.3 maturity JEs)
  - Interest paid (from v2.3 interest payment JEs — reclassified from operating under IFRS option)
  - Dividends paid (new: DividendDeclaration model)
  + Equity issuance (if stock comp exercises generate cash)
  - Cash pool sweep movements (from v2.3 physical pooling)

Reconciliation:
  Opening cash + Net cash flows = Closing cash (must equal cash GL balance)
```

**Framework sensitivity**: Under US GAAP, interest paid is operating. Under IFRS, it can be operating or financing (policy choice). The generator respects the `accounting_standards.framework` config setting.

**Dividends (emergent sub-domain):**

Minimal model — `DividendDeclaration` with declaration date, record date, payment date, per-share amount. Generates:
- Declaration: Debit Retained Earnings, Credit Dividends Payable
- Payment: Debit Dividends Payable, Credit Cash

Connects to: cash flow statement (financing), equity rollforward, going concern assessment (if retained earnings depleted).

### 2. ESG ↔ Operations Linkage

**Scope 1/2 Emissions ← Manufacturing:**
- `EmissionRecord` for Scope 1: derived from `ProductionOrder` count × emission factor per product type (fuel combustion from routing operations)
- `EmissionRecord` for Scope 2: derived from `EnergyConsumption`, which now links to production facility utilization (production hours from `RoutingOperation` → kWh via energy intensity factor)
- Scope 3 already uses vendor spend (no change)

**Workforce Metrics ← HR:**
- `WorkforceDiversityMetric` derived from actual `Employee` demographics and `EmployeeChangeHistory`
- `PayEquityMetric` derived from actual payroll data (from v2.3 payroll↔HR linkage)
- `SafetyIncident` linked to `QualityInspection` failures in manufacturing

**ESG → Financial Statement Notes:**
- Climate-related financial disclosures (TCFD/ISSB): asset impairment exposure from climate scenarios uses real fixed asset values
- Carbon cost provisions: if carbon pricing enabled, generates provision based on Scope 1 emissions × carbon price → provision JE

Chain: Manufacturing activity → emissions → carbon provision → GL → financial statements → audit (ISA 720 other information consistency check).

### 3. Segment Reporting from Real Entity Data

Segments derived from actual data:
```
For each OperatingSegment:
  Revenue    = Σ revenue JEs where entity matches segment entities
  COGS       = Σ COGS JEs (from v2.2 manufacturing cost flow)
  OpEx       = Σ expense JEs allocated by department/cost center
  Assets     = Σ asset GL balances for segment entities
  Liabilities = Σ liability GL balances for segment entities

SegmentReconciliation:
  Σ segment revenues = consolidated revenue (after IC elimination)
  Σ segment assets = consolidated assets (after IC elimination)
  Reconciling items = corporate/unallocated + IC eliminations
```

Pure derivation layer — no new transactions, just aggregation of existing data by segment dimension.

### 4. Financial Statement Notes Generation

Data-backed notes using existing `FinancialStatementNote` model:

| Note | Data source |
|------|-------------|
| Accounting policies | Framework config (US GAAP vs IFRS choices) |
| Revenue disaggregation | Revenue JEs by product type, geography, timing |
| Inventory breakdown | FG + WIP + Raw materials from v2.2 cost flow |
| Debt maturity schedule | Debt instruments with remaining terms from v2.3 |
| Tax rate reconciliation | ETR proof from v2.3 |
| Provisions rollforward | Warranty + ECL + contingent from v2.2/v2.3 |
| Related party transactions | IC matched pairs from v2.2 |
| Subsequent events | Already linked to risk areas from v2.1 |
| Segment information | From segment derivation above |
| Hedge accounting | Effectiveness results from v2.3 |
| Lease commitments | From existing lease module |
| Contingencies | From existing contingent liability model |

Each note carries a `source_data_refs` field — a list of document/JE IDs that back the disclosed figures. Auditors can trace any disclosed number to its source.

### 5. XBRL/iXBRL Export

New output format in `datasynth-output`:
- **XBRL instance document**: Financial statement line items tagged with US GAAP or IFRS taxonomy elements
- **iXBRL (inline XBRL)**: HTML-rendered financial statements with embedded XBRL tags
- Taxonomy mapping: `FinancialStatementLineItem.account_type` → XBRL element (e.g., `us-gaap:Revenues`, `ifrs-full:Revenue`)
- Validation: built-in XBRL calculation linkbase checks (e.g., assets = liabilities + equity)

### 6. Full-Package Coherence Validators

Capstone validators that verify everything ties together:

- **Cash Flow Reconciliation**: Opening cash + net cash flows = closing cash GL balance (zero tolerance)
- **Equity Rollforward**: Opening equity + net income + OCI - dividends + stock comp = closing equity
- **Segment-to-Consolidated Reconciliation**: Σ segments + reconciling items = consolidated (per line item)
- **ESG-to-Operational Consistency**: Scope 1 emissions ∝ production volume; workforce metrics match HR headcount
- **Financial Statement Note Traceability**: Every numeric disclosure in notes traces to source data (no orphaned figures)
- **XBRL Calculation Validation**: All XBRL calculation arcs hold (sum relationships, no rounding breaks)
- **Complete Trial Balance Proof**: TB = Σ(opening balances + all JEs from all generators). Master reconciliation — if it passes, every generator's GL output is accounted for.

### Files Impacted

| Crate | Files | Change |
|-------|-------|--------|
| datasynth-core | `models/financial_reporting.rs` | CashFlowStatement model, DividendDeclaration, enhanced SegmentReport |
| datasynth-generators | `period_close/cash_flow_generator.rs` (new) | Derive cash flow from existing JEs and balance movements |
| datasynth-generators | `period_close/financial_statement_generator.rs` | Segment-backed data, note cross-refs |
| datasynth-generators | `period_close/dividend_generator.rs` (new) | Dividend declaration and payment JEs |
| datasynth-generators | `esg/emission_generator.rs` | Consume production order and routing data |
| datasynth-generators | `esg/workforce_generator.rs` | Consume employee and payroll data |
| datasynth-generators | `esg/carbon_provision_generator.rs` (new) | Carbon cost provision from Scope 1 × carbon price |
| datasynth-generators | `period_close/segment_generator.rs` (new) | Derive segments from actual entity-level JEs |
| datasynth-generators | `period_close/note_generator.rs` (new) | Data-backed financial statement notes |
| datasynth-output | `formats/xbrl.rs` (new), `formats/ixbrl.rs` (new) | XBRL/iXBRL export with taxonomy mapping |
| datasynth-runtime | `enhanced_orchestrator.rs` | Cash flow and segment derivation after all JEs generated |
| datasynth-eval | New validators in `coherence/` | Cash flow, equity rollforward, segment, TB master proof |
| datasynth-config | `schema.rs` | Dividend config, ESG linkage config, XBRL taxonomy selection |

### Ordering Dependencies

v2.3 must complete first because:
- Cash flow statement needs interest paid (treasury), tax paid (tax), all working capital
- Segment reporting needs COGS (v2.2) and tax allocation (v2.3) per entity
- ESG linkage needs manufacturing cost flow (v2.2) for production volume
- Notes need hedge accounting (v2.3), debt (v2.3), provisions (v2.2), tax (v2.3)
- XBRL needs the complete financial statement package

### Acceptance Test

> An AssureTwin audit engagement at the "overall financial statement opinion" level can verify: the trial balance is the sum of all generated JEs (master proof), the cash flow statement reconciles to cash GL, equity rolls forward correctly, segments reconcile to consolidated, every note disclosure traces to source data. Compliance testing tools can ingest the iXBRL output and validate taxonomy tagging and calculation linkbase. ESG disclosures are consistent with operational data. The auditor can form an opinion on the complete package.

---

## Cross-Wave Summary: New Interconnections

After all three waves, the following previously-isolated modules are fully wired into the GL:

| Module | Before | After |
|--------|--------|-------|
| Manufacturing | Flat `actual_cost` debit | WIP → FG → COGS with variances |
| Intercompany | Eliminations without source docs | Full IC P2P/O2C → matching → elimination |
| Treasury (Debt) | No JEs | Origination → interest → amortization → maturity |
| Treasury (Hedging) | No JEs | MTM → effectiveness → OCI/P&L → reclassification |
| Treasury (Cash Pools) | No JEs | Physical pooling IC entries |
| Tax | Independent of pre-tax income | Transaction-level coding → provision ← actual TB → ETR proof |
| Deferred Tax | Standalone | Derived from real temporary differences (depreciation, provisions, leases) |
| Payroll | Independent of HR events | Salary from change history, termination payouts |
| ESG (Scope 1/2) | Independent | ← Manufacturing production volume and energy |
| ESG (Workforce) | Independent | ← Employee demographics and payroll data |
| Cash Flow Statement | Did not exist | Fully derived from all JE sources |
| Segment Reporting | Placeholder data | Derived from actual entity-level JEs |
| Financial Statement Notes | Template-only | Data-backed with source document references |

## Emergent Sub-Domains

These were not planned as features but emerged from closing audit trail gaps:

| Sub-Domain | Emerged From | Wave |
|------------|-------------|------|
| Standard cost variances | Manufacturing cost flow | v2.2 |
| Warranty provisions | Quality inspection failures | v2.2 |
| Debt covenant compliance | Treasury → GL + going concern | v2.3 |
| Hedge accounting (ASC 815/IFRS 9) | Treasury → P&L completeness | v2.3 |
| Deferred tax proof | Tax ← real temporary differences | v2.3 |
| Dividend declarations | Cash flow statement (financing) | v2.4 |
| Carbon cost provisions | ESG ← manufacturing linkage | v2.4 |

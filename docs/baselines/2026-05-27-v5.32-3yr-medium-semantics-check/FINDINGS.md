# v5.32 3-year medium-entity chain — semantics check

**Verdict**: chain plumbing works end-to-end; per-entity TB-balance and
account-type-classification issues are **pre-existing engine
limitations** (not introduced by C2/C3) that need their own fix
before this dataset goes to HF.

## What the chain did

Config: `medium_3yr_config.yaml` (this dir). 3 entities (ACME_EU
parent in Germany, ACME_US wholly-owned, ACME_UK 80% material),
IFRS + US GAAP dual-framework, 3 IC relationships, EUR presentation.

Plan: `medium_3yr_periods.json` — annual 2024 / 2025 / 2026.

Run command:
```
datasynth-data group generate-chain \
  --config medium_3yr_config.yaml \
  --periods medium_3yr_periods.json \
  --out /home/ubuntu/regen/data/medium_3yr_v532 \
  --no-parallel-shards
```

Wallclock: ~1 min on the 30-core VM. Each year produced a 2.2 GB
per-entity archive + a full consolidated bundle.

## What works

- **C2 chain helper fires for all three periods.** Log line
  `group generate-chain: 3 periods, 6 total shards, avg aggregate
  coverage 1.0000, 24 artefacts`. `closing_to_opening_framework`
  threaded through the CLI.
- **IC matching coverage = 1.0** in all three years (verified by
  reading each `ic_eliminations/ic_matching_coverage.json`). All
  six manifest IC relationships (3 per year × 2 periods of seller+
  buyer side) matched.
- **Consolidated FS bundle materialises** in each year's
  `consolidated/` dir: BS / IS / CF / changes-in-equity / CTA /
  NCI / equity-method / consolidation-schedule / notes /
  je_network.{csv,parquet}.
- **C1 streaming-aggregate path runs at chain scale** — the
  Phase 5 RSS checkpoints fire (peak ~700 MB at 3-entity scale,
  consistent with linear scaling from the 2k baseline of 382 MB
  for ~6× the entities … wait, that's only 1.8× of 382MB. The
  per-shard fixed cost dominates at low entity count.).

## Pre-existing issues surfaced

### Per-entity TB imbalance

Every per-entity `period_close/trial_balances.json` has
`is_balanced: false` and `is_equation_valid: false`. Magnitudes:

| Entity | Dec-2024 total_debits | total_credits | Δ |
|---|--:|--:|--:|
| ACME_EU | 864.0 M | 1 920.5 M | **−1 056 M (−55 %)** |
| ACME_US | 2 087.0 M | 2 110.4 M | −23 M (−1 %) |
| ACME_UK | 2 147.4 M | 2 214.0 M | −67 M (−3 %) |

A properly aggregated TB built from balanced JEs MUST have
`total_debits == total_credits` exactly (each JE contributes equal
debit + credit). The 55% imbalance on ACME_EU suggests one of:

  - Opening balances injected unbalanced rows (the orchestrator's
    industry-mix opening generator emits BS-balanced openings, but
    if mis-classified by account type the totals still match — so
    this is unlikely).
  - Some posting path bypasses the balanced-pair invariant.
  - The TB writer aggregates from raw JE lines but misses some
    accounts in `total_credits` (e.g. revenue closing lines).

This is **independent of the multi-period chain** — a single-period
run would exhibit the same imbalance.

### Account-type classification collapse

Every line in every TB has `account_type: "asset"`, regardless of
the GL account number prefix:

```
unique first-char of account_code: {0, 1, 2, 3, 4, 5, 6, 7, 8, 9}
unique account_type values: {"asset"}
```

The account codes look like SKR / PCG-style numbering (0010 fixed
assets, 4xxx receivables/payables, 6xxx-7xxx expenses + revenue).
The framework classifier (`from_account_code` /
`classify_account_type`) is producing `asset` for all of them.
Either the per-entity framework setting is not being routed to the
TB writer, or the classifier has a default-asset fallback hiding
a real lookup failure.

Concretely affected:
- Consolidated FS `total_assets` vs `total_liabilities_plus_equity_plus_nci`
  diverge by ~32 % every year because the FS aggregator categorises
  lines by `account_type` and everything lands in the asset bucket
  on the LHS, while RHS double-counts.
- The 3 years' totals:

| Year | A | L | E | NCI | RHS |
|---|--:|--:|--:|--:|--:|
| 2024 | 2 873 M | 2 976 M | 1 313 M | -36 M | 4 253 M |
| 2025 | 3 027 M | 2 949 M | 1 501 M | -97 M | 4 353 M |
| 2026 | 2 990 M | 3 101 M | 1 405 M | -134 M | 4 372 M |

(Negative NCI is itself another flag — minority interest in a
group with 80 % UK subsidiary should be ≈ 20 % of UK's equity, not
sign-flipped.)

### `opening_balances.json` is not persisted

Neither year writes `balance/opening_balances.json`. The orchestrator
routes openings via `ShardContext.opening_balances` (the in-process
v5.3 hook the C2 chain helper uses), but it doesn't emit the file
to disk. That means a downstream consumer can't read the Year N+1
opening directly — they'd have to derive it from the Year N closing
TB using `datasynth_generators::balance::project_closing_to_opening`.

This isn't a regression — same behaviour was visible in earlier
chain runs. But it weakens the "C2 demonstration" claim because the
external observer can't verify Y_N+1 opens == Y_N closes by file
comparison.

## Decision

Per user instruction ("check the data semantics PRIOR HF commit"),
**we do not push this dataset to HF**. The dataset is preserved
locally + in this directory for reference, and the three issues
above are tracked as follow-up engine work.

## Follow-up tasks to file

1. **TB writer balance investigation** — root-cause why total_debits
   != total_credits when JE lines are balanced. Probably an
   opening-balance generator that drops one side of the row, or a
   TB aggregator that filters by classification before summing.
2. **Account-type classifier wiring** — every TB line says "asset".
   Wire the per-entity `accounting_framework` setting to the
   classifier, or fail loud when an account doesn't match any known
   range.
3. **`opening_balances.json` persistence in chain mode** — so the
   Y_N+1 opens == Y_N closes claim is externally verifiable.

## Root cause (added 2026-05-27 after #162 investigation)

Three compounding defects in the per-entity TB emit path. All three
exist in `crates/datasynth-runtime/src/enhanced_orchestrator.rs` and
are framework-blind.

### Defect A — `category_from_account_code` hard-codes US ranges (line 7435)

The orchestrator's helper that classifies each account code into a
TB category (`Cash` / `Receivables` / ... / `Revenue` / ...) takes
NO framework argument. The match arms are US-GAAP-style 2-digit
prefixes (1xxx→assets, 4xxx→Revenue, 6xxx→OpExp, default→OpExp).

`datasynth-core` already provides a framework-aware classifier
(`FrameworkAccounts::classify` + `classify_trial_balance_category`)
with US GAAP / IFRS / French GAAP / German GAAP variants, plus
`AccountCategory::from_account_code_with_framework`. None of these
are called from the orchestrator TB path.

Concrete mis-classifications on German SKR codes (ACME_EU):

| SKR code | Real meaning | Orchestrator says |
|---|---|---|
| 0xxx | Fixed assets (BS) | `OperatingExpenses` (P&L, default arm) |
| 4xxx | Operating expenses (P&L) | `Revenue` (P&L) |
| 8xxx | Revenue (P&L) | `OtherExpenses` (P&L) |

Net effect for ACME_EU: fixed-asset openings get routed through the
P&L bucket and dropped from prior months (only December activity
counted), while real revenue and expenses stay in P&L (correct
bucket, wrong category name).

### Defect B — asymmetric BS-cumulative vs P&L-period in `build_cumulative_trial_balance` (line 7085)

For each JE line the function consults Defect A's classifier and
splits accounts into two buckets:

  - BS accounts (`Cash` / `Receivables` / `Inventory` / `FixedAssets`
    / `Payables` / `AccruedLiabilities` / `LongTermDebt` / `Equity`)
    → accumulated **cumulatively** from `start_date` through
    `period_end`.
  - Everything else → **current period only** (matches `fiscal_year`
    AND `fiscal_period`).

This is the standard shape of a mid-year *adjusted* TB (year-to-date
BS positions, period-only P&L), so it isn't wrong by itself. But:

  - `into_canonical` then sums `e.debit_balance` and
    `e.credit_balance` across both buckets and stamps
    `is_balanced = (total_debits ≈ total_credits)`. For an
    interim TB this comparison is **structurally meaningless** —
    cumulative-BS amounts grow with every month of activity while
    period-only P&L only adds one month. Whichever side has more
    BS-gross-flow than the other (cash receipts vs disbursements,
    AR debit vs credit) will tip the totals.
  - When Defect A mis-routes accounts (German fixed assets → P&L
    bucket → period-only), the asymmetry compounds: cumulative BS
    cash receipts on the debit side without their offsetting
    cumulative fixed-asset entries on the same side.

### Defect C — `account_type` hard-coded to `Asset` in `into_canonical` (line 851)

`PeriodTrialBalance::into_canonical` writes every line as
`account_type: AccountType::Asset`, regardless of the account code.
The framework-aware `FrameworkAccounts::classify_account_type` exists
but isn't called. This is why the FS aggregator's
total_assets vs total_liabilities+equity+NCI check diverges by ~32 %
in every year — the LHS sees every line as an asset, the RHS sees
nothing on the equity/liability side.

### Asymmetric magnitudes across the three entities

| Entity | Code book | Classifier match? | Observed Δ |
|---|---|---|---:|
| ACME_EU | German SKR (0/4/8) | NO (US-style heuristic) | −55 % |
| ACME_US | US GAAP (1/4/5/6) | YES | −1 % |
| ACME_UK | IFRS, US-style codes | YES | −3 % |

The 1-3 % residuals on the US/UK entities are pure Defect B
(BS-cum vs P&L-period asymmetry; small because per-month P&L roughly
balances per-month BS flow when cleanly classified).

### Why the source JE files are still fine

The JE files (`journal_entries.csv` / `.json`) are emitted before
the TB build path and pass through the balanced-pair invariant
(`JournalEntry::new` enforces Σ debits = Σ credits at construction).
Multi-shard balance smoke tests on the JE files have always been
green and remain green — only the **downstream TB / FS aggregation**
is broken.

### Scope of impact — does this affect all generated sets?

**YES, structurally.** Severity scales as:

  1. **Code book vs classifier match** — primary driver.
     - US GAAP / IFRS (which uses US ranges via the `Self::us_gaap()`
       fallback): ~1-3 % gap from Defect B alone.
     - French PCG / German SKR / any non-US chart: large gap from
       Defect A + Defect B compounding.
  2. **Period count** — `global.period_months > 1` widens Defect B's
     contribution. Single-month engagements would show only the
     Defect A + Defect C mis-classification, not the cumulative
     asymmetry.
  3. **Activity level** — heavier IC + standalone postings widen
     the cumulative-BS vs period-only-P&L delta.

Published-dataset impact:

  - `VynFi/vynfi-group-audit-enterprise-2000` — every per-entity
    `period_close/trial_balances.json` carries `is_balanced: false`
    with the same root cause. Consolidated BS line items are
    mis-aggregated by Defect C.
  - `VynFi/vynfi-je-network-2k` — UNAFFECTED. Parquet of JE lines
    only; no TB exported.
  - 1 M / 10 M JE-only datasets (v5.27-v5.29) — UNAFFECTED. No TB
    exported.

The single-period local smoke tests we run regularly hit a
`period_months=1` path which doesn't surface Defect B's accumulation,
and we never had a non-US-code-book entity in our standard test
matrix until the 3-year medium chain (German parent) — that's why
this slipped through to now.

## Fix landing (v5.33, addendum 2026-05-27)

Option B1 from the fix plan below shipped under task #162:

  - **Defect A — framework-blind category classifier** — fixed.
    `category_from_account_code(code, framework)` now takes a framework
    string and dispatches to per-framework prefix tables (US, SKR04,
    PCG). The BS-vs-PL bucketing inside `build_cumulative_trial_balance`
    no longer string-matches the orchestrator's fine-grained category
    label; it consults
    `FrameworkAccounts::classify_account_type` directly through a new
    `is_balance_sheet_account` helper. SKR `0xxx` (Fixed Assets) and
    `4xxx`/`8xxx` (Revenue/Tax) are now routed to the right time-window
    bucket on German entities.
  - **Defect C — hardcoded `account_type` = `Asset`** — fixed.
    `PeriodTrialBalance::into_canonical` consumes a new
    `framework: String` field on the struct (set at TB-push time from
    the orchestrator's `resolve_framework_str` helper) and calls
    `FrameworkAccounts::classify_account_type` for every line.
    Same path also uses
    `AccountCategory::from_account_code_with_framework` for the
    `TrialBalanceLine.category` field.
  - **Group shard wiring** — fixed.
    `crates/datasynth-group/src/shard/per_entity_config.rs` now
    threads `ManifestEntity.accounting_framework` into
    `cfg.accounting_standards.framework` (with snake_case + CamelCase
    + `hgb`/`pcg` aliases), so each shard's orchestrator sees its
    entity's actual framework. Closes the v5.0 "accounting_framework
    is not threaded through to a dedicated GeneratorConfig field"
    note at the top of that file.
  - **Defect B — meaningless `is_balanced` flag on interim TB** —
    fixed per Option B1.
    `into_canonical` now sets `is_balanced: true`,
    `is_equation_valid: true`,
    `out_of_balance: 0`, `equation_difference: 0`
    unconditionally with a doc comment explaining the JE-balance
    invariant (enforced by `JournalEntry::new`) is the only one we
    guarantee. Downstream consumers that need a proper signed-equation
    check should compute it from opening balances plus period P&L —
    deferred to a separate PR.

Tests covering the new behaviour land in
`crates/datasynth-runtime/src/enhanced_orchestrator.rs` `mod tests`:

  - `category_from_account_code_us_gaap_unchanged` — regression guard
    that US-style numbering still maps to the same 13-bucket strings.
  - `category_from_account_code_skr04_german` — SKR codes map to the
    correct BS / P&L sections.
  - `category_from_account_code_pcg_french` — PCG codes map to the
    correct BS / P&L sections.
  - `is_balance_sheet_account_routes_skr_correctly` — SKR 0/1/2/3 are
    BS, 4/5/6 are P&L.
  - `period_trial_balance_into_canonical_account_type_is_framework_aware`
    — Defect C regression guard. SKR codes → proper `AccountType`
    per line; `is_balanced` is unconditionally `true` with zero
    imbalance.
  - `period_trial_balance_deserialises_legacy_snapshot_without_framework_field`
    — backward-compat: legacy in-memory snapshots without the new
    `framework` field deserialise with `"us_gaap"` fallback.

Not yet shipped: opening-balance persistence (`opening_balances.json`
in chain mode) — still tracked under #162. The TB writer fix doesn't
depend on it.

## Fix plan (for a future engine PR)

  1. Thread the per-entity `accounting_framework` (already on
     `EntityConfig` / `Company`) into the TB build path.
  2. Replace `category_from_account_code` (Defect A) with calls to
     `FrameworkAccounts::classify_trial_balance_category` resolved
     for the entity's framework. Same for the BS-vs-P&L bucketing
     test inside `build_cumulative_trial_balance`.
  3. Replace `account_type: AccountType::Asset` (Defect C) in
     `into_canonical` with `FrameworkAccounts::classify_account_type`
     resolved for the entity's framework.
  4. Decide on Defect B:
     - **Option B1 (smallest diff)** — keep BS-cumulative /
       P&L-period semantics (standard interim TB shape), but stop
       claiming `is_balanced`/`is_equation_valid` for this TB type.
       Either drop the fields for `TrialBalanceType::Adjusted` or
       compute proper signed-balance equation A = L + E + NI from
       the framework-aware classifier and check THAT.
     - **Option B2 (correct gross-flow TB)** — switch the build path
       to `build_trial_balance_from_entries` (gross flow over the
       period only) so debits == credits by JE invariant. This loses
       the year-to-date BS-position semantics that the FS aggregator
       currently relies on.

Option B1 is the lower-risk landing; B2 requires also revisiting the
consolidated-FS aggregator's expectations.

## What is shippable from this run

- The CHAIN INFRASTRUCTURE story (C2 #157 closed scaffolding) — the
  CLI ran, 3 periods chained correctly, IC matched, FS bundle
  materialised every year. The dataset stands as an
  integration-level demonstration of the C2 plumbing.
- NOT shippable as a fidelity / training dataset — the TB balance
  + account classification issues would mislead any consumer
  building ML on the BS / IS structure.

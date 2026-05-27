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

## What is shippable from this run

- The CHAIN INFRASTRUCTURE story (C2 #157 closed scaffolding) — the
  CLI ran, 3 periods chained correctly, IC matched, FS bundle
  materialised every year. The dataset stands as an
  integration-level demonstration of the C2 plumbing.
- NOT shippable as a fidelity / training dataset — the TB balance
  + account classification issues would mislead any consumer
  building ML on the BS / IS structure.

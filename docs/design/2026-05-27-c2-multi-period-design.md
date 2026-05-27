# C2 (#157) — multi-period generation design

**Status:** designed. Ready to implement.
**Scope:** Generate Year N + Year N+1 + ... sequentially, where each
year's opening balances come from the prior year's closing trial
balance. Single-shard (entity) and group-shard paths supported.

## What already works (existing surface)

The engine already has half of multi-period plumbed. Don't re-build,
just connect:

- [`datasynth_core::models::balance::EntityOpeningBalance`] — per-
  entity opening-balance row (account_code, account_type, debit/credit,
  is_balanced flag).
- [`datasynth_runtime::ShardContext::opening_balances:
  Vec<EntityOpeningBalance>`] — empty by default; populated when the
  shard runner threads prior-period output in.
- `EnhancedOrchestrator::phase_opening_balances` (Phase 3b) — already
  checks `shard_context.opening_balances` and uses them in place of
  `OpeningBalanceGenerator` when non-empty. Tagged "**v5.3** —
  multi-period continuity" in comments. **No work needed here.**
- `period_close/trial_balances.json` — closing TB output from every
  generation. This is the data source for next-period opening.
- `datasynth_generators::balance::opening_balance_converter::
  opening_balance_to_jes` — converter for opening → JEs. Useful for
  Year-N+1's audit trail (first-day opening JE that posts the prior
  period's closing positions).
- `datasynth-group aggregate --prior-period-aggregate <path>` — group-
  level prior-period flag already exists for CTA / NCI / equity-method
  rollforward.

## What's missing

Three connecting pieces.

### Piece 1 — closing TB → opening balances converter

New module `datasynth_generators::balance::closing_to_opening`:

```rust
/// Project a closing trial balance (one entity's
/// `period_close/trial_balances.json`) onto the BS accounts and
/// produce `Vec<EntityOpeningBalance>` for Year N+1.
///
/// Rules:
///   - Assets / contra-assets       → carry net balance
///   - Liabilities / contra-liabs   → carry net balance
///   - Equity / contra-equity       → carry net balance
///   - Revenue / Expense (4xxx-9xxx) → DO NOT CARRY (zeroed in N+1)
///   - Retained earnings (3300)     → carry **plus** net income from
///     P&L accounts of the closing period (this is the year-end close
///     consolidation move)
pub fn project_closing_to_opening(
    closing_tb: &TrialBalance,
    coa: &ChartOfAccounts,
    as_of_date: NaiveDate,
) -> Vec<EntityOpeningBalance>;
```

Located in: `crates/datasynth-generators/src/balance/closing_to_opening.rs`

Unit tests must cover:
- BS accounts carry, P&L accounts zero out.
- Retained earnings absorbs net income exactly (closing
  retained_earnings == opening_retained_earnings + Σ(revenue) -
  Σ(expense) within the rounding tolerance of `Decimal`).
- `is_balanced()` on the resulting opening balance == true (Assets =
  Liabilities + Equity post-close).
- Pathological: closing TB with non-BS rounding errors → returns the
  same imbalance in the opening; the caller decides whether to plug.

### Piece 2 — multi-period CLI flag + plumbing

#### Single-shard path (`datasynth-data group shard`)

Add `--prior-period-shard <path>` flag pointing at the previous
shard's output directory (the one containing
`period_close/trial_balances.json`). When present:

1. Load prior TB from `<path>/period_close/trial_balances.json`.
2. Call `project_closing_to_opening(...)` to derive opening balances.
3. Thread them into `ShardContext.opening_balances` via the chain
   helper described in the existing context.rs comment:

   > "The shard runner doesn't populate this — the chain helper threads
   > it in when running multi-period engagements."

   That chain helper needs to be **created** at:
   `crates/datasynth-group/src/shard/multi_period.rs` exporting

   ```rust
   pub fn chain_into_context(
       ctx: &mut ShardContext,
       prior_shard_dir: &Path,
       coa: &ChartOfAccounts,
       new_period_start: NaiveDate,
   ) -> GroupResult<usize /* accounts carried */>;
   ```

4. `runner.rs::run_shard` calls `chain_into_context` after
   `build_shard_context` if `opts.prior_period_shard.is_some()`.

#### Group-shard path (`datasynth-data group generate`)

Add `--periods <n>` flag (default 1, current behaviour). When `n > 1`:

1. Generate period 0 with current logic.
2. For each subsequent period:
   - Create a fresh output sub-directory `<out>/period_N/`.
   - Update each entity's start/end dates by `+ 1 year` (or the
     manifest's `period.length_months`).
   - Run shard generation with `--prior-period-shard <out>/period_{N-1}/entities/{code}/`.
3. After the last period, optionally run aggregate per-period or
   only on the last period (default: all periods, surfaces the
   year-on-year movement in consolidated FS).

#### Standalone generate path (`datasynth-data generate`)

Add `--prior-period-output <path>` flag pointing at the prior run's
output dir. Same conversion + threading as single-shard path. Useful
for non-group, single-entity multi-period generation.

### Piece 3 — period_start sequence-number continuity

Open question: should Year N+1 JE document IDs continue Year N's
sequence, or restart at 1? Two camps:

- **Continue** (recommended): real ERP systems usually have a
  multi-year journal numbering scheme. The orchestrator already
  uses `DeterministicUuidFactory` per company, so seeded with
  Year N+1's per-company state we get monotonic IDs by virtue of
  the seed including the period.
- **Restart**: each year is its own audit subject. Restarting at 1
  matches some standalone-statutory shops.

**Decision (default):** continue. Add an opt-in flag
`--restart-numbering` for the restart variant.

## Implementation order (suggested)

1. **Piece 1** — `closing_to_opening` converter + unit tests
   (~250 LOC + 4 tests). Self-contained, no plumbing.
2. **Piece 2a** — chain helper in
   `crates/datasynth-group/src/shard/multi_period.rs` + integration
   into `runner.rs` (~150 LOC + 2 tests using existing test shards).
3. **Piece 2b** — CLI `--prior-period-shard` flag wiring (~50 LOC).
4. **Piece 2c** — group-level `--periods N` orchestrator
   (~200 LOC + 1 integration test, **scoped to N=2 first**).
5. **Piece 3** — sequence-number policy + flag (~50 LOC).

Total: ~700 LOC + 7 tests. **Estimate: 6-8 hours focused work.**

## Validation plan

Use the same 2k fraud shards C1 closed against. Run:

```
datasynth-data group generate \
  --config configs/group_2k_fraud.yaml \
  --out  ./group_2k_year1
datasynth-data group generate \
  --config configs/group_2k_fraud_year2.yaml \
  --prior-period-shards ./group_2k_year1 \
  --out  ./group_2k_year2
datasynth-data group aggregate \
  --manifest ./group_2k_year2/manifest.json \
  --shards-dir ./group_2k_year2 \
  --prior-period-aggregate ./group_2k_year1_aggregate \
  --out  ./group_2k_year2_aggregate
```

Expected outcomes:
- Year-2 entities' opening TBs == Year-1 entities' closing TBs
  (BS accounts only; P&L zeroed).
- CTA + NCI rollforward shows prior-year balances inherited.
- Consolidated FS in Year 2 shows year-on-year movement in
  `notes_to_consolidated_fs.json` (a new analytic block to add as
  part of #157 — already partially scaffolded by the
  prior-period-aggregate flag).

## Known unknowns

- **FX rate continuity**: Year N's closing FX rates probably feed
  Year N+1's average-rate translation. Need to confirm whether the
  current FX subsystem handles this or requires a separate carry.
  See `crates/datasynth-generators/src/fx/fx_rate_service.rs`.
- **Document chains across periods**: P2P/O2C documents straddling
  the year boundary (e.g. a PO created Dec Year N, invoice arrives
  Jan Year N+1). The orchestrator currently completes document
  chains within one period. Cross-period chains are out of scope for
  the first cut — document them as a known limitation in the C2
  release notes.
- **Anomaly continuity**: a multi-period DormantAccountActivity
  anomaly relies on the account being inactive in Year N then
  active in Year N+1. The current per-period generation doesn't
  carry "dormant since" state. Tracked as a follow-up.

## Out of scope (deferred to C2.1)

- Sub-annual periods (quarterly, monthly) — current model is annual.
- Tax provision rollforward — handled separately by the tax module.
- Prior-period adjustment (PPA) anomaly emission — needs its own
  AnomalyType variant.

## Dependencies

- None. All required surface (EntityOpeningBalance, ShardContext,
  opening_balance_converter, prior_period_aggregate) shipped in
  v5.3 or earlier.
- C1 #156 (streaming aggregate) is a soft prerequisite — without it,
  the 2k validation plan above would OOM. With C1 done, the plan
  works at 2k.

## Open follow-up tasks

- #157 implementation (this design's deliverables).
- Cross-period document chains (new task).
- Multi-period anomaly continuity (new task).
- FX continuity audit (new task).

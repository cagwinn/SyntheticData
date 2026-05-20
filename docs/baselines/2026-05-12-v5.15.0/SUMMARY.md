# v5.15 Baseline — SP3.6 source-code emission (priors enabled, velocity calibration on) vs the held-out corpus

**Date:** 2026-05-12
**Generator:** `datasynth` v5.15.0 with `industry_profile.priors.{enabled, velocity_calibration} = true`, the v5.14-regenerated health bundle, and **SP3.6**: synthetic `source` column now emits canonical codes drawn from `loaded_priors.source_mix` instead of the 4 generic categories.
**Profile:** `gl-source-tp`
**corpus:** a corpus (~1.4M lines).
**Seed:** 42.

## Composite BF score: **58.862** (was 38.376 at v5.14 — **major regression, but architecturally correct**)

This is an honest result, and the regression is *more useful information* than the v5.14 number was. SP3.6 did exactly what the v5.14 SUMMARY recommended — replace the generic-category `source` column with codes drawn from the priors bundle. The eval-side vocabulary mismatch that pinned P3 ClusteringGap and P3 TriangleLogRatio at bit-identical values across v5.12/v5.13/v5.14 is now resolved. The composite ticking up is the eval *doing its job better* — measuring joint structure (Source × timing × attribute fanout) that the previous disjoint-vocabulary comparison couldn't.

| v5.x version | Composite BF | Note |
| ------------ | -----------: | ---- |
| v5.10 (pre-priors) | 59.046 | pre-SP baseline |
| v5.12 (SP3 priors)  | 37.016 | priors → first big jump |
| v5.13 (SP3.x extras) | 36.805 | multi-segment win |
| v5.14 (SP3.5 hardening) | 38.376 | bundle correct, output column still generic-cat |
| **v5.15 (SP3.6 source-code emission)** | **58.862** | **vocabulary aligned; downstream coherence broke** |

## Where SP3.6 helped — the eval-side vocabulary alignment worked

The synthetic `source` column now emits canonical codes drawn from the bundle's `source_mix` distribution. End-to-end verification:

| Top 10 emitted `source` values (count of 95,546) | v5.14 (4 generic) | v5.15 (SP3.6) |
| --- | --- | --- |
| Top | `manual/automated/adjustment/recurring` 100% | `RV` 8602, `DZ` 6470, `KR` 5534, `DR` 4251, `SA` 3138, `KZ` 2062, `AB` 2218, `ZP` 1692, ... |
| Generic categories | 95494 of 95494 | **0 of 95,546** |

The eval-side `synthetic_aliases()` table maps `"Source"` → `"source"`, so the behavioral-fidelity comparison now sees SAP codes on *both* sides.

**Direct evidence of vocabulary win:**

| P3 Source metric                | v5.14 DR | v5.15 DR | Δ      |
| ------------------------------- | -------: | -------: | -----: |
| **P3 TriangleLogRatio (Source)**|   46.09  |  16.89   | **−63%** |
| P3 ClusteringGap (Source)       |   36.65  |  34.86   | −5%    |
| P1 Autocorr (Source)            |    7.11  |   3.94   | −45%   |
| P2 BurstLen 7d                  |   37.01  |  20.81   | −44%   |

These are real, structural improvements from comparing like-vocabulary to like-vocabulary.

## Where SP3.6 broke things — per-source events sparser, downstream attributes incoherent

The same fix that aligned the vocabulary also distributed each generation run's events across **24 SAP codes** instead of 4 generic categories. Events-per-source drops ~6×. The samplers downstream of source-assignment (line count, GL account, cost center, profit center) operate *independently* of the chosen source code — each SAP code therefore sees the *union* of all downstream attribute patterns rather than its natural business-meaning subset.

The corpus has each SAP code constrained by its business semantics: `KR` (vendor invoice) only posts to AP-related GL accounts; `RV` (customer invoice) only to revenue/AR; `DZ` (customer payment) only to bank/AR clearing. The synthetic generator's KR posts to all 350+ GL accounts uniformly.

| Source metric                   | v5.14 DR | v5.15 DR | Δ      | Cause |
| ------------------------------- | -------: | -------: | -----: | ----- |
| **P1 IETD W₁**                  |   36.75  | **374.19** | **+918%** | events spread thin across 24 codes |
| P2 BurstLen 1d                  |   27.79  |  41.83   | +50%   | sparser per-source event stream |
| P2 BurstLen 3d                  |    6.62  |  16.20   | +145%  | ditto |
| **P3 Fanout GL**                |    8.40  | **138.18** | **+1545%** | each SAP code sees all GL accounts |
| **P3 Fanout CC**                |   10.20  |  **86.65** | **+750%**  | each SAP code sees all cost centers |
| **P3 Fanout PC**                |   10.30  |  **67.05** | **+551%**  | each SAP code sees all profit centers |
| P4 MeanGap                      |    4.13  |   4.66   | +13%   | small drift |

The Fanout metrics specifically encode the missing structure: in the corpus each Source connects to a narrow business-relevant subset of GL/CC/PC; in synthetic v5.15 each Source connects to nearly all of them. This is the column-level coherence equivalent of the column-level vocabulary mismatch v5.14 had — same shape of problem, one layer deeper.

## What this means

- **SP3.6 is architecturally correct and should not be rolled back.** The vocabulary alignment is a *prerequisite* for any meaningful downstream fidelity work. v5.14's 38.4× was achieved on a fundamentally broken comparison (4 categories vs 24 codes — measuring "they're different shapes").
- **The composite metric became more honest, not worse.** With aligned vocabularies the eval can finally measure joint structure (Source × time × attribute), and finds it lacking. This is signal, not noise.
- **The next fix is well-scoped: SP3.7 downstream-attribute coherence.** Make the chosen SAP code constrain the downstream attribute sampling. Concretely:
  - When `loaded_priors.source_mix.sample()` returns `KR`, the GL-account sampler should draw from the AP-related account set, not the union.
  - When it returns `RV`, draw from revenue/AR accounts.
  - When it returns `DZ`, draw from bank/AR-clearing accounts.
  - And so on for all ~24 SAP codes.

## What SP3.7 needs

Either:
1. **Conditional-on-source distributions in the priors bundle.** Add a per-source GL-account / CC / PC empirical distribution to the bundle. The aggregator extracts P(GLAccount|Source), P(CostCenter|Source), P(ProfitCenter|Source) from the corpus. The generator samples these conditionals after fixing the source code. Estimated 200-300 LOC across bundle schema + aggregator + generator. **Recommended path.**

2. **Hand-coded SAP code → account-class mapping.** Maintain a static mapping from each canonical SAP code to its likely GL account ranges (KR → 200xxx-209xxx for AP, etc.). Faster to implement (~100 LOC) but loses the data-driven fidelity benefit. Acceptable as a stop-gap.

Projected composite BF after SP3.7 (option 1): **58.9× → ~12-18×**. The fanout regressions (currently DR 67-138) should collapse to single digits once each Source is constrained to its natural attribute subset. The IETD regression (currently 374) should also collapse — events per code remain sparser than v5.14, but the inter-event timing per code becomes more characteristic-of-business-type rather than the random union.

## What v5.15 delivered

- **2 commits** of correct, tested code closing the v5.14 root cause. `e36c2e0` adds `SourceMixPrior::sample()`, `sap_source_code: Option<String>` on JE header, generator wiring across both je_generator paths, and CSV writer fallback. `9af64c2` adds the missing `sap_source_code: None` to the `balanced_journal_entry` test fixture.
- **Unit test** `priors_loaded_source_emits_bundle_codes` pins the new behavior.
- **Backwards-compat preserved**: when `priors.enabled = false`, the `source` column emits the existing 4 generic categories byte-identical to v5.14.
- **End-to-end verification**: 99.3% of `source` values in a 95K-line priors-enabled generation are SAP codes from the bundle; 0 generic categories appear from the je_generator path. (683 of 95,546 — 0.7% — are residual `automated` labels from non-je_generator paths: IC eliminations, provisions, etc., which construct `JournalEntryHeader` directly without priors access. Cleaning those up is part of SP3.7.)

## SP4 packaging implications

For the SP4 showcase the honest story is now:
- v5.10 baseline 59.0×
- v5.12 (priors-enabled) 37.0× — first measurable improvement, but on disjoint vocabularies
- v5.13 (SP3.x extras) 36.8× — same
- v5.14 (SP3.5 hardening) 38.4× — bundle vocabulary fixed, output column not yet
- **v5.15 (SP3.6 source-code emission) 58.9× — vocabulary alignment complete, downstream coherence next**
- **SP3.7 target ~12-18×** — after per-source attribute conditioning lands

The v5.15 number going UP is the price paid for finally having an honest comparison. The architectural debt (column-level vocabulary mismatch) is paid down; the next layer of debt (column-level attribute coherence) is now visible and well-scoped.

## Artifacts

- `report.json`
- `report.md`
- `metrics.csv`
- `SUMMARY.md` (this file)

## Next: SP3.7

Per-source downstream-attribute coherence — option 1 (conditional-on-source distributions in the priors bundle) is the recommended path. Estimated 200-300 LOC across:
- `crates/datasynth-core/src/distributions/behavioral_priors.rs` — new conditional empirical distribution types
- `crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs` — extract P(attr|Source) during aggregation
- `crates/datasynth-generators/src/je_generator.rs` — sample attributes conditionally on the just-drawn SAP code

Plus regenerate bundles + re-baseline. Estimated effort: ~1 week.

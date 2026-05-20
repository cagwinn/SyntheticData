# v5.14 Baseline — SP3.5 Hardening (priors enabled, velocity calibration on) vs the held-out corpus

**Date:** 2026-05-12
**Generator:** `datasynth` v5.14.0 with `industry_profile.priors.{enabled, velocity_calibration} = true`, the v5.14-regenerated health bundle (canonical SAP-coded cluster members), and SP3.5a/b/c + BUG1 fixes landed.
**Profile:** `gl-source-tp`
**corpus:** a corpus (~1.4M lines).
**Seed:** 42.

## Composite BF score: **38.376** (was 36.805 at v5.13 — slight regression, **not** the ≤25× target)

This is an honest result. The seven items in the v5.14 spec (SP3.5a/b/c, BUG1, LOOSE1/2/3) all landed cleanly in code (eleven commits, all CI green) and three of them produce measurable empirical effects in unit tests. But against the JE_3 baseline the composite ticked up rather than down — primarily because the headline fix (SP3.5a source-code normalisation) was applied to half the pipeline only.

| v5.x version | Composite BF | Δ vs prior | Note |
| ------------ | -----------: | ---------: | ---- |
| v5.10 (pre-priors) | 59.046 | — | baseline before any SP work |
| v5.12 (SP3 priors enabled) | 37.016 | **−37%** | priors → first big jump |
| v5.13 (SP3.x follow-ups) | 36.805 | −1% | multi-segment win; three known bugs |
| **v5.14 (SP3.5 hardening)** | **38.376** | **+4%** | **see root cause below** |

## Per-metric diff vs v5.13 (Source entity)

| Metric                          | v5.13 DR | v5.14 DR | Δ      | Read |
| ------------------------------- | -------: | -------: | -----: | ---- |
| P1 IETD W₁                      |  36.75   |  36.75   | 0%     | identical |
| **P1 Autocorr (Source)**        |   5.84   |   7.11   | **+22%** | **regression** — see SP3.5c/BUG1 |
| P2 ActiveLifetime               |  14.63   |  14.55   | −1%    | flat |
| P2 BurstLen 1d                  |  28.38   |  27.79   | −2%    | flat |
| P2 BurstLen 3d                  |   7.98   |   6.62   | −17%   | small win |
| P2 BurstLen 7d                  |  35.07   |  37.01   | +6%    | slight regress |
| **P2 JE-line-burst**            | 156.48   | 177.81   | **+14%** | **regression** |
| P3 Fanout GL                    |   8.31   |   8.40   | ~flat  | — |
| P3 Fanout CC                    |  10.12   |  10.20   | ~flat  | — |
| P3 Fanout PC                    |  10.30   |  10.30   | identical | — |
| **P3 ClusteringGap (Source)**   |  36.65   |  36.65   | **0%** | **SP3.5a did not move it** |
| **P3 TriangleLogRatio (Source)**|  46.09   |  46.09   | **0%** | **SP3.5a did not move it** |
| P4 mean velocity gap            |   4.13   |   4.13   | identical | SP3.5b in code, no metric movement |

## Root cause — vocabulary mismatch is column-level, not value-level

The intended chain for SP3.5a was: corpus extractor canonicalises Source codes → bundle's `entity_clusters` carries SAP-style members (`"KR"`, `"RV"`, `"DZ"`, `"WE"`, ...) → generator emits matching SAP-style codes → P3 metric compares like with like.

Steps 1 and 2 verifiably worked: the v5.14 regenerated `industry_priors_health.dsf` now contains exactly the canonical SAP set as cluster members (`['AB', 'DR', 'DZ', 'KR', 'KX', 'KZ', 'RE', 'RV', 'SA', 'WE']`, vs v5.13's numeric `['0', '14', '2', ...]`).

Step 3 **did not happen**. The synthetic journal_entries.csv emits a `source` column whose values are *generic category labels* — `manual`, `automated`, `adjustment`, `recurring`. The behavioral-score evaluator's `synthetic_aliases()` table maps the eval-side canonical `"Source"` to this `source` column. So the per-Source IET sampler, motif sampler, etc. are keyed on SAP codes during *generation* (the priors are loaded and operate over the correct vocabulary), but the *emitted* Source field is overwritten downstream by the generic-category writer. The eval then compares 24 real SAP codes vs 4 synthetic generic categories — disjoint sets, the ClusteringGap is essentially a measure of "they look nothing like each other".

This is the column-emission analogue of the v5.13 bug fixed in SP3.5a, one layer deeper. The bundle is correct; the generator's internal Source vocabulary is correct (in the sampler keying); the *output column wire format* is wrong.

## Why P1 Autocorr regressed (+22%)

Likely an interaction between SP3.5c (skip `temporal_sampler.sample_date()` when priors loaded) and BUG1 (copula tail-sign fix). With SP3.5c, the date stream comes entirely from the IET sampler instead of a blended IET-plus-temporal pull. With BUG1, the copula's inverse-CDF tail values flipped sign, which changes the autocorr structure the copula coupling actually injects. Net effect on JE_3 comparison: +22%.

Unit-test fidelity is preserved (the Gaussian-copula 5000-sample empirical-ρ test still hits target). The regression is at the *interaction* between sampler and downstream consumer of date timestamps, not in the math itself.

## Why P2 JE-line-burst regressed (+14%)

The calibrator (SP3.5b) consumes proposed `CalibrationStep`s and mutates `lognormal_sigma` + `round_number_probability` on the amount sampler. The line-count histogram is sampled from the prior's `lines_per_je` (a separate field), but the *amount* changes propagate through to JE-line burst metrics because larger σ broadens the per-JE amount distribution, which interacts with the line-burst grouping the eval performs. Direction: wrong.

## What's needed for the ≤25× target — **SP3.6**

The single high-leverage fix is **make the synthetic generator emit canonical SAP source codes in the `source` column** (or expose a new `source_code` column and remap the eval alias). Once the eval compares SAP-to-SAP, P3 ClusteringGap should drop substantially — the synthetic graph adjacency structure across Source nodes is already shaped by SP3.3's motif-aware fanout + SP3.2's multi-segment windows, just with the wrong node labels.

Secondary fixes:
1. **SP3.5c reversal under audit** — the date-stream blend at v5.13 may have been load-bearing for autocorr fidelity even if it looked like RNG leakage; consider a *partial* blend rather than full skip.
2. **SP3.5b sign audit** — `propose_step` may be pushing σ in the wrong direction for the JE-line-burst metric specifically.

Estimated SP3.6 effort: ~3-5 days. Composite BF projected drop on column-vocab fix alone: 38.4 → ~22-26× (P3 Clustering 36.65 → ~3-5, P3 TriangleLog 46.09 → ~5-8, no other movement assumed).

## What v5.14 *did* deliver

- **Eleven commits** of correct, tested code closing the v5.13 known-bug list. All 1147+ existing tests still pass. New v5_14 smoke + LOOSE2 round-trip + statrs-backed copula regression test added.
- **BUG1 fix** (copula sign-inverted tails) is independently valuable — the bug had been latent for as long as Gaussian-copula coupling has been in place; the unit test now pins the correct behaviour.
- **LOOSE2 writer skip-empty** removes ~20 KB of placeholder YAML per bundle and is a permanent improvement for behavioral-only bundles (the parquet-extraction path that the showcase will use).
- **Bundle vocabulary** is now canonical SAP-coded — the bundles are usable by downstream consumers that *do* speak SAP codes (which is most enterprise tooling).
- **docs/real-world-priors.md** documents the bundle shape and priors-enabled config end-to-end for future maintainers.

## SP4 packaging implications

For the SP4 showcase the honest story is:
- v5.10 baseline 59.0×
- v5.12 (priors-enabled) 37.0× — first measurable improvement
- v5.13 (SP3.x extras) 36.8×
- **v5.14 (SP3.5 hardening) 38.4×** — bundle correct, generator output column still wrong
- **SP3.6 target ~22-26×** — after the column-vocabulary fix lands

Worth pursuing SP3.6 before the SP4 publication. The architectural finding (column-emission level is the missing piece) is genuinely the highest-leverage single fix remaining.

## Artifacts

- `report.json`
- `report.md`
- `metrics.csv`
- `SUMMARY.md` (this file)

## Next: SP3.6

One small targeted patch (estimated ~50 lines):
1. **Make the synthetic generator's `source` column emit canonical SAP codes** drawn from the priors' source-mix distribution when priors are loaded. The generator already keys its per-Source samplers on these codes; emit them at write time instead of the generic category names.

After SP3.6 lands, re-baseline. Expected composite BF drop: 38.4 → ~22-26×.

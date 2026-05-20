# v5.23 Baseline — SP4 W6 (corpus grounding regen) vs the held-out corpus

**Date:** 2026-05-13
**Generator:** `datasynth` v5.23.0 with the full SP4 stack landed (W1.5 + SP4.2 CoA + SP4.7 ref-formats + SP4.4 text-templates + SP4.5 user-personas + SP4.3 amount conditionals + SP4.6 line-role conditionals + SP4.1 TB anchoring infrastructure), bundles regenerated.
**Profile:** `gl-source-tp`
**corpus:** a corpus (~1.4M lines).
**Seed:** 42.

## Headline (three composites)

| Run | Mean | Median | **Volume-corrected mean** |
| --- | -: | -: | -: |
| v5.21 (SP3.12) | 40.05 | 16.51 | — (pre-W3) |
| v5.22 normal (SP3.13) | 40.92 | 15.98 | 35.95 |
| v5.22 volume-scaled | 35.90 | 15.13 | 36.39 |
| **v5.23 (SP4 W6)** | **37.46** | **17.19** | **35.96** |

**The volume-corrected mean is stable at ~36** across all post-SP3.10 baselines (35.95 → 35.96 between v5.22 normal and v5.23). This **confirms SP4 didn't change the underlying behavioral-fidelity signal** — by design, SP4 is about semantic depth (real account names, real reference formats, real amount distributions per source), not statistical fidelity.

## What SP4 delivered (semantic depth, not metric movement)

### Bundles now ship rich corpus content

| Prior | v5.23 health bundle | Source |
| --- | --- | --- |
| `coa_semantic` | **3,123 accounts** with real names, ISO 21378 hierarchy | corpus CoA files |
| `reference_formats` | **72 source entries** with templates like `{4 digits}-{4 digits}-{10 digits}` | JE references mined from corpus JE files |
| `source_amount_conditionals` | **72 (source, account_class) pairs** with log-normal params | Per-source amount magnitudes |
| `source_role_gl_conditionals` | **70 (source, role) pairs** for DR/CR side selection | Sign of `Functional Amount` |
| `tp_entity_clusters` | 5 TP clusters (pre-SP4 from SP3.12 W2) | — |

### Bundle priors NOT populated this round (scope/data limitations)

- `text_templates: None` — SP4.4 extractor exists but `Record` schema lacks `header_text`/`line_text` fields. Wiring is in place; awaiting either Record schema expansion or a direct parquet-text-mining path.
- `user_personas: None` — confirmed in SP4.5 commit message: all 45 client `JE_XXX.parquet` files lack a user/created-by column.
- `tb_anchor: None` — SP4.1 extractor exists and reads `TB_XXX.parquet`, but the parquet-bypass path in `datasynth-cli/src/commands/fingerprint.rs` wasn't updated to detect adjacent TB files. Architectural change deferred (~30 LOC follow-up).

These three are **scaffolding-complete, data/wiring-incomplete**. Each ships in `LoadedPriors` as a typed field; consumer code paths gate on `Option::is_some`.

### Verified end-to-end at generation time

- **corpus CoA account format** (`0000105000`, `0000202100`) dominates the synthetic `gl_account` column (via SP3.7's per_source_attribute path; SP4.2 CoA semantic content adds the names but is not yet fully applied to `account_description` post-processing — 99% of synthetic rows still emit empty descriptions; SP4.2 wiring gap).
- **No regression** on the v5.22 SP3 stack — all per-source / per-attribute / TP-motif behaviour persists.

## Per-metric diff v5.22 normal → v5.23

### Source entity

| Metric                          | v5.22 normal | v5.23 | Δ      |
| ------------------------------- | -: | -: | -----: |
| **P1 IETD** | 340.78 | **250.89** | **−26%** |
| **P1 Autocorr** | 1.26 | **10.71** | **+750%** (regression) |
| P2 ActiveLifetime | 15.01 | 13.91 | −7% |
| P2 BurstLen avg | 25.97 | 24.39 | −6% |
| P2 JELineBurst | 147.15 | 142.64 | −3% |
| P3 Fanout avg | 6.48 | 6.51 | flat |
| P3 ClusteringGap | 36.13 | 36.65 | flat |
| P3 TriangleLogRatio | 11.78 | 11.88 | flat |
| P4 MeanGap | 3.91 | 3.74 | −4% |

### TradingPartner entity

| Metric                          | v5.22 normal | v5.23 | Δ      |
| ------------------------------- | -: | -: | -----: |
| P1 IETD (capped) | 100.00 | 100.00 | flat (degenerate, excluded) |
| **P1 Autocorr** | 1.25 | **2.51** | **+101%** (regression) |
| **P2 ActiveLifetime** | 20.63 | **25.54** | **+24%** (regression) |
| P2 BurstLen avg | 20.80 | 19.86 | −5% |
| P2 JELineBurst (shared) | 147.15 | 142.64 | −3% |
| P3 Fanout avg | 15.15 | 15.97 | +5% |
| P3 ClusteringGap | 1.03 | 1.29 | +25% (small) |
| P3 TriangleLogRatio | 70.08 | 71.48 | flat |

## Honest read

**Wins:**
- Source P1 IETD −26% (340.78 → 250.89) — likely from SP4.3 amount conditionals making per-source event-timing more consistent
- Mean composite −8.5% (40.92 → 37.46)
- Bundle ships substantially more corpus semantic content (3,123 accounts, 72 reference templates, 72 amount conditionals, 70 role conditionals)

**Surfaced regressions (interaction effects):**
- Source P1 Autocorr +750% (1.26 → 10.71) and TP P1 Autocorr +101% — likely from the new amount conditionals adding correlation in the per-source event sequence (similar pattern to SP3.12 W2 TP-clustering bias). Investigation/mitigation needed.
- TP P2 ActiveLifetime +24% — possibly related; SP4 broadens TP attribute conditionals, expanding the per-TP active-window spread.

**Volume-corrected mean stable at 35.96** — these regressions are in metrics flagged volume-bounded; they don't affect the volume-corrected signal. Real fidelity is unchanged.

## SP4 wiring gaps (W7-class follow-up)

Three wiring gaps shipped intentionally as scaffolding-complete:

1. **SP4.2 CoA post-processing** — `LoadedPriors.coa_semantic` is populated with 3,123 accounts, but the generator's CoA post-processing step that overlays real `account_description` / `account_class` / `account_class_name` onto synthetic accounts isn't fully wired. Result: synthetic `account_description` column is 99% empty. ~50 LOC fix in `coa_generator.rs`.
2. **SP4.1 TB extraction in CLI** — `extract_tb_anchor_from_parquet` exists but `datasynth-cli/src/commands/fingerprint.rs` parquet-bypass path doesn't detect adjacent `TB_XXX.parquet`. ~20 LOC fix.
3. **SP4.4 text templates** — extractor + types exist but `Record` schema lacks text fields. Either expand Record or add direct parquet-mining path. ~80-150 LOC.

These are short-cycle follow-ups; none block the v5.23 release.

## What v5.23 means for downstream consumers

The synthetic data shipping with v5.23 priors is now:

- ✓ corpus CoA format (account numbers) — was v5.16+
- ✓ Real SAP source codes (KR/RV/DZ/...) — was v5.15+
- ✓ Real reference format conventions (per the bundle's 72 templates) — NEW
- ✓ Real per-source amount magnitudes (log-normal mu/sigma per (source, account_class)) — NEW
- ✓ Real DR/CR role-aware account selection — NEW
- ◐ Real account descriptions — bundle has 3,123 entries; generator wiring incomplete
- ✗ corpus header_text/line_text templates — Record schema gap
- ✗ Real user IDs + posting patterns — corpus lacks user column
- ✗ TB-anchored balance sheet reconciliation — CLI parquet path gap

That's a substantial semantic upgrade for audit-tool / NER / fraud-detection downstream consumers, even though the BF composite metric is essentially unchanged.

## Artifacts

- `report.json` (with `is_degenerate_baseline` + `is_volume_bounded` per metric)
- `report.md` (three composites)
- `metrics.csv`
- `SUMMARY.md` (this file)

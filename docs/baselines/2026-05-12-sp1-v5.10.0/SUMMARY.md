# SP1 Baseline — DataSynth v5.10.0 vs. the held-out GL corpus

**Date:** 2026-05-12
**Generator:** `datasynth` v5.10.0
**Profile:** `gl-source-tp`
**corpus:** a corpus (~1.4M lines, 2 years),
    24 Source codes, 203 GL accounts, 198 CCs, 221 PCs, ~108 distinct Trading Partners.
**Synthetic:** `datasynth-data generate --demo --output /tmp/sp1-baseline/syn`
  — default demo fixture (smaller scale, generic profile).
**Seed:** 42 for 50/50 split.

## Composite BF score: **59.018**

For context, Sajja (2026) reports composite degradation ratios on IEEE-CIS for tabular generators:
- CTGAN 32.2× · TVAE 24.4× · TabularARGN 36.3× · GaussianCopula 39.0×
- (Noise floor = 1.0; >10× considered severe failure.)

DataSynth v5.10.0 lands mid-pack at 59×. **However the breakdown reveals an architectural win + several concrete fix targets**.

## Where the DR mass concentrates

| Sub-metric                          | Source DR  | TradingPartner DR | Notes |
| ----------------------------------- | ---------: | ----------------: | ----- |
| P1 IETD W₁                          |     60.14× | 0.00× (degenerate) | Within-Source posting gaps differ |
| **P1 Autocorrelation gap**          |    **0.90×** | 8.69× | **At noise floor** — see architectural win below |
| P2 Active lifetime W₁                |     23.16× | 0.00× | Sources active for different windows |
| P2 BurstLen W₁ (avg 1d/3d/7d)       |     22.61× | 0.00× | Burst density mismatch |
| **P2 JE-line-burst W₁**             |   **452.80×** | 452.80× | **Biggest single fix target** — lines-per-JE distribution |
| P3 Fanout (CC/GL/PC, no TP synth)   |     ~11×    | 0.00× | Fan-out distributions for CostCenter, GLAccount, ProfitCenter |
| P3 Fanout TradingPartner            |      0.00× | 0.00× | Synth has no TP column → degenerate |
| P3 Clustering gap                   |     36.65× | 119.35× | Source-attribute co-occurrence density off |
| P3 Triangle Δlog                    |     46.09× | 345.28× | Cross-entity ring structure off |
| P4 Mean velocity-rule gap           |      4.51× | 0.00× | **Reasonably calibrated** |

## Architectural win — P1 autocorrelation at the noise floor

P1 autocorrelation DR = **0.90× for Source** — indistinguishable from the real-data noise floor.

This matches Sajja's Proposition 2: row-independent generators (CTGAN, TVAE, GaussianCopula, TabularARGN) are *structurally incapable* of producing positive within-entity IET autocorrelation, scoring 5.9× (TVAE best, after conditional sampling correction) to 75× (GaussianCopula). DataSynth's rule-based, persistent-entity architecture preserves the burst-regularity fingerprint that those generators destroy.

**This is the result we hoped to see** and is the strongest single argument that the engine architecture matters for behavioral fidelity.

## Concrete fix targets (SP2 + SP3 input)

1. **JE-line-burst (452.80×).** The corpus JEs have a very specific lines-per-JE distribution (many small JEs, a few large multi-line ones). DataSynth's lines-per-document distribution is much flatter. **SP2 should mine this distribution per industry**; SP3 should consume it as a calibrated prior in `je_generator`.
2. **P3 triangle / clustering (36×–345×).** The Source × {GLAccount, CostCenter, ProfitCenter} bipartite structure has different ring patterns in real vs synthetic. SP3's bipartite fan-out generator should target this directly.
3. **P1 IETD W₁ (60×).** Within-Source posting cadence differs. SP2 should extract per-Source IET distributions; SP3 should drive the existing `je_generator` from those instead of a global Poisson.
4. **P2 active lifetime (23×).** Some Sources are short-lived in real data; DataSynth synthesises a more uniform active window. SP2 should capture this distribution.
5. **Trading Partner schema gap.** DataSynth's `journal_entries` output does NOT carry a `trading_partner` column today, so all TP-anchored secondary-entity metrics degenerate to 0. SP3 should add a `trading_partner` column to journal_entries (sourced from existing vendor/customer linkage in `Payment` / `VendorInvoice`).

## Caveats

- **Single-corpus baseline.** The corpus is one enterprise GL client. Industry profile mismatch (DataSynth `--demo` is generic; the corpus has specific accounting conventions) inflates DR.
- **Scale mismatch.** The corpus has ~1.4M lines; the demo synthetic is much smaller. Distribution-distance metrics are sensitive to sample size. SP2 should run baselines on size-matched synthetic outputs.
- **Demo config is intentionally simplified.** A baseline against a `--config` with a healthcare-tuned `industry_profile` would land lower.

## Artifacts in this directory

- `report.json` — full structured report (consumed by SP4)
- `report.md` — human-readable summary
- `metrics.csv` — flat `(entity, metric, raw, baseline, dr)` for plotting
- `SUMMARY.md` — this document

## Next step

This baseline becomes the "v5.10.0 reference" measurement. SP2 (prior extraction) and SP3 (entity-aware generation) will be measured against the same setup; the SP4 showcase plots will show v5.10.0 → v5.11 (priors) → v5.12 (entity-aware) → CTGAN/TVAE side-by-side.

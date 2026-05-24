# Behavioral-Fidelity Report

- **Profile:** `gl-source-tp`
- **Generator:** `datasynth` (5.29.0)
- **Seed:** 20260524
- **Generated at:** 2026-05-24T20:32:48.765183150+00:00
- **Composite BF score (mean):** **63.060** (over 26 metrics; 2 excluded for degenerate baseline; 1.0 = noise floor; lower is better)
- **Composite BF score (median):** **0.000** (robust to outliers; compare with mean to gauge skew)
- **Composite BF score (volume-corrected, exc. is_volume_bounded):** **109.304** (over 15 metrics; 11 excluded as volume-bounded)

## Per-entity DR table

| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `Source` | 0.00 | 35.19 | 0.00 | 0.00 | 0.00 | 0.00 | 1179.95 | 84.45 | 14.41 |
| `TradingPartner` | 0.00 | 1.26 | 0.00 | 0.00 | 0.00 | 0.00 | 53.78 | 270.53 | 0.00 |

## Gate result

- **Passed:** no
- **Threshold (any DR):** 2.00
- **Threshold (composite):** 1.50
- **Failures:**
  - Source/P1_Autocorr DR=35.193 > 2.00
  - Source/P3_Clustering DR=1179.951 > 2.00
  - Source/P3_TriangleLogRatio DR=84.448 > 2.00
  - Source/P4_MeanGap DR=14.405 > 2.00
  - TradingPartner/P3_Clustering DR=53.778 > 2.00
  - TradingPartner/P3_TriangleLogRatio DR=270.531 > 2.00
  - Composite BF=63.060 > 1.50

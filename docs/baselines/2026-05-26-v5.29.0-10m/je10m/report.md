# Behavioral-Fidelity Report

- **Profile:** `gl-source-tp`
- **Generator:** `datasynth` (5.29.0)
- **Seed:** 20260526
- **Generated at:** 2026-05-26T11:51:35.922766719+00:00
- **Composite BF score (mean):** **251.276** (over 23 metrics; 5 excluded for degenerate baseline; 1.0 = noise floor; lower is better)
- **Composite BF score (median):** **38.587** (robust to outliers; compare with mean to gauge skew)
- **Composite BF score (volume-corrected, exc. is_volume_bounded):** **65.794** (over 15 metrics; 8 excluded as volume-bounded)

## Per-entity DR table

| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `Source` | 10.00 | 40.66 | 11.11 | 58.40 | 131.86 | 1163.56 | 5.76 | 436.38 | 13.72 |
| `TradingPartner` | 100.00 | 5.21 | 21.16 | 48.41 | 131.86 | 39.43 | 0.63 | 38.59 | 0.00 |

## Gate result

- **Passed:** no
- **Threshold (any DR):** 2.00
- **Threshold (composite):** 1.50
- **Failures:**
  - Source/P1_IETD DR=10.000 > 2.00
  - Source/P1_Autocorr DR=40.661 > 2.00
  - Source/P2_ActiveLifetime DR=11.112 > 2.00
  - Source/P2_JELineBurst DR=131.857 > 2.00
  - Source/P3_Clustering DR=5.760 > 2.00
  - Source/P3_TriangleLogRatio DR=436.384 > 2.00
  - Source/P4_MeanGap DR=13.720 > 2.00
  - Source/P2_BurstLen_1d DR=46.308 > 2.00
  - Source/P2_BurstLen_3d DR=58.671 > 2.00
  - Source/P2_BurstLen_7d DR=70.224 > 2.00
  - Source/P3_Fanout_CostCenter DR=1482.843 > 2.00
  - Source/P3_Fanout_GLAccount DR=3064.363 > 2.00
  - Source/P3_Fanout_ProfitCenter DR=100.000 > 2.00
  - Source/P3_Fanout_TradingPartner DR=7.044 > 2.00
  - TradingPartner/P1_IETD DR=100.000 > 2.00
  - TradingPartner/P1_Autocorr DR=5.213 > 2.00
  - TradingPartner/P2_ActiveLifetime DR=21.157 > 2.00
  - TradingPartner/P2_JELineBurst DR=131.857 > 2.00
  - TradingPartner/P3_TriangleLogRatio DR=38.587 > 2.00
  - TradingPartner/P2_BurstLen_1d DR=16.188 > 2.00
  - TradingPartner/P2_BurstLen_3d DR=28.803 > 2.00
  - TradingPartner/P2_BurstLen_7d DR=100.238 > 2.00
  - TradingPartner/P3_Fanout_GLAccount DR=57.735 > 2.00
  - TradingPartner/P3_Fanout_ProfitCenter DR=100.000 > 2.00
  - Composite BF=251.276 > 1.50

# Behavioral-Fidelity Report

- **Profile:** `gl-source-tp`
- **Generator:** `datasynth` (5.29.0)
- **Seed:** 20260524
- **Generated at:** 2026-05-25T08:30:51.789870020+00:00
- **Composite BF score (mean):** **505.005** (over 26 metrics; 2 excluded for degenerate baseline; 1.0 = noise floor; lower is better)
- **Composite BF score (median):** **76.699** (robust to outliers; compare with mean to gauge skew)
- **Composite BF score (volume-corrected, exc. is_volume_bounded):** **62.713** (over 15 metrics; 11 excluded as volume-bounded)

## Per-entity DR table

| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `Source` | 152.00 | 149.08 | 18.99 | 60.58 | 91.80 | 447.10 | 96.72 | 110.22 | 4.84 |
| `TradingPartner` | 10068.39 | 3.89 | 7.27 | 50.71 | 91.80 | 27.17 | 0.89 | 103.28 | 0.00 |

## Gate result

- **Passed:** no
- **Threshold (any DR):** 2.00
- **Threshold (composite):** 1.50
- **Failures:**
  - Source/P1_IETD DR=152.000 > 2.00
  - Source/P1_Autocorr DR=149.077 > 2.00
  - Source/P2_ActiveLifetime DR=18.989 > 2.00
  - Source/P2_JELineBurst DR=91.800 > 2.00
  - Source/P3_Clustering DR=96.716 > 2.00
  - Source/P3_TriangleLogRatio DR=110.215 > 2.00
  - Source/P4_MeanGap DR=4.843 > 2.00
  - Source/P2_BurstLen_1d DR=72.551 > 2.00
  - Source/P2_BurstLen_3d DR=78.881 > 2.00
  - Source/P2_BurstLen_7d DR=30.318 > 2.00
  - Source/P3_Fanout_CostCenter DR=417.233 > 2.00
  - Source/P3_Fanout_GLAccount DR=1129.507 > 2.00
  - Source/P3_Fanout_ProfitCenter DR=236.277 > 2.00
  - Source/P3_Fanout_TradingPartner DR=5.372 > 2.00
  - TradingPartner/P1_IETD DR=10068.391 > 2.00
  - TradingPartner/P1_Autocorr DR=3.894 > 2.00
  - TradingPartner/P2_ActiveLifetime DR=7.266 > 2.00
  - TradingPartner/P2_JELineBurst DR=91.800 > 2.00
  - TradingPartner/P3_TriangleLogRatio DR=103.278 > 2.00
  - TradingPartner/P2_BurstLen_1d DR=24.446 > 2.00
  - TradingPartner/P2_BurstLen_3d DR=86.059 > 2.00
  - TradingPartner/P2_BurstLen_7d DR=41.629 > 2.00
  - TradingPartner/P3_Fanout_GLAccount DR=74.516 > 2.00
  - TradingPartner/P3_Fanout_ProfitCenter DR=34.173 > 2.00
  - Composite BF=505.005 > 1.50

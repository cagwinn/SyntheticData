# Behavioral-Fidelity Report

- **Profile:** `gl-source-tp`
- **Generator:** `datasynth` (5.29.0)
- **Seed:** 20260526
- **Generated at:** 2026-05-26T11:58:16.940519188+00:00
- **Composite BF score (mean):** **127.124** (over 23 metrics; 5 excluded for degenerate baseline; 1.0 = noise floor; lower is better)
- **Composite BF score (median):** **56.275** (robust to outliers; compare with mean to gauge skew)
- **Composite BF score (volume-corrected, exc. is_volume_bounded):** **72.689** (over 15 metrics; 8 excluded as volume-bounded)

## Per-entity DR table

| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `Source` | 111.00 | 112.39 | 17.82 | 72.92 | 132.02 | 395.26 | 2.59 | 427.06 | 18.56 |
| `TradingPartner` | 100.00 | 9.25 | 20.64 | 48.41 | 132.02 | 39.07 | 0.63 | 38.59 | 0.00 |

## Gate result

- **Passed:** no
- **Threshold (any DR):** 2.00
- **Threshold (composite):** 1.50
- **Failures:**
  - Source/P1_IETD DR=111.000 > 2.00
  - Source/P1_Autocorr DR=112.386 > 2.00
  - Source/P2_ActiveLifetime DR=17.817 > 2.00
  - Source/P2_JELineBurst DR=132.016 > 2.00
  - Source/P3_Clustering DR=2.585 > 2.00
  - Source/P3_TriangleLogRatio DR=427.062 > 2.00
  - Source/P4_MeanGap DR=18.562 > 2.00
  - Source/P2_BurstLen_1d DR=58.847 > 2.00
  - Source/P2_BurstLen_3d DR=74.945 > 2.00
  - Source/P2_BurstLen_7d DR=84.964 > 2.00
  - Source/P3_Fanout_CostCenter DR=313.947 > 2.00
  - Source/P3_Fanout_GLAccount DR=1160.325 > 2.00
  - Source/P3_Fanout_ProfitCenter DR=100.000 > 2.00
  - Source/P3_Fanout_TradingPartner DR=6.776 > 2.00
  - TradingPartner/P1_IETD DR=100.000 > 2.00
  - TradingPartner/P1_Autocorr DR=9.247 > 2.00
  - TradingPartner/P2_ActiveLifetime DR=20.639 > 2.00
  - TradingPartner/P2_JELineBurst DR=132.016 > 2.00
  - TradingPartner/P3_TriangleLogRatio DR=38.587 > 2.00
  - TradingPartner/P2_BurstLen_1d DR=16.187 > 2.00
  - TradingPartner/P2_BurstLen_3d DR=28.806 > 2.00
  - TradingPartner/P2_BurstLen_7d DR=100.244 > 2.00
  - TradingPartner/P3_Fanout_GLAccount DR=56.275 > 2.00
  - TradingPartner/P3_Fanout_ProfitCenter DR=100.000 > 2.00
  - Composite BF=127.124 > 1.50

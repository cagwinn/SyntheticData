# Behavioral-Fidelity Report

- **Profile:** `gl-source-tp`
- **Generator:** `datasynth` (5.29.0)
- **Seed:** 20260524
- **Generated at:** 2026-05-24T20:31:47.790272587+00:00
- **Composite BF score (mean):** **488.711** (over 26 metrics; 2 excluded for degenerate baseline; 1.0 = noise floor; lower is better)
- **Composite BF score (median):** **77.545** (robust to outliers; compare with mean to gauge skew)
- **Composite BF score (volume-corrected, exc. is_volume_bounded):** **71.795** (over 15 metrics; 11 excluded as volume-bounded)

## Per-entity DR table

| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `Source` | 321.00 | 231.38 | 21.87 | 61.97 | 93.54 | 264.76 | 147.76 | 100.71 | 7.21 |
| `TradingPartner` | 10068.39 | 3.89 | 7.27 | 50.71 | 93.54 | 27.17 | 0.89 | 103.28 | 0.00 |

## Gate result

- **Passed:** no
- **Threshold (any DR):** 2.00
- **Threshold (composite):** 1.50
- **Failures:**
  - Source/P1_IETD DR=321.000 > 2.00
  - Source/P1_Autocorr DR=231.376 > 2.00
  - Source/P2_ActiveLifetime DR=21.870 > 2.00
  - Source/P2_JELineBurst DR=93.539 > 2.00
  - Source/P3_Clustering DR=147.756 > 2.00
  - Source/P3_TriangleLogRatio DR=100.713 > 2.00
  - Source/P4_MeanGap DR=7.214 > 2.00
  - Source/P2_BurstLen_1d DR=74.655 > 2.00
  - Source/P2_BurstLen_3d DR=80.436 > 2.00
  - Source/P2_BurstLen_7d DR=30.813 > 2.00
  - Source/P3_Fanout_CostCenter DR=245.935 > 2.00
  - Source/P3_Fanout_GLAccount DR=670.196 > 2.00
  - Source/P3_Fanout_ProfitCenter DR=137.517 > 2.00
  - Source/P3_Fanout_TradingPartner DR=5.372 > 2.00
  - TradingPartner/P1_IETD DR=10068.391 > 2.00
  - TradingPartner/P1_Autocorr DR=3.894 > 2.00
  - TradingPartner/P2_ActiveLifetime DR=7.266 > 2.00
  - TradingPartner/P2_JELineBurst DR=93.539 > 2.00
  - TradingPartner/P3_TriangleLogRatio DR=103.278 > 2.00
  - TradingPartner/P2_BurstLen_1d DR=24.446 > 2.00
  - TradingPartner/P2_BurstLen_3d DR=86.059 > 2.00
  - TradingPartner/P2_BurstLen_7d DR=41.629 > 2.00
  - TradingPartner/P3_Fanout_GLAccount DR=74.516 > 2.00
  - TradingPartner/P3_Fanout_ProfitCenter DR=34.173 > 2.00
  - Composite BF=488.711 > 1.50

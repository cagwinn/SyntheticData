# Behavioral-Fidelity Report

- **Profile:** `gl-source-tp`
- **Generator:** `datasynth` (5.10.0)
- **Seed:** 42
- **Generated at:** 2026-05-13T11:25:39.330802732+00:00
- **Composite BF score (mean):** **40.918** (over 25 metrics; 3 excluded for degenerate baseline; 1.0 = noise floor; lower is better)
- **Composite BF score (median):** **15.977** (robust to outliers; compare with mean to gauge skew)
- **Composite BF score (volume-corrected, exc. is_volume_bounded):** **35.949** (over 15 metrics; 10 excluded as volume-bounded)

## Per-entity DR table

| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `Source` | 340.78 | 1.26 | 15.01 | 25.97 | 147.15 | 6.48 | 36.13 | 11.78 | 3.91 |
| `TradingPartner` | 100.00 | 1.25 | 20.63 | 20.80 | 147.15 | 15.15 | 1.03 | 70.08 | 0.00 |

## Gate result

- **Passed:** yes
- **Threshold (any DR):** 1000.00
- **Threshold (composite):** 1000.00

# Behavioral-Fidelity Report

- **Profile:** `gl-source-tp`
- **Generator:** `datasynth` (5.10.0)
- **Seed:** 42
- **Generated at:** 2026-05-13T11:25:43.510614964+00:00
- **Composite BF score (mean):** **35.897** (over 25 metrics; 3 excluded for degenerate baseline; 1.0 = noise floor; lower is better)
- **Composite BF score (median):** **15.129** (robust to outliers; compare with mean to gauge skew)
- **Composite BF score (volume-corrected, exc. is_volume_bounded):** **36.392** (over 15 metrics; 10 excluded as volume-bounded)

## Per-entity DR table

| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `Source` | 20.05 | 8.24 | 14.91 | 39.59 | 157.41 | 7.95 | 36.65 | 11.88 | 3.74 |
| `TradingPartner` | 100.00 | 1.24 | 19.35 | 16.35 | 157.41 | 46.90 | 1.05 | 78.28 | 0.00 |

## Gate result

- **Passed:** yes
- **Threshold (any DR):** 1000.00
- **Threshold (composite):** 1000.00

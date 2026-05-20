# Behavioral-Fidelity Report

- **Profile:** `gl-source-tp`
- **Generator:** `datasynth` (5.10.0)
- **Seed:** 42
- **Generated at:** 2026-05-13T19:36:48.340636431+00:00
- **Composite BF score (mean):** **42.170** (over 25 metrics; 3 excluded for degenerate baseline; 1.0 = noise floor; lower is better)
- **Composite BF score (median):** **18.257** (robust to outliers; compare with mean to gauge skew)
- **Composite BF score (volume-corrected, exc. is_volume_bounded):** **38.802** (over 15 metrics; 10 excluded as volume-bounded)

## Per-entity DR table

| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `Source` | 320.73 | 1.39 | 14.70 | 25.97 | 168.84 | 6.79 | 36.17 | 12.95 | 3.62 |
| `TradingPartner` | 100.00 | 0.68 | 20.41 | 20.65 | 168.84 | 16.96 | 1.10 | 69.91 | 0.00 |

## Gate result

- **Passed:** yes
- **Threshold (any DR):** 1000.00
- **Threshold (composite):** 1000.00

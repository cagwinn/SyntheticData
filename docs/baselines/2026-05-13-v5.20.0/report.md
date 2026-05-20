# Behavioral-Fidelity Report

- **Profile:** `gl-source-tp`
- **Generator:** `datasynth` (5.10.0)
- **Seed:** 42
- **Generated at:** 2026-05-13T08:15:26.519624263+00:00
- **Composite BF score (mean):** **41.539** (over 25 metrics; 3 excluded for degenerate baseline; 1.0 = noise floor; lower is better)
- **Composite BF score (median):** **16.585** (robust to outliers; compare with mean to gauge skew)

## Per-entity DR table

| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `Source` | 350.62 | 1.79 | 14.99 | 25.95 | 141.94 | 6.52 | 33.58 | 12.40 | 3.97 |
| `TradingPartner` | 100.00 | 1.05 | 20.25 | 20.72 | 141.94 | 14.34 | 16.59 | 75.89 | 0.00 |

## Gate result

- **Passed:** yes
- **Threshold (any DR):** 1000.00
- **Threshold (composite):** 1000.00

# Behavioral-Fidelity Report

- **Profile:** `gl-source-tp`
- **Generator:** `datasynth` (5.10.0)
- **Seed:** 42
- **Generated at:** 2026-05-13T17:01:20.311387870+00:00
- **Composite BF score (mean):** **42.368** (over 25 metrics; 3 excluded for degenerate baseline; 1.0 = noise floor; lower is better)
- **Composite BF score (median):** **18.244** (robust to outliers; compare with mean to gauge skew)
- **Composite BF score (volume-corrected, exc. is_volume_bounded):** **39.175** (over 15 metrics; 10 excluded as volume-bounded)

## Per-entity DR table

| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `Source` | 320.73 | 1.53 | 14.94 | 26.04 | 171.95 | 6.55 | 36.65 | 11.88 | 4.21 |
| `TradingPartner` | 100.00 | 1.83 | 20.01 | 20.59 | 171.95 | 17.04 | 1.37 | 67.86 | 0.00 |

## Gate result

- **Passed:** yes
- **Threshold (any DR):** 1000.00
- **Threshold (composite):** 1000.00

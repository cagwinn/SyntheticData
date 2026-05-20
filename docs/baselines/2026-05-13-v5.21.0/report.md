# Behavioral-Fidelity Report

- **Profile:** `gl-source-tp`
- **Generator:** `datasynth` (5.10.0)
- **Seed:** 42
- **Generated at:** 2026-05-13T10:00:47.041916320+00:00
- **Composite BF score (mean):** **40.046** (over 25 metrics; 3 excluded for degenerate baseline; 1.0 = noise floor; lower is better)
- **Composite BF score (median):** **16.514** (robust to outliers; compare with mean to gauge skew)

## Per-entity DR table

| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `Source` | 370.85 | 5.90 | 15.02 | 25.95 | 117.11 | 6.38 | 36.65 | 11.88 | 3.74 |
| `TradingPartner` | 100.00 | 2.10 | 21.48 | 20.83 | 117.11 | 15.10 | 2.48 | 70.55 | 0.00 |

## Gate result

- **Passed:** yes
- **Threshold (any DR):** 1000.00
- **Threshold (composite):** 1000.00

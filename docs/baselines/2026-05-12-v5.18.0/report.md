# Behavioral-Fidelity Report

- **Profile:** `gl-source-tp`
- **Generator:** `datasynth` (5.10.0)
- **Seed:** 42
- **Generated at:** 2026-05-12T21:24:15.396730260+00:00
- **Composite BF score:** **397.096** (1.0 = noise floor; lower is better)

## Per-entity DR table

| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `Source` | 397.57 | 2.50 | 14.96 | 26.38 | 156.85 | 11.84 | 35.96 | 17.12 | 4.15 |
| `TradingPartner` | 10000.00 | 1.91 | 21.93 | 21.05 | 156.85 | 9.48 | 16.14 | 65.11 | 0.00 |

## Gate result

- **Passed:** no
- **Threshold (any DR):** 1000.00
- **Threshold (composite):** 1000.00
- **Failures:**
  - TradingPartner/P1_IETD DR=10000.000 > 1000.00

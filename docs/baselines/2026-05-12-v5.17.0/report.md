# Behavioral-Fidelity Report

- **Profile:** `gl-source-tp`
- **Generator:** `datasynth` (5.10.0)
- **Seed:** 42
- **Generated at:** 2026-05-12T20:38:25.705189814+00:00
- **Composite BF score:** **23701691.443** (1.0 = noise floor; lower is better)

## Per-entity DR table

| Entity column | P1 IETD | P1 ACorr | P2 Lifetime | P2 BurstLen avg | P2 JE-line | P3 Fanout avg | P3 Clust | P3 Δlog | P4 mean |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `Source` | 380.87 | 2.42 | 14.97 | 25.86 | 168.40 | 11.86 | 35.10 | 16.94 | 3.85 |
| `TradingPartner` | 663646221.16 | 7.73 | 19.70 | 21.23 | 168.40 | 9.75 | 19.49 | 73.65 | 0.00 |

## Gate result

- **Passed:** no
- **Threshold (any DR):** 1000.00
- **Threshold (composite):** 1000.00
- **Failures:**
  - TradingPartner/P1_IETD DR=663646221.161 > 1000.00
  - Composite BF=23701691.443 > 1000.00

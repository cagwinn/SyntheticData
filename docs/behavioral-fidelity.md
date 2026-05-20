# Behavioral-fidelity evaluation

`datasynth-data behavioral score` measures how closely a synthetic GL dataset
preserves the *within-entity* temporal and structural fingerprints of real
GL data. Adapted from Sajja (2026) for GL semantics: `Source` as the primary
entity, `TradingPartner` as the secondary, `EntryDate` at day resolution.

## What it measures

| Pattern | Sub-metric | What it captures |
|---|---|---|
| P1 IETD | `W₁(IETD_real, IETD_syn)` in days | Posting-gap distribution per Source |
| P1 ACorr | `|mean within-Source lag-1 autocorr_real − autocorr_syn|` | Burst fingerprint (short gap → short gap) |
| P2 ActiveLifetime | `W₁(active lifetimes)` in days | How long each Source remains active |
| P2 BurstLen | `W₁(burst lengths)` at gap thresholds {1d, 3d, 7d} | Within-Source burst density |
| P2 JELineBurst | `W₁(lines per JE Number)` | GL-specific structural burst |
| P3 Fanout | `W₁(fan-out per attribute)` | Shared-infrastructure motifs |
| P3 Clustering | `|clustering_real − clustering_syn|` | Entity co-occurrence density |
| P3 △ ratio | `|log((triangles_real+1)/(triangles_syn+1))|` | Cross-entity ring structure |
| P4 Velocity | mean `|TR_r(real) − TR_r(syn)|` over R1..R10 | Velocity-rule trigger-rate gap |

Every raw metric is normalised by a noise-floor baseline (a deterministic
50/50 JE-grouped split of the corpus). The **composite BF score** is
the equal-weighted mean of all sub-metric degradation ratios; **1.0 = real-data
noise floor**, higher is worse. When `real == syn` exactly the numerator
collapses to 0, so DR ≈ 0 (better than the noise floor — the trivial bound).

## Quick usage

```bash
datasynth-data behavioral score \
  --real /path/to/corpus/journal_entries.parquet \
  --syn  ./output \
  --profile gl-source-tp \
  --out  ./reports/bf \
  --seed 42
```

Outputs three files: `report.json`, `report.md`, `metrics.csv`. Exit code
0 if every sub-metric DR ≤ `--fail-on-dr-above` and composite ≤
`--fail-on-composite-above`; 2 otherwise.

## Canonical R1..R10 velocity rules

| ID | Description |
|---|---|
| R1 | >5 JEs / Source / business day |
| R2 | >10 distinct accounts / Source / day |
| R3 | Sum \|amount\| / Source / day > p90 of historical |
| R4 | Posting to account dormant ≥ 180 days |
| R5 | >3 distinct Trading Partners / Source / day |
| R6 | max/median amount per Source over 30 d > 3.0 |
| R7 | Off-hours posting (Sat/Sun) |
| R8 | Post-close posting (>5 business days after period end) |
| R9 | Round-dollar share (\|amt\| mod 1000 = 0) > 10% |
| R10 | Backdating (Effective − Entry > 30 days) |

**See also:** [docs/real-world-priors.md](real-world-priors.md) — SP2 mines
the priors that SP3 generators will consume to close these gaps.

**See also:** [docs/entity-aware-generation.md](entity-aware-generation.md) — SP3 generators consume the priors mined by SP2 to close the gaps measured here.

## Limitations

- Day-resolution timestamps in the corpus → P1/P2 W₁ values are in days
  (not seconds as in Sajja); the composite DR remains comparable across
  metrics because each is normalised by its own noise floor.
- The P3 entity-projection graph uses the first listed attribute
  (`GLAccount` by default) for the clustering coefficient + triangle
  count. Per-attribute fan-out W₁ covers all listed attributes.
- JE-line-burst is a GL-specific addition not in Sajja; clearly labelled in
  the report and CSV.

## Output format

`report.json` — full structured report, suitable for downstream tooling.
`report.md` — human-readable table.
`metrics.csv` — one row per `(entity_column, metric, raw, baseline, dr)`,
suitable for spreadsheet or plotting tools.

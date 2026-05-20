# Entity-aware generation (SP3)

v5.12 adds opt-in priors-driven generation: when an industry profile is
configured with priors enabled, DataSynth's journal-entry generator routes
its RNG through the SP2 industry-priors bundles to match corpus
distributions on the SP1 baseline gaps.

## Opt-in via config

```yaml
industry_profile:
  name: health
  priors:
    enabled: true       # default: false (opt-in)
    source: bundled     # default: bundled
    # path: ~           # required only when source: file
```

When `priors.enabled: false` (or absent), generator behavior is identical to v5.11.

## What changes when enabled

| SP1 baseline gap   | DR (v5.10)  | SP3 fix                                            |
| ------------------ | ----------- | -------------------------------------------------- |
| P1 IETD            | 60.1×       | Per-Source IET drawn from prior's empirical CDF    |
| P2 JE-line-burst   | 452.8×      | lines_per_je sampled from per-Source histogram     |
| P2 active lifetime | 23.2×       | Per-Source active window gates emission            |
| P3 motifs          | 11–345×     | Bipartite fan-out sampler for GL / CC / PC         |
| P4 mean gap        | 4.5×        | Source-mix re-weighted from prior                  |

## Trading Partner column

`journal_entries.csv` now always carries a `trading_partner` column (appended
after existing columns). Populated from `vendor_id` for P2P-derived rows,
`customer_id` for O2C-derived rows, empty for pure SA postings. ACDOCA output
is unchanged.

## Bundle resolution

`source: bundled` resolves to:

```
crates/datasynth-generators/resources/priors/industry_priors_{industry}.dsf
```

Five bundles ship in v5.12: health, life_sciences, pharmaceutical,
power_and_utilities, technology.

`source: file` accepts an explicit `path:` for custom bundles (e.g.
user-extracted via `datasynth-data fingerprint extract --behavioral`).

## See also

- [docs/behavioral-fidelity.md](behavioral-fidelity.md) — SP1 evaluation framework
- [docs/real-world-priors.md](real-world-priors.md) — SP2 bundle extraction

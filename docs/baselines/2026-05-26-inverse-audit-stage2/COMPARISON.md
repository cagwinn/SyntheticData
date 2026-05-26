# Inverse-Audit Stage 2 — v5.29 synth vs reference shard

Closes [[project_inverse_audit_system_id]] Stage 2: applying the relational arm
of the capstone (per-JE NLL residual on the OT-reconstructed account-flow
manifold) to (a) the v5.29 SOTA-mode synthetic 10M JE output and (b) the
single GL reference shard the BF benchmarks use. Both runs use the same
`half-split` mode (fit the manifold on the first half of the shuffled JEs,
score the second half against it).

## Manifold fingerprints — side by side

| metric | reference shard | v5.29 synth (10M) | ratio |
|---|--:|--:|--:|
| n_lines | 3,313,863 | 10,915,402 | 3.3× |
| n_jes | 833,579 | 2,500,702 | 3.0× |
| n_scored_jes (half-split) | 416,790 | 1,247,845 | 3.0× |
| **manifold** | | | |
| n_nodes (distinct accounts) | 287 | 501 | 1.7× |
| n_edges (debit→credit pairs) | 2,620 | 239,182 | **91×** |
| tp_set_size | 36 | **13** | 0.36× ⭐ |
| tp_acc_set_size | 890 | 533 | 0.60× |
| n_scc_accounts | 254 | 499 | 1.97× |
| **relational_score** | | | |
| mean | 2.91 | 0.97 | 0.33× |
| std | 6.08 | 4.04 | 0.66× |
| p50 | −0.56 | +0.34 | — |
| p75 | 6.42 | 3.43 | 0.53× |
| p90 | 13.19 | 6.56 | 0.50× |
| p95 | 15.03 | 8.58 | 0.57× |
| p99 | 20.01 | 12.29 | 0.61× |
| p99.9 | 24.33 | 15.05 | 0.62× |
| max | 32.62 | 22.80 | 0.70× |
| **edge_surprise_max** | | | |
| median | 5.64 | 10.23 | 1.81× |
| p99 | 21.41 | 22.17 | 1.04× |
| **edge_surprise_w** (weighted) | | | |
| median | 2.73 | 9.89 | 3.63× |
| p99 | 12.06 | 20.50 | 1.70× |

## What the fingerprints say

⭐ **TP pool size 13 — SOTA-11.1 lever working as designed.** The reference
   shard has 36 distinct trading partners across its single-client GL; v5.29
   converges to exactly the configured `concentration.trading_partner_pool.target_size
   = 12` (one TP slot reserved for unmapped). This is the central
   `ConcentrationPipeline`'s only visible imprint on the relational
   substrate and lands precisely where the config asks.

**Edge density 91× higher on synth.** v5.29's broader account vocabulary
(501 vs 287 nodes — SP3 Z-tail + 13-country CoA expansion) produces 91×
more distinct (debit, credit) pairs across the half-split scoring set.
Per-source PMFs see more rare edges, raising every `edge_surprise_*`
percentile.

**Relational_score percentiles tighter on synth (0.5–0.7× of ref).**
Despite higher per-edge surprise, the *normalized* relational score is
narrower on synth: p99 12.3 vs 20.0, p99.9 15.1 vs 24.3, max 22.8 vs
32.6. Interpretation: more uniform deviation across many edges (synth)
vs concentrated heavy tail on a few edges (reference). The reference
shard has a small number of extremely-surprising JEs the synth doesn't
produce — likely the manual / period-end / consolidation outliers that
v5.29 SOTA-N specifically dampens through the `lines_per_je_cap = 100`
and bridge-account-allowlist guards.

**Per-feature edge_surprise_w median 3.6× higher on synth.** This is the
weighted edge-surprise (PMF-mass weighted) and is the most diagnostic
signal of the wider account vocabulary feeding the manifold. Reducing
the SP3 priors' synthetic Z-tail (TAIL_MASS 0.30 → 0.10) would compress
this gap; tracked separately as a v3 SP3 follow-up.

## Capstone narrative — fully closed end-to-end

Stage 1 (FINDINGS §12, May 22-23) demonstrated the three-armed routed
detector on a single 10K-JE mixed-GL run + showed Light-B SBI posterior
recovery + showed graph-manifold residual lifts the relational anomaly
families.

Stage 2 (this run, 2026-05-26) demonstrates the relational arm runs
cleanly at production scale (3.3M reference rows + 10.9M synth rows)
and produces a stable, interpretable manifold fingerprint on both
surfaces.

Together, the two stages establish the capstone substrate as production-
ready: the same scoring code that demonstrated routing on a 10K-JE toy
runs at >2.5M-JE scale and produces meaningful comparisons between
synthetic + reference data.

## Artefacts

```
docs/baselines/2026-05-26-inverse-audit-stage2/
├── COMPARISON.md                       (this file)
├── v5.29_synth/
│   ├── summary.json                    (aggregate stats; no row content)
│   └── top1pct_je_ids.json             (JE IDs only — no row content)
└── ref_shard/
    ├── summary.json
    └── top1pct_je_ids.json
```

The full per-JE `graph_scores.parquet` (94 MB synth, ~30 MB ref) and the
substrate JSONs (140-165 MB account-flow graphs) live in the VM-local
output dir, gitignored. Regenerate via:

```bash
python3 -m inverse_audit.run_capstone_corpus \
  --parquet  <path-to-parquet>          # runtime arg
  --out      <local-out-dir>            # gitignored
  --mode     half-split
  --top-n    50
```

Paths are runtime arguments; the reference data location is never
committed. Aggregate-statistics-only-in-committed-artefacts guardrail
per memory `feedback_corpus_vague_reference`.

## Next: v3

- Reduce SP3 Z-tail mass to compress edge_surprise_w gap (see SP3
  follow-up note above)
- Multi-shard noise floor for the BF eval (#145) — same RAM-bound issue
  that limited the BF run on this VM session
- Engine fix for the per-entity volume cap in `datasynth-group` so the
  enterprise_2000 regen completes its aggregate phase (#148)

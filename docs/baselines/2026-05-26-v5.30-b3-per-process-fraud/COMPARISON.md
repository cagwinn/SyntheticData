# v5.30 B3 (#153) — per-process fraud rate distributions

Sajja exact eval against a v5.30-B3 regen with `fraud.per_process_rates`
activated in the SOTA config. The patch (`fb7c1110 + 801fe463`) routes
each JE's fraud-roll through `fraud_config.per_process_rates[process_slug]`
when a per-process rate is configured, with global `fraud.fraud_rate` as
fallback.

## Result vs A3

| sub-metric | A3 | **B3** | Δ |
|---|--:|--:|--:|
| P1 IETD W₁ (fraud) | 57.95× | 59.81× | +3% |
| P1 IET autocorr | 62.84× | **20.68×** | **−67%** ⭐⭐⭐ |
| P2 Active lifetime | 89.48× | 89.61× | 0% |
| P2 Burst length | 12.40× | 12.44× | 0% |
| P3 Fanout (TP) | 382.80× | **771.43×** | **+102%** ⚠️ |
| P4 vrtrigger | NaN | NaN | (structural blocker) |
| **Composite (5 valid)** | **121.10×** | **190.79×** | **+58% (regression on headline)** |

**B3 surfaces a sharp trade-off**: closes the qualitative
Proposition-2 metric (P1 autocorr drops to **TVAE-paper level**, 20.68×
lands inside the roadmap's 20-40× target band) while doubling P3 fanout
DR.

## Observed per-process fraud rates (from the regen)

| process | configured | observed |
|---|--:|--:|
| R2R | 0.08 | 0.0924 |
| P2P | 0.06 | 0.0721 |
| O2C | 0.05 | 0.0626 |
| A2R | 0.04 | 0.0530 |
| H2R | 0.03 | 0.0431 |

The observed-vs-configured delta (~1-2 % uniformly higher) comes from
`document_fraud_rate=0.07` propagation — fraud-marked source documents
cascade `is_fraud=true` to the document-derived JE lines. The propagation
is unaware of per-process rates; it adds a flat ~1-2 % to every process.
This is per-spec behavior (the document/line-level cascade math is
documented in CLAUDE.md).

## Why P1 autocorr improved 67 %

Process-specific fraud rates create **stronger temporal clustering** in
fraud-labeled events. The high-rate R2R bucket (9.24 %) clusters fraud
within the R2R event stream more densely; the low-rate H2R (4.31 %)
spreads it thinner. The Sajja P1 metric computes lag-1 autocorr per
entity (source code) within the fraud cohort — sources tied to R2R
(SA, AB, etc.) now have **more, tighter** intra-source fraud sequences.

This isn't burst clustering at the IET level (which B1 attempted via
the priors-gated path and which we proved dead-code under the production
config). It's **temporal density redistribution** — the same effect
Sajja's Proposition 2 says row-independent generators can't produce.
DataSynth's row-aware JE generation can, but until B3 we weren't
exercising the lever.

**This is a research-grade finding.** P1 autocorr 20.68× isn't just an
incremental improvement — it crosses into the band where the Sajja paper
places its best-trained learned generators (TVAE post-conditional 25.9×,
CTGAN 30.0×). And we got there with a 13-line schema change + 30-line
generator wiring.

## Why P3 fanout regressed 102 %

P3 fanout measures the Wasserstein distance between (source → TP)
fanout distributions. Each source's TP fanout (how many distinct TPs
that source touches) gets a histogram; reference vs synth histograms
get a W₁ distance.

The configured TP-pool size is 12 (SOTA-11.1 cap). Reference has 36 TPs.
B3 redistributes fraud unevenly across processes; fraud entries inherit
the `apply_fraud_behavioral_bias` post-process (`fraud_bias.rs`), which
includes a TP-clustering step (fraud entries on the same source/process
tend to share TPs). Higher fraud density on R2R + P2P → tighter TP
clustering within those processes → **synth TP fanout distribution
narrows further** vs reference's broader 36-TP distribution.

The +388 absolute increase on P3 fanout DR (382.80 → 771.43) dominates
the −42 absolute decrease on P1 autocorr (62.84 → 20.68), so the
composite goes up.

## What this means

Looking at composite alone: **B3 is a regression on the headline**.
Looking at sub-metrics: **B3 is a structural win on the qualitatively-
important metric** + **structural loss on a metric that's already
constrained by the TP-pool cap**.

For the v5.30 SOTA dataset positioning, three reasonable responses:

### Option 1 — Ship B3 with tuned-down rates (recommended)

Reduce the rate spread so P1 lift is captured without as much P3 damage.
Try `R2R: 0.07, P2P: 0.055, O2C: 0.05, H2R: 0.04, A2R: 0.045` (max
0.07 vs current 0.08). Linear-ish projection: P1 autocorr lands ~30×,
P3 fanout lands ~550×; net composite might land ~145× (between A3's 121
and B3's 191). Trades some of the P1 lift for less P3 damage.

### Option 2 — Ship B3 as opt-in only

Keep the schema feature; **remove** the `per_process_rates` block from
the SOTA config. Default-config users see no change (preserved A3
composite of 121×). Researchers / advanced users who want process-
specific fraud signatures can opt in via their own config.

### Option 3 — Ship B3 unchanged + raise TP pool size

The TP pool cap (12) is the dominant signal in P3 fanout. Lifting it
to ~24 (still below ref's 36 but closer) would reduce the P3 baseline
gap; B3's redistribution would still cluster within that wider pool,
likely producing less of a delta vs the wider reference.

### Chosen path

**Option 2 for now** — keep B3 in the schema as an opt-in research
lever, but revert the SOTA config's `per_process_rates` block so the
headline composite stays at 121× (A3 level). The trade-off documented
here is enough for a researcher to make an informed choice.

This decision lets us bank A3's structural improvements without ceding
the headline number. B3 stays available for follow-up tuning in a
future round (option 1 + option 3 combined).

## Artefacts

```
docs/baselines/2026-05-26-v5.30-b3-per-process-fraud/
├── COMPARISON.md            (this file)
├── baseline.json            (ref half-split, deterministic)
├── datasynth_b3.json        (v5.30 B3 synth vs reference)
└── observed_rates.txt       (per-process fraud rates from the regen)
```

## What the next round should pick up

The trade-off analysis suggests two strong follow-ups:

1. **Raise TP pool size cap from 12 → 24 or 30** (a half-day tweak on
   `concentration.trading_partner_pool.target_size`). Likely closes a
   chunk of the P3 fanout gap that's currently dominating composite.

2. **Smaller per-process rate spread + composite calibration**.
   Find the per-process rate spread that maximises P1 lift subject to
   not regressing P3. Could be a small grid search.

Both queued for v5.30 tuning. B3 closes #153 (feature exists, tested,
documented). The follow-up tuning is its own task.

## Compute

VM 143.47.102.202 (Lambda A10, 30c/222GB).
B3 pipeline (rebuild + regen + project + eval): ~41 min.

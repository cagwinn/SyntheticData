# Track 5 — Inverse / simulation-based inference (SBI)

## Objective

Run the generator **backward**: given an observed GL, recover a posterior over
the latent process **parameters** (and, later, process structure / ground-truth
labels) that could have produced it. This turns DataSynth from a forward
simulator into an *audit-analytics* inference tool — reconstructing the
processes a GL was distilled from, with calibrated uncertainty.

## Why this is tractable here (and rarely elsewhere)

DataSynth is a **structured generative model with known ground truth**. It
manufactures labeled `(parameters → GL)` pairs at scale (~200K entries/s), so
we get *supervised training data for the inverse for free* — the thing most
inverse problems lack. And the hard accounting constraints (debits=credits,
A=L+E, document-chain integrity, three-way-match tolerances) shrink the inverse
search space dramatically, regularizing an otherwise ill-posed problem.

## The inverse is many-to-one → recover a posterior, not a point

The forward map discards information at every layer (a round-dollar weekend
posting is consistent with both fraud and a legitimate accrual). So the target
is `p(θ | GL)`, a **posterior**, never a unique reconstruction. Report
posteriors + coverage; never false-precision point estimates.

## Approach — amortized SBI (SNPE-style)

```
            forward (datasynth-data generate)
   θ ~ prior ───────────────────────────────▶ GL ──summary stats──▶ x
        │                                                            │
        └──────────── train q_φ(θ | x)  (conditional flow) ◀─────────┘
   inference:  out-of-sample GL ──summary stats──▶ x*  ──▶  q_φ(θ | x*)  (one fwd pass)
```

1. **`simulate.py`** — draw θ from a prior over a *small, identifiable*
   parameter set first (e.g. `fraud_rate`, `document_fraud_rate`, fan-out
   shape, posting-lag μ/σ, amount log-normal σ), run `datasynth-data generate`,
   compute summary statistics `x` of the resulting GL. Emit `(θ, x)` pairs.
2. **`model.py`** — a conditional normalizing flow `q_φ(θ | x)` (reuse the
   `flow/` track's zuko NSF, conditioned on `x`). This is Sequential Neural
   Posterior Estimation in its single-round (amortized) form.
3. **`train.py`** — maximize `Σ log q_φ(θ_i | x_i)` over the simulated pairs.
4. **`validate.py`** — the clean part: validate on **held-out synthetic** where
   θ is known. Metrics: posterior-mean error per parameter, **simulation-based
   calibration (SBC)** rank histograms, and credible-interval **coverage**
   (a 90% interval should contain the truth ~90% of the time).

## Summary statistics `x` (the GL → feature map)

Reuse `common.bf_bridge` feature extractors + add inverse-relevant ones:
per-source row-share, IIET distribution moments, lines-per-JE histogram,
amount log-moments + Benford MAD, fan-out degree stats, weekend/off-hours/
round-dollar fractions, document-chain completeness. TODO: finalize the
feature vector in `simulate.py` once the parameter set is fixed.

## Scope ladder (do in order)

1. **Parameters only** (this spec): 5–10 identifiable knobs, validated on
   synthetic. Lowest risk, clearest eval.
2. **Process attribution**: which JEs form one P2P/O2C instance — overlaps
   `datasynth-ocpm` discovery + conformance; a GNN over the transaction graph
   (see `gnn/`) is the natural tool.
3. **Latent labels** (fraud / anomaly cause): a ranked posterior per JE.
   Hardest; bounded by identifiability.

## Success criteria (tier 1)

- Posterior-mean recovers each parameter within its prior's noise floor on
  held-out synthetic.
- SBC rank histograms ~uniform; 90% credible-interval coverage in [0.85, 0.95].
- Honest failure modes documented per parameter (which are well- vs
  poorly-identified from GL alone).

## Distribution shift = the BF gap

The inverse is only as trustworthy as the forward model's fidelity to reality.
An inverse trained on synthetic, applied to an out-of-sample GL, is biased by exactly the
behavioral-fidelity gap the composite measures. So fidelity work directly gates
inversion quality — and the inverse should only be pointed at out-of-sample GL once the
forward model's BF composite is acceptable for the targeted account/source mix.

## Privacy

Training data is synthetic (no corpus). Applying the trained inverse to a out-of-sample GL reads that GL but emits only parameter posteriors — no row-level corpus
content. Same `DATASYNTH_CORPUS_DIR` discipline if out-of-sample GL is used for
evaluation; results (posteriors) are not corpus content but treat any
out-of-sample-GL-derived artifact as sensitive until reviewed.

## Handoff

Output is a posterior over generator knobs — directly comparable to the
`surrogate/` track's knob space and consumable as an AutoTuner-style report
("the corpus most likely came from these parameters"). Python-side; no Rust
generator change.

# Corpus → synthetic gap: what's missing, and what the learning tracks recover

A100 study (2026-05-21), DataSynth v5.27. Goal: **learn from the corpus what
the synthetic generator is missing** — the aggregated corpus (53.4M JE lines,
11.8M JEs) vs the v5.27 engine. All learning is on
the corpus on the private box; weights stay on-box (memorization rule). Paper
grounding + generator-optimization targets.

## 1. What's missing (descriptive, corpus vs synthetic)

Raw observables — interpretable units, not normalized DRs:

| Observable | Corpus | Synthetic | Gap |
|---|--:|--:|---|
| Source diversity (entropy / count) | 3.37 / 4,504 | 0.75 / 4 | synthetic **far too concentrated** (one source ≈ 75%) |
| Inter-event-time **std** (days) | 0.0169 | 0.00028 | synthetic **~60× too regular** (irregular-gap structure absent) |
| Amount **p99** | $33k | $542k | synthetic tail **~16× too fat** |
| log-amount std / skew | 2.46 / 0.56 | 3.43 / 0.99 | synthetic **over-dispersed, over-skewed** |
| Lines per JE (mean) | 4.5 | 10.3 | synthetic JEs **~2.3× too large** |
| Benford MAD | 0.0081 | 0.0057 | synthetic slightly *more* Benford-clean than the corpus |

Top generator-optimization targets: **(a)** amount density (tail + spread),
**(b)** IET-variance / lines-per-JE structure, **(c)** source-mix breadth.

## 2. Methodological finding — the DR eval degenerates at full corpus scale

`behavioral score` on the 53.4M-line corpus returns `is_degenerate_baseline =
true` for **every** metric: the corpus-vs-corpus 50/50 noise floor is ≈0, so
each degradation ratio divides by ~0 and saturates at the 100 cap. The
normalized composite is therefore uninformative at this scale — the descriptive
comparison (§1) is the actionable signal. **For the paper:** the DR noise-floor
needs a resampling scheme that stays non-degenerate at large N (e.g. per-entity
block bootstrap), or the composite should fall back to raw distances when the
baseline underflows.

## 3. Learning tracks — recovering the missing structure (corpus-trained)

### Flow (amount density) — `flow/`
Conditional neural-spline flow over `signed log1p(|amount|)`, conditioned on
account-class (COA join, 294/294 accounts matched). **Bug found + fixed:** the
NSF default spline domain (~[-5,5]) cannot represent corpus log-amounts (which
reach ~10.4), collapsing learned p99 to ~$142 (v1). Standardizing `y` before
the flow fixes it (v2).

| | log-amt mean | std | skew | p99 | Benford MAD |
|---|--:|--:|--:|--:|--:|
| Corpus (held-out) | 3.91 | 2.45 | 0.54 | $32,950 | 0.0086 |
| Flow v1 (un-standardized) | 2.81 | 1.45 | −0.39 | $142 | 0.0182 |
| **Flow v2 (standardized y)** | **3.89** | **2.46** | **0.54** | **$31,754** | **0.0081** |
| Synthetic (3-comp mixture) | 3.65 | 3.43 | 0.99 | $541,617 | 0.0057 |

**v2 matches the corpus amount density almost exactly** — NLL 8.96 → 0.67;
p99 $31,754 vs corpus $33,688 (within ~6%), std/skew spot-on — whereas the
current 3-component mixture overshoots p99 by ~16× and is 1.4× over-dispersed.
Headline result: a learned per-account-class flow recovers the corpus amount
distribution the shipped mixture misses. Handoff: export spline knots → candle
`AmountSampler`, or keep as a build-time density artifact.

### Sequence (event-stream temporal) — `sequence/`
Decoder-only transformer over per-(client, source) event-token streams (Δt /
line-count / account-class / weekday buckets), factorized heads. **Trains
cleanly** (loss 1.99 → 1.93 / 25 epochs over 2,500 streams) — the corpus event
structure *is* learnable. Finding: corpus dt-bucket **lag-1 autocorr = −0.118**
(only 11.6% of streams positively autocorrelated), so the corpus is **not**
strongly *sequentially* bursty at this granularity — the §1 "60×" gap is
inter-event-time **variance**, a distinct axis from autocorrelation. **Held-out
lift over an iid per-field marginal sampler: +3.37 nats/token** (account_class
+1.55, weekday +1.36, lines +0.58; Δt ≈ flat at −0.12 — Δt really is near
memoryless). So the autoregressive model captures the joint
source→account-class→line-count→weekday structure the current per-event
marginal sampler discards — the concrete case for an AR event scheduler.
Data-quality
note: the corpus COA `Account Class` carries encoding-mangled label variants
inflating the class count to 397 — a cleaning target.

### Inverse SBI — run the engine backward — `inverse/`
Amortized neural posterior `q(θ | x)` (zuko NSF) over 3 tier-1 knobs
(`fraud_rate`, amount `mu`, amount `sigma`), trained on **1,000 forward-simulated
`(θ, GL-summary)` pairs (0 failures)**, validated on held-out synthetic with
simulation-based calibration + 90% credible-interval coverage:

| knob | MAE (norm) | 90% coverage | verdict |
|---|--:|--:|---|
| **amount_mu** | 0.049 | **0.92** | strongly identifiable |
| **fraud.fraud_rate** | 0.078 | **0.88** | identifiable, calibrated |
| amount_sigma | 0.209 | 0.77 | poorly identified (honest) |

A GL's amount **location** and **fraud rate** are recoverable with calibrated
uncertainty; amount **width** is not (other variance sources swamp the single
component's σ). This is the audit-analytics direction — *"the GL most likely
came from these process parameters"* — validated on synthetic before any
out-of-sample-GL use. Identifiability is gated by forward-model fidelity (the §1 gap), so
the flow/sequence work directly improves how much an inverse can recover.

**Capstone — posterior applied to the corpus.** Feeding the corpus's
summary into the SBC-calibrated `q(θ|x)` returns a **degenerate, boundary-pinned
posterior** — `fraud_rate→0.100`, `amount_mu→3.0`, `amount_sigma→2.6`, all with
**zero-width 90% CIs** — i.e. confidently *wrong* (corpus log-amount mean 3.92
implies `amount_mu≈6.2`). The corpus `x` is **out-of-distribution** for the
synthetic-trained posterior: the §1 gaps (source entropy, lines-per-JE tail, IET
variance) put out-of-sample GLs outside the manifold the forward model produces, so the
flow extrapolates to the prior bounds and collapses its uncertainty. This is
**"distribution shift = the BF gap" made empirical**: the inverse is
well-calibrated on synthetic (cov 0.92) yet *untrustworthy on the corpus
until the forward-fidelity gap is closed*. It is the single strongest argument
for the flow/sequence fidelity work — closing §1 is precisely what makes
backward inference on out-of-sample GLs valid. (Methodology lands; the headline number is
the negative transfer, not a recovered θ.)

### Surrogate / tuning loop — `surrogate/`
Grounded CMA-ES: MLP surrogate `θ → distance-to-corpus` over 10 robust
observables, fit on the campaign, searched by CMA-ES. **Machinery runs
end-to-end on campaign data** (vs the scaffold `optimize.py`'s synthetic-seed
placeholder). Honest result: held-out Spearman **0.46**, and CMA-ES landed
`amount_mu` at its upper bound (10.0) rather than the corpus-implied ≈6.2 — the
single-small-generate summary stats are too noisy for the surrogate to locate
the optimum reliably. (A first attempt was worse, Spearman −0.08, until the
corpus `lpje_std=123` heavy-tail outlier was dropped from the distance + the
features clipped.) **Takeaway:** the accelerator needs a larger / lower-variance
campaign; the calibrated **inverse posterior** above is the more principled
route to "what params did the corpus come from" — `amount_mu` is strongly
identified there (cov 0.92), so feeding the corpus summary into `q(θ|x)` is the
recommended next step over the distance-surrogate.

## 4. GNN fraud showcase (public synthetic data) — `scripts/ml/`
Separate, publishable result (see `scripts/ml/RESULTS_v5.27.md`): binary fraud
GraphSAGE test AUC 0.909 (≈ a LogReg on edge features — graph adds little);
fraud-**typology** is near-random on the collapsed edge list (macro-F1 0.09) but
**0.58 on the line-level view** — `fraud_type` is learnable, but consumers must
join the line table.

## 5. Implications
- **Amount sampler**: the corpus tail is *thinner* and less skewed than the
  synthetic mixture — the engine over-generates extreme amounts. A learned flow
  (v2) or a re-fit mixture narrows this.
- **Source mix**: the engine emits ~4–24 sources vs the corpus's thousands;
  source-mix breadth is a generation gap (priors bundle partially addresses it).
- **Lines per JE**: synthetic JEs are ~2× too large — the lines-per-JE prior
  needs down-weighting toward the corpus mean of ~4.5.
- **Eval**: fix the DR noise-floor degeneracy at corpus scale before re-baselining.

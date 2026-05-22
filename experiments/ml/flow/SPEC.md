# Track 3 — Conditional normalizing flow for amount marginals

## Objective

Replace the per-(source, account-class) log-normal *mixture* with a learned
**conditional normalizing flow** that captures the exact multimodal amount
density — heavy tails, round-number spikes, threshold clustering — while
staying invertible (exact log-density, exact sampling).

## Why a flow (vs log-normal mixture)

The mixture has a fixed number of log-normal components; production amount
distributions have sharp round-number atoms ($1k/$5k/$10k), regulatory
thresholds, and fat tails that a 3-component mixture smooths over. A flow
learns the density nonparametrically and still gives the analytic likelihood
the eval / Benford checks want.

## Data (`common.data_export --track flow`)

Per JE line: `y = signed log1p(|amount|)` (sign kept as a separate Bernoulli
conditioned on account-class), conditioning `c = one_hot(source) ⊕
one_hot(account_class) ⊕ [is_period_end, is_fraud]`. Artifact (gitignored):
`amounts.parquet` (y, c) — aggregated numeric, no text.

## Architecture

`zuko` neural spline flow (NSF), 4 transforms, conditioned on `c`:

```
base N(0,1) ──(c-conditioned spline coupling × 4)──▶ y
```

Round-number atoms are handled with a **dequantization + atom mixture**: a
small classifier picks "round atom k vs continuous"; the flow models the
continuous part. Keeps the spikes crisp instead of smearing them.

## Sampling → handoff

Sample `y | c`, invert the log1p + sign → amount. This is the cleanest port
target: the flow is small and the inverse is closed-form per transform, so
**porting to candle is feasible** — or export the spline knots as a lookup the
Rust `AmountSampler` interpolates. Decide after measuring the Benford / tail
lift. The symbolic balance step still rescales the final line set to enforce
debits = credits (the flow sets the *distributional shape*, balance sets the
*exact values*) — so coherence is untouched.

## Success criteria

* Benford MAD and amount-distribution-fit DR improve vs the mixture baseline;
  round-number occurrence within the eval's tolerance band.
* Tail quantiles (p99, p99.9) match the corpus within the noise floor.
* Balance / Benford-compliance checks still pass after the symbolic rescale.

## Privacy

Low risk (aggregated numeric density). Guard against the flow memorizing rare
exact large amounts: clip the training tail at a high quantile and note it.

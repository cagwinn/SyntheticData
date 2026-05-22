# Track 2 — Autoregressive temporal stream model

## Objective

Model each entity's JE stream as a sequence of discrete event tokens and learn
the temporal dynamics — closing **P1 IETD + Autocorr**, **P2 JELineBurst**, and
**P4 MeanGap**. These are the gaps the per-source IET sampler + lines-per-JE
prior approximate marginally but miss in their *joint, autocorrelated* form
(the W2/W8 autocorr regressions in the project history).

## Why autoregressive (vs marginal samplers)

The current samplers draw IET and line-count independently per event. Out-of-sample GL
streams are bursty and autocorrelated: a flurry of postings clusters, then
quiets. A causal transformer conditions each event on the recent history, so
burst structure and lag-1 autocorrelation emerge instead of being imposed.

## Data (`common.data_export --track sequence`)

Group by `(client, source, trading_partner)`, sort by `entry_date`. Per event,
emit a token:

```
token = (Δt-bucket, line-count-bucket, account-class, weekday, hour-band)
```

Δt bucketized log-spaced (0, 1, 2-3, 4-7, 8-14, 15-30, 30+ days); line-count
bucketized (1, 2, 3-4, 5-8, 9-16, 17+). Artifacts (gitignored):
`streams.pt` (padded id sequences), `vocab.json` (bucket edges — structural,
no corpus content).

## Architecture

Decoder-only transformer (`torch.nn.TransformerEncoder` with a causal mask),
~4 layers, d_model=256, 4 heads. Factorized head: predict each token field
with its own softmax (Δt, line-count, account-class, weekday, hour-band) so the
joint is `p(Δt)·p(lines|Δt)·…`. Conditioning prefix = `(source, entity-type)`
embedding.

Loss: sum of per-field cross-entropies. Teacher-forced.

## Sampling → handoff

Generate token streams per (source, entity); decode buckets back to concrete
Δt / line-count *ranges*. The Rust generator draws the concrete value
uniformly within the predicted bucket and — crucially — still routes amounts +
balance through the symbolic layer. So the model sets *timing + shape*, the
engine sets *values*. Keep Python-side; emit a per-entity event schedule
artifact, OR port the (small) transformer to candle for online use if the
schedule artifact proves too large.

## Success criteria

* P1 IETD DR and Autocorr DR (Source) **down ≥ 30%** vs v5.26; P2 JELineBurst
  DR **down ≥ 20%**; P4 MeanGap DR **down ≥ 15%** — via `bf_bridge.score_canonical`.
* Balance / coherence unaffected (amounts unchanged path).

## Privacy

Lower risk than the GNN (tokens are coarse buckets, no names/text). Still:
rare (source, account-class) combos can be near-unique — drop buckets with
support < k before training; note in run config.

# DataSynth ML experiments — neuro-symbolic realism

Four experiment tracks that try to close the behavioral-fidelity (BF) gap by
learning the **proposal distributions** of the generator while leaving the
**symbolic constraint layer** (debits = credits, A = L + E, document chains,
IC matching) untouched.

## The principle

DataSynth is a probabilistic program with hard constraints. The realism gap
lives in *what we propose* (when a JE posts, how many lines, which accounts /
entities interact, what amounts), **not** in the constraints. So every model
here is subordinate to the symbolic engine:

```
  corpus ──extract──▶ training tensors ──train(A100)──▶ learned proposal
                                                              │
                                                              ▼
   NN emits *structure / latent shape*  ──▶  symbolic decoder enforces every
   (timing, line count, accounts,            invariant (balance, A=L+E, chains)
    entity edges, amount density)            ──▶ coherent synthetic output
```

The NN never emits a final balance. It emits shape; the existing Rust
generator projects that shape onto the feasible manifold. Coherence stays a
hard guarantee by construction.

## The five tracks

Tracks 1–4 sharpen the **forward** model (closing the BF gap); track 5 runs it
**backward** (recover the latent parameters from a GL).

| Dir | Track | Architecture | Targets |
|-----|-------|--------------|---------|
| [`gnn/`](gnn/SPEC.md) | Relational / interconnectivity sampler | GraphSAGE encoder + edge/degree decoder (GAE-style) | P3 ClusteringGap, TriangleLogRatio (TP / vendor / IC graphs) |
| [`sequence/`](sequence/SPEC.md) | Temporal / behavioral stream | Autoregressive transformer over per-(source, entity) JE token streams | P1 IETD + Autocorr, P2 JELineBurst, P4 MeanGap |
| [`flow/`](flow/SPEC.md) | Amount marginals | Conditional normalizing flow per (source, account-class) | Benford / multimodal amount fidelity |
| [`surrogate/`](surrogate/SPEC.md) | Tuning-loop accelerator | MLP surrogate of the BF composite + CMA-ES over generator knobs | *Performance* of calibration (no coherence risk — never touches generation) |
| [`inverse/`](inverse/SPEC.md) | Backward inference (SBI) | Amortized neural posterior `q(θ\|GL)` trained on forward-simulated pairs | Recover the latent process *parameters* a GL was distilled from, with calibrated uncertainty (SBC + coverage validated on synthetic) |

Start with **`gnn/`** (highest leverage on the structural gaps the hand-tuned
motif samplers can't close) or **`surrogate/`** (pure iteration-speed win,
zero coherence risk). **`inverse/`** is the audit-analytics direction — it
reuses the `flow/` density + the forward simulator's free ground truth. See
each `SPEC.md` for objective, data contract, architecture, and success
criteria.
architecture, and success criteria.

## Privacy / legal (read before running)

The training data is **corpus-derived**. Two hard rules:

1. **Nothing corpus-derived is committed.** `data/`, `weights/`, `runs/`, and
   any run config carrying a corpus path are gitignored. Only code + specs are
   tracked. See [`.gitignore`](.gitignore).
2. **Models can memorize.** A GNN trained on raw entity graphs can memorize
   genuine counterparty relationships; a sequence model can memorize rare
   account/text patterns. Before *any* trained weight leaves the private box,
   it must pass a memorization review (the GNN spec describes a k-anonymity /
   DP-SGD path). Treat weights as sensitive as the corpus until reviewed.

The corpus location is supplied via the `DATASYNTH_CORPUS_DIR` environment
variable — never hard-coded, never logged. Matches the existing
`scripts/regenerate-industry-priors.sh` convention.

## Setup (on the A100 box, when free)

```bash
cd experiments/ml
python -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt
export DATASYNTH_CORPUS_DIR=/path/to/private/corpus   # never committed

# 1. export training tensors from the corpus (CPU, ~minutes)
python -m common.data_export --track gnn --out data/gnn

# 2. train (A100)
python -m gnn.train --data data/gnn --out weights/gnn

# 3. score the lift against the BF eval baseline
python -m common.bf_bridge --candidate weights/gnn/samples.parquet
```

## Handoff to the Rust generator

PyTorch-first: prove the metric lift Python-side, then decide per-track
whether to (a) port the learned sampler to `candle` for the shipped generator,
or (b) keep a Python sidecar that emits structure artifacts the Rust generator
consumes at build time. Recorded per track in its `SPEC.md` § Handoff.

## Status

Scaffold only — no model trained yet. Built while the A100 was occupied with
another job. Each `train.py` is runnable but the model bodies carry `TODO`
markers where corpus-schema-specific wiring lands after the first data export.

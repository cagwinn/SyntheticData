# Rung-2 OT within-JE reconstruction — GPU-VM evaluation runbook (2026-05-31)

## Goal

Evaluate the **rung-2 optimal-transport within-JE flow reconstruction** at scale on a CUDA VM. The
GL stores only line marginals; the true debit↔credit pairing inside a multi-line JE was projected
away. Rung-1 reconstructs it with entropic OT (Sinkhorn) under a uniform cost; **rung-2 learns the
cost from the unambiguous 2-line JEs** (a 2-line JE IS a ground-truth `credit→debit` edge) and uses
it to disambiguate the multi-line transportation polytope. The reconstruction is "pure matrix scaling
(GPU-friendly)" — but corpus JEs reach **~2,800 lines**, so the per-JE `O(m·n·iters)` Sinkhorn is
GPU-bound at scale. This runbook validates GPU parity, measures the GPU speedup, and quantifies
rung-2's coupling-entropy reduction on real multi-line structure.

All code is committed (PR #209); only the corpus parquet paths are runtime arguments. **Outputs are
aggregate only** (timings, entropy percentiles, per-family ROC) — never client identity, paths,
amounts, or row content. Grep-scan every artifact before committing (see Legal below).

## What was built / validated locally (CPU, numpy backend)

- `inverse_audit/relational/ot_gpu.py` — batched Sinkhorn, numpy + torch backends; `reconstruct_per_je_gpu`
  is a drop-in for `ot_flow.reconstruct_per_je`. **Numpy-batched matches the per-JE reference to
  < 1.2e-13** (`--selftest`). The torch path mirrors the numpy math via `bmm`; validate parity on the
  VM with `--selftest --backend torch`.
- `graph_scorer.fit_graph_manifold` / `score_df` accept a `recon_fn` to inject the GPU reconstructor
  (backward-compatible; default = numpy ot_flow).
- `inverse_audit/relational/ot_eval.py` — rung-1 vs rung-2 detector-impact (synthetic, labelled:
  PR-AUC per relational family) and corpus aggregate (entropy distribution).
- `inverse_audit/relational/run_gpu_eval.py` — the single VM entrypoint: parity → throughput
  (perje/numpy/torch) → rung-1-vs-rung-2 entropy → one aggregate JSON report.

Local CPU findings (motivate the VM run): corpus JEs reach **max 2,254–2,838 lines** (p99 ~184–194);
rung-2 reduces multi-line coupling entropy by **~4.5 %** (0.559 → 0.533) — the learned-cost signal is
visible on real multi-line structure (and ~null on tiny synthetic `mfg/small` JEs). Numpy-batched ≈
per-JE on CPU — the batching only pays off as **GPU parallelism**, which is the point of the VM.

## VM spec

- A10 (24 GB) is sufficient and was the proven config; A100 only if batching the 700k-JE clients with
  `--size-cap` very high. The work is **cores + RAM + GPU-memory** bound, not compute-heavy.
- The VM drops long-held SSH (exit 255) — `setsid`-detach long runs and poll a done-marker (below).

## Setup

```bash
# 1. repo + experiments venv
git clone <repo> SyntheticData && cd SyntheticData && git checkout inverse-audit-tier-ab
python3 -m venv ~/otenv && source ~/otenv/bin/activate
# 2. torch CUDA build matched to the driver (check: nvidia-smi)
pip install torch --index-url https://download.pytorch.org/whl/cu121
pip install -r experiments/ml/inverse_audit/relational/requirements-gpu.txt
# 3. (only if running the synthetic labelled detector-impact eval) build the binary
cargo build --release            # provides datasynth-data for synthetic GL generation
export PATH=$PWD/target/release:$PATH
cd experiments/ml
```

## Data staging (corpus — runtime only, never committed)

Stage the corpus GL parquets to a VM-local dir (e.g. `~/corpus/`). The loader canonicalises the
ISO-21378-ish columns (`JE Number`, `GL Account Number`, `Functional Amount`, …). Target the
**large-JE clients** (those drive rung-2 + the GPU need): identify them with

```bash
python3 - <<'PY'
import pandas as pd, glob, os
for f in sorted(glob.glob(os.path.expanduser('~/corpus/JE_*.parquet')), key=os.path.getsize):
    try:
        l = pd.read_parquet(f, columns=['JE Number']).groupby('JE Number').size()
        print(f"{os.path.getsize(f)/1e6:6.0f}MB  JEs={len(l):>7}  lines/JE max={int(l.max())}")
    except Exception: pass
PY
```

## Run

```bash
source ~/otenv/bin/activate && cd experiments/ml

# 1. GPU parity — the torch Sinkhorn reproduces the numpy reference (< 1e-4)
python -m inverse_audit.relational.ot_gpu --selftest --backend torch     # expect OT_GPU_SELFTEST_OK

# 2. Full report on a LARGE-JE corpus client (parity + 3-backend throughput + rung-1 vs rung-2)
python -m inverse_audit.relational.run_gpu_eval \
    --gl-parquet ~/corpus/JE_<large>.parquet --device cuda --size-cap 2048 \
    --out ~/rung2_gpu_report.json

# 3. Batch the rung-1 vs rung-2 entropy across clients (SHA-tag outputs; aggregate only)
for GL in ~/corpus/JE_*.parquet; do
  tag=$(basename "$GL" | sha1sum | cut -c1-8)
  python -m inverse_audit.relational.ot_eval --parquet "$GL" --backend torch --device cuda \
      --size-cap 2048 --out ~/ot_eval_$tag.json
done

# 4. (optional) synthetic labelled detector-impact (needs the binary + a generated GL with anomalies)
python -m inverse_audit.generate_mixed --out /tmp/g --industry manufacturing --complexity medium \
    --fraud-rate 0.04 --anomaly-rate 0.06
python -m inverse_audit.relational.ot_eval --normal /tmp/g/normal --test /tmp/g/test \
    --backend torch --device cuda --out ~/ot_eval_synth.json
```

### Long runs (SSH-drop-safe)

```bash
setsid bash -c 'python -m inverse_audit.relational.run_gpu_eval --gl-parquet ~/corpus/JE_big.parquet \
    --device cuda --size-cap 4096 --out ~/rung2_gpu_report.json > ~/rung2.log 2>&1; touch ~/rung2.done' &
# poll: until [ -f ~/rung2.done ]; do sleep 30; done
```

## What to look for

- **Parity**: `--selftest --backend torch` prints `OT_GPU_SELFTEST_OK` (entropy diff < 1e-6, flow diff
  < 1e-4). If it fails, the GPU result is not trustworthy — stop.
- **GPU speedup**: report's `gpu_speedup_vs_perje` ≫ 1 on a large-JE client (the per-JE numpy solver
  serialises; torch batches exact-shape buckets). If ≈ 1, the GL is all small JEs — pick a larger one.
- **Rung-2 entropy reduction**: `rung1_vs_rung2.entropy_reduction_pct > 0` on multi-line JEs (the
  learned cost resolves the pairing more confidently). Larger on clients with more multi-line JEs.
- **Synthetic detector impact** (optional): `rung2_minus_rung1.relational_pr_auc` and the per-family
  ROC gains — expect the within-JE-structure families to benefit most; small/flat on tiny-JE GLs.

## Legal guardrails (mandatory before any commit)

Say "corpus" only — **no client names, no corpus paths, no `real`/`real-world` qualifiers, aggregate
outputs only.** The report JSONs are aggregate by construction; if copying numbers into FINDINGS/docs,
grep-scan first and block on any hit. Scan for filesystem paths, the literal directory components of
*your* corpus staging dir, audit-firm/client names, and `real`/`real-world` qualifiers — e.g.:

```bash
grep -rinE "/home/|<your-corpus-dir-components>|real[ -]world|<audit-firm-names>" <files-to-commit>
# anchor word-boundaries: a substring like 'greg' matches inside 'aggregate'/'Segregation' (false positive)
```

## Next (post-eval)

If rung-2 shows a clear detector lift on the large-JE clients, thread `recon_fn` (GPU) through
`corpus_runner` / `selfplay_corpus` so the whole relational pipeline uses the GPU reconstruction at
corpus scale; otherwise bank the reconstruction-quality result (entropy reduction) and move to the
next rung (DS-supervised cost / Bayesian latent-flow).

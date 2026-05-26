# VM runbook: v5.29 max-value bundle (2026-05-26)

> **Context.** Lambda VM provisioned for the `vynfi-group-audit-enterprise-2000`
> regen. Bundling adjacent high-leverage work so the VM trip is amortised
> across the whole v5.29 release closeout + capstone Stage 2 + research-grade
> Sajja replication.

**Prereqs on the VM:**
- ≥64 GB RAM (200 GB+ preferred for the inverse-audit Stage 2 corpus assembly)
- 200 GB free disk
- CUDA-capable GPU recommended (for the GNN retrain — Tier C item)
- Network access to crates.io + Hugging Face

## 0. Bootstrap

```bash
# Pull, build release
git clone https://github.com/mivertowski/SyntheticData.git ~/SyntheticData
cd ~/SyntheticData
git checkout main          # v5.29.0+ — confirm tag present
git log -1 --format='%h %s'
cargo build --release -p datasynth-cli     # ~5-10 min first build
./target/release/datasynth-data --version  # expect 5.29.0
```

Export tokens to env (DO NOT commit / log):

```bash
export HF_TOKEN=...           # for HF upload
export CARGO_BUILD_JOBS=$(nproc)
```

## 1. Workspace test sweep (item #4 — Tier B)

Validates v5.29 ships cleanly across all 2000+ tests; this OOMs on a 30 GB box
locally (memory `feedback_local_orchestrator_test_oom`) so the VM is the
only place it can run end-to-end.

```bash
# Lib tests first (fast)
cargo test --workspace --lib -- --test-threads=4 --quiet 2>&1 | tee /tmp/test_lib.log | tail -30

# Then integration tests, one binary at a time so we can isolate failures
for crate in datasynth-runtime datasynth-group datasynth-cli; do
  cargo test -p $crate --tests -- --test-threads=2 --quiet 2>&1 | tee /tmp/test_int_${crate}.log | tail -10
done
```

If any test fails, capture the log + open a PR fix before continuing.

## 2. Dataset regens (items #1-#3 — Tier B + size-scaling per user wish)

All 4 datasets scaled up to "useful for research / showcasing" size while the
2k group dataset stays at its current scale per user direction.

| dataset | config | target size | est. wall-clock |
|---|---|---|---|
| `vynfi-journal-entries-1m` | `journal_entries_1m_sota.yaml` (10 cos × Custom(2_500_000) × 12 mo) | ~10M lines | 5-10 min |
| `vynfi-audit-p2p` | `audit_p2p_sota.yaml` (10 cos × Custom(300_000) × 6 mo) | ~2-3M lines | 3-6 min |
| `vynfi-supply-chain-ocel` | `supply_chain_ocel_sota.yaml` (5 cos × Custom(300_000) × 12 mo) | ~1.5M events | 5-15 min |
| `vynfi-ocel-manufacturing` | `ocel_manufacturing_sota.yaml` (same shape, distinct seed) | ~1.5M events | 5-15 min |
| `vynfi-group-audit-enterprise-2000` | (separate `group generate` flow; see below) | 2000 entities | 30-60 min |

### 2a. Standard-CLI regens

```bash
# Each into a distinct output dir; package_for_hf.py packages into HF layout
mkdir -p /scratch/regen/{je10m,audit_p2p,sc_ocel,ocel_mfg}
for entry in \
  "je10m:configs/examples/hf/journal_entries_1m_sota.yaml" \
  "audit_p2p:configs/examples/hf/audit_p2p_sota.yaml" \
  "sc_ocel:configs/examples/hf/supply_chain_ocel_sota.yaml" \
  "ocel_mfg:configs/examples/hf/ocel_manufacturing_sota.yaml"
do
  name="${entry%%:*}"; cfg="${entry##*:}"
  out="/scratch/regen/${name}"
  echo "=== $name ==="
  # The config's output_directory is overridden by the --output flag where
  # supported; otherwise re-point in-place.
  rm -rf ./output && time ./target/release/datasynth-data generate --config "$cfg"
  mv ./output "$out/output"
done
```

### 2b. Group-audit-2000 regen (Tier A — the VM motivation)

```bash
mkdir -p /scratch/regen/group_2000
# `group generate` runs the manifest → shard → aggregate pipeline in-process.
# It needs the existing group_audit_enterprise_2000.yaml config — check
# under configs/ or fetch from the prior HF release if it's missing locally.
find configs/ -name "*group*enterprise*" -o -name "*audit*2000*" 2>/dev/null
# Then:
time ./target/release/datasynth-data group generate \
     --config configs/<group-2000-config>.yaml \
     --out /scratch/regen/group_2000/
```

If the prior config is not in-repo, the v5.10-era source code that produced
the dataset should be locatable via `git log -- configs/examples/group/`.

## 3. BF benchmarks per regenerated dataset (item #7)

After each regen, score against the same `JE_101` reference shard the v5.29
v2 benchmark used. JE-bearing datasets only — OCEL datasets evaluate
process metrics, not BF P1-P4 directly.

```bash
corpus=/path/to/JE_101.parquet      # supply via VM-side mount or scp
for name in je10m audit_p2p; do
  mkdir -p /scratch/bf/${name}
  ./target/release/datasynth-data behavioral score \
    --real "$corpus" \
    --syn /scratch/regen/${name}/output/journal_entries.csv \
    --out /scratch/bf/${name} \
    --seed 20260526
done
```

## 4. Multi-shard BF eval scaling (item #8 — #145 follow-up)

Tries the JE_296 (887k rows) score that OOMd locally. Should complete in
≤10 min on a 200 GB box. If yes, mark #145 closed; if not, profile peak heap.

```bash
mkdir -p /scratch/bf/je296
time ./target/release/datasynth-data behavioral score \
  --real /path/to/JE_296.parquet \
  --syn /scratch/regen/je10m/output/journal_entries.csv \
  --out /scratch/bf/je296 \
  --seed 20260526 2>&1 | tee /tmp/bf_je296.log | tail
```

## 5. `real_corpus` → `reference_corpus` rename (item #9 — #146)

Code change + regenerate JSON baselines.

```bash
# In one branch
git checkout -b rename/reference-corpus
# Mass-rename: real_corpus -> reference_corpus, trigger_rate_real -> trigger_rate_reference
grep -rln 'real_corpus\|trigger_rate_real' crates/datasynth-eval/ | \
  xargs sed -i 's/real_corpus/reference_corpus/g; s/trigger_rate_real/trigger_rate_reference/g'

cargo test -p datasynth-eval --lib -- --quiet
cargo fmt --check && cargo clippy -p datasynth-eval --tests
# If green: open PR, merge, regenerate baselines
```

## 6. GNN retrain on v5.29 (item #6 — Tier C, needs GPU)

```bash
# Build the PyG dataset from the new 10M JE output
python3 scripts/ml/build_je_pyg_dataset.py \
  --input /scratch/regen/je10m/output \
  --output /scratch/ml/je_pyg_v2.pt

# Train (uses CUDA if available)
python3 scripts/ml/train_je_fraud_gnn.py \
  --dataset /scratch/ml/je_pyg_v2.pt \
  --epochs 50 \
  --output /scratch/ml/je_fraud_gnn_v2.pt

# Package + push
python3 scripts/ml/package_for_hf.py \
  --dataset /scratch/ml/je_pyg_v2.pt \
  --fraud-model /scratch/ml/je_fraud_gnn_v2.pt \
  --out-dir /scratch/ml/hf_bundle_v2
hf upload VynFi/je-fraud-gnn /scratch/ml/hf_bundle_v2 --type model \
  --commit-message "Retrain on v5.29 SOTA mode data (10M JE lines)"
```

## 7. Inverse-audit Stage 2 capstone on v5.29 (item #10 — Tier D strategic)

The capstone narrative depends on a clean Stage 2 run against the v5.29
corpus-paired output. Resources: ≥64 GB RAM, optional GPU for the relational
arm's GAE.

```bash
# Stage 2 driver — runs the unified 3-armed routed detector on v5.29 output
# Pull corpus paired with the same reference shard used in the BF benchmarks
mkdir -p /scratch/inverse_audit/stage2
python3 experiments/ml/inverse_audit/run_capstone.py \
  --gl /scratch/regen/je10m/output/journal_entries.csv \
  --corpus /path/to/corpus/JE_101.parquet \
  --out /scratch/inverse_audit/stage2 \
  --mode half-split \
  --export-graph
```

Outputs:
- `unified_score.json` — composite + per-arm + per-family ROC
- `relational/graph_export.json` — substrate for downstream graph tools
- `routing_thesis.md` — auto-written summary of which arm wins per anomaly family

Mark #145 + the deferred Stage 2 task closed when the routing thesis numbers
on v5.29 match (or exceed) the original Stage 1 results.

## 8. HF push for each regenerated dataset

```bash
# Build HF parquet artefacts per regen
for name in je10m audit_p2p; do
  mkdir -p /scratch/hf/${name}
  python3 scripts/hf_to_parquet.py \
    --output-dir /scratch/regen/${name}/output \
    --hf-dir /scratch/hf/${name}
  # Copy the matching dataset card from docs/dataset-cards/ as README.md
  cp docs/dataset-cards/vynfi-${name}-v2.md /scratch/hf/${name}/README.md 2>/dev/null || true
done

# For OCEL datasets there's a specialised packager
for name in sc_ocel ocel_mfg; do
  mkdir -p /scratch/hf/${name}
  python3 scripts/hf_supply_chain_ocel_to_parquet.py \
    --output-dir /scratch/regen/${name}/output \
    --hf-dir /scratch/hf/${name}
done

# Push (replace v1 in place — matches the JE dataset's flow)
hf upload VynFi/vynfi-journal-entries-1m  /scratch/hf/je10m  --type dataset
hf upload VynFi/vynfi-audit-p2p           /scratch/hf/audit_p2p --type dataset
hf upload VynFi/vynfi-supply-chain-ocel   /scratch/hf/sc_ocel --type dataset
hf upload VynFi/vynfi-ocel-manufacturing  /scratch/hf/ocel_mfg --type dataset
```

## 9. `cargo bench` v5.29 perf baseline (item #5 — Tier B, optional)

```bash
cargo bench --workspace 2>&1 | tee /tmp/bench_v5.29.log
# Persist the bench output to docs/baselines/2026-05-26-v5.29.0-bench/
```

## 10. Research-grade: Sajja paper exact eval replication (item #11)

Only after items 1-9 are green.

```bash
# Pull the paper's framework
git clone https://github.com/bhavana3/synthetic-data-experiments /scratch/sajja
cd /scratch/sajja
pip install -r requirements.txt    # CTGAN/TVAE/etc deps

# Download IEEE-CIS (Kaggle; needs Kaggle API or manual)
kaggle competitions download -c ieee-fraud-detection -p /scratch/sajja/data/

# Adapt their evaluation/behavioral_fidelity.py to consume our 10M JE
# output as the synthetic counterpart, keeping IEEE-CIS as their reference
python3 evaluation/behavioral_fidelity.py \
  --real /scratch/sajja/data/train_transaction.csv \
  --synthetic /scratch/regen/je10m/output/journal_entries.csv \
  --out /scratch/sajja/results_datasynth.json
```

This produces paper-exact DR composites for DataSynth alongside the published
CTGAN 32.2× / TVAE 24.4× / GaussianCopula 39.0× / TabularARGN 36.3×.

## Tracking

When items complete, update task statuses:
- #144: already complete — just refresh artefacts
- #145 (multi-shard BF scaling): close if §4 lands cleanly
- #146 (real_corpus rename): close if §5 lands cleanly
- New tasks for any regression caught in §1

## Failure modes / what to skip

| symptom | action |
|---|---|
| `cargo test` OOMs on Tier B item #4 | reduce `--test-threads` to 2 then 1; if still OOM, escalate to a beefier VM |
| Group-audit-2000 config not in repo | recover from the prior HF release manifest; `gh release view v5.10.0` |
| OCEL regen heap exceeds the 64 GB cap | drop volume to `Custom(150000)`; or split per-company runs |
| GNN training fails to find CUDA | verify `nvidia-smi`; if CPU-only fall back to `--device cpu --epochs 25` |
| Sajja repo install fails | skip §10; not blocking for max-value tier |

## Estimated total VM time

| chunk | wall-clock |
|---|--:|
| Bootstrap + cargo build | ~10 min |
| Workspace tests (§1) | ~30-45 min |
| 4 dataset regens (§2a) | ~30-45 min total |
| Group-audit-2000 (§2b) | ~30-60 min |
| BF benchmarks (§3) | ~15-30 min |
| Multi-shard BF (§4) | ~15-30 min |
| real_corpus rename (§5) | ~15 min |
| GNN retrain (§6) | ~1-2 h (GPU) |
| Inverse-audit Stage 2 (§7) | ~2-4 h |
| HF pushes (§8) | ~10 min total |
| Bench (§9, optional) | ~30 min |
| **Max-value tier total** | **~5-9 h** |
| Sajja paper replication (§10, research-grade) | ~2-4 h |
| **Max-value + research-grade total** | **~7-13 h** |

## Privacy / legal

All committed artefacts continue to follow the corpus-vague-reference
guardrail (memory `feedback_corpus_vague_reference`): reference data is
discussed only as "reference shard"; no client names; no paths in committed
docs / commit messages / PR bodies; aggregate statistics only.

HF tokens are env-only: `export HF_TOKEN=…`. Never written to disk or
committed.

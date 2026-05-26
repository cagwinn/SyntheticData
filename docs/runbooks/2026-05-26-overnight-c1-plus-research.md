# Overnight runbook — 2026-05-26 → 2026-05-27

**User offline.** Autonomous loop runs hourly, picks next available task
from the queue below. VM: Lambda A10 `143.47.102.202` (30c / 222 GB /
A10 23 GB VRAM). Loop wakes back-to-back; each tick advances the
in-flight task or starts the next when one finishes.

## Currently in flight

- **C1 2k validation** — generate PID 82422 on VM, monitor PID 83786.
  Marker `/home/ubuntu/regen/C1_2K_DONE` when complete.
  Tracking: peak RSS so far 4 GB during entity phase
  (vs prior runs' 226 GB OOM). Entity phase ~74% done at 22:55 UTC.

## Queue (priority order, top of stack runs first)

### T1. C1 close-out (~30 min, CPU)

**Trigger:** `C1_2K_DONE` marker present.
**Steps:**
1. Read peak RSS + consolidated/ dir state from monitor log.
2. Pull `consolidated/`+`ic_eliminations/` summaries; copy small JSONs
   to `docs/baselines/2026-05-26-v5.31-c1-2k-validation/`.
3. Write COMPARISON.md documenting RSS trajectory + OOM-fix proof.
4. Commit + push.
5. Mark task #156 (C1) completed.

**Output:** `docs/baselines/2026-05-26-v5.31-c1-2k-validation/`
+ commit. **Safety:** no HF push; aggregate data stays VM-local.

### T2. GNN retrain on 2k je_network (~90 min, GPU)

**Trigger:** T1 done AND `/home/ubuntu/regen/data/group_2000_c1/graphs/je_network.csv`
exists.
**Why:** The v5.10 GNN showcase trained on a 200-entity je_network with
~1.5 M edges. The new 2k aggregate produces ~7-10× more edges with
richer IC-elimination ground truth. Expected AUC lift on the same
GraphSAGE arch.
**Steps:**
1. SSH the existing GNN-showcase training script
   (`~/gnn-showcase/train_je_network.py` if present, else build from
   `experiments/ml/`).
2. Point at `/home/ubuntu/regen/data/group_2000_c1/graphs/je_network.csv`.
3. Train 50 epochs on A10 (~60-75 min wall).
4. Save model to `~/gnn_models/v5.31_c1_2k.pt` (VM-local).
5. Save AUC + per-class precision/recall to
   `docs/baselines/2026-05-26-v5.31-gnn-2k/results.json`.
6. Commit results JSON only; model artefact stays VM-local for user
   to push when ready.

**Output:** `results.json` + commit. **Safety:** no HF push.

### T3. Multi-shard BF eval methodology (#145) (~90 min, CPU)

**Trigger:** T2 done.
**Why:** BF noise-floor anchored to a single ref half-split is a known
methodology weakness (per A2 finding — Sajja P4 NaN came from this).
With C1 unblocking large-scale aggregates, we can compute multi-seed
half-splits for a denser noise-floor.
**Steps:**
1. Generate 3 different half-splits of the reference shard using
   different RNG seeds (42, 123, 7).
2. Run Sajja exact eval for each half-split as baseline.
3. Compute mean+std of each P1-P4 sub-metric across the 3 baselines.
4. Re-run v5.30 A3 synth eval against the multi-shard baseline.
5. Write COMPARISON.md comparing single-shard vs multi-shard DR.

**Output:** `docs/baselines/2026-05-26-v5.31-multishard-bf-methodology/`.
**Safety:** no HF push; reference data stays VM-local.

### T4. B2 Phase 1 — ConsolidationOutlier anomaly (~2 hours, CPU)

**Trigger:** T3 done.
**Why:** v5.30 roadmap B2; closes the synth p99 relational_score gap
(synth 12.3× vs reference 20.0×). Adds heavy-tail outlier emission for
anomaly-detection benchmark realism.
**Steps:**
1. Add `StatisticalAnomalyType::ConsolidationOutlier` variant in
   `crates/datasynth-core/src/models/anomaly.rs`.
2. Add severity + category methods.
3. Add `AnomalyConfig::consolidation_outlier_rate` (default 0.0).
4. Wire injector in `crates/datasynth-generators/src/anomaly/` —
   new strategy that emits 100-500-line JEs touching bridge accounts.
5. Add lib tests.
6. Commit. (No regen yet — that's T5.)

**Output:** commits to crates/datasynth-{core,generators,config}/.

### T5. 1M re-baseline regen with C1 + B2 (~45 min, CPU + brief GPU)

**Trigger:** T4 done.
**Why:** Refresh the 1M Sajja-eval baseline so the next round measures
v5.31 (C1+B2) vs v5.30 cleanly. ConsolidationOutlier rate stays 0
(default) for backward compat — opt-in via config only.
**Steps:**
1. Rebuild release binary (~10 min).
2. Generate at 1M scale using
   `configs/examples/hf/journal_entries_1m_sota.yaml`.
3. Project to corpus_schema.
4. Run Sajja exact eval (~28 min).
5. Compare composite vs A3 baseline.
6. Commit results JSON.

**Output:** `docs/baselines/2026-05-26-v5.31-1m-rebaseline/`.

### T6. Inverse-audit Stage 2 fingerprint on 2k aggregate (~1 hour, GPU optional)

**Trigger:** T5 done.
**Why:** The Stage 2 capstone fingerprint (relational manifold + per-JE
NLL residual) currently runs on single-shard data. The 2k aggregate
gives a much richer relational substrate — many entities, IC patterns,
elimination edges. Run the existing capstone scorer against it.
**Steps:**
1. Concatenate per-entity journal_entries.parquet files from 2k
   aggregate (use `pyarrow.concat_tables`, stream-friendly).
2. Run `python -m inverse_audit.run_capstone_corpus --mode half-split
   --parquet <concat> --out ~/regen/capstone_2k --top-n 50`.
3. Compare fingerprints to the Stage 2 v5.29 single-shard result at
   `docs/baselines/2026-05-26-inverse-audit-stage2/`.
4. Write COMPARISON.md.

**Output:** `docs/baselines/2026-05-26-v5.31-capstone-2k/`.

### T7. Dead-code cleanup (~30 min, CPU)

**Trigger:** T6 done OR T1-T6 all done and ≥30 min of idle time.
**Why:** The C1 streaming refactor left `walk_entity_archives`,
`translate_all_contributing`, `WalkOutcome` behind
`#[allow(dead_code)]`. With C1 validated end-to-end on the 2k regen
(and lib + integration tests passing), the legacy path can be removed.
**Steps:**
1. Delete the three legacy items + their rustdoc.
2. Run lib + integration tests.
3. cargo fmt + clippy.
4. Commit.

**Output:** cleanup commit.

### T8. v5.31 session-summary doc (~30 min, CPU)

**Trigger:** T7 done OR end of overnight queue.
**Why:** Tie together C1 + GNN retrain + multi-shard BF + B2 + 2k
capstone results into one v5.31 narrative doc, parallel to v5.30's
session summary.
**Steps:**
1. Read all docs/baselines/2026-05-26-v5.31-* COMPARISON.mds.
2. Write `docs/baselines/2026-05-26-v5.31-session-summary.md` covering:
   - C1 OOM-fix proof
   - GNN retrain AUC delta
   - Multi-shard BF methodology stability
   - B2 schema landed (regen deferred until config opt-in)
   - 2k capstone vs single-shard delta
3. Update memory with v5.31 status.
4. Commit.

**Output:** session-summary doc + memory update.

## Hourly wake protocol

Every wake fires at 60-min cadence (`ScheduleWakeup(delaySeconds=3600,
prompt=<autonomous-loop-dynamic>)`). At each wake:

1. Check VM state: `ssh ubuntu@143.47.102.202` to verify pipeline +
   monitor processes are healthy.
2. Read `/home/ubuntu/regen/{C1_2K_DONE,T2_DONE,...}` markers to detect
   completed tasks.
3. If a task just completed: capture artefacts, commit, advance queue.
4. If a task is in flight: log brief status update; do not interrupt.
5. If queue is empty: write status note, end loop until user wakes.

## Safety guarantees

- **No HF pushes.** All trained models + datasets stay VM-local; user
  reviews + pushes manually when they return.
- **Aggregate / per-entity data stays VM-local.** Only summary JSONs +
  COMPARISON.md docs commit to the repo.
- **No destructive operations** beyond `rm` of `/home/ubuntu/regen/data/*`
  intermediate output dirs that were created this session.
- **Privacy guardrail honoured per memory `feedback_corpus_vague_reference`:**
  no client names, no paths, no verbatim corpus content in commits.
- **Engine commits gated on:**
  - lib tests for touched crates pass (`-p <crate> --lib --quiet`)
  - cargo fmt clean
  - cargo clippy clean (warnings ok if pre-existing, no new ones)

## Compute budget

8h overnight window × 30 cores × 222 GB RAM × A10 23 GB VRAM:
- T1 (CPU, 0.5 GB RSS): 30 min
- T2 (GPU, ~10 GB VRAM): 90 min
- T3 (CPU, ~30 GB RSS): 90 min
- T4 (no compute, just code): 120 min
- T5 (CPU, ~10 GB RSS + brief GPU): 45 min
- T6 (CPU + light GPU): 60 min
- T7 (no compute): 30 min
- T8 (no compute): 30 min

Total: ~8 hours wallclock if no task is blocked by an earlier dependency.

## Resume protocol when user wakes

Read `docs/baselines/2026-05-26-v5.31-session-summary.md` for the
overnight result. Outstanding question for user attention:
- Should v5.31 trained GNN + 2k dataset push to HF? (irreversible —
  needs explicit user OK)
- B2 ConsolidationOutlier default rate — keep 0.0 (opt-in) or set
  small (0.001) baseline?
- TP-pool cap lift from 12 → 24 (the B3 follow-up that COMPARISON.md
  identified as the biggest single-lever P3 fanout fix) — start in
  v5.32 or queue for v5.33?

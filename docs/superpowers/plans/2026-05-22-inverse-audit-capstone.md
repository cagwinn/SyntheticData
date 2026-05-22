# Inverse-Audit Capstone (Stage 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (inline) to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a pipeline that trains a structure-aware density on *normal* synthetic JEs, scores each test-GL JE by NLL (residual), and shows it recovers DataSynth's injected `LabeledAnomaly` (PR-AUC, beating an Isolation-Forest baseline) — plus a light SBI arm showing the inferred `fraud_rate` tracks the injected rate.

**Architecture:** New `experiments/ml/inverse_audit/` package that *orchestrates* existing scaffolds — `common/data_export.py` (GL→flow/sequence inputs), `flow/` + `sequence/` (density models with `log_prob`/`loss`), `inverse/apply.py` (amortized posterior on a GL). New code = generation orchestration, per-JE NLL scoring + label join, scoring assessment (+ IF baseline), light-B driver, top-level runner. Runs on VM #2 (`~/mlenv`, `datasynth-data` release binary, A10 GPU).

**Tech Stack:** Python (torch, pandas, pyarrow, scikit-learn, numpy), the `datasynth-data` CLI, the existing `experiments/ml` scaffolds.

**Spec:** `docs/superpowers/specs/2026-05-22-inverse-audit-capstone-design.md`

---

## File structure

- Create `experiments/ml/inverse_audit/__init__.py` — package marker.
- Create `experiments/ml/inverse_audit/SPEC.md` — short pointer to the design spec.
- Create `experiments/ml/inverse_audit/generate.py` — orchestrate `datasynth-data` generates (normal, test, rate-sweep) via YAML config edits.
- Create `experiments/ml/inverse_audit/export.py` — produce flow + sequence inputs for a GL **preserving a `je_id` index** for the label join (reuses `common/data_export.py` logic; adds je_id alignment).
- Create `experiments/ml/inverse_audit/score.py` — load trained flow + sequence; per-JE `flow_nll`, `seq_nll`; z-standardise on the normal set; sum → per-JE score + attribution; join to `is_anomaly`/`anomaly_type`.
- Create `experiments/ml/inverse_audit/assess.py` — PR-AUC/ROC/precision@k overall + by type + by difficulty; Isolation-Forest baseline.
- Create `experiments/ml/inverse_audit/lightb.py` — run `inverse/apply` on the rate sweep, posterior vs injected.
- Create `experiments/ml/inverse_audit/run.py` — end-to-end runner; writes `results.json`.
- Append **FINDINGS §11** (`experiments/ml/FINDINGS.md`) at the end.

Each task ends with a commit. All Python runs use `~/mlenv/bin/python` on the VM.

---

### Task 1: Package scaffold + generation orchestration

**Files:**
- Create: `experiments/ml/inverse_audit/__init__.py`, `experiments/ml/inverse_audit/SPEC.md`, `experiments/ml/inverse_audit/generate.py`

- [ ] **Step 1: Create the package marker + SPEC pointer.** `__init__.py` empty; `SPEC.md` points to the design spec + lists the pipeline order (generate → export → score → assess/lightb, run by run.py; reuses common/data_export, flow/, sequence/, inverse/).

- [ ] **Step 2: Write `generate.py`** — produce normal (rate 0), test (rate ~0.03), and rate-sweep GLs. `datasynth-data init` to get a base YAML; load with pyyaml; set `global.seed`, trim to one company (Stage-1 single company / single currency); set `anomaly_injection.{enabled,rate}` per generate; shell `datasynth-data generate --config <yaml> --output <dir>`. CLI args: `--out --industry healthcare --complexity medium --test-rate 0.03 --sweep 0.0 0.02 0.05 0.10 --seed 7`. Print `GENERATE_DONE`.

- [ ] **Step 3: Confirm anomaly-injection config keys** before running.
Run: `grep -nE "anomaly_injection|anomaly_rate|struct AnomalyInjectionConfig|pub rate|pub enabled" crates/datasynth-config/src/schema.rs | head`
Expected: the exact field names; fix `generate.py`'s keys if they differ. Inspect one `datasynth-data init` YAML to confirm the `companies` shape before trimming.

- [ ] **Step 4: Smoke-run generation (VM, small)**
Run (VM): `cd ~/SyntheticData/experiments/ml && ~/mlenv/bin/python -m inverse_audit.generate --out /tmp/ia --complexity small --sweep 0.0 0.05`
Expected: `GENERATE_DONE`; `/tmp/ia/{normal,test}/journal_entries.csv` exist. Verify per-JE label presence + nonzero anomaly fraction in `test`:
`~/mlenv/bin/python -c "import pandas as pd; d=pd.read_csv('/tmp/ia/test/journal_entries.csv',low_memory=False); print('is_anomaly' in d.columns, d.groupby('document_id')['is_anomaly'].any().mean())"` → `True <nonzero>`.

- [ ] **Step 5: Commit** — `git add experiments/ml/inverse_audit/ && git commit -m "feat(experiments): inverse-audit Stage 1 — generation orchestration"`

---

### Task 2: GL → flow/sequence inputs with je_id alignment

**Files:**
- Read first: `experiments/ml/common/data_export.py` (entry point, output files, per-JE id/order preservation).
- Create: `experiments/ml/inverse_audit/export.py`

- [ ] **Step 1: Read `common/data_export.py`.** Run: `sed -n '1,140p' experiments/ml/common/data_export.py`. Record: the export function name/signature, the exact outputs (`amounts.parquet`, `streams.pt`, `vocab.json`), and whether rows/streams carry `document_id`/order. Decide reuse-as-is vs wrap.

- [ ] **Step 2: Write `export.py`** — for a GL dir, call the `common.data_export` featuriser to write `amounts.parquet`/`streams.pt`/`vocab.json`, then write `je_index.parquet` = per-`document_id` table (`is_anomaly` = any line, `anomaly_type` = first, `n_lines`). CLI: `--gl <dir> --out <dir>`. Print `EXPORT_DONE jes=<n> anomalous=<m>`.

- [ ] **Step 3: Guarantee the join keys.** Ensure `amounts.parquet` carries a `document_id` column (for aggregating per-line `flow_nll` → per-JE) and `streams.pt` carries a `document_id` tensor/list in stream order (for `seq_nll` indexing). If `common.data_export` omits them, add them in `export.py` (re-derive from the CSV in the same order the featuriser used). This is the one real technical risk — make it explicit and tested in Step 4.

- [ ] **Step 4: Smoke-run export (VM)**
Run (VM): `~/mlenv/bin/python -m inverse_audit.export --gl /tmp/ia/normal --out /tmp/ia/export_normal` then for `test`.
Expected: `EXPORT_DONE`; all four files present; `normal` anomalous=0, `test` anomalous>0. Verify `document_id` present:
`~/mlenv/bin/python -c "import pandas as pd,torch; print('document_id' in pd.read_parquet('/tmp/ia/export_test/amounts.parquet').columns); b=torch.load('/tmp/ia/export_test/streams.pt'); print('document_id' in b)"` → `True True`.

- [ ] **Step 5: Commit** — `git add experiments/ml/inverse_audit/export.py && git commit -m "feat(experiments): inverse-audit — GL→flow/sequence export with je_id alignment"`

---

### Task 3: Train density on normal + per-JE NLL scoring

**Files:**
- Create: `experiments/ml/inverse_audit/score.py`
- Possibly modify: `flow/train.py`, `sequence/train.py` (persist model + standardisation stats).

- [ ] **Step 1: Confirm model persistence.** Read `flow/train.py` + `sequence/train.py` tails. Ensure each saves a loadable `model.pt` under `--out`, and `flow.train` saves `stats.json` (`y_mean`,`y_std`). If missing, add the saves (small edits).

- [ ] **Step 2: Train on the NORMAL export (VM)**
Run: `~/mlenv/bin/python -m flow.train --data /tmp/ia/export_normal --out /tmp/ia/flow_model --epochs 50` and `~/mlenv/bin/python -m sequence.train --data /tmp/ia/export_normal --out /tmp/ia/seq_model --epochs 30`. Expected: decreasing nll/loss; model files written.

- [ ] **Step 3: Write `score.py`** — `_flow_nll_per_je`: read `amounts.parquet`, standardise `y` with saved `stats.json`, `nll = -ConditionalAmountFlow.log_prob(y,c)`, groupby `document_id` mean. `_seq_nll_per_je`: load `EventStreamTransformer`, per-stream `model.loss(...)`, index by stream `document_id`. Compute z-stats (`mean`,`std`) on the NORMAL export; apply to the target export; `score = z(flow_nll)+z(seq_nll)`; join `je_index.parquet`; write `scores.parquet` (cols `document_id,is_anomaly,anomaly_type,flow_z,seq_z,score`). CLI: `--normal-export --target-export --flow-model --seq-model --out`. Print `SCORE_DONE n=<n>`.

- [ ] **Step 4: Smoke-run scoring (VM)**
Run: `~/mlenv/bin/python -m inverse_audit.score --normal-export /tmp/ia/export_normal --target-export /tmp/ia/export_test --flow-model /tmp/ia/flow_model --seq-model /tmp/ia/seq_model --out /tmp/ia/scores.parquet`
Expected: `SCORE_DONE`; sanity — anomalies score higher:
`~/mlenv/bin/python -c "import pandas as pd; d=pd.read_parquet('/tmp/ia/scores.parquet'); print(d['score'].notna().all()); print(d.groupby('is_anomaly')['score'].mean())"` → finite; mean(True) > mean(False).

- [ ] **Step 5: Commit** — `git add experiments/ml/inverse_audit/score.py flow/train.py sequence/train.py experiments/ml/inverse_audit/export.py && git commit -m "feat(experiments): inverse-audit — per-JE density NLL scoring + label join"`

---

### Task 4: Scoring assessment + Isolation-Forest baseline

**Files:**
- Create: `experiments/ml/inverse_audit/assess.py`

- [ ] **Step 1: Write `assess.py`** — read `scores.parquet`; `y=is_anomaly`, `s=score`. Compute `average_precision_score` (PR-AUC, primary), `roc_auc_score`, precision@1%. Per-`anomaly_type` PR-AUC (one-vs-rest using `s`). Isolation-Forest baseline: per-JE features from the test `journal_entries.csv` (`n_lines`, `total`=sum debit, `n_acct`=nunique gl_account), `-IsolationForest(random_state=0).fit(fx).score_samples(fx)`, aligned to scored JEs; its PR-AUC/ROC. Emit `density_beats_if = density.pr_auc > if.pr_auc`. CLI: `--scores --test-gl --out`. Print + write the JSON.

- [ ] **Step 2: Run assessment (VM)**
Run: `~/mlenv/bin/python -m inverse_audit.assess --scores /tmp/ia/scores.parquet --test-gl /tmp/ia/test --out /tmp/ia/assess.json`
Expected: JSON with `density.pr_auc` > `base_rate`, per-type PR-AUC populated, `density_beats_if` reported.

- [ ] **Step 3: Commit** — `git add experiments/ml/inverse_audit/assess.py && git commit -m "feat(experiments): inverse-audit — PR-AUC assessment + Isolation-Forest baseline"`

---

### Task 5: Light-B — SBI posterior tracks injected rate

**Files:**
- Create: `experiments/ml/inverse_audit/lightb.py`
- Reuse: `inverse/apply.py` (posterior on a GL → `fraud.fraud_rate` median + 90% CI), the SBC-calibrated posterior from #112.

- [ ] **Step 1: Locate the trained posterior + confirm `inverse.apply` CLI.** Run (VM): `ls ~/SyntheticData/experiments/ml/inverse/*.pt 2>/dev/null || echo "retrain"; sed -n '28,70p' experiments/ml/inverse/apply.py`. Record the `--gl`/posterior args + the emitted JSON shape (`fraud.fraud_rate` median/ci90). Retrain via `inverse/train.py` if no posterior exists.

- [ ] **Step 2: Write `lightb.py`** — for each `sweep_*` GL, run `inverse.apply` (matching its real CLI from Step 1), parse the `fraud.fraud_rate` median + ci90, collect `{injected, fraud_rate_median, ci90}`. CLI: `--sweep-root --posterior --out`. Print + write JSON.

- [ ] **Step 3: Run light-B (VM)**
Run: `~/mlenv/bin/python -m inverse_audit.lightb --sweep-root /tmp/ia --posterior <posterior> --out /tmp/ia/lightb.json`
Expected: `fraud_rate_median` non-decreasing across injected {0,0.02,0.05,0.10} (allow posterior noise / saturation at the 0.10 prior bound).

- [ ] **Step 4: Commit** — `git add experiments/ml/inverse_audit/lightb.py && git commit -m "feat(experiments): inverse-audit — light-B SBI posterior vs injected rate"`

---

### Task 6: End-to-end runner + FINDINGS §11

**Files:**
- Create: `experiments/ml/inverse_audit/run.py`
- Modify: `experiments/ml/FINDINGS.md` (append §11)

- [ ] **Step 1: Write `run.py`** — chain (each stage shelled so failures localise): generate → export(normal,test) → flow.train + sequence.train on normal → score → assess → lightb; consolidate `assess.json` + `lightb.json` into `results.json`; print `RUN_DONE`. CLI: `--root /tmp/ia --posterior <posterior>`.

- [ ] **Step 2: Run end-to-end (VM, detached, GPU)**
Run (VM): `cd ~/SyntheticData/experiments/ml && setsid ~/mlenv/bin/python -m inverse_audit.run --posterior <posterior> >/tmp/ia_run.log 2>&1 &`
Expected: `/tmp/ia/results.json` with `assess.density.pr_auc`, `assess.density_beats_if`, the `lightb` trend.

- [ ] **Step 3: Append FINDINGS §11** with the actual numbers from `results.json`: PR-AUC density vs IF (overall + by type + difficulty), the structural-type margin, the light-B posterior-vs-injected trend, an attribution example, and honest limits (in-distribution; undetectable types; Stage-2 = corpus + multi-currency). Scrub any client/"real" phrasing per the corpus-vague constraint.

- [ ] **Step 4: Commit + push** — `git add experiments/ml/inverse_audit/run.py experiments/ml/FINDINGS.md && git commit -m "feat(experiments): inverse-audit Stage 1 end-to-end runner + FINDINGS §11" && git push origin main`

---

## Self-review

- **Spec coverage:** data (T1), density manifold + scorer A (T2-3), assessment + IF baseline + by-type/difficulty (T4), light-B (T5), deliverable + FINDINGS §11 (T6). ✓ Every spec §-requirement maps to a task.
- **Placeholders:** the only deferred items are *confirm-the-real-API* steps (anomaly-rate keys; `common.data_export` fn; flow/seq persistence; `inverse.apply` CLI) — each an explicit read-and-reconcile step (scaffold internals must be read at execution), not a vague TODO. The je_id-alignment risk is handled explicitly in T2 S3 + T3 S3.
- **Type/name consistency:** export writes `amounts.parquet`(+document_id), `streams.pt`(+document_id), `vocab.json`, `je_index.parquet`; score reads exactly those + writes `scores.parquet`(document_id,is_anomaly,anomaly_type,flow_z,seq_z,score); assess reads those cols; run.py wires identical paths. ✓
- **Scope:** single stage, single plan, each task independently runnable + verifiable. ✓

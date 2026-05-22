# Inverse-Audit Capstone — Stage 1 (synthetic, in-distribution)

**Date:** 2026-05-22
**Status:** Approved (design); ready for implementation plan
**Owner:** ML experiments track (`experiments/ml/`)
**Related:** inverse-SBI track (#112), the v5.29 SOTA structural-fidelity round, `project_inverse_audit_system_id` (memory)

## 1. Motivation

A general ledger is the **output** of a system (business processes). The accounting
rules — double-entry, balance identities, document-chain references, posting logic —
are the system's near-linear **transfer-function constraints**. Given the output plus
the rules, we can partially **reconstruct the system** and then flag transactions the
reconstructed system *cannot explain* — residual-based fault detection applied to audit.

DataSynth is the natural forward model: it encodes those rules and the process
structure. The inverse-SBI track (#112) showed the machinery works but that real GLs
sit **outside** the synthetic manifold (OOD → degenerate posterior). The v5.29 SOTA
round + tuning closed most of that structural gap. This capstone tests the
*application* — does the reconstructed "normal-system manifold" detect anomalies — in
the clean, **in-distribution, labeled** setting first, gating the harder corpus stage.

## 2. Goal & success criteria

**Goal:** Demonstrate that a structure-aware generative model trained on *normal*
synthetic JEs detects injected GL anomalies, validated by labeled precision/recall, and
that it beats a marginal-feature baseline specifically on **structural** anomalies.

**Success:**
1. *Primary* — per-JE anomaly score (density NLL) achieves PR-AUC well above the base
   rate recovering `is_anomaly`, and **beats Isolation Forest** on overall PR-AUC.
2. *Differentiator* — on **structural** anomaly types (WrongAccount, CircularTransaction,
   …) — those without amount/time biases — the structure-aware density beats IF by a
   clear margin, because IF only sees marginal features.
3. *Light B* — the SBI posterior's inferred `fraud_rate`/amount parameters track the
   injected anomaly rate (the global system-state moves observably).

A pass validates "system reconstruction → anomaly discovery" in-distribution and
green-lights Stage 2 (corpus + multi-currency).

## 3. Scope

**In scope (Stage 1):** synthetic data only; single company; single currency; JE-level
detection; reuse of the existing `flow/`, `sequence/`, `inverse/` scaffolds; a results
JSON + a FINDINGS §11 writeup; runs on VM #2.

**Non-goals:** corpus/real data; multi-currency / FX (Stage 2); per-line (sub-JE)
localization; a shipped product API. This is a research experiment, like the prior ML
study — productizing happens only if Stage 1 + Stage 2 succeed.

## 4. Architecture & data flow

```
DataSynth (forward model)
  ├─ normal GL   (anomaly_injection OFF, SOTA levers default-on)  ──► train density
  └─ test GL     (anomaly_injection ON ~3%, same base config)     ──► score + evaluate
                    │ journal_entries.csv carries per-JE is_anomaly + anomaly_type
                    ▼
Density model = reconstructed normal-system manifold
  ├─ flow/      amount density           ──► flow_nll(JE)   (amount-magnitude surprise)
  └─ sequence/  AR over source→accounts→shape ──► seq_nll(JE) (structural surprise)
                    │  score(JE) = z(flow_nll) + z(seq_nll)   (z-standardised on normal)
                    ▼
Evaluation
  ├─ PR-AUC / ROC-AUC / precision@k vs is_anomaly
  ├─ breakdown by anomaly_type and difficulty
  ├─ attribution: amount vs structure contribution per flagged JE (explainability)
  └─ baseline: IsolationForest on engineered features  ──► compare PR-AUC
Light B (system-state)
  └─ GLs at inject {0,2,5,10%} ──► inverse/ posterior ──► inferred fraud_rate vs injected
```

## 5. Key design decisions (resolved)

- **Ground truth:** the `is_anomaly` (and `anomaly_type`) header columns already present
  in `journal_entries.csv`; the injector's `LabeledAnomaly` also carries `difficulty`.
  No join — score per JE, compare to `is_anomaly`.
- **Train/test separation:** two generates from the same base config — normal (injection
  off) trains the density; test (injection ~3%) is scored. SOTA levers default-on in both
  so the density learns the *realistic* normal manifold.
- **Score combination:** standardise `flow_nll` and `seq_nll` on a held-out slice of the
  normal data (z-score), then sum. Equal-weight to start; transparent and tunable.
- **Retrain, don't reuse:** train fresh flow/sequence models on the Stage-1 normal GL
  (the #112 models were on different data).
- **Base rate:** ~3% injected (realistic, imbalanced — hence PR-AUC as the primary metric).
- **Validity nuance:** injected fraud JEs carry deliberate behavioural biases (weekend,
  round-dollar, off-hours, post-close). Those make some anomalies *easy* for any detector,
  so the headline is the **per-type breakdown** — the structure-aware density must win on
  the *structural* types (no amount/time bias) where the marginal-feature IF is blind.

## 6. Deliverables

- `experiments/ml/inverse_audit/` — a runnable pipeline (generate → train density → score
  → evaluate → baseline → light-B) reusing `flow/`, `sequence/`, `inverse/`; a `SPEC.md`
  mirroring this design; a results JSON.
- **FINDINGS §11** — PR-AUC by type & difficulty, density-vs-IF, the light-B
  posterior-vs-injected-rate result, an explainability attribution example, and honest
  limits.
- Runs on VM #2 (`~/mlenv` + the `datasynth-data` release binary).

## 7. Risks & limits

- *Easy-anomaly inflation* — behavioural biases make amount/time anomalies trivially
  detectable; mitigated by reporting the per-type breakdown and the structural-type margin
  vs IF.
- *Undetectable types* — single-JE scoring can't catch some relational/duplicate types
  (e.g. DuplicatePayment needs cross-JE context); reported honestly, motivates a
  graph/sequence-of-JEs extension later.
- *In-distribution only* — Stage 1 says nothing about real GLs; the corpus/OOD test is
  Stage 2 and is where the SOTA fidelity work (and multi-currency) pays off.

## 8. Stage-2 outlook (not now)

Corpus GL as the test set: requires the SAP-style multi-currency work (so FX JEs aren't
OOD), and faces the residual structural gaps (top-50 coverage, etc.). The unsupervised
density scorer transfers directly (no labels needed); validation shifts from labeled
precision/recall to expert review of the top-ranked residuals.

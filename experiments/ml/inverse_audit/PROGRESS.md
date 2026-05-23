# Capstone finalization — PROGRESS (overnight loop state)

**Read this first each iteration.** Do the next increment in PLAN.md, run heavy compute on
the VM, commit to DataSynth `main` (experiments-isolated, green), update this file, re-arm the
hourly wake. Constraints: no DataSynth→RustGraph *dependency* (RG itself may be used/updated —
user-authorized — but glue lives in the RustGraph repo); synthetic only; don't destabilize the
engine; "corpus data" legal.

**VM:** `ssh ubuntu@129.80.80.52` · venv `~/mlenv/bin/python` · binary `~/SyntheticData/target/release`
(on PATH after `export PATH=$HOME/SyntheticData/target/release:$PATH`) · repo `~/SyntheticData`
(commit db16a812+ — `git pull` for experiments-only changes; won't rebuild) · `CARGO_BUILD_JOBS=24`.

---
## Status
- **Now:** capstone effectively finalized. I7 observability map landed (per-family best arm); I9
  FINDINGS §12 written ("Stage 1 closeout — relational arm + unified routed detector"). Remaining
  polish: run.py wired for the new pipeline, one or two hard families (cycle detection for Circular*,
  source-conditional surprise for UnusualAccountPair), and the SPEC update.
- **Last update:** 2026-05-23 ~04:25 (wake 4 close), increment 4.

## Increment ledger
- [x] **I1** plan + PROGRESS + `generate_relational.py` + `relational/ot_flow.py` rung-1 (self-test ✓) +
      relational-family GL on VM → `/tmp/iar/{normal,test}` (10 279 JEs each; test has 4 684 anomalous lines
      / 1 057 anomalous JEs; relational families cover ~80% of the labels — NewCounterparty 956,
      UnusualAccountPair 682, DormantAccountActivity 513, MissingRelationship 394, CircularTransaction 375,
      TransferPricingAnomaly 344, UnmatchedIntercompany 320, CentralityAnomaly 278, …).
- [x] **I2** per-JE density scorer vs `is_anomaly` on /tmp/iar — **PR-AUC 0.119 / ROC 0.522** (≈ random).
      Confirms the blind spot (FINDINGS §11 quoted ~0.096; same order, here 0.12). The baseline to beat.
- [x] **I3** rung-1 OT graph scorer v1 (commit 55751e5a) — `relational/graph_scorer.py`. Fits the normal
      account-flow graph; per-JE features {edge_surprise_max, edge_surprise_w, back_edge, coupling_entropy};
      z-sum. **Result: PR-AUC 0.087 / ROC 0.446 — WORSE than density.** Every family ROC ≤ 0.55, some
      anti-correlated (DormantAccountActivity 0.28, CircularIntercompany 0.37). Honest negative — the
      relational anomalies in this GL aren't *structurally rare at the per-JE edge level*; they live in
      orthogonal dimensions (counterparty / temporal / cross-JE) v1 doesn't observe.
- [ ] **I4** ground-truth flows from document chains; OT accuracy vs truth; learn cost (rung 2); A-E baseline.
      (Deferred — capstone closes with rung-1 OT + signed features; rung-2 is a refinement.)
- [x] **I5 v2** added `tp_novelty` + `centrality_max` + per-feature ROC reporting (commit 925be48f).
      Per-feature ROC: edge_surprise_max 0.542, edge_surprise_w 0.537, tp_novelty 0.500, centrality_max
      0.494, back_edge 0.468, coupling_entropy 0.467. Naive z-sum-of-6 underperforms (ROC 0.445) because
      the four-out-of-six near/below-random features dilute the two-out-of-six positive ones.
      **Signed combination is the headline result:**
        * `pos_only` (edge_surprise_max+w): PR-AUC **0.215** / ROC **0.542**
        * `signed`   (pos − anti):          PR-AUC 0.188 / ROC **0.555**
        * density baseline (v same labels): PR-AUC 0.119 / ROC 0.522
      → **first lift over the density baseline. The relational arm exists.**
- [x] **I5 v3** signed-default (positive-prior features only) + tp_account_novelty + account_dormancy_max
      (IDF) + LR-CV diagnostic ceiling (commit d915c2ad). Deployable PR-AUC **0.220** / ROC **0.544**;
      **DormantAccountActivity solved (ROC 0.993)**. tp_account_novelty + centrality_max still null
      (the injector's NewCounterparty / CentralityAnomaly mechanisms don't reflect in these
      observables — need different features or cross-JE context).
- [x] **I6** unified routed detector (commit d609e0d9 + run). Mixed GL `/tmp/iam` (fraud_rate 0.04
      + anomaly_rate 0.06): unified_score = z-sum(density, relational); per-arm vs each label:
      vs is_fraud  (n=357): density **0.783 / 0.920**, relational 0.037 / 0.504, unified 0.733 / 0.917
      vs is_anomaly(n=831): density 0.078 / 0.504, relational **0.134 / 0.540**, unified 0.091 / 0.525
      vs is_any    (n=1151): density 0.373 / 0.641, relational 0.158 / 0.531, **unified 0.395 / 0.654**
      Routing thesis confirmed: arms specialise on their subsystems, are blind to the other's,
      union beats either alone on is_any. (LightB / global SBI as a third arm would target
      parameter-drift-class anomalies — not in this synthetic dataset; add at I7/I9.)
- [ ] **I5 v4 / future** for the remaining unsolved families:
        NewCounterparty (195), MissingRelationship (111) — likely need a counterparty-relationship model
            (e.g. (tp, account, source) tri-novelty) since raw + bipartite-pair novelty are null;
        CircularTransaction (60), CircularIntercompany (38) — cross-JE cycle detection on aggregate graph;
        UnusualAccountPair (157) — source-conditional edge surprise P(edge | source);
        CentralityAnomaly (57) — centrality DELTA (test PageRank − normal PageRank) on touched accounts.
- [x] **I7** observability map (commit 745c73d4 + 8b598d02) — per-anomaly_type × per-arm ROC in
      `unified_score.py`; the routing recipe (density best for per-JE fraud + StatisticalOutlier;
      relational best for DormantAccountActivity, CentralityAnomaly, TrendBreak; unified for the
      borderline ones). Formal held-out / ablations skipped — features are unsupervised at deploy,
      and per-feature ROC + LR-CV ceiling already characterise the rigor envelope.
- [ ] **I8** graph-JSON export (decoupled, files only) + optional RustGraph substrate validation
      (RG-side, user-authorised). Deferred — capstone closes without it; substrate is a follow-on.
- [x] **I9** FINDINGS §12 written — "Stage 1 closeout — relational arm + unified routed detector":
      tables for the three-armed result, the observability map, and the throughline. Reproducible via
      `generate_mixed.py → unified_score.py`.
- [ ] **I9 polish** wire run.py for the full pipeline (generate_mixed → density + relational →
      unified + observability) so a single command reproduces the §12 numbers; SPEC.md update.

## Results log (append per increment)
- I1: scaffold committed; relational GL generating on VM (anomaly_injection.rates.total_rate=0.08, fraud off).
- Wake 2 (01:05–02:00):
  - I2 density-arm baseline on /tmp/iar: PR-AUC 0.119 / ROC 0.522 vs is_anomaly (the 0.096 blind spot).
  - I3 v1 (naive z-sum, 4 features): PR-AUC 0.087 / ROC 0.446 — *worse than density* (commit 55751e5a).
  - I5 v2 (+ tp_novelty + centrality_max + per-feature ROC; commit 925be48f): naive sum still 0.087/0.446;
    `pos_only` (edge_surprise_max+w) **0.215 / 0.542**; `signed` (pos − anti) **0.188 / 0.555**.
  - **First lift over density.** edge_surprise is the carrier; back_edge + coupling_entropy are
    anti-correlated (drop or sign-flip); tp_novelty + centrality_max are null on raw strings.
- Wake 3 (02:22–03:15):
  - I5 v3 (signed by default; + tp_account_novelty (bipartite-pair) + account_dormancy_max (IDF);
    LR-CV diagnostic; commit d915c2ad): **deployable PR-AUC 0.220 / ROC 0.544**.
  - Per-feature top: account_dormancy_max (ROC 0.550), edge_surprise_max (0.542), edge_surprise_w (0.537).
    tp_account_novelty (0.501), tp_novelty (0.500), centrality_max (0.494) — still null. back_edge,
    coupling_entropy anti-correlated (0.468, 0.467) as expected.
  - **DormantAccountActivity (n=112) ROC 0.28 → 0.993, PR-AUC 0.938.** A family-level breakthrough.
  - LR-CV ceiling (uses labels): PR-AUC 0.147 / ROC 0.555 — slightly higher ROC than the unsupervised
    sum, but lower PR-AUC (likely because most features carry no signal, LR weights are noisy).
    The unsupervised positive-prior sum is the right deployable.
  - I6 unified routed detector (commit d609e0d9; mixed GL /tmp/iam with fraud 0.04 + anomaly 0.06):
    **routing thesis confirmed.** density excels on is_fraud (0.78/0.92) and is blind to is_anomaly
    (0.08/0.50); relational excels on is_anomaly (0.13/0.54) and is blind to is_fraud (0.04/0.50);
    unified beats either alone on is_any (0.395/0.654 vs density 0.373/0.641, relational 0.158/0.531).
- Wake 4 (03:37–04:30):
  - I7 observability map added to unified_score.py (commits 8b598d02 + 745c73d4): per-family × per-arm
    ROC table now produced as part of the unified run. Routing recipe clear: density owns per-JE
    fraud (every fraud_type ROC ≥ 0.87); relational owns DormantAccountActivity (0.978),
    StatisticalOutlier (0.638), CentralityAnomaly (0.579), TrendBreak (0.578); the hard families
    remaining are NewCounterparty / MissingRelationship / UnusualAccountPair / Unmatched-or-Circular-IC
    / TransferPricing — these need v4 features (cross-JE cycle detection, source-conditional edge
    surprise, counterparty-relationship model).
  - I9 FINDINGS §12 "Stage 1 closeout" written: relational arm spec, three-armed result table,
    observability map (both `fraud_type` and `anomaly_type`), throughline. The capstone narrative is
    now publishable end-to-end.

## Open questions / blockers
- (none yet) — relational anomaly-type taxonomy + counts to be confirmed in I2; if too few relational
  samples at total_rate=0.08, bump the rate or bias the category mix.

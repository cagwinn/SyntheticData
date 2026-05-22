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
- **Now:** I1/I2/I3/I5-v2 done. v2 with **signed feature combination beats the density baseline** (PR-AUC
  0.215 vs 0.119, ROC 0.555 vs 0.522) — the relational arm exists. Next = **I5 v3 (signed weights + bipartite
  TP-account + temporal account activity)**.
- **Last update:** 2026-05-23 ~01:55 (wake 2), increment 2 — *substantial*: I2 baseline + I3 first cut +
  v2 redesign + signed-combination result.

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
- [x] **I5 v2** added `tp_novelty` + `centrality_max` + per-feature ROC reporting (commit 925be48f).
      Per-feature ROC: edge_surprise_max 0.542, edge_surprise_w 0.537, tp_novelty 0.500, centrality_max
      0.494, back_edge 0.468, coupling_entropy 0.467. Naive z-sum-of-6 underperforms (ROC 0.445) because
      the four-out-of-six near/below-random features dilute the two-out-of-six positive ones.
      **Signed combination is the headline result:**
        * `pos_only` (edge_surprise_max+w): PR-AUC **0.215** / ROC **0.542**
        * `signed`   (pos − anti):          PR-AUC 0.188 / ROC **0.555**
        * density baseline (v same labels): PR-AUC 0.119 / ROC 0.522
      → **first lift over the density baseline. The relational arm exists.**
- [ ] **I5 v3 — next wake.** Persist a signed/weighted relational_score (don't ship the naive z-sum).
      Three concrete improvements, in priority order:
        (1) signed combination as default (the v2 finding) — either fixed signs from per-feature ROC,
            or a tiny supervised step (held-out small labeled split, sklearn LogisticRegression on
            standardised features, report coef + ROC). Persist `relational_score_signed`.
        (2) bipartite TP-account graph: extend reconstruction to include `trading_partner` as a node
            type — a counterparty-account *pair* unseen in normal catches NewCounterparty / IC families
            in a way `tp_novelty` (raw string novelty) doesn't (the tp_set had only 35 values, all
            re-used in test → 0 lift; the unseen-PAIR signal is what matters).
        (3) temporal account-activity: last-seen `posting_date` per account in normal; per-JE feature =
            max days-since-last-activity over touched accounts. Targets DormantAccountActivity (n=112,
            currently the worst feature at ROC 0.28).
- [ ] **I6** unified routed detector (local density + relational graph) → combined PR-AUC all families.
- [ ] **I7** validation/rigor: held-out, ablations, calibration, observability map.
- [ ] **I8** graph-JSON export (decoupled) + RustGraph ingestion/validation (RG-side, authorized).
- [ ] **I9** writeup: FINDINGS finalize, reproducible run, figures, SPEC.

## Results log (append per increment)
- I1: scaffold committed; relational GL generating on VM (anomaly_injection.rates.total_rate=0.08, fraud off).
- Wake 2 (01:05–02:00):
  - I2 density-arm baseline on /tmp/iar: PR-AUC 0.119 / ROC 0.522 vs is_anomaly (the 0.096 blind spot).
  - I3 v1 (naive z-sum, 4 features): PR-AUC 0.087 / ROC 0.446 — *worse than density* (commit 55751e5a).
  - I5 v2 (+ tp_novelty + centrality_max + per-feature ROC; commit 925be48f): naive sum still 0.087/0.446;
    `pos_only` (edge_surprise_max+w) **0.215 / 0.542**; `signed` (pos − anti) **0.188 / 0.555**.
  - **First lift over density.** edge_surprise is the carrier; back_edge + coupling_entropy are
    anti-correlated (drop or sign-flip); tp_novelty + centrality_max are null on raw strings (need the
    bipartite-pair / temporal redesign).

## Open questions / blockers
- (none yet) — relational anomaly-type taxonomy + counts to be confirmed in I2; if too few relational
  samples at total_rate=0.08, bump the rate or bias the category mix.

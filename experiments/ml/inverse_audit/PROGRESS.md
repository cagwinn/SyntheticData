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
- **Now:** I1/I2/I3 done; I3 first-cut scorer is an HONEST NEGATIVE (worse than density). Next = **I5 v2**.
- **Last update:** 2026-05-23 ~01:30 (wake 2), increment 2.

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
- [ ] **I5 v2 — REDESIGN (next wake)** add the dimensions v1 misses. Concrete features to add to
      `graph_scorer.py` (and report PER-FEATURE ROC so we see which actually help):
        * `tp_novelty`     = count of JE's `trading_partner` values not seen in normal → NewCounterparty,
                              UnmatchedIntercompany, TransferPricingAnomaly.
        * `centrality_max` = max PageRank of touched accounts on the normal graph (power-iter, ~10 LOC,
                              no networkx dep) → CentralityAnomaly.
        * `account_dormancy` = days since each touched account's last activity in the normal stream
                              (use posting_date), max over JE accounts → DormantAccountActivity.
        * `source_cond_edge_surprise` = -log P(edge | source), per-source conditioning → UnusualAccountPair.
        * (drop `back_edge` — fires for many normal JEs, not discriminative).
      Also: cross-JE cycle detection on aggregate graph for CircularTransaction (per-JE attribution via
      "this JE closes a normal-graph cycle of length <=3").
- [ ] **I6** unified routed detector (local density + relational graph) → combined PR-AUC all families.
- [ ] **I7** validation/rigor: held-out, ablations, calibration, observability map.
- [ ] **I8** graph-JSON export (decoupled) + RustGraph ingestion/validation (RG-side, authorized).
- [ ] **I9** writeup: FINDINGS finalize, reproducible run, figures, SPEC.

## Results log (append per increment)
- I1: scaffold committed; relational GL generating on VM (anomaly_injection.rates.total_rate=0.08, fraud off).

## Open questions / blockers
- (none yet) — relational anomaly-type taxonomy + counts to be confirmed in I2; if too few relational
  samples at total_rate=0.08, bump the rate or bias the category mix.

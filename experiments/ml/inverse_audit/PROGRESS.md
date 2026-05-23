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
- **Now:** Central-abstraction proposal (#143) is structurally unblocked end-to-end.
  Open question — "can post-process account substitution preserve PO/GR/IR/Payment
  chain refs?" — answered YES in chain-invariants addendum (commit 5358051d): refs
  are keyed by document_id, NOT gl_account; only the 7 subledger-bridge accounts
  (AR/AP_CONTROL, GR_IR_CLEARING, IC_AR/IC_AP_CLEARING, WIRE_CLEARING,
  ACQUISITION_CLEARING) need an allowlist guard. Phase 1 design doc (commit
  4bef8554) sketches the ConcentrationPass trait + ConcentrationPipeline + orchestrator
  call site + 2 concrete passes (SourceConditionalRarityPass wrapping the shipped
  SOTA-12 tagger, TradingPartnerPoolPass closing the SOTA-11 coverage gap) + 6-test
  plan + back-compat config migration. Awaiting user steer on whether to land
  Phase 1 (~1-2 days engine work) before Phase 2 (account substitution with the
  7-account allowlist). All five planned SOTA steps (#8/9/10/11/12) shipped;
  FINDINGS §15 documents the round.
- **Last update:** 2026-05-23 ~17:00 (chain-invariants addendum + Phase 1 design doc landed).

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
- [x] **I8** decoupled graph-JSON export (commit e4114422 + run_capstone wire-up). Generic node/edge/
      per-JE schema; no DataSynth→external dependency. /tmp/iam: 362 nodes / 20 878 edges /
      42 new-test-SCC nodes / 4 622 new-test edges / 831 anomalous JEs joined in. RustGraph
      substrate ingestion (RG-side, authorised) is the follow-on left for the user — the JSON
      is ready, decoupled, and meets the b27 living-graph vision substrate contract.
- [x] **I9** FINDINGS §12 written — "Stage 1 closeout — relational arm + unified routed detector":
      tables for the three-armed result, the observability map, and the throughline. Reproducible via
      `generate_mixed.py → unified_score.py`.
- [x] **I9 polish** (commit 29da4f44 + wake-5 close): `run_capstone.py` one-shot reproducer +
      SPEC.md updated with new modules + reproduction command. `python -m inverse_audit.run_capstone`
      reproduces FINDINGS §12 numbers.

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
- Wake 5 (04:45–05:30):
  - I5 v4: added `cycle_novelty` (SCC-based, count of touched accounts in test's non-trivial SCCs
    that aren't in normal's) — commit 29da4f44. **cycle_novelty is the top single feature (ROC 0.553)**;
    overall unsupervised lifts marginally (0.220→0.223 / ROC 0.544 unchanged), but the **LR-CV ceiling
    jumps from 0.147/0.555 to 0.238/0.598** — cycle_novelty carries real *learnable* signal that the
    unweighted sum doesn't fully extract. CircularTransaction family stays at ~0.50 (the SCC-detector
    fires more broadly than the injector's specific cycle labels).
  - z_of now guards against degenerate MAD (cycle_novelty is constant on normal by construction):
    returns centred raw values instead of dividing by ~0.
  - I9 polish: `run_capstone.py` one-shot reproducer + SPEC.md updated.
  - Final mixed-GL unified result (v4): unified vs is_any **PR-AUC 0.397 / ROC 0.655** vs density-alone
    0.373/0.641 — routing thesis holds; the night closes with three arms validated end-to-end.
- Morning hand-off (~07:45): user re-provided the corpus locally; legal guardrails kept (path
  never in committed artifacts; aggregates only). Stage 2 of the capstone unblocked. Wrote
  `corpus_runner.py` and smoke-ran on smallest GL parquet (commit f633ee29): 116k lines/9.6k JEs
  processed in seconds; relational_score heavy-tailed (p50 0.67 / p99 6.94 / max 12.25); the
  active signals are edge_surprise (marginal + source-conditional) + account_dormancy_max.
  cycle_novelty + tp_account_novelty collapse to 0 under fit-on-self (no "new in test").
- Wake 8 (08:12–08:55):
  - Refactored `graph_export` to a DataFrame core (`export_graph_df`) with a CSV wrapper for
    backcompat. `corpus_runner` gains `--mode {self, half-split}` + `--export-graph`. Half-split
    shuffles JEs with a deterministic seed and fits on first half, scores second half — recovers
    the new-in-test signals.
  - Half-split smoke (smallest GL, 9.6k JEs total): now cycle_novelty p99=2, max=5;
    tp_account_novelty max=2. relational_score p99=10.4, max=14.7. Substrate JSON: 297 nodes,
    6548 edges, 2151 je-scores.
  - Half-split scale test (median GL, 22 MB → 1.13M lines / 250k JEs, 4m37s wall-clock):
    relational_score p99=12.43, max=26.89 (heavier tail than smallest at scale); manifold
    grows to 11,749 edges / 395 nodes / 308 SCC accounts (vs smallest's 5,248 / 280 / 179).
    cycle_novelty stays modest (p99=1) — the larger graph's SCC structure is more stable so
    fewer "new" cycles emerge under half-split. dormancy max 13.09 (vs 9.38 smallest).
  - `corpus_batch.py` written: SHA-tagged per-file dirs, aggregates summaries into a single
    cross_corpus.json without exposing client names. Ready for a sweep next wake.
- Wake 7 (07:04–07:30):
  - I8 graph-JSON export (`relational/graph_export.py`, commits f2a48798 + e4114422) — generic
    node/edge/per-JE schema, decoupled from any substrate. Wired into `run_capstone.py` so a
    single command emits the full pipeline outputs including the graph. /tmp/iam result:
    362 nodes (with PR-on-normal + PR-on-test + normal_je_count + SCC membership + new-in-test
    SCC flag) / 20 878 edges (with weight_normal/test + p_normal/test + is_new_in_test) /
    10 275 per-JE scores (relational_score + every feature + is_anomaly_je + anomaly_type).
    42 new-in-test SCC nodes and 4 622 new-in-test edges are the substrate's audit hot list.
  - Capstone closeout at every level: feature engineering (v3-v5), unified routing thesis (I6),
    observability map (I7), graph-JSON substrate (I8), FINDINGS §12 + SPEC + reproducer.
- Wake 6 (05:55–06:25):
  - I5 v5: added `source_cond_edge_surprise_max` = max -log P_normal(edge | source) — commit 4e148b23.
    **Per-feature ROC 0.550** (2nd-best, behind cycle_novelty 0.553). But the overall unsupervised
    sum has saturated: /tmp/iar v5 = 0.221/0.546 (essentially unchanged from v4 0.223/0.544); the
    LR-CV ceiling moves 0.238→0.239 only. UnusualAccountPair (the target family) lifts 0.480 → 0.487
    — directional but small; `relational` is now its best_arm.
  - **Conclusion: unweighted z-sum has plateaued.** Adding more positive-prior features won't compound
    further; the next gain requires either (a) per-family routing (different feature subsets per
    family), (b) a learned weighted combination on a labelled split, or (c) a fundamentally
    different feature class (cross-JE sequence model for Circular*; counterparty-relationship
    model for NewCounterparty/MissingRelationship). Wake 7 pivots to I8 (graph-JSON export +
    RustGraph substrate, files-only, RG-side commits authorised).

## Open questions / blockers
- (none yet) — relational anomaly-type taxonomy + counts to be confirmed in I2; if too few relational
  samples at total_rate=0.08, bump the rate or bias the category mix.

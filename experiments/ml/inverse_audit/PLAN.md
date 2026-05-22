# Inverse-audit capstone — finalization plan (overnight, autonomous)

**North star (user, 2026-05-22):** *finalize the capstone.* Everything below earns its
place only by advancing that. The accounting-network reconstruction (the methodology
papers, [[reference_accounting_network_papers]]) is integrated **as the capstone's
missing relational arm** — not as a substrate project of its own.

## Where the capstone stands (Stage 1, FINDINGS §11)
- **Local arm** — per-JE density residual (amount + structure + behavioral NLL):
  PR-AUC **0.741**, beats Isolation Forest ~19×. Strong on structural (ROC 0.91–1.0) +
  behavioral (0.81–0.88) fraud.
- **Global arm** — SBI light-B: recovers injected `fraud.fraud_rate` at **r=0.989**
  on-manifold (collapses OOD — the binding-constraint demo).
- **THE GAP — relational arm:** the per-JE residual is near-blind to *relational*
  anomalies (circular flows, duplicate payments, dormant-account reactivation,
  centrality) — ROC **~0.096**. A single JE's residual cannot see graph context.

## The thesis to complete
> Route each anomaly family to the detector that **observes its subsystem**.
> local density (structural+behavioral) + **relational graph residual** (relational) +
> global SBI (system-state). The relational arm is the account-flow graph the
> methodology paper reconstructs — so reconstruction = the capstone's relational organ.

## Increments (each: VM compute → validate → commit to DataSynth main, experiments/-isolated, green → update PROGRESS.md)
- **I1 (done):** plan + PROGRESS + `generate_relational.py`; kick off relational-family GL on VM.
- **I2:** confirm relational GL + labels — anomaly_injection enabled, `is_anomaly`/`anomaly_type`
  taxonomy; count per relational family. Re-baseline the per-JE density scorer on it (reproduce ~0.096).
- **I3 — rung 1:** entropic-OT (Sinkhorn) within-JE flow reconstruction (`relational/ot_flow.py`).
  credit-sources × debit-sinks, exact marginals (balance-preserving), uniform cost first.
  Build the account-flow graph (account→account weighted, temporal). Validate balance/marginals.
- **I4 — rung 2 + accuracy:** extract DataSynth ground-truth flows from document chains
  (`document_references`/cross-process links); measure OT reconstruction accuracy vs truth;
  learn the cost (co-occurrence / COA-semantic prior). Baseline = reimplement the paper's A–E solver.
- **I5 — relational scorer:** from the account-flow graph detect the relational families —
  cycles (CircularTransaction), repeated-motif (DuplicatePayment), reactivation
  (DormantAccountActivity), PageRank/betweenness spike (CentralityAnomaly), suspense routing.
  Per-JE relational score. Assess vs `anomaly_type` → **lift from 0.096**.
- **I6 — unified routed detector:** combine local density + relational graph score (+ optional
  global SBI). Combined PR-AUC across **all** families. Realize the routing thesis.
- **I7 — validation/rigor:** held-out splits, ablations (which graph features matter),
  calibration, the observability/identifiability map (which subsystem sees which family).
- **I8 — graph export + RustGraph substrate validation:** emit the reconstructed account-flow
  graph as plain JSON files (decoupled — DataSynth side stays dependency-free). THEN, RG-side
  (user-authorized), ingest those files into RustGraph and validate the relational analytics
  (cycles, centrality, hypergraph) against RG's engine — realizing the b27 living-graph/actor
  vision. All RustGraph glue/commits live in the RustGraph repo, never here.
- **I9 — writeup:** finalize FINDINGS (§11 → capstone-complete), reproducible `run.py`, figures, SPEC.

## Constraints (every iteration must honour)
- **NO DataSynth→RustGraph dependency.** RustGraph is private/not public. Graph output =
  plain JSON files, decoupled. Do not add RustGraph imports/Cargo-deps or new RustGraph docs to this repo.
- **Synthetic data only** — no corpus (privacy + we have ground-truth flows, which is the point).
- **Do not destabilize the shipping engine** — isolated `experiments/ml/inverse_audit/` code; keep
  `main` green; never run OOM/orchestrator/CLI-spawning tests locally (let CI do it).
- **Legal:** "corpus data" only; no client names; no real paths/content in committed artifacts.
- **VM:** `ssh ubuntu@129.80.80.52`, venv `~/mlenv`, binary `~/SyntheticData/target/release` (db16a812
  built), `CARGO_BUILD_JOBS=24`. Sync py via scp or `git pull` (experiments-only push won't rebuild).

## Decision log
- 2026-05-22: user — capstone finalization is the focus; no RG dependency; "files only, decoupled".
- Reconstruction lives in `inverse_audit/relational/` (Python, numpy/scipy/networkx); standalone, no engine touch.

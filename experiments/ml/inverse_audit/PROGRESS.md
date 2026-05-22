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
- **Now:** I1 done (scaffold + generation kicked off + rung-1 code written). Next = **I2/I3**.
- **Last update:** 2026-05-23 (night shift start), increment 1.

## Increment ledger
- [x] **I1** plan + PROGRESS + `generate_relational.py` + `relational/ot_flow.py` (rung-1, written, untested) +
      relational-family GL generation kicked off on VM → `/tmp/iar/{normal,test}` (marker `/tmp/iar_gen.done`).
- [ ] **I2** confirm relational GL + labels: `is_anomaly`/`anomaly_type` taxonomy + per-family counts on
      `/tmp/iar/test`. Reproduce the per-JE density scorer's ~0.096 relational ROC (the baseline to beat).
- [ ] **I3** validate `ot_flow.py` (run its self-test; then on real JEs: marginals exact, balance preserved);
      build the account-flow graph from `/tmp/iar/test`. Persist graph (edges + per-JE coupling entropy).
- [ ] **I4** ground-truth flows from document chains; OT accuracy vs truth; learn cost (rung 2); A–E baseline.
- [ ] **I5** relational scorer (cycles / duplicate-motif / dormant / centrality / suspense) → assess vs
      `anomaly_type` → lift from 0.096.
- [ ] **I6** unified routed detector (local density + relational graph) → combined PR-AUC all families.
- [ ] **I7** validation/rigor: held-out, ablations, calibration, observability map.
- [ ] **I8** graph-JSON export (decoupled) + RustGraph ingestion/validation (RG-side, authorized).
- [ ] **I9** writeup: FINDINGS finalize, reproducible run, figures, SPEC.

## Results log (append per increment)
- I1: scaffold committed; relational GL generating on VM (anomaly_injection.rates.total_rate=0.08, fraud off).

## Open questions / blockers
- (none yet) — relational anomaly-type taxonomy + counts to be confirmed in I2; if too few relational
  samples at total_rate=0.08, bump the rate or bias the category mix.

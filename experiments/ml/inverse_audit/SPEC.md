# inverse_audit — Stage 1 capstone

Design: `../../../docs/superpowers/specs/2026-05-22-inverse-audit-capstone-design.md`
Plan:   `../../../docs/superpowers/plans/2026-05-22-inverse-audit-capstone.md`

Pipeline: generate.py → export.py → score.py → assess.py / lightb.py, orchestrated
by run.py. Reuses `common/data_export.py` (flow inputs), `flow/` (amount density),
`inverse/apply.py` (global posterior, light-B).

Execution refinement (2026-05-22): the structural per-JE scorer is an
archetype-conditional-frequency likelihood — `-log P(account-set signature | source)`
under the normal data — NOT the per-(client,source) sequence transformer (wrong
granularity for per-JE labels). Flow (amounts) + IF baseline + light-B unchanged.

## Stage 1 closeout — relational arm + unified routed detector (2026-05-23, FINDINGS §12)

Three observability layers: local density (per-JE, §11) + global SBI (parameters, §11
light-B) + **relational graph-manifold residual (new)**. The relational arm is the bridge
to the methodology-paper integration (account-flow-graph reconstruction at audit-grade
fidelity, [[reference_accounting_network_papers]]).

New modules:
  relational/ot_flow.py        rung-1 entropic-OT (Sinkhorn) within-JE flow reconstruction;
                                exact marginals (balance preserved); aggregate account-flow
                                graph + per-JE coupling entropy.
  relational/graph_scorer.py   relational manifold residual. Five positive-prior z-features
                                summed into `relational_score`:
                                  edge_surprise_max/_w  (rare edge under P_normal)
                                  tp_account_novelty     (bipartite (tp, gl_account) pair novel)
                                  account_dormancy_max   (IDF of touched-account counts)
                                  cycle_novelty          (count of touched accounts in test's
                                                          new strongly-connected components)
                                Diagnostic (per-feature ROC, not summed): back_edge,
                                centrality_max, tp_novelty raw, coupling_entropy. LR-CV
                                ceiling reported as upper bound.
  generate_relational.py       relational-only GL (fraud off, anomaly_injection on).
  generate_mixed.py            mixed GL (both on) — substrate for routing measurement.
  unified_score.py             three-arm join + per-family observability map.
  relational/graph_export.py   decoupled graph-JSON export — node/edge/per-JE payload for
                                downstream substrate ingestion (graph DB / RustGraph living
                                graph / notebook). No DataSynth→external dependency.
  run_capstone.py              one-shot end-to-end reproduction of §12 + graph export.

Reproduce (synthetic, in-distribution):

    python -m inverse_audit.run_capstone --root /tmp/ia_full

writes density_scores.parquet + graph_scores.parquet + unified.json. The §12 tables come
from unified.json (`overall` / `per_fraud_type` / `per_anomaly_type`); the routing recipe
follows the `best_arm` column in each per-family entry.

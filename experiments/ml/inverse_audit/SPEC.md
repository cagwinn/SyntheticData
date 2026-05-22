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

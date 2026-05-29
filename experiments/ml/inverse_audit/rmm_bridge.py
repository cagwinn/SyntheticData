"""Deepen P2 — bridge inverse-audit per-account residual signals -> methodology RMM-by-assertion.

Reads DataSynth's exported account-flow graph (account_flow_graph.json), derives per-account risk
signals from the inverse-audit detector's substrate (dormancy IDF, new-in-test SCC membership,
new-in-test edges), maps each signal to ISA-315 assertions + RMM factors, converts to Bayesian
FactorEvidence, and runs the methodology's RMM engine (compute_rmm) to produce an
RMM-by-(account, assertion) matrix — the methodology-grade output the cortex / engagement runner
consumes. Compares against the priors-only baseline. Aggregate output only (accounts keyed by index).

    .venv/bin/python scripts/inverse_audit_rmm_bridge.py <account_flow_graph.json>

Signal -> (assertion, factors) mapping (each touches one INHERENT + one CONTROL factor so RMM=IR*CR
elevates):
  dormancy (rarely-used account active) -> existence : FRAUD_SUSCEPTIBILITY + OVERRIDE_SIGNALS
  new-in-test SCC (circular/round-trip) -> existence : FRAUD_SUSCEPTIBILITY + RELATED_PARTY_DENSITY
                                                       + OVERRIDE_SIGNALS
  new-in-test edges (unusual pairing)   -> accuracy  : ACCOUNT_COMPLEXITY + CONTROL_TEST_RESULTS
"""
from __future__ import annotations

import json
import math
import sys
from collections import defaultdict

from gam_scraper.audit_scoring.engine import RMMRequest, compute_rmm
from gam_scraper.audit_scoring.factors import RMMFactor as F
from gam_scraper.audit_scoring.posterior import FactorEvidence


def ev(factor, strength: float, scale: int = 8) -> FactorEvidence:
    """Residual strength [0,1] -> Beta evidence (failures=higher risk, passes=lower)."""
    s = max(0.0, min(1.0, strength))
    return FactorEvidence(factor=factor, failures=int(round(s * scale)),
                          passes=int(round((1.0 - s) * max(1, scale // 4))))


def main() -> None:
    path = sys.argv[1] if len(sys.argv) > 1 else "account_flow_graph.json"
    g = json.load(open(path))
    nodes, edges = g["nodes"], g["edges"]

    new_edges = defaultdict(int)
    for e in edges:
        if e.get("props", {}).get("is_new_in_test"):
            new_edges[e["src"]] += 1
            new_edges[e["dst"]] += 1

    N = sum(n["props"].get("normal_je_count", 0) for n in nodes) or 1
    max_idf = math.log(N / 1.0)
    base = compute_rmm(RMMRequest(account_id="_baseline", assertion_kind="existence", evidence=[]))

    rows = []
    for i, n in enumerate(nodes):
        p = n["props"]
        cnt = p.get("normal_je_count", 0)
        dormancy = (math.log(N / (cnt + 1)) / max_idf) if max_idf > 0 else 0.0
        new_scc = bool(p.get("in_test_scc")) and not bool(p.get("in_normal_scc"))
        ne = new_edges.get(n["id"], 0)
        per_assertion: dict[str, list] = defaultdict(list)
        if dormancy > 0.5:
            per_assertion["existence"] += [ev(F.FRAUD_SUSCEPTIBILITY, dormancy), ev(F.OVERRIDE_SIGNALS, dormancy)]
        if new_scc:
            per_assertion["existence"] += [ev(F.FRAUD_SUSCEPTIBILITY, 1.0), ev(F.RELATED_PARTY_DENSITY, 1.0),
                                           ev(F.OVERRIDE_SIGNALS, 1.0)]
        if ne > 0:
            s = min(1.0, ne / 5.0)
            per_assertion["accuracy"] += [ev(F.ACCOUNT_COMPLEXITY, s), ev(F.CONTROL_TEST_RESULTS, s)]
        for assertion, evid in per_assertion.items():
            r = compute_rmm(RMMRequest(account_id=f"acct_{i}", assertion_kind=assertion, evidence=evid))
            rows.append((i, assertion, r.rmm_mean, r.inherent_risk_mean, r.control_risk_mean,
                         {"dormancy": round(dormancy, 2), "new_scc": new_scc, "new_edges": ne}))

    rows.sort(key=lambda x: -x[2])
    n_elev = sum(1 for r in rows if r[2] > base.rmm_mean)
    print(f"baseline (priors-only) RMM = {base.rmm_mean:.3f}  (IR={base.inherent_risk_mean:.2f} CR={base.control_risk_mean:.2f})")
    print(f"residual-driven (account,assertion) RMM pairs: {len(rows)}; {n_elev} elevated above baseline")
    print(f"{'rmm':>6} {'IR':>5} {'CR':>5}  assertion    signals")
    for _, assn, rmm, ir, cr, sig in rows[:12]:
        print(f"{rmm:6.3f} {ir:5.2f} {cr:5.2f}  {assn:11s}  {sig}")
    out = {"baseline_rmm": round(base.rmm_mean, 4), "n_pairs": len(rows),
           "matrix": [{"acct_idx": i, "assertion": assn, "rmm_mean": round(rmm, 4),
                       "inherent": round(ir, 4), "control": round(cr, 4)}
                      for i, assn, rmm, ir, cr, _ in rows]}
    json.dump(out, open("inverse_audit_rmm_matrix.json", "w"), indent=2)
    print(f"-> inverse_audit_rmm_matrix.json ({len(rows)} pairs)")


if __name__ == "__main__":
    main()

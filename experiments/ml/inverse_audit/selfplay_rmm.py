"""P3.1 close — self-play belief state -> AuditMethodology RMM-by-account.

The self-play loop (selfplay.py) evolves an audit-faithful auditor's belief over periods:
per-account carried-forward risk built from CONFIRMED prior-period findings. This bridge
feeds that belief — `final_prior_confirmed`, the carried-forward CONFIRMED-finding prior — to
the methodology's Bayesian RMM engine (gam_scraper.audit_scoring.compute_rmm) as carried-
forward control-risk evidence, producing account-level ISA-315 risk-of-material-misstatement.
It closes the substrate loop:  residual (P1) -> cortex ISA-315 (P2) -> temporal self-play
belief (P3.1) -> methodology RMM.

NB the carry-forward belief is the CONFIRMED-finding prior, not the residual `risk_density`:
the persistent bad actor is invisible to any residual ranking (its JEs look normal — that is
why only confirmed-finding memory catches it), so confirmed findings are the faithful
carry-forward signal. The belief over-weights high-volume control accounts (a known
limitation of the max-prior representation — refined in P3.2); the persistent bad actors are
nonetheless present and elevated above the priors-only baseline.

This is the audit-doctrine carry-forward: prior-period KNOWN findings raise the current
period's RMM. We show the accounts the self-play auditor learned to watch (incl. the
persistent bad actors) get RMM elevated above the priors-only baseline.

Runs in the AuditMethodology venv (needs gam_scraper):

    /path/AuditMethodology/.venv/bin/python -m inverse_audit.selfplay_rmm /tmp/sp_full/self_play_persistent.json

(or pass the path as argv[1]); writes selfplay_rmm_matrix.json next to the input.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

from gam_scraper.audit_scoring.engine import RMMRequest, compute_rmm
from gam_scraper.audit_scoring.factors import RMMFactor as F
from gam_scraper.audit_scoring.posterior import FactorEvidence


def ev(factor, strength: float, scale: int = 8) -> FactorEvidence:
    """Carried-forward risk strength [0,1] -> Beta evidence (failures=higher risk)."""
    s = max(0.0, min(1.0, strength))
    return FactorEvidence(factor=factor, failures=int(round(s * scale)),
                          passes=int(round((1.0 - s) * max(1, scale // 4))))


def main() -> None:
    path = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/sp_full/self_play_persistent.json")
    sp = json.loads(path.read_text())
    belief = sp.get("final_prior_confirmed", {})              # account -> carried-forward confirmed-finding risk
    persistent = set(sp.get("persistent_accounts_used", []))
    if not belief:
        print("no final_prior_confirmed in input (run selfplay.py first)")
        return
    dmax = max(belief.values()) or 1.0

    base = compute_rmm(RMMRequest(account_id="_baseline", assertion_kind="existence", evidence=[]))
    rows = []
    for acct, d in belief.items():
        s = d / dmax                                          # normalise carried-risk to [0,1]
        # carried-forward findings: a fraud-prone account with prior-period confirmed issues ->
        # elevated inherent fraud susceptibility + control deficiencies (prior findings, override risk)
        evid = [ev(F.FRAUD_SUSCEPTIBILITY, s), ev(F.CONTROL_TEST_RESULTS, s), ev(F.OVERRIDE_SIGNALS, s)]
        r = compute_rmm(RMMRequest(account_id=f"acct_{acct}", assertion_kind="existence", evidence=evid))
        rows.append((acct, r.rmm_mean, r.inherent_risk_mean, r.control_risk_mean, s, acct in persistent))
    rows.sort(key=lambda x: -x[1])
    n_elev = sum(1 for x in rows if x[1] > base.rmm_mean)

    print(f"baseline (priors-only) RMM = {base.rmm_mean:.3f}  (IR={base.inherent_risk_mean:.2f} CR={base.control_risk_mean:.2f})")
    print(f"self-play belief -> {len(rows)} accounts scored; {n_elev} elevated above baseline; "
          f"persistent bad actors = {sorted(persistent)}")
    print(f"{'rmm':>6} {'IR':>5} {'CR':>5}  {'belief':>6}  account   persistent?")
    for acct, rmm, ir, cr, s, isp in rows[:12]:
        print(f"{rmm:6.3f} {ir:5.2f} {cr:5.2f}  {s:6.3f}  {acct:8s}  {'<<< BAD ACTOR' if isp else ''}")
    # where do the persistent accounts rank?
    rank = {acct: i for i, (acct, *_rest) in enumerate(rows)}
    for p in sorted(persistent):
        print(f"  persistent {p}: RMM-rank {rank.get(p, 'absent')}/{len(rows)}"
              + (f", RMM={dict((a, rmm) for a, rmm, *_ in rows)[p]:.3f}" if p in rank else ""))

    out = {"baseline_rmm": round(base.rmm_mean, 4), "n_accounts": len(rows),
           "persistent_accounts": sorted(persistent),
           "matrix": [{"account": a, "rmm_mean": round(rmm, 4), "inherent": round(ir, 4),
                       "control": round(cr, 4), "belief": round(s, 4), "persistent": isp}
                      for a, rmm, ir, cr, s, isp in rows]}
    outp = path.with_name("selfplay_rmm_matrix.json")
    outp.write_text(json.dumps(out, indent=2))
    print(f"-> {outp} ({len(rows)} accounts)")


if __name__ == "__main__":
    main()

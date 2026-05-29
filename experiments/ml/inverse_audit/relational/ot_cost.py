"""Rung-2 — learn the within-JE OT cost from DataSynth ground-truth flows.

ot_flow rung-1 reconstructs the within-JE debit↔credit pairing with a UNIFORM cost (the
max-entropy guess, since the pairing is projected away in the GL). Rung-2 learns the cost
from the flows DataSynth *does* reveal: a TWO-LINE JE (1 debit, 1 credit) is an unambiguous
ground-truth `credit_acct → debit_acct` edge. Aggregating 2-line edges across the GL gives an
empirical P(credit→debit); `cost(c,d) = -log P(c,d)` is the learned OT cost that disambiguates
the MULTI-line JEs (where the transportation polytope is non-trivial). This is the methodology
paper's rung-2 — supervision a real GL can't give, only possible because DataSynth generates
the flows. No engine coupling; numpy/pandas only.

    python -m inverse_audit.relational.ot_cost --gl runs/B/iar/normal --eval
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import pandas as pd

from inverse_audit.relational.ot_flow import reconstruct_je


def _two_line_edges(df: pd.DataFrame) -> list[tuple[str, str]]:
    """Ground-truth (credit_acct, debit_acct) edges from JEs with exactly 1 debit + 1 credit."""
    edges = []
    for _, g in df.groupby("document_id", sort=False):
        deb = g[g["debit_amount"] > 0]
        cred = g[g["credit_amount"] > 0]
        if len(deb) == 1 and len(cred) == 1:
            edges.append((str(cred["gl_account"].iloc[0]), str(deb["gl_account"].iloc[0])))
    return edges


def learn_edge_cost(edges: list[tuple[str, str]]) -> tuple[dict[tuple[str, str], float], float]:
    """Empirical P(credit→debit) → cost = -log P. Returns (cost_map, default_cost_for_unseen)."""
    from collections import Counter
    c = Counter(edges)
    total = sum(c.values()) or 1
    cost_map = {e: -np.log(n / total) for e, n in c.items()}
    # unseen edge cost = slightly worse than the rarest seen edge (so unseen pairs are penalised)
    default = (max(cost_map.values()) + 1.0) if cost_map else 1.0
    return cost_map, float(default)


def make_cost_fn(cost_map: dict, default: float):
    """cost_fn(credit_accts, debit_accts) -> (m_credit x n_debit) cost matrix for Sinkhorn."""
    def cost_fn(credit_accts, debit_accts):
        return np.array([[cost_map.get((str(c), str(d)), default) for d in debit_accts]
                         for c in credit_accts], dtype=float)
    return cost_fn


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--gl", type=Path, required=True, help="GL dir with journal_entries.csv")
    ap.add_argument("--out", type=Path, default=None)
    ap.add_argument("--seed", type=int, default=0)
    a = ap.parse_args(argv)

    df = pd.read_csv(a.gl / "journal_entries.csv", low_memory=False)
    for col in ("debit_amount", "credit_amount"):
        df[col] = pd.to_numeric(df[col], errors="coerce").fillna(0.0)
    lpje = df.groupby("document_id").size()
    multi = lpje[lpje > 2].index
    edges = _two_line_edges(df)
    print(f"JEs={len(lpje)}  2-line={int((lpje==2).sum())}  multi-line(>2)={len(multi)}  "
          f"ground-truth 2-line edges={len(edges)} ({len(set(edges))} distinct)")

    # Held-out validation: does the learned cost rank a held-out TRUE 2-line edge below a
    # random (credit, debit) pair? (lower cost = more likely true). AUC over held-out edges.
    rng = np.random.default_rng(a.seed)
    idx = np.arange(len(edges)); rng.shuffle(idx)
    cut = len(idx) // 2
    train_e = [edges[i] for i in idx[:cut]]
    held_e = [edges[i] for i in idx[cut:]]
    cost_map, default = learn_edge_cost(train_e)
    creds = [c for c, _ in edges]; debs = [d for _, d in edges]
    wins = ties = 0
    for (c, d) in held_e:
        rc, rd = creds[rng.integers(len(creds))], debs[rng.integers(len(debs))]
        true_cost = cost_map.get((c, d), default)
        rand_cost = cost_map.get((rc, rd), default)
        if true_cost < rand_cost:
            wins += 1
        elif true_cost == rand_cost:
            ties += 1
    auc = (wins + 0.5 * ties) / max(len(held_e), 1)
    print(f"held-out 2-line edge ranking AUC (cost(true) < cost(random)): {auc:.3f}  "
          f"(0.5 = no signal, 1.0 = perfect)")

    # Multi-line impact: mean coupling entropy rung-1 (uniform) vs rung-2 (learned cost).
    # Lower entropy under rung-2 = the learned cost resolves the polytope more confidently.
    full_cost, full_default = learn_edge_cost(edges)
    h1, h2, n = 0.0, 0.0, 0
    for je in multi[:2000]:
        g = df[df["document_id"] == je]
        deb = g[g["debit_amount"] > 0]; cred = g[g["credit_amount"] > 0]
        if deb.empty or cred.empty:
            continue
        ca = cred["gl_account"].astype(str).to_numpy(); cv = cred["credit_amount"].to_numpy()
        da = deb["gl_account"].astype(str).to_numpy(); dv = deb["debit_amount"].to_numpy()
        _, e1 = reconstruct_je(ca, cv, da, dv, cost=None)
        cost = make_cost_fn(full_cost, full_default)(ca, da)
        _, e2 = reconstruct_je(ca, cv, da, dv, cost=cost)
        h1 += e1; h2 += e2; n += 1
    if n:
        print(f"multi-line coupling entropy (n={n}): rung-1 uniform={h1/n:.3f}  "
              f"rung-2 learned={h2/n:.3f}  reduction={100*(h1-h2)/max(h1,1e-9):.1f}%")
    if a.out:
        a.out.write_text(json.dumps({
            "n_jes": int(len(lpje)), "n_two_line": int((lpje == 2).sum()),
            "n_multi": int(len(multi)), "n_gt_edges": len(edges),
            "heldout_edge_ranking_auc": round(auc, 4),
            "multi_entropy_rung1": round(h1 / n, 4) if n else None,
            "multi_entropy_rung2": round(h2 / n, 4) if n else None}, indent=2))
        print(f"-> {a.out}")


if __name__ == "__main__":
    main()

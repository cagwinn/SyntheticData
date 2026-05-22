"""Rung 1 — entropic optimal-transport within-JE flow reconstruction.

The GL stores only line *marginals*; the true debit<-credit pairing inside a multi-line
JE was projected away at posting time (it lives in a transportation polytope). The
methodology paper's A-E solver picks one vertex of that polytope by heuristic. We instead
solve the *transportation problem* directly with entropic OT (Sinkhorn):

    credit lines = sources (supply = credit amounts)
    debit  lines = sinks   (demand = debit amounts)      sum(supply) == sum(demand)  (balanced JE)
    find flow matrix X (m_credit x n_debit), X 1 = supply, X^T 1 = demand,
        minimising <C, X> - eps * H(X)

Properties (matching the paper's own paradigms): marginals satisfied exactly => balance
preserved (bijective/integrity); Sinkhorn is pure matrix scaling (GPU-friendly); the
entropic solution is a *distribution* over matchings whose normalised entropy is a
principled confidence, replacing the paper's ad-hoc 1/n. Edge direction follows the paper:
a credit is an out-edge (source) and a debit an in-edge (sink), so flow runs
credit_account -> debit_account.

Uniform cost (C=0) gives the maximum-entropy / independence coupling (outer product of
marginals) — the honest prior when nothing else is known. Rung 2 (I4) learns C from
DataSynth ground-truth flows. numpy-only; no engine / RustGraph coupling.
"""
from __future__ import annotations

import numpy as np


def sinkhorn(supply, demand, cost=None, eps: float = 0.05, iters: int = 400,
             tol: float = 1e-9) -> np.ndarray:
    """Entropic-OT transport plan with exact marginals. Returns X (len(supply) x len(demand))
    in the same units as supply/demand (X.sum(1)==supply, X.sum(0)==demand)."""
    a = np.asarray(supply, dtype=float)
    b = np.asarray(demand, dtype=float)
    total = a.sum()
    if total <= 0 or b.sum() <= 0:
        return np.zeros((len(a), len(b)))
    an, bn = a / total, b / (b.sum())  # normalise each marginal to a simplex
    m, n = len(an), len(bn)
    C = np.zeros((m, n)) if cost is None else np.asarray(cost, dtype=float)
    K = np.exp(-C / eps)
    u, v = np.ones(m), np.ones(n)
    for _ in range(iters):
        u_prev = u
        u = an / np.maximum(K @ v, 1e-300)
        v = bn / np.maximum(K.T @ u, 1e-300)
        if np.max(np.abs(u - u_prev)) < tol:
            break
    X = (u[:, None] * K) * v[None, :]      # normalised coupling (sums to 1)
    return X * total                        # scale back to JE amounts


def coupling_entropy(X: np.ndarray) -> float:
    """Normalised entropy of the coupling in [0,1]: 0 = deterministic match (high
    confidence), 1 = maximally diffuse (ambiguous => structurally unusual JE)."""
    t = X.sum()
    if t <= 0 or X.size <= 1:
        return 0.0
    p = (X / t).ravel()
    p = p[p > 0]
    return float(-np.sum(p * np.log(p)) / np.log(X.size))


def reconstruct_je(credit_accounts, credit_amounts, debit_accounts, debit_amounts,
                   cost=None, eps: float = 0.05):
    """One JE -> (flows, coupling_entropy). flows = [(credit_acct, debit_acct, amount), ...]."""
    X = sinkhorn(credit_amounts, debit_amounts, cost=cost, eps=eps)
    flows = []
    for i, ca in enumerate(credit_accounts):
        for j, da in enumerate(debit_accounts):
            w = X[i, j]
            if w > 1e-9:
                flows.append((str(ca), str(da), float(w)))
    return flows, coupling_entropy(X)


def reconstruct_per_je(df, account_col="gl_account", debit_col="debit_amount",
                       credit_col="credit_amount", je_col="document_id",
                       eps: float = 0.05, cost_fn=None):
    """Reconstruct flows for every JE. Returns {je_id: (flows, coupling_entropy)} where
    flows = [(credit_acct, debit_acct, amount), ...]. The per-JE basis for both the
    aggregate graph and the relational scorer."""
    import pandas as pd

    out: dict[str, tuple[list, float]] = {}
    for je_id, g in df.groupby(je_col, sort=False):
        deb = pd.to_numeric(g[debit_col], errors="coerce").fillna(0.0).to_numpy()
        cred = pd.to_numeric(g[credit_col], errors="coerce").fillna(0.0).to_numpy()
        acct = g[account_col].astype(str).to_numpy()
        di = np.where(deb > 0)[0]
        ci = np.where(cred > 0)[0]
        if di.size == 0 or ci.size == 0:
            continue
        cost = cost_fn(acct[ci], acct[di]) if cost_fn else None
        out[str(je_id)] = reconstruct_je(acct[ci], cred[ci], acct[di], deb[di], cost=cost, eps=eps)
    return out


def build_account_graph(df, **kw):
    """Aggregate account-flow graph. Returns (edges, je_entropy):
      edges      : {(src_account, dst_account): summed_flow_amount}
      je_entropy : {je_id: normalised coupling entropy}  (the relational-ambiguity signal)
    """
    per = reconstruct_per_je(df, **kw)
    edges: dict[tuple[str, str], float] = {}
    je_entropy: dict[str, float] = {}
    for je_id, (flows, h) in per.items():
        je_entropy[je_id] = h
        for s, d, w in flows:
            edges[(s, d)] = edges.get((s, d), 0.0) + w
    return edges, je_entropy


if __name__ == "__main__":
    # Self-test: marginals exact, balance preserved, forced/degenerate cases.
    x = sinkhorn([60.0, 40.0], [70.0, 30.0])
    assert np.allclose(x.sum(1), [60, 40]) and np.allclose(x.sum(0), [70, 30]), x
    x1 = sinkhorn([100.0], [60.0, 40.0])           # 1 credit -> 2 debits (forced)
    assert np.allclose(x1.ravel(), [60, 40]), x1
    f, h = reconstruct_je(["2000"], [100.0], ["6000", "6100"], [60.0, 40.0])
    assert len(f) == 2 and abs(sum(w for *_, w in f) - 100.0) < 1e-6, f
    # uniform-cost 2x2 with equal marginals => max-entropy (h near 1)
    assert coupling_entropy(sinkhorn([50.0, 50.0], [50.0, 50.0])) > 0.9
    print("flows:", f, "entropy:", round(h, 3))
    print("OT_FLOW_SELFTEST_OK")

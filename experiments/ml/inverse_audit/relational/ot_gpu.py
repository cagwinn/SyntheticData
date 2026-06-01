"""Rung-2 OT within-JE reconstruction — GPU-batched Sinkhorn (the substrate's L2 flow recon).

`ot_flow.reconstruct_per_je` (numpy, one JE at a time) is correct but does not scale: corpus GLs
have JEs up to ~1000 lines and hundreds of thousands of JEs, each needing a 400-iter Sinkhorn.
This module batches JEs of the same (m_credit x n_debit) shape into a single [B, m, n] tensor and
runs Sinkhorn as batched matrix products — the operation the methodology paper notes is "pure matrix
scaling (GPU-friendly)". Two interchangeable backends:

  * numpy  — vectorised batched Sinkhorn; validates the batching against ot_flow locally and is the
             CPU fallback when torch is absent.
  * torch  — identical math via bmm on CUDA; the GPU path for VM-scale evaluation.

`reconstruct_per_je_gpu` returns the same `{je_id: (flows, coupling_entropy)}` shape as
`ot_flow.reconstruct_per_je`, so it is a drop-in accelerator for `graph_scorer.fit_graph_manifold` /
`score_df` (which already accept a rung-2 `cost_fn`). Marginals are satisfied exactly (balance
preserved); JEs larger than `--size-cap` per side fall back to the exact numpy per-JE solver.

    python -m inverse_audit.relational.ot_gpu --selftest          # numpy-batched vs ot_flow parity
    python -m inverse_audit.relational.ot_gpu --selftest --backend torch   # + GPU parity (needs torch)
"""
from __future__ import annotations

import numpy as np

try:
    import torch
    _HAS_TORCH = True
except ImportError:  # torch is a VM-only dependency; numpy backend always works
    _HAS_TORCH = False


# --------------------------------------------------------------------------- batched Sinkhorn cores
def _sinkhorn_np(a, b, C, eps, iters, tol):
    """Batched numpy Sinkhorn. a[B,m], b[B,n] simplex marginals, C[B,m,n] cost. Returns X[B,m,n]
    (normalised coupling, rows sum to a, cols sum to b)."""
    K = np.exp(-C / eps)                                  # [B,m,n]
    B, m, n = K.shape
    u = np.ones((B, m)); v = np.ones((B, n))
    for _ in range(iters):
        u_prev = u
        Kv = np.einsum("bmn,bn->bm", K, v)
        u = a / np.maximum(Kv, 1e-300)
        Ktu = np.einsum("bmn,bm->bn", K, u)
        v = b / np.maximum(Ktu, 1e-300)
        if np.max(np.abs(u - u_prev)) < tol:
            break
    return u[:, :, None] * K * v[:, None, :]


def _sinkhorn_torch(a, b, C, eps, iters, tol, device):
    """Batched torch Sinkhorn (CUDA). Same math as _sinkhorn_np via bmm."""
    a = torch.as_tensor(a, dtype=torch.float64, device=device)
    b = torch.as_tensor(b, dtype=torch.float64, device=device)
    C = torch.as_tensor(C, dtype=torch.float64, device=device)
    K = torch.exp(-C / eps)                               # [B,m,n]
    Bn, m, n = K.shape
    u = torch.ones((Bn, m), dtype=torch.float64, device=device)
    v = torch.ones((Bn, n), dtype=torch.float64, device=device)
    for _ in range(iters):
        u_prev = u
        Kv = torch.bmm(K, v.unsqueeze(-1)).squeeze(-1)
        u = a / Kv.clamp_min(1e-300)
        Ktu = torch.bmm(K.transpose(1, 2), u.unsqueeze(-1)).squeeze(-1)
        v = b / Ktu.clamp_min(1e-300)
        if torch.max(torch.abs(u - u_prev)).item() < tol:
            break
    X = u.unsqueeze(-1) * K * v.unsqueeze(1)
    return X.cpu().numpy()


def batched_sinkhorn(supply, demand, cost, eps=0.05, iters=400, tol=1e-9,
                     backend="numpy", device="cuda"):
    """supply[B,m], demand[B,n], cost[B,m,n] (raw amounts). Normalises each item's marginals to a
    simplex, runs the chosen backend, scales back to amounts. Returns X[B,m,n]."""
    a = np.asarray(supply, float); b = np.asarray(demand, float); C = np.asarray(cost, float)
    totals = a.sum(axis=1, keepdims=True)                 # [B,1]
    an = a / np.maximum(totals, 1e-300)
    bn = b / np.maximum(b.sum(axis=1, keepdims=True), 1e-300)
    if backend == "torch":
        if not _HAS_TORCH:
            raise RuntimeError("backend='torch' requested but torch is not installed (VM-only dep).")
        X = _sinkhorn_torch(an, bn, C, eps, iters, tol, device)
    else:
        X = _sinkhorn_np(an, bn, C, eps, iters, tol)
    return X * totals[:, :, None]                         # back to JE amounts


def _entropy(X):
    """Normalised coupling entropy in [0,1] per item. X[B,m,n] -> [B]."""
    B = X.shape[0]
    out = np.zeros(B)
    for k in range(B):
        t = X[k].sum()
        if t <= 0 or X[k].size <= 1:
            continue
        p = (X[k] / t).ravel(); p = p[p > 0]
        out[k] = -np.sum(p * np.log(p)) / np.log(X[k].size)
    return out


# --------------------------------------------------------------------------- per-JE driver (bucketed)
def reconstruct_per_je_gpu(df, account_col="gl_account", debit_col="debit_amount",
                           credit_col="credit_amount", je_col="document_id",
                           cost_map=None, default=0.0, cost_fn=None, eps=0.05, iters=400,
                           backend="numpy", device="cuda", size_cap=2048, batch_cap=4096,
                           skipped=None):
    """Drop-in GPU/batched replacement for ot_flow.reconstruct_per_je. Buckets JEs by exact (m,n)
    shape and runs batched Sinkhorn per bucket (even a unique large shape is a size-1 bucket — no
    padding waste). JEs with m or n > `size_cap` are SKIPPED: corpus GLs contain pathological
    batch/allocation JEs up to ~100k lines (a 100k² cost matrix is infeasible); these structural
    outliers are not reconstructed. Pass a dict as `skipped` to receive {"n_skipped", "max_seen"}.
    Rung-2 cost supplied as a `cost_map` (+default) or a `cost_fn(creds, debs)->matrix`.
    Returns {je_id: ([(credit, debit, amount), ...], coupling_entropy)}."""
    import pandas as pd

    def _cost(creds, debs):
        if cost_fn is not None:
            return np.asarray(cost_fn(creds, debs), dtype=float)
        if cost_map is None:
            return np.zeros((len(creds), len(debs)))
        return np.array([[cost_map.get((str(c), str(d)), default) for d in debs] for c in creds])
    _has_cost = cost_fn is not None or cost_map is not None

    # collect per-JE (credit accts/amts, debit accts/amts); skip pathologically large ones
    jes = []
    n_skipped, max_seen = 0, 0
    for je_id, g in df.groupby(je_col, sort=False):
        deb = pd.to_numeric(g[debit_col], errors="coerce").fillna(0.0).to_numpy()
        cred = pd.to_numeric(g[credit_col], errors="coerce").fillna(0.0).to_numpy()
        acct = g[account_col].astype(str).to_numpy()
        di = np.where(deb > 0)[0]; ci = np.where(cred > 0)[0]
        if di.size == 0 or ci.size == 0:
            continue
        max_seen = max(max_seen, ci.size, di.size)
        if ci.size > size_cap or di.size > size_cap:
            n_skipped += 1
            continue
        jes.append((str(je_id), acct[ci], cred[ci], acct[di], deb[di]))
    if skipped is not None:
        skipped["n_skipped"] = n_skipped; skipped["max_seen"] = int(max_seen)

    out: dict[str, tuple[list, float]] = {}
    # bucket by (m_credit, n_debit) — exact shape, no padding
    buckets: dict[tuple[int, int], list] = {}
    for it in jes:
        buckets.setdefault((len(it[1]), len(it[3])), []).append(it)

    for (m, n), items in buckets.items():
        for s in range(0, len(items), batch_cap):
            chunk = items[s:s + batch_cap]
            B = len(chunk)
            supply = np.zeros((B, m)); demand = np.zeros((B, n)); cost = np.zeros((B, m, n))
            for k, (_jid, ca, cv, da, dv) in enumerate(chunk):
                supply[k] = cv; demand[k] = dv
                if _has_cost:
                    cost[k] = _cost(ca, da)
            X = batched_sinkhorn(supply, demand, cost, eps=eps, iters=iters,
                                 backend=backend, device=device)
            H = _entropy(X)
            for k, (jid, ca, _cv, da, _dv) in enumerate(chunk):
                flows = [(str(ca[i]), str(da[j]), float(X[k, i, j]))
                         for i in range(m) for j in range(n) if X[k, i, j] > 1e-9]
                out[jid] = (flows, float(H[k]))
    return out


def _selftest(backend="numpy"):
    from inverse_audit.relational.ot_flow import reconstruct_per_je
    import pandas as pd
    rng = np.random.default_rng(0)
    rows = []
    for jid in range(200):
        nl = int(rng.integers(2, 7))
        accts = [f"{rng.integers(1000, 9999)}" for _ in range(nl)]
        amts = rng.uniform(10, 1000, nl)
        # split into debit/credit halves, balance them
        half = max(1, nl // 2)
        deb = np.zeros(nl); cred = np.zeros(nl)
        deb[:half] = amts[:half]; cred[half:] = amts[half:]
        s = deb.sum(); c = cred.sum()
        if s == 0 or c == 0:
            continue
        cred *= s / c                                       # balance
        for a_, d_, c_ in zip(accts, deb, cred):
            rows.append({"document_id": f"JE{jid}", "gl_account": a_,
                         "debit_amount": d_, "credit_amount": c_})
    df = pd.DataFrame(rows)
    ref = reconstruct_per_je(df)                            # numpy per-JE (ground truth)
    got = reconstruct_per_je_gpu(df, backend=backend, device=("cuda" if backend == "torch" else "cpu"))
    assert set(ref) == set(got), "JE id mismatch"
    max_h, max_flow = 0.0, 0.0
    for jid in ref:
        rh = ref[jid][1]; gh = got[jid][1]
        max_h = max(max_h, abs(rh - gh))
        rf = {(s, d): w for s, d, w in ref[jid][0]}
        gf = {(s, d): w for s, d, w in got[jid][0]}
        for k in set(rf) | set(gf):
            max_flow = max(max_flow, abs(rf.get(k, 0) - gf.get(k, 0)))
    print(f"[selftest backend={backend}] {len(ref)} JEs | max entropy diff={max_h:.2e} "
          f"max flow diff={max_flow:.2e}")
    assert max_h < 1e-6 and max_flow < 1e-4, "GPU/batched reconstruction diverges from ot_flow"
    print("OT_GPU_SELFTEST_OK")


if __name__ == "__main__":
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--backend", default="numpy", choices=("numpy", "torch"))
    a = ap.parse_args()
    if a.selftest:
        _selftest(a.backend)
    else:
        print(f"torch available: {_HAS_TORCH}" + (f" | cuda: {torch.cuda.is_available()}" if _HAS_TORCH else ""))

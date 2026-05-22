"""Grounded surrogate + CMA-ES: find the generator params that best match the
corpus, using the inverse forward campaign as the surrogate's training data.

The scaffold `optimize.py` targets the BF composite over SP-internal knobs, but
`load_history` is a TODO, those knobs aren't `generate --config`-settable, and
the DR eval degenerates at corpus scale (FINDINGS.md §2). This is the runnable,
grounded variant: reuse the inverse campaign's `(θ, summary-stat)` pairs, define
the objective as `distance(summary_stats(θ), corpus)`, fit an MLP surrogate,
and CMA-ES to the corpus-matching `θ*`. `θ*` is the config the corpus "most
likely came from" — cross-checking the flow finding (corpus log-amount mean
≈ 3.9). Demonstrates the tuning-loop accelerator end-to-end on out-of-sample data.

    python -m surrogate.match_corpus --campaign data/inverse \\
        --corpus /home/ubuntu/corpus_health.parquet
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import pandas as pd
import torch
import torch.nn as nn

from inverse import params as P
from inverse.simulate import FEATURE_NAMES, summary_stats

# Features comparable corpus↔synthetic. Excludes doc-flow / behavioural-only
# observables the corpus columns don't carry (post-close, manual, lag) AND
# heavy-tailed/unstable ones that dominate an L2 distance: lpje_std (corpus has
# JEs with thousands of lines → std≈123) and the iet_* terms. Kept set is the
# robust amount + structure signal.
CMP = [
    "log_amt_mean", "log_amt_std", "log_amt_skew", "benford_mad", "round_frac",
    "weekend_frac", "monthend_frac", "lpje_mean", "lpje_frac2", "src_entropy",
]


def corpus_features(corpus_parquet: Path, tmp_csv: str) -> np.ndarray:
    """Map corpus columns → canonical, then reuse the campaign summary_stats."""
    df = pd.read_parquet(corpus_parquet)
    out = pd.DataFrame()
    out["debit_amount"] = pd.to_numeric(df["Functional Amount"], errors="coerce")
    out["credit_amount"] = 0.0
    out["posting_date"] = df["Entry Date"]
    out["document_date"] = df["Entry Date"]
    out["source"] = df["Source"]
    out["document_id"] = df["JE Number"]
    out["gl_account"] = df["GL Account Number"]
    out.to_csv(tmp_csv, index=False)
    return summary_stats(Path(tmp_csv))


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--campaign", type=Path, required=True)
    ap.add_argument("--corpus", type=Path, default=None)
    ap.add_argument("--corpus-cache", type=Path, default=None,
                    help="reuse corpus_features from a prior match_corpus.json (skips the 53M-row pass)")
    ap.add_argument("--out", type=Path, default=Path("weights/surrogate"))
    a = ap.parse_args(argv)
    a.out.mkdir(parents=True, exist_ok=True)

    blob = np.load(a.campaign / "pairs.npz")
    theta, x = blob["theta"], blob["x"]
    idx = [FEATURE_NAMES.index(c) for c in CMP]
    if a.corpus_cache and a.corpus_cache.exists():
        cache = json.loads(a.corpus_cache.read_text())["corpus_features"]
        cx_cmp = np.array([cache[c] for c in CMP], dtype=float)
        print(f"[surrogate] corpus features from cache {a.corpus_cache}")
    elif a.corpus:
        cx_cmp = corpus_features(a.corpus, "/tmp/_corp_canon.csv")[idx]
    else:
        raise SystemExit("need --corpus or --corpus-cache")

    # Standardize comparable features by campaign std, clip to ±4 so a single
    # heavy-tailed corpus feature can't dominate the L2 distance.
    xs = x[:, idx]
    mu, sd = xs.mean(0), xs.std(0) + 1e-6
    cxn = np.clip((cx_cmp - mu) / sd, -4, 4)
    xn = np.clip((xs - mu) / sd, -4, 4)
    dist = np.linalg.norm(xn - cxn, axis=1).astype("float32")  # objective per sim

    tn = P.normalize(theta).astype("float32")
    nval = max(1, len(tn) // 5)
    Xtr, Xva = torch.tensor(tn[nval:]), torch.tensor(tn[:nval])
    ytr, yva = torch.tensor(dist[nval:]), torch.tensor(dist[:nval])

    net = nn.Sequential(nn.Linear(P.dim(), 64), nn.SiLU(), nn.Linear(64, 64), nn.SiLU(), nn.Linear(64, 1))
    opt = torch.optim.Adam(net.parameters(), 1e-3)
    for _ in range(1500):
        opt.zero_grad()
        loss = ((net(Xtr).squeeze(-1) - ytr) ** 2).mean()
        loss.backward()
        opt.step()
    net.train(False)  # inference mode
    with torch.no_grad():
        pv = net(Xva).squeeze(-1).numpy()
    from scipy.stats import spearmanr
    rho = float(spearmanr(pv, yva.numpy()).statistic)
    print(f"surrogate Spearman (held-out, predicted vs true distance) = {rho:.3f}")

    import cma
    es = cma.CMAEvolutionStrategy(np.full(P.dim(), 0.5), 0.2,
                                  {"bounds": [0, 1], "verbose": -9, "seed": 0})
    for _ in range(80):
        sols = es.ask()
        with torch.no_grad():
            vals = net(torch.tensor(np.array(sols), dtype=torch.float32)).squeeze(-1).numpy()
        es.tell(sols, list(vals))
    theta_star = P.denormalize(np.clip(es.result.xbest, 0, 1))
    names = [p.name for p in P.PARAMS]
    print("corpus-matching θ* (surrogate argmin):")
    for n, v in zip(names, theta_star):
        print(f"  {n:14s} = {v:.3f}")
    print(f"(corpus log_amt_mean={cx_cmp[0]:.2f} std={cx_cmp[1]:.2f}; "
          f"amount_mu≈mean/0.63 sanity → ~{cx_cmp[0] / 0.63:.1f})")

    (a.out / "match_corpus.json").write_text(json.dumps({
        "surrogate_spearman": rho,
        "theta_star": dict(zip(names, theta_star.tolist())),
        "corpus_features": dict(zip(CMP, [float(v) for v in cx_cmp])),
    }, indent=2))
    print(f"saved {a.out / 'match_corpus.json'}")


if __name__ == "__main__":
    main()

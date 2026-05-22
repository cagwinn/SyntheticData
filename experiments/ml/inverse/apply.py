"""Apply the trained inverse posterior q(θ | x) to an out-of-sample GL → a posterior over
the generator parameters that GL most likely came from. The audit-analytics
capstone: point the SBC-calibrated posterior at the corpus.

Emits ONLY parameter posteriors (median + 90% credible interval), never
row-level corpus content — the privacy contract in inverse/SPEC.md.

    python -m inverse.apply --weights weights/inverse \\
        --gl-canonical /tmp/_corp_canon.csv --x-cache /tmp/corpus_x29.json --n 4000

Caveat (SPEC § "Distribution shift = the BF gap"): the posterior is trained on
synthetic; applied to an out-of-sample GL it is biased by exactly the forward-fidelity
gap §1 measures. Trust the well-identified knobs (amount_mu, fraud_rate);
read the rest as gap-limited.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import torch

from . import params as P
from .model import PosteriorFlow
from .simulate import summary_stats


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--weights", type=Path, required=True)
    ap.add_argument("--gl-canonical", type=Path, default=None,
                    help="GL with canonical columns (debit/credit/posting_date/source/...)")
    ap.add_argument("--x-cache", type=Path, default=None,
                    help="cached 29-dim summary-stat vector json (skips the GL pass)")
    ap.add_argument("--n", type=int, default=4000)
    ap.add_argument("--out", type=Path, default=None)
    a = ap.parse_args(argv)

    if a.x_cache and a.x_cache.exists():
        x = np.array(json.loads(a.x_cache.read_text())["x"], dtype="float32")
        print(f"[apply] x from cache {a.x_cache}")
    elif a.gl_canonical:
        x = summary_stats(a.gl_canonical)
        if a.x_cache:
            a.x_cache.write_text(json.dumps({"x": [float(v) for v in x]}))
            print(f"[apply] cached x → {a.x_cache}")
    else:
        raise SystemExit("need --gl-canonical or --x-cache")

    ck = torch.load(a.weights / "posterior.pt", map_location="cpu")
    m = PosteriorFlow(dim_theta=P.dim(), dim_x=ck["dim_x"])
    m.load_state_dict(ck["model"])
    m.train(False)
    with torch.no_grad():
        s = m.sample(torch.tensor(x, dtype=torch.float32), a.n).cpu().numpy()  # (n, d) normalized
    theta = P.denormalize(s)

    names = [p.name for p in P.PARAMS]
    print("posterior over the generator params the corpus most likely came from:")
    res = {}
    for j, nm in enumerate(names):
        col = theta[:, j]
        lo, med, hi = (float(v) for v in np.percentile(col, [5, 50, 95]))
        res[nm] = {"median": med, "ci90": [lo, hi]}
        print(f"  {nm:14s} median={med:.3f}  90% CI=[{lo:.3f}, {hi:.3f}]")
    if a.out:
        a.out.write_text(json.dumps(res, indent=2))
        print(f"saved {a.out}")


if __name__ == "__main__":
    main()

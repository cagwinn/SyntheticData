"""Apply the trained inverse posterior to MANY named x vectors (per-industry or
per-client) in one pass → a recovered-θ table. Reuses the single synthetic-
trained posterior (it is industry-agnostic — it maps x→θ); only the
conditioning x changes. Same privacy contract as inverse.apply: emits only
parameter posteriors (median + 90% CI), never row-level corpus content.

    python -m inverse.apply_batch --weights weights/inverse \\
        --xs runs/inverse/corpus_full_x29.by_industry.json --key industries \\
        --out runs/inverse/by_industry_posterior.json

Pairs with inverse.corpus_x --industries / --per-client (which write the
``.by_industry.json`` / ``.per_client.json`` this consumes).
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import torch

from . import params as P
from .model import PosteriorFlow


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--weights", type=Path, required=True)
    ap.add_argument("--xs", type=Path, required=True)
    ap.add_argument("--key", default="industries",
                    help="top-level dict key holding the {name: {x:[...]}} map")
    ap.add_argument("--n", type=int, default=4000)
    ap.add_argument("--out", type=Path, default=None)
    a = ap.parse_args(argv)

    blob = json.loads(a.xs.read_text())
    entries = blob[a.key]
    ck = torch.load(a.weights / "posterior.pt", map_location="cpu")
    m = PosteriorFlow(dim_theta=P.dim(), dim_x=ck["dim_x"])
    m.load_state_dict(ck["model"])
    m.train(False)

    names = [p.name for p in P.PARAMS]
    hdr = f"{'group':<30}{'n_lines':>12}  " + "  ".join(f"{n.split('.')[-1]:>20s}" for n in names)
    print(hdr)
    print("-" * len(hdr))
    res = {}
    for grp, d in entries.items():
        x = np.array(d["x"], dtype="float32")
        with torch.no_grad():
            s = m.sample(torch.tensor(x), a.n).cpu().numpy()
        theta = P.denormalize(s)
        row, cells = {}, []
        for j, nm in enumerate(names):
            lo, med, hi = (float(v) for v in np.percentile(theta[:, j], [5, 50, 95]))
            row[nm] = {"median": med, "ci90": [lo, hi], "ci_width": hi - lo}
            cells.append(f"{med:5.2f}[{lo:4.1f},{hi:4.1f}]")
        res[grp] = {"n_lines": d.get("n_lines"), "params": row}
        nl = d.get("n_lines") or 0
        print(f"{grp:<30}{nl:>12,}  " + "  ".join(f"{c:>20s}" for c in cells))
    if a.out:
        a.out.write_text(json.dumps(res, indent=2))
        print(f"saved {a.out}")


if __name__ == "__main__":
    main()

"""Validate the inverse posterior on held-out synthetic, where θ is known.

    python -m inverse.validate --data data/inverse --weights weights/inverse

Reports, per parameter:
  - posterior-mean absolute error (vs the true θ)
  - simulation-based calibration (SBC) rank: for a calibrated posterior, the
    rank of the true θ among posterior samples is uniform on [0, n_samples].
  - central credible-interval coverage (a 90% interval should contain the
    truth ~90% of the time) — the headline trust metric.

This is the whole point of doing inversion against a forward simulator: we can
measure how well 'running the engine backward' works BEFORE pointing it at any
out-of-sample GL.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import numpy as np
import torch

from . import params as P
from .model import PosteriorFlow


def coverage_and_sbc(model: PosteriorFlow, theta_true_n: np.ndarray,
                     x: torch.Tensor, n_samples: int = 500, cred: float = 0.90):
    d = P.dim()
    ranks = np.zeros((len(theta_true_n), d), dtype=int)
    covered = np.zeros((len(theta_true_n), d), dtype=bool)
    abs_err = np.zeros((len(theta_true_n), d))
    lo_q, hi_q = (1 - cred) / 2, 1 - (1 - cred) / 2
    model.train(False)  # inference mode: freeze dropout / running stats
    for i in range(len(theta_true_n)):
        s = model.sample(x[i], n_samples).cpu().numpy()   # (n_samples, d) normalized
        truth = theta_true_n[i]
        ranks[i] = (s < truth).sum(axis=0)
        lo = np.quantile(s, lo_q, axis=0)
        hi = np.quantile(s, hi_q, axis=0)
        covered[i] = (truth >= lo) & (truth <= hi)
        abs_err[i] = np.abs(s.mean(axis=0) - truth)
    return ranks, covered.mean(axis=0), abs_err.mean(axis=0)


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--data", type=Path, required=True)
    ap.add_argument("--weights", type=Path, required=True)
    ap.add_argument("--val-frac", type=float, default=0.2)
    ap.add_argument("--cred", type=float, default=0.90)
    ap.add_argument("--device", default="cpu")
    args = ap.parse_args(argv)
    dev = torch.device(args.device)

    blob = np.load(args.data / "pairs.npz")
    theta_n = P.normalize(blob["theta"]).astype("float32")
    x = blob["x"].astype("float32")
    n_val = max(1, int(len(theta_n) * args.val_frac))
    theta_va, x_va = theta_n[:n_val], torch.tensor(x[:n_val], device=dev)

    ckpt = torch.load(args.weights / "posterior.pt", map_location=dev)
    model = PosteriorFlow(dim_theta=P.dim(), dim_x=ckpt["dim_x"]).to(dev)
    model.load_state_dict(ckpt["model"])

    ranks, cov, err = coverage_and_sbc(model, theta_va, x_va, cred=args.cred)
    print(f"{'parameter':<55} {'mae(norm)':>10} {f'{int(args.cred*100)}%cov':>8}")
    for j, p in enumerate(P.PARAMS):
        flag = "" if 0.85 <= cov[j] <= 0.95 else "  miscalibrated"
        print(f"{p.name:<55} {err[j]:>10.3f} {cov[j]:>8.2f}{flag}")
    print("\nSBC: rank histograms should be ~uniform (export ranks for a plot).")
    print("TODO: save ranks -> SBC rank-histogram PNG; flag non-uniform params "
          "as poorly identified from GL alone (expected for some — an honest "
          "finding, not a bug).")


if __name__ == "__main__":
    main()

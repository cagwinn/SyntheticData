"""Train the amortized posterior q_φ(θ | x) on simulated pairs.

    python -m inverse.train --data data/inverse --out weights/inverse --epochs 300
"""

from __future__ import annotations

import argparse
from pathlib import Path

import numpy as np
import torch
from torch.utils.data import DataLoader, TensorDataset

from . import params as P
from .model import PosteriorFlow


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--data", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--epochs", type=int, default=300)
    ap.add_argument("--batch-size", type=int, default=256)
    ap.add_argument("--lr", type=float, default=1e-3)
    ap.add_argument("--val-frac", type=float, default=0.2)
    ap.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    args = ap.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)
    dev = torch.device(args.device)

    blob = np.load(args.data / "pairs.npz")
    theta = P.normalize(blob["theta"]).astype("float32")  # (n, d) in [0,1]
    x = blob["x"].astype("float32")                        # (n, dim_x)

    n_val = max(1, int(len(theta) * args.val_frac))
    tr = slice(n_val, None)
    va = slice(0, n_val)

    model = PosteriorFlow(dim_theta=P.dim(), dim_x=x.shape[1]).to(dev)
    xt = torch.tensor(x, device=dev)
    model.set_x_norm(xt[tr].mean(0), xt[tr].std(0))

    ds = TensorDataset(torch.tensor(theta[tr], device=dev), xt[tr])
    dl = DataLoader(ds, batch_size=args.batch_size, shuffle=True)
    opt = torch.optim.Adam(model.parameters(), lr=args.lr)

    theta_va = torch.tensor(theta[va], device=dev)
    x_va = xt[va]
    best = float("inf")
    for ep in range(1, args.epochs + 1):
        model.train(True)
        run = 0.0
        for th, xx in dl:
            loss = -model.log_prob(th, xx).mean()
            opt.zero_grad()
            loss.backward()
            opt.step()
            run += loss.item()
        model.train(False)  # inference mode (equivalent to .eval())
        with torch.no_grad():
            vloss = -model.log_prob(theta_va, x_va).mean().item()
        if vloss < best:
            best = vloss
            torch.save({"model": model.state_dict(), "dim_x": x.shape[1]},
                       args.out / "posterior.pt")
        if ep % 25 == 0:
            print(f"epoch {ep:4d}  train_nll={run/len(dl):.3f}  val_nll={vloss:.3f}")
    print(f"[inverse.train] best val_nll={best:.3f} -> {args.out/'posterior.pt'}")
    print("Next: python -m inverse.validate --data ... --weights ... (SBC + coverage)")


if __name__ == "__main__":
    main()

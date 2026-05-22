"""Train the conditional amount flow (Track 3).

    python -m flow.train --data data/flow --out weights/flow --epochs 50
"""

from __future__ import annotations

import argparse
from pathlib import Path

import torch
from torch.utils.data import DataLoader, TensorDataset

from .model import ConditionalAmountFlow


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--data", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--epochs", type=int, default=50)
    ap.add_argument("--batch-size", type=int, default=4096)
    ap.add_argument("--lr", type=float, default=1e-3)
    ap.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    args = ap.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)
    dev = torch.device(args.device)

    import pandas as pd

    df = pd.read_parquet(args.data / "amounts.parquet")
    y_raw = torch.tensor(df["y"].to_numpy(), dtype=torch.float32).unsqueeze(-1)
    # Standardize y so it lands inside the neural-spline domain. Without this
    # the NSF (default bound ~[-5,5]) cannot represent the heavy amount tail —
    # corpus signed-log1p amounts reach ~10.4, collapsing p99 to a few hundred.
    y_mean = float(y_raw.mean())
    y_std = float(y_raw.std()) or 1.0
    y = (y_raw - y_mean) / y_std
    c = torch.tensor(
        df.drop(columns=["y"]).to_numpy(), dtype=torch.float32
    )  # conditioning one-hots
    ds = TensorDataset(y, c)
    dl = DataLoader(ds, batch_size=args.batch_size, shuffle=True)

    model = ConditionalAmountFlow(cond_dim=c.size(1)).to(dev)
    opt = torch.optim.Adam(model.parameters(), lr=args.lr)

    model.train()
    for epoch in range(1, args.epochs + 1):
        running = 0.0
        for yb, cb in dl:
            yb, cb = yb.to(dev), cb.to(dev)
            loss = -model.log_prob(yb, cb).mean()
            opt.zero_grad()
            loss.backward()
            opt.step()
            running += loss.item()
        print(f"epoch {epoch:3d}  nll={running/len(dl):.4f}")

    torch.save({"model": model.state_dict(), "cond_dim": c.size(1),
                "y_mean": y_mean, "y_std": y_std},
               args.out / "amount_flow.pt")
    print(f"[flow.train] saved {args.out/'amount_flow.pt'}")
    print("TODO(flow): export spline knots for the candle AmountSampler port, "
          "and validate Benford MAD via common.bf_bridge.")


if __name__ == "__main__":
    main()

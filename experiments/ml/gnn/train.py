"""Train the relational GAE (Track 1).

    python -m gnn.train --data data/gnn --out weights/gnn --epochs 200

Runnable skeleton: loads the exported tensors, trains reconstruction, and
checkpoints. The degree-KL / triangle regularizers and the --privacy-check
gate are wired but raise until the corpus targets are exported (see SPEC.md).
"""

from __future__ import annotations

import argparse
from pathlib import Path

import torch

from .model import GraphAutoencoder


def negative_sample(num_nodes: int, num_neg: int, device) -> torch.Tensor:
    return torch.randint(0, num_nodes, (2, num_neg), device=device)


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--data", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--epochs", type=int, default=200)
    ap.add_argument("--lr", type=float, default=1e-3)
    ap.add_argument("--latent", type=int, default=64)
    ap.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    ap.add_argument("--lambda-degree", type=float, default=0.1)
    ap.add_argument("--lambda-triangle", type=float, default=0.1)
    ap.add_argument("--privacy-check", action="store_true",
                    help="run the embedding nearest-neighbour memorization probe")
    ap.add_argument("--dp-sgd", action="store_true",
                    help="enable DP-SGD (records epsilon in run config)")
    args = ap.parse_args(argv)

    args.out.mkdir(parents=True, exist_ok=True)
    dev = torch.device(args.device)

    edge_index = torch.load(args.data / "edge_index.pt").to(dev)
    x = torch.load(args.data / "node_feat.pt").to(dev)
    num_nodes = x.size(0)

    model = GraphAutoencoder(in_dim=x.size(1), latent=args.latent).to(dev)
    opt = torch.optim.Adam(model.parameters(), lr=args.lr)

    if args.dp_sgd:
        raise NotImplementedError(
            "TODO(gnn): wrap opt with opacus PrivacyEngine; persist (eps, delta) "
            "to out/run.json before any weight leaves the box."
        )

    model.train()
    for epoch in range(1, args.epochs + 1):
        opt.zero_grad()
        z = model.encode(x, edge_index)
        neg = negative_sample(num_nodes, edge_index.size(1), dev)
        loss = model.recon_loss(z, edge_index, neg)
        # Structural regularizers (raise until corpus targets exported):
        #   loss += args.lambda_degree   * model.degree_kl(z, target_hist)
        #   loss += args.lambda_triangle * model.triangle_penalty(z)
        loss.backward()
        opt.step()
        if epoch % 20 == 0:
            print(f"epoch {epoch:4d}  recon_loss={loss.item():.4f}")

    torch.save({"model": model.state_dict(), "args": vars(args)},
               args.out / "gae.pt")
    print(f"[gnn.train] saved {args.out/'gae.pt'}")

    if args.privacy_check:
        raise NotImplementedError(
            "TODO(gnn): nearest-neighbour membership probe — held-out edges must "
            "not be recoverable from embeddings above chance (SPEC.md § Privacy)."
        )


if __name__ == "__main__":
    main()

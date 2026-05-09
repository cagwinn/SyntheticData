"""Train an attribute-reconstruction Graph Autoencoder for node-level
anomaly scoring on the JE network.

Encoder: 2-layer GraphSAGE producing node embeddings z.
Decoder: MLP on concat(z[src], z[dst]) -> reconstructed edge_attr.
Loss: MSE on edge_attr (trained on train edges, evaluated on test).

Anomaly score per *edge* = MSE of edge_attr reconstruction.
Anomaly score per *node* = mean per-edge reconstruction MSE across
incident test edges — high error means the GNN can't predict the
edge attributes from local graph structure, which is what anomaly
labels capture by construction (round amounts, weekend dates, etc).

Ground truth: ``is_anomaly`` aggregated per node as
*fraction of incident edges flagged anomalous*.  We then evaluate
precision@K, AUC, and AUC-PR for K in {10, 25, 50}.

Usage::

    python -m scripts.ml.train_je_anomaly_gae \\
        --dataset data/ml/je_pyg_v1.pt \\
        --output models/ml/je_anomaly_gae.pt \\
        --epochs 80
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import torch
import torch.nn.functional as F
from sklearn.metrics import average_precision_score, roc_auc_score
from torch import nn
from torch_geometric.data import Data
from torch_geometric.nn import SAGEConv
from torch_geometric.utils import negative_sampling


def _set_inference_mode(module: nn.Module) -> None:
    module.train(False)


# ─── Encoder ─────────────────────────────────────────────────────────────────


class SageEncoder(nn.Module):
    def __init__(self, in_dim: int, hidden: int = 64, out: int = 32, dropout: float = 0.2) -> None:
        super().__init__()
        self.conv1 = SAGEConv(in_dim, hidden, aggr="mean")
        self.conv2 = SAGEConv(hidden, out, aggr="mean")
        self.dropout = dropout

    def forward(self, x: torch.Tensor, edge_index: torch.Tensor) -> torch.Tensor:
        h = F.relu(self.conv1(x, edge_index))
        h = F.dropout(h, p=self.dropout, training=self.training)
        return self.conv2(h, edge_index)


class AttrDecoder(nn.Module):
    """Map (z_src, z_dst) -> reconstructed edge_attr."""

    def __init__(self, z_dim: int, edge_attr_dim: int, hidden: int = 128, dropout: float = 0.2) -> None:
        super().__init__()
        self.net = nn.Sequential(
            nn.Linear(2 * z_dim, hidden),
            nn.ReLU(),
            nn.Dropout(dropout),
            nn.Linear(hidden, edge_attr_dim),
        )

    def forward(self, z: torch.Tensor, edge_index: torch.Tensor) -> torch.Tensor:
        src, dst = edge_index
        return self.net(torch.cat([z[src], z[dst]], dim=-1))


class AttrGAE(nn.Module):
    """Encoder + AttrDecoder bundled for state-dict portability."""

    def __init__(self, in_dim: int, edge_attr_dim: int, hidden: int = 64, out: int = 32, dropout: float = 0.2) -> None:
        super().__init__()
        self.encoder = SageEncoder(in_dim=in_dim, hidden=hidden, out=out, dropout=dropout)
        self.decoder = AttrDecoder(z_dim=out, edge_attr_dim=edge_attr_dim, hidden=hidden * 2, dropout=dropout)

    def forward(self, x: torch.Tensor, edge_index: torch.Tensor, target_edges: torch.Tensor) -> torch.Tensor:
        z = self.encoder(x, edge_index)
        return self.decoder(z, target_edges)


# ─── Training ────────────────────────────────────────────────────────────────


def train_gae(
    data: Data,
    epochs: int,
    lr: float,
    weight_decay: float,
    seed: int,
    device: torch.device,
    hidden: int = 64,
    out: int = 32,
) -> tuple[AttrGAE, list[dict[str, float]]]:
    torch.manual_seed(seed)
    np.random.seed(seed)

    data = data.to(device)
    train_mask = data.train_mask

    model = AttrGAE(
        in_dim=data.x.shape[1],
        edge_attr_dim=data.edge_attr.shape[1],
        hidden=hidden,
        out=out,
    ).to(device)
    optim = torch.optim.Adam(model.parameters(), lr=lr, weight_decay=weight_decay)

    train_edge_index = data.edge_index[:, train_mask]
    train_edge_attr = data.edge_attr[train_mask]

    history: list[dict[str, float]] = []
    for epoch in range(1, epochs + 1):
        model.train()
        optim.zero_grad()
        # Encoder sees train edges (no val/test leakage in messaging)
        recon = model(data.x, train_edge_index, train_edge_index)
        loss = F.mse_loss(recon, train_edge_attr)
        loss.backward()
        optim.step()

        if epoch % 5 == 0 or epoch == 1:
            history.append({"epoch": epoch, "loss": float(loss.item())})
            print(f"epoch {epoch:3d}  loss={loss.item():.4f}")

    return model, history


# ─── Per-node anomaly score ──────────────────────────────────────────────────


def compute_anomaly_scores(
    model: AttrGAE,
    data: Data,
    device: torch.device,
) -> tuple[np.ndarray, np.ndarray]:
    """Compute per-edge reconstruction MSE on test edges and aggregate
    to per-node anomaly scores.

    Returns (per_edge_score_for_test, per_node_score)."""
    _set_inference_mode(model)
    train_edge_index = data.edge_index[:, data.train_mask].to(device)
    test_edge_index = data.edge_index[:, data.test_mask].to(device)
    test_edge_attr = data.edge_attr[data.test_mask].to(device)

    with torch.no_grad():
        recon = model(data.x.to(device), train_edge_index, test_edge_index)
        per_edge_mse = ((recon - test_edge_attr) ** 2).mean(dim=-1).cpu().numpy()

    test_edges_np = test_edge_index.cpu().numpy()
    n_nodes = int(data.x.shape[0])
    incident_count = np.zeros(n_nodes, dtype=np.int64)
    incident_error = np.zeros(n_nodes, dtype=np.float64)
    for i in range(test_edges_np.shape[1]):
        src, dst = int(test_edges_np[0, i]), int(test_edges_np[1, i])
        incident_count[src] += 1
        incident_count[dst] += 1
        incident_error[src] += float(per_edge_mse[i])
        incident_error[dst] += float(per_edge_mse[i])

    score = np.zeros(n_nodes, dtype=np.float32)
    nz = incident_count > 0
    score[nz] = incident_error[nz] / incident_count[nz]
    return per_edge_mse, score


# ─── Ground truth ────────────────────────────────────────────────────────────


def node_anomaly_truth(data: Data) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Return (frac_anomalous_per_node, was_touched_per_node, incident_count_per_node)
    aggregating ``is_anomaly`` over all edges."""
    edge_index = data.edge_index.cpu().numpy()
    is_anomaly = data.is_anomaly.cpu().numpy()
    n_nodes = int(data.x.shape[0])

    incident = np.zeros(n_nodes, dtype=np.int64)
    incident_anom = np.zeros(n_nodes, dtype=np.int64)
    for i in range(edge_index.shape[1]):
        src, dst = int(edge_index[0, i]), int(edge_index[1, i])
        incident[src] += 1
        incident[dst] += 1
        if is_anomaly[i]:
            incident_anom[src] += 1
            incident_anom[dst] += 1

    frac = np.zeros(n_nodes, dtype=np.float32)
    nz = incident > 0
    frac[nz] = incident_anom[nz] / incident[nz]
    touched = (incident > 0).astype(np.float32)
    return frac, touched, incident


# ─── Evaluation ──────────────────────────────────────────────────────────────


def evaluate(scores: np.ndarray, truth_frac: np.ndarray, touched: np.ndarray, ks: list[int]) -> dict[str, float | dict[str, float]]:
    valid = touched > 0
    s = scores[valid]
    y_frac = truth_frac[valid]
    # binary label: above-median anomaly fraction = anomalous node
    threshold = np.median(y_frac[y_frac > 0]) if (y_frac > 0).any() else 0.0
    y_bin = (y_frac >= max(threshold, 0.05)).astype(np.float32)

    out: dict[str, float | dict[str, float]] = {
        "auc_roc": float(roc_auc_score(y_bin, s)) if y_bin.sum() > 0 and y_bin.sum() < len(y_bin) else 0.0,
        "auc_pr": float(average_precision_score(y_bin, s)) if y_bin.sum() > 0 else 0.0,
        "n_nodes": int(valid.sum()),
        "n_anomalous": int(y_bin.sum()),
        "anomaly_threshold_used": float(max(threshold, 0.05)),
    }
    for k in ks:
        if k > len(s):
            continue
        topk = np.argsort(-s)[:k]
        prec = float(y_bin[topk].mean())
        out[f"precision@{k}"] = prec
    return out


# ─── Main ────────────────────────────────────────────────────────────────────


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dataset", type=Path, default=Path("data/ml/je_pyg_v1.pt"))
    parser.add_argument("--output", type=Path, default=Path("models/ml/je_anomaly_gae.pt"))
    parser.add_argument("--metrics", type=Path, default=Path("models/ml/je_anomaly_metrics.json"))
    parser.add_argument("--epochs", type=int, default=80)
    parser.add_argument("--lr", type=float, default=0.005)
    parser.add_argument("--weight-decay", type=float, default=1e-5)
    parser.add_argument("--hidden", type=int, default=64)
    parser.add_argument("--out", type=int, default=32)
    parser.add_argument("--seed", type=int, default=20260509)
    parser.add_argument("--device", type=str, default="auto", choices=["auto", "cpu", "cuda"])
    args = parser.parse_args()

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.metrics.parent.mkdir(parents=True, exist_ok=True)

    payload = torch.load(args.dataset, weights_only=False)
    data: Data = payload["data"]
    print(f"loaded dataset: {data}")

    if args.device == "auto":
        device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    else:
        device = torch.device(args.device)
    print(f"device: {device}")

    model, history = train_gae(
        data=data,
        epochs=args.epochs,
        lr=args.lr,
        weight_decay=args.weight_decay,
        seed=args.seed,
        device=device,
        hidden=args.hidden,
        out=args.out,
    )

    print("\n=== anomaly scoring ===")
    per_edge_mse, node_scores = compute_anomaly_scores(model, data, device=device)
    truth_frac, touched, incident = node_anomaly_truth(data)
    print(
        f"  nodes touched: {int(touched.sum())} / {len(touched)}  "
        f"avg anomaly frac (touched): {truth_frac[touched > 0].mean():.4f}"
    )

    # Edge-level evaluation: does per-edge MSE separate anomalous from normal?
    test_anomaly = data.is_anomaly[data.test_mask].cpu().numpy().astype(np.float32)
    test_edge_auc = float(roc_auc_score(test_anomaly, per_edge_mse)) if test_anomaly.sum() > 0 else 0.0
    test_edge_pr = float(average_precision_score(test_anomaly, per_edge_mse)) if test_anomaly.sum() > 0 else 0.0
    print(f"  per-edge anomaly  AUC={test_edge_auc:.4f}  PR={test_edge_pr:.4f}  n_pos={int(test_anomaly.sum())}")

    results = evaluate(node_scores, truth_frac, touched, ks=[10, 25, 50])
    print("  per-node anomaly:")
    for k, v in results.items():
        print(f"    {k}: {v}")

    torch.save(
        {
            "model_state_dict": model.state_dict(),
            "model_config": {
                "in_dim": data.x.shape[1],
                "edge_attr_dim": data.edge_attr.shape[1],
                "hidden": args.hidden,
                "out": args.out,
                "dropout": 0.2,
            },
            "node_scores": node_scores,
            "per_edge_mse_test": per_edge_mse,
            "training_config": vars(args) | {"device": str(device)},
        },
        args.output,
    )
    print(f"\nsaved -> {args.output}")

    metrics_payload = {
        "node_results": {k: (v if not isinstance(v, np.floating) else float(v)) for k, v in results.items()},
        "edge_results": {
            "auc_roc": test_edge_auc,
            "auc_pr": test_edge_pr,
            "n_test_anomaly": int(test_anomaly.sum()),
            "n_test_edges": int(len(test_anomaly)),
        },
        "history": history,
        "summary": {
            "n_nodes_total": int(data.x.shape[0]),
            "n_nodes_touched": int(touched.sum()),
            "avg_anomaly_frac_touched": float(truth_frac[touched > 0].mean()),
        },
    }
    args.metrics.write_text(json.dumps(metrics_payload, indent=2))
    print(f"saved metrics -> {args.metrics}")


if __name__ == "__main__":
    main()

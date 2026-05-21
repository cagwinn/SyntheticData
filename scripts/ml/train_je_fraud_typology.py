"""Train a multi-class fraud-*typology* classifier on the JE network.

The binary ``train_je_fraud_gnn.py`` answers *is this edge fraud?*. This
trainer answers the harder, v5.27-enabled question: *which of the 20 fraud
typologies is it?* (``SuspenseAccountAbuse``, ``SplitTransaction``,
``RevenueManipulation``, …) using the ``fraud_type`` column now carried on
``je_network``.

Architecture mirrors the binary model so the comparison is clean:

  * sklearn multinomial LogisticRegression on edge features alone — the
    "does the graph help?" control.
  * GraphSAGE 2-layer encoder (message-passing over the full account graph)
    + a softmax edge head over the K fraud typologies.

Trained on fraud edges only (``fraud_type_idx > 0``); class-balanced loss
(inverse-frequency weights) so the rare typologies aren't ignored. Reports
accuracy, macro-F1, weighted-F1, top-3 accuracy, and a per-typology
precision/recall/support breakdown.

Usage::

    python -m scripts.ml.train_je_fraud_typology \\
        --dataset data/ml/je_pyg_v527.pt \\
        --output models/ml/je_fraud_typology.pt \\
        --epochs 120
"""
from __future__ import annotations

import argparse
import json
import time
from pathlib import Path

import numpy as np
import torch
import torch.nn.functional as F
from sklearn.linear_model import LogisticRegression
from sklearn.metrics import (
    accuracy_score,
    classification_report,
    f1_score,
    top_k_accuracy_score,
)
from torch import nn
from torch_geometric.data import Data
from torch_geometric.nn import SAGEConv


def _set_inference_mode(module: nn.Module) -> None:
    module.train(False)


# ─── Model ───────────────────────────────────────────────────────────────────


class TypologyGNN(nn.Module):
    """GraphSAGE encoder + multi-class edge head."""

    def __init__(
        self,
        node_in: int,
        edge_in: int,
        num_classes: int,
        hidden: int = 64,
        out: int = 64,
        head_hidden: int = 128,
        dropout: float = 0.2,
    ) -> None:
        super().__init__()
        self.conv1 = SAGEConv(node_in, hidden, aggr="mean")
        self.conv2 = SAGEConv(hidden, out, aggr="mean")
        self.dropout = dropout
        self.head = nn.Sequential(
            nn.Linear(2 * out + edge_in, head_hidden),
            nn.ReLU(),
            nn.Dropout(dropout),
            nn.Linear(head_hidden, num_classes),
        )

    def encode(self, x: torch.Tensor, edge_index: torch.Tensor) -> torch.Tensor:
        h = F.relu(self.conv1(x, edge_index))
        h = F.dropout(h, p=self.dropout, training=self.training)
        return self.conv2(h, edge_index)

    def edge_logits(
        self, h: torch.Tensor, edge_index: torch.Tensor, edge_attr: torch.Tensor
    ) -> torch.Tensor:
        src, dst = edge_index
        z = torch.cat([h[src], h[dst], edge_attr], dim=-1)
        return self.head(z)

    def forward(
        self, x: torch.Tensor, edge_index: torch.Tensor, edge_attr: torch.Tensor
    ) -> torch.Tensor:
        return self.edge_logits(self.encode(x, edge_index), edge_index, edge_attr)


# ─── Metrics ─────────────────────────────────────────────────────────────────


def multiclass_metrics(
    y_true: np.ndarray,
    y_pred: np.ndarray,
    y_proba: np.ndarray,
    class_names: list[str],
) -> dict:
    labels = list(range(len(class_names)))
    out = {
        "accuracy": float(accuracy_score(y_true, y_pred)),
        "macro_f1": float(f1_score(y_true, y_pred, average="macro", labels=labels, zero_division=0)),
        "weighted_f1": float(f1_score(y_true, y_pred, average="weighted", labels=labels, zero_division=0)),
        "n": int(len(y_true)),
    }
    # top-3 (only meaningful with ≥4 classes)
    if len(class_names) >= 4:
        out["top3_accuracy"] = float(
            top_k_accuracy_score(y_true, y_proba, k=3, labels=labels)
        )
    out["per_class"] = classification_report(
        y_true, y_pred, labels=labels, target_names=class_names,
        output_dict=True, zero_division=0,
    )
    return out


# ─── Baseline ────────────────────────────────────────────────────────────────


def baseline_logreg(
    edge_attr: np.ndarray,
    y: np.ndarray,
    train_sel: np.ndarray,
    test_sel: np.ndarray,
    class_names: list[str],
) -> dict:
    # lbfgs defaults to multinomial for multiclass; the explicit multi_class
    # arg was deprecated/removed in recent sklearn, so we omit it.
    clf = LogisticRegression(max_iter=3000, class_weight="balanced", random_state=0)
    clf.fit(edge_attr[train_sel], y[train_sel])
    proba = clf.predict_proba(edge_attr[test_sel])
    # Map sklearn's class order back to the dense 0..K-1 index space.
    full = np.zeros((proba.shape[0], len(class_names)), dtype=np.float64)
    full[:, clf.classes_] = proba
    pred = full.argmax(axis=1)
    return multiclass_metrics(y[test_sel], pred, full, class_names)


# ─── Train ───────────────────────────────────────────────────────────────────


def train(
    data: Data,
    num_classes: int,
    class_weight: torch.Tensor,
    epochs: int,
    lr: float,
    weight_decay: float,
    patience: int,
    seed: int,
    device: torch.device,
) -> tuple[TypologyGNN, list[dict]]:
    torch.manual_seed(seed)
    np.random.seed(seed)
    data = data.to(device)

    # target = fraud_type_idx - 1 (drop the reserved <none>=0 class).
    target = (data.fraud_type_idx - 1).long()
    fraud = data.fraud_type_idx > 0
    train_sel = (fraud & data.train_mask).to(device)
    val_sel = (fraud & data.val_mask).to(device)

    model = TypologyGNN(
        node_in=data.x.shape[1], edge_in=data.edge_attr.shape[1], num_classes=num_classes
    ).to(device)
    optim = torch.optim.Adam(model.parameters(), lr=lr, weight_decay=weight_decay)
    loss_fn = nn.CrossEntropyLoss(weight=class_weight.to(device))

    history: list[dict] = []
    best_macro_f1 = -1.0
    best_state = None
    no_improve = 0
    y_val_np = target[val_sel].cpu().numpy()

    for epoch in range(1, epochs + 1):
        model.train()
        optim.zero_grad()
        logits = model(data.x, data.edge_index, data.edge_attr)
        loss = loss_fn(logits[train_sel], target[train_sel])
        loss.backward()
        optim.step()

        _set_inference_mode(model)
        with torch.no_grad():
            logits_eval = model(data.x, data.edge_index, data.edge_attr)
            val_pred = logits_eval[val_sel].argmax(dim=-1).cpu().numpy()
        val_macro_f1 = f1_score(
            y_val_np, val_pred, average="macro", labels=list(range(num_classes)), zero_division=0
        )
        history.append({"epoch": epoch, "loss": float(loss.item()), "val_macro_f1": float(val_macro_f1)})
        if epoch % 10 == 0 or epoch == 1:
            print(f"epoch {epoch:3d}  loss={loss.item():.4f}  val_macro_f1={val_macro_f1:.4f}")

        if val_macro_f1 > best_macro_f1:
            best_macro_f1 = val_macro_f1
            best_state = {k: v.detach().cpu().clone() for k, v in model.state_dict().items()}
            no_improve = 0
        else:
            no_improve += 1
            if no_improve >= patience:
                print(f"early stop at epoch {epoch} (val macro-F1 plateaued at {best_macro_f1:.4f})")
                break

    if best_state is not None:
        model.load_state_dict(best_state)
    return model, history


# ─── Main ────────────────────────────────────────────────────────────────────


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dataset", type=Path, default=Path("data/ml/je_pyg_v527.pt"))
    parser.add_argument("--output", type=Path, default=Path("models/ml/je_fraud_typology.pt"))
    parser.add_argument("--metrics", type=Path, default=Path("models/ml/je_fraud_typology_metrics.json"))
    parser.add_argument("--epochs", type=int, default=120)
    parser.add_argument("--lr", type=float, default=0.005)
    parser.add_argument("--weight-decay", type=float, default=1e-5)
    parser.add_argument("--patience", type=int, default=20)
    parser.add_argument("--seed", type=int, default=20260521)
    parser.add_argument("--device", type=str, default="auto", choices=["auto", "cpu", "cuda"])
    args = parser.parse_args()

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.metrics.parent.mkdir(parents=True, exist_ok=True)

    payload = torch.load(args.dataset, weights_only=False)
    data: Data = payload["data"]
    vocab: dict[str, int] = payload["fraud_type_vocab"]
    if not hasattr(data, "fraud_type_idx"):
        raise SystemExit("dataset has no fraud_type_idx — rebuild with the v5.27 builder")

    # class_names ordered by dense index (drop the reserved <none>=0).
    idx_to_name = {v: k for k, v in vocab.items()}
    num_classes = len(vocab) - 1
    class_names = [idx_to_name[i + 1] for i in range(num_classes)]
    print(f"loaded dataset: {data}")
    print(f"{num_classes} fraud typologies: {class_names}")

    device = torch.device("cuda" if (args.device == "auto" and torch.cuda.is_available()) else
                          (args.device if args.device != "auto" else "cpu"))
    print(f"device: {device}")

    target_np = (data.fraud_type_idx - 1).numpy()
    fraud_np = data.fraud_type_idx.numpy() > 0
    train_sel = fraud_np & data.train_mask.numpy()
    test_sel = fraud_np & data.test_mask.numpy()
    print(f"fraud edges — train: {int(train_sel.sum())}  test: {int(test_sel.sum())}")

    # Inverse-frequency class weights from the train split.
    counts = np.bincount(target_np[train_sel], minlength=num_classes).astype(np.float64)
    weights = np.where(counts > 0, counts.sum() / (num_classes * np.maximum(counts, 1)), 0.0)
    class_weight = torch.tensor(weights, dtype=torch.float32)

    # ── Baseline ─────────────────────────────────────────────────────────
    print("\n=== multinomial LogisticRegression baseline (edge features only) ===")
    t0 = time.time()
    baseline = baseline_logreg(data.edge_attr.numpy(), target_np, train_sel, test_sel, class_names)
    print(f"  acc={baseline['accuracy']:.4f}  macro_f1={baseline['macro_f1']:.4f}  "
          f"weighted_f1={baseline['weighted_f1']:.4f}  top3={baseline.get('top3_accuracy', float('nan')):.4f}"
          f"  ({time.time() - t0:.2f}s)")

    # ── GNN ──────────────────────────────────────────────────────────────
    print("\n=== GraphSAGE typology classifier ===")
    model, history = train(
        data=data, num_classes=num_classes, class_weight=class_weight,
        epochs=args.epochs, lr=args.lr, weight_decay=args.weight_decay,
        patience=args.patience, seed=args.seed, device=device,
    )

    _set_inference_mode(model)
    with torch.no_grad():
        logits = model(data.x.to(device), data.edge_index.to(device), data.edge_attr.to(device))
        proba = torch.softmax(logits, dim=-1).cpu().numpy()
    test_proba = proba[test_sel]
    test_pred = test_proba.argmax(axis=1)
    gnn = multiclass_metrics(target_np[test_sel], test_pred, test_proba, class_names)
    print(f"  acc={gnn['accuracy']:.4f}  macro_f1={gnn['macro_f1']:.4f}  "
          f"weighted_f1={gnn['weighted_f1']:.4f}  top3={gnn.get('top3_accuracy', float('nan')):.4f}")

    print("\n=== per-typology (GNN, test) ===")
    for name in class_names:
        r = gnn["per_class"].get(name, {})
        if r.get("support", 0):
            print(f"  {name:30s} P={r['precision']:.3f} R={r['recall']:.3f} "
                  f"F1={r['f1-score']:.3f} n={int(r['support'])}")

    torch.save(
        {
            "model_state_dict": model.state_dict(),
            "model_config": {
                "node_in": data.x.shape[1], "edge_in": data.edge_attr.shape[1],
                "num_classes": num_classes, "hidden": 64, "out": 64,
                "head_hidden": 128, "dropout": 0.2,
            },
            "fraud_type_vocab": vocab,
            "class_names": class_names,
            "feature_columns": payload.get("feature_columns"),
            "training_config": vars(args) | {"device": str(device)},
        },
        args.output,
    )
    print(f"\nsaved -> {args.output}")
    args.metrics.write_text(json.dumps({"baseline_logreg": baseline, "gnn": gnn, "history": history}, indent=2))
    print(f"saved metrics -> {args.metrics}")


if __name__ == "__main__":
    main()

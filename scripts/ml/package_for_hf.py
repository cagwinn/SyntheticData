"""Package the trained models + preprocessor + metadata for HF Hub upload.

Produces a self-contained directory ready to push to
``VynFi/je-fraud-gnn``.
"""
from __future__ import annotations

import argparse
import json
import shutil
from pathlib import Path

import numpy as np
import torch


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dataset", type=Path, default=Path("data/ml/je_pyg_v1.pt"))
    parser.add_argument("--fraud-model", type=Path, default=Path("models/ml/je_fraud_gnn.pt"))
    parser.add_argument("--fraud-metrics", type=Path, default=Path("models/ml/je_fraud_metrics.json"))
    parser.add_argument("--anomaly-model", type=Path, default=Path("models/ml/je_anomaly_gae.pt"))
    parser.add_argument("--anomaly-metrics", type=Path, default=Path("models/ml/je_anomaly_metrics.json"))
    parser.add_argument("--out-dir", type=Path, default=Path("models/ml/hf_bundle"))
    args = parser.parse_args()

    args.out_dir.mkdir(parents=True, exist_ok=True)

    # 1) Copy weights
    shutil.copy(args.fraud_model, args.out_dir / "je_fraud_gnn.pt")
    shutil.copy(args.anomaly_model, args.out_dir / "je_anomaly_gae.pt")

    # 2) Build preprocessor — pulls scalers, node_index, full graph from dataset
    payload = torch.load(args.dataset, weights_only=False)
    data = payload["data"]
    # Recover unscaled node features (they were scaled in the .pt; the scaler
    # stores mean/scale so we can invert)
    node_x_scaled = data.x.cpu().numpy()
    node_mean = np.asarray(payload["node_feature_scaler_mean"], dtype=np.float32)
    node_scale = np.asarray(payload["node_feature_scaler_scale"], dtype=np.float32)
    node_x_raw = node_x_scaled * node_scale + node_mean

    preprocessor = {
        "node_index": payload["node_index"],
        "edge_attr_scaler_mean": payload["edge_attr_scaler_mean"],
        "edge_attr_scaler_scale": payload["edge_attr_scaler_scale"],
        "node_feature_scaler_mean": payload["node_feature_scaler_mean"],
        "node_feature_scaler_scale": payload["node_feature_scaler_scale"],
        "node_features_raw": node_x_raw,
        "edge_index": data.edge_index.cpu().numpy(),
        "feature_columns": payload["feature_columns"],
        "schema_version": payload.get("schema_version", 1),
        "build_seed": payload.get("build_seed"),
    }
    torch.save(preprocessor, args.out_dir / "preprocessor.pt")

    # 3) Compose metadata.json (model card data)
    fraud_metrics = json.loads(args.fraud_metrics.read_text())
    anomaly_metrics = json.loads(args.anomaly_metrics.read_text())
    metadata = {
        "datasynth_release": "v5.9.0",
        "source_dataset": "VynFi/vynfi-journal-entries-1m",
        "n_nodes": int(data.x.shape[0]),
        "n_edges": int(data.edge_index.shape[1]),
        "n_train_edges": int(data.train_mask.sum().item()),
        "n_val_edges": int(data.val_mask.sum().item()),
        "n_test_edges": int(data.test_mask.sum().item()),
        "fraud_rate": float(data.y.float().mean().item()),
        "anomaly_rate": float(data.is_anomaly.float().mean().item()),
        "fraud_threshold": float(fraud_metrics["gnn"]["test"]["threshold"]),
        "fraud_metrics": fraud_metrics,
        "anomaly_metrics": anomaly_metrics,
    }
    (args.out_dir / "metadata.json").write_text(json.dumps(metadata, indent=2, default=float))

    print(f"packaged HF bundle -> {args.out_dir}")
    for p in sorted(args.out_dir.iterdir()):
        size_kb = p.stat().st_size / 1024
        print(f"  {p.name:30s}  {size_kb:8.1f} KB")


if __name__ == "__main__":
    main()

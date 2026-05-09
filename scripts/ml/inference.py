"""Shared inference utilities for the JE fraud GNN showcase.

Used by both the model-packaging script and the Gradio Space.
"""
from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np
import pandas as pd
import torch
from torch import nn

from scripts.ml.train_je_fraud_gnn import EdgeFraudGNN
from scripts.ml.train_je_anomaly_gae import AttrGAE


_ROUND_LEVELS = np.array([1_000.0, 5_000.0, 10_000.0, 25_000.0, 50_000.0, 100_000.0])
BUSINESS_PROCESSES = ["P2P", "O2C", "R2R", "H2R", "A2R"]


@dataclass
class InferenceBundle:
    fraud_model: EdgeFraudGNN
    anomaly_model: AttrGAE
    node_index: dict[str, int]
    edge_attr_scaler_mean: np.ndarray
    edge_attr_scaler_scale: np.ndarray
    node_feature_scaler_mean: np.ndarray
    node_feature_scaler_scale: np.ndarray
    node_features_raw: np.ndarray  # un-scaled, (n_nodes, n_node_feat)
    edge_index: np.ndarray  # full graph (2, n_edges) — for message passing
    feature_columns: dict[str, list[str]]
    fraud_threshold: float
    metadata: dict[str, Any]

    @property
    def node_features_scaled(self) -> torch.Tensor:
        x = (self.node_features_raw - self.node_feature_scaler_mean) / self.node_feature_scaler_scale
        return torch.from_numpy(x.astype(np.float32))

    def encode_edges(
        self,
        from_account: list[str],
        to_account: list[str],
        amount: list[float],
        business_process: list[str],
        posting_date: list[str],
        confidence: list[float] | None = None,
    ) -> tuple[torch.Tensor, torch.Tensor]:
        """Map raw edge inputs -> (edge_index, edge_attr_scaled) tensors."""
        n = len(from_account)
        if confidence is None:
            confidence = [1.0] * n
        df = pd.DataFrame(
            {
                "from_account": [str(a) for a in from_account],
                "to_account": [str(a) for a in to_account],
                "amount": amount,
                "business_process": business_process,
                "posting_date": pd.to_datetime(posting_date, errors="coerce"),
                "confidence": confidence,
            }
        )

        src = df["from_account"].map(self.node_index).to_numpy(dtype=np.int64)
        dst = df["to_account"].map(self.node_index).to_numpy(dtype=np.int64)
        if np.isnan(src.astype(float)).any() or np.isnan(dst.astype(float)).any():
            missing = df.loc[df["from_account"].isin(self.node_index) == False, "from_account"].unique().tolist()
            missing += df.loc[df["to_account"].isin(self.node_index) == False, "to_account"].unique().tolist()
            raise ValueError(f"unknown account number(s): {missing}")
        edge_index = np.stack([src, dst], axis=0)

        # Amount block
        a = df["amount"].astype(float).to_numpy()
        log_amt = np.log1p(a).astype(np.float32)
        diffs = np.abs(a[:, None] - _ROUND_LEVELS[None, :])
        nearest = diffs.min(axis=1)
        is_round = (nearest < 1.0).astype(np.float32)
        log_dist = np.log1p(nearest).astype(np.float32)
        nearest_idx = diffs.argmin(axis=1)
        per_level = np.zeros((n, len(_ROUND_LEVELS)), dtype=np.float32)
        is_close = nearest < 1.0
        per_level[is_close, nearest_idx[is_close]] = 1.0

        # Process one-hot
        bp_oh = (
            pd.get_dummies(df["business_process"].fillna("UNK"), prefix="bp")
            .reindex(columns=[f"bp_{p}" for p in BUSINESS_PROCESSES], fill_value=0)
            .astype(np.float32)
            .to_numpy()
        )

        # Date features
        dt = df["posting_date"]
        doy = dt.dt.dayofyear.fillna(1).to_numpy()
        woy = dt.dt.isocalendar().week.astype(int).to_numpy()
        dow = dt.dt.dayofweek.fillna(0).to_numpy()
        is_weekend = (dow >= 5).astype(np.float32)
        date_feats = np.stack(
            [
                np.sin(2 * np.pi * doy / 366),
                np.cos(2 * np.pi * doy / 366),
                np.sin(2 * np.pi * woy / 53),
                np.cos(2 * np.pi * woy / 53),
                np.sin(2 * np.pi * dow / 7),
                np.cos(2 * np.pi * dow / 7),
                is_weekend,
            ],
            axis=1,
        ).astype(np.float32)

        confidence_arr = df["confidence"].astype(float).to_numpy().reshape(-1, 1).astype(np.float32)

        edge_attr = np.concatenate(
            [
                log_amt[:, None],
                is_round[:, None],
                log_dist[:, None],
                per_level,
                confidence_arr,
                bp_oh,
                date_feats,
            ],
            axis=1,
        )
        edge_attr_scaled = (
            (edge_attr - self.edge_attr_scaler_mean) / self.edge_attr_scaler_scale
        ).astype(np.float32)

        return torch.from_numpy(edge_index), torch.from_numpy(edge_attr_scaled)

    @torch.no_grad()
    def predict_fraud(
        self,
        from_account: list[str],
        to_account: list[str],
        amount: list[float],
        business_process: list[str],
        posting_date: list[str],
        confidence: list[float] | None = None,
    ) -> np.ndarray:
        target_edge_index, target_edge_attr = self.encode_edges(
            from_account, to_account, amount, business_process, posting_date, confidence
        )
        # Use the published full graph (training edges) for message passing —
        # this is required for the GraphSAGE encoder to produce stable node
        # embeddings that match training.
        graph_edge_index = torch.from_numpy(self.edge_index)
        x = self.node_features_scaled

        self.fraud_model.train(False)
        h = self.fraud_model.encode(x, graph_edge_index)
        logits = self.fraud_model.edge_logits(h, target_edge_index, target_edge_attr)
        return torch.sigmoid(logits).cpu().numpy()

    @torch.no_grad()
    def anomaly_score_edges(
        self,
        from_account: list[str],
        to_account: list[str],
        amount: list[float],
        business_process: list[str],
        posting_date: list[str],
        confidence: list[float] | None = None,
    ) -> np.ndarray:
        """Return per-edge MSE — high values indicate unusual edge attributes."""
        target_edge_index, target_edge_attr = self.encode_edges(
            from_account, to_account, amount, business_process, posting_date, confidence
        )
        graph_edge_index = torch.from_numpy(self.edge_index)
        x = self.node_features_scaled

        self.anomaly_model.train(False)
        recon = self.anomaly_model(x, graph_edge_index, target_edge_index)
        return ((recon - target_edge_attr) ** 2).mean(dim=-1).cpu().numpy()


def load_bundle(model_dir: Path | str) -> InferenceBundle:
    """Load every artefact needed for inference from a directory layout::

        model_dir/
          ├── je_fraud_gnn.pt
          ├── je_anomaly_gae.pt
          ├── preprocessor.pt
          └── metadata.json
    """
    model_dir = Path(model_dir)

    fraud_payload = torch.load(model_dir / "je_fraud_gnn.pt", weights_only=False, map_location="cpu")
    anomaly_payload = torch.load(model_dir / "je_anomaly_gae.pt", weights_only=False, map_location="cpu")
    preprocessor = torch.load(model_dir / "preprocessor.pt", weights_only=False, map_location="cpu")
    metadata = json.loads((model_dir / "metadata.json").read_text())

    fraud_model = EdgeFraudGNN(**fraud_payload["model_config"])
    fraud_model.load_state_dict(fraud_payload["model_state_dict"])
    fraud_model.train(False)

    anomaly_model = AttrGAE(**anomaly_payload["model_config"])
    anomaly_model.load_state_dict(anomaly_payload["model_state_dict"])
    anomaly_model.train(False)

    return InferenceBundle(
        fraud_model=fraud_model,
        anomaly_model=anomaly_model,
        node_index=preprocessor["node_index"],
        edge_attr_scaler_mean=np.asarray(preprocessor["edge_attr_scaler_mean"], dtype=np.float32),
        edge_attr_scaler_scale=np.asarray(preprocessor["edge_attr_scaler_scale"], dtype=np.float32),
        node_feature_scaler_mean=np.asarray(preprocessor["node_feature_scaler_mean"], dtype=np.float32),
        node_feature_scaler_scale=np.asarray(preprocessor["node_feature_scaler_scale"], dtype=np.float32),
        node_features_raw=np.asarray(preprocessor["node_features_raw"], dtype=np.float32),
        edge_index=np.asarray(preprocessor["edge_index"], dtype=np.int64),
        feature_columns=preprocessor["feature_columns"],
        fraud_threshold=float(metadata.get("fraud_threshold", 0.5)),
        metadata=metadata,
    )

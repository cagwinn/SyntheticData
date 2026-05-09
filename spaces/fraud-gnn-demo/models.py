"""Vendored model classes + inference bundle for the Gradio Space.

Self-contained — does not import from the engine repo so the Space can
deploy from `VynFi/je-fraud-gnn` without pulling the full SyntheticData
codebase.
"""
from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np
import pandas as pd
import torch
import torch.nn.functional as F
from torch import nn
from torch_geometric.nn import SAGEConv

ROUND_LEVELS = np.array([1_000.0, 5_000.0, 10_000.0, 25_000.0, 50_000.0, 100_000.0])
BUSINESS_PROCESSES = ["P2P", "O2C", "R2R", "H2R", "A2R"]


# ─── Model classes (must match training scripts byte-for-byte) ───────────────


class EdgeFraudGNN(nn.Module):
    def __init__(
        self,
        node_in: int,
        edge_in: int,
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
            nn.Linear(head_hidden, 1),
        )

    def encode(self, x: torch.Tensor, edge_index: torch.Tensor) -> torch.Tensor:
        h = F.relu(self.conv1(x, edge_index))
        h = F.dropout(h, p=self.dropout, training=self.training)
        h = self.conv2(h, edge_index)
        return h

    def edge_logits(self, h, edge_index, edge_attr):
        src, dst = edge_index
        z = torch.cat([h[src], h[dst], edge_attr], dim=-1)
        return self.head(z).squeeze(-1)


class SageEncoder(nn.Module):
    def __init__(self, in_dim: int, hidden: int = 64, out: int = 32, dropout: float = 0.2) -> None:
        super().__init__()
        self.conv1 = SAGEConv(in_dim, hidden, aggr="mean")
        self.conv2 = SAGEConv(hidden, out, aggr="mean")
        self.dropout = dropout

    def forward(self, x, edge_index):
        h = F.relu(self.conv1(x, edge_index))
        h = F.dropout(h, p=self.dropout, training=self.training)
        return self.conv2(h, edge_index)


class AttrDecoder(nn.Module):
    def __init__(self, z_dim: int, edge_attr_dim: int, hidden: int = 128, dropout: float = 0.2) -> None:
        super().__init__()
        self.net = nn.Sequential(
            nn.Linear(2 * z_dim, hidden),
            nn.ReLU(),
            nn.Dropout(dropout),
            nn.Linear(hidden, edge_attr_dim),
        )

    def forward(self, z, edge_index):
        src, dst = edge_index
        return self.net(torch.cat([z[src], z[dst]], dim=-1))


class AttrGAE(nn.Module):
    def __init__(self, in_dim: int, edge_attr_dim: int, hidden: int = 64, out: int = 32, dropout: float = 0.2) -> None:
        super().__init__()
        self.encoder = SageEncoder(in_dim=in_dim, hidden=hidden, out=out, dropout=dropout)
        self.decoder = AttrDecoder(z_dim=out, edge_attr_dim=edge_attr_dim, hidden=hidden * 2, dropout=dropout)

    def forward(self, x, edge_index, target_edges):
        z = self.encoder(x, edge_index)
        return self.decoder(z, target_edges)


# ─── Inference bundle ────────────────────────────────────────────────────────


@dataclass
class InferenceBundle:
    fraud_model: EdgeFraudGNN
    anomaly_model: AttrGAE
    node_index: dict[str, int]
    edge_attr_scaler_mean: np.ndarray
    edge_attr_scaler_scale: np.ndarray
    node_feature_scaler_mean: np.ndarray
    node_feature_scaler_scale: np.ndarray
    node_features_raw: np.ndarray
    edge_index: np.ndarray
    feature_columns: dict[str, list[str]]
    fraud_threshold: float
    metadata: dict[str, Any]

    @property
    def node_features_scaled(self) -> torch.Tensor:
        x = (self.node_features_raw - self.node_feature_scaler_mean) / self.node_feature_scaler_scale
        return torch.from_numpy(x.astype(np.float32))

    @property
    def reverse_node_index(self) -> dict[int, str]:
        return {v: k for k, v in self.node_index.items()}

    def encode_edges(
        self,
        from_account,
        to_account,
        amount,
        business_process,
        posting_date,
        confidence=None,
    ) -> tuple[torch.Tensor, torch.Tensor]:
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

        unknown = set(df["from_account"]) | set(df["to_account"])
        unknown -= set(self.node_index.keys())
        if unknown:
            raise ValueError(f"unknown account number(s): {sorted(unknown)}")

        src = df["from_account"].map(self.node_index).to_numpy(dtype=np.int64)
        dst = df["to_account"].map(self.node_index).to_numpy(dtype=np.int64)
        edge_index = np.stack([src, dst], axis=0)

        a = df["amount"].astype(float).to_numpy()
        log_amt = np.log1p(a).astype(np.float32)
        diffs = np.abs(a[:, None] - ROUND_LEVELS[None, :])
        nearest = diffs.min(axis=1)
        is_round = (nearest < 1.0).astype(np.float32)
        log_dist = np.log1p(nearest).astype(np.float32)
        nearest_idx = diffs.argmin(axis=1)
        per_level = np.zeros((n, len(ROUND_LEVELS)), dtype=np.float32)
        is_close = nearest < 1.0
        per_level[is_close, nearest_idx[is_close]] = 1.0

        bp_oh = (
            pd.get_dummies(df["business_process"].fillna("UNK"), prefix="bp")
            .reindex(columns=[f"bp_{p}" for p in BUSINESS_PROCESSES], fill_value=0)
            .astype(np.float32)
            .to_numpy()
        )

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
    def predict_fraud(self, **kwargs) -> np.ndarray:
        target_edge_index, target_edge_attr = self.encode_edges(**kwargs)
        graph_edge_index = torch.from_numpy(self.edge_index)
        x = self.node_features_scaled

        self.fraud_model.train(False)
        h = self.fraud_model.encode(x, graph_edge_index)
        logits = self.fraud_model.edge_logits(h, target_edge_index, target_edge_attr)
        return torch.sigmoid(logits).cpu().numpy()

    @torch.no_grad()
    def anomaly_score_edges(self, **kwargs) -> np.ndarray:
        target_edge_index, target_edge_attr = self.encode_edges(**kwargs)
        graph_edge_index = torch.from_numpy(self.edge_index)
        x = self.node_features_scaled

        self.anomaly_model.train(False)
        recon = self.anomaly_model(x, graph_edge_index, target_edge_index)
        return ((recon - target_edge_attr) ** 2).mean(dim=-1).cpu().numpy()


def load_bundle(model_dir: Path | str) -> InferenceBundle:
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

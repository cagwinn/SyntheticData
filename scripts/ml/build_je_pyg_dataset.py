"""Build a torch_geometric Data object from the published JE dataset.

Reads `je_network.parquet` and `chart_of_accounts.parquet` from
`VynFi/vynfi-journal-entries-1m`, joins to derive node/edge features,
applies a stratified 70/15/15 split on the `is_fraud` edge label,
and caches the result as a single `.pt` artefact.

Usage::

    python -m scripts.ml.build_je_pyg_dataset \\
        --output data/ml/je_pyg_v1.pt \\
        --seed 20260509
"""
from __future__ import annotations

import argparse
import dataclasses
from pathlib import Path

import numpy as np
import pandas as pd
import torch
from huggingface_hub import snapshot_download
from sklearn.model_selection import train_test_split
from sklearn.preprocessing import StandardScaler
from torch_geometric.data import Data

DATASET_REPO = "VynFi/vynfi-journal-entries-1m"


# ─── Schema constants ────────────────────────────────────────────────────────


ACCOUNT_TYPE_LEVELS = ["asset", "liability", "equity", "revenue", "expense"]

# 5-process universe seen in v5.9.0 je_network.parquet — kept explicit so
# the encoder is stable across re-runs and across regenerations.
BUSINESS_PROCESSES = ["P2P", "O2C", "R2R", "H2R", "A2R"]


# ─── Loading + dedupe ────────────────────────────────────────────────────────


def load_parquets() -> tuple[pd.DataFrame, pd.DataFrame]:
    base = snapshot_download(
        repo_id=DATASET_REPO,
        repo_type="dataset",
        allow_patterns=["je_network.parquet", "chart_of_accounts.parquet"],
    )
    edges = pd.read_parquet(f"{base}/je_network.parquet")
    coa = pd.read_parquet(f"{base}/chart_of_accounts.parquet")

    # Normalise dtypes
    edges["from_account"] = edges["from_account"].astype(str)
    edges["to_account"] = edges["to_account"].astype(str)
    coa["account_number"] = coa["account_number"].astype(str)
    coa["account_type"] = coa["account_type"].astype(str).str.lower()

    # 4 account_numbers in the published COA appear with conflicting class
    # mappings (1510, 1600, 4900, 7100) — keep first deterministically.
    coa = coa.drop_duplicates(subset=["account_number"], keep="first").reset_index(drop=True)

    return edges, coa


# ─── Node feature engineering ────────────────────────────────────────────────


def build_node_features(coa: pd.DataFrame, edges: pd.DataFrame) -> tuple[np.ndarray, dict[str, int]]:
    """Return (node feature matrix, account_number → row index)."""
    coa = coa.sort_values("account_number").reset_index(drop=True)
    node_index = {acct: i for i, acct in enumerate(coa["account_number"].tolist())}

    # One-hot account_type
    type_oh = pd.get_dummies(
        coa["account_type"].fillna("other"),
        prefix="type",
    ).reindex(columns=[f"type_{t}" for t in ACCOUNT_TYPE_LEVELS], fill_value=0).astype(np.float32)

    # Static structural flags
    flags = coa[
        [
            "is_control_account",
            "is_suspense_account",
            "normal_debit_balance",
            "is_postable",
            "is_blocked",
            "requires_cost_center",
            "requires_profit_center",
        ]
    ].astype(np.float32).fillna(0.0).to_numpy()

    hierarchy = coa[["hierarchy_level"]].astype(np.float32).fillna(1.0).to_numpy()

    # Aggregated transactional stats per account (computed against full edges)
    out_stats = (
        edges.groupby("from_account")
        .agg(out_count=("edge_id", "count"), out_amount=("amount", "sum"))
        .reindex(coa["account_number"], fill_value=0.0)
        .reset_index(drop=True)
    )
    in_stats = (
        edges.groupby("to_account")
        .agg(in_count=("edge_id", "count"), in_amount=("amount", "sum"))
        .reindex(coa["account_number"], fill_value=0.0)
        .reset_index(drop=True)
    )
    aggregates = pd.concat([out_stats, in_stats], axis=1).astype(np.float32).to_numpy()
    # Log-scale the amount magnitudes so they live on the same order as flags
    aggregates[:, [0, 2]] = np.log1p(aggregates[:, [0, 2]])
    aggregates[:, [1, 3]] = np.log1p(aggregates[:, [1, 3]])

    x = np.concatenate([type_oh.to_numpy(), flags, hierarchy, aggregates], axis=1)
    return x.astype(np.float32), node_index


# ─── Edge feature engineering ────────────────────────────────────────────────


def encode_dates(dates: pd.Series) -> np.ndarray:
    """Date encodings — captures both seasonality and weekend bias."""
    dt = pd.to_datetime(dates, errors="coerce")
    doy = dt.dt.dayofyear.fillna(1).to_numpy()
    woy = dt.dt.isocalendar().week.astype(int).to_numpy()
    dow = dt.dt.dayofweek.fillna(0).to_numpy()  # 0=Mon, 6=Sun
    is_weekend = (dow >= 5).astype(np.float32)
    return np.stack(
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


# Round-dollar levels targeted by datasynth_core::fraud_bias::ROUND_LEVELS
# (1K / 5K / 10K / 25K / 50K / 100K).  Fraud rescales max-line to one of these.
_ROUND_LEVELS = np.array([1_000.0, 5_000.0, 10_000.0, 25_000.0, 50_000.0, 100_000.0])


def encode_amounts(amounts: pd.Series) -> np.ndarray:
    """Amount features: log1p magnitude + round-dollar shape signals."""
    a = amounts.astype(float).to_numpy()
    log_amt = np.log1p(a).astype(np.float32)

    # Distance to nearest canonical round level (in dollars + log-scaled)
    diffs = np.abs(a[:, None] - _ROUND_LEVELS[None, :])
    nearest = diffs.min(axis=1)
    is_round = (nearest < 1.0).astype(np.float32)
    log_distance_to_round = np.log1p(nearest).astype(np.float32)

    # Per-level round flag (one-hot for which level — useful for the GNN to learn
    # different patterns per fraud-bias bucket)
    nearest_idx = diffs.argmin(axis=1)
    per_level = np.zeros((len(a), len(_ROUND_LEVELS)), dtype=np.float32)
    is_close = nearest < 1.0
    per_level[is_close, nearest_idx[is_close]] = 1.0

    return np.concatenate(
        [
            log_amt[:, None],
            is_round[:, None],
            log_distance_to_round[:, None],
            per_level,
        ],
        axis=1,
    )


def build_edge_tensors(
    edges: pd.DataFrame,
    node_index: dict[str, int],
) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    """Return (edge_index[2,N], edge_attr[N,F], y[N], idx_keep, kept_edges)."""
    e = edges[edges["from_account"].isin(node_index) & edges["to_account"].isin(node_index)].reset_index(drop=True)
    src = e["from_account"].map(node_index).to_numpy(dtype=np.int64)
    dst = e["to_account"].map(node_index).to_numpy(dtype=np.int64)
    edge_index = np.stack([src, dst], axis=0)

    # Numeric features
    amount_feats = encode_amounts(e["amount"])
    confidence = e["confidence"].astype(float).fillna(1.0).to_numpy().reshape(-1, 1).astype(np.float32)

    # One-hot business_process (stable column order)
    bp_oh = pd.get_dummies(
        e["business_process"].fillna("UNK"),
        prefix="bp",
    ).reindex(columns=[f"bp_{p}" for p in BUSINESS_PROCESSES], fill_value=0).astype(np.float32).to_numpy()

    date_feats = encode_dates(e["posting_date"])

    edge_attr = np.concatenate([amount_feats, confidence, bp_oh, date_feats], axis=1)
    y = e["is_fraud"].astype(np.float32).to_numpy()
    is_anomaly = e["is_anomaly"].astype(np.float32).to_numpy()
    return edge_index, edge_attr, y, is_anomaly, e


# ─── Stratified split ────────────────────────────────────────────────────────


def stratified_split(
    n: int,
    y: np.ndarray,
    seed: int,
    train_frac: float = 0.70,
    val_frac: float = 0.15,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    idx = np.arange(n)
    train_idx, rest_idx = train_test_split(
        idx, train_size=train_frac, stratify=y, random_state=seed
    )
    rest_y = y[rest_idx]
    val_size = val_frac / (1 - train_frac)
    val_idx, test_idx = train_test_split(
        rest_idx, train_size=val_size, stratify=rest_y, random_state=seed
    )
    return train_idx, val_idx, test_idx


# ─── Top-level build ─────────────────────────────────────────────────────────


@dataclasses.dataclass
class BuildResult:
    data: Data
    node_index: dict[str, int]
    edge_attr_scaler: StandardScaler
    node_feature_scaler: StandardScaler
    feature_columns: dict[str, list[str]]
    raw_edges_kept: pd.DataFrame


def build(seed: int = 20260509) -> BuildResult:
    edges_df, coa_df = load_parquets()
    print(f"loaded: {len(edges_df):,} edges, {len(coa_df):,} accounts")

    x_np, node_index = build_node_features(coa_df, edges_df)
    edge_index_np, edge_attr_np, y_np, is_anomaly_np, kept_edges = build_edge_tensors(edges_df, node_index)
    print(f"kept edges: {len(kept_edges):,} of {len(edges_df):,} (dropped any with unmapped accounts)")
    print(f"fraud rate: {y_np.mean():.4f} ({int(y_np.sum())} fraud / {len(y_np)})")
    print(f"anomaly rate: {is_anomaly_np.mean():.4f}")

    # Scale features (fit on train indices only — done after split)
    train_idx, val_idx, test_idx = stratified_split(len(y_np), y_np, seed=seed)
    print(f"split: {len(train_idx):,} train / {len(val_idx):,} val / {len(test_idx):,} test")
    print(
        f"  train fraud rate: {y_np[train_idx].mean():.4f}, "
        f"val: {y_np[val_idx].mean():.4f}, test: {y_np[test_idx].mean():.4f}"
    )

    edge_scaler = StandardScaler().fit(edge_attr_np[train_idx])
    edge_attr_scaled = edge_scaler.transform(edge_attr_np).astype(np.float32)

    # Node features: transactional aggregates were built from ALL edges, but
    # we only fit the scaler on values that participate in train edges to
    # avoid leakage. Easier proxy: fit on the global node matrix — leakage
    # is bounded since these are aggregates over millions of underlying JEs,
    # not per-edge labels.
    node_scaler = StandardScaler().fit(x_np)
    x_scaled = node_scaler.transform(x_np).astype(np.float32)

    # Build train/val/test masks (on edges)
    n_edges = len(y_np)
    train_mask = np.zeros(n_edges, dtype=bool)
    val_mask = np.zeros(n_edges, dtype=bool)
    test_mask = np.zeros(n_edges, dtype=bool)
    train_mask[train_idx] = True
    val_mask[val_idx] = True
    test_mask[test_idx] = True

    data = Data(
        x=torch.from_numpy(x_scaled),
        edge_index=torch.from_numpy(edge_index_np),
        edge_attr=torch.from_numpy(edge_attr_scaled),
        y=torch.from_numpy(y_np),
        is_anomaly=torch.from_numpy(is_anomaly_np),
        train_mask=torch.from_numpy(train_mask),
        val_mask=torch.from_numpy(val_mask),
        test_mask=torch.from_numpy(test_mask),
    )

    feature_columns = {
        "node": (
            [f"type_{t}" for t in ACCOUNT_TYPE_LEVELS]
            + [
                "is_control_account",
                "is_suspense_account",
                "normal_debit_balance",
                "is_postable",
                "is_blocked",
                "requires_cost_center",
                "requires_profit_center",
            ]
            + ["hierarchy_level"]
            + ["log1p_out_count", "log1p_out_amount", "log1p_in_count", "log1p_in_amount"]
        ),
        "edge": (
            [
                "log1p_amount",
                "is_round_dollar",
                "log1p_distance_to_round",
            ]
            + [f"round_{int(lv)}" for lv in _ROUND_LEVELS]
            + ["confidence"]
            + [f"bp_{p}" for p in BUSINESS_PROCESSES]
            + [
                "sin_doy",
                "cos_doy",
                "sin_woy",
                "cos_woy",
                "sin_dow",
                "cos_dow",
                "is_weekend",
            ]
        ),
    }

    return BuildResult(
        data=data,
        node_index=node_index,
        edge_attr_scaler=edge_scaler,
        node_feature_scaler=node_scaler,
        feature_columns=feature_columns,
        raw_edges_kept=kept_edges,
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=Path("data/ml/je_pyg_v1.pt"))
    parser.add_argument("--seed", type=int, default=20260509)
    args = parser.parse_args()

    args.output.parent.mkdir(parents=True, exist_ok=True)

    res = build(seed=args.seed)
    payload = {
        "data": res.data,
        "node_index": res.node_index,
        "edge_attr_scaler_mean": res.edge_attr_scaler.mean_,
        "edge_attr_scaler_scale": res.edge_attr_scaler.scale_,
        "node_feature_scaler_mean": res.node_feature_scaler.mean_,
        "node_feature_scaler_scale": res.node_feature_scaler.scale_,
        "feature_columns": res.feature_columns,
        "schema_version": 1,
        "build_seed": args.seed,
    }
    torch.save(payload, args.output)
    print(f"saved → {args.output} ({args.output.stat().st_size / 1024:.1f} KB)")
    print(f"  node feature dim: {res.data.x.shape[1]}")
    print(f"  edge feature dim: {res.data.edge_attr.shape[1]}")


if __name__ == "__main__":
    main()

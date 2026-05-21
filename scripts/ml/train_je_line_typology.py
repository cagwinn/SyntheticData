"""Line-level fraud-*typology* classifier on the journal_entries table.

Companion to ``train_je_fraud_typology.py`` (which classifies typology from
the collapsed Method-A *edge list*). That edge view keeps only
amount/date/process/account-pair, and the 20 typologies turn out near-random
there. This script tests the hypothesis that the **richer line-level view**
— ``gl_account``, ``account_class``, ``cost_center``, ``is_post_close``,
``is_manual``, posting lag, round-dollar shape — makes ``fraud_type``
learnable.

Model: ``HistGradientBoostingClassifier`` (sklearn, native categorical
support) — a strong tabular baseline, no GPU needed. Trained on the fraud
lines only (``is_fraud == True``); reports accuracy / macro-F1 / weighted-F1
/ top-3 + per-typology P/R/support, so the lift over the edge-list GNN is
directly comparable.

Usage::

    python -m scripts.ml.train_je_line_typology --out models/ml/je_line_typology
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import pandas as pd
from huggingface_hub import snapshot_download
from sklearn.ensemble import HistGradientBoostingClassifier
from sklearn.metrics import (
    accuracy_score,
    classification_report,
    f1_score,
    top_k_accuracy_score,
)
from sklearn.model_selection import train_test_split

DATASET_REPO = "VynFi/vynfi-journal-entries-1m"

CATEGORICAL = [
    "business_process", "source", "document_type", "ledger", "currency",
    "account_class", "account_sub_class", "financial_statement_category",
    "gl_account", "cost_center", "profit_center",
]
_ROUND_LEVELS = np.array([1_000.0, 5_000.0, 10_000.0, 25_000.0, 50_000.0, 100_000.0])


def load_je() -> pd.DataFrame:
    base = snapshot_download(
        repo_id=DATASET_REPO, repo_type="dataset", allow_patterns=["data/*.parquet"]
    )
    shards = sorted(Path(base, "data").glob("*.parquet"))
    df = pd.concat((pd.read_parquet(s) for s in shards), ignore_index=True)
    print(f"loaded {len(df):,} JE lines from {len(shards)} shard(s)")
    return df


def engineer(df: pd.DataFrame) -> pd.DataFrame:
    out = pd.DataFrame(index=df.index)
    # Amounts — the posted side is whichever of debit/credit is non-zero.
    deb = pd.to_numeric(df["debit_amount"], errors="coerce").fillna(0.0)
    cred = pd.to_numeric(df["credit_amount"], errors="coerce").fillna(0.0)
    amt = np.where(deb != 0, deb, cred).astype(float)
    out["log_amount"] = np.log1p(np.abs(amt))
    out["is_debit"] = (deb != 0).astype(np.int8)
    if "local_amount" in df:
        out["log_local"] = np.log1p(np.abs(pd.to_numeric(df["local_amount"], errors="coerce").fillna(0.0)))
    # Round-dollar shape (mirrors fraud_bias ROUND_LEVELS).
    nearest = np.abs(amt[:, None] - _ROUND_LEVELS[None, :]).min(axis=1)
    out["is_round"] = (nearest < 1.0).astype(np.int8)
    out["log_dist_round"] = np.log1p(nearest)
    # Temporal
    pdt = pd.to_datetime(df["posting_date"], errors="coerce")
    out["dow"] = pdt.dt.dayofweek.fillna(0).astype(np.int8)
    out["is_weekend"] = (pdt.dt.dayofweek >= 5).fillna(False).astype(np.int8)
    out["month"] = pdt.dt.month.fillna(1).astype(np.int8)
    if "document_date" in df:
        ddt = pd.to_datetime(df["document_date"], errors="coerce")
        out["posting_lag_days"] = (pdt - ddt).dt.days.fillna(0).clip(-30, 365).astype(np.float32)
    # Behavioural flags
    for c in ("is_manual", "is_post_close"):
        if c in df:
            out[c] = df[c].astype("boolean").fillna(False).astype(np.int8)
    if "line_number" in df:
        out["line_number"] = pd.to_numeric(df["line_number"], errors="coerce").fillna(0).clip(0, 100).astype(np.int16)
    # Categoricals → pandas category dtype (HistGBM reads codes natively).
    for c in CATEGORICAL:
        if c in df:
            out[c] = df[c].astype(str).fillna("NA").astype("category")
    return out


def metrics(y_true, y_pred, y_proba, names) -> dict:
    labels = list(range(len(names)))
    m = {
        "accuracy": float(accuracy_score(y_true, y_pred)),
        "macro_f1": float(f1_score(y_true, y_pred, average="macro", labels=labels, zero_division=0)),
        "weighted_f1": float(f1_score(y_true, y_pred, average="weighted", labels=labels, zero_division=0)),
        "top3_accuracy": float(top_k_accuracy_score(y_true, y_proba, k=3, labels=labels)),
        "n": int(len(y_true)),
    }
    m["per_class"] = classification_report(
        y_true, y_pred, labels=labels, target_names=names, output_dict=True, zero_division=0
    )
    return m


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=Path("models/ml/je_line_typology"))
    ap.add_argument("--seed", type=int, default=20260521)
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)

    df = load_je()
    is_fraud = df["is_fraud"].astype("boolean").fillna(False).to_numpy()
    ft = df["fraud_type"].fillna("").astype(str)
    fraud = is_fraud & (ft != "")
    print(f"fraud lines with a typology: {int(fraud.sum()):,} / {len(df):,}")

    names = sorted(ft[fraud].unique())
    name_to_idx = {n: i for i, n in enumerate(names)}
    y = ft[fraud].map(name_to_idx).to_numpy()
    X = engineer(df[fraud].reset_index(drop=True))
    print(f"{len(names)} typologies; {X.shape[1]} features ({sum(str(X[c].dtype)=='category' for c in X.columns)} categorical)")

    Xtr, Xte, ytr, yte = train_test_split(X, y, test_size=0.2, stratify=y, random_state=args.seed)
    clf = HistGradientBoostingClassifier(
        max_iter=400, learning_rate=0.08, max_depth=None, l2_regularization=1.0,
        categorical_features="from_dtype", class_weight="balanced", random_state=args.seed,
    )
    clf.fit(Xtr, ytr)
    proba = clf.predict_proba(Xte)
    pred = proba.argmax(axis=1)
    res = metrics(yte, pred, proba, names)
    print(f"\n=== HistGBM line-level typology ===")
    print(f"  acc={res['accuracy']:.4f}  macro_f1={res['macro_f1']:.4f}  "
          f"weighted_f1={res['weighted_f1']:.4f}  top3={res['top3_accuracy']:.4f}  n={res['n']}")
    print("\n=== per-typology (test) ===")
    for n in names:
        r = res["per_class"].get(n, {})
        if r.get("support", 0):
            print(f"  {n:30s} P={r['precision']:.3f} R={r['recall']:.3f} F1={r['f1-score']:.3f} n={int(r['support'])}")

    (args.out / "metrics.json").write_text(json.dumps(res, indent=2))
    print(f"\nsaved metrics -> {args.out / 'metrics.json'}")


if __name__ == "__main__":
    main()

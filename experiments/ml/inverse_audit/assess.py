"""Assess per-JE scores vs is_fraud (primary; the per-JE fraud the residual approach
targets) and is_anomaly (secondary; relational — expected weak). PR-AUC/ROC + per
fraud_type + Isolation-Forest baseline (the structure-blind detector to beat)."""
from __future__ import annotations
import argparse, json
from pathlib import Path
import numpy as np, pandas as pd
from sklearn.metrics import average_precision_score, roc_auc_score
from sklearn.ensemble import IsolationForest


def _p_at(y, s, frac):
    k = max(1, int(frac * len(y)))
    return float(y[np.argsort(-s)][:k].mean())


def _metrics(y, s):
    if y.sum() == 0:
        return {"n_pos": 0}
    return {"n_pos": int(y.sum()), "pr_auc": float(average_precision_score(y, s)),
            "roc_auc": float(roc_auc_score(y, s)), "p_at_1pct": _p_at(y, s, 0.01),
            "p_at_5pct": _p_at(y, s, 0.05)}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--scores", type=Path, required=True)
    ap.add_argument("--test-gl", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args(argv)
    d = pd.read_parquet(a.scores)
    s = d["score"].fillna(d["score"].min()).to_numpy()
    yf = d["is_fraud"].astype(int).to_numpy()
    res = {"n": int(len(d)), "fraud_base_rate": float(yf.mean()),
           "density_fraud": _metrics(yf, s),
           "density_anomaly": _metrics(d["is_anomaly"].astype(int).to_numpy(), s)}
    # per fraud_type, measured type-vs-NORMAL (exclude other fraud types, which
    # also score high under the global scorer and would otherwise count as FPs).
    bt = {}
    normal = ~d["is_fraud"].astype(bool)
    for t in d["fraud_type"].dropna().unique():
        is_t = (d["fraud_type"] == t)
        if int(is_t.sum()) < 3:
            continue
        mask = (is_t | normal).to_numpy()
        bt[t] = {"n": int(is_t.sum()),
                 "pr_auc": float(average_precision_score(is_t[mask].astype(int).to_numpy(), s[mask])),
                 "roc_auc": float(roc_auc_score(is_t[mask].astype(int).to_numpy(), s[mask]))}
    res["fraud_by_type"] = dict(sorted(bt.items(), key=lambda kv: -kv[1]["pr_auc"]))
    # IF baseline on engineered per-JE features
    raw = pd.read_csv(a.test_gl / "journal_entries.csv", low_memory=False)
    raw["amt"] = pd.to_numeric(raw.get("debit_amount", 0), errors="coerce").fillna(0.0)
    g = raw.groupby("document_id")
    fx = pd.DataFrame({"n_lines": g.size(), "total": g["amt"].sum(),
                       "n_acct": g["gl_account"].nunique()}).reindex(d["document_id"]).fillna(0.0)
    if_s = -IsolationForest(random_state=0, n_estimators=200).fit(fx).score_samples(fx)
    res["if_fraud"] = _metrics(yf, if_s)
    res["density_beats_if"] = (res["density_fraud"].get("pr_auc", 0) > res["if_fraud"].get("pr_auc", 0))
    a.out.write_text(json.dumps(res, indent=2)); print(json.dumps(res, indent=2)); print("ASSESS_DONE")


if __name__ == "__main__":
    main()

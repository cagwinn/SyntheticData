"""Assess per-JE scores vs is_anomaly: PR-AUC / ROC-AUC overall + per anomaly_type;
Isolation-Forest baseline on engineered per-JE features (the structure-blind detector
the structure-aware density should beat — especially on structural anomaly types)."""
from __future__ import annotations
import argparse, json
from pathlib import Path
import numpy as np, pandas as pd
from sklearn.metrics import average_precision_score, roc_auc_score
from sklearn.ensemble import IsolationForest


def _prec_at(y, s, frac):
    k = max(1, int(frac * len(y)))
    return float(y[np.argsort(-s)][:k].mean())


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--scores", type=Path, required=True)
    ap.add_argument("--test-gl", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args(argv)

    d = pd.read_parquet(a.scores)
    y = d["is_anomaly"].astype(int).to_numpy()
    s = d["score"].fillna(d["score"].min()).to_numpy()
    res = {"n": int(len(y)), "base_rate": float(y.mean()),
           "density": {"pr_auc": float(average_precision_score(y, s)),
                       "roc_auc": float(roc_auc_score(y, s)),
                       "p_at_1pct": _prec_at(y, s, 0.01),
                       "p_at_5pct": _prec_at(y, s, 0.05)}}
    # per anomaly_type (one-vs-rest using the same score)
    bt = {}
    for t in d["anomaly_type"].dropna().unique():
        yt = (d["anomaly_type"] == t).astype(int).to_numpy()
        if yt.sum() >= 3:
            bt[t] = {"n": int(yt.sum()), "pr_auc": float(average_precision_score(yt, s))}
    res["by_type"] = dict(sorted(bt.items(), key=lambda kv: -kv[1]["pr_auc"]))

    # Isolation-Forest baseline on engineered per-JE features
    raw = pd.read_csv(a.test_gl / "journal_entries.csv", low_memory=False)
    raw["amt"] = pd.to_numeric(raw.get("debit_amount", 0), errors="coerce").fillna(0.0)
    g = raw.groupby("document_id")
    fx = pd.DataFrame({"n_lines": g.size(), "total": g["amt"].sum(),
                       "n_acct": g["gl_account"].nunique()})
    fx = fx.reindex(d["document_id"]).fillna(0.0)
    if_s = -IsolationForest(random_state=0, n_estimators=200).fit(fx).score_samples(fx)
    res["isolation_forest"] = {"pr_auc": float(average_precision_score(y, if_s)),
                               "roc_auc": float(roc_auc_score(y, if_s))}
    res["density_beats_if"] = res["density"]["pr_auc"] > res["isolation_forest"]["pr_auc"]
    a.out.write_text(json.dumps(res, indent=2))
    print(json.dumps(res, indent=2))
    print("ASSESS_DONE")


if __name__ == "__main__":
    main()

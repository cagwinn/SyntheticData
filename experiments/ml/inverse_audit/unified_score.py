"""I6 — Unified routed detector. The capstone thesis: each anomaly subsystem is observed
by the detector aligned to it; the union catches more than any single arm.

  density arm     : per-JE NLL residual (score.py)   -> per-JE fraud (is_fraud)
  relational arm  : graph manifold residual          -> graph families  (is_anomaly)
  unified         : z-sum of the two (deployable, no labels at score time)

Runs both arms on a MIXED GL (generate_mixed.py: fraud + anomaly_injection both on),
joins per JE, then reports each arm vs each label and vs their union. The diagonal
(density vs is_fraud, relational vs is_anomaly) confirms specialisation; the off-diagonal
shows blindness; the unified column shows the routing payoff.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

import numpy as np
import pandas as pd

_EPS = 1e-9


def _sh(mod: str, *args: str) -> None:
    subprocess.run([sys.executable, "-m", mod, *args], check=True)


def _z(x: np.ndarray) -> np.ndarray:
    m = float(np.nanmedian(x))
    s = max(float(np.nanmedian(np.abs(x - m))) * 1.4826, _EPS)
    return (x - m) / s


def _metrics(y_true: np.ndarray, score: np.ndarray) -> dict:
    from sklearn.metrics import average_precision_score, roc_auc_score
    y = y_true.astype(int)
    if y.sum() == 0 or y.sum() == len(y):
        return {"n_pos": int(y.sum()), "pr_auc": None, "roc_auc": None}
    return {"n_pos": int(y.sum()),
            "pr_auc": float(average_precision_score(y, score)),
            "roc_auc": float(roc_auc_score(y, score))}


def main(argv=None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, required=True,
                    help="dir containing normal/ and test/ (mixed GL)")
    ap.add_argument("--out", type=Path, required=True, help="metrics JSON")
    ap.add_argument("--skip-scoring", action="store_true",
                    help="reuse existing density/graph parquet under --root")
    a = ap.parse_args(argv)
    R = a.root
    dpath = R / "density_scores.parquet"
    gpath = R / "graph_scores.parquet"

    if not a.skip_scoring:
        _sh("inverse_audit.score", "--normal", str(R / "normal"),
            "--test", str(R / "test"), "--out", str(dpath))
        _sh("inverse_audit.relational.graph_scorer",
            "--normal", str(R / "normal"), "--test", str(R / "test"), "--out", str(gpath))

    d = pd.read_parquet(dpath)
    g = pd.read_parquet(gpath)

    # density score is per-JE; column "document_id" is its key (it was reset_index in score.py)
    if "document_id" in d.columns:
        d = d.rename(columns={"document_id": "je_id"})
    elif d.index.name == "document_id":
        d = d.reset_index().rename(columns={"document_id": "je_id"})
    d["je_id"] = d["je_id"].astype(str)
    g["je_id"] = g["je_id"].astype(str)

    keep_d = ["je_id", "score", "is_fraud", "is_anomaly", "fraud_type", "anomaly_type"]
    keep_d = [c for c in keep_d if c in d.columns]
    keep_g = ["je_id", "relational_score"]
    j = d[keep_d].merge(g[keep_g], on="je_id", how="inner")
    j["density_z"] = _z(j["score"].to_numpy())
    j["relational_z"] = _z(j["relational_score"].to_numpy())
    j["unified_score"] = j[["density_z", "relational_z"]].fillna(0).sum(axis=1)

    is_f = j.get("is_fraud", pd.Series(False, index=j.index)).fillna(False).astype(bool).to_numpy()
    is_a = j.get("is_anomaly", pd.Series(False, index=j.index)).fillna(False).astype(bool).to_numpy()
    is_any = (is_f | is_a)

    result: dict = {}
    targets = {"vs_is_fraud": is_f, "vs_is_anomaly": is_a, "vs_is_any": is_any}
    arms = {"density": j["score"].to_numpy(),
            "relational": j["relational_score"].to_numpy(),
            "unified": j["unified_score"].to_numpy()}
    for tgt, y in targets.items():
        result[tgt] = {arm: _metrics(y, s) for arm, s in arms.items()}
    a.out.write_text(json.dumps(result, indent=2))
    print("UNIFIED_DONE")
    for tgt, scores in result.items():
        n = scores["density"]["n_pos"]
        print(f"  {tgt} (n_pos={n}):")
        for arm, m in scores.items():
            pa = "—" if m["pr_auc"] is None else f"{m['pr_auc']:.3f}"
            rc = "—" if m["roc_auc"] is None else f"{m['roc_auc']:.3f}"
            print(f"    {arm:11s} pr_auc={pa} roc={rc}")


if __name__ == "__main__":
    main()

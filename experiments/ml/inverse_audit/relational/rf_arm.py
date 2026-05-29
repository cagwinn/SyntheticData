"""Tier-B supervised relational arm + hybrid — the plateau-breaker.

The unsupervised z-sum (graph_scorer) plateaus on the hard relational families
(NewCounterparty, MissingRelationship, UnusualAccountPair, Circular*, TransferPricing
all ~0.5 ROC). Hand-engineered features (centrality-delta, counterparty tri-novelty)
were null. But a NONLINEAR SUPERVISED model (RandomForest) over the EXISTING relational
features — trainable because DataSynth provides the labels — lifts every hard family and
TRANSFERS across GLs. The deployable detector is the HYBRID: rank-z-sum of the
unsupervised residual (label-free; nails dormancy/edge-rarity) and the RF (the
discriminative arm; the hard families). This is the §12 routing thesis extended with a
supervised arm, and it sidesteps the A1 SBI-OOD problem because the features are
z-normalized per-GL (fit-on-self) so they generalize.

Train on one scored GL, evaluate transfer on another:

    python -m inverse_audit.relational.rf_arm \
        --train runs/B/iar/graph_scores.parquet \
        --test  runs/B/iar23/graph_scores.parquet \
        --out   runs/B/rf_arm_transfer.json [--model runs/B/rf_arm.joblib]
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import pandas as pd
from scipy.stats import rankdata
from sklearn.ensemble import RandomForestClassifier
from sklearn.metrics import average_precision_score, roc_auc_score

_HARD = ("DormantAccountActivity", "CentralityAnomaly", "NewCounterparty",
         "MissingRelationship", "UnusualAccountPair", "CircularTransaction",
         "TransferPricingAnomaly")


def _rank_z(x: np.ndarray) -> np.ndarray:
    r = rankdata(x)
    return (r - r.mean()) / (r.std() + 1e-9)


def _y(df: pd.DataFrame) -> np.ndarray:
    return df["is_anomaly_je"].fillna(False).astype(int).to_numpy()


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--train", type=Path, required=True, help="scored GL parquet (train)")
    ap.add_argument("--test", type=Path, required=True, help="scored GL parquet (transfer eval)")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--model", type=Path, default=None, help="optional joblib of the trained RF")
    ap.add_argument("--n-estimators", type=int, default=400)
    a = ap.parse_args(argv)

    tr, te = pd.read_parquet(a.train), pd.read_parquet(a.test)
    zc = [c for c in tr.columns if c.endswith("_z") and c in te.columns]
    Xtr, ytr = tr[zc].fillna(0).to_numpy(), _y(tr)
    Xte, yte = te[zc].fillna(0).to_numpy(), _y(te)

    rf = RandomForestClassifier(n_estimators=a.n_estimators, class_weight="balanced_subsample",
                                n_jobs=-1, random_state=0).fit(Xtr, ytr)
    p = rf.predict_proba(Xte)[:, 1]
    us = te["relational_score"].to_numpy() if "relational_score" in te else np.zeros(len(te))
    hyb = _rank_z(p) + _rank_z(us)

    def m(s):
        return {"pr_auc": round(float(average_precision_score(yte, s)), 4),
                "roc_auc": round(float(roc_auc_score(yte, s)), 4)}

    out = {"features": zc, "n_train_pos": int(ytr.sum()), "n_test_pos": int(yte.sum()),
           "overall": {"unsup": m(us), "rf": m(p), "hybrid": m(hyb)},
           "rf_importances": dict(sorted(zip(zc, (round(float(v), 4) for v in rf.feature_importances_)),
                                         key=lambda t: -t[1])),
           "per_family": {}}
    at, nrm = te.get("anomaly_type"), ~yte.astype(bool)
    if at is not None:
        for fam in _HARD:
            isf = (at == fam).fillna(False).to_numpy()
            if isf.sum() < 5:
                continue
            mask = isf | nrm
            yy = isf[mask].astype(int)
            out["per_family"][fam] = {
                "n": int(isf.sum()),
                "unsup": round(float(roc_auc_score(yy, us[mask])), 3),
                "rf": round(float(roc_auc_score(yy, p[mask])), 3),
                "hybrid": round(float(roc_auc_score(yy, hyb[mask])), 3)}

    a.out.write_text(json.dumps(out, indent=2))
    if a.model:
        import joblib
        joblib.dump({"rf": rf, "features": zc}, a.model)
    o = out["overall"]
    print(f"RF_ARM transfer {a.train.parent.name}->{a.test.parent.name}: "
          f"unsup {o['unsup']['pr_auc']}/{o['unsup']['roc_auc']}  "
          f"rf {o['rf']['pr_auc']}/{o['rf']['roc_auc']}  "
          f"HYBRID {o['hybrid']['pr_auc']}/{o['hybrid']['roc_auc']}  -> {a.out}")


if __name__ == "__main__":
    main()

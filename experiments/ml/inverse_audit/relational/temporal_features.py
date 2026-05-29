"""Tier C-2 — first cross-JE temporal observable for the inverse-audit detector.

The per-JE relational residual (and the Tier-B RF over it) are blind to CROSS-JE temporal
structure — an account's activity *trajectory* over G(t). This adds per-JE temporal features:
for each JE, the max over its touched accounts of that account's weekly-activity
  - burst z-score  : (count in the JE's week − account's robust weekly baseline) / MAD
  - trend          : (recent-window mean − earlier-window mean) / MAD  (level shift)
Targets the temporal families (TransactionBurst, UnusualFrequency, TrendBreak, UnusualTiming)
the structural arms don't model — a complement, not a replacement. This is the tractable first
rung of the G(t) state-space direction; a full grey-box Kalman/CUSUM system is the follow-on.

    python -m inverse_audit.relational.temporal_features \
        --gl runs/B/iar/test --scores runs/B/iar/graph_scores_improved.parquet --eval
"""
from __future__ import annotations

import argparse
from pathlib import Path

import numpy as np
import pandas as pd


def temporal_features(df: pd.DataFrame) -> pd.DataFrame:
    """Per-JE temporal features from per-account weekly activity over the JE stream."""
    d = df.copy()
    d["posting_date"] = pd.to_datetime(d["posting_date"], errors="coerce", dayfirst=True)
    d["gl_account"] = d["gl_account"].astype(str)
    d = d.dropna(subset=["posting_date"])
    d["week"] = d["posting_date"].dt.to_period("W").astype(str)
    weeks_sorted = sorted(d["week"].unique())
    wk_idx = {w: i for i, w in enumerate(weeks_sorted)}
    nweeks = len(weeks_sorted)

    # per-account weekly activity count (distinct JEs touching the account that week)
    aw = (d.groupby(["gl_account", "week"])["document_id"].nunique()
            .reset_index(name="cnt"))
    # robust baseline (median + MAD) over the account's active weeks
    base = aw.groupby("gl_account")["cnt"].agg(
        med="median", mad=lambda s: (np.median(np.abs(s - np.median(s))) * 1.4826) or 1.0)
    aw = aw.merge(base, on="gl_account")
    aw["burst"] = (aw["cnt"] - aw["med"]) / aw["mad"].clip(lower=1e-9)
    burst_map = {(r.gl_account, r.week): float(r.burst) for r in aw.itertuples()}

    # per-account trend: (mean of latter-half weeks) − (mean of earlier-half), in MAD units
    trend_map: dict[str, float] = {}
    for acct, g in aw.groupby("gl_account"):
        if len(g) < 4:
            continue
        order = g.assign(wi=g["week"].map(wk_idx)).sort_values("wi")
        half = len(order) // 2
        early, late = order["cnt"].iloc[:half].mean(), order["cnt"].iloc[half:].mean()
        mad = order["mad"].iloc[0]
        trend_map[acct] = float((late - early) / max(mad, 1e-9))

    # per-JE: week + touched accounts → max burst, max |trend|
    je_week = d.groupby("document_id")["week"].first().to_dict()
    je_accts = d.groupby("document_id")["gl_account"].apply(set).to_dict()
    rows = []
    for je, accts in je_accts.items():
        w = je_week.get(je)
        b = max((burst_map.get((a, w), 0.0) for a in accts), default=0.0)
        t = max((abs(trend_map.get(a, 0.0)) for a in accts), default=0.0)
        rows.append({"je_id": str(je), "acct_burst_max": b, "acct_trend_max": t})
    out = pd.DataFrame(rows).set_index("je_id")
    return out


_TEMPORAL_FAMS = ("TransactionBurst", "UnusualFrequency", "TrendBreak", "UnusualTiming",
                  "CentralityAnomaly", "CircularTransaction")


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--gl", type=Path, required=True)
    ap.add_argument("--scores", type=Path, required=True, help="graph_scores parquet (has labels + _z)")
    ap.add_argument("--eval", action="store_true", help="RF-CV with vs without temporal features")
    ap.add_argument("--out", type=Path, default=None)
    a = ap.parse_args(argv)

    df = pd.read_csv(a.gl / "journal_entries.csv", low_memory=False)
    tf = temporal_features(df)
    print(f"temporal features for {len(tf)} JEs; "
          f"acct_burst_max p99={np.percentile(tf['acct_burst_max'], 99):.2f} "
          f"max={tf['acct_burst_max'].max():.2f}")

    sc = pd.read_parquet(a.scores)
    sc["je_id"] = sc["je_id"].astype(str) if "je_id" in sc.columns else sc.index.astype(str)
    sc = sc.set_index("je_id") if "je_id" in sc.columns else sc
    j = sc.join(tf, how="left").fillna({"acct_burst_max": 0.0, "acct_trend_max": 0.0})
    if a.out:
        j.reset_index().to_parquet(a.out)

    if a.eval:
        from sklearn.ensemble import RandomForestClassifier
        from sklearn.model_selection import cross_val_predict
        from sklearn.metrics import average_precision_score, roc_auc_score
        y = j["is_anomaly_je"].fillna(False).astype(int).to_numpy()
        zc = [c for c in j.columns if c.endswith("_z")]
        tcols = ["acct_burst_max", "acct_trend_max"]
        def rfcv(cols):
            X = j[cols].fillna(0).to_numpy()
            p = cross_val_predict(RandomForestClassifier(n_estimators=400, class_weight="balanced_subsample",
                                  n_jobs=-1, random_state=0), X, y, cv=5, method="predict_proba")[:, 1]
            return p
        p_base, p_temp = rfcv(zc), rfcv(zc + tcols)
        at = j["anomaly_type"]; nrm = ~y.astype(bool)
        def per(s, fam):
            isf = (at == fam).fillna(False).to_numpy()
            if isf.sum() < 5: return None
            m = isf | nrm
            return round(roc_auc_score(isf[m].astype(int), s[m]), 3)
        print(f"RF-CV overall:  base {average_precision_score(y,p_base):.4f}/{roc_auc_score(y,p_base):.3f}"
              f"   +temporal {average_precision_score(y,p_temp):.4f}/{roc_auc_score(y,p_temp):.3f}")
        print("per-family ROC (base / +temporal):")
        for fam in _TEMPORAL_FAMS:
            b, t = per(p_base, fam), per(p_temp, fam)
            if b is not None:
                print(f"  {fam:24s} {b:.3f} / {t:.3f}")


if __name__ == "__main__":
    main()

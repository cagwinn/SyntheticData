"""Research — reconstruct CoA account TYPE from the JE cube, WITHOUT the CoA mapping.

Given only JE lines (account numbers + debit/credit amounts + posting dates), infer each
account's type (asset / liability / equity / revenue / expense) from behaviour:
  - flow nature   : debit_frac, net_frac  (debit-nature {asset,expense} vs credit-nature)
  - balance-sheet vs P&L : temporal profile — P&L accounts (revenue/expense) are operational
    (active most months, lower lumpiness) and get closed at year-end; BS accounts (asset/
    liability/equity) persist. Features: months-active fraction, monthly activity CV,
    last-period share. This is the axis that splits asset↔expense and revenue↔equity↔liability.
Validate against the account-number first-digit convention (the synthetic CoA's ground-truth
type): 1=asset 2=liability 3=equity 4=revenue 5/6=expense 7=revenue/other. Two read-outs:
unsupervised KMeans (purity/ARI) + a supervised RF-CV ceiling (how much type-signal the cube
carries). No CoA file used at inference.

    python -m inverse_audit.coa_reconstruct --gl runs/B/iar/test --out runs/research/coa.json
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import pandas as pd

_DIGIT_TYPE = {"1": "asset", "2": "liability", "3": "equity",
               "4": "revenue", "5": "expense", "6": "expense", "7": "revenue"}
_DEBIT_NATURE = {"asset", "expense"}
_FEATURES = ["debit_frac", "net_frac", "log_act", "months_active_frac", "monthly_cv",
             "last_period_share", "bal_persistence", "bal_to_flow", "runbal_signchg"]


def account_features(df: pd.DataFrame) -> pd.DataFrame:
    d = df.copy()
    d["debit_amount"] = pd.to_numeric(d["debit_amount"], errors="coerce").fillna(0.0)
    d["credit_amount"] = pd.to_numeric(d["credit_amount"], errors="coerce").fillna(0.0)
    d["gl_account"] = d["gl_account"].astype(str)
    d["amt"] = d["debit_amount"] + d["credit_amount"]
    g = d.groupby("gl_account")
    f = pd.DataFrame({"tot_deb": g["debit_amount"].sum(), "tot_cred": g["credit_amount"].sum(),
                      "n_lines": g.size()})
    tot = f["tot_deb"] + f["tot_cred"] + 1e-9
    f["debit_frac"] = f["tot_deb"] / tot
    f["net_frac"] = (f["tot_deb"] - f["tot_cred"]) / tot
    f["log_act"] = np.log1p(f["tot_deb"] + f["tot_cred"])

    # Temporal profile (balance-sheet vs P&L axis).
    d["month"] = pd.to_datetime(d["posting_date"], errors="coerce", dayfirst=True).dt.to_period("M").astype(str)
    d = d[d["month"] != "NaT"]
    n_months = max(d["month"].nunique(), 1)
    am = d.groupby(["gl_account", "month"])["amt"].sum().reset_index()
    prof = am.groupby("gl_account")["amt"].agg(
        n_mon="count", mean="mean", std="std").fillna(0.0)
    prof["months_active_frac"] = prof["n_mon"] / n_months
    prof["monthly_cv"] = prof["std"] / (prof["mean"].abs() + 1e-9)
    last_month = sorted(d["month"].unique())[-1]
    last_amt = am[am["month"] == last_month].set_index("gl_account")["amt"]
    tot_amt = am.groupby("gl_account")["amt"].sum()
    prof["last_period_share"] = (last_amt / (tot_amt + 1e-9)).reindex(prof.index).fillna(0.0)
    f = f.join(prof[["months_active_frac", "monthly_cv", "last_period_share"]]).fillna(0.0)

    # Running-balance dynamics (the balance-sheet vs P&L axis). BS accounts persist a
    # cumulative balance; P&L accounts net-accumulate then close toward ~0 at year-end.
    d["dt"] = pd.to_datetime(d["posting_date"], errors="coerce", dayfirst=True)
    ds = d.dropna(subset=["dt"]).sort_values("dt").copy()
    ds["signed"] = ds["debit_amount"] - ds["credit_amount"]
    ds["runbal"] = ds.groupby("gl_account")["signed"].cumsum()
    gb = ds.groupby("gl_account")["runbal"]
    final_bal = gb.last()
    max_abs = gb.apply(lambda s: float(np.abs(s.to_numpy()).max()))
    signchg = gb.apply(lambda s: int((np.sign(s.to_numpy()[1:]) != np.sign(s.to_numpy()[:-1])).sum())
                       if len(s) > 1 else 0)
    bal = pd.DataFrame({"final_bal": final_bal, "max_abs": max_abs, "signchg": signchg})
    f["bal_persistence"] = (bal["final_bal"].abs() / (bal["max_abs"] + 1e-9)).reindex(f.index).fillna(0.0)
    f["bal_to_flow"] = (bal["final_bal"].abs()).reindex(f.index).fillna(0.0) / (f["tot_deb"] + f["tot_cred"] + 1e-9)
    f["runbal_signchg"] = (bal["signchg"].reindex(f.index).fillna(0.0)) / f["n_lines"].clip(lower=1)
    return f


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--gl", type=Path, required=True)
    ap.add_argument("--out", type=Path, default=None)
    a = ap.parse_args(argv)

    df = pd.read_csv(a.gl / "journal_entries.csv", low_memory=False)
    f = account_features(df)
    f["truth"] = [_DIGIT_TYPE.get(str(x)[0], "other") for x in f.index]
    f = f[f["truth"] != "other"].copy()
    if f.empty:
        print("no typeable accounts"); return

    # (1) debit/credit-nature.
    f["pred_nature"] = np.where(f["debit_frac"] > 0.5, "debit", "credit")
    f["truth_nature"] = np.where(f["truth"].isin(_DEBIT_NATURE), "debit", "credit")
    nature_acc = float((f["pred_nature"] == f["truth_nature"]).mean())

    X = f[_FEATURES].to_numpy()
    X = (X - X.mean(0)) / (X.std(0) + 1e-9)
    y = f["truth"].to_numpy()
    k = int(f["truth"].nunique())

    # (2) unsupervised KMeans → majority-map → purity + ARI.
    from sklearn.cluster import KMeans
    from sklearn.metrics import adjusted_rand_score
    km = KMeans(n_clusters=k, n_init=10, random_state=0).fit(X)
    f["cluster"] = km.labels_
    cl2t = f.groupby("cluster")["truth"].agg(lambda s: s.value_counts().index[0])
    f["pred_unsup"] = f["cluster"].map(cl2t)
    purity = float((f["pred_unsup"] == f["truth"]).mean())
    ari = float(adjusted_rand_score(y, km.labels_))

    # (3) supervised RF-CV ceiling — how much type-signal the cube carries.
    from sklearn.ensemble import RandomForestClassifier
    from sklearn.model_selection import cross_val_predict
    sup_pred = cross_val_predict(
        RandomForestClassifier(n_estimators=400, class_weight="balanced_subsample",
                               n_jobs=-1, random_state=0), X, y, cv=5)
    sup_acc = float((sup_pred == y).mean())

    print(f"accounts={len(f)}  types={k}")
    print(f"debit/credit-nature accuracy: {nature_acc:.3f}  (baseline 0.5)")
    print(f"unsupervised KMeans purity:   {purity:.3f}   ARI: {ari:.3f}")
    print(f"supervised RF-CV ceiling acc: {sup_acc:.3f}   (cube's type-signal upper bound)")
    print("per-type recall (unsup / supervised):")
    rec = {}
    for t in sorted(f["truth"].unique()):
        m = f["truth"] == t
        ru = float((f.loc[m, "pred_unsup"] == t).mean())
        rs = float((sup_pred[m.to_numpy()] == t).mean())
        rec[t] = {"n": int(m.sum()), "recall_unsup": round(ru, 3), "recall_sup": round(rs, 3)}
        print(f"  {t:10s} n={int(m.sum()):4d}  {ru:.3f} / {rs:.3f}")
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(json.dumps({"n_accounts": int(len(f)), "n_types": k,
                                     "nature_acc": round(nature_acc, 4),
                                     "unsup_purity": round(purity, 4), "ari": round(ari, 4),
                                     "supervised_ceiling_acc": round(sup_acc, 4),
                                     "per_type": rec, "features": _FEATURES}, indent=2))


if __name__ == "__main__":
    main()

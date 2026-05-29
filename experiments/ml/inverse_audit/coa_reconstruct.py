"""Research — reconstruct CoA account TYPE from the JE cube, WITHOUT the CoA mapping.

Given only JE lines (account numbers + debit/credit amounts), infer each account's type
(asset / liability / equity / revenue / expense) from flow behaviour — debit/credit nature,
net-balance sign, activity. Validate against the account-number first-digit convention (the
synthetic CoA's ground-truth type): 1=asset 2=liability 3=equity 4=revenue 5/6=expense
7=revenue/other. Unsupervised structure inference from the cube; no CoA file used at inference.

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
_DEBIT_NATURE = {"asset", "expense"}  # normal debit-balance types


def account_features(df: pd.DataFrame) -> pd.DataFrame:
    d = df.copy()
    d["debit_amount"] = pd.to_numeric(d["debit_amount"], errors="coerce").fillna(0.0)
    d["credit_amount"] = pd.to_numeric(d["credit_amount"], errors="coerce").fillna(0.0)
    d["gl_account"] = d["gl_account"].astype(str)
    g = d.groupby("gl_account")
    f = pd.DataFrame({"tot_deb": g["debit_amount"].sum(), "tot_cred": g["credit_amount"].sum(),
                      "n_lines": g.size()})
    tot = f["tot_deb"] + f["tot_cred"] + 1e-9
    f["debit_frac"] = f["tot_deb"] / tot
    f["net_frac"] = (f["tot_deb"] - f["tot_cred"]) / tot   # +1 debit-nature, -1 credit-nature
    f["log_act"] = np.log1p(f["tot_deb"] + f["tot_cred"])
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

    # (1) debit/credit-nature (the cleanest cube signal): debit_frac>0.5 ⇒ debit-nature.
    f["pred_nature"] = np.where(f["debit_frac"] > 0.5, "debit", "credit")
    f["truth_nature"] = np.where(f["truth"].isin(_DEBIT_NATURE), "debit", "credit")
    nature_acc = float((f["pred_nature"] == f["truth_nature"]).mean())

    # (2) 5-class unsupervised: KMeans on flow features → majority-map clusters → purity + ARI.
    from sklearn.cluster import KMeans
    from sklearn.metrics import adjusted_rand_score
    X = f[["debit_frac", "net_frac", "log_act"]].to_numpy()
    X = (X - X.mean(0)) / (X.std(0) + 1e-9)
    k = int(f["truth"].nunique())
    km = KMeans(n_clusters=k, n_init=10, random_state=0).fit(X)
    f["cluster"] = km.labels_
    cl2t = f.groupby("cluster")["truth"].agg(lambda s: s.value_counts().index[0])
    f["pred_type"] = f["cluster"].map(cl2t)
    type_acc = float((f["pred_type"] == f["truth"]).mean())
    ari = float(adjusted_rand_score(f["truth"], f["cluster"]))

    print(f"accounts={len(f)}  types={k}")
    print(f"debit/credit-nature accuracy: {nature_acc:.3f}  (baseline 0.5)")
    print(f"5-class cluster→type purity:  {type_acc:.3f}   ARI: {ari:.3f}")
    print("per-type recall:")
    for t in sorted(f["truth"].unique()):
        m = f["truth"] == t
        print(f"  {t:10s} n={int(m.sum()):4d} recall={float((f.loc[m,'pred_type']==t).mean()):.3f}")
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(json.dumps({"n_accounts": int(len(f)), "n_types": k,
                                     "nature_acc": round(nature_acc, 4),
                                     "type_purity": round(type_acc, 4), "ari": round(ari, 4)}, indent=2))


if __name__ == "__main__":
    main()

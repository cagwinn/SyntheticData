"""#8 deepening — CoA account-TYPE from relational flow-graph NEIGHBOURS (account2vec).

The aggregate flow/balance features ceiling at ~0.40 for fine 5-class type (FINDINGS §24).
This tests the relational lever: infer an account's type from WHICH accounts it co-occurs with in
JEs (revenue pairs with AR, expense with AP/cash, …) rather than its own aggregate stats. Build
the account co-occurrence matrix (accounts in the same JE), log-weight + TruncatedSVD → a dense
neighbour embedding per account, then supervised RF-CV classify type. Compared to the §24
aggregate-feature accuracy and the combination. Truth = account-number first digit. No CoA file.

    python -m inverse_audit.coa_neighbour --gl runs/B/iar/test --out runs/research/coa_nb.json
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import pandas as pd

from inverse_audit.coa_reconstruct import _DIGIT_TYPE, _FEATURES, account_features


def neighbour_embedding(df: pd.DataFrame, k: int = 24, max_je_accts: int = 30) -> pd.DataFrame:
    """Per-account dense embedding from the JE co-occurrence graph (log-weighted + SVD)."""
    from scipy.sparse import lil_matrix
    from sklearn.decomposition import TruncatedSVD
    d = df.copy()
    d["gl_account"] = d["gl_account"].astype(str)
    accts = sorted(d["gl_account"].unique())
    idx = {a: i for i, a in enumerate(accts)}
    n = len(accts)
    C = lil_matrix((n, n), dtype=float)
    for _, s in d.groupby("document_id", sort=False)["gl_account"]:
        al = sorted(set(s))
        if len(al) > max_je_accts:           # cap consolidation-outlier JEs (else O(n^2) blowup)
            continue
        ii = [idx[a] for a in al]
        for p in range(len(ii)):
            for q in range(p + 1, len(ii)):
                C[ii[p], ii[q]] += 1.0
                C[ii[q], ii[p]] += 1.0
    C = C.tocsr()
    C.data = np.log1p(C.data)
    k = min(k, max(2, n - 1))
    emb = TruncatedSVD(n_components=k, random_state=0).fit_transform(C)
    return pd.DataFrame(emb, index=accts, columns=[f"nb{i}" for i in range(k)])


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--gl", type=Path, required=True)
    ap.add_argument("--out", type=Path, default=None)
    ap.add_argument("--k", type=int, default=24)
    a = ap.parse_args(argv)

    df = pd.read_csv(a.gl / "journal_entries.csv", low_memory=False)
    agg = account_features(df)
    agg["truth"] = [_DIGIT_TYPE.get(str(x)[0], "other") for x in agg.index]
    emb = neighbour_embedding(df, k=a.k)
    f = agg.join(emb, how="inner")
    f = f[f["truth"] != "other"].copy()
    nb_cols = [c for c in f.columns if c.startswith("nb")]
    y = f["truth"].to_numpy()

    from sklearn.ensemble import RandomForestClassifier
    from sklearn.model_selection import cross_val_predict

    def rfcv(cols):
        X = f[cols].fillna(0).to_numpy()
        X = (X - X.mean(0)) / (X.std(0) + 1e-9)
        p = cross_val_predict(RandomForestClassifier(n_estimators=400, class_weight="balanced_subsample",
                              n_jobs=-1, random_state=0), X, y, cv=5)
        return float((p == y).mean()), p

    a_agg, _ = rfcv(_FEATURES)
    a_nb, p_nb = rfcv(nb_cols)
    a_both, _ = rfcv(_FEATURES + nb_cols)
    print(f"accounts={len(f)}  types={f['truth'].nunique()}  nb_dim={len(nb_cols)}")
    print(f"5-class RF-CV accuracy:  aggregate {a_agg:.3f}   neighbour {a_nb:.3f}   "
          f"combined {a_both:.3f}   (§24 aggregate ceiling ~0.40)")
    print("neighbour per-type recall:")
    rec = {}
    for t in sorted(set(y)):
        m = (y == t)
        rec[t] = {"n": int(m.sum()), "recall": round(float((p_nb[m] == t).mean()), 3)}
        print(f"  {t:10s} n={int(m.sum()):4d} recall={rec[t]['recall']:.3f}")
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(json.dumps({"n_accounts": int(len(f)), "nb_dim": len(nb_cols),
                                     "acc_aggregate": round(a_agg, 4), "acc_neighbour": round(a_nb, 4),
                                     "acc_combined": round(a_both, 4), "neighbour_per_type": rec}, indent=2))


if __name__ == "__main__":
    main()

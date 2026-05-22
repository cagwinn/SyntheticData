"""Per-JE anomaly score = sum of standardised NLLs under the normal-system manifold:
  amount     : per-account_class signed-log1p density (robust Gaussian), max over lines.
  struct     : -log P(archetype_sig | source), add-1 smoothed (the per-JE structural manifold).
  behavioral : per-source Bernoulli surprise of {weekend, post_close, round-dollar} —
               the fraud-bias signatures (CLAUDE.md: weekend/off-hours/post-close/round).
score(JE) = z(amount) + z(struct) + z(behavioral), z-standardised on the normal set.
Writes scores.parquet: document_id, is_fraud, fraud_type, is_anomaly, amount_z, struct_z,
behav_z, score.  (The 'reconstructed normal manifold'; high score = residual = anomaly.)
"""
from __future__ import annotations
import argparse
from pathlib import Path
import numpy as np, pandas as pd

_EPS = 1e-6


def _load_lines(gl: Path) -> pd.DataFrame:
    df = pd.read_csv(gl / "journal_entries.csv", low_memory=False)
    deb = pd.to_numeric(df.get("debit_amount", 0), errors="coerce").fillna(0.0)
    cred = pd.to_numeric(df.get("credit_amount", 0), errors="coerce").fillna(0.0)
    df["amt"] = np.where(deb != 0, deb, -cred)
    df["aamt"] = np.abs(df["amt"])
    df["y"] = np.sign(df["amt"]) * np.log1p(df["aamt"])
    df["account_class"] = df.get("account_class").astype(str).fillna("UNK").replace("nan", "UNK")
    df["dr"] = df["amt"] > 0
    wd = pd.to_datetime(df.get("posting_date"), errors="coerce").dt.weekday
    df["weekend"] = (wd >= 5).fillna(False)
    pc = df.get("is_post_close")
    df["post_close"] = (pc.astype(str).str.lower().isin(["true", "1"]) if pc is not None else False)
    df["is_round"] = (df["aamt"] >= 1000) & (np.mod(df["aamt"], 500) == 0)
    return df


def _je_features(df: pd.DataFrame) -> pd.DataFrame:
    src = "source" if "source" in df.columns else "document_type"

    def sig(g):
        return "|".join(sorted(f"{a}:{'D' if d else 'C'}"
                               for a, d in zip(g["gl_account"].astype(str), g["dr"])))
    grp = df.groupby("document_id")
    je = pd.DataFrame({
        "source": grp[src].first().astype(str),
        "archetype_sig": grp.apply(sig),
        "weekend": grp["weekend"].any(),
        "post_close": grp["post_close"].any(),
        "is_round": grp["is_round"].any(),
    })
    je["is_fraud"] = grp["is_fraud"].any() if "is_fraud" in df.columns else False
    je["fraud_type"] = grp["fraud_type"].first() if "fraud_type" in df.columns else None
    je["is_anomaly"] = grp["is_anomaly"].any() if "is_anomaly" in df.columns else False
    return je


def _fit_amount(df):
    out = {}
    for cls, g in df.groupby("account_class"):
        med = float(g["y"].median()); mad = max(float((g["y"] - med).abs().median()) * 1.4826, _EPS)
        out[cls] = (med, mad)
    gm = float(df["y"].median()); out["__g__"] = (gm, max(float((df["y"] - gm).abs().median()) * 1.4826, _EPS))
    return out


def _amount_nll(df, fit):
    loc = df["account_class"].map(lambda c: fit.get(c, fit["__g__"])[0])
    sc = df["account_class"].map(lambda c: fit.get(c, fit["__g__"])[1])
    z = (df["y"] - loc) / sc
    return df.assign(_n=0.5 * z * z + np.log(sc)).groupby("document_id")["_n"].max()


def _fit_struct(je):
    counts, vocab = {}, {}
    for s, g in je.groupby("source"):
        vc = g["archetype_sig"].value_counts().to_dict(); counts[s] = vc; vocab[s] = len(vc)
    return counts, vocab


def _struct_nll(je, counts, vocab):
    def f(r):
        vc = counts.get(r["source"], {}); V = vocab.get(r["source"], 1)
        return -np.log((vc.get(r["archetype_sig"], 0) + 1) / (sum(vc.values()) + V + 1))
    return je.apply(f, axis=1)


def _fit_behav(je, feats):
    p = {}
    for s, g in je.groupby("source"):
        p[s] = {fe: float(g[fe].mean()) for fe in feats}
    glob = {fe: float(je[fe].mean()) for fe in feats}
    return p, glob


def _behav_nll(je, p, glob, feats):
    def f(r):
        ps = p.get(r["source"], glob); tot = 0.0
        for fe in feats:
            pe = min(max(ps.get(fe, glob[fe]), _EPS), 1 - _EPS)
            tot += -np.log(pe if r[fe] else (1 - pe))
        return tot
    return je.apply(f, axis=1)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--normal", type=Path, required=True)
    ap.add_argument("--test", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args(argv)
    feats = ["weekend", "post_close", "is_round"]
    nd, td = _load_lines(a.normal), _load_lines(a.test)
    nje, tje = _je_features(nd), _je_features(td)
    afit = _fit_amount(nd); counts, vocab = _fit_struct(nje); bp, bg = _fit_behav(nje, feats)

    def z_of(series, ref):
        mu, sd = ref.mean(), (ref.std() or 1.0)
        return (series - mu) / sd
    n_amt, n_str, n_beh = _amount_nll(nd, afit), _struct_nll(nje, counts, vocab), _behav_nll(nje, bp, bg, feats)
    out = tje.copy()
    out["amount_z"] = z_of(_amount_nll(td, afit).reindex(out.index), n_amt)
    out["struct_z"] = z_of(_struct_nll(tje, counts, vocab), n_str)
    out["behav_z"] = z_of(_behav_nll(tje, bp, bg, feats), n_beh)
    out["score"] = out[["amount_z", "struct_z", "behav_z"]].fillna(0).sum(axis=1)
    out.reset_index().to_parquet(a.out)
    fr = out["is_fraud"]
    print(f"SCORE_DONE n={len(out)} fraud_rate={fr.mean():.4f} "
          f"mean_score_fraud={out.loc[fr,'score'].mean():.3f} mean_score_norm={out.loc[~fr,'score'].mean():.3f}")


if __name__ == "__main__":
    main()

"""Self-contained per-JE anomaly scorer (inverse-audit capstone, Stage 1).

The 'reconstructed normal-system manifold' is a conditional density fit on the
NORMAL GL; a test JE's score is its negative log-likelihood (residual) under it.

  amount term  : per-(account_class) signed-log1p amount density (robust Gaussian,
                 median + 1.4826*MAD); per-line surprise aggregated to the JE by max
                 (one extreme line flags the JE). A conditional amount density — the
                 lightweight Stage-1 stand-in for the spline flow (drop-in upgrade).
  struct term  : -log P(archetype_sig | source) under the normal data (add-1
                 smoothed). archetype_sig = sorted (gl_account, dr/cr) set per JE,
                 the same signature corpus_structure.py uses.
  score(JE)    = z(amount_nll) + z(struct_nll), z-standardised on the normal set.

Writes scores.parquet: document_id, is_anomaly, anomaly_type, amount_z, struct_z, score.
"""
from __future__ import annotations
import argparse
from pathlib import Path

import numpy as np
import pandas as pd

_EPS = 1e-6


def _load_lines(gl: Path) -> pd.DataFrame:
    df = pd.read_csv(gl / "journal_entries.csv", low_memory=False)
    deb = pd.to_numeric(df.get("debit_amount", 0), errors="coerce").fillna(0.0)
    cred = pd.to_numeric(df.get("credit_amount", 0), errors="coerce").fillna(0.0)
    df["amt"] = np.where(deb != 0, deb, -cred)
    df["y"] = np.sign(df["amt"]) * np.log1p(np.abs(df["amt"]))
    df["account_class"] = df.get("account_class").astype(str).fillna("UNK").replace("nan", "UNK")
    df["dr"] = df["amt"] > 0
    return df


def _fit_amount(df: pd.DataFrame) -> dict:
    """Per-account_class robust location/scale of y, + a global fallback."""
    out = {}
    for cls, g in df.groupby("account_class"):
        med = float(g["y"].median())
        mad = float((g["y"] - med).abs().median()) * 1.4826
        out[cls] = (med, max(mad, _EPS))
    g_med = float(df["y"].median())
    g_mad = max(float((df["y"] - g_med).abs().median()) * 1.4826, _EPS)
    out["__global__"] = (g_med, g_mad)
    return out


def _amount_nll_per_je(df: pd.DataFrame, fit: dict) -> pd.Series:
    loc = df["account_class"].map(lambda c: fit.get(c, fit["__global__"])[0])
    scale = df["account_class"].map(lambda c: fit.get(c, fit["__global__"])[1])
    z = (df["y"] - loc) / scale
    df = df.assign(_nll=0.5 * z * z + np.log(scale))     # Gaussian NLL up to const
    return df.groupby("document_id")["_nll"].max()       # one surprising line flags the JE


def _je_meta(df: pd.DataFrame) -> pd.DataFrame:
    src = "source" if "source" in df.columns else "document_type"

    def sig(g: pd.DataFrame) -> str:
        return "|".join(sorted(f"{a}:{'D' if d else 'C'}"
                               for a, d in zip(g["gl_account"].astype(str), g["dr"])))

    grp = df.groupby("document_id")
    meta = pd.DataFrame({
        "source": grp[src].first().astype(str),
        "archetype_sig": grp.apply(sig),
    })
    if "is_anomaly" in df.columns:
        meta["is_anomaly"] = grp["is_anomaly"].any()
    else:
        meta["is_anomaly"] = False
    meta["anomaly_type"] = (grp["anomaly_type"].first() if "anomaly_type" in df.columns else None)
    return meta


def _fit_struct(meta: pd.DataFrame) -> tuple[dict, dict]:
    """P(archetype_sig | source) with add-1 smoothing over each source's sig vocab."""
    counts, vocab = {}, {}
    for src, g in meta.groupby("source"):
        vc = g["archetype_sig"].value_counts().to_dict()
        counts[src] = vc
        vocab[src] = len(vc)
    return counts, vocab


def _struct_nll(meta: pd.DataFrame, counts: dict, vocab: dict) -> pd.Series:
    def nll(row) -> float:
        src = row["source"]; vc = counts.get(src, {}); V = vocab.get(src, 1)
        n = vc.get(row["archetype_sig"], 0)
        total = sum(vc.values())
        p = (n + 1) / (total + V + 1)            # add-1 smoothed; unseen sig → small p
        return -np.log(p)
    return meta.apply(nll, axis=1)


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--normal", type=Path, required=True)
    ap.add_argument("--test", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args(argv)

    nd, td = _load_lines(a.normal), _load_lines(a.test)
    nmeta, tmeta = _je_meta(nd), _je_meta(td)

    afit = _fit_amount(nd)
    counts, vocab = _fit_struct(nmeta)

    # z-stats on the NORMAL set
    n_amt = _amount_nll_per_je(nd, afit)
    n_str = _struct_nll(nmeta, counts, vocab)
    amu, asd = n_amt.mean(), n_amt.std() or 1.0
    smu, ssd = n_str.mean(), n_str.std() or 1.0

    t_amt = _amount_nll_per_je(td, afit)
    t_str = _struct_nll(tmeta, counts, vocab)
    out = tmeta.copy()
    out["amount_z"] = ((t_amt - amu) / asd).reindex(out.index)
    out["struct_z"] = ((t_str.reindex(out.index)) - smu) / ssd
    out["score"] = out["amount_z"].fillna(0) + out["struct_z"].fillna(0)
    out.reset_index().to_parquet(a.out)
    print(f"SCORE_DONE n={len(out)} "
          f"mean_score_anom={out.loc[out.is_anomaly,'score'].mean():.3f} "
          f"mean_score_norm={out.loc[~out.is_anomaly,'score'].mean():.3f}")


if __name__ == "__main__":
    main()

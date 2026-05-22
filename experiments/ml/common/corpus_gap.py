"""Descriptive corpus-vs-synthetic gap — 'what's missing on the synthetic end'.

Complements `datasynth-data behavioral score` (normalized degradation ratios)
with raw, interpretable observables in plain units, so the gap is legible:
lines-per-JE, amount distribution (log-moments / Benford / round-dollar / small-
ticket share), source mix, weekend share, and per-source inter-event times.

    python -m common.corpus_gap --corpus /path/corpus.parquet --syn /path/journal_entries.csv

Corpus uses its own column names; synthetic uses canonical names. Both are
mapped here. Emits a side-by-side table + a JSON of the gaps.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import pandas as pd

_ROUND = np.array([1_000.0, 5_000.0, 10_000.0, 25_000.0, 50_000.0, 100_000.0])


def _benford_mad(a: np.ndarray) -> float:
    fd = np.array([int(str(int(x))[0]) for x in a if x >= 1], dtype=int)
    if not fd.size:
        return float("nan")
    obs = np.array([(fd == d).mean() for d in range(1, 10)])
    exp = np.log10(1 + 1 / np.arange(1, 10))
    return float(np.abs(obs - exp).mean())


def _iet_stats(df: pd.DataFrame, src: str, date: str) -> tuple[float, float]:
    iets = []
    sub = df[[src, date]].dropna()
    sub = sub.assign(_d=pd.to_datetime(sub[date], errors="coerce")).dropna(subset=["_d"])
    for _, g in sub.groupby(src):
        days = np.sort(g["_d"].astype("int64").to_numpy()) / 86_400_000_000_000
        if days.size > 1:
            iets.append(np.diff(days))
    if not iets:
        return float("nan"), float("nan")
    allg = np.concatenate(iets)
    return float(allg.mean()), float(allg.std())


def observables(df: pd.DataFrame, jeid: str, src: str, amt: np.ndarray, date: str) -> dict:
    a = np.abs(amt)
    a = a[np.isfinite(a) & (a > 0)]
    la = np.log1p(a)
    lpje = df.groupby(jeid).size().to_numpy() if jeid in df else np.array([np.nan])
    pdt = pd.to_datetime(df[date], errors="coerce") if date in df else pd.Series([], dtype="datetime64[ns]")
    nearest = np.abs(a[:, None] - _ROUND[None, :]).min(axis=1) if a.size else np.array([1e9])
    iet_m, iet_s = _iet_stats(df, src, date) if src in df and date in df else (float("nan"), float("nan"))
    vc = df[src].astype(str).value_counts(normalize=True).to_numpy() if src in df else np.array([1.0])
    return {
        "n_lines": int(len(df)),
        "n_JEs": int(df[jeid].nunique()) if jeid in df else float("nan"),
        "lines_per_JE_mean": float(np.nanmean(lpje)),
        "lines_per_JE_p95": float(np.nanpercentile(lpje, 95)),
        "log_amt_mean": float(la.mean()),
        "log_amt_std": float(la.std()),
        "log_amt_skew": float(((la - la.mean()) ** 3).mean() / (la.std() ** 3 + 1e-9)),
        "benford_mad": _benford_mad(a),
        "round_dollar_frac": float((nearest < 1.0).mean()),
        "small_ticket_frac(<100)": float((a < 100).mean()),
        "p99_amount": float(np.percentile(a, 99)) if a.size else float("nan"),
        "weekend_frac": float((pdt.dt.dayofweek >= 5).mean()) if len(pdt) else float("nan"),
        "n_sources": int(len(vc)),
        "source_top1_share": float(vc.max()),
        "source_entropy": float(-(vc[vc > 0] * np.log(vc[vc > 0])).sum()),
        "iet_days_mean": iet_m,
        "iet_days_std": iet_s,
    }


def load_corpus(path: Path) -> dict:
    df = pd.read_parquet(path)
    amt = pd.to_numeric(df["Functional Amount"], errors="coerce").to_numpy()
    return observables(df, "JE Number", "Source", amt, "Entry Date")


def load_syn(path: Path) -> dict:
    df = pd.read_csv(path, low_memory=False)
    deb = pd.to_numeric(df.get("debit_amount", 0), errors="coerce").fillna(0.0)
    cred = pd.to_numeric(df.get("credit_amount", 0), errors="coerce").fillna(0.0)
    amt = np.where(deb != 0, deb, cred).astype(float)
    return observables(df, "document_id", "source", amt, "posting_date")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", type=Path, required=True)
    ap.add_argument("--syn", type=Path, required=True)
    ap.add_argument("--out", type=Path, default=None)
    args = ap.parse_args()

    corp = load_corpus(args.corpus)
    syn = load_syn(args.syn)
    keys = list(corp.keys())
    print(f"{'observable':<26} {'corpus':>16} {'synthetic':>16} {'ratio syn/corp':>16}")
    print("-" * 78)
    gaps = {}
    for k in keys:
        c, s = corp[k], syn[k]
        r = (s / c) if (isinstance(c, (int, float)) and c not in (0, float("nan")) and np.isfinite(c) and c != 0) else float("nan")
        gaps[k] = {"corpus": c, "synthetic": s, "ratio": r}
        print(f"{k:<26} {c:>16.4g} {s:>16.4g} {r:>16.3g}")
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(gaps, indent=2))
        print(f"\nwrote {args.out}")


if __name__ == "__main__":
    main()

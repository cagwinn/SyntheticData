"""Corpus PROCESS footprint — profile the distinct posting processes the corpus
exhibits (per source code: line share, mean lines/JE, the dominant account-prefix
archetype) so we can find IMPORTANT process families the engine doesn't model.

Account-prefix (first 3 chars) is used as a chart-agnostic account-group proxy,
so the posting *shape* of each process is visible without a chart-specific
semantic classifier. Runs on the corpus (parquet) and a synthetic generate.

    DATASYNTH_CORPUS_DIR=/path python -m corpus_processes --input <dir|csv> \\
        --label corpus --out runs/structure/proc_corpus.json [--sample-jes 300000]

Aggregate stats only (no row content; descriptions are not read — PII).
"""
from __future__ import annotations

import argparse
import glob
import json
import os
from collections import Counter
from pathlib import Path

import numpy as np
import pandas as pd

_CORPUS_MAP = {
    "JE Number": "je",
    "GL Account Number": "account",
    "Functional Amount": "amount",
    "Source": "source",
}


def load(path: Path, sample_jes: int, max_rows: int = 12_000_000) -> pd.DataFrame:
    p = Path(path)
    if p.is_file():  # synthetic canonical CSV
        raw = pd.read_csv(p, low_memory=False)
        deb = pd.to_numeric(raw.get("debit_amount", 0), errors="coerce").fillna(0.0)
        cred = pd.to_numeric(raw.get("credit_amount", 0), errors="coerce").fillna(0.0)
        df = pd.DataFrame({
            "je": raw.get("document_id"),
            "account": raw.get("gl_account").astype(str),
            "amount": np.where(deb != 0, deb, -cred),
            "source": raw.get("source"),
        })
    else:
        import pyarrow.parquet as pq

        frames, total = [], 0
        for f in sorted(glob.glob(str(p / "JE_*.parquet"))):
            avail = set(pq.read_schema(f).names)
            cols = [c for c in _CORPUS_MAP if c in avail]
            fr = pd.read_parquet(f, columns=cols).rename(columns=_CORPUS_MAP)
            frames.append(fr)
            total += len(fr)
            if total >= max_rows:
                break
        df = pd.concat(frames, ignore_index=True)
        df["amount"] = pd.to_numeric(df["amount"], errors="coerce").fillna(0.0)

    je = df["je"].astype(str)
    uniq = je.unique()
    if sample_jes > 0 and len(uniq) > sample_jes:
        rng = np.random.default_rng(0)
        keep = set(rng.choice(uniq, size=sample_jes, replace=False).tolist())
        df = df[je.isin(keep).values].reset_index(drop=True)
    return df


def profile(df: pd.DataFrame, label: str, top_n: int = 40) -> dict:
    df = df.copy()
    df["account"] = df["account"].astype(str)
    df["dr"] = df["amount"] > 0
    df["pfx"] = df["account"].str.slice(0, 3)
    df["source"] = df["source"].astype(str)
    n_lines = len(df)
    src_vc = df["source"].value_counts()

    procs = []
    for src, cnt in src_vc.head(top_n).items():
        sub = df[df["source"] == src]
        lpje = sub.groupby("je", sort=False).size()

        def sig(g):
            return tuple(sorted(set(zip(g["pfx"], np.where(g["dr"], "D", "C")))))

        arche = sub.groupby("je", sort=False).apply(sig)
        top = Counter(arche).most_common(1)
        procs.append({
            "source": src,
            "line_share": round(cnt / n_lines, 4),
            "mean_lpje": round(float(lpje.mean()), 2),
            "distinct_acct_prefixes": int(sub["pfx"].nunique()),
            "top_prefix_archetype": str(top[0][0]) if top else "",
            "top_archetype_share": round(top[0][1] / max(lpje.size, 1), 3) if top else 0.0,
        })
    return {"label": label, "n_lines": n_lines,
            "distinct_sources": int(src_vc.size), "processes": procs}


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--input", type=Path, default=os.environ.get("DATASYNTH_CORPUS_DIR"))
    ap.add_argument("--label", required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--sample-jes", type=int, default=300_000)
    a = ap.parse_args(argv)
    if not a.input:
        raise SystemExit("pass --input or set DATASYNTH_CORPUS_DIR")
    df = load(a.input, a.sample_jes)
    prof = profile(df, a.label)
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(prof, indent=2))
    print(json.dumps(prof, indent=2))


if __name__ == "__main__":
    main()

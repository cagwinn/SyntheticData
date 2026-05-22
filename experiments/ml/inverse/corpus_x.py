"""Corpus → 29-dim summary-stat vector(s) x for the inverse capstone.

Reads the corpus (one ``JE_*.parquet`` per client under ``DATASYNTH_CORPUS_DIR``),
maps its column names to the canonical schema, and runs the SAME
``summary_stats_from_df`` the synthetic forward-sim uses — so the corpus x is
directly comparable to the training x. Emits ONLY the aggregate 29-dim
vector(s) as json, never row-level corpus content (inverse/SPEC.md privacy
contract).

    DATASYNTH_CORPUS_DIR=/path/corpus \\
        python -m inverse.corpus_x --out runs/inverse/corpus_x29.json [--per-client]

Reads only the columns the summary needs, so memory stays bounded across the
full subset. ``--per-client`` additionally writes one x per client (each client
is a single GL, closer to the synthetic per-run scale than the pooled vector)
to ``<out>.per_client.json`` for the recovered-θ spread analysis.
"""
from __future__ import annotations

import argparse
import glob
import json
import os
from pathlib import Path

import pandas as pd
import pyarrow.parquet as pq

from .simulate import FEATURE_NAMES, summary_stats_from_df

# corpus column -> canonical column expected by summary_stats_from_df.
# Functional Amount is signed (debit +, credit -); the summary uses |amount|,
# so routing it through debit_amount with credit_amount=0 is exact.
_COLMAP = {
    "JE Number": "document_id",
    "GL Account Number": "gl_account",
    "Functional Amount": "debit_amount",
    "Entry Date": "posting_date",
    "Effective Date": "document_date",
    "Source": "source",
}


def _read_client(path: str) -> pd.DataFrame:
    avail = set(pq.read_schema(path).names)
    cols = [c for c in _COLMAP if c in avail]
    df = pd.read_parquet(path, columns=cols).rename(columns=_COLMAP)
    df["credit_amount"] = 0.0
    return df


def _client_id(path: str) -> str:
    return Path(path).stem.replace("JE_", "")


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--corpus-dir", type=Path,
                    default=os.environ.get("DATASYNTH_CORPUS_DIR"))
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--per-client", action="store_true")
    a = ap.parse_args(argv)
    if not a.corpus_dir:
        raise SystemExit("set DATASYNTH_CORPUS_DIR or pass --corpus-dir")

    files = sorted(glob.glob(str(Path(a.corpus_dir) / "JE_*.parquet")))
    if not files:
        raise SystemExit(f"no JE_*.parquet under {a.corpus_dir}")
    a.out.parent.mkdir(parents=True, exist_ok=True)

    per_client = {}
    frames = []
    for f in files:
        df = _read_client(f)
        frames.append(df)
        if a.per_client:
            x = summary_stats_from_df(df)
            per_client[_client_id(f)] = {"x": [float(v) for v in x],
                                         "n_lines": int(len(df))}

    pooled = pd.concat(frames, ignore_index=True)
    x = summary_stats_from_df(pooled)
    a.out.write_text(json.dumps({"x": [float(v) for v in x],
                                 "feature_names": FEATURE_NAMES,
                                 "n_clients": len(files),
                                 "n_lines": int(len(pooled))}))
    print(f"[corpus_x] {len(files)} clients, {len(pooled):,} lines -> x29 -> {a.out}")
    for nm, v in zip(FEATURE_NAMES, x):
        print(f"  {nm:16s} {float(v):12.4f}")

    if a.per_client:
        pc_path = a.out.with_suffix(".per_client.json")
        pc_path.write_text(json.dumps({"feature_names": FEATURE_NAMES,
                                       "clients": per_client}))
        print(f"[corpus_x] per-client x ({len(per_client)}) -> {pc_path}")


if __name__ == "__main__":
    main()

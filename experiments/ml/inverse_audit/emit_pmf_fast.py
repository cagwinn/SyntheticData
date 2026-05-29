"""Fast per-source (debit,credit) pair-PMF extractor for AccountPairSubstitutionPass.

Vectorized drop-in for `corpus_vs_synth_gap.py --emit-pair-pmf`, whose dominant-pair
step uses `groupby("document_id").apply(python_fn)` — a per-group Python callback over
millions of JE groups (~hours on the full corpus). This computes the dominant
(max-debit-acct, max-credit-acct) pair per JE with vectorized `groupby.idxmax`, then
counts distinct pairs per source with a vectorized groupby. Emits the SAME JSON schema
the Rust `AccountPairSubstitutionPass` consumes (schema_version / min_jes_per_source /
produced_by / pmfs:[{source, n_jes, pmf:[[debit,credit,p],...]}]). Aggregate only — no
row content, no client ids, no document ids.

    cd experiments/ml && ~/mlenv/bin/python inverse_audit/emit_pmf_fast.py \
        --corpus-parquets ~/corpus/JE_*.parquet --out runs/A1/corpus_pair_pmf.json
"""
from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path

import pandas as pd

from inverse_audit.corpus_runner import load_canonical

MIN_JES_PER_SOURCE = 50


def dominant_pairs(df: pd.DataFrame) -> pd.DataFrame | None:
    """Per JE: (max-|debit| account, max-|credit| account, source). Vectorized."""
    need = {"document_id", "gl_account", "debit_amount", "credit_amount", "source"}
    if not need.issubset(df.columns):
        return None
    deb = df.loc[df["debit_amount"] > 0, ["document_id", "gl_account", "debit_amount"]]
    cred = df.loc[df["credit_amount"] > 0, ["document_id", "gl_account", "credit_amount"]]
    if deb.empty or cred.empty:
        return None
    d = deb.loc[deb.groupby("document_id", sort=False)["debit_amount"].idxmax()]
    c = cred.loc[cred.groupby("document_id", sort=False)["credit_amount"].idxmax()]
    d = d.set_index("document_id")["gl_account"].astype(str).rename("d")
    c = c.set_index("document_id")["gl_account"].astype(str).rename("c")
    src = (df.groupby("document_id", sort=False)["source"]
             .first().astype(str).rename("source"))
    m = pd.concat([d, c, src], axis=1, join="inner").dropna()
    return m


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--corpus-parquets", type=Path, nargs="+", required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--min-jes-per-source", type=int, default=MIN_JES_PER_SOURCE)
    a = ap.parse_args(argv)

    per_source: dict[str, Counter] = {}
    for p in a.corpus_parquets:
        try:
            df = load_canonical(p)
        except Exception as exc:  # noqa: BLE001 — skip non-GL / malformed shards
            print(f"skip {p.name}: {exc}", flush=True)
            continue
        m = dominant_pairs(df)
        if m is None or m.empty:
            print(f"skip {p.name}: no usable (debit,credit) pairs", flush=True)
            continue
        for src, sub in m.groupby("source"):
            cnt = per_source.setdefault(str(src), Counter())
            for (d, c), n in sub.groupby(["d", "c"]).size().items():
                cnt[(d, c)] += int(n)
        print(f"{p.name}: {len(m):,} JE-pairs, {m['source'].nunique()} sources", flush=True)

    pmfs = []
    for src, counts in per_source.items():
        total = sum(counts.values())
        if total < a.min_jes_per_source:
            continue
        triples = [[d, c, n / total] for (d, c), n in counts.most_common()]
        pmfs.append({"source": src, "n_jes": int(total), "pmf": triples})
    pmfs.sort(key=lambda x: x["source"])

    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps({
        "schema_version": 1,
        "min_jes_per_source": a.min_jes_per_source,
        "produced_by": "emit_pmf_fast.py",
        "pmfs": pmfs,
    }, indent=2))
    print(f"PMF: {len(pmfs)} sources, {sum(len(p['pmf']) for p in pmfs):,} pairs, "
          f"{sum(p['n_jes'] for p in pmfs):,} JEs -> {a.out}", flush=True)


if __name__ == "__main__":
    main()

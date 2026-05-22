"""Corpus structural / process fingerprint — what STRUCTURE and PROCESSES the
corpus carries that a SOTA synthetic engine should reproduce, beyond the §1
marginals (amount / lines-per-JE / source / IET, mostly closed in T2).

Extracts, identically for the corpus (parquet dir) and a synthetic generate
(journal_entries.csv), so gaps rank directly:

  1. JE archetypes      — the (sorted GL-account, dr/cr) signature per JE:
                          top archetypes, distinct count, top-50 coverage.
  2. GL flow graph      — debit-account -> credit-account edges per JE:
                          distinct edges, top-edge share, edge entropy.
  3. Dimensions         — Cost Center / Profit Center / Trading Partner /
                          Business Unit: distinct, entropy, fill rate.
  4. Multi-currency     — distinct currencies; fraction functional != reporting.
  5. Account Pareto     — distinct accounts; top-10% line share (activity skew).
  6. Reversal proxy     — share of (account, |amount|) seen with BOTH signs.
  7. Recurring proxy    — share of archetypes recurring across >= 2 periods.

    DATASYNTH_CORPUS_DIR=/path python -m corpus_structure --input <dir|csv> \\
        --label corpus --out runs/structure/corpus.json [--sample-jes 1000000]

Emits ONLY aggregate structural statistics — never row-level content (privacy).
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

# corpus column -> canonical
_CORPUS_MAP = {
    "JE Number": "je",
    "GL Account Number": "account",
    "Functional Amount": "amount",
    "Source": "source",
    "Cost Center": "cost_center",
    "Profit Center": "profit_center",
    "Trading Partner": "trading_partner",
    "Tarding Partner": "trading_partner",  # corpus typo (sic)
    "Business Unit": "business_unit",
    "Functional Currency Code": "func_ccy",
    "Reporting Currency Code": "rep_ccy",
    "Period": "period",
}
# synthetic (canonical CSV) columns of interest; amount handled separately
_SYN_COLS = [
    "document_id", "gl_account", "debit_amount", "credit_amount", "source",
    "cost_center", "profit_center", "trading_partner", "posting_date",
]


def _entropy(counts) -> float:
    arr = np.asarray([c for c in counts if c > 0], dtype=float)
    if arr.size == 0:
        return 0.0
    p = arr / arr.sum()
    return float(-(p * np.log(p)).sum())


def load_corpus(corpus_dir: Path, sample_jes: int, max_rows: int = 12_000_000) -> pd.DataFrame:
    """Read the needed columns, stopping once `max_rows` accumulate so memory +
    time stay bounded on the 100M+-line corpus; the per-JE subsample then
    reduces to `sample_jes` distinct JEs (whole JEs kept by the isin filter)."""
    import pyarrow.parquet as pq

    frames, total = [], 0
    for f in sorted(glob.glob(str(corpus_dir / "JE_*.parquet"))):
        avail = set(pq.read_schema(f).names)
        cols = [c for c in _CORPUS_MAP if c in avail]
        fr = pd.read_parquet(f, columns=cols).rename(columns=_CORPUS_MAP)
        frames.append(fr)
        total += len(fr)
        if total >= max_rows:
            break
    df = pd.concat(frames, ignore_index=True)
    df["amount"] = pd.to_numeric(df["amount"], errors="coerce").fillna(0.0)
    return _subsample(df, sample_jes)


def load_syn(csv: Path, sample_jes: int) -> pd.DataFrame:
    raw = pd.read_csv(csv, low_memory=False)
    deb = pd.to_numeric(raw.get("debit_amount", 0), errors="coerce").fillna(0.0)
    cred = pd.to_numeric(raw.get("credit_amount", 0), errors="coerce").fillna(0.0)
    df = pd.DataFrame({
        "je": raw.get("document_id"),
        "account": raw.get("gl_account").astype(str),
        "amount": np.where(deb != 0, deb, -cred),  # signed: dr +, cr -
        "source": raw.get("source"),
        "cost_center": raw.get("cost_center"),
        "profit_center": raw.get("profit_center"),
        "trading_partner": raw.get("trading_partner"),
        "currency": raw.get("currency"),  # SAP WAERS (doc currency); foreign line = != entity base
        "period": pd.to_datetime(raw.get("posting_date"), errors="coerce").dt.to_period("M").astype(str),
    })
    return _subsample(df, sample_jes)


def _subsample(df: pd.DataFrame, sample_jes: int) -> pd.DataFrame:
    """Keep a deterministic random subset of `sample_jes` whole JEs."""
    if sample_jes <= 0:
        return df
    je = df["je"].astype(str)
    uniq = je.unique()
    if len(uniq) <= sample_jes:
        return df
    rng = np.random.default_rng(0)
    keep = set(rng.choice(uniq, size=sample_jes, replace=False).tolist())
    return df[je.isin(keep).values].reset_index(drop=True)


def fingerprint(df: pd.DataFrame, label: str) -> dict:
    df = df.copy()
    df["account"] = df["account"].astype(str)
    df["dr"] = df["amount"] > 0
    n_lines = len(df)
    n_jes = df["je"].nunique()

    # 1. JE archetypes: signature = sorted tuple of (account, dr/cr).
    def sig(g):
        return tuple(sorted(zip(g["account"], np.where(g["dr"], "D", "C"))))
    arche = df.groupby("je", sort=False).apply(sig)
    arche_counts = Counter(arche)
    top50_cov = sum(c for _, c in arche_counts.most_common(50)) / max(n_jes, 1)

    # 2. GL flow graph: debit-account -> credit-account edges per JE.
    edges = Counter()
    for _, g in df.groupby("je", sort=False):
        ds = g.loc[g["dr"], "account"].unique()
        cs = g.loc[~g["dr"], "account"].unique()
        for d in ds:
            for c in cs:
                edges[(d, c)] += 1
    edge_tot = sum(edges.values()) or 1
    top_edge_share = (edges.most_common(1)[0][1] / edge_tot) if edges else 0.0

    # 3. Dimensions.
    def dim(col):
        if col not in df or df[col].isna().all():
            return {"distinct": 0, "entropy": 0.0, "fill": 0.0}
        s = df[col].dropna().astype(str)
        s = s[s != ""]
        vc = s.value_counts()
        return {"distinct": int(vc.size), "entropy": round(_entropy(vc.values), 3),
                "fill": round(len(s) / n_lines, 3)}

    # 4. Multi-currency. Two schemas: corpus canonical (func_ccy/rep_ccy) and the
    # synthetic SAP-style WAERS column `currency` (a foreign line = doc ccy != entity base).
    if "func_ccy" in df and "rep_ccy" in df:
        mismatch = float((df["func_ccy"].astype(str) != df["rep_ccy"].astype(str)).mean())
        n_ccy = int(pd.concat([df["func_ccy"], df["rep_ccy"]]).astype(str).nunique())
    elif "currency" in df:
        cc = df["currency"].astype(str)
        n_ccy = int(cc.nunique())
        base = cc.mode().iloc[0] if not cc.mode().empty else None
        mismatch = float((cc != base).mean()) if base is not None else 0.0
    else:
        mismatch, n_ccy = 0.0, 1

    # 5. Account Pareto.
    acc_vc = df["account"].value_counts()
    top10pct = max(1, int(np.ceil(acc_vc.size * 0.10)))
    top10_share = float(acc_vc.iloc[:top10pct].sum() / n_lines)

    # 6. Reversal proxy: (account, round |amount|) seen with both signs.
    key = list(zip(df["account"], np.round(np.abs(df["amount"]), 2)))
    pos = set(k for k, d in zip(key, df["dr"]) if d)
    neg = set(k for k, d in zip(key, df["dr"]) if not d)
    both = pos & neg
    reversal_share = round(len(both) / max(len(pos | neg), 1), 4)

    # 7. Recurring proxy: archetypes that recur across >= 2 periods.
    recurring = 0.0
    if "period" in df:
        ap = pd.DataFrame({"arche": arche.values, "je": arche.index})
        per = df.groupby("je", sort=False)["period"].first()
        ap["period"] = ap["je"].map(per)
        by_arche_periods = ap.groupby("arche")["period"].nunique()
        multi = by_arche_periods[by_arche_periods >= 2].index
        recurring = round(float(ap["arche"].isin(set(multi)).mean()), 4)

    return {
        "label": label,
        "n_lines": int(n_lines),
        "n_jes": int(n_jes),
        "archetypes": {"distinct": len(arche_counts),
                       "per_1k_jes": round(1000 * len(arche_counts) / max(n_jes, 1), 2),
                       "top50_coverage": round(top50_cov, 3)},
        "flow_graph": {"distinct_edges": len(edges),
                       "top_edge_share": round(top_edge_share, 4),
                       "edge_entropy": round(_entropy(list(edges.values())), 3)},
        "dimensions": {d: dim(d) for d in
                       ["cost_center", "profit_center", "trading_partner", "business_unit"]},
        "multi_currency": {"distinct_currencies": n_ccy, "func_ne_rep_frac": round(mismatch, 4)},
        "account_pareto": {"distinct_accounts": int(acc_vc.size),
                           "top10pct_line_share": round(top10_share, 3)},
        "reversal_proxy_share": reversal_share,
        "recurring_archetype_share": recurring,
    }


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--input", type=Path,
                    default=os.environ.get("DATASYNTH_CORPUS_DIR"))
    ap.add_argument("--label", required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--sample-jes", type=int, default=1_000_000)
    a = ap.parse_args(argv)
    if not a.input:
        raise SystemExit("pass --input (corpus dir or synthetic csv) or set DATASYNTH_CORPUS_DIR")

    p = Path(a.input)
    df = load_syn(p, a.sample_jes) if p.is_file() else load_corpus(p, a.sample_jes)
    fp = fingerprint(df, a.label)
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(fp, indent=2))
    print(json.dumps(fp, indent=2))


if __name__ == "__main__":
    main()

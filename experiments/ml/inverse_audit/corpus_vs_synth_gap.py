"""Round 0 — quantify the corpus-vs-synthetic realism gap on the five metrics that
emerged from the Stage 2 + audit-packet work (FINDINGS §13). Each subsequent SOTA
round will close part of this gap; this script provides the before/after baseline.

Metrics (all aggregate; no row content):
  1. source_cond_tightness — per-source distinct-edge-pair entropy distribution.
     Tight (low entropy) = razor-thin source-conditional account-pair prior — the
     audit-relevant signal that today's synthetic doesn't fully model.
  2. edges_per_je — total distinct directed account-flow edges / scored JE count.
     Manifold density; corpus shows ~10x variance across clients.
  3. lines_per_je_p{50,99,99.9,max} — line-count distribution; corpus production-scale
     top JEs averaged 834 lines, max 994.
  4. tp_set_size — distinct trading partners. Corpus has consolidated-entity clients
     (tp_count=1) at one extreme.
  5. edge_concentration_top10pct — fraction of total reconstructed flow concentrated
     on the top-10% of edges (a Gini-ish concentration scalar).

Privacy: corpus paths are runtime args only; commits / docs get aggregate stats only.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path

import numpy as np
import pandas as pd

from inverse_audit.corpus_runner import CORPUS_COLMAP, load_canonical


def _je_line_counts(df: pd.DataFrame) -> np.ndarray:
    return df.groupby("document_id", sort=False).size().to_numpy()


def _source_cond_tightness(df: pd.DataFrame) -> dict:
    """For each source value, count distinct (debit_acct, credit_acct) edge pairs touched
    and the entropy over those pairs. Return median / p10 / p90 across sources."""
    if "source" not in df.columns:
        return {"n_sources": 0, "entropy_median": None, "entropy_p10": None,
                "entropy_p90": None, "edges_per_source_median": None}
    # Light-weight: per-line "edge proxy" = the JE's (max-debit-acct, max-credit-acct)
    # would need flow reconstruction. Instead, characterise the source-conditional
    # account-touch distribution: per source, distribution of `gl_account` values.
    g = df.groupby("source")["gl_account"]
    ents, n_accts = [], []
    for _, accts in g:
        vc = accts.value_counts(normalize=True)
        n_accts.append(int(len(vc)))
        if len(vc) > 1:
            ents.append(float(-(vc * np.log(vc)).sum() / np.log(len(vc))))   # normalised
        else:
            ents.append(0.0)
    if not ents:
        return {"n_sources": 0, "entropy_median": None, "entropy_p10": None,
                "entropy_p90": None, "edges_per_source_median": None}
    ents = np.asarray(ents); na = np.asarray(n_accts)
    return {"n_sources": len(ents),
            "entropy_median": float(np.median(ents)),
            "entropy_p10":    float(np.percentile(ents, 10)),
            "entropy_p90":    float(np.percentile(ents, 90)),
            "accts_per_source_median": float(np.median(na)),
            "accts_per_source_p99":    float(np.percentile(na, 99))}


def _edges_per_je_and_concentration(df: pd.DataFrame) -> dict:
    """Compute the aggregate account-pair adjacency: (most-positive-debit-acct,
    most-negative-credit-acct) per JE → count distinct pairs / concentration. This is
    a *cheap* edge proxy (vs full OT reconstruction) — good enough for gap measurement."""
    # Per JE, pick the dominant debit-acct and dominant credit-acct (largest |amount|).
    sign = np.sign(df["debit_amount"] - df["credit_amount"])     # +1 debit, -1 credit
    # group per JE, pick first debit acct + first credit acct by magnitude
    def _pair(g: pd.DataFrame) -> tuple:
        d = g[g["debit_amount"] > 0]
        c = g[g["credit_amount"] > 0]
        if d.empty or c.empty:
            return (None, None)
        return (str(d.loc[d["debit_amount"].idxmax(),  "gl_account"]),
                str(c.loc[c["credit_amount"].idxmax(), "gl_account"]))
    pairs = df.groupby("document_id", sort=False).apply(_pair, include_groups=False)
    pairs = [p for p in pairs if p[0] is not None]
    if not pairs:
        return {"n_jes": 0, "n_edges": 0, "edges_per_je": None, "top10pct_share": None}
    n_jes = len(pairs)
    edge_counts: dict[tuple[str, str], int] = {}
    for p in pairs:
        edge_counts[p] = edge_counts.get(p, 0) + 1
    n_edges = len(edge_counts)
    counts_desc = np.sort(np.array(list(edge_counts.values())))[::-1]
    top10pct = max(1, int(np.ceil(0.10 * n_edges)))
    top10_share = float(counts_desc[:top10pct].sum() / counts_desc.sum())
    return {"n_jes": n_jes, "n_edges": n_edges,
            "edges_per_je": round(n_edges / n_jes, 5),
            "jes_per_edge": round(n_jes / n_edges, 2),
            "top10pct_share": round(top10_share, 3)}


def measure_one(df: pd.DataFrame, label: str) -> dict:
    out = {"label": label, "n_lines": int(len(df)),
           "n_jes": int(df["document_id"].nunique())}
    lc = _je_line_counts(df)
    out["lines_per_je"] = {
        "p50":    int(np.percentile(lc, 50)),
        "p90":    int(np.percentile(lc, 90)),
        "p99":    int(np.percentile(lc, 99)),
        "p99_9":  int(np.percentile(lc, 99.9)),
        "max":    int(lc.max()),
        "mean":   round(float(lc.mean()), 2),
    }
    out["tp_set_size"] = (int(df["trading_partner"].dropna().nunique())
                          if "trading_partner" in df.columns else 0)
    out["source_cond"] = _source_cond_tightness(df)
    out["edges"]       = _edges_per_je_and_concentration(df)
    return out


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--corpus-parquets", type=Path, nargs="+", required=True,
                    help="corpus GL parquets (runtime args only — never committed)")
    ap.add_argument("--synth-csv", type=Path, required=True,
                    help="canonical synthetic GL journal_entries.csv (typically the "
                         "Stage 1 archive ia_canonical_v5/test/journal_entries.csv)")
    ap.add_argument("--out", type=Path, required=True, help="gap report json")
    a = ap.parse_args(argv)
    a.out.parent.mkdir(parents=True, exist_ok=True)

    import hashlib
    corpus_results = []
    for p in a.corpus_parquets:
        tag = os.path.basename(p)
        sha = hashlib.sha1(tag.encode()).hexdigest()[:8]
        try:
            df = load_canonical(p)
        except ValueError as exc:
            print(f"corpus[{sha}] SKIP (not a GL): {exc}")
            continue
        r = measure_one(df, label=f"corpus[{sha}]")
        r["size_mb"] = round(os.path.getsize(p) / 1e6, 1)
        corpus_results.append(r)
        print(f"corpus[{sha}] {r['size_mb']:>5.1f}MB  jes={r['n_jes']:>7,}  "
              f"lines/je p99={r['lines_per_je']['p99']:>4d}  tp={r['tp_set_size']:>4d}  "
              f"edges/je={r['edges']['edges_per_je']:.4f}  "
              f"src_ent_med={r['source_cond']['entropy_median']:.3f}")

    # Synth: load CSV, rename to canonical (synth uses canonical names natively)
    s = pd.read_csv(a.synth_csv, low_memory=False)
    # synth already has debit_amount / credit_amount / source / trading_partner
    synth = measure_one(s, label="synth[canonical_v5]")
    synth["size_mb"] = round(os.path.getsize(a.synth_csv) / 1e6, 1)
    print(f"\nsynth[canonical_v5] {synth['size_mb']:>5.1f}MB  jes={synth['n_jes']:>7,}  "
          f"lines/je p99={synth['lines_per_je']['p99']:>4d}  tp={synth['tp_set_size']:>4d}  "
          f"edges/je={synth['edges']['edges_per_je']:.4f}  "
          f"src_ent_med={synth['source_cond']['entropy_median']:.3f}")

    # Build the gap table: corpus median vs synth
    def med(field_path: list[str]):
        vals = []
        for r in corpus_results:
            v = r
            for k in field_path:
                v = v.get(k, None) if isinstance(v, dict) else None
                if v is None: break
            if v is not None: vals.append(v)
        return float(np.median(vals)) if vals else None

    def get_synth(field_path: list[str]):
        v = synth
        for k in field_path:
            v = v.get(k, None) if isinstance(v, dict) else None
            if v is None: return None
        return v

    metrics = [
        ("lines/je p99",                 ["lines_per_je", "p99"]),
        ("lines/je p99.9",               ["lines_per_je", "p99_9"]),
        ("lines/je max",                 ["lines_per_je", "max"]),
        ("lines/je mean",                ["lines_per_je", "mean"]),
        ("tp_set_size",                  ["tp_set_size"]),
        ("edges/je",                     ["edges", "edges_per_je"]),
        ("jes/edge",                     ["edges", "jes_per_edge"]),
        ("top10pct edge concentration",  ["edges", "top10pct_share"]),
        ("src-cond entropy median",      ["source_cond", "entropy_median"]),
        ("src-cond entropy p10 (tight)", ["source_cond", "entropy_p10"]),
        ("accts/source median",          ["source_cond", "accts_per_source_median"]),
        ("accts/source p99",             ["source_cond", "accts_per_source_p99"]),
    ]
    print(f"\n{'metric':32s}{'corpus median':>16s}{'synth':>12s}{'corpus/synth':>16s}")
    print("-" * 80)
    gap_table = []
    for name, path in metrics:
        c = med(path); s_ = get_synth(path)
        if c is None or s_ is None:
            ratio = None
            print(f"{name:32s}{'-':>16s}{'-':>12s}{'-':>16s}")
        else:
            ratio = (c / s_) if s_ not in (0, 0.0) else None
            r_str = f"{ratio:.2f}x" if ratio is not None else "-"
            print(f"{name:32s}{c:>16.3f}{s_:>12.3f}{r_str:>16s}")
        gap_table.append({"metric": name, "corpus_median": c, "synth": s_,
                          "corpus_over_synth": ratio})

    a.out.write_text(json.dumps({
        "corpus": corpus_results, "synth": synth, "gap_table": gap_table,
    }, indent=2))
    print(f"\nfull report -> {a.out}")


if __name__ == "__main__":
    main()

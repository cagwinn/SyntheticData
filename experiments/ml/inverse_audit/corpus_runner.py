"""Stage 2 — apply the relational arm of the capstone to corpus GL data.

Loads a corpus parquet, canonicalises columns into the synthetic-data schema the
relational scorer expects, fits the graph manifold on the file (no labelled `normal`
set exists in the corpus → fit-on-self; the score becomes the JE's deviation from the
*self-consistent* manifold), then emits **aggregate statistics only** plus the decoupled
substrate JSON. No JE descriptions, amounts, account labels, or counterparty values
leave this module.

Usage (path is a runtime argument; never committed):
    python -m inverse_audit.corpus_runner --parquet <path> --out <out_dir>
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import pandas as pd

from inverse_audit.relational.ot_flow import reconstruct_per_je  # noqa: F401  (used via score_df)
from inverse_audit.relational.graph_scorer import (
    _SCORE_FEATURES,
    fit_graph_manifold,
    score_df,
    z_of,
)
# Note: graph_export currently reads CSVs (synthetic schema). Corpus-graph export
# is a follow-on (refactor export_graph to accept a DataFrame or add a parquet path).

# corpus parquet → synthetic-canonical column mapping (ISO 21378-ish ↔ DataSynth).
# Note "Tarding Partner" (sic) is the corpus's literal column name.
CORPUS_COLMAP = {
    "JE Number":         "document_id",
    "GL Account Number": "gl_account",
    "Source":            "source",
    "Tarding Partner":   "trading_partner",
    "Effective Date":    "posting_date",
    "Period":            "period",
    "Business Unit":     "business_unit",
    "Cost Center":       "cost_center",
    "Profit Center":     "profit_center",
}


def load_canonical(parquet_path: Path) -> pd.DataFrame:
    """Map corpus columns → canonical; split signed Functional Amount → debit/credit."""
    df = pd.read_parquet(parquet_path).rename(columns=CORPUS_COLMAP)
    fa = pd.to_numeric(df.get("Functional Amount", 0), errors="coerce").fillna(0.0)
    df["debit_amount"]  = fa.where(fa > 0, 0.0)
    df["credit_amount"] = (-fa).where(fa < 0, 0.0)
    # Trim to relational-scorer's columns (drop everything else — defence in depth).
    keep = ["document_id", "gl_account", "debit_amount", "credit_amount",
            "source", "trading_partner"]
    return df[[c for c in keep if c in df.columns]].copy()


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--parquet", type=Path, required=True,
                    help="corpus GL parquet (path stays in runtime args, never in commits)")
    ap.add_argument("--out", type=Path, required=True, help="output dir for aggregates")
    # --export-graph deferred until graph_export accepts a parquet path / DataFrame.
    a = ap.parse_args(argv)
    a.out.mkdir(parents=True, exist_ok=True)

    df = load_canonical(a.parquet)
    n_lines = len(df)
    n_jes = int(df["document_id"].nunique())
    print(f"loaded: {n_lines:,} lines / {n_jes:,} JEs")

    # Fit-on-self: no labelled normal set, so the manifold IS the corpus and the score
    # becomes each JE's deviation from the self-consistent reference distribution.
    manifold = fit_graph_manifold(df)
    print(f"manifold: edges={len(manifold['edge_p']):,} nodes={len(manifold['pagerank']):,} "
          f"tp_set={len(manifold['tp_set']):,} normal_scc_accts={len(manifold['normal_scc_set']):,}")

    scored = score_df(df, manifold)
    for c in _SCORE_FEATURES:
        if c in scored.columns:
            scored[c + "_z"] = z_of(scored[c].to_numpy(), scored[c].to_numpy())
    z_cols = [c + "_z" for c in _SCORE_FEATURES if (c + "_z") in scored.columns]
    scored["relational_score"] = scored[z_cols].fillna(0).sum(axis=1)
    scored.reset_index().to_parquet(a.out / "graph_scores.parquet")

    rs = scored["relational_score"].to_numpy()
    pcts = np.percentile(rs, [50, 75, 90, 95, 99, 99.9])
    print(f"\nrelational_score distribution:")
    print(f"  p50={pcts[0]:6.2f}  p75={pcts[1]:6.2f}  p90={pcts[2]:6.2f}  "
          f"p95={pcts[3]:6.2f}  p99={pcts[4]:6.2f}  p99.9={pcts[5]:6.2f}")
    print(f"  mean={rs.mean():6.2f}  std={rs.std():6.2f}  max={rs.max():6.2f}")

    print(f"\nper-feature aggregates (corpus medians/maxes; no JE content):")
    feat_summary: dict = {}
    for c in _SCORE_FEATURES:
        if c not in scored.columns:
            continue
        v = scored[c].to_numpy()
        med, mx, p99 = float(np.median(v)), float(v.max()), float(np.percentile(v, 99))
        feat_summary[c] = {"median": med, "max": mx, "p99": p99,
                           "mean": float(v.mean()), "std": float(v.std())}
        print(f"  {c:32s} med={med:8.3f}  p99={p99:8.3f}  max={mx:8.3f}")

    # Hot list: top-1% JE IDs by relational_score (IDs only, no per-JE content).
    n_top = max(1, int(len(scored) * 0.01))
    top_ids = scored.nlargest(n_top, "relational_score").index.astype(str).tolist()
    (a.out / "top1pct_je_ids.json").write_text(
        json.dumps({"n_jes": len(scored), "n_top1pct": n_top, "je_ids": top_ids}, indent=2))
    print(f"\ntop 1% candidates: {n_top:,} JE IDs written to {a.out}/top1pct_je_ids.json")

    summary = {
        "n_lines": int(n_lines), "n_jes": int(n_jes),
        "manifold": {
            "n_edges":          len(manifold["edge_p"]),
            "n_nodes":          len(manifold["pagerank"]),
            "tp_set_size":      len(manifold["tp_set"]),
            "tp_acc_set_size":  len(manifold["tp_acc_set"]),
            "n_scc_accounts":   len(manifold["normal_scc_set"]),
            "n_accounts":       len(manifold["acc_count"]),
        },
        "relational_score": {
            "percentiles": {f"p{p}": float(v) for p, v in zip([50,75,90,95,99,99.9], pcts)},
            "mean": float(rs.mean()), "std": float(rs.std()),
            "min":  float(rs.min()),  "max": float(rs.max()),
        },
        "per_feature": feat_summary,
        "top1pct_n_jes": n_top,
    }
    (a.out / "summary.json").write_text(json.dumps(summary, indent=2))
    print(f"summary -> {a.out}/summary.json")

    print("CORPUS_STAGE2_DONE")


if __name__ == "__main__":
    main()

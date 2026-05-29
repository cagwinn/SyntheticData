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
    _ALL_FEATURES,
    _SCORE_FEATURES,
    fit_graph_manifold,
    score_df,
    z_of,
)
from inverse_audit.relational.graph_export import export_graph_df

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
    """Map corpus columns → canonical; split signed Functional Amount → debit/credit.
    Raises a clear ValueError on non-GL parquets (registry files, schema drift)."""
    df = pd.read_parquet(parquet_path).rename(columns=CORPUS_COLMAP)
    if "Functional Amount" not in df.columns:
        raise ValueError(
            f"parquet at {parquet_path.name} is missing 'Functional Amount' "
            f"(columns={list(df.columns)[:8]}…) — likely a registry file or a different schema."
        )
    fa = pd.to_numeric(df["Functional Amount"], errors="coerce").fillna(0.0)
    df["debit_amount"]  = fa.where(fa > 0, 0.0)
    df["credit_amount"] = (-fa).where(fa < 0, 0.0)
    keep = ["document_id", "gl_account", "debit_amount", "credit_amount",
            "source", "trading_partner"]
    return df[[c for c in keep if c in df.columns]].copy()


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--parquet", type=Path, required=True,
                    help="corpus GL parquet (path stays in runtime args, never in commits)")
    ap.add_argument("--out", type=Path, required=True, help="output dir for aggregates")
    ap.add_argument("--mode", choices=("self", "half-split"), default="self",
                    help="self = fit on full corpus, score on full (cycle/tp_account_novelty=0 by "
                         "construction); half-split = shuffle by JE, fit on first half, score second "
                         "half (recovers all new-in-test signals)")
    ap.add_argument("--seed", type=int, default=42, help="deterministic half-split shuffle")
    ap.add_argument("--rf-model", type=Path, default=None,
                    help="optional rf_arm.joblib → apply the DataSynth-trained RF + emit hybrid hot list (Tier C-1)")
    ap.add_argument("--export-graph", action="store_true",
                    help="also emit the decoupled substrate JSON (no row content)")
    a = ap.parse_args(argv)
    a.out.mkdir(parents=True, exist_ok=True)

    df = load_canonical(a.parquet)
    n_lines = len(df)
    n_jes = int(df["document_id"].nunique())
    print(f"loaded: {n_lines:,} lines / {n_jes:,} JEs (mode={a.mode})")

    if a.mode == "self":
        # Fit-on-self: the manifold IS the corpus; score = each JE's deviation from the
        # self-consistent reference. cycle_novelty + tp_account_novelty collapse to 0
        # by construction (no "new in test").
        nd, td = df, df
    else:
        # Half-split by document_id with a deterministic shuffle. Brings back the
        # novelty-vs-baseline features at the cost of halving each set.
        rng = np.random.default_rng(a.seed)
        jes = df["document_id"].dropna().astype(str).unique()
        rng.shuffle(jes)
        cut = len(jes) // 2
        normal_jes = set(jes[:cut]); test_jes = set(jes[cut:])
        nd = df[df["document_id"].astype(str).isin(normal_jes)].copy()
        td = df[df["document_id"].astype(str).isin(test_jes)].copy()
        print(f"half-split: normal={nd['document_id'].nunique():,} JEs  "
              f"test={td['document_id'].nunique():,} JEs")

    manifold = fit_graph_manifold(nd)
    print(f"manifold: edges={len(manifold['edge_p']):,} nodes={len(manifold['pagerank']):,} "
          f"tp_set={len(manifold['tp_set']):,} normal_scc_accts={len(manifold['normal_scc_set']):,}")

    scored = score_df(td, manifold)
    # z-baseline: normal-scored features if half-split (proper out-of-sample), else self.
    n_scored = score_df(nd, manifold) if a.mode == "half-split" else scored
    # Compute _z for ALL features (the unsupervised relational_score still uses only the
    # positive-prior _SCORE_FEATURES; the rest are needed when a supervised RF arm is applied).
    for c in _ALL_FEATURES:
        if c in scored.columns and c in n_scored.columns:
            scored[c + "_z"] = z_of(scored[c].to_numpy(), n_scored[c].to_numpy())
    z_cols = [c + "_z" for c in _SCORE_FEATURES if (c + "_z") in scored.columns]
    scored["relational_score"] = scored[z_cols].fillna(0).sum(axis=1)

    # Tier C-1: optional supervised hybrid arm. Apply a DataSynth-label-trained RF
    # (rf_arm.joblib) to the corpus _z features and hybridise (rank-z-sum) with the
    # unsupervised residual. The per-GL z-normalisation makes the synthetic-trained RF
    # transferable to the corpus (fit-on-self), sidestepping the A1 SBI-OOD problem.
    if a.rf_model:
        import joblib
        from scipy.stats import rankdata
        # joblib.load is pickle-based; safe here — the model is OUR OWN artifact written by
        # rf_arm.py (joblib.dump) on this trusted box, not a third-party/untrusted file.
        bundle = joblib.load(a.rf_model)
        rf, feats = bundle["rf"], bundle["features"]
        for f in feats:
            if f not in scored.columns:
                scored[f] = 0.0
        proba = rf.predict_proba(scored[feats].fillna(0).to_numpy())[:, 1]
        scored["rf_score"] = proba
        def _rz(x):
            r = rankdata(x); return (r - r.mean()) / (r.std() + 1e-9)
        scored["hybrid_score"] = _rz(scored["relational_score"].to_numpy()) + _rz(proba)

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

    if a.rf_model and "hybrid_score" in scored.columns:
        hyb_top = scored.nlargest(n_top, "hybrid_score").index.astype(str).tolist()
        uns_top = set(scored.nlargest(n_top, "relational_score").index.astype(str))
        overlap = len(set(hyb_top) & uns_top) / n_top
        rfp = scored["rf_score"].to_numpy()
        (a.out / "top1pct_hybrid_je_ids.json").write_text(json.dumps({
            "n_top": n_top, "je_ids": hyb_top,
            "overlap_with_unsup_top1pct": round(overlap, 3),
            "rf_score": {"median": float(np.median(rfp)), "p99": float(np.percentile(rfp, 99)),
                         "max": float(rfp.max())}}, indent=2))
        print(f"HYBRID hot list: {n_top:,} JE IDs; overlap with unsup top-1% = {overlap:.2f}; "
              f"rf_score med={np.median(rfp):.3f} p99={np.percentile(rfp, 99):.3f} max={rfp.max():.3f} "
              f"-> {a.out}/top1pct_hybrid_je_ids.json")

    summary = {
        "mode": a.mode,
        "n_lines": int(n_lines), "n_jes": int(n_jes),
        "n_scored_jes": int(len(scored)),
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

    if a.export_graph:
        export_graph_df(nd, td, a.out / "graph_scores.parquet",
                        a.out / "account_flow_graph.json")
        print(f"graph JSON -> {a.out}/account_flow_graph.json")

    print("CORPUS_STAGE2_DONE")


if __name__ == "__main__":
    main()

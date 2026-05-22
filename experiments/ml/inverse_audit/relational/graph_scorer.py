"""Rung 1+ relational scorer: fit the NORMAL account-flow graph as the relational
'manifold', score test JEs by how off-manifold their reconstructed edges/structure are.
Targets the families the per-JE density residual is blind to (UnusualAccountPair,
NewCounterparty, CircularTransaction, ...) — the relational analogue of score.py.

Per-JE relational features (z-standardised on normal-JE scores, summed -> relational_score):
  edge_surprise_max : max over JE flows of -log P_normal(edge); novel/rare account-pair pops.
  edge_surprise_w   : amount-weighted mean -log P_normal(edge); a more balanced signal.
  back_edge         : 1 iff any JE flow (s->d) has its reverse (d->s) seen in normal
                      (a 2-cycle / circular-transaction proxy).
  coupling_entropy  : OT coupling entropy (ot_flow) — high = reconstruction-ambiguous JE.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import pandas as pd

from inverse_audit.relational.ot_flow import reconstruct_per_je

_EPS = 1e-9
_LOG_EPS = 30.0      # cap surprise when an edge is unseen in normal (P -> _EPS)
_LABEL_COLS = ("is_anomaly", "anomaly_type", "is_fraud", "fraud_type")


def _load_lines(d: Path) -> pd.DataFrame:
    return pd.read_csv(d / "journal_entries.csv", low_memory=False)


def fit_graph_manifold(normal_df: pd.DataFrame) -> dict:
    """Per-edge frequency from the reconstructed normal graph."""
    per = reconstruct_per_je(normal_df)
    edge_w: dict[tuple[str, str], float] = {}
    for _, (flows, _) in per.items():
        for s, d, w in flows:
            edge_w[(s, d)] = edge_w.get((s, d), 0.0) + w
    total = sum(edge_w.values()) or 1.0
    return {"edge_p": {k: v / total for k, v in edge_w.items()},
            "n_normal_jes": len(per)}


def score_je(flows, manifold) -> dict:
    """Per-JE relational features from reconstructed flows + a fitted manifold."""
    edge_p = manifold["edge_p"]
    if not flows:
        return {"edge_surprise_max": 0.0, "edge_surprise_w": 0.0,
                "back_edge": 0.0, "n_edges": 0}
    sur, w_, back = [], [], 0.0
    for s, d, w in flows:
        p = edge_p.get((s, d), _EPS)
        sur.append(min(-np.log(p), _LOG_EPS))
        w_.append(w)
        if (d, s) in edge_p:    # back-edge in normal => potential cycle if the test edge closes it
            back = 1.0
    sur_a = np.asarray(sur); w_a = np.asarray(w_)
    return {"edge_surprise_max": float(sur_a.max()),
            "edge_surprise_w": float((sur_a * w_a).sum() / max(w_a.sum(), _EPS)),
            "back_edge": back,
            "n_edges": len(flows)}


def _je_labels(df: pd.DataFrame) -> pd.DataFrame:
    """Roll per-line labels up to per-JE: is_anomaly_je = any-line, anomaly_type = first non-null."""
    g = df.groupby("document_id", sort=False)
    out = pd.DataFrame(index=g.size().index)
    if "is_anomaly" in df.columns:
        out["is_anomaly_je"] = g["is_anomaly"].max().astype("boolean").fillna(False).astype(bool)
    if "anomaly_type" in df.columns:
        out["anomaly_type"] = g["anomaly_type"].apply(lambda s: s.dropna().iloc[0] if s.notna().any() else None)
    return out


def score_df(test_df: pd.DataFrame, manifold) -> pd.DataFrame:
    per = reconstruct_per_je(test_df)
    rows = []
    for je_id, (flows, h) in per.items():
        f = score_je(flows, manifold)
        f["je_id"] = je_id
        f["coupling_entropy"] = h
        rows.append(f)
    res = pd.DataFrame(rows).set_index("je_id")
    res.index = res.index.astype(str)
    labels = _je_labels(test_df)
    labels.index = labels.index.astype(str)
    return res.join(labels, how="left")


def z_of(test: np.ndarray, normal: np.ndarray) -> np.ndarray:
    med = float(np.nanmedian(normal))
    mad = max(float(np.nanmedian(np.abs(normal - med))) * 1.4826, _EPS)
    return (test - med) / mad


def assess(t_scored: pd.DataFrame) -> dict:
    """Headline + per-family metrics for the relational_score vs is_anomaly_je."""
    from sklearn.metrics import average_precision_score, roc_auc_score
    ia = t_scored["is_anomaly_je"].fillna(False).astype(bool).to_numpy()
    s = t_scored["relational_score"].to_numpy()
    n_pos = int(ia.sum())
    out: dict = {"n_jes": len(t_scored), "n_pos": n_pos,
                 "overall": {"pr_auc": float(average_precision_score(ia, s)),
                             "roc_auc": float(roc_auc_score(ia, s))}}
    by_type: dict = {}
    if "anomaly_type" in t_scored.columns:
        normal = ~ia
        for t in t_scored["anomaly_type"].dropna().unique():
            is_t = (t_scored["anomaly_type"] == t).fillna(False).to_numpy()
            if is_t.sum() < 5:
                continue
            mask = is_t | normal
            try:
                pr = average_precision_score(is_t[mask].astype(int), s[mask])
                roc = roc_auc_score(is_t[mask].astype(int), s[mask])
                by_type[str(t)] = {"n": int(is_t.sum()), "pr_auc": float(pr), "roc_auc": float(roc)}
            except Exception:
                pass
    out["per_anomaly_type"] = by_type
    return out


def main(argv=None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--normal", type=Path, required=True)
    ap.add_argument("--test", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True, help="per-JE relational features (parquet)")
    ap.add_argument("--assess-out", type=Path, default=None, help="metrics JSON (default: alongside --out)")
    a = ap.parse_args(argv)

    nd = _load_lines(a.normal); td = _load_lines(a.test)
    print(f"[graph_scorer] fit on normal: n_lines={len(nd)} n_jes={nd['document_id'].nunique()}")
    manifold = fit_graph_manifold(nd)
    print(f"  manifold: n_edges={len(manifold['edge_p'])} n_normal_jes={manifold['n_normal_jes']}")

    n_scored = score_df(nd, manifold)
    t_scored = score_df(td, manifold)
    for c in ("edge_surprise_max", "edge_surprise_w", "back_edge", "coupling_entropy"):
        t_scored[c + "_z"] = z_of(t_scored[c].to_numpy(), n_scored[c].to_numpy())
    t_scored["relational_score"] = (t_scored[[c + "_z" for c in
        ("edge_surprise_max", "edge_surprise_w", "back_edge", "coupling_entropy")]]
        .fillna(0).sum(axis=1))

    t_scored.reset_index().to_parquet(a.out)
    print(f"GRAPH_SCORE_DONE -> {a.out}")

    metrics = assess(t_scored)
    out_json = a.assess_out or a.out.with_suffix(".assess.json")
    out_json.write_text(json.dumps(metrics, indent=2))
    o = metrics["overall"]
    print(f"\nRELATIONAL_ARM_VS_IS_ANOMALY: n_pos={metrics['n_pos']} "
          f"pr_auc={o['pr_auc']:.4f} roc_auc={o['roc_auc']:.4f}")
    if metrics["per_anomaly_type"]:
        print("  per family (type vs normal-JE):")
        ranked = sorted(metrics["per_anomaly_type"].items(),
                        key=lambda kv: -kv[1]["roc_auc"])
        for t, m in ranked:
            print(f"    {t:30s} n={m['n']:4d} pr_auc={m['pr_auc']:.3f} roc={m['roc_auc']:.3f}")


if __name__ == "__main__":
    main()

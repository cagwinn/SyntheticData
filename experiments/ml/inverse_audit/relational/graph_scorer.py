"""Rung 1+ relational scorer (v2): fit the NORMAL account-flow graph as the relational
manifold, score test JEs by how off-manifold they are. v1 (edges-only marginal) under-
performed even the density baseline (ROC 0.45 vs 0.52) — the relational anomalies live
in dimensions edges alone don't see. v2 adds trading-partner novelty + PageRank
centrality and reports PER-FEATURE ROC so we know which signals actually carry weight.

Per-JE features (each z-standardised on normal-JE scores, summed -> relational_score):
  edge_surprise_max  : max over JE flows of -log P_normal(edge); rare/unseen account-pair pops.
  edge_surprise_w    : amount-weighted mean -log P_normal(edge).
  coupling_entropy   : OT coupling entropy; high = reconstruction-ambiguous JE.
  tp_novelty         : count of trading_partner values on this JE not seen in normal
                       (targets NewCounterparty + Unmatched/TransferPricing IC families).
  centrality_max     : max PageRank of touched accounts on the normal graph (PR is computed
                       by power iteration; no networkx dep) — targets CentralityAnomaly.
  back_edge          : 1 iff any (s->d) has its reverse (d->s) in normal (2-cycle proxy;
                       kept but fires often -> typically weak; per-feature ROC will show).
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import pandas as pd

from inverse_audit.relational.ot_flow import reconstruct_per_je

_EPS = 1e-9
_LOG_EPS = 30.0


def _load_lines(d: Path) -> pd.DataFrame:
    return pd.read_csv(d / "journal_entries.csv", low_memory=False)


def _pagerank(edge_w: dict[tuple[str, str], float], damping: float = 0.85,
              iters: int = 60, tol: float = 1e-9) -> dict[str, float]:
    """PageRank by power iteration on a weighted dict-of-edges. Returns {node: pr}."""
    nodes = sorted({n for e in edge_w for n in e})
    if not nodes:
        return {}
    idx = {n: i for i, n in enumerate(nodes)}
    N = len(nodes)
    out_w = np.zeros(N)
    for (s, _d), w in edge_w.items():
        out_w[idx[s]] += w
    r = np.full(N, 1.0 / N)
    for _ in range(iters):
        rn = np.full(N, (1.0 - damping) / N)
        for (s, d), w in edge_w.items():
            si = idx[s]
            if out_w[si] > 0:
                rn[idx[d]] += damping * r[si] * (w / out_w[si])
        # redistribute dangling mass
        dangling = float(((out_w == 0).astype(float) * r).sum()) * damping / N
        rn += dangling
        if np.max(np.abs(rn - r)) < tol:
            r = rn
            break
        r = rn
    s = r.sum() or 1.0
    return {n: float(r[idx[n]] / s) for n in nodes}


def fit_graph_manifold(normal_df: pd.DataFrame) -> dict:
    """Per-edge frequency + PageRank + trading_partner set, from the normal graph."""
    per = reconstruct_per_je(normal_df)
    edge_w: dict[tuple[str, str], float] = {}
    for _, (flows, _) in per.items():
        for s, d, w in flows:
            edge_w[(s, d)] = edge_w.get((s, d), 0.0) + w
    total = sum(edge_w.values()) or 1.0
    edge_p = {k: v / total for k, v in edge_w.items()}
    pr = _pagerank(edge_w)
    tp_set: set[str] = set()
    if "trading_partner" in normal_df.columns:
        tp_set = set(normal_df["trading_partner"].dropna().astype(str).unique())
    return {"edge_p": edge_p, "pagerank": pr, "tp_set": tp_set,
            "n_normal_jes": len(per)}


def score_je(flows, je_tp: list[str] | None, manifold) -> dict:
    """Per-JE relational features."""
    edge_p = manifold["edge_p"]; pr = manifold["pagerank"]; tp_set = manifold["tp_set"]
    if not flows:
        return {"edge_surprise_max": 0.0, "edge_surprise_w": 0.0, "back_edge": 0.0,
                "centrality_max": 0.0, "tp_novelty": 0, "n_edges": 0}
    sur, wts, back, cmax = [], [], 0.0, 0.0
    accts: set[str] = set()
    for s, d, w in flows:
        p = edge_p.get((s, d), _EPS)
        sur.append(min(-np.log(p), _LOG_EPS))
        wts.append(w)
        if (d, s) in edge_p:
            back = 1.0
        accts.add(s); accts.add(d)
    for a in accts:
        cmax = max(cmax, pr.get(a, 0.0))
    sur_a = np.asarray(sur); w_a = np.asarray(wts)
    novel = 0
    if je_tp and tp_set:
        novel = sum(1 for t in je_tp if t and str(t) not in tp_set)
    return {"edge_surprise_max": float(sur_a.max()),
            "edge_surprise_w": float((sur_a * w_a).sum() / max(w_a.sum(), _EPS)),
            "back_edge": back,
            "centrality_max": float(cmax),
            "tp_novelty": int(novel),
            "n_edges": len(flows)}


def _je_labels(df: pd.DataFrame) -> pd.DataFrame:
    g = df.groupby("document_id", sort=False)
    out = pd.DataFrame(index=g.size().index)
    if "is_anomaly" in df.columns:
        out["is_anomaly_je"] = g["is_anomaly"].max().astype("boolean").fillna(False).astype(bool)
    if "anomaly_type" in df.columns:
        out["anomaly_type"] = g["anomaly_type"].apply(
            lambda s: s.dropna().iloc[0] if s.notna().any() else None)
    return out


def _je_tps(df: pd.DataFrame) -> dict[str, list[str]]:
    """Per-JE list of trading_partner values (unique, non-null)."""
    if "trading_partner" not in df.columns:
        return {}
    out: dict[str, list[str]] = {}
    for je_id, g in df.groupby("document_id", sort=False):
        vals = g["trading_partner"].dropna().astype(str).unique().tolist()
        out[str(je_id)] = vals
    return out


def score_df(df: pd.DataFrame, manifold) -> pd.DataFrame:
    per = reconstruct_per_je(df)
    tps = _je_tps(df)
    rows = []
    for je_id, (flows, h) in per.items():
        f = score_je(flows, tps.get(je_id), manifold)
        f["je_id"] = je_id; f["coupling_entropy"] = h
        rows.append(f)
    res = pd.DataFrame(rows).set_index("je_id")
    res.index = res.index.astype(str)
    labels = _je_labels(df); labels.index = labels.index.astype(str)
    return res.join(labels, how="left")


def z_of(test: np.ndarray, normal: np.ndarray) -> np.ndarray:
    med = float(np.nanmedian(normal))
    mad = max(float(np.nanmedian(np.abs(normal - med))) * 1.4826, _EPS)
    return (test - med) / mad


_FEATURES = ("edge_surprise_max", "edge_surprise_w", "back_edge",
             "centrality_max", "tp_novelty", "coupling_entropy")


def assess(t_scored: pd.DataFrame) -> dict:
    from sklearn.metrics import average_precision_score, roc_auc_score
    ia = t_scored["is_anomaly_je"].fillna(False).astype(bool).to_numpy()
    s = t_scored["relational_score"].to_numpy()
    out: dict = {"n_jes": len(t_scored), "n_pos": int(ia.sum())}
    out["overall"] = {"pr_auc": float(average_precision_score(ia, s)),
                      "roc_auc": float(roc_auc_score(ia, s))}
    # per-feature ROC (which signals actually help)
    per_feat: dict = {}
    for f in _FEATURES:
        if f not in t_scored.columns:
            continue
        v = t_scored[f].fillna(0).to_numpy()
        try:
            per_feat[f] = {"pr_auc": float(average_precision_score(ia, v)),
                           "roc_auc": float(roc_auc_score(ia, v))}
        except Exception:
            pass
    out["per_feature"] = per_feat
    # per-family
    by_type: dict = {}
    if "anomaly_type" in t_scored.columns:
        normal = ~ia
        for t in t_scored["anomaly_type"].dropna().unique():
            is_t = (t_scored["anomaly_type"] == t).fillna(False).to_numpy()
            if is_t.sum() < 5:
                continue
            mask = is_t | normal
            try:
                by_type[str(t)] = {"n": int(is_t.sum()),
                                   "pr_auc": float(average_precision_score(is_t[mask].astype(int), s[mask])),
                                   "roc_auc": float(roc_auc_score(is_t[mask].astype(int), s[mask]))}
            except Exception:
                pass
    out["per_anomaly_type"] = by_type
    return out


def main(argv=None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--normal", type=Path, required=True)
    ap.add_argument("--test", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--assess-out", type=Path, default=None)
    a = ap.parse_args(argv)

    nd = _load_lines(a.normal); td = _load_lines(a.test)
    print(f"[graph_scorer v2] fit on normal: lines={len(nd)} jes={nd['document_id'].nunique()}")
    manifold = fit_graph_manifold(nd)
    print(f"  manifold: edges={len(manifold['edge_p'])} nodes={len(manifold['pagerank'])} "
          f"tp_set={len(manifold['tp_set'])}")

    n_scored = score_df(nd, manifold)
    t_scored = score_df(td, manifold)
    for c in _FEATURES:
        if c in t_scored.columns:
            t_scored[c + "_z"] = z_of(t_scored[c].to_numpy(), n_scored[c].to_numpy())
    z_cols = [c + "_z" for c in _FEATURES if (c + "_z") in t_scored.columns]
    t_scored["relational_score"] = t_scored[z_cols].fillna(0).sum(axis=1)

    t_scored.reset_index().to_parquet(a.out)
    print(f"GRAPH_SCORE_DONE -> {a.out}")

    m = assess(t_scored)
    out_json = a.assess_out or a.out.with_suffix(".assess.json")
    out_json.write_text(json.dumps(m, indent=2))
    o = m["overall"]
    print(f"\nRELATIONAL_ARM_VS_IS_ANOMALY: n_pos={m['n_pos']} "
          f"pr_auc={o['pr_auc']:.4f} roc_auc={o['roc_auc']:.4f}")
    print("  per-feature ROC (which signals carry weight):")
    for f, v in sorted(m["per_feature"].items(), key=lambda kv: -kv[1]["roc_auc"]):
        print(f"    {f:22s} pr_auc={v['pr_auc']:.3f} roc={v['roc_auc']:.3f}")
    if m["per_anomaly_type"]:
        print("  per family (type vs normal-JE), top by ROC:")
        for t, mm in sorted(m["per_anomaly_type"].items(),
                             key=lambda kv: -kv[1]["roc_auc"]):
            print(f"    {t:30s} n={mm['n']:4d} pr_auc={mm['pr_auc']:.3f} roc={mm['roc_auc']:.3f}")


if __name__ == "__main__":
    main()

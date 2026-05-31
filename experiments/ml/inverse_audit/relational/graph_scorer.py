"""Rung 1+ relational scorer (v3): fit the NORMAL account-flow graph as the relational
manifold; score test JEs by how off-manifold they are. v2 with naive z-sum-of-all under-
performed (the four mid/anti features diluted the two positive ones); v3 keeps only
positive-prior features in the deployable score and adds two targeted ones:

  positive-prior features (high value => anomaly; summed -> relational_score):
    edge_surprise_max    : max over JE flows of -log P_normal(edge)
    edge_surprise_w      : amount-weighted mean -log P_normal(edge)
    tp_account_novelty   : count of (trading_partner, account) PAIRS on this JE NOT seen in
                           normal -- this catches NewCounterparty / IC families where the
                           tp string itself isn't novel but the tp-touches-this-account pair is
    account_dormancy_max : max over touched accounts of -log(normal_count_of_account + 1) --
                           rarely-used (dormant) account proxy without needing date arithmetic

  reported-but-not-scored (per-feature ROC + as a visibility/diagnostic only):
    back_edge, centrality_max, tp_novelty (raw string), coupling_entropy

  diagnostic ceiling: relational_score_lr -- LogisticRegression over all standardised
  features with 5-fold CV (out-of-fold proba). Uses test labels => UPPER BOUND only, not a
  deployable scorer. Shows the best a linear combination can achieve.
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

# features summed (positive-prior, unsupervised) into relational_score
_SCORE_FEATURES = ("edge_surprise_max", "edge_surprise_w",
                   "tp_account_novelty", "account_dormancy_max",
                   "cycle_novelty", "source_cond_edge_surprise_max")
# Tier-B candidate features — computed + per-feature ROC + LR-CV, but NOT in the
# deployable sum yet. Promote to _SCORE_FEATURES only if per-feature ROC is positive
# (v2 lesson: null features dilute the unweighted sum).
#   centrality_delta_max      : max over touched accounts of (test_PR - normal_PR), >=0 —
#                               an account that became MORE central in test (CentralityAnomaly)
#   tp_account_source_novelty : count of (tp, account, source) TRIPLES new vs normal —
#                               source context sharpens the (tp, account) pair for the
#                               NewCounterparty / MissingRelationship families
_CANDIDATE_FEATURES = ("centrality_delta_max", "tp_account_source_novelty")
# all features computed (for per-feature ROC reporting / LR-CV)
_ALL_FEATURES = (_SCORE_FEATURES + _CANDIDATE_FEATURES
                 + ("back_edge", "centrality_max", "tp_novelty", "coupling_entropy"))


def _load_lines(d: Path) -> pd.DataFrame:
    return pd.read_csv(d / "journal_entries.csv", low_memory=False)


def _scc_set(edge_w: dict[tuple[str, str], float]) -> set[str]:
    """Accounts in non-trivial (size >= 2) strongly-connected components — i.e. accounts
    that participate in a directed cycle in the aggregate flow graph."""
    if not edge_w:
        return set()
    from scipy.sparse import csr_matrix
    from scipy.sparse.csgraph import connected_components
    nodes = sorted({n for e in edge_w for n in e})
    idx = {n: i for i, n in enumerate(nodes)}
    rows, cols, data = [], [], []
    for (s, d), w in edge_w.items():
        rows.append(idx[s]); cols.append(idx[d]); data.append(float(w))
    g = csr_matrix((data, (rows, cols)), shape=(len(nodes), len(nodes)))
    n_comp, labels = connected_components(g, connection="strong")
    sizes = np.bincount(labels, minlength=n_comp)
    return {nodes[i] for i in range(len(nodes)) if sizes[labels[i]] >= 2}


def _pagerank(edge_w: dict[tuple[str, str], float], damping: float = 0.85,
              iters: int = 60, tol: float = 1e-9) -> dict[str, float]:
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
        dangling = float(((out_w == 0).astype(float) * r).sum()) * damping / N
        rn += dangling
        if np.max(np.abs(rn - r)) < tol:
            r = rn
            break
        r = rn
    s = r.sum() or 1.0
    return {n: float(r[idx[n]] / s) for n in nodes}


def _je_tp_accounts(df: pd.DataFrame) -> dict[str, set[tuple[str, str]]]:
    """Per-JE set of (trading_partner, gl_account) pairs (non-null tp only)."""
    out: dict[str, set[tuple[str, str]]] = {}
    if "trading_partner" not in df.columns or "gl_account" not in df.columns:
        return out
    sub = df[["document_id", "trading_partner", "gl_account"]].dropna(subset=["trading_partner"])
    if sub.empty:
        return out
    for je_id, g in sub.groupby("document_id", sort=False):
        out[str(je_id)] = {(str(t), str(a)) for t, a in zip(g["trading_partner"], g["gl_account"])}
    return out


def fit_graph_manifold(normal_df: pd.DataFrame, cost_fn=None, recon_fn=None) -> dict:
    """Per-edge frequency + PageRank + tp set + tp-account pair set + per-account count.
    cost_fn (optional, rung-2): learned within-JE OT cost passed to the flow reconstruction.
    recon_fn (optional): swap the per-JE reconstructor — e.g. ot_gpu.reconstruct_per_je_gpu (GPU
    batched Sinkhorn) for corpus-scale; defaults to the numpy ot_flow.reconstruct_per_je."""
    per = (recon_fn or reconstruct_per_je)(normal_df, cost_fn=cost_fn)
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
    je_tp_acc = _je_tp_accounts(normal_df)
    tp_acc_set: set[tuple[str, str]] = set()
    for pairs in je_tp_acc.values():
        tp_acc_set.update(pairs)
    # (tp, account, source) triples in normal — source context for the NewCounterparty /
    # MissingRelationship families (tp_account_source_novelty candidate).
    tp_acc_src_set: set[tuple[str, str, str]] = set()
    if "source" in normal_df.columns:
        _je_src0 = (normal_df.groupby("document_id", sort=False)["source"]
                             .first().astype(str).to_dict())
        for je_id, pairs in je_tp_acc.items():
            src = str(_je_src0.get(je_id, "nan"))
            for (t, a) in pairs:
                tp_acc_src_set.add((t, a, src))
    # per-account JE count in normal (for dormancy proxy)
    acc_count: dict[str, int] = {}
    if "gl_account" in normal_df.columns:
        c = (normal_df.groupby(["gl_account", "document_id"]).size().reset_index()
                      .groupby("gl_account").size())
        acc_count = {str(k): int(v) for k, v in c.items()}
    normal_scc_set = _scc_set(edge_w)
    # per-source edge frequencies — for source-conditional surprise.
    # P_normal(edge | source): a JE flow rare under its source's prior fires high
    # even when the global edge_p is moderate (the UnusualAccountPair mechanism).
    edge_p_by_source: dict[str, dict[tuple[str, str], float]] = {}
    if "source" in normal_df.columns:
        je_src = (normal_df.groupby("document_id", sort=False)["source"]
                            .first().astype(str).to_dict())
        by_src: dict[str, dict[tuple[str, str], float]] = {}
        for je_id, (flows, _) in per.items():
            src = je_src.get(je_id)
            if src is None or src == "nan":
                continue
            bs = by_src.setdefault(str(src), {})
            for s, d, w in flows:
                bs[(s, d)] = bs.get((s, d), 0.0) + w
        for src, edges in by_src.items():
            t = sum(edges.values()) or 1.0
            edge_p_by_source[src] = {k: v / t for k, v in edges.items()}
    return {"edge_p": edge_p, "pagerank": pr, "tp_set": tp_set,
            "tp_acc_set": tp_acc_set, "tp_acc_src_set": tp_acc_src_set,
            "acc_count": acc_count,
            "normal_scc_set": normal_scc_set,
            "edge_p_by_source": edge_p_by_source,
            "n_normal_jes": len(per)}


def score_je(flows, je_tp: list[str] | None,
             je_tp_acc_pairs: set[tuple[str, str]] | None,
             manifold, je_source: str | None = None,
             test_pr: dict | None = None) -> dict:
    edge_p = manifold["edge_p"]; pr = manifold["pagerank"]
    tp_set = manifold["tp_set"]; tp_acc_set = manifold["tp_acc_set"]
    tp_acc_src_set = manifold.get("tp_acc_src_set", set())
    acc_count = manifold["acc_count"]
    edge_p_by_src = manifold.get("edge_p_by_source", {})
    src_p = edge_p_by_src.get(str(je_source), {}) if je_source else {}
    if not flows:
        return {f: 0.0 for f in _ALL_FEATURES} | {"n_edges": 0}
    sur, wts, back, cmax = [], [], 0.0, 0.0
    src_sur: list[float] = []
    accts: set[str] = set()
    for s, d, w in flows:
        p = edge_p.get((s, d), _EPS)
        sur.append(min(-np.log(p), _LOG_EPS))
        wts.append(w)
        if (d, s) in edge_p:
            back = 1.0
        accts.add(s); accts.add(d)
        if src_p:
            ps = src_p.get((s, d), _EPS)
            src_sur.append(min(-np.log(ps), _LOG_EPS))
    cdelta = 0.0
    for a in accts:
        cmax = max(cmax, pr.get(a, 0.0))
        if test_pr is not None:
            cdelta = max(cdelta, float(test_pr.get(a, 0.0)) - float(pr.get(a, 0.0)))
    sur_a = np.asarray(sur); w_a = np.asarray(wts)
    novel_tp = 0
    if je_tp and tp_set:
        novel_tp = sum(1 for t in je_tp if t and str(t) not in tp_set)
    novel_pair = 0
    if je_tp_acc_pairs and tp_acc_set:
        novel_pair = sum(1 for p in je_tp_acc_pairs if p not in tp_acc_set)
    elif je_tp_acc_pairs:    # normal has no tp dimension => count all as novel
        novel_pair = len(je_tp_acc_pairs)
    novel_triple = 0
    if je_tp_acc_pairs and tp_acc_src_set:
        _src = str(je_source)
        novel_triple = sum(1 for (t, a) in je_tp_acc_pairs
                           if (t, a, _src) not in tp_acc_src_set)
    elif je_tp_acc_pairs and je_source is not None:
        novel_triple = len(je_tp_acc_pairs)
    # Dormancy proxy = IDF of touched accounts: log(N_total_account_activity / (account_count + 1)).
    # High when the account is rarely touched in normal => dormant-ish. Per-JE: max over accounts.
    n_acc = max(sum(acc_count.values()), 1)
    dorm_max = 0.0
    for a in accts:
        dorm_max = max(dorm_max, float(np.log(n_acc / (acc_count.get(a, 0) + 1))))
    return {"edge_surprise_max": float(sur_a.max()),
            "edge_surprise_w": float((sur_a * w_a).sum() / max(w_a.sum(), _EPS)),
            "back_edge": back,
            "centrality_max": float(cmax),
            "tp_novelty": int(novel_tp),
            "tp_account_novelty": int(novel_pair),
            "account_dormancy_max": dorm_max,
            "source_cond_edge_surprise_max": float(max(src_sur)) if src_sur else 0.0,
            "centrality_delta_max": float(cdelta),
            "tp_account_source_novelty": int(novel_triple),
            "coupling_entropy": 0.0,            # filled by caller from ot_flow
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
    if "trading_partner" not in df.columns:
        return {}
    out: dict[str, list[str]] = {}
    for je_id, g in df.groupby("document_id", sort=False):
        out[str(je_id)] = g["trading_partner"].dropna().astype(str).unique().tolist()
    return out


def score_df(df: pd.DataFrame, manifold, cost_fn=None, recon_fn=None) -> pd.DataFrame:
    per = (recon_fn or reconstruct_per_je)(df, cost_fn=cost_fn)
    # Build the AGGREGATE flow graph of *this* df and find its SCCs; accounts in
    # non-trivial SCCs that are NOT in the normal manifold's SCC set are participating
    # in cycles that didn't exist in normal — the cycle_novelty signal (Circular* families).
    df_edge_w: dict[tuple[str, str], float] = {}
    for _, (flows, _) in per.items():
        for s, d, w in flows:
            df_edge_w[(s, d)] = df_edge_w.get((s, d), 0.0) + w
    df_scc = _scc_set(df_edge_w)
    new_scc_accts = df_scc - manifold.get("normal_scc_set", set())
    df_pr = _pagerank(df_edge_w)   # test-graph PageRank for centrality_delta

    tps = _je_tps(df)
    tp_accs = _je_tp_accounts(df)
    # per-JE source (header column; all lines of a JE share it) — for source-conditional surprise.
    if "source" in df.columns:
        je_src = (df.groupby("document_id", sort=False)["source"]
                    .first().astype(str).to_dict())
        je_src = {str(k): v for k, v in je_src.items()}
    else:
        je_src = {}
    rows = []
    for je_id, (flows, h) in per.items():
        f = score_je(flows, tps.get(je_id), tp_accs.get(je_id),
                     manifold, je_source=je_src.get(je_id), test_pr=df_pr)
        f["je_id"] = je_id
        f["coupling_entropy"] = h
        accts = {s for s, d, _ in flows} | {d for s, d, _ in flows}
        f["cycle_novelty"] = float(len(accts & new_scc_accts))
        rows.append(f)
    res = pd.DataFrame(rows).set_index("je_id")
    res.index = res.index.astype(str)
    labels = _je_labels(df); labels.index = labels.index.astype(str)
    return res.join(labels, how="left")


def z_of(test: np.ndarray, normal: np.ndarray) -> np.ndarray:
    med = float(np.nanmedian(normal))
    mad = float(np.nanmedian(np.abs(normal - med))) * 1.4826
    if mad < 1e-6:
        # Feature is constant (or near-constant) on normal — e.g. cycle_novelty is 0
        # for normal by construction. Z-standardising would divide by ~0 and explode.
        # Return centred raw values; the feature's natural scale (0/1/k) contributes directly.
        return test - med
    # Clip per-feature z to ±10. Some highly regimented clients (e.g. one with a
    # source_cond MAD of 0.009) would otherwise produce z ≈ 1700, dwarfing every
    # other feature in the sum. Clipping is rank-preserving so PR-AUC/ROC are
    # unaffected — it just bounds the score's numerical scale.
    return np.clip((test - med) / mad, -10.0, 10.0)


def assess(t_scored: pd.DataFrame, score_col: str = "relational_score") -> dict:
    from sklearn.metrics import average_precision_score, roc_auc_score
    ia = t_scored["is_anomaly_je"].fillna(False).astype(bool).to_numpy()
    s = t_scored[score_col].to_numpy()
    out: dict = {"n_jes": len(t_scored), "n_pos": int(ia.sum())}
    out["overall"] = {"pr_auc": float(average_precision_score(ia, s)),
                      "roc_auc": float(roc_auc_score(ia, s))}
    per_feat: dict = {}
    for f in _ALL_FEATURES:
        if f not in t_scored.columns:
            continue
        v = t_scored[f].fillna(0).to_numpy()
        try:
            per_feat[f] = {"pr_auc": float(average_precision_score(ia, v)),
                           "roc_auc": float(roc_auc_score(ia, v))}
        except Exception:
            pass
    out["per_feature"] = per_feat
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
    ap.add_argument("--rung2", action="store_true",
                    help="rung-2: learn the within-JE OT cost from 2-line GT edges + use it for reconstruction")
    a = ap.parse_args(argv)

    nd = _load_lines(a.normal); td = _load_lines(a.test)
    print(f"[graph_scorer v3] fit on normal: lines={len(nd)} jes={nd['document_id'].nunique()}")
    cost_fn = None
    if a.rung2:
        from inverse_audit.relational.ot_cost import _two_line_edges, learn_edge_cost, make_cost_fn
        _edges = _two_line_edges(nd)
        _cm, _dflt = learn_edge_cost(_edges)
        cost_fn = make_cost_fn(_cm, _dflt)
        print(f"[rung2] learned OT cost from {len(_edges)} 2-line GT edges ({len(_cm)} distinct pairs)")
    manifold = fit_graph_manifold(nd, cost_fn=cost_fn)
    print(f"  manifold: edges={len(manifold['edge_p'])} nodes={len(manifold['pagerank'])} "
          f"tp_set={len(manifold['tp_set'])} tp_acc_set={len(manifold['tp_acc_set'])} "
          f"accounts_w_count={len(manifold['acc_count'])}")

    n_scored = score_df(nd, manifold, cost_fn=cost_fn)
    t_scored = score_df(td, manifold, cost_fn=cost_fn)
    for c in _ALL_FEATURES:
        if c in t_scored.columns:
            t_scored[c + "_z"] = z_of(t_scored[c].to_numpy(), n_scored[c].to_numpy())

    # deployable, unsupervised relational_score: sum of POSITIVE-PRIOR features only
    score_cols = [c + "_z" for c in _SCORE_FEATURES if (c + "_z") in t_scored.columns]
    t_scored["relational_score"] = t_scored[score_cols].fillna(0).sum(axis=1)
    print(f"  score components (positive-prior, unsupervised): {score_cols}")

    # diagnostic LR-CV ceiling (uses labels => UPPER BOUND, not deployable)
    try:
        from sklearn.linear_model import LogisticRegression
        from sklearn.model_selection import cross_val_predict
        all_z = [c + "_z" for c in _ALL_FEATURES if (c + "_z") in t_scored.columns]
        X = t_scored[all_z].fillna(0).to_numpy()
        y = t_scored["is_anomaly_je"].fillna(False).astype(int).to_numpy()
        if y.sum() >= 10 and (~y.astype(bool)).sum() >= 10:
            proba = cross_val_predict(
                LogisticRegression(max_iter=500, class_weight="balanced"),
                X, y, cv=5, method="predict_proba")[:, 1]
            t_scored["relational_score_lr"] = proba
    except Exception as e:
        print(f"  LR-CV skipped: {e}")

    t_scored.reset_index().to_parquet(a.out)
    print(f"GRAPH_SCORE_DONE -> {a.out}")

    m_unsup = assess(t_scored, "relational_score")
    out_json = a.assess_out or a.out.with_suffix(".assess.json")
    payload = {"unsupervised": m_unsup}
    if "relational_score_lr" in t_scored.columns:
        payload["lr_cv_ceiling"] = assess(t_scored, "relational_score_lr")
    out_json.write_text(json.dumps(payload, indent=2))

    o = m_unsup["overall"]
    print(f"\nRELATIONAL_ARM_VS_IS_ANOMALY (unsupervised, deployable):")
    print(f"  n_pos={m_unsup['n_pos']}  pr_auc={o['pr_auc']:.4f}  roc_auc={o['roc_auc']:.4f}")
    if "lr_cv_ceiling" in payload:
        oc = payload["lr_cv_ceiling"]["overall"]
        print(f"  LR-CV ceiling: pr_auc={oc['pr_auc']:.4f}  roc_auc={oc['roc_auc']:.4f}")
    print("  per-feature ROC (high = useful, < 0.5 = anti-correlated):")
    for f, v in sorted(m_unsup["per_feature"].items(), key=lambda kv: -kv[1]["roc_auc"]):
        print(f"    {f:24s} pr_auc={v['pr_auc']:.3f} roc={v['roc_auc']:.3f}")
    if m_unsup["per_anomaly_type"]:
        print("  per family (type vs normal-JE), top by ROC:")
        for t, mm in sorted(m_unsup["per_anomaly_type"].items(),
                             key=lambda kv: -kv[1]["roc_auc"]):
            print(f"    {t:30s} n={mm['n']:4d} pr_auc={mm['pr_auc']:.3f} roc={mm['roc_auc']:.3f}")


if __name__ == "__main__":
    main()

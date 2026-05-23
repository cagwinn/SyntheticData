"""Decoupled graph-JSON export of the reconstructed account-flow graph (I8).

Plain JSON, no DataSynth → external-substrate dependency. Downstream consumers (a graph
DB, the RustGraph living-graph substrate, an analyst notebook) ingest this file directly.
The schema is intentionally generic — node/edge with typed properties — so it's portable.

Per node (gl_account):
    pagerank_normal, pagerank_test    — PageRank on each aggregate flow graph
    normal_je_count                   — # of normal JEs that touched the account
    in_normal_scc / in_test_scc       — membership in non-trivial SCCs (cycles)
    new_in_test_scc                   — in test cycle but NOT in normal cycle (cycle_novelty
                                        attribution at the node level)

Per edge (account → account):
    weight_normal, weight_test        — aggregate reconstructed-flow amounts
    p_normal, p_test                  — edge probabilities under each graph
    is_new_in_test                    — edge absent from normal, present in test

Per JE (optional, joined from graph_scores.parquet if --scores provided):
    relational_score + every feature column + is_anomaly_je + anomaly_type.

Usage:
    python -m inverse_audit.relational.graph_export \\
        --normal /tmp/iam/normal --test /tmp/iam/test \\
        --scores /tmp/iam/graph_scores.parquet \\
        --out /tmp/iam/account_flow_graph.json
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import pandas as pd

from inverse_audit.relational.ot_flow import reconstruct_per_je
from inverse_audit.relational.graph_scorer import _pagerank, _scc_set


def _aggregate_edges(per: dict) -> dict[tuple[str, str], float]:
    edge_w: dict[tuple[str, str], float] = {}
    for _, (flows, _) in per.items():
        for s, d, w in flows:
            edge_w[(s, d)] = edge_w.get((s, d), 0.0) + w
    return edge_w


def _coerce(v):
    """JSON-safe coercion of pandas/numpy scalars."""
    if pd.isna(v):
        return None
    if isinstance(v, (bool, np.bool_)):
        return bool(v)
    if isinstance(v, (np.integer,)):
        return int(v)
    if isinstance(v, (np.floating,)):
        return float(v)
    if isinstance(v, (int, float)):
        return v
    return str(v)


def export_graph(normal_csv: Path, test_csv: Path,
                 scores_parquet: Path | None, out: Path) -> dict:
    nd = pd.read_csv(normal_csv, low_memory=False)
    td = pd.read_csv(test_csv, low_memory=False)

    n_per = reconstruct_per_je(nd)
    t_per = reconstruct_per_je(td)
    n_ew = _aggregate_edges(n_per); t_ew = _aggregate_edges(t_per)
    n_tot = sum(n_ew.values()) or 1.0
    t_tot = sum(t_ew.values()) or 1.0
    n_pr = _pagerank(n_ew); t_pr = _pagerank(t_ew)
    n_scc = _scc_set(n_ew); t_scc = _scc_set(t_ew)

    # ot_flow returns str-keyed accounts; pandas may yield non-str keys for gl_account.
    # Stringify uniformly so the union/sort works (otherwise: int<->str comparison error).
    acc_count = {str(k): int(v) for k, v in
                 (nd.groupby(["gl_account", "document_id"]).size().reset_index()
                    .groupby("gl_account").size().to_dict()).items()}

    nodes = []
    for n in sorted(set(n_pr) | set(t_pr) | set(acc_count)):
        nodes.append({
            "id": n, "type": "account",
            "props": {
                "pagerank_normal": float(n_pr.get(n, 0.0)),
                "pagerank_test":   float(t_pr.get(n, 0.0)),
                "normal_je_count": int(acc_count.get(n, 0)),
                "in_normal_scc":   bool(n in n_scc),
                "in_test_scc":     bool(n in t_scc),
                "new_in_test_scc": bool(n in t_scc and n not in n_scc),
            },
        })

    edges = []
    for s, d in sorted(set(n_ew) | set(t_ew)):
        nw = n_ew.get((s, d), 0.0); tw = t_ew.get((s, d), 0.0)
        edges.append({
            "src": s, "dst": d,
            "weight_normal": float(nw), "weight_test": float(tw),
            "props": {
                "p_normal":        float(nw / n_tot),
                "p_test":          float(tw / t_tot),
                "is_new_in_test":  bool(nw == 0 and tw > 0),
            },
        })

    je_scores = []
    if scores_parquet and scores_parquet.exists():
        sp = pd.read_parquet(scores_parquet)
        wanted = ("je_id", "relational_score", "edge_surprise_max", "edge_surprise_w",
                  "tp_account_novelty", "account_dormancy_max", "cycle_novelty",
                  "source_cond_edge_surprise_max", "coupling_entropy",
                  "is_anomaly_je", "anomaly_type")
        cols = [c for c in wanted if c in sp.columns]
        for r in sp[cols].itertuples(index=False):
            je_scores.append({c: _coerce(v) for c, v in zip(cols, r)})

    payload = {
        "metadata": {
            "source": "inverse-audit relational arm — account-flow graph",
            "format": "generic graph JSON (nodes + edges + per-je-scores)",
            "version": "1.0",
            "n_nodes": len(nodes),
            "n_edges": len(edges),
            "n_je_scores": len(je_scores),
        },
        "nodes": nodes,
        "edges": edges,
        "per_je_scores": je_scores,
    }
    out.write_text(json.dumps(payload, indent=2))
    print(f"GRAPH_EXPORT_DONE -> {out}  "
          f"({len(nodes)} nodes, {len(edges)} edges, {len(je_scores)} je-scores)")
    return payload


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--normal", type=Path, required=True, help="dir with journal_entries.csv")
    ap.add_argument("--test",   type=Path, required=True, help="dir with journal_entries.csv")
    ap.add_argument("--scores", type=Path, default=None,
                    help="optional graph_scores.parquet for per-JE features")
    ap.add_argument("--out",    type=Path, required=True)
    a = ap.parse_args(argv)
    export_graph(a.normal / "journal_entries.csv", a.test / "journal_entries.csv",
                 a.scores, a.out)


if __name__ == "__main__":
    main()

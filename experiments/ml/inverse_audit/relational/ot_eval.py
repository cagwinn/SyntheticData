"""Rung-2 OT within-JE reconstruction — detector-impact evaluation (GPU-VM ready).

Does the rung-2 LEARNED-cost within-JE flow reconstruction improve anomaly detection over rung-1
(uniform cost), and is the GPU-batched Sinkhorn (ot_gpu) fast + correct at scale? This harness runs
both rungs through the relational scorer and reports, for two OT-derived signals:

  coupling_entropy  — per-JE reconstruction ambiguity (high = the debit<->credit pairing is hard to
                      resolve = structurally unusual). The rung-2 cost should SHARPEN this: a fraud
                      JE pairing accounts that never co-occur in normal 2-line flows gets high cost
                      -> a low-confidence / high-entropy reconstruction.
  relational_score  — the full fit-on-self residual (which already includes coupling_entropy as a
                      feature) built on the rung-1 vs rung-2 reconstructed account-flow graph.

Modes:
  synthetic (--normal/--test GL dirs, labelled): PR-AUC/ROC of each signal vs is_anomaly/is_fraud,
    overall + per relational family (the within-JE-structure families are where rung-2 should help).
    Reports rung-1 vs rung-2 side by side.
  corpus (--parquet, unlabelled): aggregate entropy distribution + flow-graph stats (no row content).

Backend selects the reconstructor: perje = numpy ot_flow (baseline), numpy = ot_gpu batched (CPU),
torch = ot_gpu batched on CUDA (the VM path). Reconstruction wall-time is reported for throughput.

    # local synthetic validation (numpy):
    python -m inverse_audit.relational.ot_eval --normal <gl>/normal --test <gl>/test --backend numpy
    # VM corpus run (GPU):
    python -m inverse_audit.relational.ot_eval --parquet <corpus.parquet> --backend torch --device cuda
"""
from __future__ import annotations

import argparse
import functools
import json
import time
from pathlib import Path

import numpy as np
import pandas as pd

from inverse_audit.relational.graph_scorer import _SCORE_FEATURES, fit_graph_manifold, score_df, z_of
from inverse_audit.relational.ot_cost import _two_line_edges, learn_edge_cost, make_cost_fn
from inverse_audit.relational.ot_gpu import _HAS_TORCH, reconstruct_per_je_gpu


def _recon_fn(backend: str, device: str, size_cap: int = 2048):
    if backend == "perje":
        return None                                          # graph_scorer's default numpy ot_flow
    # size_cap: JEs with >size_cap lines on a side are SKIPPED (pathological batch/allocation JEs —
    # corpus has JEs up to ~100k lines; a 100k² cost matrix is infeasible). The GPU batches everything
    # else as exact-shape buckets (a unique large shape = a size-1 bucket; no padding waste).
    return functools.partial(reconstruct_per_je_gpu, backend=("torch" if backend == "torch" else "numpy"),
                             device=device, size_cap=size_cap)


def _pr(y, s):
    from sklearn.metrics import average_precision_score, roc_auc_score
    y = np.asarray(y, int)
    if y.sum() == 0 or y.sum() == len(y):
        return None, None
    return float(average_precision_score(y, s)), float(roc_auc_score(y, s))


def _score(df, manifold, cost_fn, recon_fn, ref_feats):
    scored = score_df(df, manifold, cost_fn=cost_fn, recon_fn=recon_fn)
    for c in _SCORE_FEATURES:
        if c in scored.columns and c in ref_feats:
            scored[c + "_z"] = z_of(scored[c].to_numpy(), ref_feats[c])
    zc = [c + "_z" for c in _SCORE_FEATURES if (c + "_z") in scored.columns]
    scored["relational_score"] = scored[zc].fillna(0).sum(axis=1)
    return scored


def _labels(gl_dir: Path) -> pd.DataFrame:
    gl = pd.read_csv(gl_dir / "journal_entries.csv", low_memory=False)
    gl["document_id"] = gl["document_id"].astype(str)
    g = gl.groupby("document_id")
    out = pd.DataFrame({"je_id": list(g.groups.keys())})
    out["is_fraud"] = g["is_fraud"].any().values if "is_fraud" in gl else False
    out["is_anomaly"] = g["is_anomaly"].any().values if "is_anomaly" in gl else False
    out["anomaly_type"] = g["anomaly_type"].first().values if "anomaly_type" in gl else None
    return out


def main(argv=None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--normal", type=Path, help="synthetic normal GL dir (journal_entries.csv)")
    ap.add_argument("--test", type=Path, help="synthetic test GL dir (labelled)")
    ap.add_argument("--parquet", type=Path, help="corpus GL parquet (runtime only; aggregate out)")
    ap.add_argument("--backend", default="numpy", choices=("perje", "numpy", "torch"))
    ap.add_argument("--device", default="cuda")
    ap.add_argument("--size-cap", type=int, default=2048,
                    help="skip JEs with >size-cap lines/side (pathological batch JEs; corpus has up to ~100k)")
    ap.add_argument("--out", type=Path, default=Path("/tmp/ot_eval.json"))
    a = ap.parse_args(argv)
    recon = _recon_fn(a.backend, a.device, a.size_cap)
    print(f"backend={a.backend} device={a.device} torch_available={_HAS_TORCH}")

    if a.parquet:                                            # ---------------- corpus (aggregate) -----
        from inverse_audit.corpus_runner import load_canonical
        df = load_canonical(a.parquet)
        edges = _two_line_edges(df)
        cmap, dflt = learn_edge_cost(edges)
        cost_fn = make_cost_fn(cmap, dflt)
        lpje = df.groupby("document_id").size()
        res = {"n_lines": int(len(df)), "n_jes": int(df["document_id"].nunique()),
               "lines_per_je_p50": int(lpje.median()), "lines_per_je_p99": int(lpje.quantile(0.99)),
               "lines_per_je_max": int(lpje.max()), "n_two_line_gt_edges": len(edges), "backend": a.backend}
        for rung, cf in (("rung1_uniform", None), ("rung2_learned", cost_fn)):
            t0 = time.time()
            sc = score_df(df, fit_graph_manifold(df, cost_fn=cf, recon_fn=recon),
                          cost_fn=cf, recon_fn=recon)            # coupling_entropy is a raw feature
            dt = time.time() - t0
            ent = sc["coupling_entropy"].to_numpy() if "coupling_entropy" in sc else np.zeros(len(sc))
            res[rung] = {"recon_score_seconds": round(dt, 2),
                         "coupling_entropy_pctiles": {q: round(float(np.percentile(ent, q)), 4)
                                                      for q in (50, 90, 99)},
                         "mean_entropy": round(float(ent.mean()), 4)}
            print(f"[corpus {rung}] {dt:.1f}s  mean_entropy={ent.mean():.4f}  "
                  f"entropy_p99={np.percentile(ent,99):.4f}")
        a.out.write_text(json.dumps(res, indent=2))
        print(f"-> {a.out}")
        return

    # ---------------------------------------------------------------- synthetic (labelled) -----------
    assert a.normal and a.test, "synthetic mode needs --normal and --test"
    nd = pd.read_csv(a.normal / "journal_entries.csv", low_memory=False)
    td = pd.read_csv(a.test / "journal_entries.csv", low_memory=False)
    for d in (nd, td):
        for c in ("debit_amount", "credit_amount"):
            d[c] = pd.to_numeric(d[c], errors="coerce").fillna(0.0)
    edges = _two_line_edges(nd)
    cmap, dflt = learn_edge_cost(edges)
    cost_fn = make_cost_fn(cmap, dflt)
    lab = _labels(a.test)
    print(f"GL: normal {nd['document_id'].nunique()} JEs / test {td['document_id'].nunique()} JEs; "
          f"2-line GT edges {len(edges)} ({len(cmap)} distinct); "
          f"is_anomaly JEs {int(lab['is_anomaly'].sum())} is_fraud {int(lab['is_fraud'].sum())}")

    result = {"backend": a.backend, "n_gt_edges": len(edges), "rungs": {}}
    timing = {}
    scored_by_rung = {}
    for rung, cf in (("rung1_uniform", None), ("rung2_learned", cost_fn)):
        t0 = time.time()
        man = fit_graph_manifold(nd, cost_fn=cf, recon_fn=recon)
        n_sc = _score(nd, man, cf, recon, {})                 # self-ref features for z baseline
        ref_feats = {c: n_sc[c].to_numpy() for c in _SCORE_FEATURES if c in n_sc}
        sc = _score(td, man, cf, recon, ref_feats)
        timing[rung] = round(time.time() - t0, 2)
        sc = sc.reset_index(drop=True) if "je_id" in sc.columns else sc.reset_index()
        sc["je_id"] = sc["je_id"].astype(str)
        # score_df may already carry labels; only merge the ones it's missing (avoid _x/_y collisions)
        need = [c for c in ("is_fraud", "is_anomaly", "anomaly_type") if c not in sc.columns]
        if need:
            sc = sc.merge(lab[["je_id"] + need], on="je_id", how="inner")
        scored_by_rung[rung] = sc
        ent = sc["coupling_entropy"].to_numpy() if "coupling_entropy" in sc else np.zeros(len(sc))
        rel = sc["relational_score"].to_numpy()
        is_a = sc["is_anomaly"].fillna(False).to_numpy(); is_f = sc["is_fraud"].fillna(False).to_numpy()
        ent_pr = _pr(is_a, ent); rel_pr = _pr(is_a, rel)
        # per relational family (anomaly_type vs clean)
        clean = ~(is_a | is_f)
        fam = {}
        for t in pd.Series(sc["anomaly_type"]).dropna().unique():
            ist = (sc["anomaly_type"] == t).to_numpy()
            if ist.sum() < 8:
                continue
            mask = ist | clean
            fam[str(t)] = {"n": int(ist.sum()),
                           "entropy_roc": (_pr(ist[mask], ent[mask])[1]),
                           "relational_roc": (_pr(ist[mask], rel[mask])[1])}
        result["rungs"][rung] = {
            "recon_score_seconds": timing[rung],
            "coupling_entropy_vs_anomaly": {"pr_auc": ent_pr[0], "roc": ent_pr[1]},
            "relational_vs_anomaly": {"pr_auc": rel_pr[0], "roc": rel_pr[1]},
            "per_family": fam}
        print(f"[{rung}] {timing[rung]}s | entropy PR-AUC={ent_pr[0]:.3f} ROC={ent_pr[1]:.3f} | "
              f"relational PR-AUC={rel_pr[0]:.3f} ROC={rel_pr[1]:.3f}")

    # rung-1 vs rung-2 delta, overall + the families where rung-2 helps most
    r1, r2 = result["rungs"]["rung1_uniform"], result["rungs"]["rung2_learned"]
    d_ent = (r2["coupling_entropy_vs_anomaly"]["pr_auc"] or 0) - (r1["coupling_entropy_vs_anomaly"]["pr_auc"] or 0)
    d_rel = (r2["relational_vs_anomaly"]["pr_auc"] or 0) - (r1["relational_vs_anomaly"]["pr_auc"] or 0)
    fam_deltas = []
    for t, e2 in r2["per_family"].items():
        e1 = r1["per_family"].get(t, {})
        if e2.get("relational_roc") is not None and e1.get("relational_roc") is not None:
            fam_deltas.append((t, e2["n"], round(e2["relational_roc"] - e1["relational_roc"], 3)))
    fam_deltas.sort(key=lambda x: -x[2])
    result["rung2_minus_rung1"] = {"entropy_pr_auc": round(d_ent, 4), "relational_pr_auc": round(d_rel, 4),
                                   "top_family_relational_roc_gains": fam_deltas[:8]}
    a.out.write_text(json.dumps(result, indent=2))
    print(f"\nRUNG2_vs_RUNG1: Δentropy PR-AUC={d_ent:+.4f}  Δrelational PR-AUC={d_rel:+.4f}")
    print(f"top family ROC gains (rung2−rung1): {fam_deltas[:6]}")
    print(f"recon+score time: rung1={timing['rung1_uniform']}s rung2={timing['rung2_learned']}s (backend={a.backend})")
    print(f"-> {a.out}")


if __name__ == "__main__":
    main()

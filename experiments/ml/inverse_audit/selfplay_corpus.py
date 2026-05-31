"""P3 corpus transfer — does the label-free residual + self-play memory hold on CORPUS data?

The thesis (FINDINGS §12/§16): the GLOBAL SBI arm collapses out-of-distribution on corpus, but the
label-free relational fit-on-self residual TRANSFERS. §36 proved the counterparty-node invariant
neutralises a relocating adversary on a SYNTHETIC normal background. This module runs that same
self-play game with the normal background replaced by CORPUS data — the realistic manifold that was
OOD for the global arm — using the corpus's own Period column as the periods.

  1. Load a corpus GL (canonical loader; path is a runtime arg, never committed). Fit the relational
     manifold fit-on-self and score every JE -> `relational_score` (the label-free residual; this is
     the corpus analogue of synthetic `unified_z`).
  2. Split JEs by Period -> periods. Each period the adversary INJECTS counterparty fraud {B_t, C} at
     CORPUS accounts (residual-NORMAL scores drawn from the corpus residual band -> catchable only via
     memory on the pinned C; B relocates). Three arms: node memory (sees C), blind (C excluded), floor.
  3. Measure recall on the injected adversary across corpus periods. If node >> blind ≈ floor, the
     residual + self-play memory machinery TRANSFERS to a corpus-realistic background.

Privacy: corpus path is a runtime argument only; outputs are aggregate (per-period recalls + the
residual distribution); no JE content / account labels / amounts leave this module.

    python -m inverse_audit.selfplay_corpus --parquet <path> --out /tmp/corpus_selfplay

Outputs: {out}/corpus_selfplay.json
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import pandas as pd

from inverse_audit.corpus_runner import CORPUS_COLMAP
from inverse_audit.relational.graph_scorer import _SCORE_FEATURES, fit_graph_manifold, score_df, z_of
from inverse_audit.selfplay import Auditor
from inverse_audit.selfplay_adaptive import AdaptiveAdversary
from inverse_audit.selfplay_counterparty import _inject, _idf


def load_corpus_with_period(parquet: Path) -> pd.DataFrame:
    """Canonical loader that also retains Period (for self-play periods)."""
    df = pd.read_parquet(parquet).rename(columns=CORPUS_COLMAP)
    if "Functional Amount" not in df.columns:
        raise ValueError(f"{parquet.name}: not a GL parquet (no 'Functional Amount').")
    fa = pd.to_numeric(df["Functional Amount"], errors="coerce").fillna(0.0)
    df["debit_amount"] = fa.where(fa > 0, 0.0)
    df["credit_amount"] = (-fa).where(fa < 0, 0.0)
    df["document_id"] = df["document_id"].astype(str)
    df["gl_account"] = df["gl_account"].astype(str)
    if "period" not in df.columns:
        df["period"] = 0
    keep = ["document_id", "gl_account", "debit_amount", "credit_amount", "source",
            "trading_partner", "period"]
    return df[[c for c in keep if c in df.columns]].copy()


def main(argv=None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--parquet", type=Path, required=True, help="corpus GL parquet (runtime only)")
    ap.add_argument("--out", type=Path, default=Path("/tmp/corpus_selfplay"))
    ap.add_argument("--budget", type=float, default=0.05)
    ap.add_argument("--beta", type=float, default=3.0)
    ap.add_argument("--lam", type=float, default=0.5)
    ap.add_argument("--active", type=int, default=3, help="booking accounts B active per period")
    ap.add_argument("--n-fraud", type=int, default=0,
                    help="fixed counterparty fraud JEs per period (0 = scale to --fraud-frac of the period)")
    ap.add_argument("--fraud-frac", type=float, default=0.02,
                    help="counterparty fraud as a fraction of period JEs (scale-relative across clients)")
    ap.add_argument("--cp-volume", type=int, default=120, help="target volume for the pinned counterparty C")
    ap.add_argument("--sloppy-frac", type=float, default=0.15)
    a = ap.parse_args(argv)
    a.out.mkdir(parents=True, exist_ok=True)

    df = load_corpus_with_period(a.parquet)
    n_lines, n_jes = len(df), df["document_id"].nunique()
    # --- label-free residual, fit-on-self on the corpus manifold ---
    manifold = fit_graph_manifold(df)
    scored = score_df(df, manifold)
    for c in _SCORE_FEATURES:
        if c in scored.columns:
            scored[c + "_z"] = z_of(scored[c].to_numpy(), scored[c].to_numpy())
    zc = [c + "_z" for c in _SCORE_FEATURES if (c + "_z") in scored.columns]
    scored["relational_score"] = scored[zc].fillna(0).sum(axis=1)
    scored["je_id"] = scored["je_id"].astype(str) if "je_id" in scored.columns else scored.index.astype(str)
    rs = scored["relational_score"].to_numpy()
    print(f"corpus: {n_lines:,} lines / {n_jes:,} JEs | manifold edges={len(manifold['edge_p']):,} "
          f"nodes={len(manifold['pagerank']):,} scc={len(manifold['normal_scc_set']):,}")
    print(f"label-free residual: p50={np.percentile(rs,50):.2f} p95={np.percentile(rs,95):.2f} "
          f"p99={np.percentile(rs,99):.2f} max={rs.max():.2f}  (non-degenerate: {rs.std()>0.1})")

    # per-JE frame the self-play machinery expects (unified_z := corpus residual)
    je_score = dict(zip(scored["je_id"], scored["relational_score"]))
    je_period = df.groupby("document_id")["period"].first().to_dict()
    je_acct = {k: list(v) for k, v in df.groupby("document_id")["gl_account"]}
    df_acct: dict = {}
    for accts in je_acct.values():
        for ac in set(accts):
            df_acct[ac] = df_acct.get(ac, 0) + 1
    def _pkey(p):                              # numeric period order when possible (not string 1,10,11,2)
        try:
            return (0, float(p))
        except (TypeError, ValueError):
            return (1, str(p))
    periods = sorted({p for p in je_period.values() if pd.notna(p)}, key=_pkey)

    # pick a distinct counterparty C + booking pool B from CORPUS accounts (mid-volume)
    mid = sorted(((c, ac) for ac, c in df_acct.items() if 20 <= c <= 600), reverse=True)
    C = min(df_acct, key=lambda ac: abs(df_acct[ac] - a.cp_volume))
    B_pool = [ac for _, ac in mid if ac != C][:12]
    if len(B_pool) < a.active:
        print(f"too few mid-volume corpus accounts ({len(B_pool)}) for the booking pool; aborting.")
        return

    def mk(mem):
        return Auditor("a", use_memory=mem, use_confirmation=mem, beta=a.beta, lam=a.lam)
    arms = {"adaptive_node": {"aud": mk(True), "blind": False},
            "adaptive_blind": {"aud": mk(True), "blind": True},
            "floor": {"aud": mk(False), "blind": False}}
    for arm in arms.values():
        arm["adv"] = AdaptiveAdversary(B_pool, a.active, 0.0, evasion=True, df=df_acct, strategy="highvol")

    history = []
    for ri, p in enumerate(periods):
        je_ids = [k for k, pp in je_period.items() if pp == p]
        if len(je_ids) < 50:
            continue
        jp = pd.DataFrame({"je_id": je_ids,
                           "unified_z": [je_score.get(k, 0.0) for k in je_ids],
                           "is_fraud": False, "is_anomaly": False})
        n_fraud = a.n_fraud if a.n_fraud > 0 else max(10, int(a.fraud_frac * len(jp)))
        rec = {"period": int(p) if isinstance(p, (int, np.integer)) else str(p),
               "n_je": len(jp), "n_fraud": n_fraud, "arms": {}}
        for name, arm in arms.items():
            adv, aud = arm["adv"], arm["aud"]
            j2, je_acct2, is_adv = _inject(jp, je_acct, adv.active, C, n_fraud, ri, a.sloppy_frac)
            key_map = je_acct2
            if arm["blind"]:
                key_map = {je: [x for x in ks if x != C] for je, ks in je_acct2.items()}
            idf = _idf(key_map)
            is_target = (j2["is_fraud"] | j2["is_anomaly"]).to_numpy()
            K = max(1, int(a.budget * len(j2)))
            s = aud.score(j2, key_map)
            inv = np.zeros(len(j2), dtype=bool)
            inv[np.argsort(-s)[:K]] = True
            recall = int((inv & is_adv).sum()) / max(1, int(is_adv.sum()))
            aud.update(j2, key_map, inv, is_target, idf)
            adv.observe_and_relocate(aud, lambda ac, _a=aud: _a.prior.get(ac, 0.0))
            rec["arms"][name] = {"recall_adv": round(recall, 4), "prior_on_C": round(aud.prior.get(C, 0.0), 2)}
        history.append(rec)
        g = lambda n: rec["arms"][n]["recall_adv"]
        print(f"[period {rec['period']}] n_je={len(jp)} | recall on counterparty-fraud: "
              f"node={g('adaptive_node'):.3f}  BLIND={g('adaptive_blind'):.3f}  floor={g('floor'):.3f}")

    def mr(n):
        v = [h["arms"][n]["recall_adv"] for h in history[1:]]
        return float(np.mean(v)) if v else 0.0
    node_, blind_, floor_ = mr("adaptive_node"), mr("adaptive_blind"), mr("floor")
    transfers = node_ - max(blind_, floor_) > 0.1 and node_ > 0.4
    summary = {
        "n_lines": int(n_lines), "n_jes": int(n_jes), "n_periods_used": len(history),
        "residual_pctiles": {q: round(float(np.percentile(rs, q)), 3) for q in (50, 90, 95, 99)},
        "residual_nondegenerate": bool(rs.std() > 0.1),
        "counterparty_volume": int(df_acct.get(C, 0)),
        "mean_recall_periods_ge1": {"adaptive_node": round(node_, 4),
                                    "adaptive_blind": round(blind_, 4), "floor": round(floor_, 4)},
        "label_free_arm_transfers": bool(transfers),
        "verdict": ("label-free residual + self-play memory TRANSFERS to the corpus background"
                    if transfers else "transfer weak/failed on this corpus GL"),
    }
    (a.out / "corpus_selfplay.json").write_text(json.dumps({**summary, "periods": history}, indent=2))
    print(f"\nCORPUS_SELFPLAY_DONE periods_used={len(history)}  C_volume={df_acct.get(C,0)}")
    print(f"  mean recall on counterparty-fraud (periods>=1): node={node_:.3f}  blind={blind_:.3f}  floor={floor_:.3f}")
    print(f"  -> {summary['verdict']}\n  -> {a.out / 'corpus_selfplay.json'}")


if __name__ == "__main__":
    main()

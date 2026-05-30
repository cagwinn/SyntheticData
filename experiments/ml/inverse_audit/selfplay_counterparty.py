"""P3.3b — the counterparty-node invariant: the fraud-specific relocation counter.

P3.3 found class-level "relocation-invariant" memory too coarse, and that PERSISTENCE (slow-decay
node memory) only partially corners a relocating adversary. The tightest counter is a fraud-SPECIFIC
invariant: a colluding COUNTERPARTY. A realistic fraud has a fixed beneficiary — the money always
moves to/from the same account C — even as the fraudster varies the OTHER leg (the booking account
B) to avoid a pattern. So every fraud JE shares the node C regardless of where B moves.

We model it directly (the relabel-overlay couldn't): each period the adversary INJECTS synthetic
fraud JEs, each a residual-NORMAL pairing {B_t, C} — B_t relocates under audit pressure (highvol),
C is pinned. The injected JEs get residual-normal scores (drawn from the normal-JE score band), so
the residual is blind by construction — only memory can catch them.

Arms (all face the counterparty-pinned fraud; B relocates via highvol):
  static_node    : B fixed,      node memory (sees all accounts incl C)   -> ceiling.
  adaptive_node  : B relocates,  node memory (sees C)                     -> does C neutralise it?
  adaptive_blind : B relocates,  node memory but C EXCLUDED from the keys -> ablation: prove C is the handle.
  floor          : B relocates,  memoryless.

Headline: with a pinned counterparty, does ORDINARY node memory defeat relocation (adaptive_node ->
the ceiling) because the fixed endpoint C accumulates prior — and does removing C (adaptive_blind)
restore the evasion? Also a sweep over C's VOLUME: a distinct C is caught; a ubiquitous C dilutes.

    python -m inverse_audit.selfplay_counterparty --root /tmp/sp_inv --rounds 12 --skip-generate --skip-scoring

Outputs: {root}/self_play_counterparty.json
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import pandas as pd

from inverse_audit.generate import base_config, generate, set_fraud_rate
from inverse_audit.selfplay import Auditor, _je_accounts, _lean, _score_round, _with_anomaly
from inverse_audit.selfplay_adaptive import AdaptiveAdversary
from inverse_audit.unified_score import _metrics


def _inject(j: pd.DataFrame, je_acct: dict, active_B: list[str], C: str, n_fraud: int,
            rnd: int, sloppy_frac: float = 0.15) -> tuple[pd.DataFrame, dict, np.ndarray]:
    """Add n_fraud synthetic fraud JEs, each pairing a booking account B_t with the pinned
    counterparty C. A small `sloppy_frac` are residual-DETECTABLE (high band — these bootstrap the
    auditor's attention on C, as a few careless entries or a tip would); the rest are residual-
    INVISIBLE (normal band — catchable only via memory on the pinned C). Returns (j2, je_acct2, is_adv)."""
    base = np.sort(j.loc[~(j["is_fraud"] | j["is_anomaly"]), "unified_z"].to_numpy())
    n_sloppy = int(round(sloppy_frac * n_fraud))
    hi = np.quantile(base, np.linspace(0.95, 0.999, max(1, n_sloppy))) if n_sloppy else np.array([])
    lo = np.quantile(base, np.linspace(0.30, 0.60, n_fraud - n_sloppy))   # residual-invisible careful fraud
    scores = np.concatenate([hi, lo])
    je_acct2 = dict(je_acct)
    rows = []
    per = max(1, n_fraud // len(active_B))
    k = 0
    for bi, B in enumerate(active_B):
        cnt = per if bi < len(active_B) - 1 else n_fraud - per * (len(active_B) - 1)
        for _ in range(cnt):
            jid = f"adv_{rnd}_{k}"
            rows.append({"je_id": jid, "unified_z": float(scores[k]), "is_fraud": True, "is_anomaly": False})
            je_acct2[jid] = [B, C]
            k += 1
    inj = pd.DataFrame(rows)
    cols = ["je_id", "unified_z", "is_fraud", "is_anomaly"]
    j2 = pd.concat([j[cols], inj[cols]], ignore_index=True)
    is_adv = j2["je_id"].astype(str).str.startswith("adv_").to_numpy()
    return j2, je_acct2, is_adv


def _idf(key_map: dict) -> dict:
    df: dict = {}
    for ks in key_map.values():
        for k in set(ks):
            df[k] = df.get(k, 0) + 1
    n = max(1, len(key_map))
    return {k: float(np.log(n / (c + 1))) for k, c in df.items()}


def main(argv=None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=Path("/tmp/sp_inv"))
    ap.add_argument("--rounds", type=int, default=12)
    ap.add_argument("--industry", default="manufacturing")
    ap.add_argument("--complexity", default="small")
    ap.add_argument("--fraud-rate", type=float, default=0.04)
    ap.add_argument("--anomaly-rate", type=float, default=0.04)
    ap.add_argument("--seed", type=int, default=20260530)
    ap.add_argument("--budget", type=float, default=0.05)
    ap.add_argument("--beta", type=float, default=3.0)
    ap.add_argument("--lam", type=float, default=0.5)
    ap.add_argument("--active", type=int, default=3, help="booking accounts B active per period")
    ap.add_argument("--n-fraud", type=int, default=150, help="counterparty fraud JEs injected per period")
    ap.add_argument("--cp-volume", type=int, default=120,
                    help="target document-frequency for the pinned counterparty C (distinct vs ubiquitous)")
    ap.add_argument("--sloppy-frac", type=float, default=0.15,
                    help="fraction of counterparty fraud that is residual-detectable (bootstraps attention on C)")
    ap.add_argument("--skip-generate", action="store_true")
    ap.add_argument("--skip-scoring", action="store_true")
    a = ap.parse_args(argv)
    a.root.mkdir(parents=True, exist_ok=True)

    def mk(mem):
        return Auditor("a", use_memory=mem, use_confirmation=mem, beta=a.beta, lam=a.lam)
    arms = {
        "static_node":   {"aud": mk(True),  "evasion": False, "blind": False, "mem": True},
        "adaptive_node": {"aud": mk(True),  "evasion": True,  "blind": False, "mem": True},
        "adaptive_blind":{"aud": mk(True),  "evasion": True,  "blind": True,  "mem": True},
        "floor":         {"aud": mk(False), "evasion": True,  "blind": False, "mem": False},
    }
    B_pool: list[str] = []
    C = None
    history: list[dict] = []

    for r in range(a.rounds):
        rdir = a.root / f"round_{r}"
        rdir.mkdir(parents=True, exist_ok=True)
        if not a.skip_generate:
            base = _lean(base_config(a.industry, a.complexity, a.seed + r))
            generate(set_fraud_rate(base, 0.0), rdir / "normal")
            generate(_with_anomaly(set_fraud_rate(base, a.fraud_rate), a.anomaly_rate), rdir / "test")

        j = _score_round(rdir, reuse=a.skip_scoring)
        je_acct = _je_accounts(rdir / "test")
        df_acct: dict = {}
        for accts in je_acct.values():
            for ac in set(accts):
                df_acct[ac] = df_acct.get(ac, 0) + 1

        if not B_pool:
            mid = sorted(((c, ac) for ac, c in df_acct.items() if 20 <= c <= 400), reverse=True)
            # pin the counterparty C = account whose volume is closest to --cp-volume (distinct, not ubiquitous)
            C = min(df_acct, key=lambda ac: abs(df_acct[ac] - a.cp_volume))
            B_pool = [ac for _, ac in mid if ac != C][:12]
            for arm in arms.values():
                arm["adv"] = AdaptiveAdversary(B_pool, a.active, 0.0, arm["evasion"],
                                               df=df_acct, strategy="highvol")

        rec: dict = {"round": r, "C": C, "C_volume": df_acct.get(C, 0), "arms": {}}
        for name, arm in arms.items():
            adv, aud = arm["adv"], arm["aud"]
            j2, je_acct2, is_adv = _inject(j, je_acct, adv.active, C, a.n_fraud, r, a.sloppy_frac)
            # FOCUSED MEASUREMENT: the counterparty fraud is residual-INVISIBLE and hides in the
            # NORMAL population; the engine's residual-detectable anomalies are a separate problem
            # (the density/relational arms already catch them) that would otherwise saturate the
            # budget. Rank the counterparty fraud against the normal background it hides in.
            engine_anom = (j2["is_fraud"] | j2["is_anomaly"]).to_numpy() & (~is_adv)
            keep = ~engine_anom
            j2 = j2[keep].reset_index(drop=True)
            is_adv = is_adv[keep]
            key_map = je_acct2
            if arm["blind"]:                              # counterparty-blind ablation: drop C from every key set
                key_map = {je: [x for x in ks if x != C] for je, ks in je_acct2.items()}
            idf = _idf(key_map)
            is_target = (j2["is_fraud"] | j2["is_anomaly"]).to_numpy()
            K = max(1, int(a.budget * len(j2)))
            s = aud.score(j2, key_map)
            inv = np.zeros(len(j2), dtype=bool)
            inv[np.argsort(-s)[:K]] = True
            recall_adv = int((inv & is_adv).sum()) / max(1, int(is_adv.sum()))
            aud.update(j2, key_map, inv, is_target, idf)
            heat = (lambda ac, _a=aud: _a.prior.get(ac, 0.0))
            adv.observe_and_relocate(aud, heat)
            rec["arms"][name] = {"recall_adv": round(recall_adv, 4),
                                 "prior_on_C": round(aud.prior.get(C, 0.0), 3),
                                 "active_B": list(adv.active), "reloc": adv.relocations}
        history.append(rec)
        g = lambda n: rec["arms"][n]["recall_adv"]
        print(f"[round {r}] C={C}(vol {df_acct.get(C,0)}) | recall on counterparty-fraud: "
              f"static/node={g('static_node'):.3f}  adaptive/node={g('adaptive_node'):.3f}  "
              f"adaptive/BLIND={g('adaptive_blind'):.3f}  floor={g('floor'):.3f}  "
              f"| prior(C) node={rec['arms']['adaptive_node']['prior_on_C']:.2f}")

    def mr(name):
        return float(np.mean([h["arms"][name]["recall_adv"] for h in history[1:]])) if len(history) > 1 else 0.0
    ceil_, node_, blind_, floor_ = (mr(n) for n in ("static_node", "adaptive_node", "adaptive_blind", "floor"))
    neutralised = node_ - blind_                      # how much the counterparty endpoint recovers
    gap_remaining = ceil_ - node_                     # residual evasion gap with C visible
    verdict = ("counterparty-node memory NEUTRALISES relocation (node ≈ static ceiling, ≫ blind)"
               if gap_remaining < 0.02 and neutralised > 0.03 else
               "counterparty helps but doesn't fully neutralise" if neutralised > 0.01 else
               "counterparty does NOT help (C too diluted?)")
    summary = {
        "config": {k: getattr(a, k) for k in ("rounds", "industry", "complexity", "budget", "beta",
                                              "lam", "active", "n_fraud", "cp_volume", "seed")},
        "counterparty_C": C, "C_volume": history[-1]["C_volume"],
        "mean_recall_rounds_ge1": {"static_node_ceiling": round(ceil_, 4), "adaptive_node": round(node_, 4),
                                   "adaptive_blind_ablation": round(blind_, 4), "floor": round(floor_, 4)},
        "neutralised_by_counterparty": round(neutralised, 4),
        "residual_evasion_gap_with_C": round(gap_remaining, 4),
        "final_prior_on_C_node": round(history[-1]["arms"]["adaptive_node"]["prior_on_C"], 3),
        "verdict": verdict,
    }
    out = {**summary, "rounds": history}
    (a.root / "self_play_counterparty.json").write_text(json.dumps(out, indent=2))
    print(f"\nSELFPLAY_COUNTERPARTY_DONE rounds={a.rounds}  C={C} (vol {history[-1]['C_volume']})")
    print(f"  mean recall on counterparty-fraud (r>=1): ceiling(static/node)={ceil_:.3f}  "
          f"adaptive/node={node_:.3f}  adaptive/BLIND={blind_:.3f}  floor={floor_:.3f}")
    print(f"  neutralised by counterparty endpoint = {neutralised:+.4f}   residual gap (ceiling-node) = {gap_remaining:+.4f}")
    print(f"  -> {verdict}\n  -> {a.root / 'self_play_counterparty.json'}")


if __name__ == "__main__":
    main()

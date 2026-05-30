"""P3.3 — relocation-invariant audit memory: neutralising the adaptive adversary.

P3.2 found the adversary's one winning move: flee to un-watched HIGH-VOLUME accounts (blend in
volume while dodging the auditor's account-code memory). The counter, motivated game-theoretically:
key the carried belief on a signature that SURVIVES relocation — the account's CLASS (the fraud's
modus operandi) rather than the account code. When the adversary switches booking accounts WITHIN
its class (its MO), the class is unchanged, so class-keyed memory carries straight over; and an
adversary facing class memory sees its whole class lit up — there is no fresh in-class hideout.

Four arms on identical worlds, adversary hiding in a single account CLASS (its MO), highvol evasion:
  static_vs_node      : adversary stays, node (account-code) memory  -> ceiling.
  adaptive_vs_node    : adversary flees, node memory                 -> P3.2's evadable auditor.
  adaptive_vs_archetype: adversary flees, CLASS (relocation-invariant) memory -> the counter.
  adaptive_vs_memoryless: floor.

Headline: does class memory recover the evasion gap (adaptive_vs_archetype recall -> the static
ceiling)? And at what PRECISION cost (class memory is coarser — it boosts the whole class)? We
report recall@budget on the bad actor AND its PR-AUC (precision proxy) for node vs class memory.

The Auditor machinery is generic over its key-map (belief keyed by any string), so node memory
passes je->[accounts] and class memory passes je->[account_class per account] — same code, different
granularity. The adversary's heat sensor matches the auditor's granularity (class memory => the
adversary reads class-level heat, so within-class flight is futile).

    python -m inverse_audit.selfplay_invariant --root /tmp/sp_adapt --rounds 8 --skip-generate --skip-scoring

Outputs: {root}/self_play_invariant.json
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


def _je_classes(test_dir: Path, col: str = "account_class") -> tuple[dict[str, list[str]], dict[str, str]]:
    """document_id -> [grain per touched account];  gl_account -> grain. `col` selects the
    relocation-invariant granularity: account_class (coarse) or account_sub_class (tighter)."""
    gl = pd.read_csv(test_dir / "journal_entries.csv", low_memory=False)
    gl["document_id"] = gl["document_id"].astype(str)
    gl["gl_account"] = gl["gl_account"].astype(str)
    gl[col] = gl.get(col).astype(str).fillna("NA").replace("nan", "NA")
    je_grain = {k: list(v) for k, v in gl.groupby("document_id")[col]}
    acct_grain = gl.groupby("gl_account")[col].first().to_dict()
    return je_grain, acct_grain


def _idf(key_map: dict[str, list[str]]) -> dict[str, float]:
    df: dict[str, int] = {}
    for keys in key_map.values():
        for k in set(keys):
            df[k] = df.get(k, 0) + 1
    n = max(1, len(key_map))
    return {k: float(np.log(n / (c + 1))) for k, c in df.items()}


def main(argv=None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=Path("/tmp/sp_adapt"))
    ap.add_argument("--rounds", type=int, default=8)
    ap.add_argument("--industry", default="manufacturing")
    ap.add_argument("--complexity", default="small")
    ap.add_argument("--fraud-rate", type=float, default=0.04)
    ap.add_argument("--anomaly-rate", type=float, default=0.04)
    ap.add_argument("--seed", type=int, default=20260530)
    ap.add_argument("--budget", type=float, default=0.05)
    ap.add_argument("--beta", type=float, default=3.0)
    ap.add_argument("--lam", type=float, default=0.5)
    ap.add_argument("--active", type=int, default=3)
    ap.add_argument("--adv-rate", type=float, default=0.3)
    ap.add_argument("--pool-class", default=None,
                    help="restrict the hiding pool to one grain value (the adversary's MO); "
                         "default = the grain with the most mid-volume accounts")
    ap.add_argument("--grain", default="account_sub_class",
                    choices=("account_class", "account_sub_class"),
                    help="relocation-invariant memory granularity (sub_class is tighter -> less dilution)")
    ap.add_argument("--strategy", default="highvol", choices=("coldest", "highvol"))
    ap.add_argument("--skip-generate", action="store_true")
    ap.add_argument("--skip-scoring", action="store_true")
    a = ap.parse_args(argv)
    a.root.mkdir(parents=True, exist_ok=True)

    def mk(mem):
        return Auditor("a", use_memory=mem, use_confirmation=mem, beta=a.beta, lam=a.lam)
    # arm: (auditor, keying 'node'/'class'/'none', adversary evasion on/off)
    arms = {
        "static_vs_node":       {"aud": mk(True),  "keying": "node",  "evasion": False},
        "adaptive_vs_node":     {"aud": mk(True),  "keying": "node",  "evasion": True},
        "adaptive_vs_archetype":{"aud": mk(True),  "keying": "class", "evasion": True},
        "adaptive_vs_memoryless":{"aud": mk(False), "keying": "node", "evasion": True},
    }
    pool: list[str] = []
    pool_class = a.pool_class
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
        je_class, acct_class = _je_classes(rdir / "test", a.grain)
        idf_node, idf_class = _idf(je_acct), _idf(je_class)
        df_acct: dict[str, int] = {}
        for accts in je_acct.values():
            for acct in set(accts):
                df_acct[acct] = df_acct.get(acct, 0) + 1
        engine_target = (j["is_fraud"] | j["is_anomaly"]).to_numpy()
        K = max(1, int(a.budget * len(j)))

        if not pool:
            mid = [(c, ac) for ac, c in df_acct.items() if 20 <= c <= 400]
            if pool_class is None:                  # auto-pick the class with the most mid-vol accounts
                from collections import Counter
                cls_count = Counter(acct_class.get(ac, "NA") for _, ac in mid)
                pool_class = cls_count.most_common(1)[0][0]
            inclass = sorted(((c, ac) for c, ac in mid if acct_class.get(ac, "NA") == pool_class), reverse=True)
            pool = [ac for _, ac in inclass]
            for arm in arms.values():
                arm["adv"] = AdaptiveAdversary(pool, a.active, a.adv_rate, arm["evasion"],
                                               df=df_acct, strategy=a.strategy)

        rec: dict = {"round": r, "pool_class": pool_class, "pool_size": len(pool), "K": K, "arms": {}}
        for name, arm in arms.items():
            adv, aud, keying = arm["adv"], arm["aud"], arm["keying"]
            key_map = je_class if keying == "class" else je_acct
            idf = idf_class if keying == "class" else idf_node
            is_adv = adv.place_fraud(j, je_acct)        # placement is account-based (the active set)
            is_target = engine_target | is_adv
            s = aud.score(j, key_map)
            inv = np.zeros(len(j), dtype=bool)
            inv[np.argsort(-s)[:K]] = True
            n_adv = int(is_adv.sum())
            recall_adv = int((inv & is_adv).sum()) / max(1, n_adv)
            pr = _metrics(is_adv, s)                    # bad-actor PR-AUC = precision proxy
            aud.update(j, key_map, inv, is_target, idf)
            # the adversary reads heat at the AUDITOR'S granularity (class memory => class heat)
            heat_fn = ((lambda ac, _a=aud: _a.prior.get(acct_class.get(ac, "NA"), 0.0))
                       if keying == "class" else None)
            adv.observe_and_relocate(aud, heat_fn)
            rec["arms"][name] = {"recall_adv": round(recall_adv, 4), "n_adv": n_adv,
                                 "pr_auc_adv": (None if pr["pr_auc"] is None else round(pr["pr_auc"], 4)),
                                 "active": list(adv.active), "relocations_total": adv.relocations}
        history.append(rec)
        g = lambda n: rec["arms"][n]["recall_adv"]
        print(f"[round {r}] cls={pool_class} | recall on bad actor: static/node={g('static_vs_node'):.3f}  "
              f"adaptive/node={g('adaptive_vs_node'):.3f}  adaptive/ARCHETYPE={g('adaptive_vs_archetype'):.3f}  "
              f"floor={g('adaptive_vs_memoryless'):.3f}")

    def mr(name, key="recall_adv"):
        vals = [h["arms"][name][key] for h in history[1:] if h["arms"][name][key] is not None]
        return float(np.mean(vals)) if vals else 0.0
    ceil_, node_, arch_, floor_ = (mr(n) for n in
        ("static_vs_node", "adaptive_vs_node", "adaptive_vs_archetype", "adaptive_vs_memoryless"))
    evasion_gap = ceil_ - node_                          # how much node memory lost to evasion
    recovered = arch_ - node_                            # how much class memory recovered
    frac = (recovered / evasion_gap) if abs(evasion_gap) > 1e-6 else 0.0
    node_prec, arch_prec = mr("adaptive_vs_node", "pr_auc_adv"), mr("adaptive_vs_archetype", "pr_auc_adv")

    verdict = ("relocation-invariant memory NEUTRALISES the evasion (recovers the gap)" if recovered > 0.5 * max(evasion_gap, 1e-6) and recovered > 0.005 else
               "class memory PARTIALLY recovers the evasion gap" if recovered > 0.005 else
               "class memory does NOT recover the gap (too coarse on this DGP)")
    summary = {
        "config": {k: getattr(a, k) for k in ("rounds", "industry", "complexity", "budget", "beta",
                                              "lam", "active", "adv_rate", "strategy", "seed")},
        "pool_class": pool_class, "pool_size": len(pool),
        "mean_recall_rounds_ge1": {"static_vs_node_ceiling": round(ceil_, 4),
                                   "adaptive_vs_node": round(node_, 4),
                                   "adaptive_vs_archetype": round(arch_, 4),
                                   "floor": round(floor_, 4)},
        "evasion_gap_node": round(evasion_gap, 4),
        "recovered_by_archetype": round(recovered, 4),
        "fraction_recovered": round(frac, 3),
        "bad_actor_pr_auc": {"node": round(node_prec, 4), "archetype": round(arch_prec, 4)},
        "verdict": verdict,
    }
    out = {**summary, "rounds": history}
    (a.root / "self_play_invariant.json").write_text(json.dumps(out, indent=2))
    print(f"\nSELFPLAY_INVARIANT_DONE rounds={a.rounds} pool_class={pool_class} (size {len(pool)})")
    print(f"  mean recall on bad actor (r>=1): ceiling(static/node)={ceil_:.3f}  adaptive/node={node_:.3f}  "
          f"adaptive/ARCHETYPE={arch_:.3f}  floor={floor_:.3f}")
    print(f"  evasion gap (node lost) = {evasion_gap:+.4f}   recovered by class memory = {recovered:+.4f} "
          f"({frac*100:.0f}% of the gap)")
    print(f"  bad-actor PR-AUC (precision proxy): node={node_prec:.3f}  archetype={arch_prec:.3f}")
    print(f"  -> {verdict}\n  -> {a.root / 'self_play_invariant.json'}")


if __name__ == "__main__":
    main()

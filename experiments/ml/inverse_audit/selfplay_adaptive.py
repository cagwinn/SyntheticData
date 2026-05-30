"""P3.2 — adaptive adversary: true self-play (detection vs evasion equilibrium).

P3.1 (selfplay.py) showed per-account audit memory pays off iff fraud has account
CONTINUITY. A real fraudster has continuity — but a SMART one breaks it: once the auditor
is onto an account, the fraud RELOCATES to an un-watched one. This module closes that loop.

  ADVERSARY holds a small `active` set of bad-actor accounts drawn from a hiding `pool`
    (mid-volume accounts — enough activity to blend, not ubiquitous). Each period it places
    residual-NORMAL fraud there (real JEs relabelled, so the residual is blind — only memory
    can catch it). Then it OBSERVES the auditor's heat (the carried-forward prior) and FLEES:
    any active account the auditor now watches (prior above the pool median) is abandoned for
    the freshest (least-watched) pool account. A fraudster moving accounts once the auditor
    closes in.
  AUDITOR is the P3.1 memory auditor (residual + IDF-weighted carried-forward belief).

Three arms on identical worlds isolate the equilibrium:
  static_vs_memory     : adversary stays put          -> auditor learns it, recall CLIMBS (ceiling).
  adaptive_vs_memory    : adversary flees             -> the arms race (recall suppressed).
  adaptive_vs_memoryless: no watch-list to flee from  -> floor (memoryless can't catch residual-
                          normal fraud; the adversary senses no heat so it doesn't move).

Headline: the EVASION GAP = recall(static_vs_memory) − recall(adaptive_vs_memory) — how much of
memory's gain a fleeing adversary claws back — plus whether the auditor CORNERS the adversary in
later rounds (the hiding pool exhausts as the belief spreads).

    python -m inverse_audit.selfplay_adaptive --root /tmp/sp_adapt --rounds 8 \
        --industry manufacturing --complexity small --pool 12 --active 3 --adv-rate 0.3 --beta 3.0

Outputs:  {root}/round_*/...  and  {root}/self_play_adaptive.json
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np

from inverse_audit.generate import base_config, generate, set_fraud_rate
from inverse_audit.selfplay import (Auditor, _je_accounts, _lean, _score_round, _with_anomaly)


class AdaptiveAdversary:
    """A bad actor that places residual-normal fraud at `active` accounts and flees the ones
    the auditor starts watching."""

    def __init__(self, pool: list[str], n_active: int, rate: float, evasion: bool,
                 df: dict[str, int] | None = None, strategy: str = "coldest"):
        self.pool = list(pool)
        self.n_active = n_active
        self.rate = rate
        self.evasion = evasion
        self.df = df or {}                # account -> volume (for the 'highvol' blend strategy)
        self.strategy = strategy          # 'coldest' = flee to least-watched; 'highvol' = blend in volume
        self.active = list(pool[:n_active])
        self.relocations = 0
        self.ever_used: set[str] = set(self.active)

    def place_fraud(self, j, je_acct) -> np.ndarray:
        """Relabel a deterministic fraction of JEs touching the CURRENT active accounts."""
        aset = set(self.active)
        cand = sorted(je for je, accts in je_acct.items() if aset & set(accts))
        chosen = set(cand[: int(self.rate * len(cand))])
        return j["je_id"].isin(chosen).to_numpy()

    def observe_and_relocate(self, auditor: Auditor, heat_fn=None) -> int:
        """Flee active accounts the auditor now watches (heat above pool median) to the
        freshest (lowest-heat) un-used pool accounts. `heat_fn(account)->float` reports the
        auditor's attention on an account AT THE AUDITOR'S OWN GRANULARITY (account-code for
        node memory; the account's CLASS for relocation-invariant memory — so an adversary
        facing class memory sees its whole class lit up and finds no fresh in-class hideout).
        Defaults to the node-level carried prior. Returns #relocations this round."""
        if not self.evasion:
            return 0
        heat = heat_fn if heat_fn is not None else (lambda a: auditor.prior.get(a, 0.0))
        pp = sorted(heat(a) for a in self.pool)
        med = pp[len(pp) // 2] if pp else 0.0
        burned = [a for a in self.active if heat(a) > max(med, 1e-9)]
        if not burned:
            return 0
        fresh_pool = [a for a in self.pool if a not in self.active]
        if self.strategy == "highvol":
            # blend into volume: among the un-watched (heat <= median), prefer HIGHEST volume
            cool = [a for a in fresh_pool if heat(a) <= med]
            avail = sorted(cool or fresh_pool, key=lambda a: (-self.df.get(a, 0), a))
        else:                              # 'coldest' — flee to the least-watched account
            avail = sorted(fresh_pool, key=lambda a: (heat(a), a))
        moved = 0
        for b in burned:
            if avail:
                fresh = avail.pop(0)
                self.active[self.active.index(b)] = fresh
                self.ever_used.add(fresh)
                self.relocations += 1
                moved += 1
        return moved


def main(argv=None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=Path("/tmp/sp_adapt"))
    ap.add_argument("--rounds", type=int, default=8)
    ap.add_argument("--industry", default="manufacturing")
    ap.add_argument("--complexity", default="small")
    ap.add_argument("--fraud-rate", type=float, default=0.04, help="engine background fraud")
    ap.add_argument("--anomaly-rate", type=float, default=0.04, help="engine background anomaly")
    ap.add_argument("--seed", type=int, default=20260530)
    ap.add_argument("--budget", type=float, default=0.05)
    ap.add_argument("--beta", type=float, default=3.0)
    ap.add_argument("--lam", type=float, default=0.5)
    ap.add_argument("--pool", type=int, default=12, help="size of the hiding pool (mid-volume accts)")
    ap.add_argument("--active", type=int, default=3, help="bad-actor accounts active per period")
    ap.add_argument("--adv-rate", type=float, default=0.3, help="fraction of an active acct's JEs that are fraud")
    ap.add_argument("--evasion-strategy", default="coldest", choices=("coldest", "highvol"),
                    help="coldest = flee to least-watched account; highvol = flee to un-watched but "
                         "HIGHEST-volume account (blend into volume — the smarter adversary)")
    ap.add_argument("--skip-generate", action="store_true")
    ap.add_argument("--skip-scoring", action="store_true")
    a = ap.parse_args(argv)
    a.root.mkdir(parents=True, exist_ok=True)

    # three arms: (adversary evasion, auditor memory)
    def mk_auditor(mem):
        return Auditor("a", use_memory=mem, use_confirmation=mem, beta=a.beta, lam=a.lam)
    arms = {
        "static_vs_memory":      {"adv": None, "aud": mk_auditor(True),  "evasion": False, "mem": True},
        "adaptive_vs_memory":    {"adv": None, "aud": mk_auditor(True),  "evasion": True,  "mem": True},
        "adaptive_vs_memoryless":{"adv": None, "aud": mk_auditor(False), "evasion": True,  "mem": False},
    }
    pool: list[str] = []
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
        df_acct: dict[str, int] = {}
        for accts in je_acct.values():
            for acct in set(accts):
                df_acct[acct] = df_acct.get(acct, 0) + 1
        n_je = max(1, len(je_acct))
        idf = {acct: float(np.log(n_je / (c + 1))) for acct, c in df_acct.items()}
        engine_target = (j["is_fraud"] | j["is_anomaly"]).to_numpy()
        K = max(1, int(a.budget * len(j)))

        if not pool:                                # choose the hiding pool ONCE (stable CoA codes)
            mid = sorted(((c, acct) for acct, c in df_acct.items() if 20 <= c <= 400), reverse=True)
            pool = [acct for _, acct in mid[: a.pool]]
            for arm in arms.values():
                arm["adv"] = AdaptiveAdversary(pool, a.active, a.adv_rate, arm["evasion"],
                                               df=df_acct, strategy=a.evasion_strategy)

        rec: dict = {"round": r, "n_je": int(len(j)), "K": K, "arms": {}}
        for name, arm in arms.items():
            adv, aud = arm["adv"], arm["aud"]
            is_adv = adv.place_fraud(j, je_acct)
            is_target = engine_target | is_adv
            s = aud.score(j, je_acct)
            order = np.argsort(-s)
            inv = np.zeros(len(j), dtype=bool)
            inv[order[:K]] = True
            n_adv = int(is_adv.sum())
            caught = int((inv & is_adv).sum())
            recall_adv = caught / max(1, n_adv)
            aud.update(j, je_acct, inv, is_target, idf)     # auditor learns (uses confirmed fraud)
            moved = adv.observe_and_relocate(aud)           # adversary flees the heat
            rec["arms"][name] = {
                "recall_adv": round(recall_adv, 4), "n_adv": n_adv, "caught": caught,
                "active": list(adv.active), "relocations_this_round": moved,
                "relocations_total": adv.relocations, "distinct_accts_used": len(adv.ever_used),
            }
        history.append(rec)
        sm, am, al = (rec["arms"][n]["recall_adv"] for n in
                      ("static_vs_memory", "adaptive_vs_memory", "adaptive_vs_memoryless"))
        mv = rec["arms"]["adaptive_vs_memory"]["relocations_this_round"]
        print(f"[round {r}] recall on bad actor: static+mem={sm:.3f}  adaptive+mem={am:.3f}  "
              f"adaptive+memless={al:.3f} | adversary fled {mv} acct(s), now {rec['arms']['adaptive_vs_memory']['active']}")

    def mean_recall(name, lo=1):
        return float(np.mean([h["arms"][name]["recall_adv"] for h in history[lo:]])) if len(history) > lo else 0.0
    static_m = mean_recall("static_vs_memory")
    adapt_m = mean_recall("adaptive_vs_memory")
    floor = mean_recall("adaptive_vs_memoryless")
    evasion_gap = static_m - adapt_m
    # cornering: did adaptive recall in the LAST third exceed the first post-warmup third?
    third = max(1, len(history) // 3)
    early = float(np.mean([h["arms"]["adaptive_vs_memory"]["recall_adv"] for h in history[1:1 + third]])) if len(history) > 1 else 0.0
    late = float(np.mean([h["arms"]["adaptive_vs_memory"]["recall_adv"] for h in history[-third:]]))
    cornered = late - early
    distinct = history[-1]["arms"]["adaptive_vs_memory"]["distinct_accts_used"]

    verdict = ("EVASION WORKS — a fleeing adversary claws back memory's gain" if evasion_gap > 0.02 else
               "auditor KEEPS UP — memory tracks the moving adversary" if evasion_gap < 0.005 else
               "PARTIAL evasion — adversary suppresses but doesn't escape memory")
    summary = {
        "config": {k: getattr(a, k) for k in ("rounds", "industry", "complexity", "budget", "beta",
                                              "lam", "pool", "active", "adv_rate", "evasion_strategy", "seed")},
        "hiding_pool": pool,
        "mean_recall_rounds_ge1": {"static_vs_memory": round(static_m, 4),
                                   "adaptive_vs_memory": round(adapt_m, 4),
                                   "adaptive_vs_memoryless_floor": round(floor, 4)},
        "evasion_gap": round(evasion_gap, 4),
        "cornering_late_minus_early": round(cornered, 4),
        "distinct_accounts_adversary_burned_through": distinct,
        "verdict": verdict,
    }
    out = {**summary, "rounds": history}
    (a.root / "self_play_adaptive.json").write_text(json.dumps(out, indent=2))
    print(f"\nSELFPLAY_ADAPTIVE_DONE rounds={a.rounds}")
    print(f"  mean recall on bad actor (r>=1): static+mem={static_m:.3f}  adaptive+mem={adapt_m:.3f}  floor={floor:.3f}")
    print(f"  evasion gap (static-adaptive) = {evasion_gap:+.4f}   cornering (late-early) = {cornered:+.4f}   "
          f"adversary burned through {distinct}/{len(pool)} pool accts")
    print(f"  -> {verdict}\n  -> {a.root / 'self_play_adaptive.json'}")


if __name__ == "__main__":
    main()

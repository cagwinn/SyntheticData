"""P3.1 — autonomous-audit self-play: the closed round-trip skeleton.

A multi-period audit game. Each round = one fiscal period:

  ADVERSARY (DataSynth) posts a GL with embedded fraud. In P3.1 its policy is STATIC
    (fixed fraud_rate / anomaly_rate); P3.2 makes it adaptive (concealment responds to
    what evaded detection).
  AUDITOR (the substrate) ranks JEs by the density + relational residual (substrate P1),
    BOOSTS each JE by its accounts' carried-forward risk priors (the cortex / methodology
    belief state, substrate P2), investigates the top of the ranking under a fixed audit
    budget, and UPDATES its belief from what it found — carrying it into the next period.
    This is exactly how prior-period audit findings raise the current period's
    risk-of-material-misstatement.

We run THREE auditor policies on the SAME generated worlds, each with its own belief
trajectory, and compare detection under a fixed investigation budget:

  memoryless        : rank by the residual only (no carry-forward).
  memory_flagged    : carry forward accounts it FLAGGED (label-free — the corpus-deployable
                      variant; you re-examine accounts you flagged before, confirmed or not).
  memory_confirmed  : carry forward accounts where investigation CONFIRMED fraud (audit-
                      faithful — prior-year KNOWN findings; the upper bound). Investigation
                      = the labels, standing in for the auditor's confirmation outcome.

Headline P3.1 measurement: does cross-period audit memory improve recall@budget on
PERSISTENT fraud vs the memoryless auditor? Reported per round, plus PR-AUC (threshold-free),
prior concentration on the truly fraud-prone accounts, and an honest verdict.

    python -m inverse_audit.selfplay --root /tmp/sp --rounds 6 \
        --industry manufacturing --complexity small --fraud-rate 0.04 --anomaly-rate 0.04

Outputs:
  {root}/round_{r}/normal,test        — the per-period GLs (lean: heavy subsystems off)
  {root}/round_{r}/{density,graph}_scores.parquet
  {root}/self_play.json               — per-round scorecard + belief evolution + verdict
"""
from __future__ import annotations

import argparse
import copy
import json
import subprocess
import sys
from pathlib import Path

import numpy as np
import pandas as pd

from inverse_audit.generate import base_config, generate, set_fraud_rate
from inverse_audit.unified_score import _metrics, _z

# heavy subsystems irrelevant to the GL audit — disabling them takes a single-company
# manufacturing/small period from ~12 s / 2.6 GB to ~0.4 s / 0.2 GB (no OOM, many rounds cheap).
_HEAVY = ("banking", "esg", "treasury", "tax", "project_accounting", "process_mining",
          "manufacturing", "graph_export")


def _lean(cfg: dict) -> dict:
    cfg = copy.deepcopy(cfg)
    for k in _HEAVY:
        cfg.setdefault(k, {})["enabled"] = False
    cfg.setdefault("global", {})["memory_limit_mb"] = 4096
    return cfg


def _with_anomaly(cfg: dict, rate: float) -> dict:
    cfg = copy.deepcopy(cfg)
    ai = cfg.setdefault("anomaly_injection", {})
    ai["enabled"] = rate > 0.0
    ai.setdefault("rates", {})["total_rate"] = rate
    return cfg


def _sh(mod: str, *args: str) -> None:
    subprocess.run([sys.executable, "-m", mod, *args], check=True)


def _score_round(rdir: Path, reuse: bool = False) -> pd.DataFrame:
    """Run both arms on rdir/{normal,test}; return per-JE frame with unified_z + labels.
    reuse=True loads existing parquets (valid even under label overlays — the residual is
    label-free, so relabeling is_fraud changes only metrics/learning, not the score)."""
    dpath, gpath = rdir / "density_scores.parquet", rdir / "graph_scores.parquet"
    if not (reuse and dpath.exists() and gpath.exists()):
        _sh("inverse_audit.score", "--normal", str(rdir / "normal"),
            "--test", str(rdir / "test"), "--out", str(dpath))
        _sh("inverse_audit.relational.graph_scorer", "--normal", str(rdir / "normal"),
            "--test", str(rdir / "test"), "--out", str(gpath))
    d = pd.read_parquet(dpath)
    g = pd.read_parquet(gpath)
    if "document_id" in d.columns:
        d = d.rename(columns={"document_id": "je_id"})
    elif d.index.name == "document_id":
        d = d.reset_index().rename(columns={"document_id": "je_id"})
    d["je_id"] = d["je_id"].astype(str)
    g["je_id"] = g["je_id"].astype(str)
    keep_d = [c for c in ("je_id", "score", "is_fraud", "is_anomaly", "fraud_type") if c in d.columns]
    keep_g = ["je_id", "relational_score"] + [c for c in ("anomaly_type",) if c in g.columns]
    j = d[keep_d].merge(g[keep_g], on="je_id", how="inner")
    j["density_z"] = _z(j["score"].to_numpy())
    j["relational_z"] = _z(j["relational_score"].to_numpy())
    j["unified_z"] = j[["density_z", "relational_z"]].fillna(0).sum(axis=1)
    for c in ("is_fraud", "is_anomaly"):
        j[c] = j.get(c, pd.Series(False, index=j.index)).fillna(False).astype(bool)
    return j


def _je_accounts(test_dir: Path) -> dict[str, list[str]]:
    """document_id -> list of gl_accounts it touches (for per-account attribution)."""
    gl = pd.read_csv(test_dir / "journal_entries.csv", low_memory=False)
    gl["document_id"] = gl["document_id"].astype(str)
    gl["gl_account"] = gl["gl_account"].astype(str)
    return {k: list(v) for k, v in gl.groupby("document_id")["gl_account"]}


def _fraud_accounts(test_dir: Path) -> set[str]:
    gl = pd.read_csv(test_dir / "journal_entries.csv", low_memory=False)
    if "is_fraud" not in gl.columns:
        return set()
    f = gl[gl["is_fraud"].fillna(False).astype(bool)]
    return set(f["gl_account"].astype(str).unique())


class Auditor:
    """One belief trajectory. prior[account] = carried-forward risk (EWMA exposure)."""

    def __init__(self, name: str, use_memory: bool, use_confirmation: bool,
                 beta: float, lam: float):
        self.name = name
        self.use_memory = use_memory
        self.use_confirmation = use_confirmation     # learn from confirmed (label) vs flagged
        self.beta, self.lam = beta, lam
        self.prior: dict[str, float] = {}

    def _je_boost(self, je_ids, je_acct) -> np.ndarray:
        if not self.use_memory or not self.prior:
            return np.zeros(len(je_ids))
        vals = np.array(list(self.prior.values()))
        mu, sd = float(vals.mean()), float(vals.std() or 1.0)
        out = np.zeros(len(je_ids))
        for i, je in enumerate(je_ids):
            accts = je_acct.get(je, [])
            if accts:
                out[i] = (max(self.prior.get(a, 0.0) for a in accts) - mu) / sd
        return out

    def score(self, j: pd.DataFrame, je_acct) -> np.ndarray:
        return j["unified_z"].to_numpy() + self.beta * self._je_boost(list(j["je_id"]), je_acct)

    def update(self, j: pd.DataFrame, je_acct, investigated_mask, is_target, idf) -> None:
        """EWMA the per-account exposure of what the auditor LEARNED this period into the
        carried prior. confirmed mode: only investigated AND truly anomalous; flagged mode:
        all investigated. Exposure is IDF-weighted (idf[a]) so ubiquitous control accounts
        — touched by ~every JE incl. every fraud — don't swamp the genuinely fraud-ENRICHED
        mid-volume accounts; specificity is what carries useful audit memory."""
        learn = investigated_mask & is_target if self.use_confirmation else investigated_mask
        exposure: dict[str, float] = {}
        for je in j["je_id"].to_numpy()[learn]:
            for a in je_acct.get(je, []):
                exposure[a] = exposure.get(a, 0.0) + idf.get(a, 1.0)
        seen = set(self.prior) | set(exposure)
        for a in seen:
            self.prior[a] = (1 - self.lam) * self.prior.get(a, 0.0) + self.lam * exposure.get(a, 0.0)


def main(argv=None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=Path("/tmp/sp"))
    ap.add_argument("--rounds", type=int, default=6)
    ap.add_argument("--industry", default="manufacturing")
    ap.add_argument("--complexity", default="small")
    ap.add_argument("--fraud-rate", type=float, default=0.04)
    ap.add_argument("--anomaly-rate", type=float, default=0.04)
    ap.add_argument("--seed", type=int, default=20260529)
    ap.add_argument("--budget", type=float, default=0.05, help="fraction of JEs investigated per period")
    ap.add_argument("--beta", type=float, default=1.0, help="carried-prior boost weight")
    ap.add_argument("--lam", type=float, default=0.5, help="EWMA belief-update rate")
    ap.add_argument("--skip-generate", action="store_true")
    ap.add_argument("--skip-scoring", action="store_true", help="reuse existing per-round parquets")
    ap.add_argument("--persistent-accounts", type=int, default=0,
                    help="SCIENTIFIC CONTROL: N fixed bad-actor accounts that re-offend every "
                         "period. Their transactions look NORMAL (low residual) so only "
                         "cross-period account memory can catch them — isolates the value of "
                         "audit memory from the residual.")
    ap.add_argument("--persistent-rate", type=float, default=0.4,
                    help="fraction of a persistent account's JEs that are the bad actor's")
    a = ap.parse_args(argv)
    a.root.mkdir(parents=True, exist_ok=True)

    auditors = [
        Auditor("memoryless", use_memory=False, use_confirmation=False, beta=a.beta, lam=a.lam),
        Auditor("memory_flagged", use_memory=True, use_confirmation=False, beta=a.beta, lam=a.lam),
        Auditor("memory_confirmed", use_memory=True, use_confirmation=True, beta=a.beta, lam=a.lam),
    ]
    fraud_seen: dict[str, int] = {}
    persistent_set: list[str] = []        # the fixed bad-actor accounts (scientific control)
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
        # account document-frequency -> IDF (specificity); ubiquitous control accounts ~0, rare accounts high
        df_acct: dict[str, int] = {}
        for accts in je_acct.values():
            for acct in set(accts):
                df_acct[acct] = df_acct.get(acct, 0) + 1
        n_je_total = max(1, len(je_acct))
        idf = {acct: float(np.log(n_je_total / (c + 1))) for acct, c in df_acct.items()}

        # --- SCIENTIFIC CONTROL: persistent adversary overlay ---------------------------
        # A FIXED set of bad-actor accounts re-offends every period. Their JEs look normal
        # (real JEs relabelled, residual unchanged) so the residual cannot catch them on
        # their own merits — ONLY cross-period account memory can. Isolates memory's value.
        is_persist = np.zeros(len(j), dtype=bool)
        if a.persistent_accounts > 0:
            if not persistent_set:                       # choose ONCE (round 0), stable CoA codes
                mid = sorted(((c, acct) for acct, c in df_acct.items() if 20 <= c <= 400),
                             reverse=True)
                persistent_set = [acct for _, acct in mid[:a.persistent_accounts]]
            pset = set(persistent_set)
            cand = sorted(je for je, accts in je_acct.items() if pset & set(accts))
            chosen = set(cand[:int(a.persistent_rate * len(cand))])
            is_persist = j["je_id"].isin(chosen).to_numpy()
            j["is_fraud"] = j["is_fraud"].to_numpy() | is_persist
        j["is_persistent"] = is_persist

        is_target = (j["is_fraud"] | j["is_anomaly"]).to_numpy()
        n_target = int(is_target.sum())
        K = max(1, int(a.budget * len(j)))

        rec: dict = {"round": r, "n_je": int(len(j)), "n_target": n_target, "budget_K": K,
                     "n_persistent": int(is_persist.sum()),
                     "fraud_rate": round(float(j["is_fraud"].mean()), 4),
                     "anomaly_rate": round(float(j["is_anomaly"].mean()), 4), "policies": {}}

        # persistence: fraud accounts that ALSO carried fraud in an earlier round
        fa = _fraud_accounts(rdir / "test")
        recurring = sorted(x for x in fa if fraud_seen.get(x, 0) >= 1)

        for au in auditors:
            s = au.score(j, je_acct)
            order = np.argsort(-s)
            inv = np.zeros(len(j), dtype=bool)
            inv[order[:K]] = True
            confirmed = int((inv & is_target).sum())
            recall = confirmed / max(1, n_target)
            precision = confirmed / K
            persist_caught = int((inv & is_persist).sum())
            recall_persist = persist_caught / max(1, int(is_persist.sum()))
            pr = _metrics(is_target, s)
            # how many recurring-fraud accounts has THIS policy elevated above its prior mean?
            watched = 0
            if au.prior:
                mu = float(np.mean(list(au.prior.values())))
                watched = sum(1 for x in recurring if au.prior.get(x, 0.0) > mu)
            rec["policies"][au.name] = {
                "recall_at_budget": round(recall, 4), "precision_at_budget": round(precision, 4),
                "recall_persistent": round(recall_persist, 4),
                "confirmed": confirmed, "pr_auc": (None if pr["pr_auc"] is None else round(pr["pr_auc"], 4)),
                "roc_auc": (None if pr["roc_auc"] is None else round(pr["roc_auc"], 4)),
                "recurring_watched": watched, "n_recurring": len(recurring),
                "prior_nonzero_accts": int(sum(1 for v in au.prior.values() if v > 0)),
            }
            au.update(j, je_acct, inv, is_target, idf)

        for x in fa:
            fraud_seen[x] = fraud_seen.get(x, 0) + 1

        # top carried accounts for the confirmed auditor + whether they're truly fraud-prone
        conf = auditors[-1]
        top = sorted(conf.prior.items(), key=lambda kv: -kv[1])[:8]
        rec["confirmed_top_prior"] = [{"account": x, "prior": round(v, 2),
                                       "is_fraud_acct_now": x in fa} for x, v in top]
        history.append(rec)
        ml, mf, mc = (rec["policies"][n] for n in ("memoryless", "memory_flagged", "memory_confirmed"))
        tail = ("" if not is_persist.any() else
                f" | recall_persist[{int(is_persist.sum())}] memoryless={ml['recall_persistent']:.3f} "
                f"flagged={mf['recall_persistent']:.3f} confirmed={mc['recall_persistent']:.3f}")
        print(f"[round {r}] n_je={len(j)} target={n_target} K={K} recurring_fraud_accts={len(recurring)} | "
              f"recall@budget  memoryless={ml['recall_at_budget']:.3f}  "
              f"flagged={mf['recall_at_budget']:.3f}  confirmed={mc['recall_at_budget']:.3f}{tail}")

    def mean_delta(metric: str, policy: str) -> float:
        ds = [history[r]["policies"][policy][metric] - history[r]["policies"]["memoryless"][metric]
              for r in range(1, len(history))
              if history[r]["policies"][policy][metric] is not None
              and history[r]["policies"]["memoryless"][metric] is not None]
        return float(np.mean(ds)) if ds else 0.0

    persist_on = a.persistent_accounts > 0
    summary = {
        "config": {k: getattr(a, k) for k in ("rounds", "industry", "complexity", "fraud_rate",
                                              "anomaly_rate", "budget", "beta", "lam", "seed",
                                              "persistent_accounts", "persistent_rate")},
        "persistent_accounts_used": persistent_set,
        "mean_delta_recall_at_budget_rounds_ge1": {
            "memory_flagged": round(mean_delta("recall_at_budget", "memory_flagged"), 4),
            "memory_confirmed": round(mean_delta("recall_at_budget", "memory_confirmed"), 4)},
        "mean_delta_pr_auc_rounds_ge1": {
            "memory_flagged": round(mean_delta("pr_auc", "memory_flagged"), 4),
            "memory_confirmed": round(mean_delta("pr_auc", "memory_confirmed"), 4)},
        "mean_delta_recall_persistent_rounds_ge1": {
            "memory_flagged": round(mean_delta("recall_persistent", "memory_flagged"), 4),
            "memory_confirmed": round(mean_delta("recall_persistent", "memory_confirmed"), 4)},
        # the evolved belief state of the audit-faithful auditor — carried-forward per-account
        # risk after all periods; this is what feeds the AuditMethodology RMM engine (see
        # selfplay_rmm.py): self-play belief -> ISA-315 risk-of-material-misstatement.
        "final_prior_confirmed": {acct: round(v, 3) for acct, v in
                                  sorted(auditors[-1].prior.items(), key=lambda kv: -kv[1])[:40]},
        # volume-normalised belief (carried risk PER UNIT activity, last period's df) — isolates
        # the specifically-risky accounts (persistent bad actors / fraud-enriched) from the merely
        # high-volume control accounts. This density is the cleaner carried-forward-finding signal.
        "final_belief_risk_density": {acct: round(v / max(1, df_acct.get(acct, 1)), 4) for acct, v in
                                      sorted(((a2, p2) for a2, p2 in auditors[-1].prior.items()),
                                             key=lambda kv: -kv[1] / max(1, df_acct.get(kv[0], 1)))[:40]},
    }
    # headline = the persistent-adversary recall delta when the control is on; else recall@budget
    head_key = "mean_delta_recall_persistent_rounds_ge1" if persist_on else "mean_delta_recall_at_budget_rounds_ge1"
    best = max(summary[head_key].values())
    target = "the PERSISTENT adversary" if persist_on else "persistent fraud"
    summary["verdict"] = (f"audit memory IMPROVES detection of {target} (Δ={best:+.4f})" if best > 0.01 else
                          f"audit memory NEUTRAL on {target} (Δ={best:+.4f})" if best > -0.01 else
                          f"audit memory HURTS on {target} (Δ={best:+.4f})")
    out = {**summary, "rounds": history}
    (a.root / "self_play.json").write_text(json.dumps(out, indent=2))
    print(f"\nSELFPLAY_DONE rounds={a.rounds}  persistent_adversary={'ON '+str(persistent_set) if persist_on else 'off'}")
    print(f"  mean Δrecall@budget   (r>=1): flagged={summary['mean_delta_recall_at_budget_rounds_ge1']['memory_flagged']:+.4f}  "
          f"confirmed={summary['mean_delta_recall_at_budget_rounds_ge1']['memory_confirmed']:+.4f}")
    if persist_on:
        print(f"  mean Δrecall_persist  (r>=1): flagged={summary['mean_delta_recall_persistent_rounds_ge1']['memory_flagged']:+.4f}  "
              f"confirmed={summary['mean_delta_recall_persistent_rounds_ge1']['memory_confirmed']:+.4f}")
    print(f"  mean ΔPR-AUC          (r>=1): flagged={summary['mean_delta_pr_auc_rounds_ge1']['memory_flagged']:+.4f}  "
          f"confirmed={summary['mean_delta_pr_auc_rounds_ge1']['memory_confirmed']:+.4f}")
    print(f"  -> {summary['verdict']}\n  -> {a.root / 'self_play.json'}")


if __name__ == "__main__":
    main()

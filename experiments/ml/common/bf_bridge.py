"""Bridge to the behavioral-fidelity (BF) eval.

Two scoring paths:

1. `score_canonical(candidate_dir)` — shells out to the Rust
   `datasynth-data behavioral score`, the single source of truth for the
   composite. Use for final lift numbers.

2. `score_fast(corpus_df, candidate_df)` — a lightweight Python
   re-implementation of the headline degradation ratios (IETD, JELineBurst,
   MeanGap, clustering) for *in-loop* use where shelling out per iteration is
   too slow (the surrogate track trains on these). It is an APPROXIMATION —
   always confirm a candidate with `score_canonical` before believing a win.

Degradation ratio (DR), per the eval: DR = d(synth, corpus) / noise_floor,
where the noise floor is the corpus-vs-corpus distance under resampling.
DR = 1.0 means "indistinguishable from a fresh corpus draw"; higher = worse.
The composite is the (volume-corrected) mean / median of per-metric DRs.
"""

from __future__ import annotations

import json
import shutil
import subprocess
from pathlib import Path

import numpy as np


# --------------------------------------------------------------------------
# 1. Canonical scorer — defer to the Rust eval.
# --------------------------------------------------------------------------
def _find_cli() -> str:
    for cand in ("./target/release/datasynth-data", "datasynth-data"):
        if shutil.which(cand) or Path(cand).exists():
            return cand
    raise FileNotFoundError(
        "datasynth-data binary not found — build with "
        "`cargo build --release -p datasynth-cli` or add it to PATH"
    )


def score_canonical(candidate_dir: Path, profile: str = "gl-source-tp") -> dict:
    """Run the Rust BF eval on a generated candidate archive.

    Returns the parsed composite report. The exact subcommand/flags can drift;
    confirm with `datasynth-data behavioral --help`. We capture JSON output.
    """
    cli = _find_cli()
    cmd = [
        cli, "behavioral", "score",
        "--synthetic", str(candidate_dir),
        "--profile", profile,
        "--format", "json",
    ]
    print(f"[bf_bridge] $ {' '.join(cmd)}")
    out = subprocess.run(cmd, capture_output=True, text=True)
    if out.returncode != 0:
        raise RuntimeError(f"behavioral score failed:\n{out.stderr}")
    return json.loads(out.stdout)


# --------------------------------------------------------------------------
# 2. Fast in-loop approximation (Python). APPROXIMATE — see module docstring.
# --------------------------------------------------------------------------
def _hist_distance(a: np.ndarray, b: np.ndarray, bins: int = 64) -> float:
    """Symmetric histogram (Jensen-Shannon) distance on a shared support."""
    lo = float(min(a.min(), b.min()))
    hi = float(max(a.max(), b.max()))
    if hi <= lo:
        return 0.0
    edges = np.linspace(lo, hi, bins + 1)
    pa, _ = np.histogram(a, edges, density=True)
    pb, _ = np.histogram(b, edges, density=True)
    pa = pa / (pa.sum() + 1e-12)
    pb = pb / (pb.sum() + 1e-12)
    m = 0.5 * (pa + pb)

    def _kl(p, q):
        mask = p > 0
        return float(np.sum(p[mask] * np.log(p[mask] / (q[mask] + 1e-12))))

    return 0.5 * _kl(pa, m) + 0.5 * _kl(pb, m)


def _iet_days(df) -> np.ndarray:
    """Inter-event times (days) within (source, entity) streams."""
    g = df.sort_values("entry_date").groupby(["source", "trading_partner"])
    out = []
    for _, sub in g:
        dates = sub["entry_date"].values.astype("datetime64[D]")
        if len(dates) >= 2:
            out.append(np.diff(dates).astype("timedelta64[D]").astype(float))
    return np.concatenate(out) if out else np.array([0.0])


def _lines_per_je(df) -> np.ndarray:
    return df.groupby("je_number").size().to_numpy(dtype=float)


def score_fast(corpus_df, candidate_df) -> dict:
    """Approximate per-metric DRs from two pandas frames.

    Noise floor here is a crude constant per metric; the canonical eval
    computes it by corpus resampling. Good enough to give CMA-ES / the
    surrogate a smooth, correctly-ordered signal between full evals.
    """
    # IETD (P1) and JELineBurst (P2) via JS distance on the relevant dists.
    ietd = _hist_distance(_iet_days(corpus_df), _iet_days(candidate_df))
    burst = _hist_distance(_lines_per_je(corpus_df), _lines_per_je(candidate_df))
    # MeanGap (P4): |Δ mean inter-event time|, normalized.
    mean_gap = abs(_iet_days(corpus_df).mean() - _iet_days(candidate_df).mean())

    # TODO(P3 clustering): port the TP co-occurrence triangle/clustering
    # distance once the GNN export lands (shares the edge builder).
    noise = {"IETD": 0.02, "JELineBurst": 0.05, "MeanGap": 1.0}
    return {
        "IETD": ietd / noise["IETD"],
        "JELineBurst": burst / noise["JELineBurst"],
        "MeanGap": mean_gap / noise["MeanGap"],
    }


def composite(drs: dict) -> dict:
    vals = np.array(list(drs.values()), dtype=float)
    return {"mean": float(vals.mean()), "median": float(np.median(vals))}


if __name__ == "__main__":
    import argparse

    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--candidate", type=Path, required=True,
                    help="generated candidate archive dir for the canonical eval")
    ap.add_argument("--profile", default="gl-source-tp")
    args = ap.parse_args()
    report = score_canonical(args.candidate, args.profile)
    print(json.dumps(report, indent=2))

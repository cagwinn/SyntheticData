"""Generate (θ, x) training pairs for the inverse model.

For each θ drawn from the prior: write a config with those overrides, run
`datasynth-data generate`, read the resulting journal_entries, compute the
summary-stat feature vector x. Emit (θ, x) to `--out`.

    python -m inverse.simulate --n 2000 --out data/inverse --base configs/inverse_base.yaml

CPU-bound and embarrassingly parallel across θ; we fan out over a process
pool. Does NOT need the corpus — pure synthetic self-simulation (the SBI
training set). Runs that fail (bad override / generate error) are dropped, not
fatal.
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import shutil
import subprocess
import tempfile
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

import numpy as np
import pandas as pd
import yaml

from . import params as P

_ROUND_LEVELS = np.array([1_000.0, 5_000.0, 10_000.0, 25_000.0, 50_000.0, 100_000.0])

# Fixed feature order — keep stable so x is comparable across runs / re-runs.
FEATURE_NAMES = [
    "log_amt_mean", "log_amt_std", "log_amt_skew", "benford_mad", "round_frac",
    "weekend_frac", "monthend_frac", "postclose_frac", "manual_frac",
    "lpje_mean", "lpje_std", "lpje_frac2", "lpje_frac_gt2",
    "lag_mean", "lag_std", "lag_pos_frac",
    "src_share1", "src_share2", "src_share3", "src_share4", "src_share5", "src_entropy",
    "iet_mean", "iet_std",
    "gl_n_log", "gl_top5_share", "gl_entropy",
    "n_lines_log", "n_docs_log",
]
DIM_X = len(FEATURE_NAMES)


def _cli() -> str:
    for c in ("./target/release/datasynth-data", "datasynth-data"):
        if Path(c).exists() or shutil.which(c):
            return c
    raise FileNotFoundError("build datasynth-data (cargo build --release -p datasynth-cli)")


def _moments(v: np.ndarray) -> tuple[float, float, float]:
    v = v[np.isfinite(v)]
    if v.size == 0:
        return 0.0, 0.0, 0.0
    m, s = float(v.mean()), float(v.std())
    sk = float(((v - m) ** 3).mean() / (s ** 3)) if s > 1e-9 else 0.0
    return m, s, sk


def _entropy(shares: np.ndarray) -> float:
    p = shares[shares > 0]
    return float(-(p * np.log(p)).sum()) if p.size else 0.0


def summary_stats(je_csv: Path) -> np.ndarray:
    """GL → fixed-length feature vector x (DIM_X,). Observable-only (no labels)
    so the same map applies to an out-of-sample GL at inference time."""
    df = pd.read_csv(je_csv, low_memory=False)
    n = len(df)
    if n == 0:
        return np.zeros(DIM_X, dtype=np.float32)

    deb = pd.to_numeric(df.get("debit_amount", 0), errors="coerce").fillna(0.0).to_numpy()
    cred = pd.to_numeric(df.get("credit_amount", 0), errors="coerce").fillna(0.0).to_numpy()
    amt = np.where(deb != 0, deb, cred).astype(float)
    nz = np.abs(amt[amt != 0])
    log_amt = np.log1p(nz)
    la_mean, la_std, la_skew = _moments(log_amt)

    # Benford first-digit MAD vs the ideal law.
    fd = np.array([int(str(int(a))[0]) for a in nz if a >= 1], dtype=int)
    if fd.size:
        obs = np.array([(fd == d).mean() for d in range(1, 10)])
        exp = np.log10(1 + 1 / np.arange(1, 10))
        benford_mad = float(np.abs(obs - exp).mean())
    else:
        benford_mad = 0.0

    nearest = np.abs(nz[:, None] - _ROUND_LEVELS[None, :]).min(axis=1) if nz.size else np.array([1e9])
    round_frac = float((nearest < 1.0).mean())

    pdt = pd.to_datetime(df.get("posting_date"), errors="coerce")
    dow = pdt.dt.dayofweek
    weekend_frac = float((dow >= 5).mean()) if n else 0.0
    monthend_frac = float((pdt.dt.day >= 25).mean()) if n else 0.0

    def _frac_true(col: str) -> float:
        if col not in df:
            return 0.0
        return float(df[col].astype("boolean").fillna(False).mean())

    postclose_frac = _frac_true("is_post_close")
    manual_frac = _frac_true("is_manual")

    # Lines per JE
    if "document_id" in df:
        lpje = df.groupby("document_id").size().to_numpy()
        lpje_mean, lpje_std = float(lpje.mean()), float(lpje.std())
        lpje_f2 = float((lpje == 2).mean())
        lpje_fgt2 = float((lpje > 2).mean())
    else:
        lpje_mean = lpje_std = lpje_f2 = lpje_fgt2 = 0.0

    # Posting lag (posting - document), days
    if "document_date" in df:
        ddt = pd.to_datetime(df["document_date"], errors="coerce")
        lag = (pdt - ddt).dt.days.to_numpy().astype(float)
        lag = lag[np.isfinite(lag)]
        lag_mean, lag_std = (float(lag.mean()), float(lag.std())) if lag.size else (0.0, 0.0)
        lag_pos = float((lag > 0).mean()) if lag.size else 0.0
    else:
        lag_mean = lag_std = lag_pos = 0.0

    # Source mix
    if "source" in df:
        vc = df["source"].astype(str).value_counts(normalize=True)
        shares = vc.to_numpy()
        src5 = list(shares[:5]) + [0.0] * (5 - min(5, len(shares)))
        src_ent = _entropy(shares)
    else:
        src5, src_ent = [0.0] * 5, 0.0

    # Inter-event time per source (pooled gaps between sorted posting days)
    iets = []
    if "source" in df and pdt.notna().any():
        tmp = pd.DataFrame({"s": df["source"].astype(str), "d": pdt})
        for _, g in tmp.dropna().groupby("s"):
            days = np.sort(g["d"].astype("int64").to_numpy()) / 86_400_000_000_000
            if days.size > 1:
                iets.append(np.diff(days))
    if iets:
        allg = np.concatenate(iets)
        iet_mean, iet_std = float(allg.mean()), float(allg.std())
    else:
        iet_mean = iet_std = 0.0

    # GL account fan-out / concentration
    if "gl_account" in df:
        gvc = df["gl_account"].astype(str).value_counts(normalize=True)
        gl_n_log = float(np.log1p(len(gvc)))
        gl_top5 = float(gvc.to_numpy()[:5].sum())
        gl_ent = _entropy(gvc.to_numpy())
    else:
        gl_n_log = gl_top5 = gl_ent = 0.0

    n_docs = df["document_id"].nunique() if "document_id" in df else n
    feats = [
        la_mean, la_std, la_skew, benford_mad, round_frac,
        weekend_frac, monthend_frac, postclose_frac, manual_frac,
        lpje_mean, lpje_std, lpje_f2, lpje_fgt2,
        lag_mean, lag_std, lag_pos,
        *src5, src_ent,
        iet_mean, iet_std,
        gl_n_log, gl_top5, gl_ent,
        float(np.log1p(n)), float(np.log1p(n_docs)),
    ]
    return np.asarray(feats, dtype=np.float32)


def _set_dotted(cfg: dict, key: str, value) -> None:
    """Set a dotted key into a nested dict, creating intermediate dicts."""
    parts = key.split(".")
    node = cfg
    for p in parts[:-1]:
        node = node.setdefault(p, {})
    node[parts[-1]] = value


def run_one(cli: str, base_cfg: dict, theta: np.ndarray, seed: int, workdir: Path) -> np.ndarray | None:
    cfg = copy.deepcopy(base_cfg)
    for k, v in P.to_config_overrides(theta).items():
        _set_dotted(cfg, k, v)
    _set_dotted(cfg, "global.seed", int(seed))
    cfg_path = workdir / "cfg.yaml"
    out = workdir / "out"
    cfg_path.write_text(yaml.safe_dump(cfg))
    try:
        subprocess.run(
            [cli, "generate", "--config", str(cfg_path), "--output", str(out),
             "--max-threads", "1"],
            check=True, capture_output=True, timeout=300,
        )
        je = out / "journal_entries.csv"
        if not je.exists():
            return None
        return summary_stats(je)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
        return None
    finally:
        shutil.rmtree(out, ignore_errors=True)


def _worker(args) -> tuple[int, np.ndarray | None]:
    i, theta, seed, cli, base_cfg = args
    with tempfile.TemporaryDirectory(prefix="sbi_") as td:
        return i, run_one(cli, base_cfg, theta, seed, Path(td))


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--n", type=int, default=2000, help="number of (θ, x) pairs")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--base", type=Path, required=True, help="base generate config")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--workers", type=int, default=0, help="0 = os.cpu_count()-2")
    args = ap.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)

    workers = args.workers or max(1, (os.cpu_count() or 4) - 2)
    rng = np.random.default_rng(args.seed)
    thetas = P.sample_prior(rng, args.n)
    cli = _cli()
    base_cfg = yaml.safe_load(args.base.read_text())

    jobs = [(i, thetas[i], args.seed + 1 + i, cli, base_cfg) for i in range(args.n)]
    xs: dict[int, np.ndarray] = {}
    done = fail = 0
    with ProcessPoolExecutor(max_workers=workers) as ex:
        futs = [ex.submit(_worker, j) for j in jobs]
        for fut in as_completed(futs):
            i, x = fut.result()
            done += 1
            if x is None:
                fail += 1
            else:
                xs[i] = x
            if done % 50 == 0:
                print(f"[simulate] {done}/{args.n}  (failed {fail})", flush=True)

    keep = sorted(xs)
    if not keep:
        raise SystemExit("[simulate] all runs failed — check the base config / override keys")
    X = np.stack([xs[i] for i in keep])
    theta_keep = thetas[keep]
    np.savez(args.out / "pairs.npz", theta=theta_keep, x=X,
             param_names=[p.name for p in P.PARAMS], feature_names=FEATURE_NAMES)
    (args.out / "meta.json").write_text(json.dumps(
        {"n_requested": args.n, "n_kept": len(keep), "n_failed": fail,
         "dim_theta": P.dim(), "dim_x": int(X.shape[1])}, indent=2))
    print(f"[simulate] kept {len(keep)}/{args.n} (failed {fail}) -> {args.out/'pairs.npz'}")


if __name__ == "__main__":
    main()

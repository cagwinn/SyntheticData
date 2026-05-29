"""A1 OOD probe — how far is the corpus x from the synthetic x-cloud, OFF vs ON?

Training-free complement to inverse.apply: measures the Mahalanobis distance of the
corpus 29-dim summary vector to the synthetic campaign x-cloud, for the
ConcentrationPass-OFF and ON arms. Reported both on the full 29-dim space and on the
"shape-only" subspace that drops the two pure-scale features (`n_lines_log`,
`n_docs_log`) — the corpus is OOD on scale regardless of concentration (synthetic
~few-k lines vs corpus ~millions), so the shape subspace is where the fidelity work
can actually pull the corpus inside.

If ON < OFF (especially shape-only), ConcentrationPass tightened the manifold toward
the corpus — the Tier A headline. Aggregate-only (distances + per-feature z), never
row content.

    cd experiments/ml && ~/mlenv/bin/python inverse/ood_probe.py --root runs/A1
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np

_SCALE_FEATURES = ("n_lines_log", "n_docs_log")


def _maha(cloud: np.ndarray, x: np.ndarray) -> float:
    mu = cloud.mean(0)
    cov = np.cov(cloud.T) + 1e-6 * np.eye(cloud.shape[1])
    inv = np.linalg.pinv(cov)
    d = x - mu
    return float(np.sqrt(max(0.0, d @ inv @ d)))


def _topz(cloud: np.ndarray, x: np.ndarray, names: list[str], k: int = 6):
    mu, sd = cloud.mean(0), cloud.std(0) + 1e-9
    z = (x - mu) / sd
    order = np.argsort(-np.abs(z))[:k]
    return [(names[i], round(float(x[i]), 3), round(float(mu[i]), 3), round(float(z[i]), 2))
            for i in order]


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=Path("runs/A1"))
    ap.add_argument("--corpus-x", type=Path, default=None,
                    help="defaults to <root>/corpus_x29.json")
    a = ap.parse_args(argv)
    R = a.root
    corpus_x_path = a.corpus_x or (R / "corpus_x29.json")
    corpus = np.asarray(json.loads(corpus_x_path.read_text())["x"], dtype=float)

    out = {"corpus_x": str(corpus_x_path), "arms": {}}
    for arm in ("off", "on"):
        npz = R / f"pairs_{arm}" / "pairs.npz"
        if not npz.exists():
            print(f"[ood_probe] skip {arm}: {npz} missing")
            continue
        blob = np.load(npz)
        X = blob["x"].astype(float)
        names = [str(s) for s in blob["feature_names"]]
        scale_idx = [names.index(f) for f in _SCALE_FEATURES if f in names]
        shape_idx = [i for i in range(len(names)) if i not in scale_idx]
        out["arms"][arm] = {
            "n_sims": int(X.shape[0]),
            "maha_full": round(_maha(X, corpus), 3),
            "maha_shape_only": round(_maha(X[:, shape_idx], corpus[shape_idx]), 3),
            "top_z_shape": _topz(X[:, shape_idx], corpus[shape_idx],
                                 [names[i] for i in shape_idx]),
        }

    off, on = out["arms"].get("off"), out["arms"].get("on")
    if off and on:
        out["delta"] = {
            "maha_full": round(on["maha_full"] - off["maha_full"], 3),
            "maha_shape_only": round(on["maha_shape_only"] - off["maha_shape_only"], 3),
            "verdict": ("ON closer (concentration tightened manifold)"
                        if on["maha_shape_only"] < off["maha_shape_only"]
                        else "ON not closer on shape subspace"),
        }
    print(json.dumps(out, indent=2))
    (R / "ood_probe.json").write_text(json.dumps(out, indent=2))
    print(f"[ood_probe] -> {R/'ood_probe.json'}")


if __name__ == "__main__":
    main()

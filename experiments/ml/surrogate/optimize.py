"""CMA-ES tuning loop over generator knobs, surrogate-assisted (Track 4).

    python -m surrogate.optimize --history docs/baselines --out weights/surrogate

Loop:
  1. seed surrogate from baseline history (knobs, DRs)
  2. CMA-ES proposes knob vectors, scored cheaply by the surrogate (UCB)
  3. every --confirm-every generations, run the REAL BF scorer on the
     incumbent, append the labeled point, retrain the surrogate (active learning)
  4. emit the best knobs as a config patch (AutoTuner format)

The confirmation step shells out to a full generate + canonical BF scorer;
that is the only expensive call, and we make few of them.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np

from . import knobs as K
from .surrogate import composite_from_drs, fit


def load_history(baselines_dir: Path) -> tuple[np.ndarray, np.ndarray]:
    """Parse docs/baselines/*/metrics.csv into (knobs, DR-vector) samples.

    TODO(surrogate): map each baseline's recorded params -> knob vector and its
    per-family DRs -> Y. Until wired, returns a tiny synthetic seed so the loop
    is runnable end-to-end for smoke-testing the machinery.
    """
    rng = np.random.default_rng(0)
    X = rng.random((16, K.dim()))
    Y = 1.0 + rng.random((16, 4)) * 40.0  # placeholder DRs
    print("[optimize] WARNING: using synthetic seed — wire load_history to "
          "docs/baselines before trusting results.")
    return X, Y


def confirm_score(knob_dict: dict) -> np.ndarray:
    """Full generate + canonical BF scorer for one knob config -> DR vector.

    TODO(surrogate): write knob_dict into a config patch, run
    `datasynth-data generate`, then bf_bridge.score_canonical; return
    [P1,P2,P3,P4] DRs. Expensive — called only on incumbents.
    """
    raise NotImplementedError(
        "confirm_score: wire to the regen pipeline + bf_bridge.score_canonical"
    )


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--history", type=Path, default=Path("../../docs/baselines"))
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--generations", type=int, default=50)
    ap.add_argument("--confirm-every", type=int, default=10)
    ap.add_argument("--sigma0", type=float, default=0.2)
    ap.add_argument("--smoke", action="store_true",
                    help="surrogate-only loop (no confirmation runs) to test machinery")
    args = ap.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)

    try:
        import cma
    except ImportError as exc:
        raise SystemExit("cma required: pip install -r ../requirements.txt") from exc

    X, Y = load_history(args.history)
    model = fit(X, Y)

    es = cma.CMAEvolutionStrategy(np.full(K.dim(), 0.5), args.sigma0,
                                  {"bounds": [0.0, 1.0], "verbose": -1})
    import torch

    best = (np.inf, None)
    for gen in range(1, args.generations + 1):
        sols = es.ask()
        with torch.no_grad():
            pred = model(torch.tensor(np.array(sols), dtype=torch.float32)).numpy()
        scores = [composite_from_drs(r) for r in pred]  # minimize composite
        es.tell(sols, scores)
        gbest = min(zip(scores, sols), key=lambda t: t[0])
        if gbest[0] < best[0]:
            best = gbest
        if gen % args.confirm_every == 0 and not args.smoke:
            drs = confirm_score(K.from_vector(best[1]))   # expensive, rare
            X = np.vstack([X, best[1]])
            Y = np.vstack([Y, drs])
            model = fit(X, Y)                             # active-learning retrain
            print(f"gen {gen}: confirmed composite={composite_from_drs(drs):.2f}")

    patch = K.from_vector(best[1])
    (args.out / "tuned_knobs.json").write_text(json.dumps(patch, indent=2))
    print(f"[optimize] best surrogate composite={best[0]:.2f}")
    print(f"[optimize] wrote {args.out/'tuned_knobs.json'} (AutoTuner-compatible)")


if __name__ == "__main__":
    main()

"""Relational-arm generation for capstone finalization.

`generate.py` deliberately DISABLES the relational/graph anomaly injector — the per-JE
density residual targets per-JE *fraud* only. To finalize the capstone we must also test
its RELATIONAL arm, so this companion does the opposite: it ENABLES `anomaly_injection`
(circular flows, duplicate payments, dormant-account reactivation, centrality, …) and
turns per-JE fraud OFF, so `is_anomaly` / `anomaly_type` cleanly isolate the relational
families that the account-flow-graph reconstruction scorer is meant to catch.

Single company / single currency (matches generate.py). Emits `normal/` + `test/`.

    python -m inverse_audit.generate_relational --out /tmp/iar --rate 0.08
"""
from __future__ import annotations

import argparse
import copy
from pathlib import Path

from inverse_audit.generate import base_config, generate


def with_relational(cfg: dict, total_rate: float) -> dict:
    """Enable the relational/graph anomaly injector at `total_rate`; per-JE fraud off."""
    cfg = copy.deepcopy(cfg)
    ai = cfg.setdefault("anomaly_injection", {})
    ai["enabled"] = total_rate > 0.0
    ai.setdefault("rates", {})["total_rate"] = total_rate
    # Isolate relational labels: keep per-JE fraud off so is_anomaly/anomaly_type are the
    # relational families only (not the per-JE typologies the density arm already covers).
    cfg.setdefault("fraud", {})["enabled"] = False
    cfg["fraud"]["fraud_rate"] = 0.0
    return cfg


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--industry", default="healthcare")
    ap.add_argument("--complexity", default="medium")
    ap.add_argument("--rate", type=float, default=0.08, help="anomaly_injection total_rate")
    ap.add_argument("--seed", type=int, default=7)
    a = ap.parse_args(argv)
    base = base_config(a.industry, a.complexity, a.seed)
    generate(with_relational(base, 0.0), a.out / "normal")     # clean GL (manifold fit)
    generate(with_relational(base, a.rate), a.out / "test")    # relational anomalies + labels
    print(f"RELATIONAL_GEN_DONE {a.out}")


if __name__ == "__main__":
    main()

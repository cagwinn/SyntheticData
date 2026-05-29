"""Mixed-GL generation for the unified routed detector (I6).

generate.py turns relational anomalies OFF (per-JE fraud only). generate_relational.py
turns per-JE fraud OFF (relational only). This generator turns BOTH ON, so the resulting
GL carries `is_fraud`/`fraud_type` AND `is_anomaly`/`anomaly_type` labels simultaneously —
the substrate to measure the routed thesis: density catches per-JE fraud, relational
catches graph families, the union catches more than either.
"""
from __future__ import annotations

import argparse
import copy
from pathlib import Path

from inverse_audit.generate import base_config, generate


def with_both(cfg: dict, fraud_rate: float, anomaly_rate: float) -> dict:
    cfg = copy.deepcopy(cfg)
    f = cfg.setdefault("fraud", {})
    f["enabled"] = fraud_rate > 0.0
    f["fraud_rate"] = fraud_rate
    f["document_fraud_rate"] = 0.0
    ai = cfg.setdefault("anomaly_injection", {})
    ai["enabled"] = anomaly_rate > 0.0
    ai.setdefault("rates", {})["total_rate"] = anomaly_rate
    return cfg


def with_concentration(cfg: dict, pmf_path: str) -> dict:
    """Enable the #143 ConcentrationPass post-process so the GL's *normal*
    manifold is corpus-realistic (A2 "Stage 1.5"). Field names are validated
    against the schema; apply to BOTH normal and test so the relational arm
    fits + scores on a consistent manifold."""
    cfg = copy.deepcopy(cfg)
    cfg["concentration"] = {
        "enabled": True,
        "source_conditional_rarity": {"rate": 0.01},
        "trading_partner_pool": {"target_size": 12},
        "account_pair_substitution": {"pmf_path": pmf_path,
                                      "rarity_threshold": 0.005, "top_k": 10},
        "source_blanking": {"rate": 0.21},
        "consolidation_outlier": {"rate": 0.001},
    }
    return cfg


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--industry", default="healthcare")
    ap.add_argument("--complexity", default="medium")
    ap.add_argument("--fraud-rate", type=float, default=0.04)
    ap.add_argument("--anomaly-rate", type=float, default=0.06)
    ap.add_argument("--concentration", default=None,
                    help="PMF path → enable #143 ConcentrationPass on both GLs (A2 corpus-realism)")
    ap.add_argument("--seed", type=int, default=7)
    a = ap.parse_args(argv)
    base = base_config(a.industry, a.complexity, a.seed)
    normal = with_both(base, 0.0, 0.0)
    test = with_both(base, a.fraud_rate, a.anomaly_rate)
    if a.concentration:
        normal = with_concentration(normal, a.concentration)
        test = with_concentration(test, a.concentration)
    generate(normal, a.out / "normal")
    generate(test, a.out / "test")
    print(f"MIXED_GEN_DONE {a.out}")


if __name__ == "__main__":
    main()

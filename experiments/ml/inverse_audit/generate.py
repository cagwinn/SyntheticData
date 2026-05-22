"""Stage-1 capstone generation, fraud-driven: a *normal* GL (no fraud) to fit the
normal-system manifold, a *test* GL (~3% line-level fraud, labelled is_fraud) to
score, and a fraud_rate sweep for light-B. Single company / single currency.

Anomalies come from the FRAUD system (fraud.fraud_rate) — per-JE typologies
(RoundDollar, FictitiousVendor, WrongAccount, ...) carrying the fraud-bias
signatures (round-dollar / weekend / off-hours / post-close) a per-JE residual can
detect — NOT the relational/graph anomaly injector (whose anomalies are anomalous by
their cross-JE relationships, which a per-JE scorer cannot see; left disabled here).
fraud.fraud_rate is also exactly the parameter the inverse posterior infers (light-B).
"""
from __future__ import annotations
import argparse, copy, subprocess, tempfile
from pathlib import Path
import yaml


def base_config(industry: str, complexity: str, seed: int) -> dict:
    with tempfile.NamedTemporaryFile(suffix=".yaml", delete=False) as f:
        cfg_path = Path(f.name)
    subprocess.run(["datasynth-data", "init", "--industry", industry,
                    "--complexity", complexity, "-o", str(cfg_path)], check=True)
    cfg = yaml.safe_load(cfg_path.read_text())
    cfg.setdefault("global", {})["seed"] = seed
    if isinstance(cfg.get("companies"), list) and len(cfg["companies"]) > 1:
        cfg["companies"] = cfg["companies"][:1]                  # single company / USD
    cfg.setdefault("anomaly_injection", {})["enabled"] = False    # no relational anomalies
    return cfg


def set_fraud_rate(cfg: dict, rate: float) -> dict:
    cfg = copy.deepcopy(cfg)
    fr = cfg.setdefault("fraud", {})
    fr["enabled"] = rate > 0.0
    fr["fraud_rate"] = rate
    fr["document_fraud_rate"] = 0.0          # keep it clean line-level fraud
    return cfg


def generate(cfg: dict, out: Path) -> None:
    with tempfile.NamedTemporaryFile(suffix=".yaml", delete=False, mode="w") as f:
        yaml.safe_dump(cfg, f)
        cfg_path = f.name
    subprocess.run(["datasynth-data", "generate", "--config", cfg_path,
                    "--output", str(out)], check=True)


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--industry", default="healthcare")
    ap.add_argument("--complexity", default="medium")
    ap.add_argument("--test-rate", type=float, default=0.03)
    ap.add_argument("--sweep", type=float, nargs="*", default=[0.0, 0.02, 0.05, 0.10])
    ap.add_argument("--seed", type=int, default=7)
    a = ap.parse_args(argv)
    base = base_config(a.industry, a.complexity, a.seed)
    generate(set_fraud_rate(base, 0.0), a.out / "normal")
    generate(set_fraud_rate(base, a.test_rate), a.out / "test")
    for r in a.sweep:
        generate(set_fraud_rate(base, r), a.out / f"sweep_{int(round(r * 100)):02d}")
    print("GENERATE_DONE")


if __name__ == "__main__":
    main()

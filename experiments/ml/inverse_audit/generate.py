"""Stage-1 capstone generation: a *normal* GL (no anomalies) to fit the
normal-system manifold, a *test* GL (~3% injected, labelled) to score, and a
rate sweep for light-B. Single company / single currency (Stage 1: no FX) — we
trim the company list to the first (USD) entry so EUR/foreign JEs don't appear.

Anomaly config (confirmed from `datasynth-data init`):
  anomaly_injection.enabled: bool
  anomaly_injection.rates: {total_rate, fraud_rate, error_rate, process_rate}
The default split is 0.01:0.015:0.005 (= 2:3:1, summing to total 0.03); we keep
that ratio and scale to the requested total.
"""
from __future__ import annotations
import argparse
import copy
import subprocess
import tempfile
from pathlib import Path

import yaml


def base_config(industry: str, complexity: str, seed: int) -> dict:
    with tempfile.NamedTemporaryFile(suffix=".yaml", delete=False) as f:
        cfg_path = Path(f.name)
    subprocess.run(
        ["datasynth-data", "init", "--industry", industry,
         "--complexity", complexity, "-o", str(cfg_path)],
        check=True,
    )
    cfg = yaml.safe_load(cfg_path.read_text())
    cfg.setdefault("global", {})["seed"] = seed
    # Stage 1: single company, single currency. Keep the first (USD) company.
    if isinstance(cfg.get("companies"), list) and len(cfg["companies"]) > 1:
        cfg["companies"] = cfg["companies"][:1]
    return cfg


def set_anomaly_rate(cfg: dict, total: float) -> dict:
    cfg = copy.deepcopy(cfg)
    ai = cfg.setdefault("anomaly_injection", {})
    ai["enabled"] = total > 0.0
    rates = ai.setdefault("rates", {})
    rates["total_rate"] = total
    rates["fraud_rate"] = total * 2.0 / 6.0
    rates["error_rate"] = total * 3.0 / 6.0
    rates["process_rate"] = total * 1.0 / 6.0
    return cfg


def generate(cfg: dict, out: Path) -> None:
    with tempfile.NamedTemporaryFile(suffix=".yaml", delete=False, mode="w") as f:
        yaml.safe_dump(cfg, f)
        cfg_path = f.name
    subprocess.run(
        ["datasynth-data", "generate", "--config", cfg_path, "--output", str(out)],
        check=True,
    )


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
    generate(set_anomaly_rate(base, 0.0), a.out / "normal")
    generate(set_anomaly_rate(base, a.test_rate), a.out / "test")
    for r in a.sweep:
        generate(set_anomaly_rate(base, r), a.out / f"sweep_{int(round(r * 100)):02d}")
    print("GENERATE_DONE")


if __name__ == "__main__":
    main()

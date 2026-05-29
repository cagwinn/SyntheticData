"""Build a small, fast generate config for the inverse/surrogate forward
campaign. Enables only what the tier-1 knobs touch (fraud + distributions) and
shrinks to one period so each forward sim is quick. The campaign overrides
fraud_rate + the amount mixture per draw (see params.to_config_overrides).

    python -m inverse.make_base --out inverse_base.yaml
"""
from __future__ import annotations

import argparse
import shutil
import subprocess
from pathlib import Path

import yaml


def _cli() -> str:
    for c in ("./target/release/datasynth-data", "../../target/release/datasynth-data",
              "datasynth-data"):
        if Path(c).exists() or shutil.which(c):
            return c
    raise SystemExit("datasynth-data not found — build with cargo build --release -p datasynth-cli")


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", type=Path, default=Path("inverse_base.yaml"))
    ap.add_argument("--industry", default="manufacturing")
    a = ap.parse_args(argv)

    tmp = Path("/tmp/_inv_init.yaml")
    subprocess.run([_cli(), "init", "--industry", a.industry, "--complexity", "small",
                    "-o", str(tmp)], check=True, capture_output=True)
    c = yaml.safe_load(tmp.read_text())

    if isinstance(c.get("fraud"), dict):
        c["fraud"]["enabled"] = True
    if isinstance(c.get("distributions"), dict):
        c["distributions"]["enabled"] = True
        amt = c["distributions"].setdefault("amounts", {})
        amt["enabled"] = True
        amt["distribution_type"] = "log_normal"
        # Force (not setdefault): `init` now emits `components: []`, which fails
        # `validate` ("components cannot be empty when enabled"). simulate.py
        # overrides this per-θ anyway, but keep the bare base valid.
        amt["components"] = [{"weight": 1.0, "mu": 7.0, "sigma": 1.2, "label": "base"}]
    c.setdefault("global", {})["period_months"] = 1

    a.out.write_text(yaml.safe_dump(c))
    print(f"wrote {a.out} (fraud + distributions enabled, 1 month)")


if __name__ == "__main__":
    main()

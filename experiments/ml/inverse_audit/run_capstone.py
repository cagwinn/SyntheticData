"""End-to-end Stage-1 capstone reproduction with the relational arm.

One command reproduces the FINDINGS §12 numbers: generate a mixed GL (per-JE fraud +
relational anomalies both labelled), run the density scorer (§11 per-JE NLL residual),
run the relational graph-manifold scorer, and emit the unified routed detector + the
per-family observability map.

    python -m inverse_audit.run_capstone --root /tmp/ia_full

Outputs:
  {root}/normal/  {root}/test/        — the mixed GL (with prov.json + manifest)
  {root}/density_scores.parquet       — per-JE density residual + labels
  {root}/graph_scores.parquet         — per-JE relational features + relational_score
  {root}/unified.json                 — three-arm results + observability map
"""
from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


def sh(mod: str, *args: str) -> None:
    subprocess.run([sys.executable, "-m", mod, *args], check=True)


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=Path("/tmp/ia_full"))
    ap.add_argument("--industry", default="healthcare")
    ap.add_argument("--complexity", default="medium")
    ap.add_argument("--fraud-rate", type=float, default=0.04)
    ap.add_argument("--anomaly-rate", type=float, default=0.06)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--skip-generate", action="store_true",
                    help="reuse existing normal/ + test/ under --root")
    a = ap.parse_args(argv)
    R = a.root
    R.mkdir(parents=True, exist_ok=True)

    if not a.skip_generate:
        sh("inverse_audit.generate_mixed",
           "--out", str(R),
           "--industry", a.industry,
           "--complexity", a.complexity,
           "--fraud-rate", str(a.fraud_rate),
           "--anomaly-rate", str(a.anomaly_rate),
           "--seed", str(a.seed))

    # density + relational + unified + observability map (unified_score shells out to both arms)
    sh("inverse_audit.unified_score", "--root", str(R), "--out", str(R / "unified.json"))

    print()
    print("=" * 64)
    print(f"CAPSTONE_DONE — three-arm routed detector measured.")
    print(f"  raw GL          : {R}/normal,  {R}/test")
    print(f"  density scores  : {R}/density_scores.parquet")
    print(f"  graph scores    : {R}/graph_scores.parquet")
    print(f"  routing result  : {R}/unified.json   (reproduces FINDINGS §12)")


if __name__ == "__main__":
    main()

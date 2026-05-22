"""End-to-end Stage-1 runner: generate → score → assess → light-B → results.json.
Each stage is shelled so a failure is localised."""
from __future__ import annotations
import argparse, json, subprocess, sys
from pathlib import Path


def sh(mod: str, *args: str) -> None:
    subprocess.run([sys.executable, "-m", mod, *args], check=True)


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=Path("/tmp/ia"))
    ap.add_argument("--weights", type=Path, default=Path("inverse/weights"))
    ap.add_argument("--skip-generate", action="store_true")
    a = ap.parse_args(argv); R = a.root
    if not a.skip_generate:
        sh("inverse_audit.generate", "--out", str(R))
    sh("inverse_audit.score", "--normal", str(R / "normal"), "--test", str(R / "test"),
       "--out", str(R / "scores.parquet"))
    sh("inverse_audit.assess", "--scores", str(R / "scores.parquet"),
       "--test-gl", str(R / "test"), "--out", str(R / "assess.json"))
    sh("inverse_audit.lightb", "--sweep-root", str(R), "--weights", str(a.weights),
       "--out", str(R / "lightb.json"))
    results = {"assess": json.loads((R / "assess.json").read_text()),
               "lightb": json.loads((R / "lightb.json").read_text())}
    (R / "results.json").write_text(json.dumps(results, indent=2))
    print("RUN_DONE")


if __name__ == "__main__":
    main()

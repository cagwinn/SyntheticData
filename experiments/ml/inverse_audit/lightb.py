"""Light-B: run the amortized SBI posterior (inverse.apply) on GLs at varying
injected anomaly rates; show the inferred fraud.fraud_rate tracks the injected rate
(the global system-state moves observably). Skips gracefully if no posterior."""
from __future__ import annotations
import argparse, json, subprocess, sys
from pathlib import Path


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--sweep-root", type=Path, required=True)
    ap.add_argument("--weights", type=Path, required=True)   # dir containing posterior.pt
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args(argv)
    if not (a.weights / "posterior.pt").exists():
        a.out.write_text(json.dumps({"skipped": "no posterior.pt"}, indent=2))
        print("LIGHTB_SKIPPED no posterior.pt"); return
    rows = []
    for d in sorted(a.sweep_root.glob("sweep_*")):
        rate = int(d.name.split("_")[1]) / 100.0
        post_out = d / "posterior.json"
        subprocess.run([sys.executable, "-m", "inverse.apply", "--weights", str(a.weights),
                        "--gl-canonical", str(d / "journal_entries.csv"), "--out", str(post_out)],
                       check=True)
        post = json.loads(post_out.read_text())
        fr = post.get("fraud.fraud_rate", {})
        rows.append({"injected": rate, "fraud_rate_median": fr.get("median"), "ci90": fr.get("ci90")})
    a.out.write_text(json.dumps(rows, indent=2))
    print(json.dumps(rows, indent=2)); print("LIGHTB_DONE")


if __name__ == "__main__":
    main()

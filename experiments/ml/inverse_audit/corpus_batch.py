"""Stage 2 — batch the corpus_runner over multiple GL parquets and aggregate the
per-file summaries into a cross-corpus shape table. Privacy guarantees mirror
corpus_runner: paths are runtime args only (never in commits), output dirs are tagged
by SHA-1 of the source filename (not by client name), only summary.json + the top-1%
JE ID list per file leave the box, and the aggregate cross-corpus JSON contains tags
+ aggregate statistics only (no client names, no row content).

Usage:
    python -m inverse_audit.corpus_batch \\
        --parquet-glob "/path/to/Client Data/*.parquet" \\
        --out-root /tmp/stage2_batch \\
        --mode half-split  --min-size-mb 1  --limit 10
"""
from __future__ import annotations

import argparse
import glob
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--parquet-glob", required=True,
                    help="glob of corpus parquets (runtime only — never committed)")
    ap.add_argument("--out-root", type=Path, required=True)
    ap.add_argument("--mode", choices=("self", "half-split"), default="half-split")
    ap.add_argument("--min-size-mb", type=float, default=1.0,
                    help="skip files smaller than this (registries etc.)")
    ap.add_argument("--limit", type=int, default=None,
                    help="cap on # of files to process this batch")
    ap.add_argument("--export-graph", action="store_true",
                    help="also emit substrate JSON per file (larger output)")
    a = ap.parse_args(argv)

    files = sorted(glob.glob(a.parquet_glob), key=os.path.getsize)
    files = [f for f in files if os.path.getsize(f) >= a.min_size_mb * 1e6]
    if a.limit:
        files = files[: a.limit]
    a.out_root.mkdir(parents=True, exist_ok=True)
    print(f"batch: {len(files)} files, mode={a.mode}, out={a.out_root}")

    summaries: list[dict] = []
    for i, f in enumerate(files, 1):
        tag = hashlib.sha1(os.path.basename(f).encode()).hexdigest()[:8]
        sz_mb = os.path.getsize(f) / 1e6
        out = a.out_root / f"file_{tag}"
        print(f"[{i}/{len(files)}] size={sz_mb:5.1f}MB tag={tag} ", end="", flush=True)
        cmd = [sys.executable, "-m", "inverse_audit.corpus_runner",
               "--parquet", f, "--out", str(out), "--mode", a.mode]
        if a.export_graph:
            cmd.append("--export-graph")
        rc = subprocess.run(cmd, capture_output=True, text=True)
        if rc.returncode != 0:
            print(f"FAILED ({rc.returncode}); skipping. tail: {rc.stderr.strip()[-200:]}")
            continue
        try:
            s = json.loads((out / "summary.json").read_text())
        except FileNotFoundError:
            print("NO summary.json")
            continue
        s["tag"] = tag; s["size_mb"] = sz_mb
        summaries.append(s)
        rs = s.get("relational_score", {})
        pcts = rs.get("percentiles", {})
        print(f"jes={s.get('n_scored_jes', 0):>7,} "
              f"rs_p99={pcts.get('p99', 0):5.2f} rs_max={rs.get('max', 0):5.2f}")

    (a.out_root / "cross_corpus.json").write_text(json.dumps(summaries, indent=2))
    print(f"\ncross-corpus aggregate -> {a.out_root}/cross_corpus.json "
          f"({len(summaries)} files)")


if __name__ == "__main__":
    main()

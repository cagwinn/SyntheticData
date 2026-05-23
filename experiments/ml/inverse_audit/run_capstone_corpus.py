"""One-shot Stage 2 pipeline — corpus parquet → relational scoring + substrate graph
JSON + auditor-ready packet. The corpus counterpart to `run_capstone.py` (which runs
the synthetic Stage 1 end-to-end).

    python -m inverse_audit.run_capstone_corpus \\
        --parquet  <corpus parquet>             (runtime arg; never committed)
        --out      <local out dir>              (gitignored)
        --mode     half-split | self            (default half-split)
        --top-n    50                           (audit packet size)

Outputs (all local, never committed):
    {out}/graph_scores.parquet         per-JE relational features + score
    {out}/summary.json                 aggregate statistics
    {out}/top1pct_je_ids.json          JE IDs only (no row content)
    {out}/account_flow_graph.json      decoupled substrate graph (no row content)
    {out}/audit_packet.json + .md      auditor-ready triage list (contains row
                                       content — stays local, gitignored)

Privacy: paths and row content live in runtime args + local outputs; no commits.
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
    ap.add_argument("--parquet", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--mode", choices=("self", "half-split"), default="half-split")
    ap.add_argument("--top-n", type=int, default=50, help="audit packet size")
    a = ap.parse_args(argv)
    a.out.mkdir(parents=True, exist_ok=True)

    # 1. Relational scoring + substrate JSON (corpus_runner does both with --export-graph).
    sh("inverse_audit.corpus_runner",
       "--parquet", str(a.parquet), "--out", str(a.out),
       "--mode", a.mode, "--export-graph")

    # 2. Auditor-ready packet (top-N by relational_score with original GL lines).
    sh("inverse_audit.audit_packet",
       "--gl-parquet", str(a.parquet),
       "--scored-parquet", str(a.out / "graph_scores.parquet"),
       "--out", str(a.out / "audit"),
       "--top-n", str(a.top_n))

    print()
    print("=" * 64)
    print("CORPUS_CAPSTONE_DONE — Stage 2 pipeline complete")
    print(f"  relational scoring : {a.out}/graph_scores.parquet")
    print(f"  aggregate summary  : {a.out}/summary.json")
    print(f"  substrate graph    : {a.out}/account_flow_graph.json")
    print(f"  audit packet       : {a.out}/audit/audit_packet.json + .md")


if __name__ == "__main__":
    main()

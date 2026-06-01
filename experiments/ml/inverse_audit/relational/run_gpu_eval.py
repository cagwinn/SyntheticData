"""GPU-VM entrypoint for the rung-2 OT within-JE reconstruction evaluation.

One command on a CUDA VM produces the full rung-2 OT report:
  1. PARITY  — the torch GPU Sinkhorn reproduces the numpy reference to < 1e-4 (ot_gpu selftest).
  2. THROUGHPUT — reconstruct one GL with each backend (perje numpy / batched numpy / batched torch)
                  and report wall-time + speedup. This is why the GPU is needed: corpus JEs reach
                  ~2800 lines, so the per-JE O(m·n·iters) Sinkhorn is GPU-bound at scale.
  3. RUNG-1 vs RUNG-2 — coupling-entropy reduction from the learned within-JE cost on real multi-line
                  structure (the rung-2 signal that is invisible on tiny synthetic JEs).
Optionally, if --synthetic-normal/--synthetic-test are given (a labelled GL), it also runs the full
ot_eval detector-impact comparison (PR-AUC per relational family).

Privacy: corpus parquet paths are runtime arguments only; the emitted report carries aggregate
statistics (timings, entropy percentiles, per-family ROC) — never client identity, paths, amounts,
or row content. Run inside the experiments venv with torch+CUDA (see requirements-gpu.txt).

    python -m inverse_audit.relational.run_gpu_eval --gl-parquet <corpus.parquet> \
        --size-cap 1024 --device cuda --out /tmp/rung2_gpu_report.json
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import pandas as pd

from inverse_audit.relational.ot_cost import _two_line_edges, learn_edge_cost, make_cost_fn
from inverse_audit.relational.ot_flow import reconstruct_per_je
from inverse_audit.relational.ot_gpu import _HAS_TORCH, reconstruct_per_je_gpu


def _load_gl(parquet: Path) -> pd.DataFrame:
    from inverse_audit.corpus_runner import load_canonical
    return load_canonical(parquet)


def _mean_entropy(per: dict) -> float:
    return float(np.mean([h for _flows, h in per.values()])) if per else 0.0


def main(argv=None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--gl-parquet", type=Path, required=True,
                    help="one corpus GL parquet for throughput + entropy (runtime only)")
    ap.add_argument("--size-cap", type=int, default=2048,
                    help="skip JEs with >cap lines/side (pathological batch JEs; corpus has up to ~100k)")
    ap.add_argument("--device", default="cuda")
    ap.add_argument("--throughput-backends", default="perje,numpy,torch")
    ap.add_argument("--out", type=Path, default=Path("/tmp/rung2_gpu_report.json"))
    a = ap.parse_args(argv)

    report: dict = {"torch_available": _HAS_TORCH}
    if _HAS_TORCH:
        import torch
        report["cuda_available"] = torch.cuda.is_available()
        report["device_name"] = torch.cuda.get_device_name(0) if torch.cuda.is_available() else None
        print(f"torch {torch.__version__} | cuda {torch.cuda.is_available()} | "
              f"{report.get('device_name')}")

    # 1. PARITY (numpy-batched always; torch if available)
    print("\n[1] PARITY — batched Sinkhorn vs ot_flow reference")
    for be in (["numpy", "torch"] if _HAS_TORCH else ["numpy"]):
        r = subprocess.run([sys.executable, "-m", "inverse_audit.relational.ot_gpu",
                            "--selftest", "--backend", be], capture_output=True, text=True)
        ok = "OT_GPU_SELFTEST_OK" in r.stdout
        report.setdefault("parity", {})[be] = "OK" if ok else "FAIL"
        print(f"  {be:6s}: {'OK' if ok else 'FAIL'}  {r.stdout.strip().splitlines()[-2] if ok else r.stdout[-300:]}")

    # load GL + learn rung-2 cost
    df = _load_gl(a.gl_parquet)
    lpje = df.groupby("document_id").size()
    edges = _two_line_edges(df)
    cmap, dflt = learn_edge_cost(edges)
    cost_fn = make_cost_fn(cmap, dflt)
    report["gl"] = {"n_lines": int(len(df)), "n_jes": int(df["document_id"].nunique()),
                    "lines_per_je_p50": int(lpje.median()), "lines_per_je_p99": int(lpje.quantile(0.99)),
                    "lines_per_je_max": int(lpje.max()), "n_two_line_gt_edges": len(edges)}
    print(f"\nGL: {len(df):,} lines / {df['document_id'].nunique():,} JEs | "
          f"lines/JE p50={int(lpje.median())} p99={int(lpje.quantile(0.99))} max={int(lpje.max())} | "
          f"2-line GT edges={len(edges)}")

    # 2. THROUGHPUT — reconstruct (uniform cost) with each backend, time it
    print("\n[2] THROUGHPUT — within-JE reconstruction wall-time by backend")
    report["throughput"] = {}
    base_h = None
    for be in a.throughput_backends.split(","):
        be = be.strip()
        if be == "torch" and not _HAS_TORCH:
            print("  torch: skipped (not installed)"); continue
        t0 = time.time()
        if be == "perje":
            per = reconstruct_per_je(df)
        else:
            per = reconstruct_per_je_gpu(df, backend=("torch" if be == "torch" else "numpy"),
                                         device=a.device, size_cap=a.size_cap)
        dt = time.time() - t0
        h = _mean_entropy(per)
        if base_h is None:
            base_h = h
        report["throughput"][be] = {"seconds": round(dt, 2), "mean_entropy": round(h, 5),
                                    "entropy_matches_ref": bool(abs(h - base_h) < 1e-4)}
        print(f"  {be:6s}: {dt:7.2f}s  mean_entropy={h:.5f}")
    tp = report["throughput"]
    if "perje" in tp and "torch" in tp and tp["torch"]["seconds"] > 0:
        report["gpu_speedup_vs_perje"] = round(tp["perje"]["seconds"] / tp["torch"]["seconds"], 1)
        print(f"  -> GPU speedup vs per-JE numpy: {report['gpu_speedup_vs_perje']}x")

    # 3. RUNG-1 vs RUNG-2 entropy (learned cost resolves multi-line pairing more confidently)
    print("\n[3] RUNG-1 (uniform) vs RUNG-2 (learned cost) — coupling entropy")
    be = "torch" if _HAS_TORCH else "numpy"
    sk = {}
    per1 = reconstruct_per_je_gpu(df, cost_fn=None, backend=be, device=a.device, size_cap=a.size_cap, skipped=sk)
    per2 = reconstruct_per_je_gpu(df, cost_fn=cost_fn, backend=be, device=a.device, size_cap=a.size_cap)
    report["skipped_pathological_jes"] = {"n_skipped": sk.get("n_skipped", 0),
                                          "max_lines_seen": sk.get("max_seen", 0), "size_cap": a.size_cap}
    print(f"  skipped {sk.get('n_skipped',0)} pathological JEs (max lines seen={sk.get('max_seen',0)}, cap={a.size_cap})")
    # restrict to multi-line JEs (where the polytope is non-trivial; 2-line JEs are forced)
    multi = set(lpje[lpje > 2].index.astype(str))
    h1 = np.mean([h for j, (_f, h) in per1.items() if j in multi]) if multi else 0.0
    h2 = np.mean([h for j, (_f, h) in per2.items() if j in multi]) if multi else 0.0
    report["rung1_vs_rung2"] = {"backend": be, "n_multi_line": len(multi),
                                "mean_entropy_rung1": round(float(h1), 4),
                                "mean_entropy_rung2": round(float(h2), 4),
                                "entropy_reduction_pct": round(100 * (h1 - h2) / max(h1, 1e-9), 1)}
    print(f"  multi-line JEs={len(multi)}  rung1={h1:.4f}  rung2={h2:.4f}  "
          f"reduction={100*(h1-h2)/max(h1,1e-9):.1f}%")

    a.out.write_text(json.dumps(report, indent=2))
    print(f"\nRUNG2_GPU_REPORT_DONE -> {a.out}")


if __name__ == "__main__":
    main()

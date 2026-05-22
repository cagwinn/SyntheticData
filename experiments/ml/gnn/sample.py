"""Sample a new relational scaffold from the trained GAE (Track 1).

    python -m gnn.sample --weights weights/gnn/gae.pt --out weights/gnn/scaffold.parquet

Emits the artifact the Rust generator consumes: an anonymized edge list +
per-node source-mix. No corpus content — only the learned structure.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import torch


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--weights", type=Path, required=True)
    ap.add_argument("--data", type=Path, required=True,
                    help="dir with node_feat.pt / edge_index.pt used at train time")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--target-sparsity", type=float, default=None,
                    help="calibrated edge density; default = corpus density")
    ap.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    ap.parse_args(argv)  # validate args; sampler body is a scaffold TODO below

    raise NotImplementedError(
        "TODO(gnn): load GAE, encode nodes, draw a degree sequence from the "
        "fitted tail, realize edges by thresholding sigmoid(z_i·z_j) to hit "
        "--target-sparsity, write out/scaffold.parquet (anonymized edge list + "
        "per-node source-mix). Then validate clustering/triangle stats with "
        "common.bf_bridge before handing to the generator."
    )


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Convert generation outputs into the HF-ready layout for the
`vynfi-audit-p2p` dataset.

Each document-flow JSON file is flattened — header fields are
hoisted to the top level — and written as a single-shard parquet
under its own subdirectory so HF Datasets surfaces each document
type as a separate `config`:

    purchase_orders/train-00000-of-00001.parquet
    goods_receipts/train-00000-of-00001.parquet
    vendor_invoices/train-00000-of-00001.parquet
    payments/train-00000-of-00001.parquet

Items / line-level arrays are dropped from the top-level table —
they are available in the row-level `journal_entries.csv` (joinable
via `document_id`) and on the corresponding `je_network.csv` flat
edge-list (v5.8.0+).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import pandas as pd
import pyarrow as pa
import pyarrow.parquet as pq


def flatten_document(doc: dict) -> dict:
    """Hoist `header.*` and discard nested arrays so the row is flat."""
    out: dict = {}
    for k, v in doc.items():
        if k == "header" and isinstance(v, dict):
            for hk, hv in v.items():
                out[hk] = hv
        elif k == "items":
            out["item_count"] = len(v) if isinstance(v, list) else 0
        elif k == "lines":
            out["line_count"] = len(v) if isinstance(v, list) else 0
        elif isinstance(v, (dict, list)):
            # Skip remaining nested structures — they don't fit cleanly
            # into a flat schema.  The corresponding JSON file remains
            # the source of truth.
            continue
        else:
            out[k] = v
    return out


def doc_flow_json_to_parquet(json_path: Path, out_path: Path, label: str) -> int:
    with open(json_path) as f:
        data = json.load(f)
    if isinstance(data, dict):
        for key in (
            "purchase_orders",
            "goods_receipts",
            "vendor_invoices",
            "payments",
            "sales_orders",
            "deliveries",
            "customer_invoices",
            "customer_receipts",
            "records",
            "items",
        ):
            if key in data and isinstance(data[key], list):
                data = data[key]
                break
    if not isinstance(data, list) or not data:
        print(f"  {label}: empty, skipping")
        return 0

    rows = [flatten_document(doc) for doc in data]
    df = pd.json_normalize(rows, max_level=0)

    out_path.parent.mkdir(parents=True, exist_ok=True)
    table = pa.Table.from_pandas(df, preserve_index=False)
    pq.write_table(table, out_path, compression="zstd", compression_level=9)
    size_kb = out_path.stat().st_size / 1024
    print(
        f"  wrote {out_path.parent.name}/{out_path.name}: "
        f"{len(df):,} rows x {len(df.columns)} cols, {size_kb:.1f} KB ({label})"
    )
    return len(df)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--output-dir", required=True)
    ap.add_argument("--hf-dir", required=True)
    args = ap.parse_args()

    out = Path(args.output_dir) / "document_flows"
    hf = Path(args.hf_dir)
    hf.mkdir(parents=True, exist_ok=True)

    if not out.exists():
        print(
            f"ERROR: {out} not found (regen with document_flows enabled)",
            file=sys.stderr,
        )
        return 1

    targets = [
        ("purchase_orders.json", "purchase_orders", "Purchase Orders"),
        ("goods_receipts.json", "goods_receipts", "Goods Receipts"),
        ("vendor_invoices.json", "vendor_invoices", "Vendor Invoices"),
        ("payments.json", "payments", "Payments"),
    ]
    for fname, subdir, label in targets:
        src = out / fname
        if not src.exists():
            print(f"  {label}: source {src} missing, skipping")
            continue
        dst = hf / subdir / "train-00000-of-00001.parquet"
        doc_flow_json_to_parquet(src, dst, label)

    print(f"\nAll artefacts written to {hf}/")
    return 0


if __name__ == "__main__":
    sys.exit(main())

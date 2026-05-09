#!/usr/bin/env python3
"""Convert generation outputs into the HF-ready layout for the
`vynfi-supply-chain-ocel` dataset.

Each table goes under its own subdirectory so HF Datasets surfaces
it as a separate `config`:

    events/train-00000-of-00001.parquet
    objects/train-00000-of-00001.parquet
    anomaly_labels/train-00000-of-00001.parquet
    document_events/train-00000-of-00001.parquet

`events.json` and `objects.json` come from `process_mining/`;
`anomaly_labels.json` from `labels/`; `document_events` is
synthesised from the document-flow headers (one row per
P2P/O2C document with the OCEL-style fields needed for joins).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import pandas as pd
import pyarrow as pa
import pyarrow.parquet as pq


def unwrap_list(data, *keys):
    if isinstance(data, dict):
        for k in keys:
            if k in data and isinstance(data[k], list):
                return data[k]
    if isinstance(data, list):
        return data
    return []


def write_parquet(rows: list[dict], out_path: Path, label: str) -> int:
    if not rows:
        print(f"  {label}: empty, skipping")
        return 0
    df = pd.json_normalize(rows, max_level=1)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    table = pa.Table.from_pandas(df, preserve_index=False)
    pq.write_table(table, out_path, compression="zstd", compression_level=9)
    size_kb = out_path.stat().st_size / 1024
    print(
        f"  wrote {out_path.parent.name}/{out_path.name}: "
        f"{len(df):,} rows x {len(df.columns)} cols, "
        f"{size_kb:.1f} KB ({label})"
    )
    return len(df)


def synthesize_document_events(out_root: Path) -> list[dict]:
    """Build a per-document summary table from the four P2P/O2C flows.

    Mirrors the v0.x `document_events` config: one row per (P2P or
    O2C) document with the OCEL-style fields needed to join events
    back to source documents.
    """
    rows: list[dict] = []
    flows = [
        ("purchase_orders", "PO", "P2P"),
        ("goods_receipts", "GR", "P2P"),
        ("vendor_invoices", "VI", "P2P"),
        ("payments", "PAY", "P2P"),
        ("sales_orders", "SO", "O2C"),
        ("deliveries", "DLV", "O2C"),
        ("customer_invoices", "CI", "O2C"),
        ("customer_receipts", "CR", "O2C"),
    ]
    df_dir = out_root / "document_flows"
    if not df_dir.exists():
        return rows
    for fname, doc_kind, process in flows:
        fp = df_dir / f"{fname}.json"
        if not fp.exists():
            continue
        with open(fp) as f:
            data = json.load(f)
        for doc in unwrap_list(
            data,
            "purchase_orders",
            "goods_receipts",
            "vendor_invoices",
            "payments",
            "sales_orders",
            "deliveries",
            "customer_invoices",
            "customer_receipts",
            "records",
        ):
            header = doc.get("header", {})
            rows.append(
                {
                    "document_id": header.get("document_id"),
                    "document_kind": doc_kind,
                    "process": process,
                    "company_code": header.get("company_code"),
                    "posting_date": header.get("posting_date"),
                }
            )
    return rows


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--output-dir", required=True)
    ap.add_argument("--hf-dir", required=True)
    args = ap.parse_args()

    out = Path(args.output_dir)
    hf = Path(args.hf_dir)
    hf.mkdir(parents=True, exist_ok=True)

    if not (out / "process_mining").exists():
        print(
            f"ERROR: {out / 'process_mining'} not found "
            "(regen with ocpm.enabled: true)",
            file=sys.stderr,
        )
        return 1

    # 1. events
    with open(out / "process_mining" / "events.json") as f:
        events = unwrap_list(json.load(f), "events", "records")
    write_parquet(
        events,
        hf / "events" / "train-00000-of-00001.parquet",
        "OCEL Events",
    )

    # 2. objects
    with open(out / "process_mining" / "objects.json") as f:
        objects = unwrap_list(json.load(f), "objects", "records")
    write_parquet(
        objects,
        hf / "objects" / "train-00000-of-00001.parquet",
        "OCEL Objects",
    )

    # 3. anomaly_labels (from labels/anomaly_labels.json)
    al_path = out / "labels" / "anomaly_labels.json"
    if al_path.exists():
        with open(al_path) as f:
            labels = unwrap_list(json.load(f), "labels", "anomaly_labels", "records")
        write_parquet(
            labels,
            hf / "anomaly_labels" / "train-00000-of-00001.parquet",
            "Anomaly Labels",
        )

    # 4. document_events (synthesised from the four flow headers)
    doc_events = synthesize_document_events(out)
    write_parquet(
        doc_events,
        hf / "document_events" / "train-00000-of-00001.parquet",
        "Document Events",
    )

    print(f"\nAll artefacts written to {hf}/")
    return 0


if __name__ == "__main__":
    sys.exit(main())

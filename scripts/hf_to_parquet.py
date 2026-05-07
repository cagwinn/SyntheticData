#!/usr/bin/env python3
"""
Convert v5.5.1 generation outputs into HF-ready parquet artefacts.

Inputs (under --output-dir):
  journal_entries.csv                   -> data/train-{NNNNN}-of-{TOTAL}.parquet
  chart_of_accounts.json                -> chart_of_accounts.parquet
  period_close/trial_balances.json      -> trial_balances.parquet
  master_data/cost_centers.json         -> cost_centers.parquet (optional)
  master_data/profit_centers.json       -> profit_centers.parquet (optional)

The JE table is sharded so each shard is <= ~150 MB compressed.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

import pandas as pd
import pyarrow as pa
import pyarrow.parquet as pq


def shard_je_csv(csv_path: Path, out_dir: Path, target_shards: int = 3) -> int:
    """Stream-read JE CSV in chunks, write `target_shards` parquet files."""
    out_dir.mkdir(parents=True, exist_ok=True)

    # First pass — count rows so shard boundaries can be planned.
    n_rows = sum(1 for _ in open(csv_path)) - 1
    rows_per_shard = (n_rows + target_shards - 1) // target_shards
    print(f"  JE rows: {n_rows:,}; target shards: {target_shards} (~{rows_per_shard:,}/shard)")

    # Type hints for the v5.5.1 schema. Native float for amounts (HF dataset
    # convention; downstream readers can convert to Decimal if desired).
    dtypes = {
        "fiscal_year": "int32",
        "fiscal_period": "int8",
        "exchange_rate": "float64",
        "is_fraud": "bool",
        "is_anomaly": "bool",
        "is_manual": "bool",
        "is_post_close": "bool",
        "line_number": "int32",
        "debit_amount": "float64",
        "credit_amount": "float64",
        "local_amount": "float64",
    }
    # Date columns parsed natively.
    parse_dates = ["posting_date", "document_date", "value_date", "lettrage_date"]

    chunk_iter = pd.read_csv(
        csv_path,
        chunksize=100_000,
        dtype=dtypes,
        parse_dates=parse_dates,
        low_memory=False,
    )

    rows_emitted = 0
    shard_idx = 0
    shard_buf: list[pd.DataFrame] = []
    shard_rows = 0

    def flush(shard_idx: int, dfs: list[pd.DataFrame]) -> None:
        if not dfs:
            return
        df = pd.concat(dfs, ignore_index=True)
        out_path = out_dir / f"train-{shard_idx:05d}-of-{target_shards:05d}.parquet"
        table = pa.Table.from_pandas(df, preserve_index=False)
        pq.write_table(table, out_path, compression="zstd", compression_level=9)
        size_mb = out_path.stat().st_size / 1024 / 1024
        print(f"  wrote {out_path.name}: {len(df):,} rows, {size_mb:.1f} MB")

    for chunk in chunk_iter:
        shard_buf.append(chunk)
        shard_rows += len(chunk)
        rows_emitted += len(chunk)
        if shard_rows >= rows_per_shard and shard_idx < target_shards - 1:
            flush(shard_idx, shard_buf)
            shard_buf = []
            shard_rows = 0
            shard_idx += 1

    flush(shard_idx, shard_buf)
    return rows_emitted


def json_to_parquet(json_path: Path, out_path: Path, label: str) -> int:
    """Convert a JSON array (or dict-with-list) to a single parquet file."""
    with open(json_path) as f:
        data = json.load(f)
    # Some artefacts wrap the list in {"accounts": [...]} or similar.
    if isinstance(data, dict):
        for key in ("accounts", "balances", "lines", "items", "trial_balance", "trial_balances"):
            if key in data and isinstance(data[key], list):
                data = data[key]
                break
    if not isinstance(data, list):
        raise ValueError(f"{json_path}: expected list, got {type(data).__name__}")
    if not data:
        print(f"  {label}: empty, skipping")
        return 0

    df = pd.json_normalize(data, max_level=1)
    table = pa.Table.from_pandas(df, preserve_index=False)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    pq.write_table(table, out_path, compression="zstd", compression_level=9)
    size_mb = out_path.stat().st_size / 1024 / 1024
    print(f"  wrote {out_path.name}: {len(df):,} rows, {size_mb:.1f} MB ({label})")
    return len(df)


def trial_balances_to_parquet(json_path: Path, out_path: Path) -> int:
    """Flatten TrialBalance[].lines into a wide per-line parquet.

    Each row carries the parent TB header columns (company / period /
    balanced flag / equation flag) so users can join by `(company_code,
    fiscal_period)` without a separate header table.
    """
    with open(json_path) as f:
        data = json.load(f)
    if isinstance(data, dict):
        for key in ("trial_balances", "balances"):
            if key in data and isinstance(data[key], list):
                data = data[key]
                break
    if not isinstance(data, list) or not data:
        print(f"  Trial Balances: empty, skipping")
        return 0

    rows: list[dict] = []
    for tb in data:
        header = {
            "trial_balance_id":  tb.get("trial_balance_id"),
            "company_code":      tb.get("company_code"),
            "company_name":      tb.get("company_name"),
            "as_of_date":        tb.get("as_of_date"),
            "fiscal_year":       tb.get("fiscal_year"),
            "fiscal_period":     tb.get("fiscal_period"),
            "currency":          tb.get("currency"),
            "balance_type":      tb.get("balance_type"),
            "tb_total_debits":   tb.get("total_debits"),
            "tb_total_credits":  tb.get("total_credits"),
            "tb_is_balanced":    tb.get("is_balanced"),
            "tb_out_of_balance": tb.get("out_of_balance"),
            "tb_is_equation_valid":  tb.get("is_equation_valid"),
            "tb_equation_difference": tb.get("equation_difference"),
        }
        for line in tb.get("lines", []):
            row = {**header}
            row.update(line)
            rows.append(row)

    if not rows:
        print(f"  Trial Balances: 0 lines, skipping")
        return 0

    df = pd.json_normalize(rows, max_level=1)
    # Decimal-as-string fields → float64 for analytics convenience
    for col in (
        "tb_total_debits", "tb_total_credits", "tb_out_of_balance",
        "tb_equation_difference",
        "opening_balance", "closing_balance", "period_debits", "period_credits",
        "ytd_debits", "ytd_credits",
    ):
        if col in df.columns:
            df[col] = pd.to_numeric(df[col], errors="coerce")

    table = pa.Table.from_pandas(df, preserve_index=False)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    pq.write_table(table, out_path, compression="zstd", compression_level=9)
    size_mb = out_path.stat().st_size / 1024 / 1024
    print(f"  wrote {out_path.name}: {len(df):,} rows, {size_mb:.1f} MB (Trial Balance lines)")
    return len(df)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--output-dir", required=True, help="generation output dir (./output)")
    ap.add_argument("--hf-dir", required=True, help="HF staging dir to write parquet under")
    ap.add_argument("--shards", type=int, default=3)
    args = ap.parse_args()

    out = Path(args.output_dir)
    hf = Path(args.hf_dir)
    hf.mkdir(parents=True, exist_ok=True)

    # 1. JE — sharded parquet under data/.
    je_csv = out / "journal_entries.csv"
    if je_csv.exists():
        print("Converting journal_entries.csv -> sharded parquet …")
        je_rows = shard_je_csv(je_csv, hf / "data", target_shards=args.shards)
        print(f"  total JE rows: {je_rows:,}")
    else:
        print(f"ERROR: {je_csv} not found", file=sys.stderr)
        return 1

    # 2. Chart of accounts.
    coa_json = out / "chart_of_accounts.json"
    if coa_json.exists():
        print("Converting chart_of_accounts.json -> parquet …")
        json_to_parquet(coa_json, hf / "chart_of_accounts.parquet", "COA")

    # 3. Trial balances — flatten to one row per (TB, line) for analytics.
    tb_json = out / "period_close" / "trial_balances.json"
    if tb_json.exists():
        print("Converting period_close/trial_balances.json -> parquet …")
        trial_balances_to_parquet(tb_json, hf / "trial_balances.parquet")

    # 4. Cost / profit centres (optional, very useful for joins).
    for src, dst, label in [
        (out / "master_data" / "cost_centers.json", hf / "cost_centers.parquet", "Cost Centres"),
        (out / "master_data" / "profit_centers.json", hf / "profit_centers.parquet", "Profit Centres"),
    ]:
        if src.exists():
            print(f"Converting {src.name} -> parquet …")
            json_to_parquet(src, dst, label)

    print(f"\nAll artefacts written to {hf}/")
    return 0


if __name__ == "__main__":
    sys.exit(main())

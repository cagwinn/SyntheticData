#!/usr/bin/env python3
"""Run Sajja's exact P1-P4 behavioral_fidelity evaluator on (reference, synth).

The literal Python code from the Sajja 2026 paper
[`bhavana3/synthetic-data-experiments/evaluation/behavioral_fidelity.py`](
https://github.com/bhavana3/synthetic-data-experiments), instantiated as-is
with column-mapped GL data. See `docs/baselines/2026-05-26-v5.30-a1-sajja-p3/`
for the methodology and result writeup.

This is the v5.30 A1 driver — P3 graph motifs wired via
`attr_cols=['trading_partner', 'gl_account']`.

The corpus and synth parquets are expected in the GL "corpus schema"
(columns: `Source`, `Effective Date`, `Functional Amount`,
`Tarding Partner` [sic], `GL Account Number`).

Usage:
    python run_sajja_eval.py \\
        --sajja-repo  /path/to/synthetic-data-experiments \\
        --reference   /path/to/reference_shard.parquet \\
        --synth       /path/to/journal_entries_corpus_schema.parquet \\
        --out         /path/to/output_dir \\
        [--subsample 500000]

Both inputs are sub-sampled (default 500K rows each) for runtime —
Sajja's eval has O(n²) inner loops in some sub-routines.
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from dataclasses import asdict, is_dataclass
from pathlib import Path

import pandas as pd


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--sajja-repo', required=True, type=Path,
                   help='Local clone of synthetic-data-experiments')
    p.add_argument('--reference', required=True, type=Path,
                   help='Reference parquet (corpus schema)')
    p.add_argument('--synth', required=True, type=Path,
                   help='Synth parquet (corpus schema)')
    p.add_argument('--out', required=True, type=Path,
                   help='Output dir for baseline.json + datasynth.json')
    p.add_argument('--subsample', type=int, default=500_000,
                   help='Subsample size for both sides (default: 500_000)')
    p.add_argument('--generator-name', default='DataSynth_v5.29_SOTA',
                   help='Tag for the synth side in the report')
    return p.parse_args()


def load_and_canonicalize(p: Path, kind: str, quantile_thr: float = 0.96) -> pd.DataFrame:
    """Map corpus-schema parquet → Sajja's expected columns + attr_cols."""
    df = pd.read_parquet(p)
    print(f'[{kind}] raw: {len(df):,} rows, cols={list(df.columns)[:8]}…')
    out = pd.DataFrame({
        'entity':           df['Source'].fillna('NONE').astype(str),
        'timestamp':        pd.to_datetime(df['Effective Date']).astype('int64') // 10**9,
        'amount':           pd.to_numeric(df['Functional Amount'], errors='coerce').fillna(0).abs(),
        'trading_partner':  df['Tarding Partner'].fillna('NONE').astype(str),
        'gl_account':       df['GL Account Number'].fillna('NONE').astype(str),
        'label':            0,
    })
    # Label heuristic — top quantile of |amount| → fraud
    thr = out['amount'].quantile(quantile_thr)
    out.loc[out['amount'] >= thr, 'label'] = 1
    out = out.sort_values('timestamp').reset_index(drop=True)
    print(f'[{kind}] mapped: {len(out):,} rows, {out["label"].sum():,} labeled positive')
    print(f'[{kind}] trading_partner: {out["trading_partner"].nunique()} unique')
    print(f'[{kind}] gl_account: {out["gl_account"].nunique()} unique')
    return out


def main() -> None:
    args = parse_args()
    sys.path.insert(0, str(args.sajja_repo))
    from evaluation.behavioral_fidelity import BehavioralFidelityEvaluator

    args.out.mkdir(parents=True, exist_ok=True)

    print('loading reference...')
    ref = load_and_canonicalize(args.reference, 'reference')
    print('loading synth...')
    syn = load_and_canonicalize(args.synth, 'synth')

    if len(ref) > args.subsample:
        ref = ref.sample(args.subsample, random_state=42).sort_values('timestamp').reset_index(drop=True)
        print(f'subsampled ref to {len(ref):,}')
    if len(syn) > args.subsample:
        syn = syn.sample(args.subsample, random_state=42).sort_values('timestamp').reset_index(drop=True)
        print(f'subsampled syn to {len(syn):,}')

    ev = BehavioralFidelityEvaluator(
        entity_col='entity',
        time_col='timestamp',
        label_col='label',
        amount_col='amount',
        attr_cols=['trading_partner', 'gl_account'],   # v5.30 A1
    )

    print('\n=== STEP 1: baseline (ref ↔ ref half-split) ===')
    half = len(ref) // 2
    real_A = ref.iloc[:half].reset_index(drop=True)
    real_B = ref.iloc[half:].reset_index(drop=True)
    t0 = time.time()
    baseline = ev.evaluate_all(real_A, real_B,
                               generator_name='BASELINE',
                               dataset_name='reference_shard')
    print(f'  elapsed: {time.time()-t0:.1f}s')
    print(baseline.summary())

    print(f'\n=== STEP 2: {args.generator_name} vs reference ===')
    t1 = time.time()
    report = ev.evaluate_all(ref, syn,
                             generator_name=args.generator_name,
                             dataset_name='reference_shard',
                             baseline=baseline)
    print(f'  elapsed: {time.time()-t1:.1f}s')
    print(report.summary())

    def to_dict(rep):
        return asdict(rep) if is_dataclass(rep) else rep

    args.out.joinpath('baseline.json').write_text(json.dumps(to_dict(baseline), default=str, indent=2))
    args.out.joinpath('datasynth_synth.json').write_text(json.dumps(to_dict(report), default=str, indent=2))
    print(f'\nwrote: {args.out}/baseline.json + datasynth_synth.json')


if __name__ == '__main__':
    main()

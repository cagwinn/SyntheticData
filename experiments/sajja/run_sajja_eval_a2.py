#!/usr/bin/env python3
"""v5.30 A2 (#151) — Sajja exact eval using synth's real `is_fraud` labels.

The A1 driver (`run_sajja_eval.py`) uses a top-quantile-of-|amount|
heuristic on both sides because the reference shard has no
ground-truth fraud labels. This A2 driver does the same on the
reference but uses the **synth's real `is_fraud` column** for the
synth side — that's the asymmetric-label setup Sajja's evaluator
supports and the v5.30 roadmap A2 target.

The synth's corpus-schema parquet was projected without `is_fraud`;
this driver joins it back in by reading just that column from the
full-schema parquet (same rows, same order — `document_id` and JE
Number align by position).

Usage:
    python run_sajja_eval_a2.py \\
        --sajja-repo  /path/to/synthetic-data-experiments \\
        --reference   /path/to/reference_shard.parquet \\
        --synth-cs    /path/to/journal_entries_corpus_schema.parquet \\
        --synth-full  /path/to/journal_entries.parquet \\
        --out         /path/to/output_dir
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
    p.add_argument('--sajja-repo', required=True, type=Path)
    p.add_argument('--reference',  required=True, type=Path)
    p.add_argument('--synth-cs',   required=True, type=Path,
                   help='Synth parquet in corpus-schema columns')
    p.add_argument('--synth-full', required=True, type=Path,
                   help='Synth parquet in full schema (must include is_fraud)')
    p.add_argument('--out',        required=True, type=Path)
    p.add_argument('--subsample',  type=int, default=500_000)
    return p.parse_args()


def load_reference(path: Path) -> pd.DataFrame:
    df = pd.read_parquet(path)
    out = pd.DataFrame({
        'entity':           df['Source'].fillna('NONE').astype(str),
        'timestamp':        pd.to_datetime(df['Effective Date']).astype('int64') // 10**9,
        'amount':           pd.to_numeric(df['Functional Amount'], errors='coerce').fillna(0).abs(),
        'trading_partner':  df['Tarding Partner'].fillna('NONE').astype(str),
        'gl_account':       df['GL Account Number'].fillna('NONE').astype(str),
    })
    thr = out['amount'].quantile(0.96)
    out['label'] = (out['amount'] >= thr).astype(int)
    out = out.sort_values('timestamp').reset_index(drop=True)
    print(f'[reference] {len(out):,} rows, {out["label"].sum():,} labeled (heuristic)')
    return out


def load_synth(cs_path: Path, full_path: Path) -> pd.DataFrame:
    cs = pd.read_parquet(cs_path)
    full_label = pd.read_parquet(full_path, columns=['is_fraud'])
    assert len(cs) == len(full_label), f'row count mismatch: {len(cs)} vs {len(full_label)}'
    out = pd.DataFrame({
        'entity':           cs['Source'].fillna('NONE').astype(str),
        'timestamp':        pd.to_datetime(cs['Effective Date']).astype('int64') // 10**9,
        'amount':           pd.to_numeric(cs['Functional Amount'], errors='coerce').fillna(0).abs(),
        'trading_partner':  cs['Tarding Partner'].fillna('NONE').astype(str),
        'gl_account':       cs['GL Account Number'].fillna('NONE').astype(str),
        'label':            full_label['is_fraud'].astype(int).values,
    })
    out = out.sort_values('timestamp').reset_index(drop=True)
    print(f'[synth] {len(out):,} rows, {out["label"].sum():,} labeled (real is_fraud, '
          f'{out["label"].mean()*100:.2f}%)')
    return out


def main() -> None:
    args = parse_args()
    sys.path.insert(0, str(args.sajja_repo))
    from evaluation.behavioral_fidelity import BehavioralFidelityEvaluator

    args.out.mkdir(parents=True, exist_ok=True)

    print('loading reference...')
    ref = load_reference(args.reference)
    print('loading synth (with is_fraud labels)...')
    syn = load_synth(args.synth_cs, args.synth_full)

    if len(ref) > args.subsample:
        ref = ref.sample(args.subsample, random_state=42).sort_values('timestamp').reset_index(drop=True)
        print(f'subsampled ref to {len(ref):,}, {ref["label"].sum():,} labeled')
    if len(syn) > args.subsample:
        syn = syn.sample(args.subsample, random_state=42).sort_values('timestamp').reset_index(drop=True)
        print(f'subsampled syn to {len(syn):,}, {syn["label"].sum():,} labeled')

    ev = BehavioralFidelityEvaluator(
        entity_col='entity', time_col='timestamp',
        label_col='label', amount_col='amount',
        attr_cols=['trading_partner', 'gl_account'],
    )

    print('\n=== STEP 1: baseline (ref half-split, heuristic) ===')
    half = len(ref) // 2
    real_A = ref.iloc[:half].reset_index(drop=True)
    real_B = ref.iloc[half:].reset_index(drop=True)
    t0 = time.time()
    baseline = ev.evaluate_all(real_A, real_B, generator_name='BASELINE',
                               dataset_name='reference_shard')
    print(f'  elapsed: {time.time()-t0:.1f}s')
    print(baseline.summary())

    print('\n=== STEP 2: v5.29 synth (real is_fraud) vs reference (heuristic) ===')
    t1 = time.time()
    report = ev.evaluate_all(ref, syn,
                             generator_name='DataSynth_v5.29_A2_real_labels',
                             dataset_name='reference_shard',
                             baseline=baseline)
    print(f'  elapsed: {time.time()-t1:.1f}s')
    print(report.summary())

    def to_dict(rep): return asdict(rep) if is_dataclass(rep) else rep
    args.out.joinpath('baseline.json').write_text(json.dumps(to_dict(baseline), default=str, indent=2))
    args.out.joinpath('datasynth_a2.json').write_text(json.dumps(to_dict(report), default=str, indent=2))
    print(f'\nwrote: {args.out}/baseline.json + datasynth_a2.json')


if __name__ == '__main__':
    main()

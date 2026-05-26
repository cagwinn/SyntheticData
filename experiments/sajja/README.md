# Sajja exact eval drivers

Runs Sajja 2026's `BehavioralFidelityEvaluator` from
[`bhavana3/synthetic-data-experiments`](
https://github.com/bhavana3/synthetic-data-experiments) against
DataSynth output. Two drivers:

| Driver | Closes | Synth label source |
|---|---|---|
| `run_sajja_eval.py`    | A1 (#149) | top-4% amount heuristic |
| `run_sajja_eval_a2.py` | A2 (#151) | real `is_fraud` column   |

Both drivers wire P3 graph motifs via
`attr_cols=['trading_partner', 'gl_account']` (the v5.30 A1 lever
from the roadmap).

## Setup

```bash
git clone https://github.com/bhavana3/synthetic-data-experiments ~/synthetic-data-experiments
pip install pot networkx pandas pyarrow scipy numpy
```

The Sajja repo lists `POT` in `requirements.txt`; pip installs it as
the `ot` module. Most usage doesn't trip this, but if it does:

```python
# Create a shim if POT.py import fails
echo "from ot import *" > ~/.local/lib/python3.12/site-packages/POT.py
```

The Sajja `_compute_iet` expects numeric timestamps. Convert your
`Effective Date` column with:

```python
pd.to_datetime(df['Effective Date']).astype('int64') // 10**9
```

The drivers handle this already.

## A1: heuristic labels

```bash
python run_sajja_eval.py \
    --sajja-repo  ~/synthetic-data-experiments \
    --reference   /path/to/reference_shard.parquet \
    --synth       /path/to/journal_entries_corpus_schema.parquet \
    --out         ./sajja_eval_a1
```

Reads `Source` / `Effective Date` / `Functional Amount` /
`Tarding Partner` / `GL Account Number` from the corpus-schema
parquet. Tags the top 4% of |amount| as fraud on both sides.

## A2: real synth labels

```bash
python run_sajja_eval_a2.py \
    --sajja-repo  ~/synthetic-data-experiments \
    --reference   /path/to/reference_shard.parquet \
    --synth-cs    /path/to/journal_entries_corpus_schema.parquet \
    --synth-full  /path/to/journal_entries.parquet \
    --out         ./sajja_eval_a2
```

Needs both the corpus-schema synth parquet (for the canonical column
mapping) **and** the full-schema synth parquet (for the `is_fraud`
column). The drivers join by position — the `corpus_schema.parquet`
projection preserves row order from the full parquet, so an assert
on row count is sufficient.

## Result interpretation

See [`docs/baselines/2026-05-26-sajja-exact-eval/COMPARISON.md`](
../../docs/baselines/2026-05-26-sajja-exact-eval/COMPARISON.md) for
the A0 baseline (no P3, heuristic labels) and
[`docs/baselines/2026-05-26-v5.30-a1-sajja-p3/COMPARISON.md`](
../../docs/baselines/2026-05-26-v5.30-a1-sajja-p3/COMPARISON.md) for
the A1 result with P3 wired.

## Runtime

Both drivers take ~27 min on a Lambda A10 (30c/222GB) at 500K-row
subsample. P3 step adds ~3-5 min vs A0 (driven by the bipartite
graph projection size). Larger subsamples grow quadratically in
some sub-routines — bump `--subsample` cautiously.

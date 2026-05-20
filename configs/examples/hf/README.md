# HuggingFace VynFi reference dataset configs

Configs that produce the published reference datasets at
[huggingface.co/VynFi](https://huggingface.co/VynFi).

| Config | Dataset | What it generates |
|--------|---------|-------------------|
| [`journal_entries_1m.yaml`](journal_entries_1m.yaml) | [`VynFi/vynfi-journal-entries-1m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m) | 10 manufacturing companies × 12 monthly periods → ~1 M JE lines (46 cols), full chart of accounts, trial balance, cost / profit centres. **Carries `is_fraud` + fine-grained `fraud_type` typology + `anomaly_type` labels** (v5.27.0). |

## One-shot regeneration + upload

```bash
# 1. Generate (≈90 s on commodity hardware)
./target/release/datasynth-data generate \
    --config configs/examples/hf/journal_entries_1m.yaml \
    --output ./hf_je_output

# 2. Convert to HF-ready parquet (sharded JE + COA + TB + master data)
./scripts/hf_to_parquet.py \
    --output-dir ./hf_je_output \
    --hf-dir     ./hf_staging \
    --shards     3

# 3. Upload to the dataset repo (requires `hf auth login` to a VynFi member)
hf upload --repo-type dataset \
    VynFi/vynfi-journal-entries-1m \
    ./hf_staging \
    --commit-message "v5.5.1 refresh"
```

Steps 1–2 are bit-for-bit reproducible: the YAML pins
`global.seed: 20260507`, and ChaCha8 PRNG output is platform-stable.
A clean rerun produces identical parquet bytes.

## What the config produces

`hf_je_output/` will contain (relevant subset):

```
journal_entries.csv               # 46-column flat table (sharded into parquet),
                                   # incl. is_fraud + fraud_type + anomaly_type
chart_of_accounts.json            # Full COA snapshot
period_close/trial_balances.json  # 12 monthly TBs for the primary entity
master_data/cost_centers.json     # Cost-centre master
master_data/profit_centers.json   # Profit-centre master
balance/, banking/, hr/, …        # Other domain artefacts (not uploaded to HF)
```

`hf_staging/` will contain only the files we actually publish:

```
data/train-00000-of-00003.parquet   # JE shard 1
data/train-00001-of-00003.parquet   # JE shard 2
data/train-00002-of-00003.parquet   # JE shard 3
chart_of_accounts.parquet
trial_balances.parquet
cost_centers.parquet
profit_centers.parquet
je_network.parquet                  # Accounting-network edges (v5.8.0+)
README.md                           # Dataset card (copy from previous publish)
generation_config.yaml              # Reproducibility receipt (this file)
```

`je_network.parquet` is the flat Cartesian-product edge list produced
by v5.8.0 — one row per `(debit_line, credit_line)` pair within each
journal entry, joinable back to the `data/train-*.parquet` JE shards
via `from_line_id` / `to_line_id` (which match the `transaction_id`
column on the JE side).  Schema: `edge_id`, `document_id`,
`posting_date`, `from_account`, `to_account`, `from_line_id`,
`to_line_id`, `amount`, `confidence`, `predecessor_edge_id`,
`business_process`, `is_fraud`, `is_anomaly`.  See the v5.8.0
CHANGELOG entry for the design rationale and the Methods A–E
reference (Ivertowski 2024).

## Adapting to other HF dataset repos

Each future HF dataset gets its own YAML in this directory.  The
`scripts/hf_to_parquet.py` script is dataset-agnostic — it converts
any DataSynth output into the same parquet layout — so most additions
are a config-only change.

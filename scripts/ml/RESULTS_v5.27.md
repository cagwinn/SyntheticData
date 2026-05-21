# JE fraud / anomaly / typology — v5.27 results (A100)

Retrain of the GNN fraud showcase on the **v5.27** public dataset
([`VynFi/vynfi-journal-entries-1m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m)),
which adds the community-requested **`fraud_type`** typology column. Trained on
public synthetic data only (no corpus, no PII). NVIDIA A100-40GB, seed 20260521.

Dataset: 61,656 Method-A `je_network` edges over 499 accounts; 1,058,941 JE
lines; 5.79 % fraud (3,571 fraud edges / 61,603 fraud lines) across **20 fraud
typologies**.

## Headline

| Task | Model | View | Key metric | Baseline |
|------|-------|------|-----------|----------|
| **Fraud detection (binary)** | GraphSAGE edge classifier | `je_network` | test **AUC-ROC 0.909**, PR 0.799, F1 0.790 | LogReg (edge feats) AUC 0.911 |
| **Node anomaly** | Attribute-reconstruction GAE | `je_network` | per-edge AUC 0.672; per-node AUC 0.511 | — |
| **Fraud typology (20-way)** | GraphSAGE softmax head | `je_network` | macro-F1 **0.091**, top-3 0.27 | LogReg macro-F1 0.082 |
| **Fraud typology (20-way)** | HistGradientBoosting | `journal_entries` (line) | macro-F1 **0.582**, acc 0.702, top-3 **0.812** | — |

## The two findings

1. **Binary fraud detection is easy from any view, and the graph barely helps.**
   The GraphSAGE edge classifier (AUC 0.909) does not beat a plain logistic
   regression on the edge features alone (0.911) — because the fraud-bias
   signals (round-dollar, weekend, off-hours) are linearly separable and live
   *in the edge attributes*, not in the graph topology. Honest result: this
   dataset's binary fraud is a feature problem, not a structure problem. The
   GAE node-anomaly scorer is near-random at node level (AUC 0.51); the
   anomaly signal is per-edge (AUC 0.67), not per-account.

2. **Fraud *typology* needs the line-level view — the edge list throws the
   signal away.** Classifying *which* of the 20 typologies is near-random on
   the collapsed `je_network` edge list (macro-F1 0.091 ≈ chance), because a
   2-line debit↔credit edge keeps only amount/date/process/account-pair. The
   richer `journal_entries` line view — `gl_account` (numeric, semantic
   ranges), `account_class`, `cost_center`, posting lag, round-dollar shape,
   `is_post_close`/`is_manual` — lifts macro-F1 **6.4×** to **0.582** and top-3
   to **0.812**. So the `fraud_type` column the community asked for *is*
   learnable, but consumers should join to the line table, not rely on the
   edge list alone.

### Per-typology (line-level HistGBM, test)

Strong (F1 ≥ 0.65): `JustBelowThreshold` 0.89, `ExceededApprovalLimit` 0.81,
`DuplicatePayment` 0.79, `TimingAnomaly` 0.77, `SplitTransaction` 0.75,
`SuspenseAccountAbuse` 0.74, `FictitiousEntry` 0.74, `FictitiousVendor` 0.71,
`UnauthorizedAccess` 0.71, `RoundDollarManipulation` 0.67, `FictitiousTransaction`
0.67, `RevenueManipulation` 0.65. Weak / low-support: `Level3InputManipulation`
(n=21), `PhantomVendorContract` (n=31), `BidRigging` (n=43) — the rare sourcing
typologies need more examples (regenerate at higher volume to close these).

## Reproduce

```bash
# build PyG dataset (carries fraud_type_idx, schema v2)
python -m scripts.ml.build_je_pyg_dataset --output data/ml/je_pyg_v527.pt --seed 20260521
# binary fraud GNN + LogReg baseline
python -m scripts.ml.train_je_fraud_gnn --dataset data/ml/je_pyg_v527.pt --epochs 80
# node-anomaly GAE
python -m scripts.ml.train_je_anomaly_gae --dataset data/ml/je_pyg_v527.pt --epochs 120
# typology — edge-list GNN (near-random) vs line-level HistGBM (strong)
python -m scripts.ml.train_je_fraud_typology --dataset data/ml/je_pyg_v527.pt --epochs 200
python -m scripts.ml.train_je_line_typology --out models/ml/je_line_typology
```

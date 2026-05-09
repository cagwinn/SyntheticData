"""VynFi Fraud-GNN Demo — Gradio Space.

Three tabs:

* **Edge fraud predictor**  — dataset-sampled examples + manual entry.
* **Node anomaly explorer** — top-K accounts by GAE reconstruction MSE.
* **Live check**            — random val sample with confusion matrix + ROC.
"""
from __future__ import annotations

from functools import lru_cache
from typing import Any

import gradio as gr
import matplotlib.pyplot as plt
import numpy as np
import pandas as pd
import torch
from huggingface_hub import hf_hub_download, snapshot_download
from sklearn.metrics import (
    average_precision_score,
    confusion_matrix,
    roc_auc_score,
    roc_curve,
)

from models import BUSINESS_PROCESSES, InferenceBundle, load_bundle


MODEL_REPO = "VynFi/je-fraud-gnn"
DATA_REPO = "VynFi/vynfi-journal-entries-1m"


# ─── Lazy loaders (executed once at app startup; cached thereafter) ─────────


@lru_cache(maxsize=1)
def get_bundle() -> InferenceBundle:
    local = snapshot_download(repo_id=MODEL_REPO)
    return load_bundle(local)


@lru_cache(maxsize=1)
def get_account_catalog() -> pd.DataFrame:
    fp = hf_hub_download(repo_id=DATA_REPO, filename="chart_of_accounts.parquet", repo_type="dataset")
    df = pd.read_parquet(fp)[
        ["account_number", "short_description", "account_type", "account_class", "account_class_name"]
    ]
    df["account_number"] = df["account_number"].astype(str)
    df = df.drop_duplicates(subset=["account_number"], keep="first")
    df["label"] = df["account_number"] + " — " + df["short_description"]
    return df


@lru_cache(maxsize=1)
def get_edge_sample() -> pd.DataFrame:
    fp = hf_hub_download(repo_id=DATA_REPO, filename="je_network.parquet", repo_type="dataset")
    df = pd.read_parquet(fp)
    df["from_account"] = df["from_account"].astype(str)
    df["to_account"] = df["to_account"].astype(str)
    return df


def account_choices() -> list[str]:
    bundle = get_bundle()
    cat = get_account_catalog()
    cat = cat[cat["account_number"].isin(bundle.node_index)].sort_values("account_number")
    return cat["label"].tolist()


def label_to_account(label: str) -> str:
    return label.split(" — ", 1)[0]


# ─── Tab 1: Edge fraud predictor ─────────────────────────────────────────────


CURATED_SAMPLES = [
    {
        "label": "Clear-fraud P2P (round-dollar + weekend)",
        "from": "1000 — Operating Cash",
        "to": "2000 — Trade Payables",
        "amount": 25_000.0,
        "process": "P2P",
        "date": "2024-08-10",
    },
    {
        "label": "Clear-fraud O2C (round + Sunday)",
        "from": "1100 — Accounts Receivable",
        "to": "4000 — Sales Revenue",
        "amount": 50_000.0,
        "process": "O2C",
        "date": "2024-09-08",
    },
    {
        "label": "Clear-normal P2P (off-round amount, weekday)",
        "from": "1000 — Operating Cash",
        "to": "2000 — Trade Payables",
        "amount": 7_432.89,
        "process": "P2P",
        "date": "2024-03-12",
    },
    {
        "label": "Clear-normal O2C (mid-month, weekday)",
        "from": "1100 — Accounts Receivable",
        "to": "4000 — Sales Revenue",
        "amount": 12_876.43,
        "process": "O2C",
        "date": "2024-04-17",
    },
    {
        "label": "Borderline (round amount, weekday)",
        "from": "1000 — Operating Cash",
        "to": "2000 — Trade Payables",
        "amount": 10_000.0,
        "process": "P2P",
        "date": "2024-05-15",
    },
]


def fmt_money(x: float) -> str:
    sign = "-" if x < 0 else ""
    x = abs(float(x))
    if x >= 1e9:
        return f"{sign}${x / 1e9:.2f}B"
    if x >= 1e6:
        return f"{sign}${x / 1e6:.2f}M"
    if x >= 1e3:
        return f"{sign}${x / 1e3:.2f}K"
    return f"{sign}${x:.2f}"


def predict_one(
    from_label: str,
    to_label: str,
    amount: float,
    process: str,
    date: str,
) -> tuple[str, dict]:
    bundle = get_bundle()
    src = label_to_account(from_label)
    dst = label_to_account(to_label)
    fraud_p = float(
        bundle.predict_fraud(
            from_account=[src],
            to_account=[dst],
            amount=[float(amount)],
            business_process=[process],
            posting_date=[str(date)],
        )[0]
    )
    anomaly_mse = float(
        bundle.anomaly_score_edges(
            from_account=[src],
            to_account=[dst],
            amount=[float(amount)],
            business_process=[process],
            posting_date=[str(date)],
        )[0]
    )
    threshold = bundle.fraud_threshold
    verdict = "🚨 FRAUD" if fraud_p >= threshold else "✓ normal"
    summary_md = (
        f"### {verdict}\n\n"
        f"**Fraud probability:** `{fraud_p:.4f}`  (threshold = `{threshold:.3f}`)  \n"
        f"**Anomaly MSE:** `{anomaly_mse:.4f}`  (higher = more unusual)\n\n"
        f"**Edge:** `{src}` → `{dst}`  \n"
        f"**Amount:** {fmt_money(amount)}  ·  **Process:** {process}  ·  **Date:** {date}\n"
    )
    feature_inspect = {
        "is_round_dollar": any(abs(float(amount) - lv) < 1.0 for lv in [1000, 5000, 10000, 25000, 50000, 100000]),
        "is_weekend": pd.to_datetime(date).dayofweek >= 5,
        "amount": float(amount),
        "process": process,
    }
    return summary_md, feature_inspect


def load_sample(sample_label: str) -> tuple[str, str, float, str, str]:
    s = next(s for s in CURATED_SAMPLES if s["label"] == sample_label)
    return s["from"], s["to"], s["amount"], s["process"], s["date"]


# ─── Tab 2: Node anomaly explorer ────────────────────────────────────────────


def build_node_anomaly_table(top_k: int = 50) -> pd.DataFrame:
    bundle = get_bundle()
    cat = get_account_catalog()
    edges_df = get_edge_sample()

    test_sample = edges_df.sample(min(5000, len(edges_df)), random_state=42)
    test_sample = test_sample[
        test_sample["from_account"].isin(bundle.node_index)
        & test_sample["to_account"].isin(bundle.node_index)
    ]
    per_edge_mse = bundle.anomaly_score_edges(
        from_account=test_sample["from_account"].tolist(),
        to_account=test_sample["to_account"].tolist(),
        amount=test_sample["amount"].tolist(),
        business_process=test_sample["business_process"].tolist(),
        posting_date=test_sample["posting_date"].astype(str).tolist(),
    )

    df = test_sample.copy()
    df["mse"] = per_edge_mse
    src_agg = df.groupby("from_account").agg(out_mse=("mse", "mean"), out_count=("mse", "count"))
    dst_agg = df.groupby("to_account").agg(in_mse=("mse", "mean"), in_count=("mse", "count"))
    by_node = src_agg.join(dst_agg, how="outer").fillna(0)
    by_node["mean_mse"] = (
        (by_node["out_mse"] * by_node["out_count"] + by_node["in_mse"] * by_node["in_count"])
        / (by_node["out_count"] + by_node["in_count"]).replace(0, 1)
    )
    by_node["incident_edges"] = by_node["out_count"] + by_node["in_count"]
    by_node = by_node.reset_index().rename(columns={"index": "account_number"})

    enriched = by_node.merge(cat, on="account_number", how="left")
    enriched = enriched.sort_values("mean_mse", ascending=False).head(int(top_k))
    enriched["mean_mse"] = enriched["mean_mse"].round(4)
    return enriched[
        [
            "account_number",
            "short_description",
            "account_type",
            "account_class",
            "mean_mse",
            "incident_edges",
        ]
    ].rename(
        columns={
            "account_number": "GL #",
            "short_description": "Account",
            "account_type": "Type",
            "account_class": "Class",
            "mean_mse": "Anomaly MSE",
            "incident_edges": "Sample edges",
        }
    )


# ─── Tab 3: Live check ───────────────────────────────────────────────────────


def run_live_check(n_samples: int = 200) -> tuple[Any, Any, str]:
    bundle = get_bundle()
    edges_df = get_edge_sample()
    edges_df = edges_df[
        edges_df["from_account"].isin(bundle.node_index)
        & edges_df["to_account"].isin(bundle.node_index)
    ]
    sample = edges_df.sample(int(n_samples), random_state=None)

    probs = bundle.predict_fraud(
        from_account=sample["from_account"].tolist(),
        to_account=sample["to_account"].tolist(),
        amount=sample["amount"].tolist(),
        business_process=sample["business_process"].tolist(),
        posting_date=sample["posting_date"].astype(str).tolist(),
    )
    y_true = sample["is_fraud"].astype(int).to_numpy()
    threshold = bundle.fraud_threshold
    y_pred = (probs >= threshold).astype(int)
    if y_true.sum() == 0 or y_true.sum() == len(y_true):
        return None, None, "Sampled batch had only one class — try a larger sample."

    auc = roc_auc_score(y_true, probs)
    ap = average_precision_score(y_true, probs)
    cm = confusion_matrix(y_true, y_pred)

    fig_cm = plt.figure(figsize=(4, 4), dpi=120)
    ax = fig_cm.add_subplot(111)
    ax.imshow(cm, cmap="Blues")
    ax.set_xticks([0, 1])
    ax.set_yticks([0, 1])
    ax.set_xticklabels(["normal", "fraud"])
    ax.set_yticklabels(["normal", "fraud"])
    for i in range(2):
        for j in range(2):
            ax.text(j, i, str(cm[i, j]), ha="center", va="center", fontsize=14, color="black")
    ax.set_xlabel("predicted")
    ax.set_ylabel("actual")
    ax.set_title(f"Confusion matrix (n={int(n_samples)})")
    fig_cm.tight_layout()

    fpr, tpr, _ = roc_curve(y_true, probs)
    fig_roc = plt.figure(figsize=(4, 4), dpi=120)
    ax2 = fig_roc.add_subplot(111)
    ax2.plot(fpr, tpr, label=f"ROC AUC = {auc:.3f}")
    ax2.plot([0, 1], [0, 1], "k--", alpha=0.4)
    ax2.set_xlabel("false positive rate")
    ax2.set_ylabel("true positive rate")
    ax2.set_title("ROC")
    ax2.legend()
    fig_roc.tight_layout()

    summary = (
        f"### Live check on {int(n_samples)} sampled edges\n\n"
        f"- AUC-ROC: **{auc:.4f}**\n"
        f"- AUC-PR: **{ap:.4f}**\n"
        f"- True fraud: {int(y_true.sum())} / {len(y_true)}\n"
        f"- Predicted fraud: {int(y_pred.sum())} / {len(y_pred)}\n"
        f"- Threshold: {threshold:.3f}\n"
    )
    return fig_cm, fig_roc, summary


# ─── Gradio UI ───────────────────────────────────────────────────────────────


def build_app() -> gr.Blocks:
    with gr.Blocks(title="VynFi Fraud-GNN Demo", theme=gr.themes.Soft()) as app:
        gr.Markdown(
            """
            # 🛡️ VynFi Fraud-GNN Demo

            Interactive inference on the
            [`VynFi/je-fraud-gnn`](https://huggingface.co/VynFi/je-fraud-gnn)
            model — GraphSAGE edge fraud classifier + attribute-reconstruction
            GAE node anomaly scorer, trained on the v5.9.0 Method-A network
            in
            [`VynFi/vynfi-journal-entries-1m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m).
            """
        )

        with gr.Tab("Edge fraud predictor"):
            with gr.Row():
                with gr.Column():
                    sample_picker = gr.Dropdown(
                        label="Curated samples",
                        choices=[s["label"] for s in CURATED_SAMPLES],
                        value=None,
                        info="Or fill in the form below for a custom edge.",
                    )
                    from_dd = gr.Dropdown(label="From account", choices=account_choices(), value=None)
                    to_dd = gr.Dropdown(label="To account", choices=account_choices(), value=None)
                    amount_in = gr.Number(label="Amount (USD)", value=10_000.0)
                    process_dd = gr.Dropdown(
                        label="Business process",
                        choices=BUSINESS_PROCESSES,
                        value="P2P",
                    )
                    date_in = gr.Textbox(label="Posting date (YYYY-MM-DD)", value="2024-06-15")
                    predict_btn = gr.Button("Predict", variant="primary")

                with gr.Column():
                    summary_md = gr.Markdown()
                    feat_box = gr.JSON(label="Feature trace")

            sample_picker.change(
                load_sample,
                inputs=[sample_picker],
                outputs=[from_dd, to_dd, amount_in, process_dd, date_in],
            )
            predict_btn.click(
                predict_one,
                inputs=[from_dd, to_dd, amount_in, process_dd, date_in],
                outputs=[summary_md, feat_box],
            )

        with gr.Tab("Node anomaly explorer"):
            gr.Markdown(
                "Top accounts ranked by mean per-edge reconstruction MSE on a "
                "5,000-edge sample — accounts whose *attribute patterns* don't fit the "
                "structural prior learned by the GAE."
            )
            top_k_slider = gr.Slider(label="Top K", minimum=10, maximum=200, value=50, step=10)
            anomaly_table = gr.Dataframe(value=build_node_anomaly_table(50), wrap=True)
            refresh_btn = gr.Button("Recompute")
            refresh_btn.click(build_node_anomaly_table, inputs=[top_k_slider], outputs=[anomaly_table])

        with gr.Tab("Live check"):
            gr.Markdown(
                "Sample N random edges from the published dataset, run the "
                "fraud classifier, show confusion matrix + ROC against ground truth."
            )
            n_slider = gr.Slider(label="Sample size", minimum=50, maximum=2000, value=300, step=50)
            run_btn = gr.Button("Run", variant="primary")
            with gr.Row():
                cm_plot = gr.Plot(label="Confusion matrix")
                roc_plot = gr.Plot(label="ROC curve")
            check_summary = gr.Markdown()
            run_btn.click(run_live_check, inputs=[n_slider], outputs=[cm_plot, roc_plot, check_summary])

        gr.Markdown(
            """
            ---

            **Honest caveat.**  The synthetic fraud-bias model puts strong local
            signals into edge attributes (40 % round-dollar, 30 % weekend), so a
            simple LR on edge features already gets to AUC 0.91.  GraphSAGE adds
            +0.13 AUC pts on the supervised task; the unsupervised attribute-GAE
            is where graph methods earn their keep here (AUC 0.65 *with no labels*).
            See the [model card](https://huggingface.co/VynFi/je-fraud-gnn) for
            full metrics + a discussion of where the GNN does/doesn't add value.
            """
        )

    return app


if __name__ == "__main__":
    build_app().launch()

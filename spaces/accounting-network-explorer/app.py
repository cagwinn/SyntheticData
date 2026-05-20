"""VynFi Accounting Network Explorer.

Interactive ISO 21378 Level-2 account-class network from
`VynFi/vynfi-journal-entries-1m`.  One node per account class,
one edge per (from_class, to_class) pair aggregated from the
v5.27.0 Method-A `je_network.parquet` (2-line JEs only,
confidence = 1.0).
"""
from __future__ import annotations

import math
from typing import Tuple

import pandas as pd
import streamlit as st
from huggingface_hub import snapshot_download
from streamlit_agraph import Config, Edge, Node, agraph

DATASET_REPO = "VynFi/vynfi-journal-entries-1m"

ACCOUNT_TYPE_COLORS = {
    "asset":     "#2563eb",  # blue
    "liability": "#ea580c",  # orange
    "equity":    "#16a34a",  # green
    "revenue":   "#9333ea",  # purple
    "expense":   "#dc2626",  # red
    "other":     "#6b7280",  # grey
}

st.set_page_config(
    page_title="VynFi Accounting Network Explorer",
    page_icon="🔗",
    layout="wide",
    initial_sidebar_state="expanded",
)


# ─── Data loading ────────────────────────────────────────────────────────────


@st.cache_resource(show_spinner="Downloading je_network + chart_of_accounts from HF Hub…")
def load_data() -> Tuple[pd.DataFrame, pd.DataFrame]:
    base = snapshot_download(
        repo_id=DATASET_REPO,
        repo_type="dataset",
        allow_patterns=["je_network.parquet", "chart_of_accounts.parquet"],
    )
    edges = pd.read_parquet(f"{base}/je_network.parquet")
    coa = pd.read_parquet(f"{base}/chart_of_accounts.parquet")

    # Normalise dtypes
    edges["from_account"] = edges["from_account"].astype(str)
    edges["to_account"] = edges["to_account"].astype(str)
    coa["account_number"] = coa["account_number"].astype(str)
    coa["account_type"] = coa["account_type"].astype(str).str.lower()

    # 4 account numbers in the published COA (1510, 1600, 4900, 7100) appear
    # in two rows with conflicting class mappings — keep the first deterministically
    # so the join doesn't inflate the edge count.
    coa = coa.drop_duplicates(subset=["account_number"], keep="first").reset_index(drop=True)

    return edges, coa


# ─── Aggregation ─────────────────────────────────────────────────────────────


def aggregate_to_class(edges: pd.DataFrame, coa: pd.DataFrame):
    """Join edges with COA on gl_account and aggregate by (from_class, to_class)."""
    coa_slim = coa[
        ["account_number", "account_class", "account_class_name", "account_type"]
    ].copy()

    e = (
        edges.merge(
            coa_slim.rename(
                columns={
                    "account_number": "from_account",
                    "account_class": "from_class",
                    "account_class_name": "from_class_name",
                    "account_type": "from_type",
                }
            ),
            on="from_account",
            how="left",
        )
        .merge(
            coa_slim.rename(
                columns={
                    "account_number": "to_account",
                    "account_class": "to_class",
                    "account_class_name": "to_class_name",
                    "account_type": "to_type",
                }
            ),
            on="to_account",
            how="left",
        )
        .dropna(subset=["from_class", "to_class"])
    )

    class_edges = (
        e.groupby(["from_class", "to_class"], as_index=False)
        .agg(
            total_amount=("amount", "sum"),
            edge_count=("edge_id", "count"),
            fraud_count=("is_fraud", "sum"),
            anomaly_count=("is_anomaly", "sum"),
        )
    )

    out = (
        e.groupby("from_class", as_index=False)
        .agg(out_amount=("amount", "sum"), out_count=("edge_id", "count"))
        .rename(columns={"from_class": "account_class"})
    )
    inn = (
        e.groupby("to_class", as_index=False)
        .agg(in_amount=("amount", "sum"), in_count=("edge_id", "count"))
        .rename(columns={"to_class": "account_class"})
    )
    nodes = pd.merge(out, inn, on="account_class", how="outer").fillna(0)

    meta = (
        coa.groupby("account_class", as_index=False)
        .agg(
            account_class_name=("account_class_name", "first"),
            account_type=("account_type", "first"),
        )
    )
    nodes = nodes.merge(meta, on="account_class", how="left")
    nodes["account_class_name"] = nodes["account_class_name"].fillna(nodes["account_class"])
    nodes["account_type"] = nodes["account_type"].fillna("other")
    nodes["total_flow"] = nodes["in_amount"] + nodes["out_amount"]
    nodes["total_count"] = nodes["in_count"] + nodes["out_count"]

    return nodes, class_edges


# ─── Formatters ──────────────────────────────────────────────────────────────


def fmt_money(x: float) -> str:
    sign = "-" if x < 0 else ""
    x = abs(float(x))
    if x >= 1e12:
        return f"{sign}${x / 1e12:.2f}T"
    if x >= 1e9:
        return f"{sign}${x / 1e9:.2f}B"
    if x >= 1e6:
        return f"{sign}${x / 1e6:.2f}M"
    if x >= 1e3:
        return f"{sign}${x / 1e3:.1f}K"
    return f"{sign}${x:.0f}"


def node_size(amount: float, max_amount: float) -> int:
    # Smaller nodes (7–22px, was 18–60) so the 23 class nodes don't overlap
    # into a single clump; the barnesHut spacing below does the rest.
    if amount <= 0 or max_amount <= 0:
        return 7
    ratio = math.log10(amount + 1.0) / max(math.log10(max_amount + 1.0), 1.0)
    return int(7 + ratio * 15)


def edge_width(amount: float, max_amount: float) -> int:
    if amount <= 0 or max_amount <= 0:
        return 1
    ratio = math.log10(amount + 1.0) / max(math.log10(max_amount + 1.0), 1.0)
    return max(1, int(ratio * 8))


# ─── Sidebar — filters ───────────────────────────────────────────────────────


edges_raw, coa_raw = load_data()

st.title("🔗 VynFi Accounting Network Explorer")
st.caption(
    "ISO 21378 Level-2 account-class flows from "
    "[`VynFi/vynfi-journal-entries-1m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m) · "
    "Method-A edge list (one edge per 2-line JE) · v5.27.0"
)

with st.sidebar:
    st.header("Filters")

    processes = sorted(edges_raw["business_process"].dropna().unique().tolist())
    selected_processes = st.multiselect(
        "Business process",
        processes,
        default=processes,
        help="P2P = procure-to-pay · O2C = order-to-cash · R2R = record-to-report · "
        "H2R = hire-to-retire · A2R = adjust-to-report",
    )

    col_a, col_b = st.columns(2)
    with col_a:
        fraud_only = st.checkbox("Fraud only", value=False)
    with col_b:
        anomaly_only = st.checkbox("Anomaly only", value=False)

    # Fine-grained fraud-typology filter (v5.27 — `fraud_type` on each edge).
    # Defensive: only render if the column is present (older parquet snapshots
    # lack it). Selecting a typology implies fraud-only.
    fraud_types = (
        sorted(t for t in edges_raw["fraud_type"].dropna().unique().tolist() if t)
        if "fraud_type" in edges_raw.columns
        else []
    )
    selected_fraud_types = (
        st.multiselect(
            "Fraud type",
            fraud_types,
            default=[],
            help="Show only edges of the selected fraud typologies (implies fraud).",
        )
        if fraud_types
        else []
    )

    st.divider()

    min_amount_log = st.slider(
        "Min edge total (10ⁿ)",
        min_value=0,
        max_value=12,
        value=0,
        step=1,
        help="Hide class-pairs whose summed flow is below 10ⁿ.",
    )
    top_n = st.slider("Top N edges", min_value=20, max_value=400, value=120, step=20)

    st.divider()

    layout_mode = st.radio(
        "Layout",
        ["force-directed", "hierarchical"],
        horizontal=True,
    )

    st.divider()
    st.caption(
        f"**Source rows:** {len(edges_raw):,} edges · {len(coa_raw):,} accounts  \n"
        f"_v5.27.0 · ChaCha8 seed `20260507`_"
    )


# ─── Filter the raw edges ────────────────────────────────────────────────────


filt = edges_raw[edges_raw["business_process"].isin(selected_processes)]
if fraud_only:
    filt = filt[filt["is_fraud"]]
if anomaly_only:
    filt = filt[filt["is_anomaly"]]
if selected_fraud_types:
    filt = filt[filt["fraud_type"].isin(selected_fraud_types)]

if filt.empty:
    st.warning("No edges match the current filter combination — relax the filters.")
    st.stop()

nodes_df, class_edges_df = aggregate_to_class(filt, coa_raw)

class_edges_df = class_edges_df[class_edges_df["total_amount"] >= 10**min_amount_log]
class_edges_df = class_edges_df.nlargest(top_n, "total_amount")

keep_classes = set(class_edges_df["from_class"]) | set(class_edges_df["to_class"])
nodes_df = nodes_df[nodes_df["account_class"].isin(keep_classes)].copy()

if class_edges_df.empty or nodes_df.empty:
    st.warning("Filters produced an empty graph — relax the min-amount cutoff.")
    st.stop()


# ─── Build agraph nodes/edges ────────────────────────────────────────────────


max_node = nodes_df["total_flow"].max()
max_edge = class_edges_df["total_amount"].max()

agraph_nodes = []
for _, n in nodes_df.iterrows():
    color = ACCOUNT_TYPE_COLORS.get(str(n["account_type"]).lower(), ACCOUNT_TYPE_COLORS["other"])
    label = f"{n['account_class']}\n{str(n['account_class_name'])[:24]}"
    title = (
        f"Class {n['account_class']} ({n['account_type']})\n"
        f"{n['account_class_name']}\n"
        f"Total flow: {fmt_money(n['total_flow'])}\n"
        f"Edges: {int(n['total_count'])}\n"
        f"In: {fmt_money(n['in_amount'])} ({int(n['in_count'])})\n"
        f"Out: {fmt_money(n['out_amount'])} ({int(n['out_count'])})"
    )
    agraph_nodes.append(
        Node(
            id=str(n["account_class"]),
            label=label,
            title=title,
            size=node_size(n["total_flow"], max_node),
            color=color,
            font={"color": "#e5e7eb", "size": 9, "face": "monospace"},
            shape="dot",
        )
    )

agraph_edges = []
for _, e in class_edges_df.iterrows():
    fraud_pct = (e["fraud_count"] / e["edge_count"] * 100) if e["edge_count"] else 0.0
    title = (
        f"{e['from_class']} → {e['to_class']}\n"
        f"Total: {fmt_money(e['total_amount'])}\n"
        f"Edges: {int(e['edge_count'])}\n"
        f"Fraud: {int(e['fraud_count'])} ({fraud_pct:.1f}%)\n"
        f"Anomaly: {int(e['anomaly_count'])}"
    )
    color = "#dc2626" if e["fraud_count"] > 0 else "#94a3b8"
    agraph_edges.append(
        Edge(
            source=str(e["from_class"]),
            target=str(e["to_class"]),
            title=title,
            color=color,
            type="CURVE_SMOOTH",
            width=edge_width(e["total_amount"], max_edge),
        )
    )


# ─── Layout ──────────────────────────────────────────────────────────────────


config = Config(
    width=1100,
    height=820,
    directed=True,
    physics=(layout_mode == "force-directed"),
    hierarchical=(layout_mode == "hierarchical"),
)

# Spread the force-directed layout: stronger node repulsion + longer edges +
# hard overlap avoidance, so the 23 account classes fan out instead of
# collapsing into one clump. vis-network barnesHut tuning (defaults in
# parentheses) — `config.physics` is the full physics options object.
if layout_mode == "force-directed":
    config.physics["barnesHut"] = {
        "gravitationalConstant": -14000,  # (-2000) much stronger repulsion
        "centralGravity": 0.12,           # (0.3)   less pull toward centre
        "springLength": 260,              # (95)    longer edges = more distance
        "springConstant": 0.02,           # (0.04)  softer springs
        "damping": 0.4,
        "avoidOverlap": 1.0,              # (0)     nodes must not overlap
    }
    config.physics["stabilization"] = {
        "enabled": True,
        "iterations": 400,
        "fit": True,
    }

graph_col, side_col = st.columns([3, 1])
with graph_col:
    selected = agraph(nodes=agraph_nodes, edges=agraph_edges, config=config)

with side_col:
    st.subheader("Summary")
    sm1, sm2 = st.columns(2)
    sm1.metric("Classes", len(nodes_df))
    sm2.metric("Edges", len(class_edges_df))
    st.metric("Total flow", fmt_money(class_edges_df["total_amount"].sum()))
    st.metric("Fraud edges", int(class_edges_df["fraud_count"].sum()))
    st.metric("Anomaly edges", int(class_edges_df["anomaly_count"].sum()))

    st.divider()

    if selected:
        n_match = nodes_df[nodes_df["account_class"] == selected]
        if not n_match.empty:
            n = n_match.iloc[0]
            color = ACCOUNT_TYPE_COLORS.get(
                str(n["account_type"]).lower(), ACCOUNT_TYPE_COLORS["other"]
            )
            st.markdown(
                f"<h4 style='margin:0'>"
                f"<span style='color:{color}'>●</span> "
                f"<code>{n['account_class']}</code></h4>",
                unsafe_allow_html=True,
            )
            st.markdown(f"**{n['account_class_name']}**  \n_{n['account_type']}_")
            st.markdown(
                f"- Total flow: **{fmt_money(n['total_flow'])}**  \n"
                f"- Out: {fmt_money(n['out_amount'])} ({int(n['out_count'])})  \n"
                f"- In: {fmt_money(n['in_amount'])} ({int(n['in_count'])})"
            )

            outs = class_edges_df[class_edges_df["from_class"] == selected].nlargest(
                5, "total_amount"
            )
            if not outs.empty:
                st.markdown("**Top outgoing**")
                for _, oe in outs.iterrows():
                    st.markdown(
                        f"→ `{oe['to_class']}` · {fmt_money(oe['total_amount'])} "
                        f"({int(oe['edge_count'])} edges)"
                    )

            ins = class_edges_df[class_edges_df["to_class"] == selected].nlargest(
                5, "total_amount"
            )
            if not ins.empty:
                st.markdown("**Top incoming**")
                for _, ie in ins.iterrows():
                    st.markdown(
                        f"← `{ie['from_class']}` · {fmt_money(ie['total_amount'])} "
                        f"({int(ie['edge_count'])} edges)"
                    )

            subs = (
                coa_raw[coa_raw["account_class"] == selected]
                .groupby(["account_sub_class", "account_sub_class_name"], as_index=False)
                .size()
            )
            if not subs.empty:
                with st.expander(f"Level-3 sub-classes ({len(subs)})"):
                    for _, s in subs.iterrows():
                        st.markdown(
                            f"`{s['account_sub_class']}` — {s['account_sub_class_name']}"
                        )
        else:
            st.info("Selected class is not currently visible — relax filters.")
    else:
        st.info("Click a node in the graph to drill in.")

st.divider()

with st.expander("Top edges (table view)", expanded=False):
    table = class_edges_df.assign(
        total=class_edges_df["total_amount"].apply(fmt_money),
        fraud_pct=(class_edges_df["fraud_count"] / class_edges_df["edge_count"] * 100).round(2),
    )[
        [
            "from_class",
            "to_class",
            "total",
            "edge_count",
            "fraud_count",
            "anomaly_count",
            "fraud_pct",
        ]
    ].rename(
        columns={
            "from_class": "From",
            "to_class": "To",
            "total": "Total $",
            "edge_count": "Edges",
            "fraud_count": "Fraud",
            "anomaly_count": "Anomaly",
            "fraud_pct": "Fraud %",
        }
    )
    st.dataframe(table, use_container_width=True, hide_index=True)

with st.expander("About this Space", expanded=False):
    st.markdown(
        """
**What this is.**  An interactive view of the v5.27.0 Method-A
accounting network published in
[`VynFi/vynfi-journal-entries-1m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-1m).
The 61 656 line-level edges are aggregated to ISO 21378 Level-2
account classes (~30 nodes), so you can see the macro money-flow
structure at a glance.

**Method-A.**  In v5.27.0 the JE network defaults to "Method A"
from Ivertowski 2024: exactly **one edge per 2-line journal entry**,
confidence = 1.0.  This avoids the Cartesian explosion (225 M edges
on 1 M JEs) that the legacy `cartesian` method produced, and gives
a clean topology for graph-ML training.

**Edge attributes.**  `business_process` (P2P / O2C / R2R / H2R / A2R),
`is_fraud`, `is_anomaly`, `fraud_type` (fine-grained typology — v5.27,
filterable in the sidebar), `posting_date`, `amount`, `confidence`,
`predecessor_edge_id` (chains 2-line JEs into longer document flows).

**Drill-down.**  Click any class node to see the underlying Level-3
sub-classes (`A.A.A` / `A.A.B` / …) and the top in/out flows.

**Source.**  [GitHub: mivertowski/SyntheticData](https://github.com/mivertowski/SyntheticData) ·
[Companion paper (SSRN)](https://ssrn.com/abstract=6538639)
        """
    )

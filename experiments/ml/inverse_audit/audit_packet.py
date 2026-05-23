"""Audit-packet generator — turn the top-N JEs (by relational_score) into a
prioritised, auditor-ready review list.

For each top JE, the packet contains:
  - rank, je_id, relational_score
  - per-feature breakdown (raw value + z + elevated-vs-rest-p90 flag)
  - the top 3 feature contributors to the score (which signals fired)
  - the original GL lines for that JE (account / amount / description / date / source /
    cost-/profit-center / trading_partner / etc.) — so the auditor sees exactly what
    posted, without having to cross-reference

PRIVACY CONTRACT:
- This MODULE commits no row content (just schema + logic).
- The OUTPUT FILES (JSON + Markdown summary) DO contain JE descriptions, amounts,
  counterparties — that's the whole point of the packet, but they must stay LOCAL
  (gitignored, never echoed in commits / PROGRESS / FINDINGS / memory).
- Paths are runtime args only.

Usage:
    python -m inverse_audit.audit_packet \\
        --gl-parquet      <corpus parquet>          (the original GL)
        --scored-parquet  <scored parquet>          (from corpus_runner)
        --out             <local out dir>           (gitignored)
        --top-n           50                        (default)
"""
from __future__ import annotations

import argparse
import json
from datetime import datetime
from pathlib import Path

import numpy as np
import pandas as pd

from inverse_audit.corpus_runner import CORPUS_COLMAP
from inverse_audit.relational.graph_scorer import _SCORE_FEATURES

# Columns that go into the packet's per-line payload (only if present in the GL).
# Kept conservative — what an auditor needs to look at a posting.
_LINE_COLS = [
    "gl_account", "debit_amount", "credit_amount",
    "JE Description", "JE Line Description",
    "Effective Date", "Entry Date", "Period",
    "source", "business_unit", "cost_center", "profit_center",
    "trading_partner",
    "Functional Currency Code", "Reporting Amount", "Reporting Currency Code",
]


def _coerce_value(v):
    """JSON-safe scalar coercion."""
    if pd.isna(v):
        return None
    if isinstance(v, (bool, np.bool_)):
        return bool(v)
    if isinstance(v, (np.integer,)):
        return int(v)
    if isinstance(v, (np.floating,)):
        return float(v)
    if isinstance(v, (int, float)):
        return v
    return str(v)


def assemble_packet(scored: pd.DataFrame, gl: pd.DataFrame, top_n: int) -> list[dict]:
    # Canonicalise the GL columns the same way corpus_runner does (so the line view
    # is consistent with what the scorer saw).
    gl = gl.rename(columns=CORPUS_COLMAP)
    if "Functional Amount" in gl.columns:
        fa = pd.to_numeric(gl["Functional Amount"], errors="coerce").fillna(0.0)
        gl["debit_amount"]  = fa.where(fa > 0, 0.0)
        gl["credit_amount"] = (-fa).where(fa < 0, 0.0)

    top = scored.nlargest(top_n, "relational_score").copy()
    top["rank"] = range(1, len(top) + 1)
    rest = scored[~scored.index.isin(top.index)] if scored.index.name else scored.iloc[len(top):]
    rest_p90 = {f: float(rest[f].quantile(0.9)) for f in _SCORE_FEATURES if f in rest.columns}

    keep_line_cols = [c for c in _LINE_COLS if c in gl.columns]
    gl_idx = gl.groupby("document_id", sort=False)

    packet: list[dict] = []
    for _, row in top.iterrows():
        je_id = row["je_id"] if "je_id" in row.index else row.name
        je_id = str(je_id)

        feats: dict[str, dict] = {}
        for f in _SCORE_FEATURES:
            if f not in row.index:
                continue
            v = float(row[f])
            z = _coerce_value(row.get(f + "_z"))
            feats[f] = {"value": v, "z": z,
                        "elevated_vs_rest_p90": bool(v > rest_p90.get(f, np.inf))}

        # Top 3 z-contributors (which feature explanations rank highest for this JE)
        z_pairs = [(f, row.get(f + "_z")) for f in _SCORE_FEATURES if f + "_z" in row.index]
        z_pairs = [(f, float(z)) for f, z in z_pairs if pd.notna(z)]
        z_pairs.sort(key=lambda kv: -kv[1])
        top_contribs = [{"feature": f, "z": z} for f, z in z_pairs[:3]]

        # The JE's original lines
        try:
            je_lines_df = gl_idx.get_group(je_id)
        except KeyError:
            je_lines_df = gl[gl["document_id"].astype(str) == je_id]
        lines = [{c: _coerce_value(r[c]) for c in keep_line_cols if c in r.index}
                 for _, r in je_lines_df.iterrows()]

        packet.append({
            "rank": int(row["rank"]),
            "je_id": je_id,
            "relational_score": float(row["relational_score"]),
            "n_lines": len(lines),
            "top_contributing_features": top_contribs,
            "features": feats,
            "lines": lines,
        })
    return packet


def render_markdown(packet: list[dict], header: dict) -> str:
    """Compact Markdown summary — one section per JE, easy for an auditor to skim."""
    lines = [
        f"# Audit packet — top {header['top_n']} JEs by relational_score",
        "",
        f"- generated: {datetime.now().isoformat(timespec='seconds')}",
        f"- source file tag: `{header.get('source_tag', '?')}`  ·  total JEs scored: "
        f"{header.get('n_scored_jes', '?')}",
        f"- score distribution (p50 / p90 / p99 / max): {header.get('score_p50')} / "
        f"{header.get('score_p90')} / {header.get('score_p99')} / {header.get('score_max')}",
        "",
        "Each entry: rank · JE id · score · top contributors · per-feature flags · lines.",
        "",
    ]
    for e in packet:
        contribs = ", ".join(f"`{c['feature']}` (z={c['z']:.2f})"
                              for c in e["top_contributing_features"])
        lines += [
            f"## #{e['rank']} — JE `{e['je_id']}` — score **{e['relational_score']:.2f}** "
            f"(n_lines={e['n_lines']})",
            f"top contributors: {contribs}",
            "",
            "| feature | value | z | elevated vs rest-p90 |",
            "|---|--:|--:|:--:|",
        ]
        for f, fv in e["features"].items():
            elev = "✓" if fv["elevated_vs_rest_p90"] else ""
            z = fv["z"]
            z_str = "—" if z is None else f"{z:.2f}"
            lines.append(f"| `{f}` | {fv['value']:.3f} | {z_str} | {elev} |")
        lines += ["", "### lines (from GL)", ""]
        if e["lines"]:
            cols = list(e["lines"][0].keys())
            lines.append("| " + " | ".join(cols) + " |")
            lines.append("|" + "|".join(["---"] * len(cols)) + "|")
            for ln in e["lines"]:
                lines.append("| " + " | ".join(
                    str(ln.get(c, "")) if ln.get(c) is not None else "" for c in cols) + " |")
        lines.append("")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--gl-parquet",     type=Path, required=True,
                    help="original corpus GL parquet (runtime arg; never committed)")
    ap.add_argument("--scored-parquet", type=Path, required=True,
                    help="graph_scores.parquet from corpus_runner")
    ap.add_argument("--out",            type=Path, required=True,
                    help="output dir (local — packets contain row content; gitignored)")
    ap.add_argument("--top-n", type=int, default=50)
    a = ap.parse_args(argv)
    a.out.mkdir(parents=True, exist_ok=True)

    scored = pd.read_parquet(a.scored_parquet)
    gl = pd.read_parquet(a.gl_parquet)

    pkt = assemble_packet(scored, gl, a.top_n)
    rs = scored["relational_score"]
    header = {
        "top_n": len(pkt),
        "n_scored_jes": len(scored),
        "score_p50": round(float(rs.quantile(0.50)), 2),
        "score_p90": round(float(rs.quantile(0.90)), 2),
        "score_p99": round(float(rs.quantile(0.99)), 2),
        "score_max": round(float(rs.max()), 2),
        "source_tag": a.gl_parquet.stem,
    }
    (a.out / "audit_packet.json").write_text(json.dumps(
        {"header": header, "packet": pkt}, indent=2))
    (a.out / "audit_packet.md").write_text(render_markdown(pkt, header))
    print(f"AUDIT_PACKET_DONE  top={len(pkt)}  scored_jes={len(scored):,}  "
          f"score range p50={header['score_p50']} → max={header['score_max']}")
    print(f"  json: {a.out}/audit_packet.json")
    print(f"  md:   {a.out}/audit_packet.md")


if __name__ == "__main__":
    main()

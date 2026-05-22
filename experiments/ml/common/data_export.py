"""Export per-track training tensors from the corpus.

Reads corpus parquet from `$DATASYNTH_CORPUS_DIR` (never hard-coded), applies
the `ColumnMap`, and writes track-specific artifacts under `--out`. All
outputs are gitignored.

Usage:
    export DATASYNTH_CORPUS_DIR=/path/to/private/corpus
    python -m common.data_export --track gnn      --out data/gnn
    python -m common.data_export --track sequence --out data/sequence
    python -m common.data_export --track flow     --out data/flow

Design notes
------------
* CPU-only, streaming where possible — safe to run on a laptop; does NOT
  invoke the orchestrator (which OOMs small boxes).
* Emits ONLY aggregated / structural tensors, never row-level corpus text.
  The GNN track in particular emits an *anonymized* edge index (integer node
  ids), so committed-by-accident artifacts would still carry no names — but
  they are gitignored regardless.
"""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

from .schema import ColumnMap


def _corpus_dir() -> Path:
    d = os.environ.get("DATASYNTH_CORPUS_DIR")
    if not d:
        sys.exit(
            "ERROR: set DATASYNTH_CORPUS_DIR to the private corpus directory "
            "(never hard-code it)."
        )
    p = Path(d)
    if not p.is_dir():
        sys.exit(f"ERROR: DATASYNTH_CORPUS_DIR={d} is not a directory")
    return p


def _load_je_frame(corpus: Path, cols: ColumnMap):
    """Load the JE-line table as a pandas DataFrame with canonical columns.

    The corpus ships one parquet per client; we concatenate. TODO after the
    first run: confirm the per-client file glob + any client-id column to
    keep entity namespaces disjoint across clients (see SP3.11 namespace
    canonicalisation).
    """
    import pandas as pd
    import pyarrow.parquet as pq

    files = sorted(corpus.glob("JE_*.parquet"))
    if not files:
        sys.exit(f"ERROR: no JE_*.parquet under {corpus}")
    frames = []
    rename = {getattr(cols, f): f for f in cols.__dataclass_fields__}  # noqa: SLF001
    for fp in files:
        tbl = pq.read_table(fp)
        present = [c for c in tbl.column_names if c in rename]
        df = tbl.select(present).to_pandas().rename(columns=rename)
        df["__client__"] = fp.stem  # keep namespaces disjoint
        frames.append(df)
    return pd.concat(frames, ignore_index=True)


# --------------------------------------------------------------------------
# Track exporters. Each writes a small set of .pt / .parquet artifacts.
# --------------------------------------------------------------------------
def export_gnn(corpus: Path, cols: ColumnMap, out: Path) -> None:
    """Edge list + node features for the relational graph(s).

    Builds three graphs the symbolic motif samplers approximate today:
      * TP co-occurrence (trading partners sharing a JE / source)
      * vendor / counterparty network (from gl_account ↔ trading_partner)
      * IC bilateral edges (cross-client matched flows)

    Emits anonymized integer node ids + a node-degree / source-mix feature
    matrix. See gnn/SPEC.md § Data.
    """
    raise NotImplementedError(
        "TODO(gnn): build edge_index + node features. Pseudocode in "
        "gnn/SPEC.md § Data — node = (client, trading_partner); edge weight = "
        "co-occurrence count; node feat = [degree, source-mix histogram, "
        "active-window length]. Write out/edge_index.pt, out/node_feat.pt, "
        "out/node_ids.parquet (anonymized)."
    )


def export_sequence(corpus: Path, cols: ColumnMap, out: Path, max_streams: int = 60_000,
                    seq_len: int = 128) -> None:
    """Per-(client, source) ordered event-token streams → streams.pt + vocab.json.

    Factorized fields matching `EventStreamTransformer`: (dt, lines,
    account_class, weekday, hour_band), 0 = pad. Δt + line-count carry the
    inter-event-time / burst signal (the autocorrelation gap). hour_band is a
    constant single band — the corpus dates carry no time-of-day. Processed
    per-client to bound memory over the 50M-row corpus. See sequence/SPEC.md.
    """
    import json

    import numpy as np
    import pandas as pd
    import torch

    acc = _account_class_map(corpus)
    DT_EDGES = [1, 2, 4, 8, 15, 30]   # → digitize 0..6, +1 → 1..7 (vocab 8)
    LC_EDGES = [2, 3, 5, 9, 17]       # → digitize 0..5, +1 → 1..6 (vocab 7)

    # First pass (cheap): rank sources by JE volume for the 0..62 source-id map.
    src_counts: dict[str, int] = {}
    files = sorted(corpus.glob("JE_*.parquet"))
    for fp in files:
        s = pd.read_parquet(fp, columns=[cols.source])[cols.source].astype(str)
        for k, v in s.value_counts().items():
            src_counts[k] = src_counts.get(k, 0) + int(v)
    top_src = [s for s, _ in sorted(src_counts.items(), key=lambda kv: -kv[1])[:63]]
    src_ids = {s: i for i, s in enumerate(top_src)}

    classes: dict[str, int] = {}
    fld = {k: [] for k in ("dt", "lines", "account_class", "weekday", "hour_band")}
    src_id_list: list[int] = []

    def _pad(a: np.ndarray) -> np.ndarray:
        a = a[:seq_len].astype(np.int64)
        z = np.zeros(seq_len, dtype=np.int64)
        z[: len(a)] = a
        return z

    for fp in files:
        if len(src_id_list) >= max_streams:
            break
        d = pd.read_parquet(fp, columns=[cols.source, cols.entry_date, cols.je_number, cols.gl_account])
        d.columns = ["source", "date", "je", "gl"]
        d["date"] = pd.to_datetime(d["date"], errors="coerce")
        d = d.dropna(subset=["date"])
        d["cls"] = d["gl"].astype(str).map(acc).fillna("UNK")
        je = d.groupby(["source", "je"]).agg(date=("date", "first"), lines=("je", "size"),
                                             cls=("cls", "first")).reset_index()
        for src, g in je.groupby("source"):
            if len(g) < 3:
                continue
            g = g.sort_values("date")
            dts = g["date"].diff().dt.days.fillna(0).clip(0, 3650).to_numpy()
            cl_id = np.array([classes.setdefault(c, len(classes) + 1) for c in g["cls"]], dtype=np.int64)
            fld["dt"].append(_pad(np.digitize(dts, DT_EDGES) + 1))
            fld["lines"].append(_pad(np.digitize(g["lines"].to_numpy(), LC_EDGES) + 1))
            fld["account_class"].append(_pad(cl_id))
            fld["weekday"].append(_pad(g["date"].dt.weekday.to_numpy() + 1))
            fld["hour_band"].append(_pad(np.ones(len(g), dtype=np.int64)))
            src_id_list.append(src_ids.get(str(src), 63))
            if len(src_id_list) >= max_streams:
                break

    blob = {k: torch.from_numpy(np.stack(v)) for k, v in fld.items()}
    blob["source_id"] = torch.from_numpy(np.array(src_id_list, dtype=np.int64))
    torch.save(blob, out / "streams.pt")
    sizes = {"dt": 8, "lines": 7, "account_class": len(classes) + 1, "weekday": 8, "hour_band": 2}
    (out / "vocab.json").write_text(json.dumps({"sizes": sizes, "n_streams": len(src_id_list), "T": seq_len}, indent=2))
    print(f"[sequence] {len(src_id_list)} streams (T={seq_len}), {len(classes)} account-classes "
          f"-> {out/'streams.pt'}")


def _account_class_map(corpus: Path) -> dict[str, str]:
    """Build {gl_account: account_class} from the corpus COA_*.parquet files.

    Join key column ``c`` is the zero-padded GL account number; ``Account
    Class`` is the ISO-style class label. Account numbers are consistent across
    clients, so a global map is fine (first wins on the rare conflict).
    """
    import pandas as pd

    m: dict[str, str] = {}
    for fp in sorted(corpus.glob("COA_*.parquet")):
        try:
            c = pd.read_parquet(fp, columns=["c", "Account Class"])
        except Exception:  # noqa: BLE001 — skip a malformed/empty COA shard
            continue
        for acct, cls in zip(c["c"].astype(str), c["Account Class"].astype(str)):
            m.setdefault(acct, cls)
    return m


def export_flow(corpus: Path, cols: ColumnMap, out: Path) -> None:
    """Amount samples + one-hot account-class conditioning → amounts.parquet.

    ``y = signed log1p(|amount|)``; conditioning = one-hot(account_class) from
    the COA join. Source has thousands of corpus levels (a finding in itself,
    not a useful one-hot), so account-class is the amount-shape conditioning
    axis. The extreme tail is clipped at the 99.9th percentile (privacy: don't
    memorize rare exact large amounts — flow/SPEC.md § Privacy). Aggregated
    numeric only, gitignored. See flow/SPEC.md § Data.
    """
    import json

    import numpy as np
    import pandas as pd

    acc = _account_class_map(corpus)
    parts = []
    for fp in sorted(corpus.glob("JE_*.parquet")):
        d = pd.read_parquet(fp, columns=[cols.amount, cols.gl_account])
        d.columns = ["amount", "gl_account"]
        parts.append(d)
    df = pd.concat(parts, ignore_index=True)
    df["amount"] = pd.to_numeric(df["amount"], errors="coerce")
    df = df.dropna(subset=["amount"])
    df = df[df["amount"] != 0.0]
    df["account_class"] = df["gl_account"].astype(str).map(acc).fillna("UNK")
    df["y"] = np.sign(df["amount"]) * np.log1p(np.abs(df["amount"]))
    hi = float(df["y"].quantile(0.999))
    df["y"] = df["y"].clip(upper=hi)
    if len(df) > 3_000_000:
        df = df.sample(3_000_000, random_state=0).reset_index(drop=True)
    onehot = pd.get_dummies(df["account_class"], prefix="cls").astype("float32")
    out_df = pd.concat(
        [df[["y"]].astype("float32").reset_index(drop=True), onehot.reset_index(drop=True)],
        axis=1,
    )
    out_df.to_parquet(out / "amounts.parquet")
    (out / "flow_meta.json").write_text(
        json.dumps(
            {"n": int(len(out_df)), "cond_cols": list(onehot.columns),
             "n_classes": int(onehot.shape[1]), "y_clip_hi": hi}, indent=2
        )
    )
    print(f"[flow] {len(out_df):,} amounts, {onehot.shape[1]} account-class conds "
          f"-> {out/'amounts.parquet'}")


EXPORTERS = {
    "gnn": export_gnn,
    "sequence": export_sequence,
    "flow": export_flow,
}


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--track", required=True, choices=sorted(EXPORTERS))
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument(
        "--column-map",
        type=Path,
        default=None,
        help="local (gitignored) YAML mapping canonical->corpus column names",
    )
    args = ap.parse_args(argv)

    cols = ColumnMap.from_yaml(str(args.column_map)) if args.column_map else ColumnMap()
    corpus = _corpus_dir()
    args.out.mkdir(parents=True, exist_ok=True)

    print(f"[data_export] track={args.track} corpus={corpus} -> {args.out}")
    EXPORTERS[args.track](corpus, cols, args.out)
    print("[data_export] done")


if __name__ == "__main__":
    main()

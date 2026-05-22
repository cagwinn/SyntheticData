"""Sequence-track lift: does the autoregressive transformer capture temporal
structure the marginal (iid) sampler misses?

The shipped generator draws Δt and line-count *independently per event*. This
measures the information gain of conditioning on history: held-out per-token
NLL of the trained transformer vs an iid per-field marginal baseline (the
field's own entropy). NLL_marginal − NLL_transformer > 0 ⇒ the model captures
joint / autocorrelated structure the marginal sampler cannot — per field and
pooled.

    python -m sequence.characterize --data data/sequence --weights weights/sequence
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import torch
import torch.nn.functional as F

from .model import EventStreamTransformer, FieldVocab

FIELDS = ["dt", "lines", "account_class", "weekday", "hour_band"]


@torch.no_grad()
def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--data", type=Path, required=True)
    ap.add_argument("--weights", type=Path, required=True)
    ap.add_argument("--val-frac", type=float, default=0.2)
    ap.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    args = ap.parse_args(argv)
    dev = torch.device(args.device)

    blob = torch.load(args.data / "streams.pt")
    sizes = json.loads((args.data / "vocab.json").read_text())["sizes"]
    vocab = FieldVocab(**sizes)
    ckpt = torch.load(args.weights / "stream_tf.pt", map_location=dev)
    model = EventStreamTransformer(vocab).to(dev)
    model.load_state_dict(ckpt["model"])
    model.train(False)

    n = blob["dt"].shape[0]
    nval = max(1, int(n * args.val_frac))
    tr = slice(nval, None)
    va = slice(0, nval)

    # ── Transformer per-token NLL on held-out (teacher-forced) ──────────────
    tok_va = {f: blob[f][va].to(dev) for f in FIELDS}
    logits = model(tok_va, blob["source_id"][va].to(dev))
    tf_nll = {}
    for f in FIELDS:
        pred = logits[f][:, :-1].reshape(-1, logits[f].size(-1))
        tgt = tok_va[f][:, 1:].reshape(-1)
        keep = tgt > 0  # ignore pad
        tf_nll[f] = float(F.cross_entropy(pred[keep], tgt[keep]).item())

    # ── iid marginal baseline: each field's own entropy on the train split ──
    marg_nll = {}
    for f in FIELDS:
        toks = blob[f][tr].reshape(-1).numpy()
        toks = toks[toks > 0]
        if toks.size == 0:
            marg_nll[f] = 0.0
            continue
        counts = np.bincount(toks, minlength=sizes[f]).astype(np.float64)
        p = counts / counts.sum()
        nz = p > 0
        marg_nll[f] = float(-(p[nz] * np.log(p[nz])).sum())  # nats

    print(f"{'field':<16}{'transformer':>14}{'marginal(iid)':>16}{'lift(nats)':>14}")
    print("-" * 60)
    tf_tot = marg_tot = 0.0
    for f in FIELDS:
        lift = marg_nll[f] - tf_nll[f]
        tf_tot += tf_nll[f]
        marg_tot += marg_nll[f]
        print(f"{f:<16}{tf_nll[f]:>14.4f}{marg_nll[f]:>16.4f}{lift:>14.4f}")
    print("-" * 60)
    print(f"{'TOTAL/token':<16}{tf_tot:>14.4f}{marg_tot:>16.4f}{marg_tot - tf_tot:>14.4f}")
    print("\nInterpretation: positive lift = the AR model predicts events better "
          "than drawing each field iid from its marginal — i.e. it captures the "
          "joint/temporal structure the per-event marginal sampler discards.")


if __name__ == "__main__":
    main()

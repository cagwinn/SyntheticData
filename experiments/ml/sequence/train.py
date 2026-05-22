"""Train the event-stream transformer (Track 2).

    python -m sequence.train --data data/sequence --out weights/sequence --epochs 30
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import torch
from torch.utils.data import DataLoader, TensorDataset

from .model import EventStreamTransformer, FieldVocab


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--data", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--epochs", type=int, default=30)
    ap.add_argument("--batch-size", type=int, default=64)
    ap.add_argument("--lr", type=float, default=3e-4)
    ap.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    args = ap.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)
    dev = torch.device(args.device)

    vocab = FieldVocab(**json.loads((args.data / "vocab.json").read_text())["sizes"])
    model = EventStreamTransformer(vocab).to(dev)
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr)

    # streams.pt: dict of (N, T) field tensors + (N,) source_id. TODO(seq):
    # confirm packing in data_export; this loader assumes that layout.
    blob = torch.load(args.data / "streams.pt")
    fields = ["dt", "lines", "account_class", "weekday", "hour_band"]
    ds = TensorDataset(*[blob[f] for f in fields], blob["source_id"])
    dl = DataLoader(ds, batch_size=args.batch_size, shuffle=True)

    model.train()
    for epoch in range(1, args.epochs + 1):
        running = 0.0
        for batch in dl:
            *field_tensors, source_id = [t.to(dev) for t in batch]
            tokens = dict(zip(fields, field_tensors))
            logits = model(tokens, source_id)
            loss = model.loss(logits, tokens)
            opt.zero_grad()
            loss.backward()
            opt.step()
            running += loss.item()
        print(f"epoch {epoch:3d}  loss={running/len(dl):.4f}")

    torch.save({"model": model.state_dict(), "vocab": vars(vocab)},
               args.out / "stream_tf.pt")
    print(f"[sequence.train] saved {args.out/'stream_tf.pt'}")


if __name__ == "__main__":
    main()

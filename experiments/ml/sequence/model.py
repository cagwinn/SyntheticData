"""Decoder-only transformer over JE event-token streams (Track 2).

Factorized multi-field head: each event token is the product of independent
softmaxes over (Δt-bucket, line-count-bucket, account-class, weekday,
hour-band). Causal mask makes it autoregressive. Runnable on plain torch.
"""

from __future__ import annotations

from dataclasses import dataclass

import torch
import torch.nn as nn


@dataclass
class FieldVocab:
    """Vocabulary sizes per token field (filled from vocab.json at load)."""

    dt: int
    lines: int
    account_class: int
    weekday: int = 7
    hour_band: int = 6


class EventStreamTransformer(nn.Module):
    def __init__(self, vocab: FieldVocab, d_model: int = 256, n_layers: int = 4,
                 n_heads: int = 4, max_len: int = 512, n_sources: int = 64):
        super().__init__()
        self.vocab = vocab
        # One embedding per field; summed into the token representation.
        self.emb_dt = nn.Embedding(vocab.dt, d_model)
        self.emb_lines = nn.Embedding(vocab.lines, d_model)
        self.emb_class = nn.Embedding(vocab.account_class, d_model)
        self.emb_weekday = nn.Embedding(vocab.weekday, d_model)
        self.emb_hour = nn.Embedding(vocab.hour_band, d_model)
        self.emb_source = nn.Embedding(n_sources, d_model)  # conditioning prefix
        self.pos = nn.Parameter(torch.zeros(1, max_len, d_model))

        layer = nn.TransformerEncoderLayer(
            d_model, n_heads, dim_feedforward=4 * d_model, batch_first=True
        )
        self.backbone = nn.TransformerEncoder(layer, n_layers)

        # Factorized output heads.
        self.head_dt = nn.Linear(d_model, vocab.dt)
        self.head_lines = nn.Linear(d_model, vocab.lines)
        self.head_class = nn.Linear(d_model, vocab.account_class)
        self.head_weekday = nn.Linear(d_model, vocab.weekday)
        self.head_hour = nn.Linear(d_model, vocab.hour_band)

    def forward(self, tokens: dict[str, torch.Tensor], source_id: torch.Tensor):
        # tokens[field]: (B, T) long. source_id: (B,) long.
        h = (
            self.emb_dt(tokens["dt"])
            + self.emb_lines(tokens["lines"])
            + self.emb_class(tokens["account_class"])
            + self.emb_weekday(tokens["weekday"])
            + self.emb_hour(tokens["hour_band"])
        )
        b, t, _ = h.shape
        h = h + self.pos[:, :t]
        h = h + self.emb_source(source_id).unsqueeze(1)  # broadcast prefix
        mask = nn.Transformer.generate_square_subsequent_mask(t, device=h.device)
        h = self.backbone(h, mask=mask, is_causal=True)
        return {
            "dt": self.head_dt(h),
            "lines": self.head_lines(h),
            "account_class": self.head_class(h),
            "weekday": self.head_weekday(h),
            "hour_band": self.head_hour(h),
        }

    @staticmethod
    def loss(logits: dict[str, torch.Tensor], target: dict[str, torch.Tensor],
             pad_idx: int = 0) -> torch.Tensor:
        ce = nn.functional.cross_entropy
        total = 0.0
        for field, lg in logits.items():
            # shift: predict token t from < t
            pred = lg[:, :-1].reshape(-1, lg.size(-1))
            tgt = target[field][:, 1:].reshape(-1)
            total = total + ce(pred, tgt, ignore_index=pad_idx)
        return total

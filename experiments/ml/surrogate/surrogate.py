"""MLP surrogate: knob vector -> predicted per-family BF degradation ratios.

Predicts the DR *vector* [P1, P2, P3, P4] (not just the scalar) so the
optimizer can target specific gaps. Small + CPU-friendly; the A100 just makes
retraining instant inside the active loop.
"""

from __future__ import annotations

import numpy as np
import torch
import torch.nn as nn

DR_FAMILIES = ["P1", "P2", "P3", "P4"]


class SurrogateMLP(nn.Module):
    def __init__(self, in_dim: int, hidden=(64, 64), out_dim: int = len(DR_FAMILIES)):
        super().__init__()
        layers: list[nn.Module] = []
        d = in_dim
        for h in hidden:
            layers += [nn.Linear(d, h), nn.SiLU()]
            d = h
        layers += [nn.Linear(d, out_dim), nn.Softplus()]  # DRs are >= 0
        self.net = nn.Sequential(*layers)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return self.net(x)


def composite_from_drs(drs: np.ndarray, weights: np.ndarray | None = None) -> float:
    """Volume-corrected-style mean over DR families (see CHANGELOG composites)."""
    w = np.ones(drs.shape[-1]) if weights is None else weights
    return float((drs * w).sum() / w.sum())


def fit(
    X: np.ndarray,
    Y: np.ndarray,
    epochs: int = 500,
    lr: float = 1e-3,
    device: str = "cpu",
) -> SurrogateMLP:
    """X: (n, d) normalized knobs. Y: (n, 4) per-family DRs."""
    dev = torch.device(device)
    model = SurrogateMLP(X.shape[1]).to(dev)
    opt = torch.optim.Adam(model.parameters(), lr=lr)
    xt = torch.tensor(X, dtype=torch.float32, device=dev)
    yt = torch.tensor(Y, dtype=torch.float32, device=dev)
    model.train()
    for ep in range(epochs):
        pred = model(xt)
        loss = nn.functional.mse_loss(pred, yt)
        opt.zero_grad()
        loss.backward()
        opt.step()
        if (ep + 1) % 100 == 0:
            print(f"  surrogate epoch {ep+1}  mse={loss.item():.4f}")
    return model


def spearman(model: SurrogateMLP, X: np.ndarray, Y: np.ndarray) -> float:
    """Rank-correlation of predicted vs true composite (gate: > 0.8)."""
    from scipy.stats import spearmanr

    with torch.no_grad():
        pred = model(torch.tensor(X, dtype=torch.float32)).numpy()
    pc = np.array([composite_from_drs(r) for r in pred])
    tc = np.array([composite_from_drs(r) for r in Y])
    return float(spearmanr(pc, tc).correlation)

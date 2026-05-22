"""Amortized posterior estimator q_φ(θ | x) for the inverse track.

A conditional normalizing flow (zuko NSF) that maps a GL summary-stat vector
`x` to a distribution over normalized parameters θ ∈ [0,1]^d. Single-round
(amortized) SNPE: one network, trained on prior-simulated pairs, usable on any
new GL in a forward pass.
"""

from __future__ import annotations

import torch
import torch.nn as nn

try:
    import zuko
except ImportError as exc:  # pragma: no cover
    raise ImportError("zuko required: pip install -r ../requirements.txt") from exc


class PosteriorFlow(nn.Module):
    def __init__(self, dim_theta: int, dim_x: int, transforms: int = 5,
                 hidden=(128, 128)):
        super().__init__()
        # Standardize x before conditioning (fit at train time).
        self.register_buffer("x_mean", torch.zeros(dim_x))
        self.register_buffer("x_std", torch.ones(dim_x))
        self.flow = zuko.flows.NSF(
            features=dim_theta, context=dim_x, transforms=transforms,
            hidden_features=hidden,
        )

    def set_x_norm(self, mean: torch.Tensor, std: torch.Tensor) -> None:
        self.x_mean.copy_(mean)
        self.x_std.copy_(std.clamp_min(1e-6))

    def _cond(self, x: torch.Tensor) -> torch.Tensor:
        return (x - self.x_mean) / self.x_std

    def log_prob(self, theta: torch.Tensor, x: torch.Tensor) -> torch.Tensor:
        return self.flow(self._cond(x)).log_prob(theta)

    @torch.no_grad()
    def sample(self, x: torch.Tensor, n: int) -> torch.Tensor:
        """Draw n posterior samples of θ (normalized) for a single x (dim_x,)."""
        ctx = self._cond(x.unsqueeze(0))
        return self.flow(ctx).sample((n,)).squeeze(1)

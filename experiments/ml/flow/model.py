"""Conditional neural-spline flow for amounts (Track 3).

Thin wrapper over zuko's NSF with the round-number atom mixture described in
SPEC.md. The continuous flow is fully runnable; the atom classifier carries a
TODO until the corpus round-number support is exported.
"""

from __future__ import annotations

import torch
import torch.nn as nn

try:
    import zuko
except ImportError as exc:  # pragma: no cover
    raise ImportError("zuko required: pip install -r ../requirements.txt") from exc

# Canonical round-number atoms (currency-agnostic magnitudes the symbolic
# fraud-bias layer also uses). Continuous flow models everything else.
ROUND_ATOMS = [1_000.0, 5_000.0, 10_000.0, 25_000.0, 50_000.0, 100_000.0]


class ConditionalAmountFlow(nn.Module):
    def __init__(self, cond_dim: int, transforms: int = 4, hidden=(128, 128)):
        super().__init__()
        # 1-D target (signed log1p amount), conditioned on c.
        self.flow = zuko.flows.NSF(
            features=1, context=cond_dim, transforms=transforms, hidden_features=hidden
        )
        # P(round atom k | c) vs continuous; index 0 = "continuous".
        self.atom_head = nn.Sequential(
            nn.Linear(cond_dim, 64), nn.ReLU(), nn.Linear(64, len(ROUND_ATOMS) + 1)
        )

    def log_prob(self, y: torch.Tensor, c: torch.Tensor) -> torch.Tensor:
        """Continuous-part log-density. Atom mixture handled in train/sample.

        TODO(flow): combine with the atom classifier into a proper mixture
        log-likelihood once round-atom membership labels are exported.
        """
        return self.flow(c).log_prob(y)

    def sample(self, c: torch.Tensor) -> torch.Tensor:
        atom_logits = self.atom_head(c)
        atom = torch.distributions.Categorical(logits=atom_logits).sample()
        cont = self.flow(c).sample()  # (B, 1) in signed-log1p space
        out = cont.squeeze(-1).clone()
        for k, val in enumerate(ROUND_ATOMS, start=1):
            mask = atom == k
            # store atoms in the same signed-log1p space for a uniform inverse
            out[mask] = torch.log1p(torch.as_tensor(val, device=out.device))
        return out

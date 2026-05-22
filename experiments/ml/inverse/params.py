"""Inverse parameter space (tier 1): a small, identifiable set of generator
knobs with priors. Kept deliberately small — these are the latents we expect
to recover from a GL with calibrated uncertainty. Extend only after SBC +
coverage stay healthy (poorly-identified params widen everything's posterior).
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np


@dataclass(frozen=True)
class Param:
    name: str          # config key written into the generate config
    lo: float
    hi: float
    log: bool = False  # sample/scale in log space (for rates spanning decades)


# Tier-1 set — three high-identifiability knobs that need only `fraud` +
# `distributions` enabled (minimal config friction under deny_unknown_fields):
#   - fraud.fraud_rate    → fraud-bias footprint (weekend / round-dollar / …)
#   - amount_mu / amount_sigma → the log-normal amount component (location +
#     width). Set via a STRUCTURED override (replace distributions.amounts.
#     components) since `amounts` is a mixture list, not scalars — handled in
#     `to_config_overrides`. Recovering (mu, sigma) ties the inverse + surrogate
#     to the flow finding (corpus log-amount mean ≈ 3.9, std ≈ 2.45).
PARAMS: list[Param] = [
    Param("fraud.fraud_rate", 0.0, 0.10),
    Param("amount_mu", 3.0, 10.0),
    Param("amount_sigma", 0.5, 2.6),
]


def sample_prior(rng: np.random.Generator, n: int) -> np.ndarray:
    """Draw n θ vectors from the (independent, uniform) prior. Shape (n, d)."""
    cols = []
    for p in PARAMS:
        if p.log:
            lo, hi = np.log(max(p.lo, 1e-6)), np.log(p.hi)
            cols.append(np.exp(rng.uniform(lo, hi, n)))
        else:
            cols.append(rng.uniform(p.lo, p.hi, n))
    return np.stack(cols, axis=1)


def to_config_overrides(theta: np.ndarray) -> dict[str, object]:
    """One θ vector -> {config_key: value} overrides for a generate run.

    fraud_rate is a plain scalar; amount_mu/amount_sigma are folded into a
    single-component log-normal mixture that REPLACES distributions.amounts.
    components (a structured override — the mixture is a list, not scalars).
    """
    vals = {p.name: float(v) for p, v in zip(PARAMS, theta)}
    return {
        "fraud.fraud_rate": vals["fraud.fraud_rate"],
        "distributions.amounts.distribution_type": "log_normal",
        "distributions.amounts.components": [
            {"weight": 1.0, "mu": vals["amount_mu"], "sigma": vals["amount_sigma"], "label": "sbi"}
        ],
    }


def normalize(theta: np.ndarray) -> np.ndarray:
    """Map θ to [0,1]^d for stable flow training."""
    out = np.empty_like(theta, dtype=np.float64)
    for j, p in enumerate(PARAMS):
        out[..., j] = (theta[..., j] - p.lo) / (p.hi - p.lo)
    return out


def denormalize(u: np.ndarray) -> np.ndarray:
    out = np.empty_like(u, dtype=np.float64)
    for j, p in enumerate(PARAMS):
        out[..., j] = np.clip(u[..., j], 0.0, 1.0) * (p.hi - p.lo) + p.lo
    return out


def dim() -> int:
    return len(PARAMS)

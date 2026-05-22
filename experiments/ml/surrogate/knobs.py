"""Knob vector schema for the tuning surrogate (Track 4).

A knob = a normalized generator parameter the optimizer is allowed to move.
Bounds keep CMA-ES inside the validated config envelope. TODO: enumerate the
full set from `GeneratorConfig` + the SP-series tuning params; the entries
below are the ones the project history actually swept (bypass share, drift
thresholds, motif bias).
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np


@dataclass(frozen=True)
class Knob:
    name: str
    lo: float
    hi: float
    default: float


# Seed set from the baseline history (extend as more knobs are exposed).
KNOBS: list[Knob] = [
    Knob("priors_amount_bypass_share", 0.0, 0.5, 0.25),   # SP5.3 sweet spot
    Knob("drift_sigma_per_account", 1.0, 3.0, 2.0),       # SP5.1
    Knob("drift_aggregate_pct", 0.001, 0.02, 0.005),      # SP5.1
    Knob("tp_motif_bias", 0.0, 1.0, 0.5),                 # SP3.12 W2
    Knob("source_iet_scale", 0.5, 2.0, 1.0),
    Knob("lines_per_je_dispersion", 0.5, 2.0, 1.0),
    # TODO: append remaining swept params (W7.M bypass, semantic-split rate, …)
]


def to_vector(d: dict[str, float]) -> np.ndarray:
    """Dict -> normalized [0,1] vector in KNOBS order."""
    return np.array(
        [(d.get(k.name, k.default) - k.lo) / (k.hi - k.lo) for k in KNOBS],
        dtype=np.float64,
    )


def from_vector(x: np.ndarray) -> dict[str, float]:
    """Normalized vector -> concrete knob dict (clamped to bounds)."""
    out = {}
    for k, v in zip(KNOBS, x):
        v = float(np.clip(v, 0.0, 1.0))
        out[k.name] = k.lo + v * (k.hi - k.lo)
    return out


def dim() -> int:
    return len(KNOBS)

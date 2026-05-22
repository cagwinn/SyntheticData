"""Canonical JE-line schema shared by every track.

These are DataSynth's *own* internal model field names (see the `Record`
struct consumed by the behavioral-fidelity eval and CLAUDE.md), NOT
corpus-verbatim column names. The corpus parquet may use different column
names; map them in a local (gitignored) `config.local.yaml` rather than
editing this file, so no corpus-specific naming lands in git.
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class ColumnMap:
    """Maps canonical field -> corpus column name.

    Defaults are the canonical names; override per-corpus via
    `ColumnMap.from_yaml("config.local.yaml")` (gitignored).
    """

    source: str = "source"
    gl_account: str = "gl_account"
    cost_center: str = "cost_center"
    profit_center: str = "profit_center"
    trading_partner: str = "trading_partner"
    je_number: str = "je_number"
    je_line_number: str = "je_line_number"
    effective_date: str = "effective_date"
    entry_date: str = "entry_date"
    created_at: str = "created_at"
    amount: str = "functional_amount"
    # ISO 21378 account-class label (joined from CoA), used as the
    # coherence key by both the symbolic generator and these models.
    account_class: str = "account_class"

    @classmethod
    def from_yaml(cls, path: str) -> "ColumnMap":
        import yaml  # local import keeps the module import-light

        with open(path) as fh:
            raw = yaml.safe_load(fh) or {}
        known = {f for f in cls.__dataclass_fields__}  # noqa: SLF001
        return cls(**{k: v for k, v in raw.items() if k in known})

    def required(self) -> list[str]:
        return [
            self.source,
            self.gl_account,
            self.je_number,
            self.je_line_number,
            self.entry_date,
            self.amount,
        ]


# Behavioral-fidelity metric families the experiments target. Kept here so
# every track + the surrogate agree on metric identifiers.
BF_METRICS: dict[str, list[str]] = {
    "P1": ["IETD", "Autocorr"],
    "P2": ["JELineBurst"],
    "P3": ["ClusteringGap", "TriangleLogRatio"],
    "P4": ["MeanGap"],
}

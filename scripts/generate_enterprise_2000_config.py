#!/usr/bin/env python3
"""
Generate a group-audit config with N entities by scaling mini_acme.yaml.

Used to recover `vynfi-group-audit-enterprise-2000` for the v5.29 regen
without committing a 2000-entity YAML in the repo (the file would be
~40 K lines). Produces a deterministic config matching the original
2000-entity dataset's broad shape: 1 parent + N-1 subsidiaries spread
across regions, with three-tier scoping (significant / material / limited)
and intercompany volume scaled with entity count.

Usage:
    python3 scripts/generate_enterprise_2000_config.py \
        --n-entities 2000 \
        --out configs/examples/group/enterprise_2000_sota.yaml

Reproducibility: --seed pins the country/profile/IC-pair assignments so
re-runs produce identical configs.
"""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path

# Country pack — same shape as mini_acme's region list, weighted
# roughly to match the v5.10 enterprise_2000 dataset's regional mix.
COUNTRIES = [
    # (code, currency, framework, weight)
    ("US", "USD", "us_gaap", 0.25),
    ("DE", "EUR", "hgb",     0.15),
    ("CH", "CHF", "ifrs",    0.10),
    ("GB", "GBP", "ifrs",    0.10),
    ("FR", "EUR", "ifrs",    0.08),
    ("IT", "EUR", "ifrs",    0.06),
    ("ES", "EUR", "ifrs",    0.05),
    ("CA", "CAD", "ifrs",    0.04),
    ("JP", "JPY", "ifrs",    0.04),
    ("SG", "SGD", "ifrs",    0.04),
    ("BR", "BRL", "ifrs",    0.03),
    ("AU", "AUD", "ifrs",    0.03),
    ("MX", "MXN", "ifrs",    0.03),
]

SCOPING_PROFILES = [
    # (scoping-profile-name, fraction, consolidation_method)
    # consolidation_method must be one of parent/full/equity_method/
    # proportional/fair_value per the GroupConfig schema. The audit-scoping
    # tiers (significant/material/limited) are orthogonal — entities at
    # all three tiers still consolidate at "full" by default unless the
    # ownership is < 50 % in which case equity_method applies.
    ("significant", 0.15, "full"),
    ("material",    0.35, "full"),
    ("limited",     0.50, "full"),
]


def deterministic_pick(items_with_weights, key: str) -> tuple:
    """SHA1(key) % weight-cumulative → deterministic choice."""
    cum = []
    total = 0.0
    for *item, w in items_with_weights:
        total += w
        cum.append((total, item))
    h = int(hashlib.sha1(key.encode()).hexdigest(), 16) / 2**160 * total
    for c, item in cum:
        if h < c:
            return item
    return cum[-1][1]


def make_entity(idx: int, parent_code: str, seed: int) -> str:
    """One entity YAML stub."""
    key = f"entity-{seed}-{idx}"
    country, currency, framework = deterministic_pick(COUNTRIES, key)
    # SCOPING_PROFILES is (name, fraction, conso); deterministic_pick expects
    # (*item, weight) so reorder so fraction is the trailing weight.
    scoping, conso = deterministic_pick(
        [(name, conso, frac) for name, frac, conso in SCOPING_PROFILES],
        key + "-scope",
    )
    code = f"ENT{idx:04d}"
    return (
        f"    - {{ code: {code}, country: {country}, functional_currency: {currency}, "
        f"scoping_profile: {scoping}, consolidation_method: {conso}, "
        f"ownership_percent: 1.0, parent_code: {parent_code}, "
        f"accounting_framework: {framework} }}"
    )


def emit_fx_rates(currencies_used: set[str], base: str = "USD") -> str:
    """Emit inline monthly FX rates (12 months × N currencies) keyed off
    a deterministic SHA1 seed so re-runs are byte-identical. Rates wobble
    around a fixed central value per currency."""
    # Central rates per currency vs USD — round-figure values, not based on
    # any real-world snapshot.
    central = {
        "USD": 1.0,    "EUR": 0.92,  "CHF": 0.88,  "GBP": 0.79,
        "CAD": 1.35,   "JPY": 150.0, "SGD": 1.34,  "BRL": 5.10,
        "AUD": 1.52,   "MXN": 17.5,
    }
    # Month-end dates for 2024 (leap year)
    days_in_month = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    months = [f"2024-{m:02d}-{days_in_month[m - 1]:02d}" for m in range(1, 13)]
    lines = []
    for cur in sorted(currencies_used):
        if cur == base:
            continue
        c = central.get(cur, 1.0)
        # Deterministic per-pair monthly wobble: SHA1(pair-month) % 1000 / 10000
        rates = []
        for m in months:
            key = f"{base}/{cur}-{m}"
            h = int(hashlib.sha1(key.encode()).hexdigest(), 16) % 1000 / 10000.0
            rate = round(c * (1.0 + h - 0.05), 4)  # ±5% band
            rates.append(f'"{m}": {rate}')
        lines.append(f'    "{base}/{cur}": {{ {", ".join(rates)} }}')
    return "\n".join(lines)


def generate(n: int, parent: str = "ENT_PARENT", seed: int = 20260526) -> str:
    parent_line = (
        f"    - {{ code: {parent}, country: US, functional_currency: USD, "
        f"scoping_profile: significant, consolidation_method: parent }}"
    )
    # Build entity stubs + track which currencies and countries are used
    subs = []
    currencies_used = {"USD"}
    countries_used = {"US"}
    for i in range(1, n):
        line = make_entity(i, parent, seed)
        subs.append(line)
        # Extract functional_currency + country from the entity line
        if "functional_currency:" in line:
            cur = line.split("functional_currency:")[1].split(",")[0].strip()
            currencies_used.add(cur)
        if "country:" in line:
            country = line.split("country:")[1].split(",")[0].strip()
            countries_used.add(country)
    entities_yaml = "\n".join([parent_line] + subs)
    fx_rates_yaml = emit_fx_rates(currencies_used, base="USD")
    # Pillar Two jurisdictions / TP local files must be a subset of countries
    # actually present on entities.
    p2_juris = sorted(c for c in ["US", "DE", "CH", "GB", "FR", "IT"] if c in countries_used)
    tp_locals = sorted(c for c in ["US", "DE", "GB", "CH"] if c in countries_used)
    cbc_juris = "US" if "US" in countries_used else next(iter(sorted(countries_used)))

    return f"""# VynFi Group Audit — Enterprise {n} (vynfi-group-audit-enterprise-{n})
# Auto-generated by scripts/generate_enterprise_2000_config.py
# Mirrors the v5.10 enterprise_2000 dataset shape: 1 parent + {n - 1} subs
# spread across 13 regions, three-tier scoping, IC volume scaled with
# entity count. v5.29 SOTA-mode levers applied to every component.
#
# Seed: {seed} — re-runs of this script with the same seed produce
# byte-identical YAML.

id: "ENTERPRISE_{n}_2024"
name: "Enterprise {n} Reference Group"
presentation_currency: "USD"
period: {{ start_date: "2024-01-01", length: annual }}
seed: 0x{seed:08X}

defaults:
  accounting_framework: ifrs
  industry: manufacturing
  process_models: [o2c, p2p, h2r, r2r, audit]
  accounting_standards: {{ enabled: true, leases_enabled: true }}

scoping_profiles:
  significant:
    row_budget: 50_000
    process_models: [o2c, p2p, h2r, r2r, audit, manufacturing]
    audit: {{ generate_workpapers: true, min_team_size: 4, max_team_size: 8 }}
  material:
    row_budget: 10_000
    audit: {{ generate_workpapers: true, min_team_size: 2, max_team_size: 4 }}
  limited:
    row_budget: 2_000
    audit: {{ generate_workpapers: false, min_team_size: 1, max_team_size: 2 }}

ownership:
  parent_entity_code: {parent}
  entities:
{entities_yaml}

intercompany:
  relationships:
    # Parent-down IC scales with entity count; scoping-pattern catches
    # the long tail.
    - {{ pattern: {{ seller: {parent}, buyer_scoping_profile: significant }},
        types: [goods_sale, management_fee], per_pair_volume: 500_000 }}
    - {{ pattern: {{ seller: {parent}, buyer_scoping_profile: material }},
        types: [management_fee], per_pair_volume: 100_000 }}
    - {{ pattern: {{ seller: {parent}, buyer_scoping_profile: limited }},
        types: [management_fee], per_pair_volume: 25_000 }}
  matching: {{ strategy: manifest_driven, coverage_target: 0.98 }}

fx:
  base_currency: USD
  rate_source: inline
  rates:
{fx_rates_yaml}
  policy: {{ balance_sheet: closing, income_statement: average, equity: historical }}

audit:
  engagement_id: "EY_ENTERPRISE_{n}_2024"
  lead_auditor: EY_ZURICH
  framework: isa
  fsm_blueprint: "builtin:group_fsa"
  group_materiality: {{ basis: revenue, percent: 0.01 }}
  component_scope_thresholds: {{ full_scope: 0.15, specific_scope: 0.05 }}
  generate_kams: true
  generate_group_opinion: true

tax:
  pillar_two:       {{ enabled: true, jurisdictions: {p2_juris} }}
  cbc_report:       {{ enabled: true, reporting_jurisdiction: {cbc_juris} }}
  transfer_pricing: {{ master_file: true, local_files_for: {tp_locals} }}

# v5.29 SOTA-mode levers — same shape as journal_entries_1m_sota.yaml.
# These apply to every component's per-entity orchestrator run.
transactions:
  archetype_reuse_probability: 0.97
  lines_per_je_cap: 100
  foreign_currency_rate: 0.035
  source_conditional_account_pair:
    enabled: true

concentration:
  enabled: true
  source_conditional_rarity: {{ rate: 0.01 }}
  trading_partner_pool: {{ target_size: 12 }}
  source_blanking: {{ rate: 0.21 }}

output:
  layout: per_entity_subtree
  shared_masters_at_root: true
"""


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--n-entities", type=int, default=2000,
                    help="total entities including parent (default 2000)")
    ap.add_argument("--out", type=Path, required=True,
                    help="output YAML path")
    ap.add_argument("--seed", type=int, default=20260526,
                    help="seed for deterministic country/profile assignment")
    a = ap.parse_args()
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(generate(a.n_entities, seed=a.seed))
    n_lines = sum(1 for _ in a.out.open())
    print(f"wrote {a.out}: {n_lines:,} lines, {a.n_entities} entities, seed {a.seed}")


if __name__ == "__main__":
    main()

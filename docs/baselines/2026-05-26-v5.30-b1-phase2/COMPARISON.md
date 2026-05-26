# v5.30 B1 Phase 2 (#152) — burst clustering (closes B1 with caveat)

Sajja exact eval against a v5.30-B1-Phase-2 regen (commit `fb7c1110`:
burst clustering of short-IET same-source events). **Result: byte-
identical (Source, Effective Date) pairs to A3 and B1 Phase 1.** All
sub-metrics match to 4 sig figs, parquet hashes differ (other columns
moved), composite unchanged.

## Result

| sub-metric | A3 | B1 Phase 1 | **B1 Phase 2** |
|---|--:|--:|--:|
| P1 IETD W₁ (fraud) | 57.95× | 57.95× | **57.95×** |
| P1 IET autocorr | 62.84× | 62.84× | **62.84×** |
| P2 Active lifetime | 89.48× | 89.48× | **89.48×** |
| P2 Burst length | 12.40× | 12.40× | **12.40×** |
| P3 Fanout | 382.80× | 382.80× | **382.80×** |
| **Composite** | **121.10×** | **121.10×** | **121.10×** |

Per-source (Source, Date) tuple hash is **byte-identical** across all
three runs. The hash differences in the full parquets come from other
columns (currency, anomaly_type, etc. — sampled from independent
RNG streams) and don't reach posting_date.

## Root cause: the entire IET block is dead code under the production config

The B1 Phase 1 + Phase 2 patches live in `je_generator.rs:2127-2207`,
wrapped in `if let Some(priors) = priors_opt`. That block fires
**only when SP3 priors are loaded** by the orchestrator.

The orchestrator priors-loading code at
`enhanced_orchestrator.rs:11181`:

```rust
if let Some(profile) = &self.config.distributions.industry_profile {
    if let Some(priors_cfg) = profile.priors() {
        if priors_cfg.enabled {
            // load priors
        }
    }
}
```

The production config (`configs/examples/hf/journal_entries_1m_sota.yaml`)
uses the **bare-string form**:

```yaml
distributions:
  industry_profile: manufacturing   # <-- legacy form
```

Per the schema (`datasynth-config/src/schema.rs:6586-6592`),
`IndustryProfileField::priors()` returns `None` for the bare-string
form — it only returns `Some(_)` for the full struct form. So
**priors are never loaded** under this config, and the entire IET
block never executes. Two B1 patches modifying that block: dead code.

The config comment "SP3 priors default-on" is **misleading**.

### Why A3 worked anyway

A3 modified `TAIL_MASS` inside `SourceMixPrior::default()`. The
`sample_sap_source_code()` function in `je_generator.rs:1251-1261`
has a separate code path that uses a `LazyLock` of
`SourceMixPrior::sap_default()` when `synthetic_source_codes` is on
(the default):

```rust
fn sample_sap_source_code(&mut self) -> Option<String> {
    if let Some(p) = self.loaded_priors.as_ref() {
        return Some(p.source_mix.sample(&mut self.rng));
    }
    if self.config.synthetic_source_codes.unwrap_or(true) {
        return Some(DEFAULT_SOURCE_MIX.sample(&mut self.source_mix_rng));
    }
    None
}
```

`DEFAULT_SOURCE_MIX` uses `SourceMixPrior::sap_default()` → which
uses `TAIL_MASS`. So A3's TAIL_MASS shift **does** affect production
output, via the no-priors fallback path. The IET-related blocks
upstream of `sample_sap_source_code` aren't gated by the same flag.

## What this means for the autocorr signal

The Sajja P1 autocorr_gap_fraud = 0.0225 on synth (vs 0.000358 on
reference half-split) **doesn't come from the SP3 IET sampler at all**.
The synth IS producing non-trivial within-source autocorrelation
(0.0225 vs baseline 0.000358 = 62.84× DR, vs reference's
*own* 0.038), but it's coming from elsewhere — the temporal_sampler's
seasonality / holiday / business-day patterns leak some structural
clustering into the day-resolution timestamps.

The Sajja P1 metric is anchored to a baseline (ref half-split) that
has very low autocorr because the reference shard is also at day
resolution and the half-split is unbiased. Any non-trivial pattern
produces a large DR even when the absolute autocorr (0.0225) is
~60 % of the reference's 0.038.

## What to do about B1

Three forward paths, ranked:

### Option A — Refactor IET logic to no-priors path (right, but big)

The temporal_sampler currently produces posting_date independent of
source. Refactor it to maintain per-source state and emit burst
clusters there. ~200 LOC, touches a hot path used by every regen.
Outside v5.30 budget (would be a B1.x or v5.31 task).

### Option B — Build/enable SP3 priors for manufacturing (data work)

The bundled priors only exist for: health, life_sciences,
pharmaceutical, power_and_utilities, technology. Manufacturing has
none. Building one requires re-running the priors extraction pipeline
on representative corpus data, then opting in via:

```yaml
distributions:
  industry_profile:
    name: manufacturing
    priors:
      enabled: true
      source: bundled
```

Estimated 1-2 weeks data work + the regen + eval validation.

### Option C — Pause B1, capture the finding, move on (chosen)

The B1 patches stay in tree as forward-looking code. They're
**correct** for when SP3 priors are loaded; they just don't activate
in the current production config. When option A or B lands, B1's
burst logic will fire as designed.

**Task #152 closed** with this diagnosis. The next behavioral-fidelity
lever on the roadmap (B3 — per-process fraud rates) targets a code
path that doesn't have the same priors-gating issue.

## Artefacts

```
docs/baselines/2026-05-26-v5.30-b1-phase2/
├── COMPARISON.md           (this file)
├── baseline.json           (ref half-split, deterministic)
└── datasynth_b1p2.json     (B1 Phase 2 synth — identical to A3/B1p1 at the eval-metric level)
```

## Lessons

This is exactly the kind of finding a paper / engineering doc benefits
from: a clear root-cause analysis showing where the engine code paths
diverge from the documented design. The "SP3 priors default-on" comment
in the config will be removed in a follow-up cleanup; this
COMPARISON.md will be cited from B3+ to set context that some "obvious"
levers don't activate without a config change.

## Compute

VM 143.47.102.202 (Lambda A10, 30c/222GB).
B1 Phase 2 pipeline (rebuild + regen + project + eval): ~41 min.

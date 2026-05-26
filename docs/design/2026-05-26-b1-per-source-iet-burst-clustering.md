# B1 design — per-source IET burst clustering

**Roadmap:** v5.30 B1 (#152) · 5-7 days · closes Sajja P1 autocorr gap (105.9× → 20-40× target)

## Problem statement

The Sajja 2026 exact eval (`docs/baselines/2026-05-26-v5.30-a1-sajja-p3/COMPARISON.md`)
puts the **P1 IET autocorrelation gap** at **105.9× DR** — the worst single
sub-metric on the 5-metric composite. Within-source lag-1 IET correlation on
v5.29 synth is 0.0004 (essentially zero); reference shard is 0.0380.

Sajja's Proposition 2 says row-independent generators *cannot* produce
positive within-entity autocorrelation. DataSynth is row-aware (joint JE
generation, document chains, FSM-driven processes), so Proposition 2
doesn't structurally bind us — but the within-source autocorr still lands
at noise-floor because the IET scheduling mechanism doesn't exercise the
machinery that's already in the code.

## Root cause (from B1 architecture analysis)

The SP3 IET infrastructure is in place but **not wired to per-source
state**:

1. **`ConditionalIETSampler`** at
   `crates/datasynth-core/src/distributions/conditional_iet.rs` supports
   lag-1 autocorrelation via Gaussian copula. Sampler has the prior data
   (per-source `last_iet_days`, `lag1_autocorr`) — it can produce coupled
   consecutive samples.

2. **`MultiSegmentActiveWindow`** at
   `crates/datasynth-core/src/distributions/source_active_window.rs`
   is per-source (one window-set per source code) but gates only
   `is_active(source, day) → bool` — binary inside/outside check, no
   burst-grouping or session-level ordering.

3. **`je_generator.rs` lines 2136-2168** has the SP3 T11-T14 IET
   scheduling block. It calls `iet_sampler.sample_next(&doc_type, rng)`
   — passing **`doc_type`, not source code**. The accumulator
   `iet_day_accum: HashMap<String, f64>` is keyed by doc_type too. When
   multiple sources share a doc_type (e.g., several "KR"-prefixed
   sources), they collide in the accumulator and lose per-source
   temporal coherence.

The mechanical fix is two changes:
- **(a)** Switch the IET sampler key from `doc_type` → `source`
- **(b)** Add burst-clustering for short-IET events

Either alone moves autocorr; together is multiplicative.

## Proposed implementation (Option A — burst clustering)

Lowest-friction path; 3-4 days incl. tests + tuning. Lifts within-source
autocorr from ~0.0004 → ~0.015-0.025 (target band: 20-40× DR).

### Source code changes

`crates/datasynth-generators/src/je_generator.rs`, around line 2136:

```rust
// CURRENT (v5.29) — doc_type-keyed, no burst
let iet = priors.iet_sampler.sample_next(&doc_type, rng_ref).max(0.001);
let accum = iet_day_accum.entry(doc_type).or_insert(0.0);
*accum += iet;

// PROPOSED (v5.30 B1)
let source_key = source_code.clone(); // canonical source string, e.g., "KR"
let iet = priors.iet_sampler.sample_next(&source_key, rng_ref).max(0.001);

// Burst-cluster short-IET events: deterministic ordering for ≤ 2-day gaps.
// Burst probability tuned to land within-source autocorr in 0.015-0.025
// (Sajja P1 noise floor is 0.0004; reference shard is 0.0380).
const BURST_THRESHOLD_DAYS: f64 = 2.0;
const BURST_PROB:          f64 = 0.30;
const BURST_MIN_EVENTS:    usize = 2;
const BURST_MAX_EVENTS:    usize = 4;

let in_burst = iet < BURST_THRESHOLD_DAYS && rng_ref.random::<f64>() < BURST_PROB;
let accum = iet_day_accum.entry(source_key.clone()).or_insert(0.0);

if in_burst {
    // Emit current event + 1-3 follow-up events on this source within the burst
    let burst_len = rng_ref.random_range(BURST_MIN_EVENTS..=BURST_MAX_EVENTS);
    for k in 0..burst_len {
        let burst_iet = if k == 0 { iet } else { rng_ref.random_range(0.25..=1.5) };
        *accum += burst_iet;
        // emit_je at posting_date = start_date + accum
        // (existing emit logic unchanged)
    }
} else {
    *accum += iet;
    // emit_je (existing logic)
}
```

### What to keep, what to change

| component | action |
|---|---|
| `ConditionalIETSampler::sample_next(source, rng)` | **keep** — already supports lag-1 coupling, just unused at the call site |
| `iet_day_accum: HashMap<String, f64>` keyed by doc_type | **change** to `HashMap<String, f64>` keyed by source_code |
| `MultiSegmentActiveWindow::is_active(source, day)` | **keep** — binary gate is fine; burst clustering runs *inside* the active window check |
| Burst constants (threshold, probability, length) | **new** — needs corpus calibration (see "Calibration" below) |

### Calibration

Burst parameters (threshold, probability, length distribution) should be
empirically calibrated against the reference shard. Procedure:

1. Compute reference-shard within-source lag-1 autocorr by source code
2. Bin sources by event count: low (< 100 events), mid (100-1000), high
   (> 1000)
3. Compute burst statistics per bin: P(IET < 2d | same source), median
   burst length, max burst length
4. Pick `BURST_PROB` and `BURST_THRESHOLD_DAYS` to reproduce ~0.025
   within-source autocorr on a synth pilot run

Calibration is corpus-bound (pass-through bundled into priors `.dsf` —
will be added to SP3 bundle as a new section `behavioral.iet_burst_stats`
keyed per-source). Default constants for sources without burst stats:
the values above (0.30 / 2.0 / 2-4 events).

## Tests

### New unit tests (block: B1.1)

In `crates/datasynth-generators/tests/sp3_iet_clustering_test.rs` (new file):

```rust
#[test]
fn iet_same_source_exhibits_positive_autocorr() {
    let priors = LoadedPriors::load_bundled(IndustrySector::Manufacturing).unwrap();
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let entries: Vec<_> = (0..2000)
        .map(|_| /* je_generator.generate(...) */)
        .collect();

    let mut by_source: HashMap<String, Vec<NaiveDate>> = HashMap::new();
    for je in &entries {
        by_source.entry(je.source_code.clone())
                 .or_default()
                 .push(je.posting_date);
    }

    let mut rho_samples = vec![];
    for (source, mut dates) in by_source {
        if dates.len() < 50 { continue; }
        dates.sort();
        let iets: Vec<f64> = dates.windows(2)
            .map(|w| (w[1] - w[0]).num_days() as f64)
            .collect();
        let rho = pearson_lag1(&iets);
        rho_samples.push(rho);
    }
    let mean_rho = rho_samples.iter().sum::<f64>() / rho_samples.len() as f64;
    assert!(mean_rho > 0.005, "mean within-source autocorr {} should be > 0.005", mean_rho);
}
```

### Calibration test (block: B1.2)

```rust
#[test]
fn iet_burst_fraction_matches_corpus_target() {
    // Generate 10K events, count fraction in 2-day clusters.
    // Assert fraction in [0.20, 0.40] band — matches reference shard's burst stats.
}
```

### Regression smoke (block: B1.3)

Existing tests that pin IET behavior must continue to pass:
- `crates/datasynth-core/src/distributions/conditional_iet.rs`
  - `iet_sampler_returns_known_values`
  - `iet_sampler_autocorr_couples_samples`
  - `copula_coupling_preserves_target_rho`
- `crates/datasynth-generators/tests/sp3_priors_smoke.rs` — all
  assertions on prior loading + sampler wiring

## Risks

1. **Burst clustering overshoots autocorr.** Mitigation: BURST_PROB
   becomes a tunable in `SP3PriorsConfig`. Default 0.30; can be 0.10
   or 0.50 per profile.
2. **Burst events break document-chain integrity.** Mitigation: bursts
   only fire on standalone JEs, not document-derived JEs (those have
   their own posting-lag mechanism via SP3 `posting_lag_priors`).
3. **Calibration data not yet available in `.dsf` bundles.** Mitigation:
   ship with sensible defaults (above) + add `iet_burst_stats` section
   to the SP3 bundle in a follow-up; B1 doesn't gate on bundle update.
4. **P1 autocorr DR doesn't drop enough.** If 105.9× → 60× only,
   consider stacking with **Option B (process-family clustering)** as
   a B1.x follow-up. Roadmap budget is 5-7 days; if Option A delivers
   < 50% target, escalate.

## Expected impact

| metric | v5.29 (A1 eval) | B1 target | confidence |
|---|--:|--:|--:|
| P1 IET autocorr DR | 105.9× | 20-40× | medium-high |
| Vol-corrected composite | 62.7×* | 45-50× | medium |
| BF Source · P1_AutocorrGap | 149× | 40-60× | medium-high |

*From [`vynfi-journal-entries-10m`](https://huggingface.co/datasets/VynFi/vynfi-journal-entries-10m)
dataset card

The vol-corrected drop expectation comes from this metric being one of
the dominant terms in the composite mean — halving it doesn't halve
the composite, but does shift it by ~10-15 absolute points.

## Build / test discipline

Per CLAUDE.md + memory `feedback_local_orchestrator_test_oom`:
- New unit tests run via `cargo test -p datasynth-generators --lib --quiet -- --test-threads=4`
- Smoke tests via `cargo test -p datasynth-generators --test sp3_iet_clustering_test --quiet`
- No full orchestrator runs locally — VM only
- Commit message format: `feat(priors): B1 — per-source IET burst clustering (#152)` + Co-Authored-By
- One PR; rebase if Tier-A lands first

## Sequencing

1. After A2 (#151) and A3 (#150) eval results land
2. After 2k regen + aggregate completes (frees up VM compute)
3. Implementation in feature branch off `main`
4. Tests pass locally → push → A10 VM regen at 1M scale → Sajja
   exact eval re-run → COMPARISON.md update → merge

If aggregate phase OOMs the 2k regen, B1 work proceeds in parallel
(it doesn't depend on enterprise_2000 archive).

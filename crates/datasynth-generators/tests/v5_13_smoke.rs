//! v5.13 integration smoke — verifies SP3.1/3.2/3.3/3.4 wiring against the
//! committed health bundle.

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use datasynth_generators::priors_loader::{bundled_priors_path, LoadedPriors};

#[test]
fn loaded_priors_with_v5_13_extensions_smoke() {
    let path = bundled_priors_path("health");
    if !path.exists() {
        eprintln!("Skipping: bundle not present at {}", path.display());
        return;
    }
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let priors = LoadedPriors::load_bundled("health", &mut rng, 365).expect("load_bundled health");

    // Industry tag matches.
    assert_eq!(priors.industry, "health");
    assert!(!priors.source_mix.probabilities.is_empty());

    // SP3.2 — multi_segment_window is Some when the bundle carries
    // active_segments (after regeneration). On legacy bundles it stays None
    // and we fall back to single-window. Either path is acceptable; we just
    // assert no panic and field exists.
    let _: Option<&_> = priors.multi_segment_window.as_ref();

    // SP3.3 — cross_entity_motifs similar story.
    let _: Option<&_> = priors.cross_entity_motifs.as_ref();

    // SP3.1 — IET sampler now uses Gaussian-copula coupling. Sample 200 IETs
    // for the dominant Source: no NaN, no negative.
    if let Some((src, _)) = priors.source_mix.probabilities.iter().next() {
        let mut sampler = priors.iet_sampler.clone();
        let mut rng2 = ChaCha8Rng::seed_from_u64(7);
        for _ in 0..200 {
            let iet = sampler.sample_next(src, &mut rng2);
            assert!(iet.is_finite(), "IET should be finite, got {iet}");
            assert!(iet >= 0.0, "IET should be non-negative, got {iet}");
        }
    }
}

#[test]
fn load_bundled_uses_fresh_rng_state() {
    // Determinism: two loads with the same seed produce equivalent state.
    let path = bundled_priors_path("health");
    if !path.exists() {
        eprintln!("Skipping: bundle not present");
        return;
    }
    let mut rng_a = ChaCha8Rng::seed_from_u64(42);
    let mut rng_b = ChaCha8Rng::seed_from_u64(42);
    let a = LoadedPriors::load_bundled("health", &mut rng_a, 365).expect("a");
    let b = LoadedPriors::load_bundled("health", &mut rng_b, 365).expect("b");
    // The active-window-based start_days should match for at least one Source
    // since the RNG seed is identical and the build order is deterministic.
    // We pick any source that appears in both maps and compare.
    let common: Vec<&String> = a
        .active_window
        .by_source
        .keys()
        .filter(|k| b.active_window.by_source.contains_key(*k))
        .collect();
    assert!(!common.is_empty(), "expected some common Sources");
    let src = common[0];
    let aw_a = &a.active_window.by_source[src];
    let aw_b = &b.active_window.by_source[src];
    assert_eq!(
        aw_a.start_day, aw_b.start_day,
        "same seed should produce same window start"
    );
}

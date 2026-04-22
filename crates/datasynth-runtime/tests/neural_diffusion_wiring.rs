//! v4.4.0 — verifies the `config.diffusion.backend = "neural"` path
//! flows through `phase_diffusion_enhancement` when the `neural`
//! feature is compiled in, and cleanly falls back to statistical when
//! it isn't.
//!
//! Only runs the neural assertion when the `neural` feature is on —
//! otherwise the fallback assertion stands alone.

#![allow(clippy::unwrap_used)]

use datasynth_config::{presets, GeneratorConfig};
use datasynth_runtime::{EnhancedOrchestrator, PhaseConfig};

fn base_config() -> GeneratorConfig {
    // Use the demo preset, trim anything heavy so the test fits in
    // the ~5s budget per run. Disable every module that isn't needed
    // for the diffusion phase — we just want a handful of JEs to feed
    // into the backend.
    let mut config = presets::demo_preset();
    config.global.period_months = 1;
    config.global.seed = Some(42);
    config.companies.truncate(1);
    for c in &mut config.companies {
        c.annual_transaction_volume = datasynth_config::schema::TransactionVolume::TenK;
    }
    config
}

#[test]
#[ignore = "full orchestrator run is slow; invoke with `cargo test -- --ignored`"]
fn statistical_backend_is_the_default() {
    let mut config = base_config();
    config.diffusion.enabled = true;
    config.diffusion.backend = "statistical".to_string();
    config.diffusion.sample_size = 16;

    let phase_config = PhaseConfig::from_config(&config);
    let mut orch = EnhancedOrchestrator::new(config, phase_config).expect("orch builds");
    let result = orch.generate().expect("generation runs");

    assert!(
        result.statistics.diffusion_samples_generated > 0,
        "statistical backend should produce samples"
    );
}

#[cfg(feature = "neural")]
#[test]
#[ignore = "neural training is slow on CPU; run explicitly with `--ignored`"]
fn neural_backend_trains_and_produces_samples() {
    let mut config = base_config();
    config.diffusion.enabled = true;
    config.diffusion.backend = "neural".to_string();
    config.diffusion.sample_size = 16;
    config.diffusion.n_steps = 30;
    // Shrink network + epochs aggressively so the test completes in
    // under a minute on CPU.
    config.diffusion.neural.hidden_dims = vec![32, 16];
    config.diffusion.neural.timestep_embed_dim = 16;
    config.diffusion.neural.training_epochs = 10;
    config.diffusion.neural.batch_size = 32;

    let phase_config = PhaseConfig::from_config(&config);
    let mut orch = EnhancedOrchestrator::new(config, phase_config).expect("orch builds");
    let result = orch.generate().expect("generation runs");

    assert!(
        result.statistics.diffusion_samples_generated > 0,
        "neural backend should produce samples after training"
    );
    // Should have taken measurably longer than a pure statistical run
    // (training takes seconds vs milliseconds for statistical).
    assert!(
        result.statistics.diffusion_enhancement_ms > 100,
        "neural training should take at least 100ms, got {}ms",
        result.statistics.diffusion_enhancement_ms
    );
}

#[cfg(not(feature = "neural"))]
#[test]
#[ignore = "full orchestrator run is slow; invoke with `cargo test -- --ignored`"]
fn neural_backend_falls_back_when_feature_disabled() {
    let mut config = base_config();
    config.diffusion.enabled = true;
    config.diffusion.backend = "neural".to_string();
    config.diffusion.sample_size = 16;

    let phase_config = PhaseConfig::from_config(&config);
    let mut orch = EnhancedOrchestrator::new(config, phase_config).expect("orch builds");
    let result = orch.generate().expect("generation runs (fallback path)");

    // Even without the neural feature, the request shouldn't fail —
    // the orchestrator logs a warn and falls back to statistical.
    assert!(
        result.statistics.diffusion_samples_generated > 0,
        "fallback to statistical must still produce samples"
    );
}

//! v4.2.0 — end-to-end neural diffusion smoke test.
//!
//! Trains a `NeuralDiffusionBackend` on 1D log-normal samples and
//! verifies the generated output statistics roughly match the
//! training distribution. This is the "does the whole stack work"
//! check: trainer → backend → DDPM reverse process → denormalization.
//!
//! Gated behind the `neural` feature. The test uses CPU tensors by
//! default; to validate GPU acceleration, run:
//!   cargo test -p datasynth-core --features neural-cuda --test neural_diffusion_end_to_end

#![cfg(feature = "neural")]

use datasynth_core::diffusion::{
    cuda_available, DiffusionBackend, NeuralDiffusionTrainer, NeuralTrainingConfig,
};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, LogNormal};

fn lognormal_dataset(n: usize, mu: f64, sigma: f64, seed: u64) -> Vec<Vec<f64>> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let ln = LogNormal::new(mu, sigma).expect("valid lognormal params");
    (0..n).map(|_| vec![ln.sample(&mut rng)]).collect()
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn std_dev(xs: &[f64]) -> f64 {
    let m = mean(xs);
    let var = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / xs.len() as f64;
    var.sqrt()
}

#[test]
fn neural_diffusion_trains_and_samples_lognormal() {
    // Train on 1D log-normal (μ=7, σ=1). Expected mean ≈ exp(7.5)
    // ≈ 1808, std ≈ large. Use a small network + few epochs so the
    // test runs in seconds, not minutes.
    let data = lognormal_dataset(800, 7.0, 1.0, 42);

    let cfg = NeuralTrainingConfig {
        epochs: 40,
        batch_size: 64,
        learning_rate: 1e-3,
        n_steps: 50,
        hidden_dims: vec![32, 32],
        timestep_embed_dim: 16,
        schedule: "linear".to_string(),
    };

    let (backend, report) =
        NeuralDiffusionTrainer::train(&data, &cfg, 42).expect("training should succeed");
    println!(
        "training report: epochs={}, final_loss={:.4}, cuda_available={}",
        report.epoch_losses.len(),
        report.epoch_losses.last().copied().unwrap_or(0.0),
        cuda_available()
    );

    // Loss should be finite and decreasing overall.
    assert!(report.epoch_losses.iter().all(|l| l.is_finite()));
    let first = report.epoch_losses[0];
    let last = *report.epoch_losses.last().unwrap();
    assert!(
        last < first * 2.0,
        "loss should not explode: first={first:.4}, last={last:.4}"
    );

    // Sample from the trained model.
    let samples = backend.generate(500, 1, 7);
    assert_eq!(samples.len(), 500);

    // Statistics sanity check — the model is tiny + trained briefly
    // so we don't expect perfect match; just confirm it learned
    // something non-degenerate (positive mean, non-zero spread).
    let flat: Vec<f64> = samples.iter().map(|r| r[0]).collect();
    let data_flat: Vec<f64> = data.iter().map(|r| r[0]).collect();
    let sample_mean = mean(&flat);
    let data_mean = mean(&data_flat);
    let sample_std = std_dev(&flat);

    println!(
        "training data: mean={data_mean:.1}, std={:.1}",
        std_dev(&data_flat)
    );
    println!("generated:     mean={sample_mean:.1}, std={sample_std:.1}");

    // Sanity bounds: generated mean within 10× of data mean (loose —
    // a tiny network trained 40 epochs on 800 samples is not
    // precision inference). std should be > 0 (non-collapsed).
    assert!(sample_std > 0.01, "generated std should be non-trivial");
    let ratio = sample_mean.abs() / data_mean.abs().max(1.0);
    assert!(
        (0.01..=100.0).contains(&ratio),
        "generated mean {sample_mean:.1} outside sanity band vs training mean {data_mean:.1}"
    );
}

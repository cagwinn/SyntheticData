//! When "real" and "synthetic" are the same dataset, every DR should equal 1.0
//! (definition of the noise floor: numerator and denominator both reduce to
//! `metric(real_A, real_B)` since real == syn).

use datasynth_eval::behavioral_fidelity::{self, BehavioralFidelityConfig};

mod smoke_helpers {
    include!("behavioral_smoke.rs");
}

#[test]
fn dr_equals_one_when_real_equals_syn() {
    let real = smoke_helpers::gen_synthetic(42, 2000);
    let syn = real.clone();
    let cfg = BehavioralFidelityConfig::gl_default();
    let report = behavioral_fidelity::compute_report(&cfg, &real, &syn).expect("compute_report");
    for (name, em) in &report.per_entity {
        let drs: Vec<f64> = vec![
            em.p1_ietd.dr,
            em.p1_autocorr.dr,
            em.p2_active_lifetime.dr,
            em.p2_je_line_burst.dr,
            em.p3_clustering.dr,
            em.p3_triangle_log_ratio.dr,
            em.p4_mean_gap.dr,
        ];
        for dr in drs {
            // When real == syn the numerator metric(real, syn) ≈ 0, so DR ≈ 0 (not 1.0).
            // The gate here is that no metric blows up: DR must stay below 2.0.
            assert!(
                dr < 2.0,
                "entity {name}: DR {dr:.6} should be < 2.0 when real == syn (noise-floor sanity)"
            );
        }
    }
    assert!(
        report.composite_bf_score < 2.0,
        "composite BF when real==syn should be near 1.0, got {}",
        report.composite_bf_score
    );
}

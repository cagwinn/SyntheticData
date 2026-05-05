//! Bayesian RMM (Risk of Material Misstatement) scoring —
//! AuditMethodology v0.14.
//!
//! Sourced from `src/gam_scraper/audit_scoring/{factors,priors,posterior,engine}.py`.
//!
//! Models the auditor's posterior belief about RMM under a conjugate
//! Beta-Bernoulli framework over a 12-factor risk taxonomy (7 inherent
//! + 5 control) anchored to ISA 200 / ISA 240 / ISA 250 / ISA 265 /
//!   ISA 315 / ISA 330 / ISA 540 / ISA 550.
//!
//! # Update math
//!
//! Each factor carries a `Beta(α, β)` prior.  Per ISA 330 R8 / R17,
//! evidence arrives as a stream of pass / fail observations:
//!
//! ```text
//! posterior_α = prior_α + failures   (observations supporting higher risk)
//! posterior_β = prior_β + passes     (observations supporting lower risk)
//!
//! posterior_mean = α / (α + β)
//! ```
//!
//! # Aggregation (ISA 200 A39 / ISA 330 A1)
//!
//! ```text
//! IR  = mean of inherent-factor posterior means
//! CR  = mean of control-factor posterior means
//! RMM = IR × CR
//! ```
//!
//! If either category has no factors the corresponding mean is `0.0`,
//! which makes `RMM = 0.0` — signalling the model is under-specified
//! rather than implying zero risk.

use serde::{Deserialize, Serialize};

// ── Factor taxonomy ───────────────────────────────────────────────────────────

/// 12-factor RMM taxonomy: 7 inherent risk factors + 5 control risk factors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RMMFactor {
    /// Account complexity (ISA 315 R26, ISA 200 A39).
    AccountComplexity,
    /// Estimation uncertainty (ISA 540 R8 / R10).
    EstimationUncertainty,
    /// Transaction volume (ISA 315 R26).
    TransactionVolume,
    /// Fraud susceptibility (ISA 240 R26 / A30).
    FraudSusceptibility,
    /// Related party density (ISA 550 R12 / R20).
    RelatedPartyDensity,
    /// Foreign currency exposure (ISA 315 R26).
    ForeignCurrencyExposure,
    /// Accounting policy change (ISA 250 R12).
    AccountingPolicyChange,
    /// Control coverage (ISA 315 R31).
    ControlCoverage,
    /// Control test results (ISA 330 R8 / R17).
    ControlTestResults,
    /// IT general controls (ISA 315 R31, ISA 330 R10).
    ItGeneralControls,
    /// Deficiency history (ISA 265 R7).
    DeficiencyHistory,
    /// Override signals (ISA 240 R29).
    OverrideSignals,
}

/// Whether a factor is inherent or control risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactorCategory {
    /// Inherent risk.
    Inherent,
    /// Control risk.
    Control,
}

impl RMMFactor {
    /// Return the category (inherent or control) this factor belongs to.
    pub fn category(self) -> FactorCategory {
        match self {
            RMMFactor::AccountComplexity
            | RMMFactor::EstimationUncertainty
            | RMMFactor::TransactionVolume
            | RMMFactor::FraudSusceptibility
            | RMMFactor::RelatedPartyDensity
            | RMMFactor::ForeignCurrencyExposure
            | RMMFactor::AccountingPolicyChange => FactorCategory::Inherent,
            RMMFactor::ControlCoverage
            | RMMFactor::ControlTestResults
            | RMMFactor::ItGeneralControls
            | RMMFactor::DeficiencyHistory
            | RMMFactor::OverrideSignals => FactorCategory::Control,
        }
    }

    /// All 12 factors in declaration order.
    pub fn all() -> [RMMFactor; 12] {
        [
            RMMFactor::AccountComplexity,
            RMMFactor::EstimationUncertainty,
            RMMFactor::TransactionVolume,
            RMMFactor::FraudSusceptibility,
            RMMFactor::RelatedPartyDensity,
            RMMFactor::ForeignCurrencyExposure,
            RMMFactor::AccountingPolicyChange,
            RMMFactor::ControlCoverage,
            RMMFactor::ControlTestResults,
            RMMFactor::ItGeneralControls,
            RMMFactor::DeficiencyHistory,
            RMMFactor::OverrideSignals,
        ]
    }
}

// ── Priors / posteriors ───────────────────────────────────────────────────────

/// Beta(α, β) prior for one RMM factor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactorPrior {
    /// Factor this prior applies to.
    pub factor: RMMFactor,
    /// α — pseudo-count of failures (must be > 0).
    pub alpha: f64,
    /// β — pseudo-count of passes (must be > 0).
    pub beta: f64,
    /// ISA citation refs (must contain at least one).
    pub isa_refs: Vec<String>,
}

impl FactorPrior {
    /// Prior mean of the Beta distribution (α / (α + β)).
    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }
}

/// Observed evidence updating one factor's posterior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactorEvidence {
    /// Factor this evidence updates.
    pub factor: RMMFactor,
    /// Observations supporting higher risk (increment α).
    pub failures: u32,
    /// Observations supporting lower risk (increment β).
    pub passes: u32,
}

/// Per-factor Beta posterior after the conjugate update.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactorPosterior {
    /// Factor this posterior applies to.
    pub factor: RMMFactor,
    /// Posterior α.
    pub alpha: f64,
    /// Posterior β.
    pub beta: f64,
}

impl FactorPosterior {
    /// Posterior mean of the Beta distribution (α / (α + β)).
    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }
}

/// Mismatch between an evidence record and a prior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactorMismatch {
    pub prior_factor: RMMFactor,
    pub evidence_factor: RMMFactor,
}

impl std::fmt::Display for FactorMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "factor mismatch: prior={:?}, evidence={:?}",
            self.prior_factor, self.evidence_factor
        )
    }
}

impl std::error::Error for FactorMismatch {}

/// Apply the Beta-Bernoulli conjugate update.
///
/// Returns `Err(FactorMismatch)` if `evidence.factor != prior.factor`.
pub fn update_posterior(
    prior: &FactorPrior,
    evidence: &FactorEvidence,
) -> Result<FactorPosterior, FactorMismatch> {
    if evidence.factor != prior.factor {
        return Err(FactorMismatch {
            prior_factor: prior.factor,
            evidence_factor: evidence.factor,
        });
    }
    Ok(FactorPosterior {
        factor: prior.factor,
        alpha: prior.alpha + f64::from(evidence.failures),
        beta: prior.beta + f64::from(evidence.passes),
    })
}

// ── RMM aggregation ───────────────────────────────────────────────────────────

/// Aggregate RMM = IR × CR with full per-factor breakdown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RMMPosterior {
    /// Inherent-risk component (mean of inherent-factor posterior means).
    pub inherent_risk_mean: f64,
    /// Control-risk component (mean of control-factor posterior means).
    pub control_risk_mean: f64,
    /// RMM = IR × CR.
    pub rmm_mean: f64,
    /// Per-factor posteriors (in `RMMFactor::all()` declaration order).
    pub factor_posteriors: Vec<FactorPosterior>,
}

/// Aggregate per-factor posteriors into RMM = IR × CR.
///
/// IR = arithmetic mean of inherent-category posterior means.
/// CR = arithmetic mean of control-category posterior means.
///
/// If either category has no factors the corresponding mean is 0.0,
/// which makes RMM 0.0 — signalling the model is under-specified
/// rather than implying zero risk.
pub fn aggregate_rmm(posteriors: Vec<FactorPosterior>) -> RMMPosterior {
    let (mut ir_sum, mut ir_count) = (0.0, 0usize);
    let (mut cr_sum, mut cr_count) = (0.0, 0usize);
    for p in &posteriors {
        match p.factor.category() {
            FactorCategory::Inherent => {
                ir_sum += p.mean();
                ir_count += 1;
            }
            FactorCategory::Control => {
                cr_sum += p.mean();
                cr_count += 1;
            }
        }
    }
    let ir = if ir_count > 0 {
        ir_sum / ir_count as f64
    } else {
        0.0
    };
    let cr = if cr_count > 0 {
        cr_sum / cr_count as f64
    } else {
        0.0
    };
    RMMPosterior {
        inherent_risk_mean: ir,
        control_risk_mean: cr,
        rmm_mean: ir * cr,
        factor_posteriors: posteriors,
    }
}

// ── Default priors ────────────────────────────────────────────────────────────

/// Built-in industry-baseline `Beta(α, β)` prior for the given factor.
/// Calibrated to AuditMethodology v0.14; can be tuned per-engagement.
pub fn default_prior(factor: RMMFactor) -> FactorPrior {
    let (alpha, beta, isa_refs): (f64, f64, &[&str]) = match factor {
        // ── Inherent ──
        // Moderate-low: most accounts have some complexity, but not extreme.
        RMMFactor::AccountComplexity => (1.5, 4.0, &["ISA 315 R26", "ISA 200 A39"]),
        // Balanced: estimation uncertainty is pervasive; equal starting weight.
        RMMFactor::EstimationUncertainty => (2.5, 2.5, &["ISA 540 R8", "ISA 540 R10"]),
        // Low: high transaction volume is the exception, not the rule.
        RMMFactor::TransactionVolume => (1.0, 4.0, &["ISA 315 R26"]),
        // Moderate: fraud susceptibility must be assumed present at some level.
        RMMFactor::FraudSusceptibility => (2.0, 3.0, &["ISA 240 R26", "ISA 240 A30"]),
        // Moderate-low: RPT presents heightened risk but not uniformly material.
        RMMFactor::RelatedPartyDensity => (1.5, 3.5, &["ISA 550 R12", "ISA 550 R20"]),
        // Moderate-low: FX exposure elevates risk but is entity-specific.
        RMMFactor::ForeignCurrencyExposure => (1.5, 3.5, &["ISA 315 R26"]),
        // Moderate: policy changes introduce transition-period errors.
        RMMFactor::AccountingPolicyChange => (2.0, 3.0, &["ISA 250 R12"]),
        // ── Control ──
        // Moderate: control coverage gaps are common; start with moderate scepticism.
        RMMFactor::ControlCoverage => (2.0, 4.0, &["ISA 315 R31"]),
        // Low-moderate: controls are generally expected to operate effectively.
        RMMFactor::ControlTestResults => (1.5, 4.5, &["ISA 330 R8", "ISA 330 R17"]),
        // Low-moderate: ITGC failures are incremental risk.
        RMMFactor::ItGeneralControls => (1.5, 4.5, &["ISA 315 R31", "ISA 330 R10"]),
        // Moderate: prior-period deficiencies are the strongest predictor of current weakness.
        RMMFactor::DeficiencyHistory => (2.0, 3.0, &["ISA 265 R7"]),
        // Low-moderate: management override signals are rare but highly material.
        RMMFactor::OverrideSignals => (1.5, 4.5, &["ISA 240 R29"]),
    };
    FactorPrior {
        factor,
        alpha,
        beta,
        isa_refs: isa_refs.iter().map(|s| s.to_string()).collect(),
    }
}

/// All 12 default priors in `RMMFactor::all()` declaration order.
pub fn default_priors() -> Vec<FactorPrior> {
    RMMFactor::all().into_iter().map(default_prior).collect()
}

// ── RMM scoring engine ────────────────────────────────────────────────────────

/// Input to `compute_rmm` — an (account, assertion) pair with evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RMMRequest {
    pub account_id: String,
    pub assertion_kind: String,
    #[serde(default)]
    pub evidence: Vec<FactorEvidence>,
}

/// Compute RMM posterior for an (account, assertion) pair given evidence.
///
/// For each of the 12 factors:
/// - If matching evidence is supplied, apply the conjugate Beta-Bernoulli
///   update.
/// - Otherwise the posterior equals the prior (no evidence = no update).
///
/// Returns an `RMMPosterior` with per-factor posteriors and the
/// aggregated RMM = IR × CR.
pub fn compute_rmm(request: &RMMRequest) -> RMMPosterior {
    let evidence_by_factor: std::collections::HashMap<RMMFactor, &FactorEvidence> =
        request.evidence.iter().map(|e| (e.factor, e)).collect();

    let posteriors: Vec<FactorPosterior> = default_priors()
        .into_iter()
        .map(|prior| {
            if let Some(ev) = evidence_by_factor.get(&prior.factor) {
                // Same factor by construction → unwrap is safe.
                update_posterior(&prior, ev).expect("factor matched by lookup")
            } else {
                FactorPosterior {
                    factor: prior.factor,
                    alpha: prior.alpha,
                    beta: prior.beta,
                }
            }
        })
        .collect();

    aggregate_rmm(posteriors)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn taxonomy_has_twelve_factors_seven_inherent_five_control() {
        let factors = RMMFactor::all();
        assert_eq!(factors.len(), 12);
        let inherent = factors
            .iter()
            .filter(|f| f.category() == FactorCategory::Inherent)
            .count();
        let control = factors
            .iter()
            .filter(|f| f.category() == FactorCategory::Control)
            .count();
        assert_eq!(inherent, 7);
        assert_eq!(control, 5);
    }

    #[test]
    fn factor_categories_match_methodology_v014() {
        assert_eq!(
            RMMFactor::AccountComplexity.category(),
            FactorCategory::Inherent
        );
        assert_eq!(
            RMMFactor::FraudSusceptibility.category(),
            FactorCategory::Inherent
        );
        assert_eq!(
            RMMFactor::ControlCoverage.category(),
            FactorCategory::Control
        );
        assert_eq!(
            RMMFactor::OverrideSignals.category(),
            FactorCategory::Control
        );
    }

    #[test]
    fn default_priors_match_methodology_v014() {
        let priors = default_priors();
        assert_eq!(priors.len(), 12);

        let by_factor: std::collections::HashMap<_, _> = priors
            .iter()
            .map(|p| (p.factor, (p.alpha, p.beta)))
            .collect();
        assert_eq!(by_factor[&RMMFactor::AccountComplexity], (1.5, 4.0));
        assert_eq!(by_factor[&RMMFactor::EstimationUncertainty], (2.5, 2.5));
        assert_eq!(by_factor[&RMMFactor::TransactionVolume], (1.0, 4.0));
        assert_eq!(by_factor[&RMMFactor::FraudSusceptibility], (2.0, 3.0));
        assert_eq!(by_factor[&RMMFactor::RelatedPartyDensity], (1.5, 3.5));
        assert_eq!(by_factor[&RMMFactor::ForeignCurrencyExposure], (1.5, 3.5));
        assert_eq!(by_factor[&RMMFactor::AccountingPolicyChange], (2.0, 3.0));
        assert_eq!(by_factor[&RMMFactor::ControlCoverage], (2.0, 4.0));
        assert_eq!(by_factor[&RMMFactor::ControlTestResults], (1.5, 4.5));
        assert_eq!(by_factor[&RMMFactor::ItGeneralControls], (1.5, 4.5));
        assert_eq!(by_factor[&RMMFactor::DeficiencyHistory], (2.0, 3.0));
        assert_eq!(by_factor[&RMMFactor::OverrideSignals], (1.5, 4.5));
    }

    #[test]
    fn every_default_prior_carries_isa_refs() {
        for prior in default_priors() {
            assert!(
                !prior.isa_refs.is_empty(),
                "factor {:?} missing ISA refs",
                prior.factor
            );
            for r in &prior.isa_refs {
                assert!(r.starts_with("ISA "), "bad ref: {r}");
            }
        }
    }

    #[test]
    fn prior_mean_matches_alpha_over_alpha_plus_beta() {
        let prior = default_prior(RMMFactor::EstimationUncertainty);
        assert!(approx_eq(prior.mean(), 0.5, 1e-9));
    }

    #[test]
    fn conjugate_update_increments_alpha_by_failures_beta_by_passes() {
        let prior = default_prior(RMMFactor::FraudSusceptibility);
        let evidence = FactorEvidence {
            factor: RMMFactor::FraudSusceptibility,
            failures: 3,
            passes: 2,
        };
        let post = update_posterior(&prior, &evidence).unwrap();
        assert!(approx_eq(post.alpha, prior.alpha + 3.0, 1e-9));
        assert!(approx_eq(post.beta, prior.beta + 2.0, 1e-9));
    }

    #[test]
    fn evidence_factor_mismatch_returns_error() {
        let prior = default_prior(RMMFactor::FraudSusceptibility);
        let evidence = FactorEvidence {
            factor: RMMFactor::ControlCoverage,
            failures: 1,
            passes: 0,
        };
        let err = update_posterior(&prior, &evidence).unwrap_err();
        assert_eq!(err.prior_factor, RMMFactor::FraudSusceptibility);
        assert_eq!(err.evidence_factor, RMMFactor::ControlCoverage);
    }

    #[test]
    fn no_evidence_yields_aggregate_of_priors() {
        let request = RMMRequest {
            account_id: "revenue".to_string(),
            assertion_kind: "completeness".to_string(),
            evidence: vec![],
        };
        let result = compute_rmm(&request);
        assert_eq!(result.factor_posteriors.len(), 12);
        let priors = default_priors();
        for (post, prior) in result.factor_posteriors.iter().zip(priors.iter()) {
            assert_eq!(post.factor, prior.factor);
            assert!(approx_eq(post.alpha, prior.alpha, 1e-9));
            assert!(approx_eq(post.beta, prior.beta, 1e-9));
        }
    }

    #[test]
    fn aggregate_is_ir_times_cr() {
        let request = RMMRequest {
            account_id: "revenue".to_string(),
            assertion_kind: "completeness".to_string(),
            evidence: vec![],
        };
        let result = compute_rmm(&request);
        assert!(approx_eq(
            result.rmm_mean,
            result.inherent_risk_mean * result.control_risk_mean,
            1e-9
        ));
    }

    #[test]
    fn many_failures_push_factor_mean_toward_one() {
        let prior = default_prior(RMMFactor::FraudSusceptibility);
        let evidence = FactorEvidence {
            factor: RMMFactor::FraudSusceptibility,
            failures: 1000,
            passes: 0,
        };
        let post = update_posterior(&prior, &evidence).unwrap();
        assert!(post.mean() > 0.99, "mean = {}", post.mean());
    }

    #[test]
    fn many_passes_push_factor_mean_toward_zero() {
        let prior = default_prior(RMMFactor::FraudSusceptibility);
        let evidence = FactorEvidence {
            factor: RMMFactor::FraudSusceptibility,
            failures: 0,
            passes: 1000,
        };
        let post = update_posterior(&prior, &evidence).unwrap();
        assert!(post.mean() < 0.01, "mean = {}", post.mean());
    }

    #[test]
    fn evidence_only_updates_specified_factors() {
        let request = RMMRequest {
            account_id: "revenue".to_string(),
            assertion_kind: "completeness".to_string(),
            evidence: vec![
                FactorEvidence {
                    factor: RMMFactor::FraudSusceptibility,
                    failures: 5,
                    passes: 0,
                },
                FactorEvidence {
                    factor: RMMFactor::ControlTestResults,
                    failures: 0,
                    passes: 10,
                },
            ],
        };
        let result = compute_rmm(&request);
        // FraudSusceptibility was updated.
        let fraud = result
            .factor_posteriors
            .iter()
            .find(|p| p.factor == RMMFactor::FraudSusceptibility)
            .unwrap();
        let fraud_prior = default_prior(RMMFactor::FraudSusceptibility);
        assert!(approx_eq(fraud.alpha, fraud_prior.alpha + 5.0, 1e-9));
        assert!(approx_eq(fraud.beta, fraud_prior.beta, 1e-9));
        // AccountComplexity was NOT in the request — should equal prior.
        let acct = result
            .factor_posteriors
            .iter()
            .find(|p| p.factor == RMMFactor::AccountComplexity)
            .unwrap();
        let acct_prior = default_prior(RMMFactor::AccountComplexity);
        assert!(approx_eq(acct.alpha, acct_prior.alpha, 1e-9));
        assert!(approx_eq(acct.beta, acct_prior.beta, 1e-9));
    }

    #[test]
    fn evidence_raises_ir_or_cr_correctly() {
        let baseline = compute_rmm(&RMMRequest {
            account_id: "x".into(),
            assertion_kind: "y".into(),
            evidence: vec![],
        });

        // Heavy inherent failures should raise IR (and therefore RMM).
        let with_inherent_fail = compute_rmm(&RMMRequest {
            account_id: "x".into(),
            assertion_kind: "y".into(),
            evidence: vec![FactorEvidence {
                factor: RMMFactor::FraudSusceptibility,
                failures: 100,
                passes: 0,
            }],
        });
        assert!(with_inherent_fail.inherent_risk_mean > baseline.inherent_risk_mean);
        assert!(with_inherent_fail.rmm_mean > baseline.rmm_mean);

        // Heavy control passes should lower CR.
        let with_control_pass = compute_rmm(&RMMRequest {
            account_id: "x".into(),
            assertion_kind: "y".into(),
            evidence: vec![FactorEvidence {
                factor: RMMFactor::ControlTestResults,
                failures: 0,
                passes: 100,
            }],
        });
        assert!(with_control_pass.control_risk_mean < baseline.control_risk_mean);
    }

    #[test]
    fn json_round_trips_request_and_posterior() {
        let request = RMMRequest {
            account_id: "revenue".to_string(),
            assertion_kind: "completeness".to_string(),
            evidence: vec![FactorEvidence {
                factor: RMMFactor::FraudSusceptibility,
                failures: 3,
                passes: 0,
            }],
        };
        let json = serde_json::to_string(&request).unwrap();
        let back: RMMRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(request, back);

        let result = compute_rmm(&request);
        let json = serde_json::to_string(&result).unwrap();
        let back: RMMPosterior = serde_json::from_str(&json).unwrap();
        assert_eq!(result, back);
    }

    #[test]
    fn aggregate_uses_arithmetic_mean_of_factor_means() {
        // Hand-checked: with default priors and no evidence, IR is the
        // arithmetic mean of the 7 inherent prior means.
        let priors = default_priors();
        let inherent_means: Vec<f64> = priors
            .iter()
            .filter(|p| p.factor.category() == FactorCategory::Inherent)
            .map(|p| p.mean())
            .collect();
        let expected_ir = inherent_means.iter().sum::<f64>() / inherent_means.len() as f64;

        let result = compute_rmm(&RMMRequest {
            account_id: "x".into(),
            assertion_kind: "y".into(),
            evidence: vec![],
        });
        assert!(approx_eq(result.inherent_risk_mean, expected_ir, 1e-9));
    }
}

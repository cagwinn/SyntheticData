//! Sanctions screening evaluator.
//!
//! Validates that screening outcomes correlate appropriately with risk factors:
//! - Low-risk customers mostly Clear
//! - High-risk / sanctioned-country customers have elevated hit rates
//! - PEPs have name variations populated

use serde::{Deserialize, Serialize};

use crate::error::EvalResult;

#[derive(Debug, Clone)]
pub struct ScreeningObservation {
    pub risk_tier: String, // "low" | "medium" | "high" | "very_high" | "prohibited"
    pub is_pep: bool,
    pub is_high_risk_country: bool,
    pub screening_result: String, // "clear" | "potential_match" | "confirmed_match"
    pub has_name_variations: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanctionsScreeningThresholds {
    /// Low-risk customers should be Clear >95% of the time
    pub min_low_risk_clear_rate: f64,
    /// High-risk customers should have non-Clear >5% of the time
    pub min_high_risk_match_rate: f64,
    /// PEPs should have name_variations populated >90%
    pub min_pep_variations_rate: f64,
    /// High-risk country customers should have elevated match rate
    pub min_high_risk_country_match_rate: f64,
}

impl Default for SanctionsScreeningThresholds {
    fn default() -> Self {
        Self {
            min_low_risk_clear_rate: 0.95,
            min_high_risk_match_rate: 0.05,
            min_pep_variations_rate: 0.90,
            min_high_risk_country_match_rate: 0.05,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanctionsScreeningAnalysis {
    pub total_customers: usize,
    pub low_risk_clear_rate: f64,
    pub high_risk_match_rate: f64,
    pub pep_variations_rate: f64,
    pub high_risk_country_match_rate: f64,
    pub confirmed_match_count: usize,
    pub potential_match_count: usize,
    pub passes: bool,
    pub issues: Vec<String>,
}

pub struct SanctionsScreeningAnalyzer {
    pub thresholds: SanctionsScreeningThresholds,
}

impl SanctionsScreeningAnalyzer {
    pub fn new() -> Self {
        Self {
            thresholds: SanctionsScreeningThresholds::default(),
        }
    }

    pub fn analyze(
        &self,
        observations: &[ScreeningObservation],
    ) -> EvalResult<SanctionsScreeningAnalysis> {
        let total = observations.len();
        let mut low_risk_total = 0usize;
        let mut low_risk_clear = 0usize;
        let mut high_risk_total = 0usize;
        let mut high_risk_match = 0usize;
        let mut pep_total = 0usize;
        let mut pep_with_variations = 0usize;
        let mut hrc_total = 0usize;
        let mut hrc_match = 0usize;
        let mut confirmed = 0usize;
        let mut potential = 0usize;

        for obs in observations {
            let is_match = obs.screening_result != "clear";
            if obs.screening_result == "confirmed_match" {
                confirmed += 1;
            } else if obs.screening_result == "potential_match" {
                potential += 1;
            }

            if obs.risk_tier == "low" {
                low_risk_total += 1;
                if !is_match {
                    low_risk_clear += 1;
                }
            }
            if matches!(obs.risk_tier.as_str(), "high" | "very_high" | "prohibited") {
                high_risk_total += 1;
                if is_match {
                    high_risk_match += 1;
                }
            }
            if obs.is_pep {
                pep_total += 1;
                if obs.has_name_variations {
                    pep_with_variations += 1;
                }
            }
            if obs.is_high_risk_country {
                hrc_total += 1;
                if is_match {
                    hrc_match += 1;
                }
            }
        }

        let low_rate = if low_risk_total > 0 {
            low_risk_clear as f64 / low_risk_total as f64
        } else {
            1.0
        };
        let high_rate = if high_risk_total > 0 {
            high_risk_match as f64 / high_risk_total as f64
        } else {
            1.0
        };
        let pep_rate = if pep_total > 0 {
            pep_with_variations as f64 / pep_total as f64
        } else {
            1.0
        };
        let hrc_rate = if hrc_total > 0 {
            hrc_match as f64 / hrc_total as f64
        } else {
            1.0
        };

        let mut issues = Vec::new();
        if low_risk_total > 10 && low_rate < self.thresholds.min_low_risk_clear_rate {
            issues.push(format!(
                "Low-risk clear rate {:.1}% below minimum {:.1}% — too many false matches",
                low_rate * 100.0,
                self.thresholds.min_low_risk_clear_rate * 100.0,
            ));
        }
        if high_risk_total > 10 && high_rate < self.thresholds.min_high_risk_match_rate {
            issues.push(format!(
                "High-risk match rate {:.1}% below minimum {:.1}% — screening not detecting risky customers",
                high_rate * 100.0,
                self.thresholds.min_high_risk_match_rate * 100.0,
            ));
        }
        if pep_total > 0 && pep_rate < self.thresholds.min_pep_variations_rate {
            issues.push(format!(
                "PEP name-variation rate {:.1}% below minimum {:.1}%",
                pep_rate * 100.0,
                self.thresholds.min_pep_variations_rate * 100.0,
            ));
        }
        if hrc_total > 10 && hrc_rate < self.thresholds.min_high_risk_country_match_rate {
            issues.push(format!(
                "High-risk-country match rate {:.1}% below minimum {:.1}%",
                hrc_rate * 100.0,
                self.thresholds.min_high_risk_country_match_rate * 100.0,
            ));
        }

        Ok(SanctionsScreeningAnalysis {
            total_customers: total,
            low_risk_clear_rate: low_rate,
            high_risk_match_rate: high_rate,
            pep_variations_rate: pep_rate,
            high_risk_country_match_rate: hrc_rate,
            confirmed_match_count: confirmed,
            potential_match_count: potential,
            passes: issues.is_empty(),
            issues,
        })
    }
}

impl Default for SanctionsScreeningAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn mk_obs(tier: &str, pep: bool, hrc: bool, res: &str, vars: bool) -> ScreeningObservation {
        ScreeningObservation {
            risk_tier: tier.into(),
            is_pep: pep,
            is_high_risk_country: hrc,
            screening_result: res.into(),
            has_name_variations: vars,
        }
    }

    #[test]
    fn test_realistic_distribution_passes() {
        let mut obs = Vec::new();
        // 200 low-risk, 98% clear
        for i in 0..200 {
            let res = if i < 196 { "clear" } else { "potential_match" };
            obs.push(mk_obs("low", false, false, res, false));
        }
        // 30 high-risk, 20% match
        for i in 0..30 {
            let res = if i < 24 { "clear" } else { "potential_match" };
            obs.push(mk_obs("high", false, false, res, false));
        }
        // 5 PEPs all with variations
        for _ in 0..5 {
            obs.push(mk_obs("medium", true, false, "clear", true));
        }
        let a = SanctionsScreeningAnalyzer::new();
        let r = a.analyze(&obs).unwrap();
        assert!(r.passes, "Issues: {:?}", r.issues);
    }

    #[test]
    fn test_high_risk_with_zero_matches_flagged() {
        let obs: Vec<_> = (0..50)
            .map(|_| mk_obs("very_high", false, false, "clear", false))
            .collect();
        let a = SanctionsScreeningAnalyzer::new();
        let r = a.analyze(&obs).unwrap();
        assert!(!r.passes);
        assert!(r.issues.iter().any(|i| i.contains("High-risk")));
    }
}

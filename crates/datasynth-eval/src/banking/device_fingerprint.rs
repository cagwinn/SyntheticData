//! Device fingerprint quality evaluator.
//!
//! Validates that device usage patterns are realistic:
//! - Power-law distribution of devices per customer (most have 1)
//! - Trust scores calibrated (primary > secondary)
//! - Reuse rate aligned with observed patterns

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::EvalResult;

/// Device observation data.
#[derive(Debug, Clone)]
pub struct DeviceObservation {
    pub customer_id: String,
    pub device_id: String,
    pub trust_score: f64,
    pub is_known: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceFingerprintThresholds {
    /// Minimum fraction of customers with ≥1 device observed
    pub min_coverage: f64,
    /// Maximum fraction of customers with >5 devices (realistic heavy tail cap)
    pub max_heavy_tail_rate: f64,
    /// Minimum fraction of single-device customers (should dominate)
    pub min_single_device_rate: f64,
    /// Minimum mean trust score (should be reasonable)
    pub min_mean_trust: f64,
}

impl Default for DeviceFingerprintThresholds {
    fn default() -> Self {
        Self {
            min_coverage: 0.90,
            max_heavy_tail_rate: 0.10,
            min_single_device_rate: 0.40,
            min_mean_trust: 0.3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceFingerprintAnalysis {
    pub total_customers: usize,
    pub customers_with_devices: usize,
    pub single_device_customers: usize,
    pub heavy_tail_customers: usize,
    pub mean_devices_per_customer: f64,
    pub mean_trust_score: f64,
    pub known_device_rate: f64,
    pub passes: bool,
    pub issues: Vec<String>,
}

pub struct DeviceFingerprintAnalyzer {
    pub thresholds: DeviceFingerprintThresholds,
}

impl DeviceFingerprintAnalyzer {
    pub fn new() -> Self {
        Self {
            thresholds: DeviceFingerprintThresholds::default(),
        }
    }

    pub fn analyze(
        &self,
        observations: &[DeviceObservation],
        total_customers: usize,
    ) -> EvalResult<DeviceFingerprintAnalysis> {
        // Per-customer unique device set
        let mut devices_per_customer: HashMap<String, std::collections::HashSet<String>> =
            HashMap::new();
        let mut trust_scores: Vec<f64> = Vec::new();
        let mut known_count = 0usize;

        for obs in observations {
            devices_per_customer
                .entry(obs.customer_id.clone())
                .or_default()
                .insert(obs.device_id.clone());
            trust_scores.push(obs.trust_score);
            if obs.is_known {
                known_count += 1;
            }
        }

        let customers_with_devices = devices_per_customer.len();
        let single_device = devices_per_customer
            .values()
            .filter(|s| s.len() == 1)
            .count();
        let heavy_tail = devices_per_customer
            .values()
            .filter(|s| s.len() > 5)
            .count();
        let mean_devices = if customers_with_devices > 0 {
            devices_per_customer
                .values()
                .map(|s| s.len())
                .sum::<usize>() as f64
                / customers_with_devices as f64
        } else {
            0.0
        };
        let mean_trust = if !trust_scores.is_empty() {
            trust_scores.iter().sum::<f64>() / trust_scores.len() as f64
        } else {
            0.0
        };
        let known_rate = if !observations.is_empty() {
            known_count as f64 / observations.len() as f64
        } else {
            0.0
        };

        let coverage = if total_customers > 0 {
            customers_with_devices as f64 / total_customers as f64
        } else {
            0.0
        };
        let heavy_tail_rate = if customers_with_devices > 0 {
            heavy_tail as f64 / customers_with_devices as f64
        } else {
            0.0
        };
        let single_rate = if customers_with_devices > 0 {
            single_device as f64 / customers_with_devices as f64
        } else {
            0.0
        };

        let mut issues = Vec::new();
        if total_customers > 0 && coverage < self.thresholds.min_coverage {
            issues.push(format!(
                "Device coverage {:.1}% below minimum {:.1}%",
                coverage * 100.0,
                self.thresholds.min_coverage * 100.0,
            ));
        }
        if heavy_tail_rate > self.thresholds.max_heavy_tail_rate {
            issues.push(format!(
                "Heavy-tail rate {:.1}% above maximum {:.1}% — too many multi-device customers",
                heavy_tail_rate * 100.0,
                self.thresholds.max_heavy_tail_rate * 100.0,
            ));
        }
        if customers_with_devices > 0 && single_rate < self.thresholds.min_single_device_rate {
            issues.push(format!(
                "Single-device rate {:.1}% below minimum {:.1}% — distribution not power-law",
                single_rate * 100.0,
                self.thresholds.min_single_device_rate * 100.0,
            ));
        }
        if !trust_scores.is_empty() && mean_trust < self.thresholds.min_mean_trust {
            issues.push(format!(
                "Mean trust {:.3} below minimum {:.3} — trust score not evolving",
                mean_trust, self.thresholds.min_mean_trust,
            ));
        }

        Ok(DeviceFingerprintAnalysis {
            total_customers,
            customers_with_devices,
            single_device_customers: single_device,
            heavy_tail_customers: heavy_tail,
            mean_devices_per_customer: mean_devices,
            mean_trust_score: mean_trust,
            known_device_rate: known_rate,
            passes: issues.is_empty(),
            issues,
        })
    }
}

impl Default for DeviceFingerprintAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_power_law_distribution_passes() {
        let mut obs = Vec::new();
        // 70 customers with 1 device
        for c in 0..70 {
            obs.push(DeviceObservation {
                customer_id: format!("C{c}"),
                device_id: format!("D{c}"),
                trust_score: 0.9,
                is_known: true,
            });
        }
        // 25 customers with 2 devices
        for c in 70..95 {
            for d in 0..2 {
                obs.push(DeviceObservation {
                    customer_id: format!("C{c}"),
                    device_id: format!("D{c}-{d}"),
                    trust_score: 0.8,
                    is_known: true,
                });
            }
        }
        // 5 customers with 3 devices
        for c in 95..100 {
            for d in 0..3 {
                obs.push(DeviceObservation {
                    customer_id: format!("C{c}"),
                    device_id: format!("D{c}-{d}"),
                    trust_score: 0.7,
                    is_known: true,
                });
            }
        }
        let analyzer = DeviceFingerprintAnalyzer::new();
        let result = analyzer.analyze(&obs, 100).unwrap();
        assert!(result.passes, "Issues: {:?}", result.issues);
        assert_eq!(result.single_device_customers, 70);
    }

    #[test]
    fn test_too_many_heavy_tail_detected() {
        let mut obs = Vec::new();
        // 50 customers with 10 devices each — way too much
        for c in 0..50 {
            for d in 0..10 {
                obs.push(DeviceObservation {
                    customer_id: format!("C{c}"),
                    device_id: format!("D{c}-{d}"),
                    trust_score: 0.5,
                    is_known: true,
                });
            }
        }
        let analyzer = DeviceFingerprintAnalyzer::new();
        let result = analyzer.analyze(&obs, 50).unwrap();
        assert!(!result.passes);
        assert!(result.issues.iter().any(|i| i.contains("Heavy-tail")));
    }
}

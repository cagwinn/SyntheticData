//! Realistic per-customer device behavior.
//!
//! Replaces the flat `device_reuse_rate` with a power-law distribution:
//! most customers use 1 primary device 95%+ of the time, with a long tail
//! of multi-device users. Trust scores evolve based on observed behavior.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use uuid::Uuid;

use crate::models::{DeviceFingerprint, DeviceProfiles};

/// Per-customer device profile — a small set of devices with usage weights.
#[derive(Debug, Clone)]
pub struct CustomerDeviceProfile {
    /// List of (device, usage_weight) pairs; weights sum to 1.0 (power-law-distributed)
    pub devices: Vec<(DeviceFingerprint, f64)>,
}

impl CustomerDeviceProfile {
    /// Pick a device for this customer — weighted by usage.
    /// Returns the chosen device (cloned) with trust score updated to reflect
    /// observed behavior at this point in time.
    pub fn pick(&self, rng: &mut ChaCha8Rng) -> DeviceFingerprint {
        let roll: f64 = rng.random();
        let mut cumulative = 0.0;
        for (device, weight) in &self.devices {
            cumulative += weight;
            if roll <= cumulative {
                return device.clone();
            }
        }
        // Fallback: last device
        self.devices
            .last()
            .map(|(d, _)| d.clone())
            .unwrap_or_else(|| fresh_device(rng, false))
    }
}

/// Generator for realistic per-customer device profiles.
pub struct DeviceRealismGenerator {
    rng: ChaCha8Rng,
    profiles: HashMap<Uuid, CustomerDeviceProfile>,
    /// Exponent for power-law usage distribution (higher = more skewed to primary device)
    power_law_alpha: f64,
}

impl DeviceRealismGenerator {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(7300)),
            profiles: HashMap::new(),
            power_law_alpha: 2.5, // typical power-law exponent for device reuse
        }
    }

    /// Ensure this customer has a device profile; generate one if new.
    ///
    /// The distribution of devices-per-customer follows:
    /// - 70% of customers: 1 device (single-device users)
    /// - 20% of customers: 2 devices (phone + desktop or phone + tablet)
    /// - 7% of customers: 3 devices
    /// - 3% of customers: 4-5 devices (power users, corporate, nomads)
    pub fn get_or_create_profile(
        &mut self,
        customer_id: Uuid,
        onboarding_date: DateTime<Utc>,
    ) -> &CustomerDeviceProfile {
        if !self.profiles.contains_key(&customer_id) {
            let num_devices = self.sample_device_count();
            let devices = self.generate_device_set(num_devices, onboarding_date);
            self.profiles
                .insert(customer_id, CustomerDeviceProfile { devices });
        }
        self.profiles
            .get(&customer_id)
            .expect("just inserted")
    }

    /// Sample the number of devices for a customer from realistic distribution.
    fn sample_device_count(&mut self) -> usize {
        let roll: f64 = self.rng.random();
        if roll < 0.70 {
            1
        } else if roll < 0.90 {
            2
        } else if roll < 0.97 {
            3
        } else {
            self.rng.random_range(4..=5)
        }
    }

    /// Generate N devices with power-law usage weights.
    fn generate_device_set(
        &mut self,
        n: usize,
        onboarding_date: DateTime<Utc>,
    ) -> Vec<(DeviceFingerprint, f64)> {
        let mut devices = Vec::with_capacity(n);

        // Generate raw weights from power law: weight ~ 1/rank^alpha
        let raw_weights: Vec<f64> = (1..=n)
            .map(|rank| 1.0 / (rank as f64).powf(self.power_law_alpha))
            .collect();
        let total: f64 = raw_weights.iter().sum();
        let normalized: Vec<f64> = raw_weights.iter().map(|w| w / total).collect();

        for (i, weight) in normalized.iter().enumerate() {
            // Primary device is oldest, well-trusted
            // Secondary devices added over first year
            let age_days = if i == 0 {
                self.rng.random_range(180..730) // primary: 6mo-2yr old
            } else {
                self.rng.random_range(0..360) // secondary: added within year
            };
            let device_first_seen = onboarding_date + chrono::Duration::days(age_days);

            // Trust score scales with age and usage:
            // primary device (i=0) with long age → 0.85-0.98
            // secondary/newer → 0.4-0.8
            let base_trust = if i == 0 { 0.85 } else { 0.5 };
            let age_bonus = (age_days as f64 / 730.0).min(1.0) * 0.1;
            let usage_bonus = weight * 0.3;
            let trust = (base_trust + age_bonus + usage_bonus).min(0.99);

            // Pick device class — primary tends to be mobile for most
            let is_mobile = if i == 0 {
                self.rng.random::<f64>() < 0.75
            } else {
                self.rng.random::<f64>() < 0.5
            };

            let mut device = fresh_device(&mut self.rng, is_mobile);
            device.is_known_device = true;
            device.device_first_seen = Some(device_first_seen);
            device.device_trust_score = trust;

            devices.push((device, *weight));
        }

        devices
    }

    /// Compute trust score evolution: every N clean transactions bumps trust,
    /// every anomaly penalizes. Call this periodically during generation.
    #[allow(dead_code)]
    pub fn evolve_trust(
        &mut self,
        customer_id: Uuid,
        device_id: &str,
        clean_count: u32,
        anomaly_count: u32,
    ) {
        if let Some(profile) = self.profiles.get_mut(&customer_id) {
            for (device, _) in profile.devices.iter_mut() {
                if device.device_id == device_id {
                    let delta = (clean_count as f64 * 0.002) - (anomaly_count as f64 * 0.05);
                    device.device_trust_score =
                        (device.device_trust_score + delta).clamp(0.0, 0.99);
                }
            }
        }
    }
}

/// Generate a fresh random device (new-to-system).
fn fresh_device(rng: &mut ChaCha8Rng, is_mobile: bool) -> DeviceFingerprint {
    let device_id = format!("DEV-{:016x}", rng.random::<u64>());
    if is_mobile {
        let (model, os) = DeviceProfiles::MOBILE_DEVICES
            [rng.random_range(0..DeviceProfiles::MOBILE_DEVICES.len())];
        let os_version = match os {
            "iOS" => format!("{}.{}", rng.random_range(15..=17), rng.random_range(0..7)),
            _ => format!("{}", rng.random_range(12..=14)),
        };
        let res = DeviceProfiles::MOBILE_RESOLUTIONS
            [rng.random_range(0..DeviceProfiles::MOBILE_RESOLUTIONS.len())];
        DeviceFingerprint {
            device_id,
            device_model: Some(model.to_string()),
            os: Some(os.to_string()),
            os_version: Some(os_version),
            screen_resolution: Some(res.to_string()),
            browser: None,
            is_known_device: false,
            device_first_seen: None,
            device_trust_score: 0.0,
        }
    } else {
        let (model, os) = DeviceProfiles::DESKTOP_OS
            [rng.random_range(0..DeviceProfiles::DESKTOP_OS.len())];
        let res = DeviceProfiles::DESKTOP_RESOLUTIONS
            [rng.random_range(0..DeviceProfiles::DESKTOP_RESOLUTIONS.len())];
        let browser = DeviceProfiles::BROWSERS
            [rng.random_range(0..DeviceProfiles::BROWSERS.len())];
        DeviceFingerprint {
            device_id,
            device_model: Some(model.to_string()),
            os: Some(os.to_string()),
            os_version: Some(format!("{}", rng.random_range(10..=14))),
            screen_resolution: Some(res.to_string()),
            browser: Some(browser.to_string()),
            is_known_device: false,
            device_first_seen: None,
            device_trust_score: 0.0,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_power_law_distribution() {
        let mut gen = DeviceRealismGenerator::new(42);
        let onboarding = chrono::Utc::now();

        // Generate profiles for 1000 customers, check distribution
        let mut counts = vec![0; 6];
        for _ in 0..1000 {
            let customer_id = Uuid::new_v4();
            let profile = gen.get_or_create_profile(customer_id, onboarding);
            let n = profile.devices.len().min(5);
            counts[n] += 1;
        }
        // Most customers (≥60%) should have 1 device
        assert!(counts[1] >= 600, "Expected ≥60% single-device: {counts:?}");
        // A small fraction should have 4+
        assert!(counts[4] + counts[5] < 100, "Heavy tail should be rare");
    }

    #[test]
    fn test_primary_device_most_used() {
        let mut gen = DeviceRealismGenerator::new(42);
        let onboarding = chrono::Utc::now();
        let customer = Uuid::new_v4();
        let profile = gen.get_or_create_profile(customer, onboarding);
        if profile.devices.len() >= 2 {
            // First device should have higher weight than second (power-law)
            assert!(profile.devices[0].1 > profile.devices[1].1);
        }
    }

    #[test]
    fn test_pick_returns_weighted() {
        let mut gen = DeviceRealismGenerator::new(42);
        let onboarding = chrono::Utc::now();
        let customer = Uuid::new_v4();
        // Force creation
        let _ = gen.get_or_create_profile(customer, onboarding);

        // Pick 1000 times; primary device should dominate
        let mut picks = HashMap::new();
        let profile = gen.profiles.get(&customer).unwrap().clone();
        let mut rng = ChaCha8Rng::seed_from_u64(99);
        for _ in 0..1000 {
            let d = profile.pick(&mut rng);
            *picks.entry(d.device_id).or_insert(0u32) += 1;
        }
        if profile.devices.len() >= 2 {
            let primary_id = &profile.devices[0].0.device_id;
            let primary_count = picks.get(primary_id).copied().unwrap_or(0);
            assert!(primary_count > 600, "Primary should dominate: {primary_count}/1000");
        }
    }
}

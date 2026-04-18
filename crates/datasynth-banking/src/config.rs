//! Configuration for banking data generation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for banking data generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BankingConfig {
    /// Whether banking generation is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Population configuration
    #[serde(default)]
    pub population: PopulationConfig,
    /// Product configuration
    #[serde(default)]
    pub products: ProductConfig,
    /// Compliance configuration
    #[serde(default)]
    pub compliance: ComplianceConfig,
    /// AML typology configuration
    #[serde(default)]
    pub typologies: TypologyConfig,
    /// Spoofing (adversarial) configuration
    #[serde(default)]
    pub spoofing: SpoofingConfig,
    /// Output configuration
    #[serde(default)]
    pub output: BankingOutputConfig,
    /// Temporal behavior configuration
    #[serde(default)]
    pub temporal: TemporalBehaviorConfig,
    /// Device fingerprint configuration
    #[serde(default)]
    pub device: DeviceFingerprintConfig,
}

fn default_true() -> bool {
    true
}

impl Default for BankingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            population: PopulationConfig::default(),
            products: ProductConfig::default(),
            compliance: ComplianceConfig::default(),
            typologies: TypologyConfig::default(),
            spoofing: SpoofingConfig::default(),
            output: BankingOutputConfig::default(),
            temporal: TemporalBehaviorConfig::default(),
            device: DeviceFingerprintConfig::default(),
        }
    }
}

impl BankingConfig {
    /// Create a small configuration for testing.
    pub fn small() -> Self {
        Self {
            population: PopulationConfig {
                retail_customers: 100,
                business_customers: 20,
                trusts: 5,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Create a medium configuration.
    pub fn medium() -> Self {
        Self {
            population: PopulationConfig {
                retail_customers: 1_000,
                business_customers: 200,
                trusts: 50,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Create a large configuration.
    pub fn large() -> Self {
        Self {
            population: PopulationConfig {
                retail_customers: 10_000,
                business_customers: 1_000,
                trusts: 100,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        // Validate population
        if self.population.retail_customers == 0
            && self.population.business_customers == 0
            && self.population.trusts == 0
        {
            errors.push("At least one customer type must have non-zero count".to_string());
        }

        // Validate persona weights sum to 1.0
        let retail_sum: f64 = self.population.retail_persona_weights.values().sum();
        if (retail_sum - 1.0).abs() > 0.01 {
            errors.push(format!(
                "Retail persona weights must sum to 1.0, got {retail_sum}"
            ));
        }

        // Validate typology rates
        let total_suspicious = self.typologies.structuring_rate
            + self.typologies.funnel_rate
            + self.typologies.layering_rate
            + self.typologies.mule_rate
            + self.typologies.fraud_rate;
        if total_suspicious > self.typologies.suspicious_rate + 0.001 {
            errors.push(format!(
                "Sum of typology rates ({}) exceeds suspicious_rate ({})",
                total_suspicious, self.typologies.suspicious_rate
            ));
        }

        // Validate spoofing intensity
        if self.spoofing.intensity < 0.0 || self.spoofing.intensity > 1.0 {
            errors.push("Spoofing intensity must be between 0.0 and 1.0".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Population configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PopulationConfig {
    /// Number of retail customers
    pub retail_customers: u32,
    /// Retail persona weight distribution
    pub retail_persona_weights: HashMap<String, f64>,
    /// Number of business customers
    pub business_customers: u32,
    /// Business persona weight distribution
    pub business_persona_weights: HashMap<String, f64>,
    /// Number of trust customers
    pub trusts: u32,
    /// Household formation rate (proportion of retail in households)
    pub household_rate: f64,
    /// Average household size
    pub avg_household_size: f64,
    /// Simulation period in months
    pub period_months: u32,
    /// Simulation start date (YYYY-MM-DD)
    pub start_date: String,
}

impl Default for PopulationConfig {
    fn default() -> Self {
        let mut retail_weights = HashMap::new();
        retail_weights.insert("student".to_string(), 0.15);
        retail_weights.insert("early_career".to_string(), 0.25);
        retail_weights.insert("mid_career".to_string(), 0.30);
        retail_weights.insert("retiree".to_string(), 0.15);
        retail_weights.insert("high_net_worth".to_string(), 0.05);
        retail_weights.insert("gig_worker".to_string(), 0.10);

        let mut business_weights = HashMap::new();
        business_weights.insert("small_business".to_string(), 0.50);
        business_weights.insert("mid_market".to_string(), 0.25);
        business_weights.insert("enterprise".to_string(), 0.05);
        business_weights.insert("cash_intensive".to_string(), 0.10);
        business_weights.insert("import_export".to_string(), 0.05);
        business_weights.insert("professional_services".to_string(), 0.05);

        Self {
            retail_customers: 10_000,
            retail_persona_weights: retail_weights,
            business_customers: 1_000,
            business_persona_weights: business_weights,
            trusts: 100,
            household_rate: 0.4,
            avg_household_size: 2.3,
            period_months: 12,
            start_date: "2024-01-01".to_string(),
        }
    }
}

/// Product configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductConfig {
    /// Cash transaction intensity (0.0-1.0)
    pub cash_intensity: f64,
    /// Cross-border transaction rate (0.0-1.0)
    pub cross_border_rate: f64,
    /// Card vs transfer ratio for payments
    pub card_vs_transfer: f64,
    /// Average accounts per retail customer
    pub avg_accounts_retail: f64,
    /// Average accounts per business customer
    pub avg_accounts_business: f64,
    /// Proportion of customers with debit cards
    pub debit_card_rate: f64,
    /// Proportion of customers with international capability
    pub international_rate: f64,
}

impl Default for ProductConfig {
    fn default() -> Self {
        Self {
            cash_intensity: 0.15,
            cross_border_rate: 0.05,
            card_vs_transfer: 0.6,
            avg_accounts_retail: 1.5,
            avg_accounts_business: 2.5,
            debit_card_rate: 0.85,
            international_rate: 0.10,
        }
    }
}

/// Compliance configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceConfig {
    /// Risk appetite (low, medium, high)
    pub risk_appetite: RiskAppetite,
    /// KYC completeness rate (0.0-1.0)
    pub kyc_completeness: f64,
    /// Proportion of high-risk customers accepted
    pub high_risk_tolerance: f64,
    /// PEP proportion in customer base
    pub pep_rate: f64,
    /// Enhanced due diligence trigger threshold
    pub edd_threshold: u64,
}

impl Default for ComplianceConfig {
    fn default() -> Self {
        Self {
            risk_appetite: RiskAppetite::Medium,
            kyc_completeness: 0.95,
            high_risk_tolerance: 0.05,
            pep_rate: 0.01,
            edd_threshold: 50_000,
        }
    }
}

/// Risk appetite level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RiskAppetite {
    /// Low risk tolerance
    Low,
    /// Medium risk tolerance
    #[default]
    Medium,
    /// High risk tolerance
    High,
}

impl RiskAppetite {
    /// High-risk customer multiplier.
    pub fn high_risk_multiplier(&self) -> f64 {
        match self {
            Self::Low => 0.5,
            Self::Medium => 1.0,
            Self::High => 2.0,
        }
    }
}

/// AML typology configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypologyConfig {
    /// Overall suspicious activity rate (0.0-1.0)
    pub suspicious_rate: f64,
    /// Structuring typology rate
    pub structuring_rate: f64,
    /// Funnel account rate
    pub funnel_rate: f64,
    /// Layering chain rate
    pub layering_rate: f64,
    /// Money mule rate
    pub mule_rate: f64,
    /// Fraud rate (ATO, synthetic, etc.)
    pub fraud_rate: f64,
    /// Sophistication distribution
    pub sophistication: SophisticationDistribution,
    /// Base detectability (0.0-1.0)
    pub detectability: f64,
    /// Round-tripping rate
    pub round_tripping_rate: f64,
    /// Trade-based ML rate
    pub trade_based_rate: f64,
    /// Synthetic identity fraud rate
    #[serde(default = "default_synth_id_rate")]
    pub synthetic_identity_rate: f64,
    /// Cryptocurrency integration rate
    #[serde(default = "default_crypto_rate")]
    pub crypto_integration_rate: f64,
    /// Sanctions evasion rate
    #[serde(default = "default_sanctions_rate")]
    pub sanctions_evasion_rate: f64,
    /// False positive rate (fraction of legitimate txns tagged as suspicious-looking)
    #[serde(default = "default_false_positive_rate")]
    pub false_positive_rate: f64,
    /// Cross-typology co-occurrence rate (fraction of cases combining multiple typologies)
    #[serde(default = "default_co_occurrence_rate")]
    pub co_occurrence_rate: f64,
    /// Multi-party network scenario rate (fraction using coordinated networks)
    #[serde(default = "default_network_rate")]
    pub network_typology_rate: f64,
    /// Fraction of document-flow Payments to bridge to BankTransactions (0.0 disables)
    #[serde(default = "default_payment_bridge_rate")]
    pub payment_bridge_rate: f64,
    /// Spoofing typology rate — rapid placed-and-cancelled / reversed
    /// transactions designed to probe or evade detection thresholds.
    #[serde(default = "default_spoofing_rate")]
    pub spoofing_rate: f64,
}

fn default_synth_id_rate() -> f64 {
    0.001
}
fn default_crypto_rate() -> f64 {
    0.001
}
fn default_sanctions_rate() -> f64 {
    0.0005
}
fn default_false_positive_rate() -> f64 {
    0.05
}
fn default_co_occurrence_rate() -> f64 {
    0.10
}
fn default_network_rate() -> f64 {
    // v3.1.1 — boosted from 0.05 to 0.15 so non-TransactionCounterparty
    // edges (BeneficialOwnership, MuleLink, ShellLink) make up ≥10 % of
    // the AML graph. Prior 0.05 produced 97 % TransactionCounterparty,
    // starving link-prediction models of signal.
    0.15
}
fn default_payment_bridge_rate() -> f64 {
    0.75
}
fn default_spoofing_rate() -> f64 {
    // Modest rate for spoofing — ensures Spoofing typology is covered in
    // the evaluation catalog without drowning out other signals.
    0.002
}

impl Default for TypologyConfig {
    fn default() -> Self {
        Self {
            suspicious_rate: 0.02,
            structuring_rate: 0.004,
            funnel_rate: 0.003,
            layering_rate: 0.003,
            mule_rate: 0.005,
            fraud_rate: 0.005,
            sophistication: SophisticationDistribution::default(),
            detectability: 0.5,
            // v3.1.1: bumped from 0.001 → 0.002 so round_tripping is
            // represented in select_typology's weighted draw and the
            // evaluator's 7-typology coverage check can pass.
            round_tripping_rate: 0.002,
            trade_based_rate: 0.001,
            synthetic_identity_rate: 0.001,
            crypto_integration_rate: 0.001,
            sanctions_evasion_rate: 0.0005,
            false_positive_rate: 0.05,
            co_occurrence_rate: 0.10,
            network_typology_rate: 0.15,
            payment_bridge_rate: 0.75,
            spoofing_rate: 0.002,
        }
    }
}

/// Sophistication level distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SophisticationDistribution {
    /// Basic sophistication weight
    pub basic: f64,
    /// Standard sophistication weight
    pub standard: f64,
    /// Professional sophistication weight
    pub professional: f64,
    /// Advanced sophistication weight
    pub advanced: f64,
}

impl Default for SophisticationDistribution {
    fn default() -> Self {
        Self {
            basic: 0.4,
            standard: 0.35,
            professional: 0.2,
            advanced: 0.05,
        }
    }
}

/// Spoofing (adversarial) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpoofingConfig {
    /// Enable spoofing mode
    pub enabled: bool,
    /// Spoofing intensity (0.0-1.0)
    pub intensity: f64,
    /// Spoof transaction timing
    pub spoof_timing: bool,
    /// Spoof transaction amounts
    pub spoof_amounts: bool,
    /// Spoof merchant selection
    pub spoof_merchants: bool,
    /// Spoof geographic patterns
    pub spoof_geography: bool,
    /// Add delays to reduce velocity detection
    pub add_delays: bool,
}

impl Default for SpoofingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            intensity: 0.3,
            spoof_timing: true,
            spoof_amounts: true,
            spoof_merchants: true,
            spoof_geography: false,
            add_delays: true,
        }
    }
}

/// Banking output configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BankingOutputConfig {
    /// Output directory (relative to main output)
    pub directory: String,
    /// Include customer master data
    pub include_customers: bool,
    /// Include account master data
    pub include_accounts: bool,
    /// Include transactions
    pub include_transactions: bool,
    /// Include counterparties
    pub include_counterparties: bool,
    /// Include beneficial ownership
    pub include_beneficial_ownership: bool,
    /// Include transaction labels
    pub include_transaction_labels: bool,
    /// Include entity labels
    pub include_entity_labels: bool,
    /// Include relationship labels
    pub include_relationship_labels: bool,
    /// Include case narratives
    pub include_case_narratives: bool,
    /// Export graph data
    pub include_graph: bool,
}

impl Default for BankingOutputConfig {
    fn default() -> Self {
        Self {
            directory: "banking".to_string(),
            include_customers: true,
            include_accounts: true,
            include_transactions: true,
            include_counterparties: true,
            include_beneficial_ownership: true,
            include_transaction_labels: true,
            include_entity_labels: true,
            include_relationship_labels: true,
            include_case_narratives: true,
            include_graph: true,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = BankingConfig::default();
        assert!(config.enabled);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_small_config() {
        let config = BankingConfig::small();
        assert_eq!(config.population.retail_customers, 100);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validation_empty_population() {
        let config = BankingConfig {
            population: PopulationConfig {
                retail_customers: 0,
                business_customers: 0,
                trusts: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_persona_weights() {
        let config = BankingConfig::default();
        let sum: f64 = config.population.retail_persona_weights.values().sum();
        assert!((sum - 1.0).abs() < 0.01);
    }
}

/// Temporal behavior configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalBehaviorConfig {
    /// Enable account lifecycle phases (New → RampUp → Steady → Decline → Dormant)
    #[serde(default = "default_true")]
    pub enable_lifecycle_phases: bool,
    /// Enable behavioral drift (gradual/sudden spending pattern shifts)
    #[serde(default = "default_true")]
    pub enable_behavioral_drift: bool,
    /// Enable velocity feature pre-computation on every transaction
    #[serde(default = "default_true")]
    pub enable_velocity_features: bool,
    /// Enable impossible travel injection (geolocation anomalies)
    #[serde(default = "default_true")]
    pub enable_impossible_travel: bool,
    /// Proportion of customers with behavioral drift (default 5%)
    #[serde(default = "default_drift_rate")]
    pub drift_rate: f64,
    /// Of drifting customers, proportion with sudden (suspicious) drift vs gradual (normal)
    #[serde(default = "default_sudden_ratio")]
    pub sudden_drift_ratio: f64,
    /// Impossible travel injection rate for suspicious customers
    #[serde(default = "default_impossible_travel_rate")]
    pub impossible_travel_rate: f64,
}

fn default_drift_rate() -> f64 {
    0.05
}
fn default_sudden_ratio() -> f64 {
    0.30
}
fn default_impossible_travel_rate() -> f64 {
    0.02
}

impl Default for TemporalBehaviorConfig {
    fn default() -> Self {
        Self {
            enable_lifecycle_phases: true,
            enable_behavioral_drift: true,
            enable_velocity_features: true,
            enable_impossible_travel: true,
            drift_rate: 0.05,
            sudden_drift_ratio: 0.30,
            impossible_travel_rate: 0.02,
        }
    }
}

/// Device fingerprint generation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceFingerprintConfig {
    /// Enable structured device fingerprints (replaces simple DEV-hash pattern)
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Probability a customer reuses an existing device vs generating a new one
    #[serde(default = "default_device_reuse")]
    pub device_reuse_rate: f64,
    /// Proportion of customers with 2+ devices
    #[serde(default = "default_multi_device")]
    pub multi_device_rate: f64,
}

fn default_device_reuse() -> f64 {
    0.85
}
fn default_multi_device() -> f64 {
    0.30
}

impl Default for DeviceFingerprintConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            device_reuse_rate: 0.85,
            multi_device_rate: 0.30,
        }
    }
}

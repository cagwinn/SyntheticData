//! Banking/AML fingerprint component.
//!
//! Captures aggregate patterns from the banking layer for privacy-preserving
//! re-synthesis: customer type distribution, risk tier distribution, account
//! type distribution, typology rates, amount distributions, and velocity patterns.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Banking fingerprint captured from a banking dataset.
///
/// This is separate from the main `StatisticsFingerprint` because banking has
/// a richer schema (AML typologies, KYC profiles, network structures) that
/// benefits from dedicated extraction.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BankingFingerprint {
    /// Total customers extracted
    pub customer_count: usize,
    /// Total accounts extracted
    pub account_count: usize,
    /// Total transactions extracted
    pub transaction_count: usize,

    /// Customer type distribution (retail, business, trust, etc.)
    #[serde(default)]
    pub customer_type_dist: HashMap<String, f64>,

    /// Risk tier distribution
    #[serde(default)]
    pub risk_tier_dist: HashMap<String, f64>,

    /// Retail persona distribution
    #[serde(default)]
    pub retail_persona_dist: HashMap<String, f64>,

    /// Account type distribution
    #[serde(default)]
    pub account_type_dist: HashMap<String, f64>,

    /// Transaction channel distribution
    #[serde(default)]
    pub channel_dist: HashMap<String, f64>,

    /// Transaction category distribution
    #[serde(default)]
    pub category_dist: HashMap<String, f64>,

    /// Suspicious rate (fraction of transactions with is_suspicious=true)
    pub suspicious_rate: f64,

    /// False positive rate (fraction with is_false_positive=true)
    pub false_positive_rate: f64,

    /// AML typology rates (typology -> rate across suspicious transactions)
    #[serde(default)]
    pub typology_dist: HashMap<String, f64>,

    /// Amount distribution (log-normal parameters for amount field)
    pub amount_log_mu: f64,
    pub amount_log_sigma: f64,
    pub amount_min: f64,
    pub amount_max: f64,

    /// Fraction of transactions linked to document-flow payments
    pub bridged_payment_rate: f64,

    /// Fraction of transactions with network_context (multi-party scenarios)
    pub network_rate: f64,

    /// Per-customer account count distribution
    pub accounts_per_customer_mean: f64,
    pub accounts_per_customer_std: f64,

    /// Per-account transaction count distribution
    pub txns_per_account_mean: f64,
    pub txns_per_account_std: f64,

    /// PEP rate (politically exposed persons)
    pub pep_rate: f64,

    /// Mule account rate (ground truth)
    pub mule_rate: f64,

    /// Cross-border transaction rate
    pub cross_border_rate: f64,

    /// Cash transaction rate
    pub cash_rate: f64,
}

impl BankingFingerprint {
    /// Helper to normalize a count map into a rate distribution.
    pub fn normalize_counts(counts: &HashMap<String, usize>) -> HashMap<String, f64> {
        let total: usize = counts.values().sum();
        if total == 0 {
            return HashMap::new();
        }
        counts
            .iter()
            .map(|(k, v)| (k.clone(), *v as f64 / total as f64))
            .collect()
    }
}

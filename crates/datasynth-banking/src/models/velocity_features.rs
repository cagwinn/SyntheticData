//! Pre-computed velocity features for ML training.
//!
//! Rolling-window transaction features computed post-generation so that every
//! transaction carries its own behavioral context. Eliminates the need for
//! consumers to compute these features themselves.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Pre-computed rolling-window velocity features for a single transaction.
///
/// These are computed as a post-processing pass after all transactions
/// (including typology-injected ones) are generated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VelocityFeatures {
    // --- Transaction count windows ---
    /// Number of transactions in the last 1 hour (same account)
    pub txn_count_1h: u32,
    /// Number of transactions in the last 24 hours
    pub txn_count_24h: u32,
    /// Number of transactions in the last 7 days
    pub txn_count_7d: u32,
    /// Number of transactions in the last 30 days
    pub txn_count_30d: u32,

    // --- Amount sum windows ---
    /// Total amount transacted in the last 24 hours
    #[serde(with = "datasynth_core::serde_decimal")]
    pub amount_sum_24h: Decimal,
    /// Total amount transacted in the last 7 days
    #[serde(with = "datasynth_core::serde_decimal")]
    pub amount_sum_7d: Decimal,
    /// Total amount transacted in the last 30 days
    #[serde(with = "datasynth_core::serde_decimal")]
    pub amount_sum_30d: Decimal,
    /// Maximum single transaction in the last 24 hours
    #[serde(with = "datasynth_core::serde_decimal")]
    pub amount_max_24h: Decimal,

    // --- Counterparty diversity ---
    /// Unique counterparties in the last 24 hours
    pub unique_counterparties_24h: u16,
    /// Unique counterparties in the last 7 days
    pub unique_counterparties_7d: u16,
    /// Unique countries in the last 7 days
    pub unique_countries_7d: u16,

    // --- Statistical features ---
    /// 30-day average transaction amount for this account
    #[serde(with = "datasynth_core::serde_decimal")]
    pub avg_amount_30d: Decimal,
    /// Standard deviation of transaction amounts over 30 days
    pub std_amount_30d: f64,
    /// Z-score of this transaction's amount relative to the 30-day distribution
    /// `(amount - avg_30d) / std_30d` (0.0 if insufficient history)
    pub amount_zscore: f64,
}

impl Default for VelocityFeatures {
    fn default() -> Self {
        Self {
            txn_count_1h: 0,
            txn_count_24h: 0,
            txn_count_7d: 0,
            txn_count_30d: 0,
            amount_sum_24h: Decimal::ZERO,
            amount_sum_7d: Decimal::ZERO,
            amount_sum_30d: Decimal::ZERO,
            amount_max_24h: Decimal::ZERO,
            unique_counterparties_24h: 0,
            unique_counterparties_7d: 0,
            unique_countries_7d: 0,
            avg_amount_30d: Decimal::ZERO,
            std_amount_30d: 0.0,
            amount_zscore: 0.0,
        }
    }
}

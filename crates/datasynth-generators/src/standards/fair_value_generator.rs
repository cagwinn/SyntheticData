//! Fair Value Measurement Generator (IFRS 13 / ASC 820).
//!
//! Generates `FairValueMeasurement` records across the three-level
//! hierarchy:
//!
//! - **Level 1**: Quoted prices in active markets (exchange-traded
//!   securities, commodities with daily quotes).
//! - **Level 2**: Observable inputs other than Level 1 prices
//!   (similar assets in less active markets, yield curves,
//!   interest-rate-derived valuations).
//! - **Level 3**: Unobservable inputs — discounted cash flow with
//!   internal projections, privately-held investments. Optionally
//!   carries a `SensitivityAnalysis` when `config.include_sensitivity_analysis`
//!   is `true`.
//!
//! The distribution across levels is driven by
//! `config.{level1,level2,level3}_percent`. Valuation technique is
//! chosen so that Level 1 uses Market Approach, Level 2 uses a mix
//! of Market / Income, and Level 3 defaults to Income Approach.

use chrono::NaiveDate;
use datasynth_config::schema::FairValueConfig;
use datasynth_core::utils::seeded_rng;
use datasynth_standards::accounting::fair_value::{
    FairValueCategory, FairValueHierarchyLevel, FairValueMeasurement, MeasurementType,
    SensitivityAnalysis, ValuationInput, ValuationTechnique,
};
use datasynth_standards::framework::AccountingFramework;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rand_distr::LogNormal;
use rust_decimal::prelude::*;
use rust_decimal::Decimal;

/// Generator for fair value measurements.
pub struct FairValueGenerator {
    rng: ChaCha8Rng,
}

impl FairValueGenerator {
    /// Create a new generator with the given seed.
    pub fn new(seed: u64) -> Self {
        Self {
            rng: seeded_rng(seed, 0),
        }
    }

    /// Generate `config.measurement_count` fair value measurements.
    ///
    /// # Arguments
    ///
    /// * `company_code` — Entity code to stamp on the item_id prefix.
    /// * `measurement_date` — The balance-sheet date of the measurement.
    /// * `currency` — ISO 4217 currency code.
    /// * `config` — Fair-value section from the accounting standards config.
    /// * `framework` — Accounting framework (US GAAP / IFRS / Dual).
    pub fn generate(
        &mut self,
        company_code: &str,
        measurement_date: NaiveDate,
        currency: &str,
        config: &FairValueConfig,
        framework: AccountingFramework,
    ) -> Vec<FairValueMeasurement> {
        if config.measurement_count == 0 {
            return Vec::new();
        }

        let value_dist = LogNormal::new(13.0, 1.5).expect("valid lognormal");
        let mut measurements = Vec::with_capacity(config.measurement_count);

        for i in 0..config.measurement_count {
            let hierarchy_level = self.pick_level(config);
            let category = self.pick_category(hierarchy_level);
            let valuation_technique = self.technique_for(hierarchy_level);

            let value_sample: f64 = value_dist.sample(&mut self.rng);
            let value_raw = value_sample.max(100.0_f64);
            let fair_value = Decimal::from_f64(value_raw)
                .unwrap_or_else(|| Decimal::from(100_000))
                .round_dp(2);

            let item_id = format!("{}-FV-{:05}", company_code, i + 1);
            let item_description = Self::description_for(category);

            let mut measurement = FairValueMeasurement::new(
                item_id,
                item_description,
                category,
                hierarchy_level,
                fair_value,
                measurement_date,
                currency,
                framework,
            );
            measurement.valuation_technique = valuation_technique;
            measurement.measurement_type = self.pick_measurement_type();

            // Attach observable inputs for Level 1/2; unobservable for Level 3.
            self.attach_inputs(&mut measurement);

            // Sensitivity analysis only for Level 3 when requested.
            if hierarchy_level == FairValueHierarchyLevel::Level3
                && config.include_sensitivity_analysis
            {
                measurement.sensitivity_analysis =
                    Some(self.build_sensitivity_analysis(fair_value));
            }

            measurements.push(measurement);
        }

        measurements
    }

    fn pick_level(&mut self, config: &FairValueConfig) -> FairValueHierarchyLevel {
        let roll: f64 = self.rng.random();
        if roll < config.level1_percent {
            FairValueHierarchyLevel::Level1
        } else if roll < config.level1_percent + config.level2_percent {
            FairValueHierarchyLevel::Level2
        } else {
            FairValueHierarchyLevel::Level3
        }
    }

    fn pick_category(&mut self, level: FairValueHierarchyLevel) -> FairValueCategory {
        // Level 1 favours trading/AFS securities; Level 2 covers
        // derivatives + pension plan assets; Level 3 leans toward
        // investment property, contingent consideration, private
        // instruments.
        let options: &[FairValueCategory] = match level {
            FairValueHierarchyLevel::Level1 => &[
                FairValueCategory::TradingSecurities,
                FairValueCategory::AvailableForSale,
                FairValueCategory::PensionAssets,
            ],
            FairValueHierarchyLevel::Level2 => &[
                FairValueCategory::Derivatives,
                FairValueCategory::AvailableForSale,
                FairValueCategory::PensionAssets,
                FairValueCategory::Other,
            ],
            FairValueHierarchyLevel::Level3 => &[
                FairValueCategory::InvestmentProperty,
                FairValueCategory::ContingentConsideration,
                FairValueCategory::BiologicalAssets,
                FairValueCategory::ImpairedAssets,
                FairValueCategory::Other,
            ],
        };
        *options
            .choose(&mut self.rng)
            .unwrap_or(&FairValueCategory::Other)
    }

    fn technique_for(&mut self, level: FairValueHierarchyLevel) -> ValuationTechnique {
        match level {
            FairValueHierarchyLevel::Level1 => ValuationTechnique::MarketApproach,
            FairValueHierarchyLevel::Level2 => {
                if self.rng.random_bool(0.5) {
                    ValuationTechnique::MarketApproach
                } else {
                    ValuationTechnique::IncomeApproach
                }
            }
            FairValueHierarchyLevel::Level3 => {
                if self.rng.random_bool(0.7) {
                    ValuationTechnique::IncomeApproach
                } else {
                    ValuationTechnique::MultipleApproaches
                }
            }
        }
    }

    fn pick_measurement_type(&mut self) -> MeasurementType {
        // ~85 % recurring is the typical mix for portfolio + derivatives
        // valuations; non-recurring is mostly impairment / held-for-sale.
        if self.rng.random_bool(0.85) {
            MeasurementType::Recurring
        } else {
            MeasurementType::NonRecurring
        }
    }

    fn description_for(category: FairValueCategory) -> String {
        match category {
            FairValueCategory::TradingSecurities => "Trading portfolio — listed equities",
            FairValueCategory::AvailableForSale => "AFS portfolio — government bonds",
            FairValueCategory::Derivatives => "Interest rate swap — receive fixed / pay float",
            FairValueCategory::InvestmentProperty => "Investment property — commercial building",
            FairValueCategory::BiologicalAssets => "Biological assets — forestry plantation",
            FairValueCategory::PensionAssets => "Defined-benefit plan assets — pooled equities",
            FairValueCategory::ContingentConsideration => {
                "Contingent consideration — business combination earnout"
            }
            FairValueCategory::ImpairedAssets => "Impaired fixed asset — held for sale",
            FairValueCategory::Other => "Other financial instrument",
        }
        .to_string()
    }

    fn attach_inputs(&mut self, measurement: &mut FairValueMeasurement) {
        match measurement.hierarchy_level {
            FairValueHierarchyLevel::Level1 => {
                measurement.add_input(ValuationInput::new(
                    "Quoted Price",
                    measurement.fair_value,
                    "USD",
                    true,
                    "Listed exchange close price",
                ));
            }
            FairValueHierarchyLevel::Level2 => {
                measurement.add_input(ValuationInput::discount_rate(
                    Decimal::from_f64(self.rng.random_range(0.03_f64..0.07_f64))
                        .unwrap_or(Decimal::ZERO)
                        .round_dp(4),
                    "Broker-quoted yield curve",
                ));
                measurement.add_input(ValuationInput::new(
                    "Comparable Bid-Ask Midpoint",
                    measurement.fair_value,
                    measurement.currency.clone(),
                    true,
                    "Trade publication",
                ));
            }
            FairValueHierarchyLevel::Level3 => {
                measurement.add_input(ValuationInput::discount_rate(
                    Decimal::from_f64(self.rng.random_range(0.08_f64..0.15_f64))
                        .unwrap_or(Decimal::ZERO)
                        .round_dp(4),
                    "Internal cost of capital",
                ));
                measurement.add_input(ValuationInput::growth_rate(
                    Decimal::from_f64(self.rng.random_range(0.01_f64..0.05_f64))
                        .unwrap_or(Decimal::ZERO)
                        .round_dp(4),
                    "Management projections",
                ));
            }
        }
    }

    fn build_sensitivity_analysis(&mut self, fair_value: Decimal) -> SensitivityAnalysis {
        // Shift the primary input ±50 bps → ~±10 % fair value swing.
        let low_factor = Decimal::from_str_exact("0.90").expect("const");
        let high_factor = Decimal::from_str_exact("1.10").expect("const");
        let low_rate = Decimal::from_str_exact("0.08").expect("const");
        let high_rate = Decimal::from_str_exact("0.12").expect("const");
        SensitivityAnalysis {
            input_name: "Discount Rate".to_string(),
            input_range: (low_rate, high_rate),
            fair_value_low: (fair_value * low_factor).round_dp(2),
            fair_value_high: (fair_value * high_factor).round_dp(2),
            correlated_inputs: vec!["Expected Growth Rate".to_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_config() -> FairValueConfig {
        FairValueConfig {
            enabled: true,
            measurement_count: 25,
            level1_percent: 0.60,
            level2_percent: 0.30,
            level3_percent: 0.10,
            include_sensitivity_analysis: true,
        }
    }

    #[test]
    fn generates_requested_count() {
        let mut gen = FairValueGenerator::new(42);
        let cfg = fixture_config();
        let m = gen.generate(
            "C001",
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            "USD",
            &cfg,
            AccountingFramework::UsGaap,
        );
        assert_eq!(m.len(), cfg.measurement_count);
    }

    #[test]
    fn hierarchy_distribution_is_approximately_configured() {
        let mut gen = FairValueGenerator::new(7);
        let mut cfg = fixture_config();
        cfg.measurement_count = 500;
        let m = gen.generate(
            "C001",
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            "USD",
            &cfg,
            AccountingFramework::Ifrs,
        );
        let l1 = m
            .iter()
            .filter(|x| x.hierarchy_level == FairValueHierarchyLevel::Level1)
            .count();
        let l3 = m
            .iter()
            .filter(|x| x.hierarchy_level == FairValueHierarchyLevel::Level3)
            .count();
        assert!(l1 > l3, "Level 1 should dominate over Level 3");
        assert!(l1 > 250, "Level 1 ~60 % of 500 = 300 expected, got {l1}");
    }

    #[test]
    fn sensitivity_analysis_only_on_level3() {
        let mut gen = FairValueGenerator::new(13);
        let cfg = fixture_config();
        let m = gen.generate(
            "C001",
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            "USD",
            &cfg,
            AccountingFramework::Ifrs,
        );
        for entry in &m {
            let has_sens = entry.sensitivity_analysis.is_some();
            let is_l3 = entry.hierarchy_level == FairValueHierarchyLevel::Level3;
            assert_eq!(
                has_sens, is_l3,
                "sensitivity analysis should be present iff level 3"
            );
        }
    }

    #[test]
    fn level1_uses_market_approach() {
        let mut gen = FairValueGenerator::new(5);
        let cfg = fixture_config();
        let m = gen.generate(
            "C001",
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            "USD",
            &cfg,
            AccountingFramework::UsGaap,
        );
        for entry in &m {
            if entry.hierarchy_level == FairValueHierarchyLevel::Level1 {
                assert_eq!(
                    entry.valuation_technique,
                    ValuationTechnique::MarketApproach
                );
            }
        }
    }
}

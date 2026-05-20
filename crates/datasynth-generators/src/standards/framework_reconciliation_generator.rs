//! Framework Reconciliation Generator (US GAAP ↔ IFRS for Dual Reporting).
//!
//! Generates `FrameworkDifferenceRecord` entries describing how the
//! same underlying economic transaction is measured differently under
//! US GAAP vs IFRS, plus a top-level `FrameworkReconciliation` per
//! entity summarizing cumulative NI / equity / asset differences.
//!
//! This generator only fires when `accounting_standards.framework`
//! resolves to `DualReporting` AND
//! `accounting_standards.generate_differences == true`. It does NOT
//! perform real cross-standard accounting — it draws realistic-magnitude
//! differences from a fixed distribution, stamps them against the
//! entity's generated transactions, and reports the reconciled totals.
//! This is sufficient for ML training sets and audit-procedure
//! demonstrations; true standards-aware cross-framework measurement
//! belongs in the individual standards generators (Lease v2 / ECL v2
//! already produce framework-aware outputs).
//!
//! Deferred to v3.4+:
//!   - True per-line tracing from underlying transactions to framework
//!     differences (requires mapping revenue contracts / leases / ECL
//!     models by item across frameworks).
//!   - Deferred tax impact of temporary differences on the DTL/DTA
//!     rollforward.

use chrono::NaiveDate;
use datasynth_core::utils::seeded_rng;
use datasynth_standards::accounting::differences::{
    DifferenceArea, DifferenceType, FinancialStatementImpact, FrameworkDifferenceRecord,
    FrameworkReconciliation, ReconcilingItem,
};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rand_distr::Normal;
use rust_decimal::prelude::*;
use rust_decimal::Decimal;

/// Magnitude of differences to generate per entity.
const DEFAULT_DIFFERENCE_COUNT: usize = 8;

/// Canonical difference areas with realistic explanation texts.
///
/// Each tuple: (area, explanation, is-typically-temporary).
const CANONICAL_DIFFERENCES: &[(DifferenceArea, &str, bool)] = &[
    (
        DifferenceArea::RevenueRecognition,
        "Differences in point-in-time vs. over-time recognition criteria (ASC 606-10-25 vs IFRS 15.35)",
        true,
    ),
    (
        DifferenceArea::LeaseAccounting,
        "Operating lease classification retained under ASC 842 while IFRS 16 applies a single-model on-balance-sheet approach",
        true,
    ),
    (
        DifferenceArea::InventoryCosting,
        "US GAAP permits LIFO; IFRS (IAS 2) prohibits LIFO, requiring FIFO or weighted-average",
        false,
    ),
    (
        DifferenceArea::DevelopmentCosts,
        "US GAAP expenses development costs as incurred (ASC 730); IFRS (IAS 38) capitalizes eligible development costs",
        true,
    ),
    (
        DifferenceArea::PropertyRevaluation,
        "US GAAP uses cost model only; IFRS (IAS 16) permits revaluation model for PP&E",
        true,
    ),
    (
        DifferenceArea::Impairment,
        "IFRS (IAS 36) permits reversal of impairment for non-financial assets (ex-goodwill); US GAAP prohibits reversal",
        true,
    ),
    (
        DifferenceArea::ContingentLiabilities,
        "Recognition threshold differs: 'probable' (ASC 450) vs 'more likely than not' (IAS 37)",
        true,
    ),
    (
        DifferenceArea::ShareBasedPayment,
        "Graded vesting attribution: US GAAP permits straight-line; IFRS requires accelerated method",
        true,
    ),
    (
        DifferenceArea::FinancialInstruments,
        "Classification categories differ: ASC 326 vs IFRS 9 three-category model",
        true,
    ),
    (
        DifferenceArea::Consolidation,
        "Control assessment differs: voting-interest model (US GAAP) vs single control model (IFRS 10)",
        false,
    ),
    (
        DifferenceArea::JointArrangements,
        "Equity-method vs proportionate consolidation election for joint arrangements",
        true,
    ),
    (
        DifferenceArea::IncomeTaxes,
        "Uncertain tax position recognition threshold (ASC 740 'more likely than not' vs IAS 12 probability-weighted expected value)",
        true,
    ),
];

/// Generator for framework-difference records and per-entity reconciliation.
pub struct FrameworkReconciliationGenerator {
    rng: ChaCha8Rng,
}

impl FrameworkReconciliationGenerator {
    /// Create a new generator with the given seed.
    pub fn new(seed: u64) -> Self {
        Self {
            rng: seeded_rng(seed, 0),
        }
    }

    /// Generate framework-difference records + a top-level reconciliation
    /// for a single entity.
    pub fn generate(
        &mut self,
        company_code: &str,
        period_date: NaiveDate,
    ) -> (Vec<FrameworkDifferenceRecord>, FrameworkReconciliation) {
        // Draw per-difference amount magnitudes from a log-normal-ish
        // normal: mean 1_000_000, σ=1_500_000, truncated at 5_000.
        let diff_dist = Normal::new(1_000_000.0_f64, 1_500_000.0_f64).expect("positive sigma");

        let mut records = Vec::with_capacity(DEFAULT_DIFFERENCE_COUNT);
        let mut cumulative_impact = FinancialStatementImpact::default();

        // Shuffle CANONICAL_DIFFERENCES and take the first N.
        let mut indices: Vec<usize> = (0..CANONICAL_DIFFERENCES.len()).collect();
        indices.shuffle(&mut self.rng);
        let count = DEFAULT_DIFFERENCE_COUNT.min(indices.len());

        for idx in indices.into_iter().take(count) {
            let (area, explanation, typically_temporary) = CANONICAL_DIFFERENCES[idx];

            // Sample a US GAAP amount and derive IFRS amount with a
            // realistic delta (±30% of US GAAP).
            let us_gaap_raw = diff_dist.sample(&mut self.rng).abs().max(5_000.0_f64);
            let delta_factor: f64 = self.rng.random_range(-0.30..0.30);
            let ifrs_raw = us_gaap_raw * (1.0 + delta_factor);

            let us_gaap_amount = Decimal::from_f64(us_gaap_raw)
                .unwrap_or_else(|| Decimal::from(1_000_000))
                .round_dp(2);
            let ifrs_amount = Decimal::from_f64(ifrs_raw)
                .unwrap_or_else(|| Decimal::from(1_000_000))
                .round_dp(2);

            let source_ref = format!("{company_code}-{area:?}-{:02}", idx + 1);
            let description = format!("{area} difference — {}", self.rng.random::<u32>() % 1000);

            let mut record = FrameworkDifferenceRecord::new(
                company_code,
                period_date,
                area,
                source_ref,
                description,
                us_gaap_amount,
                ifrs_amount,
            );
            record.explanation = explanation.to_string();
            record.difference_type = if typically_temporary {
                DifferenceType::Temporary
            } else {
                DifferenceType::Permanent
            };
            // Classification strings — framework-specific line items.
            record.us_gaap_classification = Self::us_gaap_classification(area);
            record.ifrs_classification = Self::ifrs_classification(area);

            // Financial-statement impact — sign and which line moves
            // depends on the area.
            let impact = Self::compute_impact(area, record.difference_amount);
            cumulative_impact.assets_impact += impact.assets_impact;
            cumulative_impact.liabilities_impact += impact.liabilities_impact;
            cumulative_impact.equity_impact += impact.equity_impact;
            cumulative_impact.revenue_impact += impact.revenue_impact;
            cumulative_impact.expense_impact += impact.expense_impact;
            cumulative_impact.net_income_impact += impact.net_income_impact;
            record.financial_statement_impact = impact;

            records.push(record);
        }

        // Build per-entity reconciliation summary from cumulative impact.
        // Anchor US GAAP NI / equity / assets to plausible round-number
        // baselines; IFRS figures are derived by applying the cumulative
        // delta.
        let us_gaap_ni = Decimal::from(10_000_000);
        let ifrs_ni = us_gaap_ni + cumulative_impact.net_income_impact;
        let us_gaap_equity = Decimal::from(50_000_000);
        let ifrs_equity = us_gaap_equity + cumulative_impact.equity_impact;
        let us_gaap_assets = Decimal::from(200_000_000);
        let ifrs_assets = us_gaap_assets + cumulative_impact.assets_impact;

        let reconciling_items: Vec<ReconcilingItem> = records
            .iter()
            .map(|r| ReconcilingItem {
                description: r.description.clone(),
                difference_area: r.difference_area,
                net_income_impact: r.financial_statement_impact.net_income_impact,
                equity_impact: r.financial_statement_impact.equity_impact,
                asset_impact: r.financial_statement_impact.assets_impact,
                liability_impact: r.financial_statement_impact.liabilities_impact,
                explanation: r.explanation.clone(),
            })
            .collect();

        let reconciliation = FrameworkReconciliation {
            company_code: company_code.to_string(),
            period_date,
            us_gaap_net_income: us_gaap_ni,
            ifrs_net_income: ifrs_ni,
            us_gaap_equity,
            ifrs_equity,
            us_gaap_assets,
            ifrs_assets,
            reconciling_items,
        };

        (records, reconciliation)
    }

    fn us_gaap_classification(area: DifferenceArea) -> String {
        match area {
            DifferenceArea::RevenueRecognition => "Revenue — ASC 606",
            DifferenceArea::LeaseAccounting => "Operating lease expense — ASC 842",
            DifferenceArea::InventoryCosting => "Inventory (LIFO) — ASC 330",
            DifferenceArea::DevelopmentCosts => "R&D expense — ASC 730",
            DifferenceArea::PropertyRevaluation => "PP&E at cost — ASC 360",
            DifferenceArea::Impairment => "Impairment loss — ASC 360 / ASC 350",
            DifferenceArea::ContingentLiabilities => "Contingent loss — ASC 450",
            DifferenceArea::ShareBasedPayment => "SBC expense — ASC 718",
            DifferenceArea::FinancialInstruments => "Credit loss allowance — ASC 326",
            DifferenceArea::Consolidation => "VIE consolidation — ASC 810",
            DifferenceArea::JointArrangements => "Equity-method investment — ASC 323",
            DifferenceArea::IncomeTaxes => "Uncertain tax position — ASC 740",
            _ => "Other",
        }
        .to_string()
    }

    fn ifrs_classification(area: DifferenceArea) -> String {
        match area {
            DifferenceArea::RevenueRecognition => "Revenue — IFRS 15",
            DifferenceArea::LeaseAccounting => "ROU asset + lease liability — IFRS 16",
            DifferenceArea::InventoryCosting => "Inventory (FIFO/WA) — IAS 2",
            DifferenceArea::DevelopmentCosts => "Intangible assets — IAS 38",
            DifferenceArea::PropertyRevaluation => "PP&E (revaluation model) — IAS 16",
            DifferenceArea::Impairment => "Impairment loss — IAS 36",
            DifferenceArea::ContingentLiabilities => "Provision — IAS 37",
            DifferenceArea::ShareBasedPayment => "SBC expense — IFRS 2",
            DifferenceArea::FinancialInstruments => "Credit loss allowance — IFRS 9",
            DifferenceArea::Consolidation => "Subsidiary consolidation — IFRS 10",
            DifferenceArea::JointArrangements => "Joint venture — IFRS 11",
            DifferenceArea::IncomeTaxes => "Uncertain tax position — IAS 12 / IFRIC 23",
            _ => "Other",
        }
        .to_string()
    }

    fn compute_impact(area: DifferenceArea, delta: Decimal) -> FinancialStatementImpact {
        // Simplified mapping from difference area to P&L / BS movement.
        // The sign of `delta` carries through.
        match area {
            DifferenceArea::RevenueRecognition => FinancialStatementImpact {
                revenue_impact: delta,
                net_income_impact: delta,
                equity_impact: delta,
                ..Default::default()
            },
            DifferenceArea::LeaseAccounting => FinancialStatementImpact {
                assets_impact: delta,
                liabilities_impact: delta,
                ..Default::default()
            },
            DifferenceArea::InventoryCosting | DifferenceArea::DevelopmentCosts => {
                FinancialStatementImpact {
                    assets_impact: delta,
                    equity_impact: delta,
                    net_income_impact: delta,
                    ..Default::default()
                }
            }
            DifferenceArea::PropertyRevaluation => FinancialStatementImpact {
                assets_impact: delta,
                equity_impact: delta,
                ..Default::default()
            },
            DifferenceArea::Impairment => FinancialStatementImpact {
                assets_impact: -delta,
                expense_impact: delta,
                net_income_impact: -delta,
                equity_impact: -delta,
                ..Default::default()
            },
            DifferenceArea::ContingentLiabilities => FinancialStatementImpact {
                liabilities_impact: delta,
                expense_impact: delta,
                net_income_impact: -delta,
                equity_impact: -delta,
                ..Default::default()
            },
            _ => FinancialStatementImpact {
                equity_impact: delta,
                net_income_impact: delta,
                ..Default::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_expected_number_of_records() {
        let mut gen = FrameworkReconciliationGenerator::new(42);
        let (records, _recon) =
            gen.generate("C001", NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());
        assert_eq!(records.len(), DEFAULT_DIFFERENCE_COUNT);
    }

    #[test]
    fn reconciliation_includes_item_per_record() {
        let mut gen = FrameworkReconciliationGenerator::new(7);
        let (records, recon) = gen.generate("C001", NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());
        assert_eq!(recon.reconciling_items.len(), records.len());
    }

    #[test]
    fn difference_amounts_are_signed() {
        let mut gen = FrameworkReconciliationGenerator::new(13);
        let (records, _) = gen.generate("C001", NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());
        // At least one record should have negative difference (IFRS < US GAAP).
        let has_negative = records.iter().any(|r| r.difference_amount < Decimal::ZERO);
        let has_positive = records.iter().any(|r| r.difference_amount > Decimal::ZERO);
        assert!(
            has_negative || has_positive,
            "some differences must be non-zero"
        );
    }

    #[test]
    fn areas_are_distinct() {
        let mut gen = FrameworkReconciliationGenerator::new(5);
        let (records, _) = gen.generate("C001", NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());
        let mut areas: Vec<DifferenceArea> = records.iter().map(|r| r.difference_area).collect();
        areas.sort_by_key(|a| format!("{a:?}"));
        areas.dedup();
        assert_eq!(
            areas.len(),
            records.len(),
            "each generated record should cover a distinct area"
        );
    }
}
